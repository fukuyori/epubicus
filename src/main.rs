#[cfg(test)]
use std::fs::File;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs,
    io::{Cursor, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use clap::Parser;
#[cfg(test)]
use quick_xml::name::QName;
use quick_xml::{
    Reader, Writer,
    events::{BytesEnd, BytesStart, BytesText, Event},
};
use reqwest::blocking::Client;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
#[cfg(test)]
#[cfg(test)]
use zip::{
    CompressionMethod, ZipWriter,
    write::{FileOptions, SimpleFileOptions},
};

mod cache;
mod config;
mod epub;
mod glossary;
mod progress;
mod prompt;
mod usage;

#[cfg(test)]
use cache::compute_input_hash;
use cache::{CacheRecord, CacheStore, ManifestParams, cache_command, glossary_sha};
use config::*;
#[cfg(test)]
use epub::local_name;
use epub::{
    EpubBook, count_xhtml_blocks, find_nav_item, find_ncx_item, is_block_tag, pack_epub,
    print_toc_entries, read_nav_toc, read_ncx_toc, unpack_epub, update_opf_metadata,
};
use glossary::{GlossaryEntry, glossary_command, load_glossary};
use progress::ProgressReporter;
use prompt::{retry_user_prompt, system_prompt, user_prompt};
#[cfg(test)]
use usage::ClaudeUsage;
use usage::{
    ApiUsage, ClaudeResponse, OllamaResponse, usage_from_claude_response,
    usage_from_ollama_response, usage_from_openai_value,
};

const ADAPTIVE_CONCURRENCY_SUCCESS_THRESHOLD: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Write,
    Stdout,
}

#[derive(Debug, Default)]
struct Stats {
    pages_seen: usize,
    pages_translated: usize,
    blocks_translated: usize,
}

#[derive(Debug, Clone, Default)]
struct InlineEntry {
    start: Option<Vec<u8>>,
    end: Option<Vec<u8>>,
    empty: Option<Vec<u8>>,
}

#[derive(Debug, Default)]
struct InlineMap {
    entries: HashMap<u32, InlineEntry>,
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
    }
}

fn translate_command(args: TranslateArgs) -> Result<()> {
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
    if translator.fallback_count > 0 {
        eprintln!("Fallback translations: {}", translator.fallback_count);
    }
    Ok(())
}

fn test_command(args: TestArgs) -> Result<()> {
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

enum XhtmlPart {
    Event(Event<'static>),
    EmptyBlock {
        start: BytesStart<'static>,
        end_name: Vec<u8>,
        inner: Vec<Event<'static>>,
    },
    TranslatableBlock {
        start: BytesStart<'static>,
        end_name: Vec<u8>,
        inner: Vec<Event<'static>>,
        inline_map: InlineMap,
        translation_index: usize,
    },
}

fn translate_xhtml_file(
    path: &Path,
    translator: &mut Translator,
    mode: Mode,
    mut progress: Option<&mut ProgressReporter>,
) -> Result<usize> {
    let source = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut reader = Reader::from_reader(Cursor::new(source));
    reader.config_mut().trim_text(false);
    let mut writer = Writer::new(Vec::new());
    let mut buf = Vec::new();
    let mut blocks = 0usize;
    let mut parts = Vec::new();
    let mut sources = Vec::new();

    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) if is_block_tag(e.name()) => {
                let start = e.into_owned();
                let end_name = start.name().as_ref().to_vec();
                let inner = collect_element_inner(&mut reader, &end_name)?;
                let (source_text, inline_map) = encode_inline(&inner)?;
                if source_text.trim().is_empty() {
                    parts.push(XhtmlPart::EmptyBlock {
                        start,
                        end_name,
                        inner,
                    });
                } else {
                    let translation_index = sources.len();
                    sources.push(source_text);
                    parts.push(XhtmlPart::TranslatableBlock {
                        start,
                        end_name,
                        inner,
                        inline_map,
                        translation_index,
                    });
                }
            }
            Event::Eof => break,
            event => {
                parts.push(XhtmlPart::Event(event.into_owned()));
            }
        }
        buf.clear();
    }

    let translations = translator.translate_many(&sources, progress.as_deref_mut())?;
    for part in parts {
        match part {
            XhtmlPart::Event(event) => {
                if mode == Mode::Write {
                    writer.write_event(event)?;
                }
            }
            XhtmlPart::EmptyBlock {
                start,
                end_name,
                inner,
            } => {
                if mode == Mode::Write {
                    writer.write_event(Event::Start(start))?;
                    write_events(&mut writer, &inner)?;
                    writer.write_event(Event::End(BytesEnd::new(String::from_utf8_lossy(
                        &end_name,
                    ))))?;
                }
            }
            XhtmlPart::TranslatableBlock {
                start,
                end_name,
                inner,
                inline_map,
                translation_index,
            } => match &translations[translation_index] {
                Translation::Translated {
                    text: translated,
                    from_cache,
                } => {
                    if !from_cache && translator.dry_run {
                        if let Some(progress) = progress.as_mut() {
                            progress.inc_model_block();
                        }
                    }
                    if mode == Mode::Stdout {
                        blocks += 1;
                        println!("{}", translated.trim());
                        println!();
                    } else {
                        let (restored, used_translation) =
                            restore_inline_or_original(translated, &inline_map, &inner);
                        if used_translation || translator.dry_run {
                            blocks += 1;
                        }
                        writer.write_event(Event::Start(start))?;
                        write_events(&mut writer, &restored)?;
                        writer.write_event(Event::End(BytesEnd::new(String::from_utf8_lossy(
                            &end_name,
                        ))))?;
                    }
                }
                Translation::Original => {
                    if let Some(progress) = progress.as_mut() {
                        progress.inc_passthrough_block();
                    }
                    if mode == Mode::Stdout {
                        println!("{}", sources[translation_index].trim());
                        println!();
                    } else {
                        writer.write_event(Event::Start(start))?;
                        write_events(&mut writer, &inner)?;
                        writer.write_event(Event::End(BytesEnd::new(String::from_utf8_lossy(
                            &end_name,
                        ))))?;
                    }
                }
            },
        }
    }

    if mode == Mode::Write {
        fs::write(path, writer.into_inner())
            .with_context(|| format!("failed to write {}", path.display()))?;
    }
    Ok(blocks)
}

