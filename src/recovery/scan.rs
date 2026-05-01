use std::{fs, io::Cursor};

use anyhow::{Context, Result, bail};
use quick_xml::{Reader, events::Event};

use super::{UntranslatedReport, recover_command};
use crate::{
    cache::CacheStore,
    config::{RecoverArgs, ScanRecoveryArgs},
    epub::{SpineItem, is_block_tag, unpack_epub},
    translator::{Translator, validate_translation_response},
    xhtml::{collect_element_inner, encode_inline},
};

pub(crate) fn scan_recovery_command(args: ScanRecoveryArgs) -> Result<()> {
    let input_book = unpack_epub(&args.input)?;
    let output_book = unpack_epub(&args.output)?;
    if input_book.spine.len() != output_book.spine.len() {
        bail!(
            "spine count differs: input has {}, output has {}",
            input_book.spine.len(),
            output_book.spine.len()
        );
    }

    let mut common = args.common.clone();
    common.no_cache = false;
    common.clear_cache = false;
    common.keep_cache = true;
    common.usage_only = false;
    common.partial_from_cache = false;
    let cache = CacheStore::from_args(&args.input, &common)?;
    let translator = Translator::new(common, cache)?;
    let mut report = UntranslatedReport::for_output(
        &args.input,
        &args.output,
        &translator.cache.root,
        &translator.cache.dir,
    );

    let mut scanned = 0usize;
    let mut suspicious = 0usize;
    'pages: for (idx, (source_item, output_item)) in input_book
        .spine
        .iter()
        .zip(output_book.spine.iter())
        .enumerate()
    {
        let page_no = idx + 1;
        let source_blocks = collect_blocks(source_item)?;
        let output_blocks = collect_blocks(output_item)?;
        if source_blocks.len() != output_blocks.len() {
            bail!(
                "block count differs on spine page {}: input {} has {}, output {} has {}",
                page_no,
                source_item.href,
                source_blocks.len(),
                output_item.href,
                output_blocks.len()
            );
        }
        for (block_idx, (source, output)) in
            source_blocks.iter().zip(output_blocks.iter()).enumerate()
        {
            scanned += 1;
            let Some((reason, error)) = suspicious_output(source, output) else {
                continue;
            };
            let record = report.recovery_record(
                &translator,
                reason,
                page_no,
                block_idx + 1,
                &source_item.href,
                source,
                Some(error.as_str()),
            );
            eprintln!(
                "recoverable error: p{} b{} {} detected suspicious output ({})",
                record.page_no, record.block_index, record.href, record.reason
            );
            report.record(&record)?;
            suspicious += 1;
            if args.limit.is_some_and(|limit| suspicious >= limit) {
                break 'pages;
            }
        }
    }

    println!("Recovery scan completed");
    println!("input: {}", args.input.display());
    println!("output: {}", args.output.display());
    println!("blocks scanned: {scanned}");
    println!("suspicious blocks: {suspicious}");
    if let Some(summary) = report.finish()? {
        println!("Untranslated report: {}", summary.path.display());
        println!("Recovery log: {}", summary.recovery_path.display());
        if args.recover {
            println!("Recovering scanned blocks...");
            recover_command(recover_args_for_scan(&args, summary.recovery_path))?;
        } else {
            println!(
                "next: epubicus recover {} --rebuild",
                summary.recovery_path.display()
            );
            return Err(crate::recoverable_error(format!(
                "recovery scan found {} suspicious block(s); recovery log: {}",
                summary.count,
                summary.recovery_path.display()
            )));
        }
    } else {
        println!("No suspicious untranslated blocks found.");
    }
    Ok(())
}

fn recover_args_for_scan(args: &ScanRecoveryArgs, log: std::path::PathBuf) -> RecoverArgs {
    RecoverArgs {
        log: Some(log),
        cache_target: None,
        input: Some(args.input.clone()),
        limit: None,
        list: false,
        page: None,
        block: None,
        reasons: Vec::new(),
        failed_log: args.failed_log.clone(),
        rebuild: args.rebuild,
        output: args.rebuild.then(|| args.output.clone()),
        common: args.common.clone(),
    }
}

fn collect_blocks(item: &SpineItem) -> Result<Vec<String>> {
    let source =
        fs::read(&item.abs_path).with_context(|| format!("failed to read {}", item.href))?;
    collect_blocks_from_bytes(&source)
}

fn collect_blocks_from_bytes(source: &[u8]) -> Result<Vec<String>> {
    let mut reader = Reader::from_reader(Cursor::new(source));
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut blocks = Vec::new();
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) if is_block_tag(e.name()) => {
                let end_name = e.name().as_ref().to_vec();
                let inner = collect_element_inner(&mut reader, &end_name)?;
                let (text, _) = encode_inline(&inner)?;
                if !text.trim().is_empty() {
                    blocks.push(text);
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(blocks)
}

fn suspicious_output<'a>(source: &'a str, output: &'a str) -> Option<(&'static str, String)> {
    if crate::collapse_ws(source) == crate::collapse_ws(output) {
        return Some((
            "unchanged_source",
            "output block is unchanged source text".to_string(),
        ));
    }
    match validate_translation_response(source, output) {
        Ok(()) => None,
        Err(err) => Some(("detected_untranslated_output", err.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        CommonArgs, DEFAULT_CLAUDE_BASE_URL, DEFAULT_CONCURRENCY, DEFAULT_MAX_CHARS_PER_REQUEST,
        DEFAULT_MODEL, DEFAULT_OLLAMA_HOST, DEFAULT_OPENAI_BASE_URL, Provider,
    };

    #[test]
    fn scan_blocks_collects_translatable_text() -> Result<()> {
        let blocks = collect_blocks_from_bytes(
            br#"<html xmlns="http://www.w3.org/1999/xhtml"><body><h1>Title</h1><p>Hello <em>world</em>.</p></body></html>"#,
        )?;

        assert_eq!(blocks, vec!["Title", "Hello ⟦E1⟧world⟦/E1⟧."]);
        Ok(())
    }

    #[test]
    fn suspicious_output_detects_unchanged_source() {
        let result = suspicious_output("Hello world.", "Hello world.").unwrap();

        assert_eq!(result.0, "unchanged_source");
    }

    #[test]
    fn suspicious_output_accepts_japanese_translation() {
        assert!(suspicious_output("Hello world.", "こんにちは世界。").is_none());
    }

    #[test]
    fn scan_recover_args_rebuild_to_inspected_output() {
        let args = ScanRecoveryArgs {
            input: "book.epub".into(),
            output: "book_jp.epub".into(),
            limit: Some(1),
            recover: true,
            rebuild: true,
            failed_log: Some("failed.jsonl".into()),
            common: test_common_args(),
        };

        let recover_args = recover_args_for_scan(&args, "recovery.jsonl".into());

        assert_eq!(
            recover_args.log,
            Some(std::path::PathBuf::from("recovery.jsonl"))
        );
        assert_eq!(
            recover_args.input,
            Some(std::path::PathBuf::from("book.epub"))
        );
        assert!(recover_args.rebuild);
        assert_eq!(
            recover_args.output,
            Some(std::path::PathBuf::from("book_jp.epub"))
        );
        assert_eq!(
            recover_args.failed_log,
            Some(std::path::PathBuf::from("failed.jsonl"))
        );
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
}
