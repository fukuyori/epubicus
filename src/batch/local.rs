use super::*;
use std::collections::{HashMap, HashSet};

pub(super) fn batch_prepare(args: BatchPrepareArgs) -> Result<()> {
    if args.common.no_cache {
        bail!("batch prepare requires cache; remove --no-cache");
    }
    if args.common.provider != Provider::Openai {
        bail!("batch prepare currently supports --provider openai only");
    }
    if args.max_requests_per_file == 0 {
        bail!("--max-requests-per-file must be greater than 0");
    }
    if args.max_bytes_per_file == 0 {
        bail!("--max-bytes-per-file must be greater than 0");
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
    clean_previous_batch_artifacts(&batch_dir)?;

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

    let part_plans = build_batch_part_plans(
        &prepared,
        args.max_requests_per_file,
        args.max_bytes_per_file,
    )?;
    let parts = part_plans
        .iter()
        .map(|plan| plan.part.clone())
        .collect::<Vec<_>>();
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
        parts: parts.clone(),
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
    for plan in &part_plans {
        write_jsonl_atomic(
            &batch_dir.join(&plan.part.request_file),
            prepared[plan.start..plan.end]
                .iter()
                .map(|item| &item.request),
        )?;
    }

    println!(
        "Prepared batch: {} request(s) in {} part(s) | provider: {} | model: {} | dir: {}",
        manifest.request_count,
        manifest.parts.len(),
        manifest.provider,
        manifest.model,
        batch_dir.display()
    );
    Ok(())
}

fn clean_previous_batch_artifacts(batch_dir: &Path) -> Result<()> {
    for file_name in [
        OUTPUT_FILE,
        REMOTE_ERRORS_FILE,
        IMPORT_REPORT_FILE,
        REJECTED_FILE,
        ERRORS_FILE,
        RETRY_REQUESTS_FILE,
    ] {
        remove_file_if_exists(&batch_dir.join(file_name))?;
    }
    for entry in fs::read_dir(batch_dir)
        .with_context(|| format!("failed to read batch dir {}", batch_dir.display()))?
    {
        let entry = entry?;
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();
        if is_generated_part_artifact(&file_name) {
            remove_file_if_exists(&entry.path())?;
        }
    }
    Ok(())
}

fn is_generated_part_artifact(file_name: &str) -> bool {
    (file_name.starts_with("requests.part-")
        || file_name.starts_with("output.part-")
        || file_name.starts_with("remote_errors.part-"))
        && file_name.ends_with(".jsonl")
}

fn remove_file_if_exists(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).with_context(|| format!("failed to remove {}", path.display())),
    }
}

fn build_batch_part_plans(
    prepared: &[PreparedItem],
    max_requests_per_file: usize,
    max_bytes_per_file: usize,
) -> Result<Vec<BatchPartPlan>> {
    let mut plans = Vec::new();
    let mut start = 0usize;
    let mut count = 0usize;
    let mut bytes = 0usize;

    for (idx, item) in prepared.iter().enumerate() {
        let line_bytes = serde_json::to_vec(&item.request)?.len() + 1;
        if line_bytes > max_bytes_per_file {
            bail!(
                "batch request {} is {} bytes, larger than --max-bytes-per-file {}",
                item.work_item.custom_id,
                line_bytes,
                max_bytes_per_file
            );
        }
        let would_exceed_count = count >= max_requests_per_file;
        let would_exceed_bytes = count > 0 && bytes + line_bytes > max_bytes_per_file;
        if would_exceed_count || would_exceed_bytes {
            plans.push(make_batch_part_plan(
                plans.len() + 1,
                start,
                idx,
                count,
                bytes,
            ));
            start = idx;
            count = 0;
            bytes = 0;
        }
        count += 1;
        bytes += line_bytes;
    }

    if count > 0 || prepared.is_empty() {
        plans.push(make_batch_part_plan(
            plans.len() + 1,
            start,
            prepared.len(),
            count,
            bytes,
        ));
    }
    Ok(plans)
}