fn collect_element_inner<R: std::io::BufRead>(
    reader: &mut Reader<R>,
    end_name: &[u8],
) -> Result<Vec<Event<'static>>> {
    let mut buf = Vec::new();
    let mut depth = 0usize;
    let mut events = Vec::new();
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) => {
                depth += 1;
                events.push(Event::Start(e.into_owned()));
            }
            Event::End(e) if depth == 0 && e.name().as_ref() == end_name => break,
            Event::End(e) => {
                depth = depth.saturating_sub(1);
                events.push(Event::End(e.into_owned()));
            }
            Event::Eof => bail!("unexpected EOF while reading XHTML block"),
            event => events.push(event.into_owned()),
        }
        buf.clear();
    }
    Ok(events)
}

fn encode_inline(events: &[Event<'static>]) -> Result<(String, InlineMap)> {
    let mut text = String::new();
    let mut map = InlineMap::default();
    let mut next_id = 1u32;
    let mut stack: Vec<(Vec<u8>, u32)> = Vec::new();
    for event in events {
        match event {
            Event::Text(t) => text.push_str(&t.decode()?),
            Event::CData(t) => text.push_str(&String::from_utf8_lossy(t.as_ref())),
            Event::Start(e) => {
                let id = next_id;
                next_id += 1;
                map.entries.entry(id).or_default().start = Some(serialize_event(event)?);
                stack.push((e.name().as_ref().to_vec(), id));
                text.push_str(&format!("⟦E{id}⟧"));
            }
            Event::End(e) => {
                let id = stack
                    .iter()
                    .rposition(|(name, _)| name.as_slice() == e.name().as_ref())
                    .map(|pos| stack.remove(pos).1)
                    .unwrap_or_else(|| {
                        let id = next_id;
                        next_id += 1;
                        id
                    });
                map.entries.entry(id).or_default().end = Some(serialize_event(event)?);
                text.push_str(&format!("⟦/E{id}⟧"));
            }
            Event::Empty(_) => {
                let id = next_id;
                next_id += 1;
                map.entries.entry(id).or_default().empty = Some(serialize_event(event)?);
                text.push_str(&format!("⟦S{id}⟧"));
            }
            _ => {}
        }
    }
    Ok((collapse_ws(&text), map))
}

fn restore_inline(translated: &str, map: &InlineMap) -> Result<Vec<Event<'static>>> {
    let tokens = tokenize_placeholders(translated);
    validate_placeholder_ids(&tokens, map)?;
    let mut events = Vec::new();
    for token in tokens {
        match token {
            Token::Text(s) if !s.is_empty() => {
                events.push(Event::Text(BytesText::new(&s).into_owned()))
            }
            Token::Text(_) => {}
            Token::Open(id) | Token::Close(id) | Token::SelfClose(id) => {
                let Some(entry) = map.entries.get(&id) else {
                    bail!("unknown placeholder id {id}");
                };
                let bytes = match token {
                    Token::Open(_) => entry.start.as_ref(),
                    Token::Close(_) => entry.end.as_ref(),
                    Token::SelfClose(_) => entry.empty.as_ref(),
                    Token::Text(_) => unreachable!(),
                };
                let Some(bytes) = bytes else {
                    bail!("placeholder kind mismatch for id {id}");
                };
                events.push(parse_single_event(bytes)?);
            }
        }
    }
    Ok(events)
}

