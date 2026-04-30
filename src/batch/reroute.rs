use super::*;
use std::collections::HashSet;

use crate::{cache::CacheRecord, translator::Translator};

pub(super) fn batch_reroute_local(args: BatchRerouteLocalArgs) -> Result<()> {
    let summary = reroute_local_items(&args)?;
    println!(
        "Rerouted batch items: {} selected | {} skipped cached/imported | {} total | dir: {}",
        summary.selected_count,
        summary.skipped_finished_count,
        summary.total_count,
        summary.batch_dir.display()
    );
    Ok(())
}

pub(super) fn batch_translate_local(args: BatchTranslateLocalArgs) -> Result<()> {
    let summary = translate_local_items(args)?;
    println!(
        "Translated local batch items: {} translated | {} cached | {} failed | {} total | dir: {}",
        summary.translated_count,
        summary.cached_count,
        summary.failed_count,
        summary.total_count,
        summary.batch_dir.display()
    );
    Ok(())
}

pub(super) fn reroute_local_items(args: &BatchRerouteLocalArgs) -> Result<RerouteLocalSummary> {
    if args.common.no_cache {
        bail!("batch reroute-local requires cache; remove --no-cache");
    }
    if args.common.clear_cache {
        bail!("batch reroute-local does not support --clear-cache");
    }
    if args.states.is_empty() && !args.remaining && args.endgame_threshold.is_none() {
        bail!("select items with --state, --remaining, or --endgame-threshold");
    }

    let _run_lock = acquire_input_run_lock(&args.input, "reroute batch input EPUB")?;
    let cache = CacheStore::from_args(&args.input, &args.common)?;
    let batch_dir = cache.dir.join(BATCH_DIR);
    let batch_lock_path = batch_lock_path(&cache);
    let _batch_lock = FileLock::acquire(&batch_lock_path, "reroute batch")?;

    let manifest_path = batch_dir.join(BATCH_MANIFEST_FILE);
    let work_items_path = batch_dir.join(WORK_ITEMS_FILE);
    let mut manifest: BatchManifest = read_json_file(&manifest_path)?;
    let mut work_items: Vec<WorkItem> = read_jsonl_file(&work_items_path)?;
    let state_filter = args
        .states
        .iter()
        .map(|state| state.trim().to_string())
        .filter(|state| !state.is_empty())
        .collect::<HashSet<_>>();
    let remaining_count = work_items
        .iter()
        .filter(|item| is_remaining_item(item, &cache))
        .count();
    if let Some(threshold) = args.endgame_threshold
        && remaining_count > threshold
    {
        println!(
            "Reroute skipped: {} remaining item(s) is above endgame threshold {}",
            remaining_count, threshold
        );
        return Ok(RerouteLocalSummary {
            batch_dir,
            total_count: work_items.len(),
            selected_count: 0,
            skipped_finished_count: work_items.len().saturating_sub(remaining_count),
        });
    }

    let now = chrono::Utc::now().to_rfc3339();
    let mut selected_count = 0usize;
    let mut skipped_finished_count = 0usize;
    let limit = args.limit.unwrap_or(usize::MAX);
    let selected_indices = prioritized_indices(&work_items, args.priority);
    for idx in selected_indices {
        if selected_count >= limit {
            break;
        }
        let item = &mut work_items[idx];
        let finished = is_finished_item(item, &cache);
        if finished {
            skipped_finished_count += 1;
        }
        let selected_by_state = state_filter.contains(&item.state);
        let selected_by_remaining = (args.remaining || args.endgame_threshold.is_some())
            && !finished
            && item.state != "local_pending";
        if selected_by_state || selected_by_remaining {
            if finished {
                continue;
            }
            item.state = "local_pending".to_string();
            item.last_error = None;
            item.updated_at = now.clone();
            selected_count += 1;
        }
    }

    if selected_count > 0 {
        manifest.status = "local_pending".to_string();
        manifest.updated_at = now;
        write_jsonl_atomic(&work_items_path, work_items.iter())?;
        write_json_pretty_atomic(&manifest_path, &manifest)?;
    }

    Ok(RerouteLocalSummary {
        batch_dir,
        total_count: work_items.len(),
        selected_count,
        skipped_finished_count,
    })
}

