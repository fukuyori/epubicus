use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

use super::log::{RecoveryRecord, hash_text, read_recovery_records, write_recovery_records};
use crate::{
    cache::{CacheStore, newest_recovery_log_for_target},
    config::{Provider, RecoverArgs, TranslateArgs},
    translate_command,
    translator::{Translator, is_provider_auth_error},
};

pub(crate) fn recover_command(args: RecoverArgs) -> Result<()> {
    let started = Instant::now();
    let log_path = resolve_recovery_log_path(&args)?;
    let records = read_recovery_records(&log_path)?;
    if records.is_empty() {
        bail!("recovery log is empty: {}", log_path.display());
    }
    let selected_records = select_records(&records, &args);
    if selected_records.is_empty() {
        println!("No recovery log items match the requested filters.");
        return Ok(());
    }
    if args.list {
        print_recovery_list(&log_path, &selected_records);
        return Ok(());
    }

    let input = args
        .input
        .clone()
        .or_else(|| first_non_empty_path(records[0].input_epub.as_str()))
        .with_context(|| {
            format!(
                "recovery log {} does not record an input EPUB; pass --input",
                log_path.display()
            )
        })?;

    let mut common = args.common.clone();
    if common.cache_root.is_none() {
        common.cache_root = first_non_empty_path(records[0].cache_root.as_str());
    }
    apply_record_defaults(&mut common, &records[0], provider_was_explicit());
    common.no_cache = false;
    common.clear_cache = false;
    common.keep_cache = true;
    common.usage_only = false;
    common.partial_from_cache = false;
    let rebuild_args = if args.rebuild {
        Some(rebuild_translate_args(
            &args,
            &log_path,
            &records[0],
            &input,
            &common,
        )?)
    } else {
        None
    };

    let cache = CacheStore::from_args(&input, &common)?;
    let mut translator = Translator::new(common, cache)?;
    translator.cache.begin_manifest_run()?;
    let failed_log = args
        .failed_log
        .clone()
        .unwrap_or_else(|| default_failed_log_path(&log_path));

    let mut total = 0usize;
    let mut already_cached = 0usize;
    let mut recovered = 0usize;
    let mut unrecoverable = Vec::new();

    for record in selected_records {
        total += 1;
        if record.source_hash != hash_text(&record.source_text) {
            let mut failed = record.clone();
            failed.error = Some("source_hash does not match source_text".to_string());
            print_unrecoverable(&failed);
            unrecoverable.push(failed);
            translator.cache.heartbeat_manifest_run()?;
            continue;
        }

        let replace_existing = record.reason == "inline_restore_failed";
        if !replace_existing && translator.cache.peek(&record.cache_key).is_some() {
            already_cached += 1;
            translator.cache.heartbeat_manifest_run()?;
            continue;
        }

        match translator.translate_uncached_source(&record.source_text) {
            Ok((translated, provider, model, fallback_used)) => {
                translator.insert_cache_translation(
                    record.cache_key.clone(),
                    translated,
                    provider,
                    model,
                    replace_existing,
                )?;
                if fallback_used {
                    translator.fallback_count += 1;
                }
                recovered += 1;
            }
            Err(err) => {
                if is_provider_auth_error(&err) {
                    let _ = translator.cache.finish_manifest_run();
                    return Err(err).context(format!(
                        "recovery aborted after provider authentication/configuration failure at p{} b{} {}",
                        record.page_no, record.block_index, record.href
                    ));
                }
                let mut failed = record.clone();
                failed.error = Some(format!("{err:#}"));
                print_unrecoverable(&failed);
                unrecoverable.push(failed);
            }
        }
        translator.cache.heartbeat_manifest_run()?;
    }

    if !unrecoverable.is_empty() {
        write_recovery_records(&failed_log, &unrecoverable)?;
    }
    let total_elapsed_secs = translator
        .cache
        .finish_manifest_run()?
        .unwrap_or_else(|| started.elapsed().as_secs());

    println!("Recovery completed");
    println!("input: {}", input.display());
    println!("cache: {}", translator.cache.dir.display());
    println!("items: {total}");
    println!("already cached: {already_cached}");
    println!("recovered: {recovered}");
    println!("unrecoverable: {}", unrecoverable.len());
    println!(
        "elapsed: {} | total active: {}",
        format_duration_hms(started.elapsed()),
        format_duration_hms(Duration::from_secs(total_elapsed_secs))
    );
    if !unrecoverable.is_empty() {
        println!("failed log: {}", failed_log.display());
        println!(
            "next: inspect failed items, try another provider/model, or keep original text intentionally"
        );
        if args.rebuild {
            println!("rebuild skipped because unrecoverable items remain");
        }
        return Err(crate::recoverable_error(format!(
            "recovery left {} unrecoverable item(s); failed log: {}",
            unrecoverable.len(),
            failed_log.display()
        )));
    } else if !args.rebuild {
        if let Some(output) = first_non_empty_path(records[0].output_epub.as_str()) {
            println!(
                "next: rebuild from cache, for example: epubicus translate {} --cache-root {} --partial-from-cache --output {}",
                input.display(),
                translator
                    .cache
                    .dir
                    .parent()
                    .unwrap_or(&translator.cache.dir)
                    .display(),
                output.display()
            );
        }
    }

    if let Some(rebuild_args) = rebuild_args {
        if unrecoverable.is_empty() {
            drop(translator);
            println!("Rebuilding EPUB from recovered cache...");
            translate_command(rebuild_args)?;
        }
    }

    Ok(())
}

