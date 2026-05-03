use super::*;
use std::collections::BTreeMap;
pub(super) fn batch_health(args: BatchHealthArgs) -> Result<()> {
    let health = collect_batch_health(&args)?;
    print_batch_health(&health);
    Ok(())
}

pub(super) fn batch_verify(args: BatchVerifyArgs) -> Result<()> {
    let report = collect_batch_verify(&args)?;
    print_batch_verify(&report);
    if report.has_findings() {
        bail!("batch verify found artifact inconsistencies");
    }
    Ok(())
}

pub(super) fn collect_batch_health(args: &BatchHealthArgs) -> Result<BatchHealth> {
    if args.common.no_cache {
        bail!("batch health requires cache; remove --no-cache");
    }
    if args.common.clear_cache {
        bail!("batch health is read-only and does not support --clear-cache");
    }

    let _run_lock = acquire_input_run_lock(&args.input, "inspect batch input EPUB")?;
    let cache = CacheStore::from_args(&args.input, &args.common)?;
    let batch_dir = cache.dir.join(BATCH_DIR);
    if !batch_dir.exists() {
        bail!(
            "batch workspace does not exist: {}; run batch prepare first",
            batch_dir.display()
        );
    }
    let batch_lock_path = batch_lock_path(&cache);
    let _batch_lock = FileLock::acquire(&batch_lock_path, "inspect batch")?;

    let manifest_path = batch_dir.join(BATCH_MANIFEST_FILE);
    let manifest = if manifest_path.exists() {
        Some(read_json_file::<BatchManifest>(&manifest_path)?)
    } else {
        None
    };
    let work_items_path = batch_dir.join(WORK_ITEMS_FILE);
    let work_items = if work_items_path.exists() {
        read_jsonl_file::<WorkItem>(&work_items_path)?
    } else {
        Vec::new()
    };
    let request_count = count_jsonl_lines(&batch_dir.join(REQUESTS_FILE))?;
    let rejected_file_count = count_jsonl_lines(&batch_dir.join(REJECTED_FILE))?;
    let error_file_count = count_jsonl_lines(&batch_dir.join(ERRORS_FILE))?;
    let import_report = read_optional_json::<ImportReport>(&batch_dir.join(IMPORT_REPORT_FILE))?;

    let mut part_status_counts = BTreeMap::new();
    if let Some(manifest) = &manifest {
        for part in &manifest.parts {
            *part_status_counts.entry(part.status.clone()).or_insert(0) += 1;
        }
    }

    let mut state_counts = BTreeMap::new();
    let mut cache_backed_items = 0usize;
    let mut cache_backed_pending_items = 0usize;
    let mut effective_remaining_items = 0usize;
    let mut oldest_pending_at = None::<String>;
    for item in &work_items {
        *state_counts.entry(item.state.clone()).or_insert(0) += 1;
        let has_cache = cache.peek(&item.cache_key).is_some();
        if has_cache {
            cache_backed_items += 1;
        }
        if !is_completed_item_state(&item.state) && has_cache {
            cache_backed_pending_items += 1;
        }
        if !is_completed_item_state(&item.state) && !has_cache {
            effective_remaining_items += 1;
        }
        if !is_completed_item_state(&item.state) {
            oldest_pending_at = match oldest_pending_at {
                Some(ref current) if current <= &item.updated_at => oldest_pending_at,
                _ => Some(item.updated_at.clone()),
            };
        }
    }
    let oldest_pending_age_secs = oldest_pending_at
        .as_deref()
        .and_then(age_since_rfc3339_secs);

    Ok(BatchHealth {
        input_hash: cache.input_hash.clone(),
        batch_dir,
        manifest_status: manifest.as_ref().map(|manifest| manifest.status.clone()),
        manifest_request_count: manifest.as_ref().map(|manifest| manifest.request_count),
        manifest_batch_id: manifest
            .as_ref()
            .and_then(|manifest| manifest.batch_id.clone()),
        manifest_output_file_id: manifest
            .as_ref()
            .and_then(|manifest| manifest.output_file_id.clone()),
        manifest_error_file_id: manifest
            .as_ref()
            .and_then(|manifest| manifest.error_file_id.clone()),
        manifest_output_file: manifest
            .as_ref()
            .and_then(|manifest| manifest.output_file.clone()),
        manifest_failed_count: manifest.as_ref().map(|manifest| manifest.failed_count),
        manifest_part_status_counts: part_status_counts,
        request_count,
        work_item_count: work_items.len(),
        state_counts,
        cache_backed_items,
        cache_backed_pending_items,
        effective_remaining_items,
        rejected_file_count,
        error_file_count,
        import_report,
        oldest_pending_at,
        oldest_pending_age_secs,
    })
}