fn translate_local_items(args: BatchTranslateLocalArgs) -> Result<TranslateLocalSummary> {
    if args.common.no_cache {
        bail!("batch translate-local requires cache; remove --no-cache");
    }
    if args.common.clear_cache {
        bail!("batch translate-local does not support --clear-cache");
    }

    let _run_lock = acquire_input_run_lock(&args.input, "translate local batch input EPUB")?;
    let cache = CacheStore::from_args(&args.input, &args.common)?;
    let batch_dir = cache.dir.join(BATCH_DIR);
    let batch_lock_path = batch_lock_path(&cache);
    let _batch_lock = FileLock::acquire(&batch_lock_path, "translate local batch")?;

    let manifest_path = batch_dir.join(BATCH_MANIFEST_FILE);
    let work_items_path = batch_dir.join(WORK_ITEMS_FILE);
    let mut manifest: BatchManifest = read_json_file(&manifest_path)?;
    let mut work_items: Vec<WorkItem> = read_jsonl_file(&work_items_path)?;
    let total_count = work_items.len();
    let mut translator = Translator::new(args.common, cache)?;
    let mut translated_count = 0usize;
    let mut cached_count = 0usize;
    let mut failed_count = 0usize;
    let mut processed_count = 0usize;
    let limit = args.limit.unwrap_or(usize::MAX);
    let candidate_indices = prioritized_indices(&work_items, args.priority);

    for idx in candidate_indices {
        if processed_count >= limit {
            break;
        }
        let item = &mut work_items[idx];
        if item.state != "local_pending" {
            continue;
        }
        processed_count += 1;
        if translator.cache.peek(&item.cache_key).is_some() {
            item.state = "local_imported".to_string();
            item.last_error = None;
            item.updated_at = chrono::Utc::now().to_rfc3339();
            cached_count += 1;
            continue;
        }

        match translator.translate_uncached_source(&item.source_text) {
            Ok((translated, provider, model, fallback_used)) => {
                translator.cache.insert(CacheRecord {
                    key: item.cache_key.clone(),
                    translated,
                    provider: provider.to_string(),
                    model,
                    at: chrono::Utc::now().to_rfc3339(),
                })?;
                item.state = "local_imported".to_string();
                item.last_error = None;
                item.updated_at = chrono::Utc::now().to_rfc3339();
                if fallback_used {
                    translator.fallback_count += 1;
                }
                translated_count += 1;
            }
            Err(err) => {
                item.last_error = Some(err.to_string());
                item.updated_at = chrono::Utc::now().to_rfc3339();
                failed_count += 1;
            }
        }
    }

    if translated_count > 0 || cached_count > 0 || failed_count > 0 {
        manifest.updated_at = chrono::Utc::now().to_rfc3339();
        if work_items.iter().any(|item| item.state == "local_pending") {
            manifest.status = "local_pending".to_string();
        } else {
            manifest.status = "local_imported".to_string();
        }
        write_jsonl_atomic(&work_items_path, work_items.iter())?;
        write_json_pretty_atomic(&manifest_path, &manifest)?;
    }

    Ok(TranslateLocalSummary {
        batch_dir,
        total_count,
        translated_count,
        cached_count,
        failed_count,
    })
}

fn is_remaining_item(item: &WorkItem, cache: &CacheStore) -> bool {
    !is_finished_item(item, cache) && item.state != "local_pending"
}

fn is_finished_item(item: &WorkItem, cache: &CacheStore) -> bool {
    matches!(item.state.as_str(), "imported" | "local_imported")
        || cache.peek(&item.cache_key).is_some()
}

pub(super) fn prioritized_indices(work_items: &[WorkItem], priority: BatchPriority) -> Vec<usize> {
    let mut indices = (0..work_items.len()).collect::<Vec<_>>();
    indices.sort_by(|left, right| {
        let left_item = &work_items[*left];
        let right_item = &work_items[*right];
        let ordering = match priority {
            BatchPriority::PageOrder => left_item
                .page_index
                .cmp(&right_item.page_index)
                .then(left_item.block_index.cmp(&right_item.block_index)),
            BatchPriority::FailedFirst => state_rank(&left_item.state)
                .cmp(&state_rank(&right_item.state))
                .then(left_item.page_index.cmp(&right_item.page_index))
                .then(left_item.block_index.cmp(&right_item.block_index)),
            BatchPriority::HardFirst => complexity_score(right_item)
                .cmp(&complexity_score(left_item))
                .then(left_item.page_index.cmp(&right_item.page_index))
                .then(left_item.block_index.cmp(&right_item.block_index)),
            BatchPriority::ShortFirst => left_item
                .source_chars
                .cmp(&right_item.source_chars)
                .then(left_item.page_index.cmp(&right_item.page_index))
                .then(left_item.block_index.cmp(&right_item.block_index)),
            BatchPriority::OldestFirst => left_item
                .updated_at
                .cmp(&right_item.updated_at)
                .then(left_item.page_index.cmp(&right_item.page_index))
                .then(left_item.block_index.cmp(&right_item.block_index)),
        };
        ordering.then(left.cmp(right))
    });
    indices
}

fn state_rank(state: &str) -> usize {
    match state {
        "failed" | "rejected" => 0,
        "local_pending" => 1,
        "submitted" => 2,
        "prepared" => 3,
        _ => 4,
    }
}

fn complexity_score(item: &WorkItem) -> usize {
    item.source_chars
        + item.source_text.matches('⟦').count() * 100
        + item.attempt as usize * 25
        + item.last_error.as_ref().map(|_| 50).unwrap_or(0)
}

#[derive(Debug)]
pub(super) struct RerouteLocalSummary {
    pub(super) batch_dir: PathBuf,
    pub(super) total_count: usize,
    pub(super) selected_count: usize,
    pub(super) skipped_finished_count: usize,
}

#[derive(Debug)]
struct TranslateLocalSummary {
    batch_dir: PathBuf,
    total_count: usize,
    translated_count: usize,
    cached_count: usize,
    failed_count: usize,
}
