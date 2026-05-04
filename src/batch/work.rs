use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn collect_page_work_items(
    book: &EpubBook,
    path: &Path,
    page_no: usize,
    cache: &CacheStore,
    provider: Provider,
    model: &str,
    style: &str,
    glossary: &[GlossaryEntry],
    skip_cached: bool,
    out: &mut Vec<PreparedItem>,
) -> Result<()> {
    let source = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut reader = Reader::from_reader(Cursor::new(source));
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut block_index = 0usize;
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) if is_translatable_block_start(&e) => {
                let end_name = e.name().as_ref().to_vec();
                let inner = collect_element_inner(&mut reader, &end_name)?;
                let (source_text, _) = encode_inline(&inner)?;
                let source_text = source_text.trim().to_string();
                if !source_text.is_empty() {
                    block_index += 1;
                    let glossary_subset = glossary_subset(glossary, &source_text);
                    let cache_key =
                        cache_key(provider, model, style, &source_text, &glossary_subset);
                    if skip_cached
                        && cache.peek(&cache_key).is_some_and(|translated| {
                            validate_translation_response(&source_text, translated).is_ok()
                        })
                    {
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

pub(super) fn normalize_batch_range(
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

pub(super) fn verify_pages(work_items: &[WorkItem], spine_len: usize) -> Result<Vec<usize>> {
    if spine_len == 0 {
        bail!("EPUB spine has no XHTML pages");
    }
    let mut pages = work_items
        .iter()
        .map(|item| item.page_index)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if pages.is_empty() {
        pages = (1..=spine_len).collect();
    }
    pages.sort_unstable();
    for page in &pages {
        if *page == 0 || *page > spine_len {
            bail!(
                "work_items.jsonl references invalid spine page {page}; valid range is 1-{spine_len}"
            );
        }
    }
    Ok(pages)
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

pub(super) fn default_model_for_provider(provider: Provider) -> &'static str {
    match provider {
        Provider::Ollama => DEFAULT_MODEL,
        Provider::Openai => DEFAULT_OPENAI_MODEL,
        Provider::Claude => DEFAULT_CLAUDE_MODEL,
    }
}

pub(super) fn parse_provider(value: &str) -> Option<Provider> {
    match value {
        "ollama" => Some(Provider::Ollama),
        "openai" => Some(Provider::Openai),
        "claude" => Some(Provider::Claude),
        _ => None,
    }
}

pub(super) fn batch_lock_path(cache: &CacheStore) -> PathBuf {
    cache
        .lock_path
        .with_file_name(format!("{}.batch.lock", cache.input_hash))
}

pub(super) fn hash_text(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    let digest = hasher.finalize();
    digest[..16].iter().map(|b| format!("{b:02x}")).collect()
}