fn restore_inline_or_original(
    translated: &str,
    map: &InlineMap,
    original: &[Event<'static>],
) -> (Vec<Event<'static>>, bool) {
    match restore_inline(translated, map) {
        Ok(events) => (events, true),
        Err(_) => (original.to_vec(), false),
    }
}

fn validate_placeholder_ids(tokens: &[Token], map: &InlineMap) -> Result<()> {
    let mut open_seen = HashSet::new();
    let mut close_seen = HashSet::new();
    let mut self_seen = HashSet::new();
    for token in tokens {
        match token {
            Token::Open(id) => {
                open_seen.insert(*id);
            }
            Token::Close(id) => {
                close_seen.insert(*id);
            }
            Token::SelfClose(id) => {
                self_seen.insert(*id);
            }
            Token::Text(_) => {}
        }
    }

    for (id, entry) in &map.entries {
        if entry.empty.is_some() {
            if !self_seen.contains(id) {
                bail!("missing self-closing placeholder S{id}");
            }
        } else {
            if entry.start.is_some() && !open_seen.contains(id) {
                bail!("missing opening placeholder E{id}");
            }
            if entry.end.is_some() && !close_seen.contains(id) {
                bail!("missing closing placeholder /E{id}");
            }
        }
    }
    Ok(())
}

#[derive(Debug)]
enum Token {
    Text(String),
    Open(u32),
    Close(u32),
    SelfClose(u32),
}

fn tokenize_placeholders(s: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut rest = s;
    while let Some(start) = rest.find('⟦') {
        let (before, after_start) = rest.split_at(start);
        tokens.push(Token::Text(before.to_string()));
        if let Some(end) = after_start.find('⟧') {
            let marker = &after_start['⟦'.len_utf8()..end];
            if let Some(num) = marker.strip_prefix("/E").and_then(|n| n.parse().ok()) {
                tokens.push(Token::Close(num));
            } else if let Some(num) = marker.strip_prefix('E').and_then(|n| n.parse().ok()) {
                tokens.push(Token::Open(num));
            } else if let Some(num) = marker.strip_prefix('S').and_then(|n| n.parse().ok()) {
                tokens.push(Token::SelfClose(num));
            } else {
                tokens.push(Token::Text(after_start[..end + '⟧'.len_utf8()].to_string()));
            }
            rest = &after_start[end + '⟧'.len_utf8()..];
        } else {
            tokens.push(Token::Text(after_start.to_string()));
            rest = "";
        }
    }
    if !rest.is_empty() {
        tokens.push(Token::Text(rest.to_string()));
    }
    tokens
}

fn serialize_event(event: &Event<'static>) -> Result<Vec<u8>> {
    let mut writer = Writer::new(Vec::new());
    writer.write_event(event.clone())?;
    Ok(writer.into_inner())
}

fn parse_single_event(bytes: &[u8]) -> Result<Event<'static>> {
    let mut reader = Reader::from_reader(Cursor::new(bytes));
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Eof => bail!("could not parse serialized inline event"),
            event => return Ok(event.into_owned()),
        }
    }
}

