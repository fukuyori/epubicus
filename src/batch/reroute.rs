use super::*;
use std::collections::HashSet;
use std::path::Path;
use std::time::{Duration, Instant};

use indicatif::{ProgressBar, ProgressStyle};
use crate::{
    cache::CacheRecord,
    translator::{
        Translator, ValidationFailureReason, is_provider_auth_error, is_reference_like_source,
        validation_failure_reason,
    },
};

const LOCAL_STALL_ABORT_ELAPSED: Duration = Duration::from_secs(10 * 60);
const LOCAL_STALL_ABORT_REQUESTS: u64 = 20;
const LOCAL_BLOCK_REQUEST_BUDGET: u64 = 3;

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
        "Translated local batch items: {} translated | {} cached | {} failed | {} total | elapsed {} | total active {} | dir: {}",
        summary.translated_count,
        summary.cached_count,
        summary.failed_count,
        summary.total_count,
        super::run::format_duration_hms(summary.run_elapsed),
        super::run::format_duration_hms(Duration::from_secs(
            summary.total_active_elapsed_secs
        )),
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
    let run_started = Instant::now();
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
    super::run::begin_batch_manifest_run(&manifest_path)?;
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
    let mut stall_guard = LocalTranslateStallGuard::new(translator.api_usage_snapshot());
    let pending_total = candidate_indices
        .iter()
        .filter(|&&idx| work_items[idx].state == "local_pending")
        .take(limit)
        .count();
    let progress = ProgressBar::new(pending_total as u64);
    progress.set_style(
        ProgressStyle::with_template(
            "{spinner:.green} [{elapsed_precise}] {bar:20.cyan/blue} {pos}/{len} | {msg}",
        )?
        .progress_chars("=> "),
    );
    progress.set_message("preparing local_pending items");

    for idx in candidate_indices {
        if processed_count >= limit {
            break;
        }
        let (page_index, block_index, href, abort_error) = {
            let item = &mut work_items[idx];
            if item.state != "local_pending" {
                continue;
            }
            processed_count += 1;
            progress.set_message(local_progress_message(
                translated_count,
                cached_count,
                failed_count,
                item.page_index + 1,
                &item.href,
            ));
            if is_reference_like_local_batch_source(&item.source_text) {
                item.state = "local_exhausted".to_string();
                item.last_error = Some(
                    "skipped local batch retry for reference-like block | suggested_action=inspect_reference_or_try_another_provider".to_string(),
                );
                item.updated_at = chrono::Utc::now().to_rfc3339();
                failed_count += 1;
                (
                    item.page_index,
                    item.block_index,
                    item.href.clone(),
                    None,
                )
            } else if translator.cache.peek(&item.cache_key).is_some() {
                item.state = "local_imported".to_string();
                item.last_error = None;
                item.updated_at = chrono::Utc::now().to_rfc3339();
                cached_count += 1;
                (item.page_index, item.block_index, item.href.clone(), None)
            } else {
                let usage_before = translator.api_usage_snapshot();
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
                        (item.page_index, item.block_index, item.href.clone(), None)
                    }
                    Err(err) => {
                        let usage_after = translator.api_usage_snapshot();
                        let error_text = local_batch_error_text(
                            &err,
                            &item.source_text,
                            usage_before,
                            usage_after,
                        );
                        item.last_error = Some(error_text.clone());
                        item.updated_at = chrono::Utc::now().to_rfc3339();
                        if should_exhaust_local_batch_error(
                            &err,
                            &item.source_text,
                            usage_before,
                            usage_after,
                        ) {
                            item.state = "local_exhausted".to_string();
                        }
                        failed_count += 1;
                        let abort_error = should_abort_local_batch_error(&err).then(|| {
                            format!(
                                "batch translate-local aborted after provider authentication/configuration failure at p{} b{} {}",
                                item.page_index, item.block_index, item.href
                            )
                        });
                        (item.page_index, item.block_index, item.href.clone(), abort_error)
                    }
                }
            }
        };
        persist_translate_local_progress(
            &manifest_path,
            &work_items_path,
            &mut manifest,
            &work_items,
        )?;
        super::run::heartbeat_batch_manifest_run(&manifest_path)?;
        if let Some(message) = abort_error {
            let total_active_elapsed_secs = super::run::finish_batch_manifest_run(&manifest_path)?
                .unwrap_or_else(|| run_started.elapsed().as_secs());
            return Err(crate::recoverable_error(local_error_summary(
                &message,
                run_started.elapsed(),
                total_active_elapsed_secs,
                total_count,
                translated_count,
                cached_count,
                failed_count,
                processed_count,
                &work_items,
            )));
        }
        let stall_error = stall_guard.observe(
            translated_count + cached_count,
            failed_count,
            translator.api_usage_snapshot(),
            page_index,
            block_index,
            &href,
        );
        if let Some(message) = stall_error {
            let item = &mut work_items[idx];
            item.state = "local_exhausted".to_string();
            let combined = match item.last_error.as_deref() {
                Some(previous) if !previous.trim().is_empty() => {
                    format!("{previous} | {message}")
                }
                _ => message.clone(),
            };
            item.last_error = Some(combined);
            item.updated_at = chrono::Utc::now().to_rfc3339();
            persist_translate_local_progress(
                &manifest_path,
                &work_items_path,
                &mut manifest,
                &work_items,
            )?;
            let total_active_elapsed_secs = super::run::finish_batch_manifest_run(&manifest_path)?
                .unwrap_or_else(|| run_started.elapsed().as_secs());
            return Err(crate::recoverable_error(local_error_summary(
                &message,
                run_started.elapsed(),
                total_active_elapsed_secs,
                total_count,
                translated_count,
                cached_count,
                failed_count,
                processed_count,
                &work_items,
            )));
        }
        progress.inc(1);
    }

    progress.finish_with_message(format!(
        "done: {} completed, {} errors | {} translated, {} cached",
        translated_count + cached_count,
        failed_count,
        translated_count,
        cached_count
    ));

    let total_active_elapsed_secs = super::run::finish_batch_manifest_run(&manifest_path)?
        .unwrap_or_else(|| run_started.elapsed().as_secs());
    Ok(TranslateLocalSummary {
        batch_dir,
        total_count,
        translated_count,
        cached_count,
        failed_count,
        run_elapsed: run_started.elapsed(),
        total_active_elapsed_secs,
    })
}