fn make_batch_part_plan(
    index: usize,
    start: usize,
    end: usize,
    request_count: usize,
    request_bytes: usize,
) -> BatchPartPlan {
    BatchPartPlan {
        part: BatchPart {
            index,
            request_file: request_part_file_name(index),
            request_count,
            request_bytes,
            file_id: None,
            batch_id: None,
            status: "prepared".to_string(),
            output_file_id: None,
            error_file_id: None,
            output_file: None,
            error_file: None,
            failed_count: 0,
        },
        start,
        end,
    }
}

pub(super) fn batch_retry_requests(args: BatchRetryArgs) -> Result<()> {
    if args.common.no_cache {
        bail!("batch retry-requests requires cache; remove --no-cache");
    }
    if args.common.clear_cache {
        bail!("batch retry-requests does not support --clear-cache");
    }
    let _run_lock = acquire_input_run_lock(&args.input, "write batch retry requests")?;
    let cache = CacheStore::from_args(&args.input, &args.common)?;
    let batch_dir = cache.dir.join(BATCH_DIR);
    let batch_lock_path = batch_lock_path(&cache);
    let _batch_lock = FileLock::acquire(&batch_lock_path, "write batch retry requests")?;

    let manifest: BatchManifest = read_json_file(&batch_dir.join(BATCH_MANIFEST_FILE))?;
    validate_batch_manifest_for_input_local(&manifest, &cache)?;
    let work_items: Vec<WorkItem> = read_jsonl_file(&batch_dir.join(WORK_ITEMS_FILE))?;
    let requests: Vec<BatchRequestLine> = read_jsonl_file(&batch_dir.join(REQUESTS_FILE))?;
    let request_by_id = requests
        .into_iter()
        .map(|request| (request.custom_id.clone(), request))
        .collect::<HashMap<_, _>>();
    let state_filter = retry_state_filter(&args.states);
    let limit = args.limit.unwrap_or(usize::MAX);
    let mut retry_requests = Vec::new();
    let mut skipped_cached = 0usize;
    let mut missing_requests = Vec::new();

    for idx in super::reroute::prioritized_indices(&work_items, args.priority) {
        if retry_requests.len() >= limit {
            break;
        }
        let item = &work_items[idx];
        if !state_filter.contains(&item.state) {
            continue;
        }
        if cache.peek(&item.cache_key).is_some() {
            skipped_cached += 1;
            continue;
        }
        let Some(request) = request_by_id.get(&item.custom_id) else {
            missing_requests.push(item.custom_id.clone());
            continue;
        };
        retry_requests.push(request.clone());
    }

    if !missing_requests.is_empty() {
        bail!(
            "{} selected work item(s) were missing from requests.jsonl; first missing custom_id: {}",
            missing_requests.len(),
            missing_requests[0]
        );
    }

    write_jsonl_atomic(&batch_dir.join(RETRY_REQUESTS_FILE), retry_requests.iter())?;
    println!(
        "Wrote retry requests: {} request(s) | {} skipped cached | states: {} | priority: {} | file: {}",
        retry_requests.len(),
        skipped_cached,
        state_filter.iter().cloned().collect::<Vec<_>>().join(","),
        args.priority,
        batch_dir.join(RETRY_REQUESTS_FILE).display()
    );
    Ok(())
}

fn retry_state_filter(states: &[String]) -> HashSet<String> {
    let selected = if states.is_empty() {
        vec!["failed".to_string(), "rejected".to_string()]
    } else {
        states
            .iter()
            .map(|state| state.trim().to_string())
            .filter(|state| !state.is_empty())
            .collect::<Vec<_>>()
    };
    selected.into_iter().collect()
}