fn write_events<W: Write>(writer: &mut Writer<W>, events: &[Event<'static>]) -> Result<()> {
    for event in events {
        writer.write_event(event.clone())?;
    }
    Ok(())
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

#[derive(Clone)]
struct TranslationBackend {
    provider: Provider,
    model: String,
    ollama_host: String,
    openai_base_url: String,
    claude_base_url: String,
    openai_api_key: Option<String>,
    anthropic_api_key: Option<String>,
    temperature: f32,
    num_ctx: u32,
    retries: u32,
    max_chars_per_request: usize,
    style: String,
    glossary: Vec<GlossaryEntry>,
    client: Client,
    api_usage: Arc<Mutex<ApiUsage>>,
    adaptive_concurrency: Arc<AdaptiveConcurrency>,
}

struct AdaptiveConcurrency {
    max: usize,
    current: AtomicUsize,
    success_streak: AtomicUsize,
}

impl AdaptiveConcurrency {
    fn new(max: usize) -> Self {
        let max = max.max(1);
        Self {
            max,
            current: AtomicUsize::new(max),
            success_streak: AtomicUsize::new(0),
        }
    }

    fn current(&self) -> usize {
        self.current.load(Ordering::Relaxed).clamp(1, self.max)
    }

    fn reduce(&self, provider: &str, err: &reqwest::Error) {
        let mut current = self.current();
        while current > 1 {
            let next = current - 1;
            match self
                .current
                .compare_exchange(current, next, Ordering::Relaxed, Ordering::Relaxed)
            {
                Ok(_) => {
                    self.success_streak.store(0, Ordering::Relaxed);
                    eprintln!(
                        "warning: reducing provider concurrency {current}->{next} after {provider} retryable error: {err}"
                    );
                    return;
                }
                Err(actual) => current = actual.clamp(1, self.max),
            }
        }
    }

    fn record_success(&self, provider: &str) {
        let streak = self.success_streak.fetch_add(1, Ordering::Relaxed) + 1;
        if streak < ADAPTIVE_CONCURRENCY_SUCCESS_THRESHOLD {
            return;
        }
        let mut current = self.current();
        while current < self.max {
            let next = current + 1;
            match self
                .current
                .compare_exchange(current, next, Ordering::Relaxed, Ordering::Relaxed)
            {
                Ok(_) => {
                    self.success_streak.store(0, Ordering::Relaxed);
                    eprintln!(
                        "warning: increasing provider concurrency {current}->{next} after {streak} successful {provider} request(s)"
                    );
                    return;
                }
                Err(actual) => current = actual.clamp(1, self.max),
            }
        }
        self.success_streak.store(0, Ordering::Relaxed);
    }
}

struct Translator {
    backend: TranslationBackend,
    fallback_backend: Option<TranslationBackend>,
    cache: CacheStore,
    partial_from_cache: bool,
    dry_run: bool,
    concurrency: usize,
    fallback_count: usize,
}

enum Translation {
    Translated { text: String, from_cache: bool },
    Original,
}

#[derive(Clone)]
struct TranslationJob {
    index: usize,
    source: String,
    glossary_subset: Vec<GlossaryEntry>,
    key: String,
}

struct TranslationJobResult {
    index: usize,
    key: String,
    translated: String,
    provider: Provider,
    model: String,
    fallback_used: bool,
}

impl Translator {
    fn new(args: CommonArgs, cache: CacheStore) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(args.timeout_secs))
            .build()
            .context("failed to create HTTP client")?;
        let provider = args.provider;
        let fallback_provider = args.fallback_provider;
        let model = args
            .model
            .clone()
            .unwrap_or_else(|| default_model_for_provider(provider).to_string());
        let openai_api_key = if args.usage_only {
            None
        } else {
            read_api_key(
                provider,
                Provider::Openai,
                args.openai_api_key.clone(),
                "OPENAI_API_KEY",
                args.prompt_api_key,
            )?
        };
        let anthropic_api_key = if args.usage_only {
            None
        } else {
            read_api_key(
                provider,
                Provider::Claude,
                args.anthropic_api_key.clone(),
                "ANTHROPIC_API_KEY",
                args.prompt_api_key,
            )?
        };
        let glossary = match &args.glossary {
            Some(path) => load_glossary(&path)?,
            None => Vec::new(),
        };
        let concurrency = args.concurrency.max(1);
        let api_usage = Arc::new(Mutex::new(ApiUsage::default()));
        let adaptive_concurrency = Arc::new(AdaptiveConcurrency::new(concurrency));
        let ollama_host = args.ollama_host.trim_end_matches('/').to_string();
        let openai_base_url = args.openai_base_url.trim_end_matches('/').to_string();
        let claude_base_url = args.claude_base_url.trim_end_matches('/').to_string();
        let fallback_backend = if let Some(provider) = fallback_provider {
            let fallback_model = args
                .fallback_model
                .clone()
                .unwrap_or_else(|| default_model_for_provider(provider).to_string());
            let fallback_openai_api_key = if args.usage_only {
                None
            } else {
                read_api_key(
                    provider,
                    Provider::Openai,
                    args.openai_api_key.clone(),
                    "OPENAI_API_KEY",
                    args.prompt_api_key,
                )?
            };
            let fallback_anthropic_api_key = if args.usage_only {
                None
            } else {
                read_api_key(
                    provider,
                    Provider::Claude,
                    args.anthropic_api_key.clone(),
                    "ANTHROPIC_API_KEY",
                    args.prompt_api_key,
                )?
            };
            Some(TranslationBackend {
                provider,
                model: fallback_model,
                ollama_host: ollama_host.clone(),
                openai_base_url: openai_base_url.clone(),
                claude_base_url: claude_base_url.clone(),
                openai_api_key: fallback_openai_api_key,
                anthropic_api_key: fallback_anthropic_api_key,
                temperature: args.temperature,
                num_ctx: args.num_ctx,
                retries: args.retries,
                max_chars_per_request: args.max_chars_per_request,
                style: args.style.clone(),
                glossary: glossary.clone(),
                client: client.clone(),
                api_usage: api_usage.clone(),
                adaptive_concurrency: adaptive_concurrency.clone(),
            })
        } else {
            None
        };
        Ok(Self {
            backend: TranslationBackend {
                provider,
                model,
                ollama_host,
                openai_base_url,
                claude_base_url,
                openai_api_key,
                anthropic_api_key,
                temperature: args.temperature,
                num_ctx: args.num_ctx,
                retries: args.retries,
                max_chars_per_request: args.max_chars_per_request,
                style: args.style,
                glossary,
                client,
                api_usage,
                adaptive_concurrency,
            },
            fallback_backend,
            cache,
            partial_from_cache: args.partial_from_cache,
            dry_run: args.dry_run,
            concurrency,
            fallback_count: 0,
        })
    }

    fn translate_many(
        &mut self,
        sources: &[String],
        progress: Option<&mut ProgressReporter>,
    ) -> Result<Vec<Translation>> {
        let mut translations = Vec::with_capacity(sources.len());
        let mut jobs = Vec::new();
        for (index, source) in sources.iter().enumerate() {
            translations.push(None);
            let translation = self.prepare_translation(index, source, &mut jobs)?;
            if let Some(translation) = translation {
                translations[index] = Some(translation);
            }
        }

        let job_count = jobs.len();
        let backend = self.backend.clone();
        let fallback_backend = self.fallback_backend.clone();
        run_translation_jobs(
            backend,
            fallback_backend,
            self.concurrency,
            jobs,
            progress,
            |result| {
                if result.fallback_used {
                    self.fallback_count += 1;
                }
                let record = CacheRecord {
                    key: result.key,
                    translated: result.translated.clone(),
                    provider: result.provider.to_string(),
                    model: result.model,
                    at: chrono::Utc::now().to_rfc3339(),
                };
                self.cache.insert(record)?;
                translations[result.index] = Some(Translation::Translated {
                    text: result.translated,
                    from_cache: false,
                });
                Ok(())
            },
        )?;

        translations
            .into_iter()
            .enumerate()
            .map(|(idx, translation)| {
                translation.with_context(|| {
                    format!(
                        "internal error: missing translation result {}/{}",
                        idx + 1,
                        sources.len()
                    )
                })
            })
            .collect::<Result<Vec<_>>>()
            .with_context(|| format!("failed to translate {job_count} uncached block(s)"))
    }

    fn prepare_translation(
        &mut self,
        index: usize,
        source: &str,
        jobs: &mut Vec<TranslationJob>,
    ) -> Result<Option<Translation>> {
        if self.dry_run {
            return Ok(Some(Translation::Translated {
                text: source.to_string(),
                from_cache: false,
            }));
        }
        let glossary_subset = self.backend.glossary_subset(source);
        let key = self.cache_key(source, &glossary_subset);
        if let Some(translated) = self.cache.get(&key) {
            match validate_cached_translation(source, &translated) {
                Ok(()) => {
                    return Ok(Some(Translation::Translated {
                        text: translated,
                        from_cache: true,
                    }));
                }
                Err(err) => {
                    eprintln!("warning: ignoring invalid cached translation for key {key}: {err}");
                    self.cache.invalidate(&key);
                }
            }
        }
        if self.partial_from_cache {
            return Ok(Some(Translation::Original));
        }
        jobs.push(TranslationJob {
            index,
            source: source.to_string(),
            glossary_subset,
            key,
        });
        Ok(None)
    }

    fn has_cached_translation(&self, source: &str) -> bool {
        if self.dry_run {
            return false;
        }
        let glossary_subset = self.backend.glossary_subset(source);
        let key = self.cache_key(source, &glossary_subset);
        self.cache
            .peek(&key)
            .is_some_and(|translated| validate_cached_translation(source, translated).is_ok())
    }

    fn cache_key(&self, source: &str, glossary_subset: &[GlossaryEntry]) -> String {
        self.backend.cache_key(source, glossary_subset)
    }

    fn manifest_params(&self) -> ManifestParams {
        ManifestParams {
            provider: self.backend.provider.to_string(),
            model: self.backend.model.clone(),
            prompt_version: "v1".to_string(),
            style_id: self.backend.style.clone(),
            glossary_sha: glossary_sha(&self.backend.glossary),
        }
    }

    fn api_usage_summary(&self) -> Option<String> {
        self.backend.api_usage_summary()
    }
}