fn print_batch_health(health: &BatchHealth) {
    println!("Batch health");
    println!("input hash: {}", health.input_hash);
    println!("batch dir: {}", health.batch_dir.display());
    println!(
        "manifest: {}",
        health.manifest_status.as_deref().unwrap_or("(missing)")
    );
    if let Some(batch_id) = &health.manifest_batch_id {
        println!("remote batch: {batch_id}");
    }
    if let Some(output_file_id) = &health.manifest_output_file_id {
        println!("remote output file: {output_file_id}");
    }
    if let Some(error_file_id) = &health.manifest_error_file_id {
        println!("remote error file: {error_file_id}");
    }
    if let Some(failed_count) = health.manifest_failed_count {
        println!("remote failed requests: {failed_count}");
    }
    if !health.manifest_part_status_counts.is_empty() {
        println!("remote parts:");
        for (status, count) in &health.manifest_part_status_counts {
            println!("  {status}: {count}");
        }
    }
    if let Some(output_file) = &health.manifest_output_file {
        println!("output file: {output_file}");
    }
    println!(
        "requests: {}{}",
        health.request_count,
        health
            .manifest_request_count
            .map(|count| format!(" (manifest {count})"))
            .unwrap_or_default()
    );
    println!("work items: {}", health.work_item_count);
    if health.state_counts.is_empty() {
        println!("states: (none)");
    } else {
        println!("states:");
        for (state, count) in &health.state_counts {
            println!("  {state}: {count}");
        }
    }
    println!(
        "cache-backed: {}/{}",
        health.cache_backed_items, health.work_item_count
    );
    if health.cache_backed_pending_items > 0 {
        println!(
            "cache-backed but still pending state: {}",
            health.cache_backed_pending_items
        );
    }
    println!(
        "effective remaining: {}",
        health.effective_remaining_items
    );
    println!("rejected lines: {}", health.rejected_file_count);
    println!("error lines: {}", health.error_file_count);
    if let Some(report) = &health.import_report {
        println!(
            "last import: {} imported | {} already cached | {} rejected | {} error(s)",
            report.imported_count,
            report.already_cached_count,
            report.rejected_count,
            report.error_count
        );
    }
    if let Some(oldest) = &health.oldest_pending_at {
        println!("oldest pending update: {oldest}");
        if let Some(age_secs) = health.oldest_pending_age_secs {
            println!("oldest pending age: {}", format_duration_secs(age_secs));
        }
    }
}

fn is_completed_item_state(state: &str) -> bool {
    matches!(state, "imported" | "local_imported" | "cached" | "skipped")
}

fn age_since_rfc3339_secs(value: &str) -> Option<i64> {
    let parsed = chrono::DateTime::parse_from_rfc3339(value).ok()?;
    let age = chrono::Utc::now().signed_duration_since(parsed.with_timezone(&chrono::Utc));
    Some(age.num_seconds().max(0))
}

fn format_duration_secs(secs: i64) -> String {
    let days = secs / 86_400;
    let hours = (secs % 86_400) / 3_600;
    let minutes = (secs % 3_600) / 60;
    let seconds = secs % 60;
    if days > 0 {
        format!("{days}d {hours}h {minutes}m")
    } else if hours > 0 {
        format!("{hours}h {minutes}m {seconds}s")
    } else if minutes > 0 {
        format!("{minutes}m {seconds}s")
    } else {
        format!("{seconds}s")
    }
}

