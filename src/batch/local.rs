use super::*;
use std::collections::{HashMap, HashSet};

pub(super) fn batch_prepare(args: BatchPrepareArgs) -> Result<()> {
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
    let batch_lock_path = batch_lock_path(&cache);
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
            true,
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

pub(super) fn batch_import(args: BatchImportArgs) -> Result<()> {
    if args.common.no_cache {
        bail!("batch import requires cache; remove --no-cache");
    }
    let _run_lock = acquire_input_run_lock(&args.input, "import batch input EPUB")?;
    let mut cache = CacheStore::from_args(&args.input, &args.common)?;
    let batch_dir = cache.dir.join(BATCH_DIR);
    let batch_lock_path = batch_lock_path(&cache);
    let _batch_lock = FileLock::acquire(&batch_lock_path, "import batch")?;

    let output_path = args.output.unwrap_or_else(|| batch_dir.join(OUTPUT_FILE));
    let manifest_path = batch_dir.join(BATCH_MANIFEST_FILE);
    let work_items_path = batch_dir.join(WORK_ITEMS_FILE);
    let mut manifest: BatchManifest = read_json_file(&manifest_path)?;
    let mut work_items: Vec<WorkItem> = read_jsonl_file(&work_items_path)?;
    let output_lines: Vec<BatchOutputLine> = read_jsonl_file(&output_path)?;
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
    manifest.output_file = Some(output_path.display().to_string());
    manifest.imported_count = imported.len();
    manifest.rejected_count = rejected.len();
    manifest.failed_count = errors.len();

    let report = ImportReport {
        imported_count: imported.len(),
        rejected_count: rejected.len(),
        error_count: errors.len(),
        output_file: output_path.display().to_string(),
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