fn run_translation_job_with_fallback(
    backend: &TranslationBackend,
    fallback_backend: Option<&TranslationBackend>,
    job: TranslationJob,
) -> Result<TranslationJobResult> {
    let primary_job = job.clone();
    match backend.run_translation_job(primary_job) {
        Ok(result) => Ok(result),
        Err(err) if is_refusal_validation_error(&err) => {
            let Some(fallback_backend) = fallback_backend else {
                return Err(err);
            };
            eprintln!(
                "warning: {} returned a refusal/explanation after validation retries; falling back to {} ({})",
                backend.provider, fallback_backend.provider, fallback_backend.model
            );
            let mut result = fallback_backend.run_translation_job(job).with_context(|| {
                format!(
                    "fallback translation failed after {} refusal/explanation",
                    backend.provider
                )
            })?;
            result.fallback_used = true;
            Ok(result)
        }
        Err(err) => Err(err),
    }
}

fn is_refusal_validation_error(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause
            .to_string()
            .contains("provider returned an explanation or refusal")
    })
}

fn run_translation_jobs<F>(
    backend: TranslationBackend,
    fallback_backend: Option<TranslationBackend>,
    concurrency_limit: usize,
    jobs: Vec<TranslationJob>,
    mut progress: Option<&mut ProgressReporter>,
    mut on_result: F,
) -> Result<()>
where
    F: FnMut(TranslationJobResult) -> Result<()>,
{
    if jobs.is_empty() {
        return Ok(());
    }
    let concurrency = concurrency_limit.max(1).min(jobs.len());
    if let Some(progress) = progress.as_mut() {
        progress.set_provider_batch(0, jobs.len(), backend.current_concurrency());
    }
    if concurrency == 1 {
        let job_count = jobs.len();
        let mut completed = 0usize;
        for job in jobs {
            let result =
                run_translation_job_with_fallback(&backend, fallback_backend.as_ref(), job)?;
            on_result(result)?;
            completed += 1;
            if let Some(progress) = progress.as_mut() {
                progress.complete_provider_block(
                    completed,
                    job_count,
                    backend.current_concurrency(),
                );
            }
        }
        return Ok(());
    }

    let (result_tx, result_rx) = mpsc::channel::<Result<TranslationJobResult>>();
    let job_count = jobs.len();
    thread::scope(|scope| {
        let mut worker_txs = Vec::with_capacity(concurrency);
        for _ in 0..concurrency {
            let (worker_tx, worker_rx) = mpsc::channel::<TranslationJob>();
            worker_txs.push(worker_tx);
            let result_tx = result_tx.clone();
            let backend = backend.clone();
            let fallback_backend = fallback_backend.clone();
            scope.spawn(move || {
                while let Ok(job) = worker_rx.recv() {
                    let result =
                        run_translation_job_with_fallback(&backend, fallback_backend.as_ref(), job);
                    if result_tx.send(result).is_err() {
                        break;
                    }
                }
            });
        }
        drop(result_tx);

        let mut queued = VecDeque::from(jobs);
        let mut next_worker = 0usize;
        let mut in_flight = 0usize;
        let mut completed = 0usize;

        dispatch_translation_jobs(
            &worker_txs,
            &mut queued,
            &mut next_worker,
            &mut in_flight,
            backend.current_concurrency(),
        )?;

        while completed < job_count {
            let received = result_rx
                .recv()
                .context("translation worker exited before sending all results")?;
            in_flight = in_flight.saturating_sub(1);
            on_result(received?)?;
            completed += 1;
            if let Some(progress) = progress.as_mut() {
                progress.complete_provider_block(
                    completed,
                    job_count,
                    backend.current_concurrency(),
                );
            }
            dispatch_translation_jobs(
                &worker_txs,
                &mut queued,
                &mut next_worker,
                &mut in_flight,
                backend.current_concurrency(),
            )?;
        }
        drop(worker_txs);
        Ok(())
    })
}

fn dispatch_translation_jobs(
    worker_txs: &[mpsc::Sender<TranslationJob>],
    queued: &mut VecDeque<TranslationJob>,
    next_worker: &mut usize,
    in_flight: &mut usize,
    current_limit: usize,
) -> Result<()> {
    let current_limit = current_limit.max(1);
    while *in_flight < current_limit {
        let Some(job) = queued.pop_front() else {
            break;
        };
        let worker_index = *next_worker % worker_txs.len();
        worker_txs[worker_index]
            .send(job)
            .context("failed to send translation job to worker")?;
        *next_worker += 1;
        *in_flight += 1;
    }
    Ok(())
}

impl TranslationBackend {
    fn current_concurrency(&self) -> usize {
        self.adaptive_concurrency.current()
    }