fn local_error_summary(
    message: &str,
    run_elapsed: Duration,
    total_active_elapsed_secs: u64,
    total_count: usize,
    translated_count: usize,
    cached_count: usize,
    failed_count: usize,
    processed_count: usize,
    work_items: &[WorkItem],
) -> String {
    let state_counts = state_counts(work_items);
    let local_imported = state_counts.get("local_imported").copied().unwrap_or(0);
    let local_pending = state_counts.get("local_pending").copied().unwrap_or(0);
    let local_exhausted = state_counts.get("local_exhausted").copied().unwrap_or(0);
    let submitted = state_counts.get("submitted").copied().unwrap_or(0);
    format!(
        "{message}\nBatch local summary: processed {processed_count} this run | translated {translated_count} | cached {cached_count} | errors {failed_count} | total {total_count} | local_imported {local_imported} | local_pending {local_pending} | local_exhausted {local_exhausted} | submitted {submitted} | elapsed {} | total active {}",
        super::run::format_duration_hms(run_elapsed),
        super::run::format_duration_hms(Duration::from_secs(total_active_elapsed_secs))
    )
}

fn state_counts(work_items: &[WorkItem]) -> BTreeMap<&str, usize> {
    let mut counts = BTreeMap::new();
    for item in work_items {
        *counts.entry(item.state.as_str()).or_insert(0) += 1;
    }
    counts
}

fn local_progress_message(
    translated_count: usize,
    cached_count: usize,
    failed_count: usize,
    page_no: usize,
    href: &str,
) -> String {
    let completed_count = translated_count + cached_count;
    let name = Path::new(href)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(href);
    format!(
        "ok{completed_count} err{failed_count} | t{translated_count} c{cached_count} | p{page_no} {name}"
    )
}

