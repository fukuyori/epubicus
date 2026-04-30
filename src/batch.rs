use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::{Cursor, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use quick_xml::{Reader, events::Event};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    cache::CacheStore,
    config::{
        BatchArgs, BatchCommand, BatchImportArgs, BatchPrepareArgs, DEFAULT_CLAUDE_MODEL,
        DEFAULT_MODEL, DEFAULT_OPENAI_MODEL, Provider,
    },
    epub::{EpubBook, is_block_tag, unpack_epub},
    glossary::{GlossaryEntry, load_glossary},
    input_lock::acquire_input_run_lock,
    lock::FileLock,
    prompt::{system_prompt, user_prompt},
    translator::{cache_key, extract_openai_text, validate_translation_response},
    xhtml::{collect_element_inner, encode_inline},
};

const BATCH_DIR: &str = "batch";
const BATCH_MANIFEST_FILE: &str = "batch_manifest.json";
const WORK_ITEMS_FILE: &str = "work_items.jsonl";
const REQUESTS_FILE: &str = "requests.jsonl";
const IMPORT_REPORT_FILE: &str = "import_report.json";
const REJECTED_FILE: &str = "rejected.jsonl";
const ERRORS_FILE: &str = "errors.jsonl";
const BATCH_SCHEMA_VERSION: u32 = 1;

pub(crate) fn batch_command(args: BatchArgs) -> Result<()> {
    match args.command {
        BatchCommand::Prepare(args) => batch_prepare(args),
        BatchCommand::Import(args) => batch_import(args),
    }
}

fn batch_prepare(args: BatchPrepareArgs) -> Result<()> {
    if args.common.no_cache {
        bail!("batch prepare requires cache; remove --no-cache");
    }
    if args.common.provider != Provider::Openai {
        bail!("batch prepare currently supports --provider openai only");
    }
    let _run_lock = acquire_input_run_lock(&args.input, "prepare batch input EPUB")?;
    let book = unpack_epub(&args.input)?;
    let range = normalize_batch_range(args.from, args.to, book.spine.len())?;
    let cache = CacheStore::from_args(&args.input, &args.common)?;
    let batch_dir = cache.dir.join(BATCH_DIR);
    fs::create_dir_all(&batch_dir)
        .with_context(|| format!("failed to create batch dir {}", batch_dir.display()))?;
    let batch_lock_path = cache
        .lock_path
        .with_file_name(format!("{}.batch.lock", cache.input_hash));
    let _batch_lock = FileLock::acquire(&batch_lock_path, "prepare batch")?;

    let model = args
        .common
        .model
        .clone()
        .unwrap_or_else(|| default_model_for_provider(args.common.provider).to_string());
    let glossary = match &args.common.glossary {
        Some(path) => load_glossary(path)?,
        None => Vec::new(),
    };
    let mut prepared = Vec::new();
    let selected: HashSet<usize> = range.clone().collect();
    for (page_idx, item) in book.spine.iter().enumerate() {
        let page_no = page_idx + 1;
        if !selected.contains(&page_no) {
            continue;
        }
        collect_page_work_items(
            &book,
            &item.abs_path,
            page_no,
            &cache,
            args.common.provider,
            &model,
            &args.common.style,
            &glossary,
            &mut prepared,
        )
        .with_context(|| format!("failed to prepare batch work for {}", item.href))?;
    }

    let manifest = BatchManifest {
        schema_version: BATCH_SCHEMA_VERSION,
        input_sha256: cache.input_sha256.clone(),
        provider: args.common.provider.to_string(),
        model: model.clone(),
        endpoint: "/v1/responses".to_string(),
        completion_window: "24h".to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
        updated_at: chrono::Utc::now().to_rfc3339(),
        request_file: REQUESTS_FILE.to_string(),
        work_items_file: WORK_ITEMS_FILE.to_string(),
        request_count: prepared.len(),
        file_id: None,
        batch_id: None,
        status: "prepared".to_string(),
        output_file_id: None,
        error_file_id: None,
        output_file: None,
        error_file: None,
        imported_count: 0,
        failed_count: 0,
        rejected_count: 0,
    };

    write_json_pretty_atomic(&batch_dir.join(BATCH_MANIFEST_FILE), &manifest)?;
    write_jsonl_atomic(
        &batch_dir.join(WORK_ITEMS_FILE),
        prepared.iter().map(|item| &item.work_item),
    )?;
    write_jsonl_atomic(
        &batch_dir.join(REQUESTS_FILE),
        prepared.iter().map(|item| &item.request),
    )?;

    println!(
        "Prepared batch: {} request(s) | provider: {} | model: {} | dir: {}",
        manifest.request_count,
        manifest.provider,
        manifest.model,
        batch_dir.display()
    );
    Ok(())
}