fn format_duration_hms(duration: Duration) -> String {
    let total = duration.as_secs();
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let seconds = total % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

fn first_non_empty_path(value: &str) -> Option<PathBuf> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| PathBuf::from(trimmed))
}

fn resolve_recovery_log_path(args: &RecoverArgs) -> Result<PathBuf> {
    if let Some(log) = &args.log {
        return Ok(log.clone());
    }
    if let Some(target) = &args.cache_target {
        return newest_recovery_log_for_target(args.common.cache_root.as_deref(), target);
    }
    bail!("pass a recovery LOG path or --cache <hash-or-input.epub>")
}

fn default_failed_log_path(path: &Path) -> PathBuf {
    path.parent()
        .unwrap_or_else(|| Path::new("."))
        .join("failed.jsonl")
}

fn rebuild_translate_args(
    args: &RecoverArgs,
    log_path: &Path,
    record: &RecoveryRecord,
    input: &Path,
    common: &crate::config::CommonArgs,
) -> Result<TranslateArgs> {
    let output = args
        .output
        .clone()
        .or_else(|| first_non_empty_path(record.output_epub.as_str()))
        .with_context(|| {
            format!(
                "recovery log {} does not record an output EPUB; pass --output with --rebuild",
                log_path.display()
            )
        })?;
    let mut common = common.clone();
    common.no_cache = false;
    common.clear_cache = false;
    common.keep_cache = true;
    common.usage_only = false;
    common.partial_from_cache = true;
    Ok(TranslateArgs {
        input: input.to_path_buf(),
        output: Some(output),
        from: None,
        to: None,
        common,
    })
}

fn select_records<'a>(
    records: &'a [RecoveryRecord],
    args: &RecoverArgs,
) -> Vec<&'a RecoveryRecord> {
    records
        .iter()
        .filter(|record| record_matches_filters(record, args))
        .take(args.limit.unwrap_or(usize::MAX))
        .collect()
}

fn record_matches_filters(record: &RecoveryRecord, args: &RecoverArgs) -> bool {
    if args.page.is_some_and(|page| record.page_no != page) {
        return false;
    }
    if args.block.is_some_and(|block| record.block_index != block) {
        return false;
    }
    if !args.reasons.is_empty()
        && !args
            .reasons
            .iter()
            .any(|reason| record.reason == reason.as_str())
    {
        return false;
    }
    true
}

fn print_recovery_list(path: &Path, records: &[&RecoveryRecord]) {
    println!("Recovery log: {}", path.display());
    println!("items: {}", records.len());
    println!();
    for record in records {
        println!(
            "p{} b{} {} ({})",
            record.page_no, record.block_index, record.href, record.reason
        );
        println!("  source_hash: {}", record.source_hash);
        println!("  cache_key: {}", record.cache_key);
        println!("  action: {}", record.suggested_action);
        if let Some(error) = &record.error {
            println!("  error: {error}");
        }
        println!("  source: {}", preview_text(&record.source_text, 120));
    }
}

fn preview_text(text: &str, max_chars: usize) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= max_chars {
        return normalized;
    }
    let end = normalized
        .char_indices()
        .nth(max_chars)
        .map(|(idx, _)| idx)
        .unwrap_or(normalized.len());
    format!("{}...", &normalized[..end])
}

fn apply_record_defaults(
    common: &mut crate::config::CommonArgs,
    record: &RecoveryRecord,
    provider_explicit: bool,
) {
    if common.model.is_none() && !record.model.trim().is_empty() {
        common.model = Some(record.model.clone());
    }
    if !provider_explicit {
        common.provider = match record.provider.as_str() {
            "openai" => Provider::Openai,
            "claude" => Provider::Claude,
            "ollama" => Provider::Ollama,
            _ => common.provider,
        };
    }
    if common.style == "essay" && !record.style.trim().is_empty() {
        common.style = record.style.clone();
    }
}