fn persist_translate_local_progress(
    manifest_path: &Path,
    work_items_path: &Path,
    manifest: &mut BatchManifest,
    work_items: &[WorkItem],
) -> Result<()> {
    manifest.updated_at = chrono::Utc::now().to_rfc3339();
    if work_items.iter().any(|item| item.state == "local_pending") {
        manifest.status = "local_pending".to_string();
    } else if work_items
        .iter()
        .any(|item| item.state == "local_exhausted")
    {
        manifest.status = "local_exhausted".to_string();
    } else {
        manifest.status = "local_imported".to_string();
    }
    write_jsonl_atomic(work_items_path, work_items.iter())?;
    write_json_pretty_atomic(manifest_path, manifest)?;
    Ok(())
}

struct LocalTranslateStallGuard {
    started: Instant,
    baseline_completed: usize,
    baseline_usage: crate::usage::ApiUsage,
}

impl LocalTranslateStallGuard {
    fn new(usage: crate::usage::ApiUsage) -> Self {
        Self {
            started: Instant::now(),
            baseline_completed: 0,
            baseline_usage: usage,
        }
    }

    fn observe(
        &mut self,
        completed_count: usize,
        failed_count: usize,
        usage: crate::usage::ApiUsage,
        page_index: usize,
        block_index: usize,
        href: &str,
    ) -> Option<String> {
        if completed_count > self.baseline_completed {
            self.started = Instant::now();
            self.baseline_completed = completed_count;
            self.baseline_usage = usage;
            return None;
        }
        if !should_abort_stalled_local_work(
            self.started.elapsed(),
            self.baseline_usage,
            usage,
            self.baseline_completed,
            completed_count,
        ) {
            return None;
        }
        Some(format!(
            "batch translate-local stalled: {} completed, {} errors; no new completions for {} while API requests increased by {}; current item p{} b{} {}",
            completed_count,
            failed_count,
            format_stall_duration(self.started.elapsed()),
            usage.requests.saturating_sub(self.baseline_usage.requests),
            page_index,
            block_index,
            href
        ))
    }
}

fn should_abort_stalled_local_work(
    elapsed: Duration,
    baseline_usage: crate::usage::ApiUsage,
    usage: crate::usage::ApiUsage,
    baseline_completed: usize,
    completed_count: usize,
) -> bool {
    elapsed >= LOCAL_STALL_ABORT_ELAPSED
        && completed_count == baseline_completed
        && usage.requests
            >= baseline_usage
                .requests
                .saturating_add(LOCAL_STALL_ABORT_REQUESTS)
}

fn format_stall_duration(duration: Duration) -> String {
    let secs = duration.as_secs();
    let minutes = secs / 60;
    let seconds = secs % 60;
    format!("{minutes}m {seconds:02}s")
}

fn should_abort_local_batch_error(err: &anyhow::Error) -> bool {
    is_provider_auth_error(err)
}

fn should_exhaust_local_batch_error(
    err: &anyhow::Error,
    source_text: &str,
    usage_before: crate::usage::ApiUsage,
    usage_after: crate::usage::ApiUsage,
) -> bool {
    if matches!(
        classify_untranslated_segment_for_local_batch(err, source_text),
        Some(LocalBatchUntranslatedSegment::ReferenceLike)
    ) {
        return true;
    }
    if matches!(
        validation_failure_reason(err),
        Some(
            ValidationFailureReason::PromptLeak
                | ValidationFailureReason::MissingPlaceholder
                | ValidationFailureReason::UnchangedSource
                | ValidationFailureReason::RefusalOrExplanation
        )
    ) {
        return true;
    }
    usage_after.requests.saturating_sub(usage_before.requests) >= LOCAL_BLOCK_REQUEST_BUDGET
}