fn batch_import(args: BatchImportArgs) -> Result<()> {
    if args.common.no_cache {
        bail!("batch import requires cache; remove --no-cache");
    }
    let _run_lock = acquire_input_run_lock(&args.input, "import batch input EPUB")?;
    let mut cache = CacheStore::from_args(&args.input, &args.common)?;
    let batch_dir = cache.dir.join(BATCH_DIR);
    let batch_lock_path = cache
        .lock_path
        .with_file_name(format!("{}.batch.lock", cache.input_hash));
    let _batch_lock = FileLock::acquire(&batch_lock_path, "import batch")?;

    let manifest_path = batch_dir.join(BATCH_MANIFEST_FILE);
    let work_items_path = batch_dir.join(WORK_ITEMS_FILE);
    let mut manifest: BatchManifest = read_json_file(&manifest_path)?;
    let mut work_items: Vec<WorkItem> = read_jsonl_file(&work_items_path)?;
    let output_lines: Vec<BatchOutputLine> = read_jsonl_file(&args.output)?;
    let mut item_by_id = work_items
        .iter()
        .enumerate()
        .map(|(idx, item)| (item.custom_id.clone(), idx))
        .collect::<HashMap<_, _>>();

    let now = chrono::Utc::now().to_rfc3339();
    let mut imported = Vec::new();
    let mut rejected = Vec::new();
    let mut errors = Vec::new();
    let mut seen_output_ids = HashSet::new();

    for line in output_lines {
        if !seen_output_ids.insert(line.custom_id.clone()) {
            errors.push(ImportErrorLine {
                custom_id: line.custom_id,
                error: "duplicate output custom_id".to_string(),
            });
            continue;
        }
        let Some(item_index) = item_by_id.remove(&line.custom_id) else {
            errors.push(ImportErrorLine {
                custom_id: line.custom_id,
                error: "output custom_id was not found in work_items.jsonl".to_string(),
            });
            continue;
        };
        let item = &mut work_items[item_index];
        match import_output_line(line, item, &mut cache) {
            Ok(()) => {
                item.state = "imported".to_string();
                item.last_error = None;
                item.updated_at = now.clone();
                imported.push(item.custom_id.clone());
            }
            Err(err) => {
                item.state = "rejected".to_string();
                item.last_error = Some(err.to_string());
                item.updated_at = now.clone();
                rejected.push(RejectedLine {
                    custom_id: item.custom_id.clone(),
                    cache_key: item.cache_key.clone(),
                    source_hash: item.source_hash.clone(),
                    error: err.to_string(),
                });
            }
        }
    }

    manifest.updated_at = now;
    manifest.status = "imported".to_string();
    manifest.output_file = Some(args.output.display().to_string());
    manifest.imported_count = imported.len();
    manifest.rejected_count = rejected.len();
    manifest.failed_count = errors.len();

    let report = ImportReport {
        imported_count: imported.len(),
        rejected_count: rejected.len(),
        error_count: errors.len(),
        output_file: args.output.display().to_string(),
    };

    write_json_pretty_atomic(&manifest_path, &manifest)?;
    write_jsonl_atomic(&work_items_path, work_items.iter())?;
    write_jsonl_atomic(&batch_dir.join(REJECTED_FILE), rejected.iter())?;
    write_jsonl_atomic(&batch_dir.join(ERRORS_FILE), errors.iter())?;
    write_json_pretty_atomic(&batch_dir.join(IMPORT_REPORT_FILE), &report)?;

    println!(
        "Imported batch output: {} imported | {} rejected | {} error(s) | dir: {}",
        report.imported_count,
        report.rejected_count,
        report.error_count,
        batch_dir.display()
    );
    Ok(())
}