    fn run_translation_job(&self, job: TranslationJob) -> Result<TranslationJobResult> {
        let translated = self.translate_uncached(&job.source, &job.glossary_subset)?;
        Ok(TranslationJobResult {
            index: job.index,
            key: job.key,
            translated,
            provider: self.provider,
            model: self.model.clone(),
            fallback_used: false,
        })
    }

    fn translate_uncached(
        &self,
        source: &str,
        glossary_subset: &[GlossaryEntry],
    ) -> Result<String> {
        let chunks = split_translation_chunks(source, self.max_chars_per_request);
        if chunks.len() == 1 {
            return self.translate_with_validation(
                source,
                user_prompt(source, glossary_subset),
                glossary_subset,
            );
        }
        eprintln!(
            "warning: splitting long translation block into {} requests ({} chars, max {} chars per request)",
            chunks.len(),
            source.chars().count(),
            self.max_chars_per_request
        );
        let mut translations = Vec::with_capacity(chunks.len());
        for (idx, chunk) in chunks.iter().enumerate() {
            let chunk_glossary = self.glossary_subset(chunk);
            let translated = self
                .translate_with_validation(
                    chunk,
                    user_prompt(chunk, &chunk_glossary),
                    &chunk_glossary,
                )
                .with_context(|| {
                    format!(
                        "failed to translate long block chunk {}/{}",
                        idx + 1,
                        chunks.len()
                    )
                })?;
            translations.push(translated);
        }
        let translated = translations
            .into_iter()
            .map(|part| part.trim().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        validate_translation_response(source, &translated)?;
        Ok(translated)
    }

    fn translate_with_validation(
        &self,
        source: &str,
        initial_prompt: String,
        glossary_subset: &[GlossaryEntry],
    ) -> Result<String> {
        let mut prompt = initial_prompt;
        let attempts = self.retries.saturating_add(1).max(1);
        for attempt in 1..=attempts {
            let translated = match self.provider {
                Provider::Ollama => self.translate_ollama(&prompt),
                Provider::Openai => self.translate_openai(&prompt),
                Provider::Claude => self.translate_claude(&prompt),
            }?;
            match validate_translation_response(source, &translated) {
                Ok(()) => return Ok(translated),
                Err(err) if attempt < attempts => {
                    let wait_secs = 2_u64.saturating_pow((attempt - 1).min(5));
                    eprintln!(
                        "warning: translation validation failed: {err}; retry {attempt}/{} in {wait_secs}s",
                        self.retries
                    );
                    prompt =
                        retry_user_prompt(source, &glossary_subset, &translated, &err.to_string());
                    thread::sleep(Duration::from_secs(wait_secs));
                }
                Err(err) => {
                    return Err(err).with_context(|| {
                        format!("translation validation failed after {attempt} attempt(s)")
                    });
                }
            }
        }
        bail!("translation validation failed")
    }

    fn cache_key(&self, source: &str, glossary_subset: &[GlossaryEntry]) -> String {
        cache_key(
            self.provider,
            &self.model,
            &self.style,
            source,
            glossary_subset,
        )
    }

    fn glossary_subset(&self, source: &str) -> Vec<GlossaryEntry> {
        let source_lower = source.to_lowercase();
        self.glossary
            .iter()
            .filter(|entry| !entry.src.trim().is_empty() && !entry.dst.trim().is_empty())
            .filter(|entry| source_lower.contains(&entry.src.to_lowercase()))
            .cloned()
            .collect()
    }

    fn translate_ollama(&self, user_prompt: &str) -> Result<String> {
        let payload = serde_json::json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": system_prompt(&self.style)},
                {"role": "user", "content": user_prompt}
            ],
            "stream": false,
            "options": {
                "temperature": self.temperature,
                "top_p": 0.9,
                "num_ctx": self.num_ctx,
                "seed": 42
            }
        });
        let response: OllamaResponse = self.request_json_with_retry("Ollama", || {
            self.client
                .post(format!("{}/api/chat", self.ollama_host))
                .json(&payload)
        })?;
        self.record_usage(usage_from_ollama_response(&response));
        Ok(response.message.content.trim().to_string())
    }

    fn translate_openai(&self, user_prompt: &str) -> Result<String> {
        let api_key = self.openai_api_key.as_deref().context(
            "OpenAI provider requires OPENAI_API_KEY, --openai-api-key, or --prompt-api-key",
        )?;
        let payload = serde_json::json!({
            "model": self.model,
            "instructions": system_prompt(&self.style),
            "input": user_prompt
        });
        let value: serde_json::Value = self.request_json_with_retry("OpenAI", || {
            self.client
                .post(format!("{}/responses", self.openai_base_url))
                .bearer_auth(api_key)
                .json(&payload)
        })?;
        self.record_usage(usage_from_openai_value(&value));
        extract_openai_text(&value).context("OpenAI response did not contain output text")
    }

    fn translate_claude(&self, user_prompt: &str) -> Result<String> {
        let api_key = self.anthropic_api_key.as_deref().context(
            "Claude provider requires ANTHROPIC_API_KEY, --anthropic-api-key, or --prompt-api-key",
        )?;
        let payload = serde_json::json!({
            "model": self.model,
            "max_tokens": 2048,
            "temperature": self.temperature,
            "system": system_prompt(&self.style),
            "messages": [
                {"role": "user", "content": user_prompt}
            ]
        });
        let response: ClaudeResponse = self.request_json_with_retry("Claude", || {
            self.client
                .post(format!("{}/messages", self.claude_base_url))
                .header("x-api-key", api_key)
                .header("anthropic-version", ANTHROPIC_VERSION)
                .header("content-type", "application/json")
                .json(&payload)
        })?;
        self.record_usage(usage_from_claude_response(&response));
        let text = response
            .content
            .into_iter()
            .filter(|part| part.kind == "text")
            .filter_map(|part| part.text)
            .collect::<Vec<_>>()
            .join("");
        if text.trim().is_empty() {
            bail!("Claude response did not contain text content");
        }
        Ok(text.trim().to_string())
    }

    fn request_json_with_retry<T, F>(&self, provider: &str, build: F) -> Result<T>
    where
        T: DeserializeOwned,
        F: Fn() -> reqwest::blocking::RequestBuilder,
    {
        let attempts = self.retries.saturating_add(1).max(1);
        for attempt in 1..=attempts {
            let result = build()
                .send()
                .and_then(|response| response.error_for_status())
                .and_then(|response| response.json::<T>());
            match result {
                Ok(value) => {
                    self.adaptive_concurrency.record_success(provider);
                    return Ok(value);
                }
                Err(err) if attempt < attempts && should_retry_request(&err) => {
                    self.adaptive_concurrency.reduce(provider, &err);
                    let wait_secs = 2_u64.saturating_pow((attempt - 1).min(5));
                    eprintln!(
                        "warning: {provider} request failed: {err}; retry {attempt}/{} in {wait_secs}s",
                        self.retries
                    );
                    thread::sleep(Duration::from_secs(wait_secs));
                }
                Err(err) => {
                    return Err(err).with_context(|| {
                        format!("failed to call {provider} after {attempt} attempt(s)")
                    });
                }
            }
        }
        bail!("failed to call {provider}")
    }

    fn record_usage(&self, usage: ApiUsage) {
        if usage.is_empty() {
            return;
        }
        if let Ok(mut total) = self.api_usage.lock() {
            total.add(usage);
        }
    }

    fn api_usage_summary(&self) -> Option<String> {
        let usage = self.api_usage.lock().ok()?;
        (!usage.is_empty()).then(|| usage.summary())
    }
}

