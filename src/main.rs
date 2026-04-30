#[cfg(test)]
use std::fs::File;
#[cfg(test)]
use std::io::Write;
#[cfg(test)]
use std::sync::atomic::Ordering;
use std::{
    collections::HashSet,
    fs,
    io::Cursor,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use clap::Parser;
#[cfg(test)]
use quick_xml::Writer;
#[cfg(test)]
use quick_xml::name::QName;
use quick_xml::{Reader, events::Event};
#[cfg(test)]
use zip::{
    CompressionMethod, ZipWriter,
    write::{FileOptions, SimpleFileOptions},
};

mod batch;
mod cache;
mod config;
mod epub;
mod glossary;
mod input_lock;
mod lock;
mod progress;
mod prompt;
mod translator;
mod usage;
mod xhtml;

use batch::batch_command;
#[cfg(test)]
use cache::{CacheRecord, ManifestParams, compute_input_hash};
use cache::{CacheStore, cache_command};
use config::*;
#[cfg(test)]
use epub::local_name;
use epub::{
    EpubBook, count_xhtml_blocks, find_nav_item, find_ncx_item, is_block_tag, pack_epub,
    print_toc_entries, read_nav_toc, read_ncx_toc, unpack_epub, update_opf_metadata,
};
#[cfg(test)]
use glossary::GlossaryEntry;
use glossary::glossary_command;
use input_lock::{acquire_input_run_lock, unlock_command};
use progress::ProgressReporter;
#[cfg(test)]
use prompt::retry_user_prompt;
use prompt::{system_prompt, user_prompt};
#[cfg(test)]
use translator::{
    ADAPTIVE_CONCURRENCY_SUCCESS_THRESHOLD, AdaptiveConcurrency, PROMPT_VERSION, Translation,
    cache_key, is_refusal_validation_error, placeholder_signature, validate_translation_response,
};
use translator::{Translator, split_translation_chunks};
#[cfg(test)]
use usage::ClaudeUsage;
#[cfg(test)]
use usage::{ClaudeResponse, usage_from_claude_response, usage_from_openai_value};
#[cfg(test)]
use xhtml::{
    Token, restore_inline_or_original, tokenize_placeholders, try_restore_inline, write_events,
};
use xhtml::{collect_element_inner, encode_inline, translate_xhtml_file};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Write,
    Stdout,
}

#[derive(Debug, Default)]
struct Stats {
    pages_seen: usize,
    pages_translated: usize,
    blocks_translated: usize,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Translate(args) => translate_command(args),
        Commands::Test(args) => test_command(args),
        Commands::Inspect(args) => inspect_command(args),
        Commands::Toc(args) => toc_command(args),
        Commands::Glossary(args) => glossary_command(args),
        Commands::Cache(args) => cache_command(args),
        Commands::Batch(args) => batch_command(args),
        Commands::Unlock(args) => unlock_command(args),
    }
}

pub(crate) fn translate_command(args: TranslateArgs) -> Result<()> {
    let _run_lock = acquire_input_run_lock(&args.input, "translate input EPUB")?;
    let output = args
        .output
        .unwrap_or_else(|| default_output_path(&args.input));
    let book = unpack_epub(&args.input)?;
    let range = normalize_range(args.from, args.to, book.spine.len())?;
    let usage_only = args.common.usage_only;
    let partial_from_cache = args.common.partial_from_cache;
    let cache_args = cache_args_for_read_only_if_needed(&args.common);
    let cache = CacheStore::from_args(&args.input, &cache_args)?;
    let mut translator = Translator::new(args.common, cache)?;
    if usage_only {
        let report = estimate_usage(&book, range, &translator)?;
        report.print(&translator);
        return Ok(());
    }
    if !partial_from_cache {
        let params = translator.manifest_params();
        translator
            .cache
            .upsert_manifest(&args.input, params, Some(&output))?;
    }
    let mut stats = translate_book(&book, range, &mut translator, Mode::Write, true)?;
    update_opf_metadata(&book.opf_path, &translator.backend.model)?;
    pack_epub(book.work_dir.path(), &output)?;
    stats.pages_seen = book.spine.len();
    let cache_dir_display = if translator.cache.enabled {
        translator.cache.dir.display().to_string()
    } else {
        "disabled".to_string()
    };
    let pack_succeeded = true;
    let full_range_translated = stats.pages_translated == book.spine.len() && !partial_from_cache;
    let cache_was_kept_or_partial =
        partial_from_cache || translator.cache.keep_cache || !full_range_translated;
    if pack_succeeded && !cache_was_kept_or_partial {
        translator.cache.finalize_completion()?;
    }
    let cache_status = if !translator.cache.enabled {
        "disabled".to_string()
    } else if pack_succeeded && !cache_was_kept_or_partial {
        format!("auto-deleted ({cache_dir_display})")
    } else {
        cache_dir_display.clone()
    };
    eprintln!(
        "Done. Output: {} | pages translated: {} | blocks translated: {} | provider: {} | model: {} | cache hits: {} | misses: {} | writes: {} | cache: {}",
        output.display(),
        stats.pages_translated,
        stats.blocks_translated,
        translator.backend.provider,
        translator.backend.model,
        translator.cache.stats.hits,
        translator.cache.stats.misses,
        translator.cache.stats.writes,
        cache_status,
    );
    if let Some(summary) = translator.api_usage_summary() {
        eprintln!("API usage: {summary}");
    }
    if partial_from_cache && translator.cache.stats.misses > 0 {
        eprintln!(
            "warning: partial output contains {} cache miss(es); unchanged source text may remain. Run batch verify/health or fill the missing items before final use.",
            translator.cache.stats.misses
        );
    }
    if translator.fallback_count > 0 {
        eprintln!("Fallback translations: {}", translator.fallback_count);
    }
    Ok(())
}