fn provider_was_explicit() -> bool {
    std::env::var_os("EPUBICUS_PROVIDER").is_some()
        || std::env::args()
            .any(|arg| arg == "--provider" || arg == "-p" || arg.starts_with("--provider="))
}

fn print_unrecoverable(record: &RecoveryRecord) {
    eprintln!(
        "unrecoverable item: p{} b{} {} ({})",
        record.page_no, record.block_index, record.href, record.reason
    );
    if let Some(error) = &record.error {
        eprintln!("  last error: {error}");
    }
    eprintln!("  action: {}", record.suggested_action);
    eprintln!("  source_hash: {}", record.source_hash);
    eprintln!("  cache_key: {}", record.cache_key);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        CommonArgs, DEFAULT_CLAUDE_BASE_URL, DEFAULT_CONCURRENCY, DEFAULT_MAX_CHARS_PER_REQUEST,
        DEFAULT_MODEL, DEFAULT_OLLAMA_HOST, DEFAULT_OPENAI_BASE_URL,
    };

    #[test]
    fn failed_log_defaults_next_to_recovery_log() {
        assert_eq!(
            default_failed_log_path(Path::new("out/book_jp.recovery/recovery.jsonl")),
            PathBuf::from("out/book_jp.recovery/failed.jsonl")
        );
    }

    fn test_args() -> RecoverArgs {
        RecoverArgs {
            log: Some(PathBuf::from("recovery.jsonl")),
            cache_target: None,
            input: None,
            limit: None,
            list: false,
            page: None,
            block: None,
            reasons: Vec::new(),
            failed_log: None,
            rebuild: false,
            output: None,
            common: test_common_args(),
        }
    }

    fn test_common_args() -> CommonArgs {
        CommonArgs {
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
            dry_run: false,
            glossary: None,
            cache_root: None,
            no_cache: false,
            clear_cache: false,
            keep_cache: true,
            usage_only: false,
            partial_from_cache: false,
            passthrough_on_validation_failure: false,
            verbose: false,
        }
    }

    fn test_record(page_no: usize, block_index: usize, reason: &str) -> RecoveryRecord {
        RecoveryRecord {
            kind: "recoverable_error".to_string(),
            reason: reason.to_string(),
            input_epub: "book.epub".to_string(),
            output_epub: "book_jp.epub".to_string(),
            cache_root: ".cache".to_string(),
            provider: "ollama".to_string(),
            model: "qwen3:14b".to_string(),
            style: "essay".to_string(),
            page_no,
            block_index,
            href: "chapter.xhtml".to_string(),
            cache_key: "key".to_string(),
            source_hash: "hash".to_string(),
            source_text: "source text".to_string(),
            error: None,
            suggested_action: "retry_translation".to_string(),
            at: "2026-05-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn recovery_filters_select_page_block_reason_and_limit() {
        let records = vec![
            test_record(1, 1, "cache_miss"),
            test_record(1, 2, "inline_restore_failed"),
            test_record(2, 1, "cache_miss"),
        ];
        let mut args = test_args();
        args.page = Some(1);
        args.reasons = vec!["cache_miss".to_string()];
        args.limit = Some(1);

        let selected = select_records(&records, &args);

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].page_no, 1);
        assert_eq!(selected[0].block_index, 1);
    }

    #[test]
    fn preview_text_truncates_without_newlines() {
        assert_eq!(preview_text("a\n b\tc", 20), "a b c");
        assert_eq!(preview_text("abcdef", 3), "abc...");
    }

    #[test]
    fn rebuild_args_use_recovery_output_and_partial_cache_mode() -> Result<()> {
        let args = test_args();
        let common = test_common_args();
        let record = test_record(1, 1, "cache_miss");

        let translate_args = rebuild_translate_args(
            &args,
            Path::new("recovery.jsonl"),
            &record,
            Path::new("book.epub"),
            &common,
        )?;

        assert_eq!(translate_args.input, PathBuf::from("book.epub"));
        assert_eq!(translate_args.output, Some(PathBuf::from("book_jp.epub")));
        assert!(translate_args.common.partial_from_cache);
        assert!(translate_args.common.keep_cache);
        assert!(!translate_args.common.clear_cache);
        assert!(!translate_args.common.no_cache);
        Ok(())
    }

    #[test]
    fn rebuild_output_option_overrides_recovery_output() -> Result<()> {
        let mut args = test_args();
        args.output = Some(PathBuf::from("fixed.epub"));
        let common = test_common_args();
        let record = test_record(1, 1, "cache_miss");

        let translate_args = rebuild_translate_args(
            &args,
            Path::new("recovery.jsonl"),
            &record,
            Path::new("book.epub"),
            &common,
        )?;

        assert_eq!(translate_args.output, Some(PathBuf::from("fixed.epub")));
        Ok(())
    }
}