fn should_retry_request(err: &reqwest::Error) -> bool {
    if err.is_timeout() || err.is_connect() || err.is_request() {
        return true;
    }
    err.status()
        .map(|status| status.as_u16() == 429 || status.is_server_error())
        .unwrap_or(false)
}

fn split_translation_chunks(source: &str, max_chars: usize) -> Vec<String> {
    let mut rest = source.trim();
    if rest.is_empty() || max_chars == 0 || rest.chars().count() <= max_chars {
        return vec![rest.to_string()];
    }
    let mut chunks = Vec::new();
    while rest.chars().count() > max_chars {
        let cut = find_translation_chunk_cut(rest, max_chars).unwrap_or(rest.len());
        let (chunk, next) = rest.split_at(cut);
        let chunk = chunk.trim();
        if chunk.is_empty() {
            break;
        }
        chunks.push(chunk.to_string());
        rest = next.trim_start();
    }
    if !rest.trim().is_empty() {
        chunks.push(rest.trim().to_string());
    }
    chunks
}

fn find_translation_chunk_cut(source: &str, max_chars: usize) -> Option<usize> {
    let mut in_placeholder = false;
    let mut last_sentence_boundary = None;
    let mut last_space_boundary = None;
    let min_boundary_chars = (max_chars / 3).max(1);
    let mut count = 0usize;

    for (idx, ch) in source.char_indices() {
        count += 1;
        let next_idx = idx + ch.len_utf8();
        match ch {
            '⟦' => in_placeholder = true,
            '⟧' => in_placeholder = false,
            _ => {}
        }
        if !in_placeholder {
            if count >= min_boundary_chars {
                if is_sentence_boundary(ch) {
                    last_sentence_boundary = Some(next_idx);
                } else if ch.is_whitespace() {
                    last_space_boundary = Some(next_idx);
                }
            }
        }
        if count >= max_chars && !in_placeholder {
            return last_sentence_boundary
                .or(last_space_boundary)
                .filter(|cut| *cut > 0)
                .or(Some(next_idx));
        }
    }
    None
}

fn is_sentence_boundary(ch: char) -> bool {
    matches!(
        ch,
        '.' | '!' | '?' | ';' | ':' | '。' | '！' | '？' | '；' | '：'
    )
}

fn validate_translation_response(source: &str, translated: &str) -> Result<()> {
    let source = source.trim();
    let translated = translated.trim();
    if translated.is_empty() {
        bail!("translation validation failed: provider returned an empty translation");
    }
    if contains_prompt_tag(translated) {
        bail!("translation validation failed: provider returned prompt wrapper tags");
    }
    validate_placeholder_tokens(source, translated)?;
    if is_meaningful_english(source)
        && normalize_for_comparison(source) == normalize_for_comparison(translated)
    {
        bail!("translation validation failed: provider returned the source text unchanged");
    }
    if looks_like_refusal_or_explanation(translated) {
        bail!(
            "translation validation failed: provider returned an explanation or refusal instead of a translation"
        );
    }
    if likely_untranslated_english(source, translated) {
        bail!(
            "translation validation failed: provider response does not appear to contain Japanese text"
        );
    }
    if likely_truncated_translation(source, translated) {
        bail!("translation validation failed: provider response appears to be truncated");
    }
    Ok(())
}

fn validate_cached_translation(source: &str, translated: &str) -> Result<()> {
    validate_translation_response(source, translated)
        .context("cached translation no longer passes validation")
}

fn contains_prompt_tag(text: &str) -> bool {
    let lower = text.to_lowercase();
    ["<source", "</source>", "<glossary", "</glossary>"]
        .iter()
        .any(|tag| lower.contains(tag))
}