fn test_command(args: TestArgs) -> Result<()> {
    let _run_lock = acquire_input_run_lock(&args.input, "test input EPUB")?;
    let book = unpack_epub(&args.input)?;
    let range = normalize_range(Some(args.from), Some(args.to), book.spine.len())?;
    let usage_only = args.common.usage_only;
    let cache_args = cache_args_for_read_only_if_needed(&args.common);
    let cache = CacheStore::from_args(&args.input, &cache_args)?;
    let mut translator = Translator::new(args.common, cache)?;
    if usage_only {
        let report = estimate_usage(&book, range, &translator)?;
        report.print(&translator);
        return Ok(());
    }
    let stats = translate_book(&book, range, &mut translator, Mode::Stdout, false)?;
    eprintln!(
        "Translated {} spine pages, {} blocks.",
        stats.pages_translated, stats.blocks_translated
    );
    if let Some(summary) = translator.api_usage_summary() {
        eprintln!("API usage: {summary}");
    }
    if translator.fallback_count > 0 {
        eprintln!("Fallback translations: {}", translator.fallback_count);
    }
    Ok(())
}

fn cache_args_for_read_only_if_needed(args: &CommonArgs) -> CommonArgs {
    let mut args = args.clone();
    if args.usage_only {
        args.clear_cache = false;
    }
    args
}

fn inspect_command(args: InspectArgs) -> Result<()> {
    let _run_lock = acquire_input_run_lock(&args.input, "inspect input EPUB")?;
    let book = unpack_epub(&args.input)?;
    println!("OPF: {}", book.opf_path.display());
    println!();
    println!(
        "{:>4}  {:<6}  {:<8}  {:>10}  {:>7}  {:<22}  {}",
        "No", "Linear", "Exists", "Bytes", "Blocks", "Media Type", "Href"
    );
    println!("{}", "-".repeat(96));
    for (idx, item) in book.spine.iter().enumerate() {
        let metadata = fs::metadata(&item.abs_path).ok();
        let exists = metadata.is_some();
        let bytes = metadata
            .map(|m| m.len().to_string())
            .unwrap_or_else(|| "-".to_string());
        let blocks = if exists {
            count_xhtml_blocks(&item.abs_path)
                .map(|count| count.to_string())
                .unwrap_or_else(|_| "parseerr".to_string())
        } else {
            "-".to_string()
        };
        println!(
            "{:>4}  {:<6}  {:<8}  {:>10}  {:>7}  {:<22}  {}",
            idx + 1,
            if item.linear { "yes" } else { "no" },
            if exists { "yes" } else { "missing" },
            bytes,
            blocks,
            item.media_type,
            item.href
        );
        println!(
            "      idref={} path={}",
            item.idref,
            item.abs_path.display()
        );
    }
    Ok(())
}

fn toc_command(args: TocArgs) -> Result<()> {
    let _run_lock = acquire_input_run_lock(&args.input, "toc input EPUB")?;
    let book = unpack_epub(&args.input)?;
    println!("OPF: {}", book.opf_path.display());
    if let Some(nav) = find_nav_item(&book.manifest) {
        println!("TOC: EPUB3 nav ({})", nav.href);
        println!();
        let entries = read_nav_toc(&nav.abs_path)?;
        print_toc_entries(&entries);
        return Ok(());
    }
    if let Some(ncx) = find_ncx_item(&book.manifest) {
        println!("TOC: EPUB2 NCX ({})", ncx.href);
        println!();
        let entries = read_ncx_toc(&ncx.abs_path)?;
        print_toc_entries(&entries);
        return Ok(());
    }
    bail!("no EPUB3 nav.xhtml or EPUB2 NCX item found in OPF manifest")
}

fn default_output_path(input: &Path) -> PathBuf {
    let stem = input.file_stem().and_then(|s| s.to_str()).unwrap_or("book");
    input.with_file_name(format!("{stem}.ja.epub"))
}

fn default_model_for_provider(provider: Provider) -> &'static str {
    match provider {
        Provider::Ollama => DEFAULT_MODEL,
        Provider::Openai => DEFAULT_OPENAI_MODEL,
        Provider::Claude => DEFAULT_CLAUDE_MODEL,
    }
}