pub(super) fn collect_batch_verify(args: &BatchVerifyArgs) -> Result<BatchVerifyReport> {
    if args.common.no_cache {
        bail!("batch verify requires cache; remove --no-cache");
    }
    if args.common.clear_cache {
        bail!("batch verify is read-only and does not support --clear-cache");
    }

    let _run_lock = acquire_input_run_lock(&args.input, "verify batch input EPUB")?;
    let book = unpack_epub(&args.input)?;
    let cache = CacheStore::from_args(&args.input, &args.common)?;
    let batch_dir = cache.dir.join(BATCH_DIR);
    if !batch_dir.exists() {
        bail!(
            "batch workspace does not exist: {}; run batch prepare first",
            batch_dir.display()
        );
    }
    let batch_lock_path = batch_lock_path(&cache);
    let _batch_lock = FileLock::acquire(&batch_lock_path, "verify batch")?;

    let manifest_path = batch_dir.join(BATCH_MANIFEST_FILE);
    let manifest = read_optional_json::<BatchManifest>(&manifest_path)?;
    let work_items_path = batch_dir.join(WORK_ITEMS_FILE);
    let work_items = if work_items_path.exists() {
        read_jsonl_file::<WorkItem>(&work_items_path)?
    } else {
        Vec::new()
    };
    let provider = manifest
        .as_ref()
        .and_then(|manifest| parse_provider(&manifest.provider))
        .unwrap_or(args.common.provider);
    let model = manifest
        .as_ref()
        .map(|manifest| manifest.model.clone())
        .or_else(|| args.common.model.clone())
        .unwrap_or_else(|| default_model_for_provider(provider).to_string());
    let glossary = match &args.common.glossary {
        Some(path) => load_glossary(path)?,
        None => Vec::new(),
    };

    let pages = verify_pages(&work_items, book.spine.len())?;
    let mut expected = Vec::new();
    for page_no in pages {
        let Some(item) = book.spine.get(page_no - 1) else {
            continue;
        };
        collect_page_work_items(
            &book,
            &item.abs_path,
            page_no,
            &cache,
            provider,
            &model,
            &args.common.style,
            &glossary,
            false,
            &mut expected,
        )
        .with_context(|| format!("failed to verify batch work for {}", item.href))?;
    }

    let expected_by_location = expected
        .iter()
        .map(|item| {
            (
                (item.work_item.page_index, item.work_item.block_index),
                &item.work_item,
            )
        })
        .collect::<HashMap<_, _>>();
    let work_by_location = work_items
        .iter()
        .map(|item| ((item.page_index, item.block_index), item))
        .collect::<HashMap<_, _>>();

    let mut missing = Vec::new();
    let mut stale = Vec::new();
    let mut orphaned = Vec::new();
    let mut cache_conflict = Vec::new();
    let mut invalid_cache = Vec::new();

    for expected_item in expected_by_location.values() {
        if !work_by_location.contains_key(&(expected_item.page_index, expected_item.block_index)) {
            missing.push(VerifyFinding::from_work_item(
                expected_item,
                "missing from work_items",
            ));
        }
    }

    for item in &work_items {
        let Some(expected_item) = expected_by_location.get(&(item.page_index, item.block_index))
        else {
            orphaned.push(VerifyFinding::from_work_item(
                item,
                "location no longer exists in current EPUB extraction",
            ));
            continue;
        };
        let mut reasons = Vec::new();
        if item.cache_key != expected_item.cache_key {
            reasons.push("cache_key changed");
        }
        if item.source_hash != expected_item.source_hash {
            reasons.push("source_hash changed");
        }
        if item.prompt_hash != expected_item.prompt_hash {
            reasons.push("prompt_hash changed");
        }
        if item.source_chars != expected_item.source_chars {
            reasons.push("source_chars changed");
        }
        if item.source_text != expected_item.source_text {
            reasons.push("source_text changed");
        }
        if !reasons.is_empty() {
            stale.push(VerifyFinding::from_work_item(item, &reasons.join(", ")));
        }

        let cached = cache.peek(&item.cache_key);
        let cache_backed_state =
            matches!(item.state.as_str(), "imported" | "local_imported" | "skipped");
        if cache_backed_state && cached.is_none() {
            cache_conflict.push(VerifyFinding::from_work_item(
                item,
                "state is cache-backed but cache entry is missing",
            ));
        } else if !cache_backed_state && cached.is_some() {
            cache_conflict.push(VerifyFinding::from_work_item(
                item,
                "cache entry exists but state is not cache-backed",
            ));
        }
        if let Some(translated) = cached {
            if item.state == "skipped" && translated == item.source_text {
                continue;
            }
            if let Err(err) = validate_translation_response(&item.source_text, translated) {
                invalid_cache.push(VerifyFinding::from_work_item(item, &err.to_string()));
            }
        }
    }

    Ok(BatchVerifyReport {
        input_hash: cache.input_hash.clone(),
        batch_dir,
        checked_pages: expected_by_location
            .keys()
            .map(|(page, _)| *page)
            .collect::<HashSet<_>>()
            .len(),
        expected_count: expected.len(),
        work_item_count: work_items.len(),
        missing,
        stale,
        orphaned,
        cache_conflict,
        invalid_cache,
    })
}

fn print_batch_verify(report: &BatchVerifyReport) {
    println!("Batch verify");
    println!("input hash: {}", report.input_hash);
    println!("batch dir: {}", report.batch_dir.display());
    println!("checked pages: {}", report.checked_pages);
    println!("expected items: {}", report.expected_count);
    println!("work items: {}", report.work_item_count);
    println!("missing: {}", report.missing.len());
    println!("stale: {}", report.stale.len());
    println!("orphaned: {}", report.orphaned.len());
    println!("cache_conflict: {}", report.cache_conflict.len());
    println!("invalid_cache: {}", report.invalid_cache.len());
    print_finding_examples("missing", &report.missing);
    print_finding_examples("stale", &report.stale);
    print_finding_examples("orphaned", &report.orphaned);
    print_finding_examples("cache_conflict", &report.cache_conflict);
    print_finding_examples("invalid_cache", &report.invalid_cache);
}

fn print_finding_examples(label: &str, findings: &[VerifyFinding]) {
    if findings.is_empty() {
        return;
    }
    println!("{label} examples:");
    for finding in findings.iter().take(5) {
        println!(
            "  p{} b{} {} | {}",
            finding.page_index, finding.block_index, finding.cache_key, finding.reason
        );
    }
}