fn validate_placeholder_tokens(source: &str, translated: &str) -> Result<()> {
    let source_tokens = placeholder_signature(source);
    if source_tokens.is_empty() {
        return Ok(());
    }
    let translated_tokens = placeholder_signature(translated);
    if source_tokens != translated_tokens {
        bail!("translation validation failed: provider changed or dropped inline placeholders");
    }
    Ok(())
}

fn placeholder_signature(text: &str) -> Vec<String> {
    let mut signature = tokenize_placeholders(text)
        .into_iter()
        .filter_map(|token| match token {
            Token::Open(id) => Some(format!("E{id}")),
            Token::Close(id) => Some(format!("/E{id}")),
            Token::SelfClose(id) => Some(format!("S{id}")),
            Token::Text(_) => None,
        })
        .collect::<Vec<_>>();
    signature.sort_unstable();
    signature
}

fn normalize_for_comparison(text: &str) -> String {
    text.chars()
        .filter(|ch| !ch.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

fn is_meaningful_english(text: &str) -> bool {
    ascii_letter_count(text) >= 6
}

fn likely_untranslated_english(source: &str, translated: &str) -> bool {
    if japanese_char_count(translated) > 0 {
        return false;
    }
    let source_words = ascii_word_count(source);
    let translated_words = ascii_word_count(translated);
    source_words >= 3 && translated_words >= 3
        || ascii_letter_count(source) >= 24 && ascii_letter_count(translated) >= 12
}

fn ascii_word_count(text: &str) -> usize {
    text.split(|ch: char| !ch.is_ascii_alphabetic())
        .filter(|part| part.len() >= 2)
        .count()
}

fn ascii_letter_count(text: &str) -> usize {
    text.chars().filter(|ch| ch.is_ascii_alphabetic()).count()
}

fn japanese_char_count(text: &str) -> usize {
    text.chars()
        .filter(|ch| {
            matches!(
                *ch,
                '\u{3040}'..='\u{309f}'
                    | '\u{30a0}'..='\u{30ff}'
                    | '\u{3400}'..='\u{9fff}'
                    | '\u{f900}'..='\u{faff}'
            )
        })
        .count()
}

fn looks_like_refusal_or_explanation(text: &str) -> bool {
    let lower = text.trim_start().to_lowercase();
    [
        "as an ai",
        "i am sorry",
        "i'm sorry",
        "i cannot",
        "i can't",
        "cannot translate",
        "can't translate",
        "translation:",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
        || ["申し訳", "翻訳できません", "翻訳不能"]
            .iter()
            .any(|prefix| text.trim_start().starts_with(prefix))
}

fn likely_truncated_translation(source: &str, translated: &str) -> bool {
    let source_letters = ascii_letter_count(source);
    let source_words = ascii_word_count(source);
    let translated_japanese = japanese_char_count(translated);
    if translated_japanese == 0 || source_words < 20 {
        return false;
    }
    if ends_with_sentence_terminal(source) && !ends_with_sentence_terminal(translated) {
        return true;
    }
    source_letters >= 240 && translated_japanese * 6 < source_letters
}

fn ends_with_sentence_terminal(text: &str) -> bool {
    text.trim_end().chars().last().is_some_and(|ch| {
        matches!(
            ch,
            '.' | '!'
                | '?'
                | '。'
                | '！'
                | '？'
                | '"'
                | '\''
                | '”'
                | '’'
                | '」'
                | '』'
                | ')'
                | '）'
                | '⟧'
        )
    })
}

fn read_api_key(
    active_provider: Provider,
    key_provider: Provider,
    explicit: Option<String>,
    env_name: &str,
    prompt: bool,
) -> Result<Option<String>> {
    if active_provider != key_provider {
        return Ok(None);
    }
    if explicit.is_some() {
        return Ok(explicit);
    }
    if let Ok(value) = std::env::var(env_name) {
        if !value.trim().is_empty() {
            return Ok(Some(value));
        }
    }
    if prompt {
        let value = rpassword::prompt_password(format!("{env_name}: "))
            .with_context(|| format!("failed to read {env_name} from prompt"))?;
        if !value.trim().is_empty() {
            return Ok(Some(value));
        }
    }
    Ok(None)
}

fn extract_openai_text(value: &serde_json::Value) -> Option<String> {
    if let Some(text) = value.get("output_text").and_then(|v| v.as_str()) {
        return Some(text.trim().to_string());
    }
    let mut parts = Vec::new();
    for item in value.get("output")?.as_array()? {
        for content in item
            .get("content")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
        {
            if let Some(text) = content.get("text").and_then(|v| v.as_str()) {
                parts.push(text);
            }
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("").trim().to_string())
    }
}

fn cache_key(
    provider: Provider,
    model: &str,
    style: &str,
    source: &str,
    glossary_subset: &[GlossaryEntry],
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"epubicus-cache-v1\n");
    hasher.update(provider.to_string().as_bytes());
    hasher.update(b"\n");
    hasher.update(model.as_bytes());
    hasher.update(b"\n");
    hasher.update(style.as_bytes());
    hasher.update(b"\n");
    for entry in glossary_subset {
        hasher.update(entry.src.as_bytes());
        hasher.update(b"=>");
        hasher.update(entry.dst.as_bytes());
        hasher.update(b"\n");
    }
    hasher.update(source.as_bytes());
    let digest = hasher.finalize();
    digest[..16].iter().map(|b| format!("{b:02x}")).collect()
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
            prompt_version: "v1".to_string(),
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