fn import_output_line(
    line: BatchOutputLine,
    item: &WorkItem,
    cache: &mut CacheStore,
) -> Result<()> {
    if item.source_hash != hash_text(&item.source_text) {
        bail!("work item source_hash does not match source_text");
    }
    if let Some(error) = line.error {
        bail!("batch output error: {error}");
    }
    let response = line.response.context("batch output line has no response")?;
    if response.status_code != 200 {
        bail!("batch output status_code was {}", response.status_code);
    }
    let translated = extract_openai_text(&response.body)
        .context("batch response body did not contain output text")?;
    validate_translation_response(&item.source_text, &translated)?;
    cache.insert(crate::cache::CacheRecord {
        key: item.cache_key.clone(),
        translated,
        provider: item.provider.clone(),
        model: item.model.clone(),
        at: chrono::Utc::now().to_rfc3339(),
    })
}

#[allow(clippy::too_many_arguments)]
fn collect_page_work_items(
    book: &EpubBook,
    path: &Path,
    page_no: usize,
    cache: &CacheStore,
    provider: Provider,
    model: &str,
    style: &str,
    glossary: &[GlossaryEntry],
    out: &mut Vec<PreparedItem>,
) -> Result<()> {
    let source = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut reader = Reader::from_reader(Cursor::new(source));
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut block_index = 0usize;
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) if is_block_tag(e.name()) => {
                let end_name = e.name().as_ref().to_vec();
                let inner = collect_element_inner(&mut reader, &end_name)?;
                let (source_text, _) = encode_inline(&inner)?;
                let source_text = source_text.trim().to_string();
                if !source_text.is_empty() {
                    block_index += 1;
                    let glossary_subset = glossary_subset(glossary, &source_text);
                    let cache_key =
                        cache_key(provider, model, style, &source_text, &glossary_subset);
                    if cache.peek(&cache_key).is_some() {
                        continue;
                    }
                    let system = system_prompt(style);
                    let prompt = user_prompt(&source_text, &glossary_subset);
                    let custom_id = format!(
                        "epubicus:{}:p{:04}:b{:04}:{}",
                        cache.input_hash, page_no, block_index, cache_key
                    );
                    let request = BatchRequestLine {
                        custom_id: custom_id.clone(),
                        method: "POST".to_string(),
                        url: "/v1/responses".to_string(),
                        body: BatchResponsesBody {
                            model: model.to_string(),
                            instructions: system.clone(),
                            input: prompt.clone(),
                        },
                    };
                    let href = book
                        .spine
                        .get(page_no - 1)
                        .map(|item| item.href.clone())
                        .unwrap_or_default();
                    let work_item = WorkItem {
                        custom_id,
                        cache_key,
                        page_index: page_no,
                        block_index,
                        href,
                        source_text: source_text.clone(),
                        source_hash: hash_text(&source_text),
                        prompt_hash: hash_text(&format!("{system}\n{prompt}")),
                        source_chars: source_text.chars().count(),
                        provider: provider.to_string(),
                        model: model.to_string(),
                        state: "prepared".to_string(),
                        attempt: 1,
                        last_error: None,
                        updated_at: chrono::Utc::now().to_rfc3339(),
                    };
                    out.push(PreparedItem { work_item, request });
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(())
}

fn normalize_batch_range(
    from: Option<usize>,
    to: Option<usize>,
    len: usize,
) -> Result<std::ops::RangeInclusive<usize>> {
    if len == 0 {
        bail!("EPUB spine has no XHTML pages");
    }
    let from = from.unwrap_or(1);
    let to = to.unwrap_or(len);
    if from == 0 || to == 0 || from > to || to > len {
        bail!("invalid range {from}-{to}; valid spine page range is 1-{len}");
    }
    Ok(from..=to)
}

fn glossary_subset(entries: &[GlossaryEntry], source: &str) -> Vec<GlossaryEntry> {
    let source_lower = source.to_lowercase();
    entries
        .iter()
        .filter(|entry| !entry.src.trim().is_empty() && !entry.dst.trim().is_empty())
        .filter(|entry| source_lower.contains(&entry.src.to_lowercase()))
        .cloned()
        .collect()
}

fn default_model_for_provider(provider: Provider) -> &'static str {
    match provider {
        Provider::Ollama => DEFAULT_MODEL,
        Provider::Openai => DEFAULT_OPENAI_MODEL,
        Provider::Claude => DEFAULT_CLAUDE_MODEL,
    }
}