fn validate_batch_manifest_for_input_local(
    manifest: &BatchManifest,
    cache: &CacheStore,
) -> Result<()> {
    if manifest.input_sha256 != cache.input_sha256 {
        bail!("batch manifest input_sha256 does not match the current input EPUB");
    }
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

    let use_default_output = args.output.is_none();
    let output_path = args.output.unwrap_or_else(|| batch_dir.join(OUTPUT_FILE));
    let remote_errors_path = batch_dir.join(REMOTE_ERRORS_FILE);
    let manifest_path = batch_dir.join(BATCH_MANIFEST_FILE);
    let work_items_path = batch_dir.join(WORK_ITEMS_FILE);
    let mut manifest: BatchManifest = read_json_file(&manifest_path)?;
    let mut work_items: Vec<WorkItem> = read_jsonl_file(&work_items_path)?;
    let output_lines: Vec<BatchOutputLine> = if output_path.exists() {
        read_jsonl_file(&output_path)?
    } else if use_default_output && remote_errors_path.exists() {
        Vec::new()
    } else {
        read_jsonl_file(&output_path)?
    };
    let remote_error_lines: Vec<BatchOutputLine> =
        if use_default_output && remote_errors_path.exists() {
            read_jsonl_file(&remote_errors_path)?
        } else {
            Vec::new()
        };
    let mut item_by_id = work_items
        .iter()
        .enumerate()
        .map(|(idx, item)| (item.custom_id.clone(), idx))
        .collect::<HashMap<_, _>>();

    let now = chrono::Utc::now().to_rfc3339();
    let mut imported = Vec::new();
    let mut already_cached = Vec::new();
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
            Ok(ImportLineStatus::Imported) => {
                item.state = "imported".to_string();
                item.last_error = None;
                item.updated_at = now.clone();
                imported.push(item.custom_id.clone());
            }
            Ok(ImportLineStatus::AlreadyCached) => {
                item.state = "imported".to_string();
                item.last_error = None;
                item.updated_at = now.clone();
                already_cached.push(item.custom_id.clone());
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

    for line in remote_error_lines {
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
        let error = import_remote_error_line(line);
        item.state = "failed".to_string();
        item.last_error = Some(error.clone());
        item.updated_at = now.clone();
        errors.push(ImportErrorLine {
            custom_id: item.custom_id.clone(),
            error,
        });
    }

    manifest.updated_at = now;
    manifest.status = if rejected.is_empty() && errors.is_empty() {
        "imported".to_string()
    } else {
        "imported_with_errors".to_string()
    };
    manifest.output_file = Some(output_path.display().to_string());
    if use_default_output && remote_errors_path.exists() {
        manifest.error_file = Some(remote_errors_path.display().to_string());
    }
    manifest.imported_count = imported.len() + already_cached.len();
    manifest.rejected_count = rejected.len();
    manifest.failed_count = errors.len();

    let report = ImportReport {
        imported_count: imported.len(),
        already_cached_count: already_cached.len(),
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
        "Imported batch output: {} imported | {} already cached | {} rejected | {} error(s) | dir: {}",
        report.imported_count,
        report.already_cached_count,
        report.rejected_count,
        report.error_count,
        batch_dir.display()
    );
    Ok(())
}

fn import_remote_error_line(line: BatchOutputLine) -> String {
    if let Some(error) = line.error {
        return format!("remote batch error: {error}");
    }
    if let Some(response) = line.response {
        return format!("remote batch error status_code: {}", response.status_code);
    }
    "remote batch error line did not contain error details".to_string()
}

fn import_output_line(
    line: BatchOutputLine,
    item: &WorkItem,
    cache: &mut CacheStore,
) -> Result<ImportLineStatus> {
    if item.source_hash != hash_text(&item.source_text) {
        bail!("work item source_hash does not match source_text");
    }
    if let Some(cached) = cache.peek(&item.cache_key)
        && validate_translation_response(&item.source_text, cached).is_ok()
    {
        return Ok(ImportLineStatus::AlreadyCached);
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
    if let Some(cached) = cache.peek(&item.cache_key)
        && cached == translated
    {
        return Ok(ImportLineStatus::AlreadyCached);
    }
    cache.insert(crate::cache::CacheRecord {
        key: item.cache_key.clone(),
        translated,
        provider: item.provider.clone(),
        model: item.model.clone(),
        at: chrono::Utc::now().to_rfc3339(),
    })?;
    Ok(ImportLineStatus::Imported)
}

enum ImportLineStatus {
    Imported,
    AlreadyCached,
}