fn normalize_range(
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

fn translate_book(
    book: &EpubBook,
    range: std::ops::RangeInclusive<usize>,
    translator: &mut Translator,
    mode: Mode,
    show_progress: bool,
) -> Result<Stats> {
    let selected: HashSet<usize> = range.collect();
    let mut stats = Stats::default();
    let mut progress = if show_progress {
        let total_blocks = count_selected_blocks(book, &selected)?;
        let cached_blocks = count_selected_cached_blocks(book, &selected, translator)?;
        Some(ProgressReporter::new(total_blocks, cached_blocks)?)
    } else {
        None
    };
    for (idx, item) in book.spine.iter().enumerate() {
        let page_no = idx + 1;
        if !selected.contains(&page_no) {
            continue;
        }
        stats.pages_translated += 1;
        if let Some(progress) = progress.as_mut() {
            progress.set_page(page_no, book.spine.len(), &item.href);
        }
        if mode == Mode::Stdout {
            println!("\n===== spine page {page_no}: {} =====\n", item.href);
        }
        let result = translate_xhtml_file(&item.abs_path, translator, mode, progress.as_mut())
            .with_context(|| format!("failed to translate spine page {page_no}: {}", item.href))?;
        stats.blocks_translated += result;
    }
    if let Some(progress) = progress {
        progress.finish(&stats);
    }
    Ok(stats)
}

#[derive(Debug, Default)]
struct UsageEstimate {
    pages_selected: usize,
    total_pages: usize,
    blocks_total: usize,
    cached_blocks: usize,
    uncached_blocks: usize,
    source_chars: usize,
    uncached_source_chars: usize,
    estimated_requests: usize,
    estimated_input_tokens: u64,
    estimated_output_tokens: u64,
}

impl UsageEstimate {
    fn estimated_total_tokens(&self) -> u64 {
        self.estimated_input_tokens + self.estimated_output_tokens
    }

    fn print(&self, translator: &Translator) {
        println!("Usage estimate only. No translation provider was called.");
        println!("Provider: {}", translator.backend.provider);
        println!("Model: {}", translator.backend.model);
        if let Some(fallback) = &translator.fallback_backend {
            println!("Fallback provider: {}", fallback.provider);
            println!("Fallback model: {}", fallback.model);
        }
        println!(
            "Pages: {}/{} selected",
            self.pages_selected, self.total_pages
        );
        println!(
            "Blocks: {} total, {} cached, {} uncached",
            self.blocks_total, self.cached_blocks, self.uncached_blocks
        );
        println!(
            "Source chars: {} total, {} uncached",
            self.source_chars, self.uncached_source_chars
        );
        println!("Estimated API requests: {}", self.estimated_requests);
        println!(
            "Estimated tokens: input {}, output {}, total {}",
            self.estimated_input_tokens,
            self.estimated_output_tokens,
            self.estimated_total_tokens()
        );
        println!("Note: token counts are approximate before the API returns actual usage.");
    }
}

fn estimate_usage(
    book: &EpubBook,
    range: std::ops::RangeInclusive<usize>,
    translator: &Translator,
) -> Result<UsageEstimate> {
    let selected: HashSet<usize> = range.collect();
    let mut estimate = UsageEstimate {
        pages_selected: selected.len(),
        total_pages: book.spine.len(),
        ..UsageEstimate::default()
    };
    for (idx, item) in book.spine.iter().enumerate() {
        if !selected.contains(&(idx + 1)) {
            continue;
        }
        estimate_xhtml_usage(&item.abs_path, translator, &mut estimate)
            .with_context(|| format!("failed to estimate usage for {}", item.href))?;
    }
    Ok(estimate)
}

fn estimate_xhtml_usage(
    path: &Path,
    translator: &Translator,
    estimate: &mut UsageEstimate,
) -> Result<()> {
    let source = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut reader = Reader::from_reader(Cursor::new(source));
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) if is_block_tag(e.name()) => {
                let end_name = e.name().as_ref().to_vec();
                let inner = collect_element_inner(&mut reader, &end_name)?;
                let (source_text, _) = encode_inline(&inner)?;
                let source_text = source_text.trim();
                if !source_text.is_empty() {
                    estimate.blocks_total += 1;
                    let source_chars = source_text.chars().count();
                    estimate.source_chars += source_chars;
                    if translator.has_cached_translation(source_text) {
                        estimate.cached_blocks += 1;
                    } else {
                        estimate.uncached_blocks += 1;
                        estimate.uncached_source_chars += source_chars;
                        add_uncached_usage_estimate(source_text, translator, estimate);
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(())
}

fn add_uncached_usage_estimate(
    source: &str,
    translator: &Translator,
    estimate: &mut UsageEstimate,
) {
    let chunks = split_translation_chunks(source, translator.backend.max_chars_per_request);
    let system = system_prompt(&translator.backend.style);
    for chunk in chunks {
        let glossary_subset = translator.backend.glossary_subset(&chunk);
        let prompt = user_prompt(&chunk, &glossary_subset);
        estimate.estimated_requests += 1;
        estimate.estimated_input_tokens += estimate_tokens_from_chars(system.chars().count());
        estimate.estimated_input_tokens += estimate_tokens_from_chars(prompt.chars().count());
        estimate.estimated_output_tokens += estimate_tokens_from_chars(chunk.chars().count());
    }
}

fn estimate_tokens_from_chars(chars: usize) -> u64 {
    chars.div_ceil(4) as u64
}

fn count_selected_blocks(book: &EpubBook, selected: &HashSet<usize>) -> Result<u64> {
    let mut total = 0u64;
    for (idx, item) in book.spine.iter().enumerate() {
        if selected.contains(&(idx + 1)) {
            total += count_xhtml_blocks(&item.abs_path)? as u64;
        }
    }
    Ok(total)
}

fn count_selected_cached_blocks(
    book: &EpubBook,
    selected: &HashSet<usize>,
    translator: &Translator,
) -> Result<u64> {
    let mut total = 0u64;
    for (idx, item) in book.spine.iter().enumerate() {
        if selected.contains(&(idx + 1)) {
            total += count_cached_xhtml_blocks(&item.abs_path, translator)? as u64;
        }
    }
    Ok(total)
}

fn count_cached_xhtml_blocks(path: &Path, translator: &Translator) -> Result<usize> {
    let source = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut reader = Reader::from_reader(Cursor::new(source));
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut count = 0usize;

    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) if is_block_tag(e.name()) => {
                let end_name = e.name().as_ref().to_vec();
                let inner = collect_element_inner(&mut reader, &end_name)?;
                let (source_text, _) = encode_inline(&inner)?;
                if !source_text.trim().is_empty() && translator.has_cached_translation(&source_text)
                {
                    count += 1;
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(count)
}

pub(crate) fn collapse_ws(s: &str) -> String {
    let mut out = String::new();
    let mut last_space = false;
    for ch in s.chars() {
        if ch.is_whitespace() {
            if !last_space {
                out.push(' ');
                last_space = true;
            }
        } else {
            out.push(ch);
            last_space = false;
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dry_run_roundtrips_minimal_epub() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let input = dir.path().join("minimal.epub");
        let output = dir.path().join("minimal.ja.epub");
        write_minimal_epub(&input)?;

        let book = unpack_epub(&input)?;
        assert_eq!(book.spine.len(), 2);

        let common = CommonArgs {
            provider: Provider::Ollama,
            model: Some(DEFAULT_MODEL.to_string()),
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
            cache_root: None,
            no_cache: true,
            clear_cache: false,
            keep_cache: false,
            usage_only: false,
            partial_from_cache: false,
            dry_run: true,
        };
        let cache = CacheStore::from_args(&input, &common)?;
        let mut translator = Translator::new(common, cache)?;
        let stats = translate_book(&book, 1..=2, &mut translator, Mode::Write, false)?;
        assert_eq!(stats.pages_translated, 2);
        assert_eq!(stats.blocks_translated, 3);
        update_opf_metadata(&book.opf_path, &translator.backend.model)?;
        pack_epub(book.work_dir.path(), &output)?;

        let repacked = unpack_epub(&output)?;
        assert_eq!(repacked.spine.len(), 2);
        Ok(())
    }

    #[test]
    fn placeholder_tokens_keep_marker_kinds() {
        let tokens = tokenize_placeholders("A ⟦E1⟧B⟦/E1⟧ ⟦S2⟧");
        assert!(matches!(tokens[1], Token::Open(1)));
        assert!(matches!(tokens[3], Token::Close(1)));
        assert!(matches!(tokens[5], Token::SelfClose(2)));
    }

    #[test]
    fn inline_restore_failure_keeps_original_link_markup() -> Result<()> {
        let source = br##"<p xmlns:epub="http://www.idpf.org/2007/ops"><a href="chapter.xhtml#note" epub:type="noteref">1</a> Footnote text.</p>"##;
        let mut reader = Reader::from_reader(Cursor::new(source));
        reader.config_mut().trim_text(false);
        let mut buf = Vec::new();
        let inner = loop {
            match reader.read_event_into(&mut buf)? {
                Event::Start(e) if local_name(e.name().as_ref()) == b"p" => {
                    let end_name = e.name().as_ref().to_vec();
                    break collect_element_inner(&mut reader, &end_name)?;
                }
                Event::Eof => bail!("missing test paragraph"),
                _ => {}
            }
            buf.clear();
        };
        let (_, inline_map) = encode_inline(&inner)?;
        let (restored, used_translation) =
            restore_inline_or_original("脚注本文だけでプレースホルダなし。", &inline_map, &inner);
        assert!(!used_translation);

        let mut writer = Writer::new(Vec::new());
        write_events(&mut writer, &restored)?;
        let restored_text = String::from_utf8(writer.into_inner())?;
        assert!(restored_text.contains("href=\"chapter.xhtml#note\""));
        assert!(restored_text.contains("epub:type=\"noteref\""));
        Ok(())
    }

    #[test]
    fn inline_restore_keeps_citation_markup_in_localized_bibliography() -> Result<()> {
        let source = br##"<li>Beston, H. B., <cite>The Firelight Fairy Book</cite>.</li>"##;
        let mut reader = Reader::from_reader(Cursor::new(source));
        reader.config_mut().trim_text(false);
        let mut buf = Vec::new();
        let inner = loop {
            match reader.read_event_into(&mut buf)? {
                Event::Start(e) if local_name(e.name().as_ref()) == b"li" => {
                    let end_name = e.name().as_ref().to_vec();
                    break collect_element_inner(&mut reader, &end_name)?;
                }
                Event::Eof => bail!("missing test list item"),
                _ => {}
            }
            buf.clear();
        };
        let (_, inline_map) = encode_inline(&inner)?;
        let restored = try_restore_inline(
            "Beston, H. B.\u{3001}⟦E1⟧The Firelight Fairy Book⟦/E1⟧。",
            &inline_map,
        )?;

        let mut writer = Writer::new(Vec::new());
        write_events(&mut writer, &restored)?;
        let restored_text = String::from_utf8(writer.into_inner())?;
        assert_eq!(
            restored_text,
            "Beston, H. B.、<cite>The Firelight Fairy Book</cite>。"
        );
        Ok(())
    }

    #[test]
    fn aside_is_translatable_block() {
        assert!(is_block_tag(QName(b"aside")));
    }

    #[test]
    fn reads_epub3_nav_toc() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let input = dir.path().join("minimal.epub");
        write_minimal_epub(&input)?;
        let book = unpack_epub(&input)?;
        let nav = find_nav_item(&book.manifest).context("missing nav item")?;
        let entries = read_nav_toc(&nav.abs_path)?;
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].label, "Chapter One");
        assert_eq!(entries[0].href.as_deref(), Some("chapter1.xhtml"));
        assert_eq!(entries[1].label, "Chapter Two");
        Ok(())
    }

    #[test]
    fn openai_usage_is_extracted_from_response() {
        let value = serde_json::json!({
            "usage": {
                "input_tokens": 120,
                "output_tokens": 45,
                "total_tokens": 165
            }
        });
        let usage = usage_from_openai_value(&value);
        assert_eq!(usage.requests, 1);
        assert_eq!(usage.input_tokens, 120);
        assert_eq!(usage.output_tokens, 45);
        assert_eq!(usage.total_tokens, 165);
    }

    #[test]
    fn claude_usage_is_extracted_from_response() {
        let response = ClaudeResponse {
            content: Vec::new(),
            usage: Some(ClaudeUsage {
                input_tokens: Some(80),
                output_tokens: Some(30),
            }),
        };
        let usage = usage_from_claude_response(&response);
        assert_eq!(usage.requests, 1);
        assert_eq!(usage.input_tokens, 80);
        assert_eq!(usage.output_tokens, 30);
        assert_eq!(usage.total_tokens, 110);
    }

    #[test]
    fn glossary_subset_is_injected_into_prompt() {
        let entries = vec![
            GlossaryEntry {
                src: "Horizon".to_string(),
                dst: "ホライゾン".to_string(),
                kind: Some("system".to_string()),
                note: None,
            },
            GlossaryEntry {
                src: "Unused".to_string(),
                dst: "未使用".to_string(),
                kind: None,
                note: None,
            },
        ];
        let subset = entries
            .iter()
            .filter(|entry| "Horizon failed.".contains(&entry.src))
            .cloned()
            .collect::<Vec<_>>();
        let prompt = user_prompt("Horizon failed.", &subset);
        assert!(prompt.contains("Horizon => ホライゾン (system)"));
        assert!(!prompt.contains("Unused"));
        assert!(prompt.contains("<source>\nHorizon failed.\n</source>"));
    }

    #[test]
    fn retry_prompt_keeps_source_and_adds_validation_context() {
        let entries = vec![GlossaryEntry {
            src: "Horizon".to_string(),
            dst: "ホライゾン".to_string(),
            kind: Some("system".to_string()),
            note: None,
        }];
        let prompt = retry_user_prompt(
            "Horizon failed.",
            &entries,
            "Horizon failed.",
            "translation validation failed: provider returned the source text unchanged",
        );
        assert!(prompt.contains("Horizon => ホライゾン (system)"));
        assert!(prompt.contains("<source>\nHorizon failed.\n</source>"));
        assert!(prompt.contains("<retry_instruction>"));
        assert!(prompt.contains("source text unchanged"));
        assert!(prompt.contains("前回の応答:\nHorizon failed."));
    }

    #[test]
    fn long_translation_chunks_split_on_sentence_boundaries() {
        let source = "First sentence. Second sentence is a little longer. Third sentence.";
        let chunks = split_translation_chunks(source, 35);
        assert_eq!(
            chunks,
            vec![
                "First sentence.",
                "Second sentence is a little longer.",
                "Third sentence."
            ]
        );
    }

    #[test]
    fn long_translation_chunks_do_not_split_inside_placeholder() {
        let source =
            "Read ⟦E123456789⟧this very long linked phrase⟦/E123456789⟧. Continue after it.";
        let chunks = split_translation_chunks(source, 20);
        assert!(
            chunks
                .iter()
                .all(|chunk| chunk.matches('⟦').count() == chunk.matches('⟧').count())
        );
        assert_eq!(
            placeholder_signature(source),
            placeholder_signature(&chunks.join(" "))
        );
    }

    #[test]
    fn adaptive_concurrency_reduces_to_one() {
        let adaptive = AdaptiveConcurrency::new(3);
        assert_eq!(adaptive.current(), 3);
        adaptive.current.store(2, Ordering::Relaxed);
        assert_eq!(adaptive.current(), 2);
        adaptive.current.store(1, Ordering::Relaxed);
        assert_eq!(adaptive.current(), 1);
        adaptive.current.store(0, Ordering::Relaxed);
        assert_eq!(adaptive.current(), 1);
    }

    #[test]
    fn adaptive_concurrency_recovers_after_success_streak() {
        let adaptive = AdaptiveConcurrency::new(3);
        adaptive.current.store(1, Ordering::Relaxed);
        for _ in 0..(ADAPTIVE_CONCURRENCY_SUCCESS_THRESHOLD - 1) {
            adaptive.record_success("test");
        }
        assert_eq!(adaptive.current(), 1);
        adaptive.record_success("test");
        assert_eq!(adaptive.current(), 2);
        for _ in 0..ADAPTIVE_CONCURRENCY_SUCCESS_THRESHOLD {
            adaptive.record_success("test");
        }
        assert_eq!(adaptive.current(), 3);
        for _ in 0..ADAPTIVE_CONCURRENCY_SUCCESS_THRESHOLD {
            adaptive.record_success("test");
        }
        assert_eq!(adaptive.current(), 3);
    }

    #[test]
    fn cache_key_changes_with_glossary() {
        let entry_a = GlossaryEntry {
            src: "Horizon".to_string(),
            dst: "ホライゾン".to_string(),
            kind: None,
            note: None,
        };
        let entry_b = GlossaryEntry {
            src: "Horizon".to_string(),
            dst: "ホライズン".to_string(),
            kind: None,
            note: None,
        };
        let key_a = cache_key(
            Provider::Ollama,
            "qwen3:14b",
            "essay",
            "Horizon",
            &[entry_a],
        );
        let key_b = cache_key(
            Provider::Ollama,
            "qwen3:14b",
            "essay",
            "Horizon",
            &[entry_b],
        );
        assert_ne!(key_a, key_b);
    }

    #[test]
    fn translation_validation_accepts_japanese_translation() -> Result<()> {
        validate_translation_response(
            "The quick brown fox jumps over the lazy dog.",
            "すばやい茶色の狐が怠け者の犬を飛び越える。",
        )
    }

    #[test]
    fn translation_validation_rejects_unchanged_source() {
        let err = validate_translation_response(
            "The quick brown fox jumps over the lazy dog.",
            "The quick brown fox jumps over the lazy dog.",
        )
        .unwrap_err();
        assert!(err.to_string().contains("unchanged"));
    }

    #[test]
    fn translation_validation_rejects_prompt_wrapper_leak() {
        let err = validate_translation_response(
            "The quick brown fox jumps over the lazy dog.",
            "<source>\nすばやい茶色の狐。\n</source>",
        )
        .unwrap_err();
        assert!(err.to_string().contains("prompt wrapper"));
    }

    #[test]
    fn translation_validation_rejects_missing_placeholder() {
        let err = validate_translation_response("Read ⟦E1⟧this link⟦/E1⟧.", "このリンクを読む。")
            .unwrap_err();
        assert!(err.to_string().contains("placeholders"));
    }

    #[test]
    fn translation_validation_accepts_reordered_placeholders() -> Result<()> {
        validate_translation_response(
            "Read ⟦E1⟧this link⟦/E1⟧ before ⟦E2⟧that note⟦/E2⟧.",
            "⟦E2⟧その注記⟦/E2⟧の前に⟦E1⟧このリンク⟦/E1⟧を読む。",
        )
    }

    #[test]
    fn translation_validation_rejects_english_response() {
        let err = validate_translation_response(
            "This paragraph should be translated into Japanese.",
            "This paragraph cannot be translated right now.",
        )
        .unwrap_err();
        assert!(err.to_string().contains("Japanese"));
    }

    #[test]
    fn translation_validation_accepts_localized_citation_line() -> Result<()> {
        validate_translation_response(
            "Beston, H. B., ⟦E1⟧The Firelight Fairy Book⟦/E1⟧.",
            "Beston, H. B.、⟦E1⟧The Firelight Fairy Book⟦/E1⟧。",
        )
    }

    #[test]
    fn translation_validation_rejects_long_partial_english_segment() {
        let source = "Wide range of the modern fairy tale. The bibliography will suggest something of the treasures in the field of the modern fanciful story. From the delightful nonsense of Alice in Wonderland and the travelers' tales of Baron Munchausen to the profound seriousness of The King of the Golden River is a far cry.";
        let translated = "近代童話の広い範囲。The bibliography will suggest something of the treasures in the field of the modern fanciful story. From the delightful nonsense of Alice in Wonderland and the travelers' tales of Baron Munchausen to the profound seriousness of The King of the Golden River is a far cry.";

        let err = validate_translation_response(source, translated).unwrap_err();
        assert!(err.to_string().contains("untranslated English segment"));
    }

    #[test]
    fn translation_validation_rejects_medium_partial_english_segment() {
        let source = "She seized the keys of the boxes, and first opened the box of gold. But how great was her terror when she gazed at its contents.";
        let translated =
            "彼女は箱の鍵をつかんだ。But how great was her terror when she gazed at its contents.";

        let err = validate_translation_response(source, translated).unwrap_err();
        assert!(err.to_string().contains("untranslated English segment"));
    }

    #[test]
    fn translation_validation_allows_short_ascii_names_in_japanese_text() -> Result<()> {
        validate_translation_response(
            "Dr. Abram S. Isaacs is a professor in New York University and is also a rabbi.",
            "アブラハム・S・アイザックス博士はNew York Universityの教授であり、またラビである。",
        )
    }

    #[test]
    fn refusal_validation_errors_are_classified_for_fallback() {
        let err = validate_translation_response(
            "This paragraph should be translated into Japanese.",
            "As an AI language model, I cannot translate this passage.",
        )
        .unwrap_err();
        assert!(is_refusal_validation_error(&err));
    }

    #[test]
    fn translation_validation_rejects_likely_truncated_long_translation() {
        let source = "This long paragraph has enough words to look like a complete sentence. It continues with more detail, context, and supporting clauses so that a very short Japanese answer should be suspicious.";
        let err =
            validate_translation_response(source, "この長い段落には十分な語数があり").unwrap_err();
        assert!(err.to_string().contains("truncated"));
    }

    #[test]
    fn cached_english_response_is_not_counted_as_valid_cache_hit() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let cache_root = dir.path().join("cache");
        let input = dir.path().join("book.epub");
        fs::write(&input, b"dummy")?;
        let args = CommonArgs {
            provider: Provider::Ollama,
            model: Some(DEFAULT_MODEL.to_string()),
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
            keep_cache: false,
            usage_only: false,
            partial_from_cache: false,
            dry_run: false,
        };
        let source = "This paragraph should be translated into Japanese.";
        let mut cache = CacheStore::from_args(&input, &args)?;
        let key = cache_key(Provider::Ollama, DEFAULT_MODEL, "essay", source, &[]);
        cache.insert(CacheRecord {
            key,
            translated: source.to_string(),
            provider: "ollama".to_string(),
            model: DEFAULT_MODEL.to_string(),
            at: chrono::Utc::now().to_rfc3339(),
        })?;
        let translator = Translator::new(args, cache)?;

        assert!(!translator.has_cached_translation(source));
        Ok(())
    }

    #[test]
    fn cache_store_roundtrips_jsonl() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let cache_root = dir.path().join("cache");
        let input = dir.path().join("book.epub");
        fs::write(&input, b"dummy")?;
        let args = CommonArgs {
            provider: Provider::Ollama,
            model: Some(DEFAULT_MODEL.to_string()),
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
            cache_root: Some(cache_root.clone()),
            no_cache: false,
            clear_cache: false,
            keep_cache: false,
            usage_only: false,
            partial_from_cache: false,
            dry_run: false,
        };
        let mut cache = CacheStore::from_args(&input, &args)?;
        let dir_path = cache.dir.clone();
        let manifest_path = cache.manifest_path.clone();
        let translations_path = cache.translations_path.clone();
        cache.insert(CacheRecord {
            key: "abc".to_string(),
            translated: "訳文".to_string(),
            provider: "ollama".to_string(),
            model: DEFAULT_MODEL.to_string(),
            at: "2026-04-29T00:00:00Z".to_string(),
        })?;
        assert!(dir_path.starts_with(&cache_root));
        assert!(translations_path.exists());

        let mut loaded = CacheStore::from_args(&input, &args)?;
        assert_eq!(loaded.get("abc").as_deref(), Some("訳文"));
        assert_eq!(loaded.stats.hits, 1);

        let params = ManifestParams {
            provider: "ollama".to_string(),
            model: DEFAULT_MODEL.to_string(),
            prompt_version: PROMPT_VERSION.to_string(),
            style_id: "essay".to_string(),
            glossary_sha: String::new(),
        };
        loaded.upsert_manifest(&input, params, Some(&input))?;
        assert!(manifest_path.exists());

        loaded.finalize_completion()?;
        assert!(!dir_path.exists());
        Ok(())
    }

    #[test]
    fn partial_from_cache_keeps_cache_misses_original() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let cache_root = dir.path().join("cache");
        let input = dir.path().join("book.epub");
        fs::write(&input, b"dummy")?;
        let args = CommonArgs {
            provider: Provider::Ollama,
            model: Some(DEFAULT_MODEL.to_string()),
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
            keep_cache: false,
            usage_only: false,
            partial_from_cache: true,
            dry_run: false,
        };
        let cache = CacheStore::from_args(&input, &args)?;
        let mut translator = Translator::new(args, cache)?;

        match translator
            .translate_many(&["Hello".to_string()], None)?
            .remove(0)
        {
            Translation::Original => {}
            Translation::Translated { text, .. } => bail!("unexpected translation: {text}"),
        }
        assert_eq!(translator.cache.stats.misses, 1);
        assert_eq!(translator.cache.stats.writes, 0);
        Ok(())
    }

    #[test]
    fn cache_dir_uses_input_hash_subdirectory() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let cache_root = dir.path().join("cache");
        let input = dir.path().join("book.epub");
        fs::write(&input, b"sample epub bytes")?;
        let args = CommonArgs {
            provider: Provider::Ollama,
            model: Some(DEFAULT_MODEL.to_string()),
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
            cache_root: Some(cache_root.clone()),
            no_cache: false,
            clear_cache: false,
            keep_cache: false,
            usage_only: false,
            partial_from_cache: false,
            dry_run: false,
        };
        let cache = CacheStore::from_args(&input, &args)?;
        let (_, expected_short) = compute_input_hash(&input)?;
        assert_eq!(cache.input_hash, expected_short);
        assert_eq!(cache.input_hash.len(), 32); // 16 bytes hex
        assert_eq!(cache.dir, cache_root.join(&expected_short));
        Ok(())
    }

    #[test]
    fn finalize_completion_keeps_dir_when_keep_cache_set() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let cache_root = dir.path().join("cache");
        let input = dir.path().join("book.epub");
        fs::write(&input, b"another sample")?;
        let args = CommonArgs {
            provider: Provider::Ollama,
            model: Some(DEFAULT_MODEL.to_string()),
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
        };
        let cache = CacheStore::from_args(&input, &args)?;
        let dir_path = cache.dir.clone();
        cache.finalize_completion()?;
        assert!(dir_path.exists());
        Ok(())
    }

    #[test]
    fn usage_only_estimates_without_api_key() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let input = dir.path().join("minimal.epub");
        write_minimal_epub(&input)?;
        let book = unpack_epub(&input)?;
        let args = CommonArgs {
            provider: Provider::Openai,
            model: Some(DEFAULT_OPENAI_MODEL.to_string()),
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
            cache_root: Some(dir.path().join("cache")),
            no_cache: false,
            clear_cache: false,
            keep_cache: false,
            usage_only: true,
            partial_from_cache: false,
            dry_run: false,
        };
        let cache = CacheStore::from_args(&input, &args)?;
        let translator = Translator::new(args, cache)?;
        let estimate = estimate_usage(&book, 1..=2, &translator)?;

        assert_eq!(estimate.pages_selected, 2);
        assert_eq!(estimate.blocks_total, 3);
        assert_eq!(estimate.cached_blocks, 0);
        assert_eq!(estimate.uncached_blocks, 3);
        assert_eq!(estimate.estimated_requests, 3);
        assert!(estimate.estimated_total_tokens() > 0);
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
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#,
        )?;
        zip.start_file("OEBPS/content.opf", deflated)?;
        zip.write_all(
            br#"<?xml version="1.0" encoding="UTF-8"?>
<package version="3.0" unique-identifier="bookid" xmlns="http://www.idpf.org/2007/opf">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="bookid">minimal</dc:identifier>
    <dc:title>Minimal</dc:title>
    <dc:language>en</dc:language>
  </metadata>
  <manifest>
    <item id="c1" href="chapter1.xhtml" media-type="application/xhtml+xml"/>
    <item id="c2" href="chapter2.xhtml" media-type="application/xhtml+xml"/>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
  </manifest>
  <spine>
    <itemref idref="c1"/>
    <itemref idref="c2"/>
  </spine>
</package>"#,
        )?;
        zip.start_file("OEBPS/chapter1.xhtml", deflated)?;
        zip.write_all(
            br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><body>
  <h1>Chapter One</h1>
  <p>This is <em>very</em> important.<br /></p>
</body></html>"#,
        )?;
        zip.start_file("OEBPS/chapter2.xhtml", deflated)?;
        zip.write_all(
            br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><body>
  <p>Second page.</p>
</body></html>"#,
        )?;
        zip.start_file("OEBPS/nav.xhtml", deflated)?;
        zip.write_all(
            br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
  <body>
    <nav epub:type="toc">
      <ol>
        <li><a href="chapter1.xhtml">Chapter One</a></li>
        <li><a href="chapter2.xhtml">Chapter Two</a></li>
      </ol>
    </nav>
  </body>
</html>"#,
        )?;
        zip.finish()?;
        Ok(())
    }
}