fn local_batch_error_text(
    err: &anyhow::Error,
    source_text: &str,
    usage_before: crate::usage::ApiUsage,
    usage_after: crate::usage::ApiUsage,
) -> String {
    let mut text = format!("{err:#}");
    let request_delta = usage_after.requests.saturating_sub(usage_before.requests);
    if request_delta >= LOCAL_BLOCK_REQUEST_BUDGET {
        text.push_str(&format!(
            " | local request budget exceeded: {} request(s) used (limit {})",
            request_delta, LOCAL_BLOCK_REQUEST_BUDGET
        ));
    }
    if let Some(reason) = validation_failure_reason(err) {
        text.push_str(&format!(" | validation_reason={}", reason.as_str()));
    }
    text.push_str(&format!(
        " | suggested_action={}",
        suggested_action_for_local_batch_error(err, source_text, usage_before, usage_after)
    ));
    text
}

fn suggested_action_for_local_batch_error(
    err: &anyhow::Error,
    source_text: &str,
    usage_before: crate::usage::ApiUsage,
    usage_after: crate::usage::ApiUsage,
) -> &'static str {
    if is_provider_auth_error(err) {
        return "fix_provider_auth";
    }
    if is_reference_like_local_batch_source(source_text) {
        return "inspect_reference_or_try_another_provider";
    }
    if usage_after.requests.saturating_sub(usage_before.requests) >= LOCAL_BLOCK_REQUEST_BUDGET {
        return "batch_retry_requests_or_try_another_provider";
    }
    match validation_failure_reason(err) {
        Some(ValidationFailureReason::MissingPlaceholder) => {
            "retry_translation_or_inspect_inline"
        }
        Some(ValidationFailureReason::UntranslatedSegment)
            if matches!(
                classify_untranslated_segment_for_local_batch(err, source_text),
                Some(LocalBatchUntranslatedSegment::ReferenceLike)
            ) =>
        {
            "inspect_reference_or_try_another_provider"
        }
        Some(
            ValidationFailureReason::UnchangedSource
            | ValidationFailureReason::UntranslatedText
            | ValidationFailureReason::UntranslatedSegment,
        ) => "retry_translation",
        Some(
            ValidationFailureReason::PromptLeak
            | ValidationFailureReason::RefusalOrExplanation
            | ValidationFailureReason::Empty,
        ) => "retry_translation",
        Some(ValidationFailureReason::Truncated) => "retry_translation",
        None => "inspect_manually",
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LocalBatchUntranslatedSegment {
    ReferenceLike,
    ProseLike,
}

fn classify_untranslated_segment_for_local_batch(
    err: &anyhow::Error,
    source_text: &str,
) -> Option<LocalBatchUntranslatedSegment> {
    if validation_failure_reason(err) != Some(ValidationFailureReason::UntranslatedSegment) {
        return None;
    }
    if is_reference_like_untranslated_segment(source_text) {
        Some(LocalBatchUntranslatedSegment::ReferenceLike)
    } else {
        Some(LocalBatchUntranslatedSegment::ProseLike)
    }
}

fn is_reference_like_untranslated_segment(source_text: &str) -> bool {
    is_reference_like_local_batch_source(source_text)
}

fn is_reference_like_local_batch_source(source_text: &str) -> bool {
    is_reference_like_source(source_text)
}

fn is_remaining_item(item: &WorkItem, cache: &CacheStore) -> bool {
    !is_finished_item(item, cache)
        && item.state != "local_pending"
        && item.state != "local_exhausted"
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
        "failed" | "rejected" | "local_exhausted" => 0,
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
    run_elapsed: Duration,
    total_active_elapsed_secs: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stalled_local_work_aborts_after_request_growth_without_completions() {
        assert!(should_abort_stalled_local_work(
            LOCAL_STALL_ABORT_ELAPSED,
            crate::usage::ApiUsage {
                requests: 1,
                ..Default::default()
            },
            crate::usage::ApiUsage {
                requests: 21,
                ..Default::default()
            },
            0,
            0,
        ));
    }

    #[test]
    fn stalled_local_work_does_not_abort_after_completion_progress() {
        assert!(!should_abort_stalled_local_work(
            LOCAL_STALL_ABORT_ELAPSED,
            crate::usage::ApiUsage {
                requests: 1,
                ..Default::default()
            },
            crate::usage::ApiUsage {
                requests: 30,
                ..Default::default()
            },
            0,
            1,
        ));
    }

    #[test]
    fn auth_errors_abort_local_batch_immediately() {
        let err = anyhow::anyhow!(
            "failed to call OpenAI after 1 attempt(s): OpenAI HTTP 401 Unauthorized: invalid_api_key"
        );
        assert!(should_abort_local_batch_error(&err));
    }

    #[test]
    fn local_error_summary_includes_counts_and_elapsed() {
        let items = vec![
            test_work_item("local_imported"),
            test_work_item("local_pending"),
            test_work_item("local_exhausted"),
            test_work_item("submitted"),
        ];

        let summary = local_error_summary(
            "stopped",
            Duration::from_secs(65),
            125,
            4,
            1,
            2,
            3,
            6,
            &items,
        );

        assert!(summary.contains("stopped"));
        assert!(summary.contains("processed 6 this run"));
        assert!(summary.contains("translated 1"));
        assert!(summary.contains("cached 2"));
        assert!(summary.contains("errors 3"));
        assert!(summary.contains("local_pending 1"));
        assert!(summary.contains("local_exhausted 1"));
        assert!(summary.contains("elapsed 00:01:05"));
        assert!(summary.contains("total active 00:02:05"));
    }

    #[test]
    fn non_retryable_validation_exhausts_local_batch_item() {
        let err = crate::translator::validate_translation_response(
            "before ⟦S1⟧ after",
            "before after",
        )
        .unwrap_err();
        assert!(should_exhaust_local_batch_error(
            &err,
            "before ⟦S1⟧ after",
            crate::usage::ApiUsage::default(),
            crate::usage::ApiUsage::default(),
        ));
    }

    #[test]
    fn request_budget_exhausts_local_batch_item() {
        assert!(should_exhaust_local_batch_error(
            &anyhow::anyhow!("failed to call OpenAI"),
            "normal source text",
            crate::usage::ApiUsage {
                requests: 1,
                ..Default::default()
            },
            crate::usage::ApiUsage {
                requests: 4,
                ..Default::default()
            },
        ));
    }

    #[test]
    fn untranslated_segment_records_retry_translation_action() {
        let err = crate::translator::validate_translation_response(
            "This is a long English sentence that should be translated into Japanese.",
            "This is a long English sentence that should be translated into Japanese.",
        )
        .unwrap_err();
        let text = local_batch_error_text(
            &err,
            "This is a long English sentence that should be translated into Japanese.",
            crate::usage::ApiUsage::default(),
            crate::usage::ApiUsage {
                requests: 1,
                ..Default::default()
            },
        );
        assert!(text.contains("suggested_action=retry_translation"));
    }

    #[test]
    fn reference_like_untranslated_segment_is_exhausted() {
        let source =
            "⟦E1⟧6⟦/E1⟧. Michael Lierow, Sebastian Jannsen, and Joris D’Inca, “Amazon Is Using Logistics to Lead a Retail Revolution,” ⟦E2⟧Forbes⟦/E2⟧ (February 21, 2016), ⟦E3⟧https://www.forbes.com/example⟦/E3⟧";
        let err = crate::translator::validate_translation_response(source, source).unwrap_err();
        assert!(should_exhaust_local_batch_error(
            &err,
            source,
            crate::usage::ApiUsage::default(),
            crate::usage::ApiUsage::default(),
        ));
        let text = local_batch_error_text(
            &err,
            source,
            crate::usage::ApiUsage::default(),
            crate::usage::ApiUsage::default(),
        );
        assert!(text.contains(
            "suggested_action=inspect_reference_or_try_another_provider"
        ));
    }

    fn test_work_item(state: &str) -> WorkItem {
        WorkItem {
            custom_id: format!("id-{state}"),
            cache_key: format!("key-{state}"),
            page_index: 0,
            block_index: 0,
            href: "chapter.xhtml".to_string(),
            source_text: "source".to_string(),
            source_hash: "hash".to_string(),
            prompt_hash: "prompt".to_string(),
            source_chars: 6,
            provider: "openai".to_string(),
            model: DEFAULT_OPENAI_MODEL.to_string(),
            state: state.to_string(),
            attempt: 0,
            last_error: None,
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }
}