fn hash_text(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    let digest = hasher.finalize();
    digest[..16].iter().map(|b| format!("{b:02x}")).collect()
}

fn write_json_pretty_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let tmp = tmp_path(path);
    let data = serde_json::to_vec_pretty(value).context("failed to serialize JSON")?;
    fs::write(&tmp, data).with_context(|| format!("failed to write {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("failed to commit {}", path.display()))?;
    Ok(())
}

fn read_json_file<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let data = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_slice(&data).with_context(|| format!("failed to parse {}", path.display()))
}

fn read_jsonl_file<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<Vec<T>> {
    let text =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    text.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(idx, line)| {
            serde_json::from_str(line)
                .with_context(|| format!("failed to parse {} line {}", path.display(), idx + 1))
        })
        .collect()
}

fn write_jsonl_atomic<'a, T, I>(path: &Path, values: I) -> Result<()>
where
    T: Serialize + 'a,
    I: IntoIterator<Item = &'a T>,
{
    let tmp = tmp_path(path);
    let mut file =
        File::create(&tmp).with_context(|| format!("failed to create {}", tmp.display()))?;
    for value in values {
        serde_json::to_writer(&mut file, value).context("failed to serialize JSONL")?;
        writeln!(file).context("failed to write JSONL newline")?;
    }
    file.flush()
        .with_context(|| format!("failed to flush {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("failed to commit {}", path.display()))?;
    Ok(())
}

fn tmp_path(path: &Path) -> PathBuf {
    path.with_extension("tmp")
}

struct PreparedItem {
    work_item: WorkItem,
    request: BatchRequestLine,
}

#[derive(Deserialize, Serialize)]
struct BatchManifest {
    schema_version: u32,
    input_sha256: String,
    provider: String,
    model: String,
    endpoint: String,
    completion_window: String,
    created_at: String,
    updated_at: String,
    request_file: String,
    work_items_file: String,
    request_count: usize,
    file_id: Option<String>,
    batch_id: Option<String>,
    status: String,
    output_file_id: Option<String>,
    error_file_id: Option<String>,
    output_file: Option<String>,
    error_file: Option<String>,
    imported_count: usize,
    failed_count: usize,
    rejected_count: usize,
}

#[derive(Deserialize, Serialize)]
struct WorkItem {
    custom_id: String,
    cache_key: String,
    page_index: usize,
    block_index: usize,
    href: String,
    source_text: String,
    source_hash: String,
    prompt_hash: String,
    source_chars: usize,
    provider: String,
    model: String,
    state: String,
    attempt: u32,
    last_error: Option<String>,
    updated_at: String,
}

#[derive(Deserialize, Serialize)]
struct BatchRequestLine {
    custom_id: String,
    method: String,
    url: String,
    body: BatchResponsesBody,
}

#[derive(Deserialize, Serialize)]
struct BatchResponsesBody {
    model: String,
    instructions: String,
    input: String,
}

#[derive(Deserialize)]
struct BatchOutputLine {
    custom_id: String,
    response: Option<BatchOutputResponse>,
    error: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct BatchOutputResponse {
    status_code: u16,
    body: serde_json::Value,
}

#[derive(Serialize)]
struct RejectedLine {
    custom_id: String,
    cache_key: String,
    source_hash: String,
    error: String,
}

#[derive(Serialize)]
struct ImportErrorLine {
    custom_id: String,
    error: String,
}

#[derive(Serialize)]
struct ImportReport {
    imported_count: usize,
    rejected_count: usize,
    error_count: usize,
    output_file: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        CommonArgs, DEFAULT_CLAUDE_BASE_URL, DEFAULT_CONCURRENCY, DEFAULT_MAX_CHARS_PER_REQUEST,
        DEFAULT_OLLAMA_HOST, DEFAULT_OPENAI_BASE_URL,
    };
    use zip::{
        CompressionMethod, ZipWriter,
        write::{FileOptions, SimpleFileOptions},
    };

    #[test]
    fn text_hash_is_stable_and_short() {
        assert_eq!(hash_text("abc"), hash_text("abc"));
        assert_eq!(hash_text("abc").len(), 32);
        assert_ne!(hash_text("abc"), hash_text("abcd"));
    }

    #[test]
    fn batch_range_defaults_to_full_spine() -> Result<()> {
        let range = normalize_batch_range(None, None, 3)?;
        assert_eq!(range.collect::<Vec<_>>(), vec![1, 2, 3]);
        Ok(())
    }

    #[test]
    fn batch_prepare_writes_manifest_work_items_and_requests() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let input = dir.path().join("minimal.epub");
        write_minimal_epub(&input)?;
        let args = BatchPrepareArgs {
            input: input.clone(),
            from: Some(1),
            to: Some(1),
            common: common_args(dir.path().join("cache")),
        };

        batch_prepare(args)?;

        let cache = CacheStore::from_args(&input, &common_args(dir.path().join("cache")))?;
        let batch_dir = cache.dir.join(BATCH_DIR);
        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(batch_dir.join(BATCH_MANIFEST_FILE))?)?;
        assert_eq!(manifest["provider"], "openai");
        assert_eq!(manifest["model"], DEFAULT_OPENAI_MODEL);
        assert_eq!(manifest["endpoint"], "/v1/responses");
        assert_eq!(manifest["request_count"], 2);

        let work_items = read_jsonl_values(&batch_dir.join(WORK_ITEMS_FILE))?;
        let requests = read_jsonl_values(&batch_dir.join(REQUESTS_FILE))?;
        assert_eq!(work_items.len(), 2);
        assert_eq!(requests.len(), 2);
        assert_eq!(work_items[0]["state"], "prepared");
        assert_eq!(work_items[0]["page_index"], 1);
        assert_eq!(work_items[0]["block_index"], 1);
        assert_eq!(requests[0]["method"], "POST");
        assert_eq!(requests[0]["url"], "/v1/responses");
        assert_eq!(requests[0]["body"]["model"], DEFAULT_OPENAI_MODEL);
        assert_eq!(requests[0]["custom_id"], work_items[0]["custom_id"]);
        assert!(
            requests[0]["body"]["input"]
                .as_str()
                .unwrap()
                .contains("<source>")
        );
        Ok(())
    }

    #[test]
    fn batch_import_writes_valid_output_to_cache() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let input = dir.path().join("minimal.epub");
        write_minimal_epub(&input)?;
        let cache_root = dir.path().join("cache");
        let (batch_dir, work_items) = prepare_minimal_batch(&input, &cache_root)?;
        let output_path = dir.path().join("output.jsonl");
        let mut output = File::create(&output_path)?;
        for item in &work_items {
            let custom_id = item["custom_id"].as_str().context("missing custom_id")?;
            let source_text = item["source_text"]
                .as_str()
                .context("missing source_text")?;
            let translated = if source_text.contains("⟦E") {
                "こんにちは、⟦E1⟧世界⟦/E1⟧。"
            } else {
                "これは有効な日本語訳です。"
            };
            writeln!(
                output,
                "{}",
                serde_json::json!({
                    "custom_id": custom_id,
                    "response": {
                        "status_code": 200,
                        "body": {
                            "output_text": translated
                        }
                    },
                    "error": null
                })
            )?;
        }
        output.flush()?;

        batch_import(BatchImportArgs {
            input: input.clone(),
            output: output_path,
            common: common_args(cache_root.clone()),
        })?;

        let imported_cache = CacheStore::from_args(&input, &common_args(cache_root))?;
        for item in &work_items {
            let key = item["cache_key"].as_str().context("missing cache_key")?;
            assert!(imported_cache.peek(key).is_some());
        }
        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(batch_dir.join(BATCH_MANIFEST_FILE))?)?;
        assert_eq!(manifest["imported_count"], 2);
        assert_eq!(manifest["rejected_count"], 0);
        assert!(batch_dir.join(IMPORT_REPORT_FILE).exists());
        Ok(())
    }

    #[test]
    fn batch_import_accepts_reordered_output() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let input = dir.path().join("minimal.epub");
        write_minimal_epub(&input)?;
        let cache_root = dir.path().join("cache");
        let (batch_dir, mut work_items) = prepare_minimal_batch(&input, &cache_root)?;
        work_items.reverse();
        let output_path = dir.path().join("output.jsonl");
        write_fixture_output(&output_path, &work_items, |source| {
            if source.contains("⟦E") {
                "こんにちは、⟦E1⟧世界⟦/E1⟧。".to_string()
            } else {
                "これは有効な日本語訳です。".to_string()
            }
        })?;

        batch_import(BatchImportArgs {
            input: input.clone(),
            output: output_path,
            common: common_args(cache_root.clone()),
        })?;

        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(batch_dir.join(BATCH_MANIFEST_FILE))?)?;
        assert_eq!(manifest["imported_count"], 2);
        assert_eq!(manifest["rejected_count"], 0);
        Ok(())
    }

    #[test]
    fn batch_import_rejects_invalid_translation_without_caching() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let input = dir.path().join("minimal.epub");
        write_minimal_epub(&input)?;
        let cache_root = dir.path().join("cache");
        let (batch_dir, work_items) = prepare_minimal_batch(&input, &cache_root)?;
        let output_path = dir.path().join("output.jsonl");
        write_fixture_output(&output_path, &work_items, |source| source.to_string())?;

        batch_import(BatchImportArgs {
            input: input.clone(),
            output: output_path,
            common: common_args(cache_root.clone()),
        })?;

        let imported_cache = CacheStore::from_args(&input, &common_args(cache_root))?;
        for item in &work_items {
            let key = item["cache_key"].as_str().context("missing cache_key")?;
            assert!(imported_cache.peek(key).is_none());
        }
        let rejected = read_jsonl_values(&batch_dir.join(REJECTED_FILE))?;
        assert_eq!(rejected.len(), 2);
        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(batch_dir.join(BATCH_MANIFEST_FILE))?)?;
        assert_eq!(manifest["imported_count"], 0);
        assert_eq!(manifest["rejected_count"], 2);
        Ok(())
    }

    #[test]
    fn batch_import_reports_duplicate_output_custom_id() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let input = dir.path().join("minimal.epub");
        write_minimal_epub(&input)?;
        let cache_root = dir.path().join("cache");
        let (batch_dir, work_items) = prepare_minimal_batch(&input, &cache_root)?;
        let output_path = dir.path().join("output.jsonl");
        let duplicate = vec![work_items[0].clone(), work_items[0].clone()];
        write_fixture_output(&output_path, &duplicate, |_source| {
            "これは有効な日本語訳です。".to_string()
        })?;

        batch_import(BatchImportArgs {
            input: input.clone(),
            output: output_path,
            common: common_args(cache_root),
        })?;

        let errors = read_jsonl_values(&batch_dir.join(ERRORS_FILE))?;
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0]["error"], "duplicate output custom_id");
        Ok(())
    }

    fn common_args(cache_root: PathBuf) -> CommonArgs {
        CommonArgs {
            provider: Provider::Openai,
            model: None,
            fallback_provider: None,
            fallback_model: None,
            ollama_host: DEFAULT_OLLAMA_HOST.to_string(),
            openai_base_url: DEFAULT_OPENAI_BASE_URL.to_string(),
            claude_base_url: DEFAULT_CLAUDE_BASE_URL.to_string(),
            openai_api_key: None,
            anthropic_api_key: None,
            prompt_api_key: false,
            temperature: 0.3,
            num_ctx: 8192,
            timeout_secs: 900,
            retries: 3,
            max_chars_per_request: DEFAULT_MAX_CHARS_PER_REQUEST,
            concurrency: DEFAULT_CONCURRENCY,
            style: "essay".to_string(),
            glossary: None,
            cache_root: Some(cache_root),
            no_cache: false,
            clear_cache: false,
            keep_cache: true,
            usage_only: false,
            partial_from_cache: false,
            dry_run: false,
        }
    }

    fn read_jsonl_values(path: &Path) -> Result<Vec<serde_json::Value>> {
        let text = fs::read_to_string(path)?;
        text.lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str(line).map_err(Into::into))
            .collect()
    }

    fn prepare_minimal_batch(
        input: &Path,
        cache_root: &Path,
    ) -> Result<(PathBuf, Vec<serde_json::Value>)> {
        batch_prepare(BatchPrepareArgs {
            input: input.to_path_buf(),
            from: Some(1),
            to: Some(1),
            common: common_args(cache_root.to_path_buf()),
        })?;
        let cache = CacheStore::from_args(input, &common_args(cache_root.to_path_buf()))?;
        let batch_dir = cache.dir.join(BATCH_DIR);
        let work_items = read_jsonl_values(&batch_dir.join(WORK_ITEMS_FILE))?;
        Ok((batch_dir, work_items))
    }

    fn write_fixture_output<F>(
        path: &Path,
        work_items: &[serde_json::Value],
        translate: F,
    ) -> Result<()>
    where
        F: Fn(&str) -> String,
    {
        let mut output = File::create(path)?;
        for item in work_items {
            let custom_id = item["custom_id"].as_str().context("missing custom_id")?;
            let source_text = item["source_text"]
                .as_str()
                .context("missing source_text")?;
            writeln!(
                output,
                "{}",
                serde_json::json!({
                    "custom_id": custom_id,
                    "response": {
                        "status_code": 200,
                        "body": {
                            "output_text": translate(source_text)
                        }
                    },
                    "error": null
                })
            )?;
        }
        output.flush()?;
        Ok(())
    }

    fn write_minimal_epub(path: &Path) -> Result<()> {
        let file = File::create(path)?;
        let mut zip = ZipWriter::new(file);
        let stored: SimpleFileOptions =
            FileOptions::default().compression_method(CompressionMethod::Stored);
        let deflated: SimpleFileOptions =
            FileOptions::default().compression_method(CompressionMethod::Deflated);

        zip.start_file("mimetype", stored)?;
        zip.write_all(b"application/epub+zip")?;
        zip.start_file("META-INF/container.xml", deflated)?;
        zip.write_all(
            br#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
<rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>"#,
        )?;
        zip.start_file("OEBPS/content.opf", deflated)?;
        zip.write_all(
            br#"<?xml version="1.0" encoding="UTF-8"?>
<package version="3.0" unique-identifier="bookid" xmlns="http://www.idpf.org/2007/opf">
<metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Test Book</dc:title><dc:language>en</dc:language></metadata>
<manifest><item id="c1" href="chapter1.xhtml" media-type="application/xhtml+xml"/></manifest>
<spine><itemref idref="c1"/></spine>
</package>"#,
        )?;
        zip.start_file("OEBPS/chapter1.xhtml", deflated)?;
        zip.write_all(
            br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><body>
<h1>Chapter One</h1>
<p>Hello <em>world</em>.</p>
</body></html>"#,
        )?;
        zip.finish()?;
        Ok(())
    }
}
