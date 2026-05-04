use super::*;

pub(super) fn batch_run(args: BatchRunArgs) -> Result<()> {
    let run_started = Instant::now();
    if args.common.no_cache {
        bail!("batch run requires cache; remove --no-cache");
    }
    if args.common.clear_cache && !args.force_prepare {
        bail!("batch run only accepts --clear-cache together with --force-prepare");
    }
    if args.poll_secs == 0 {
        bail!("--poll-secs must be greater than 0");
    }

    let manifest_path = batch_manifest_path(&args.input, &args.common)?;
    if args.force_prepare || !manifest_path.exists() {
        println!("Batch run step: prepare");
        local::batch_prepare(BatchPrepareArgs {
            input: args.input.clone(),
            from: args.from,
            to: args.to,
            max_requests_per_file: args.max_requests_per_file,
            max_bytes_per_file: args.max_bytes_per_file,
            common: args.common.clone(),
        })?;
    } else {
        println!(
            "Batch run step: prepare skipped; existing manifest found at {}",
            manifest_path.display()
        );
    }
    begin_batch_manifest_run(&manifest_path)?;

    let manifest = read_json_file::<BatchManifest>(&manifest_path)?;
    if needs_submit(&manifest) {
        println!("Batch run step: submit");
        with_batch_run_error_summary(
            remote::batch_submit(BatchSubmitArgs {
                input: args.input.clone(),
                force: false,
                common: args.common.clone(),
            }),
            &manifest_path,
            run_started,
        )?;
        heartbeat_batch_manifest_run(&manifest_path)?;
    } else {
        println!("Batch run step: submit skipped; remote batch id is already recorded");
    }

    let manifest =
        with_batch_run_error_summary(wait_for_fetchable_status(&args, &manifest_path), &manifest_path, run_started)?;
    if !is_fetchable_status(&manifest.status) {
        let total_elapsed_secs = finish_batch_manifest_run(&manifest_path)?
            .unwrap_or_else(|| run_started.elapsed().as_secs());
        println!(
            "Batch run paused: remote status is {} after {} (total active {}). Re-run with --wait or run `batch status` later.",
            manifest.status,
            format_duration_hms(run_started.elapsed()),
            format_duration_hms(Duration::from_secs(total_elapsed_secs))
        );
        return Ok(());
    }

    println!("Batch run step: fetch");
    with_batch_run_error_summary(
        remote::batch_fetch(BatchFetchArgs {
            input: args.input.clone(),
            force: args.force_fetch,
            common: args.common.clone(),
        }),
        &manifest_path,
        run_started,
    )?;
    heartbeat_batch_manifest_run(&manifest_path)?;

    println!("Batch run step: import");
    with_batch_run_error_summary(
        local::batch_import(BatchImportArgs {
            input: args.input.clone(),
            output: None,
            common: args.common.clone(),
        }),
        &manifest_path,
        run_started,
    )?;
    heartbeat_batch_manifest_run(&manifest_path)?;

    println!("Batch run step: health");
    with_batch_run_error_summary(
        report::batch_health(BatchHealthArgs {
            input: args.input.clone(),
            common: args.common.clone(),
        }),
        &manifest_path,
        run_started,
    )?;
    heartbeat_batch_manifest_run(&manifest_path)?;

    if args.skip_verify {
        println!("Batch run step: verify skipped");
    } else {
        println!("Batch run step: verify");
        with_batch_run_error_summary(
            report::batch_verify(BatchVerifyArgs {
                input: args.input.clone(),
                common: args.common.clone(),
            }),
            &manifest_path,
            run_started,
        )?;
        heartbeat_batch_manifest_run(&manifest_path)?;
    }
    if let Some(output) = args.output {
        println!("Batch run step: assemble EPUB");
        let mut common = args.common;
        common.partial_from_cache = true;
        common.keep_cache = true;
        common.usage_only = false;
        common.clear_cache = false;
        with_batch_run_error_summary(
            translate_command(TranslateArgs {
                input: args.input,
                output: Some(output),
                from: None,
                to: None,
                common,
            }),
            &manifest_path,
            run_started,
        )?;
        heartbeat_batch_manifest_run(&manifest_path)?;
    }
    let total_elapsed_secs = finish_batch_manifest_run(&manifest_path)?
        .unwrap_or_else(|| run_started.elapsed().as_secs());
    println!(
        "Batch run complete: elapsed {} | total active {}",
        format_duration_hms(run_started.elapsed()),
        format_duration_hms(Duration::from_secs(total_elapsed_secs))
    );
    Ok(())
}

fn with_batch_run_error_summary<T>(
    result: Result<T>,
    manifest_path: &Path,
    started: Instant,
) -> Result<T> {
    result.map_err(|err| err.context(batch_run_error_summary(manifest_path, started)))
}

fn batch_run_error_summary(manifest_path: &Path, started: Instant) -> String {
    let total_active_elapsed_secs = finish_batch_manifest_run(manifest_path)
        .ok()
        .flatten()
        .unwrap_or_else(|| started.elapsed().as_secs());
    let elapsed = format_duration_hms(started.elapsed());
    let total_active = format_duration_hms(Duration::from_secs(total_active_elapsed_secs));
    let Ok(manifest) = read_json_file::<BatchManifest>(manifest_path) else {
        return format!("Batch run summary at error: elapsed {elapsed} | total active {total_active}");
    };
    let work_items_path = manifest_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(WORK_ITEMS_FILE);
    let state_counts = read_jsonl_file::<WorkItem>(&work_items_path)
        .map(|items| owned_state_counts(&items))
        .unwrap_or_default();
    let local_imported = state_counts.get("local_imported").copied().unwrap_or(0);
    let local_pending = state_counts.get("local_pending").copied().unwrap_or(0);
    let local_exhausted = state_counts.get("local_exhausted").copied().unwrap_or(0);
    let submitted = state_counts.get("submitted").copied().unwrap_or(0);
    let prepared = state_counts.get("prepared").copied().unwrap_or(0);
    let remote_completed = remote_completed_requests(&manifest);
    format!(
        "Batch run summary at error: elapsed {elapsed} | total active {total_active} | requests {} | remote done {remote_completed}/{} | imported {} | failed {} | rejected {} | local_imported {local_imported} | local_pending {local_pending} | local_exhausted {local_exhausted} | submitted {submitted} | prepared {prepared} | status {}",
        manifest.request_count,
        manifest.request_count,
        manifest.imported_count,
        manifest.failed_count,
        manifest.rejected_count,
        manifest.status
    )
}

fn owned_state_counts(work_items: &[WorkItem]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for item in work_items {
        *counts.entry(item.state.clone()).or_insert(0) += 1;
    }
    counts
}

fn batch_manifest_path(input: &Path, common: &CommonArgs) -> Result<PathBuf> {
    let cache = CacheStore::from_args(input, common)?;
    Ok(cache.dir.join(BATCH_DIR).join(BATCH_MANIFEST_FILE))
}

fn needs_submit(manifest: &BatchManifest) -> bool {
    manifest.parts.is_empty() && manifest.batch_id.is_none()
        || manifest.parts.iter().any(|part| part.batch_id.is_none())
}

fn wait_for_fetchable_status(args: &BatchRunArgs, manifest_path: &Path) -> Result<BatchManifest> {
    let started = Instant::now();
    let mut poll_count = 0usize;
    loop {
        poll_count += 1;
        let (manifest, _) = remote::refresh_batch_status(&args.input, &args.common)?;
        heartbeat_batch_manifest_run(manifest_path)?;
        if is_fetchable_status(&manifest.status) || !args.wait {
            println!(
                "Batch run status: {}",
                batch_wait_status_message(&manifest, poll_count, started.elapsed(), None)
            );
            return Ok(manifest);
        }
        if let Some(max_wait_secs) = args.max_wait_secs
            && started.elapsed() >= Duration::from_secs(max_wait_secs)
        {
            println!(
                "Batch run wait limit reached: {}",
                batch_wait_status_message(&manifest, poll_count, started.elapsed(), None)
            );
            return Ok(manifest);
        }
        println!(
            "Batch run waiting: {}",
            batch_wait_status_message(
                &manifest,
                poll_count,
                started.elapsed(),
                Some(args.poll_secs)
            )
        );
        std::thread::sleep(Duration::from_secs(args.poll_secs));
    }
}

pub(super) fn begin_batch_manifest_run(manifest_path: &Path) -> Result<()> {
    let mut manifest: BatchManifest = read_json_file(manifest_path)?;
    recover_open_batch_manifest_run(&mut manifest);
    let now = chrono::Utc::now().to_rfc3339();
    manifest.current_run_started_at = Some(now.clone());
    manifest.current_run_heartbeat_at = Some(now.clone());
    manifest.updated_at = now;
    write_json_pretty_atomic(manifest_path, &manifest)
}

pub(super) fn heartbeat_batch_manifest_run(manifest_path: &Path) -> Result<()> {
    let mut manifest: BatchManifest = read_json_file(manifest_path)?;
    if manifest.current_run_started_at.is_none() {
        return Ok(());
    }
    let now = chrono::Utc::now().to_rfc3339();
    manifest.current_run_heartbeat_at = Some(now.clone());
    manifest.updated_at = now;
    write_json_pretty_atomic(manifest_path, &manifest)
}

pub(super) fn finish_batch_manifest_run(manifest_path: &Path) -> Result<Option<u64>> {
    if !manifest_path.exists() {
        return Ok(None);
    }
    let mut manifest: BatchManifest = read_json_file(manifest_path)?;
    finish_open_batch_manifest_run(&mut manifest);
    let total = manifest.active_elapsed_secs;
    manifest.updated_at = chrono::Utc::now().to_rfc3339();
    write_json_pretty_atomic(manifest_path, &manifest)?;
    Ok(Some(total))
}

fn recover_open_batch_manifest_run(manifest: &mut BatchManifest) {
    let Some(started_at) = manifest.current_run_started_at.take() else {
        manifest.current_run_heartbeat_at = None;
        return;
    };
    let heartbeat_at = manifest
        .current_run_heartbeat_at
        .take()
        .unwrap_or_else(|| started_at.clone());
    manifest.active_elapsed_secs = manifest
        .active_elapsed_secs
        .saturating_add(elapsed_secs_between(&started_at, &heartbeat_at));
}

fn finish_open_batch_manifest_run(manifest: &mut BatchManifest) {
    let Some(started_at) = manifest.current_run_started_at.take() else {
        manifest.current_run_heartbeat_at = None;
        return;
    };
    let finished_at = chrono::Utc::now().to_rfc3339();
    manifest.current_run_heartbeat_at = None;
    manifest.active_elapsed_secs = manifest
        .active_elapsed_secs
        .saturating_add(elapsed_secs_between(&started_at, &finished_at));
}

fn elapsed_secs_between(started_at: &str, finished_at: &str) -> u64 {
    let Ok(started) = chrono::DateTime::parse_from_rfc3339(started_at) else {
        return 0;
    };
    let Ok(finished) = chrono::DateTime::parse_from_rfc3339(finished_at) else {
        return 0;
    };
    finished
        .signed_duration_since(started)
        .num_seconds()
        .max(0) as u64
}

fn is_fetchable_status(status: &str) -> bool {
    matches!(status, "completed" | "failed" | "expired" | "cancelled")
}

fn batch_wait_status_message(
    manifest: &BatchManifest,
    poll_count: usize,
    elapsed: Duration,
    next_poll_secs: Option<u64>,
) -> String {
    let completed = remote_completed_requests(manifest);
    let failed = manifest.failed_count;
    let total = manifest.request_count;
    let part_summary = part_status_summary(manifest);
    let mut message = format!(
        "poll #{poll_count} | elapsed {} | status {} | parts {} | remote {completed}/{total} done, {failed} failed",
        format_duration_hms(elapsed),
        manifest.status,
        part_summary
    );
    if let Some(next_poll_secs) = next_poll_secs {
        message.push_str(&format!(" | next check in {next_poll_secs}s"));
    }
    message
}

fn remote_completed_requests(manifest: &BatchManifest) -> usize {
    if manifest.parts.is_empty() {
        return if manifest.status == "completed" {
            manifest.request_count
        } else {
            0
        };
    }
    manifest
        .parts
        .iter()
        .map(|part| {
            if part.completed_count > 0 || part.status != "completed" {
                part.completed_count
            } else {
                part.request_count
            }
        })
        .sum()
}

fn part_status_summary(manifest: &BatchManifest) -> String {
    if manifest.parts.is_empty() {
        return "1".to_string();
    }
    let mut counts = BTreeMap::<&str, usize>::new();
    for part in &manifest.parts {
        *counts.entry(part.status.as_str()).or_insert(0) += 1;
    }
    counts
        .into_iter()
        .map(|(status, count)| format!("{status}:{count}"))
        .collect::<Vec<_>>()
        .join(",")
}

pub(super) fn format_duration_hms(duration: Duration) -> String {
    let total = duration.as_secs();
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let seconds = total % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_run_fetchable_statuses_are_terminal() {
        assert!(is_fetchable_status("completed"));
        assert!(is_fetchable_status("failed"));
        assert!(is_fetchable_status("expired"));
        assert!(is_fetchable_status("cancelled"));
        assert!(!is_fetchable_status("in_progress"));
        assert!(!is_fetchable_status("validating"));
    }

    #[test]
    fn batch_run_submit_detection_respects_parts() {
        let mut manifest = BatchManifest {
            schema_version: BATCH_SCHEMA_VERSION,
            input_sha256: "sha".to_string(),
            provider: "openai".to_string(),
            model: DEFAULT_OPENAI_MODEL.to_string(),
            endpoint: "/v1/responses".to_string(),
            completion_window: "24h".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            active_elapsed_secs: 0,
            current_run_started_at: None,
            current_run_heartbeat_at: None,
            request_file: REQUESTS_FILE.to_string(),
            work_items_file: WORK_ITEMS_FILE.to_string(),
            request_count: 1,
            parts: Vec::new(),
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
        assert!(needs_submit(&manifest));

        manifest.batch_id = Some("batch_1".to_string());
        assert!(!needs_submit(&manifest));

        manifest.parts.push(BatchPart {
            index: 1,
            request_file: request_part_file_name(1),
            request_count: 1,
            request_bytes: 10,
            file_id: Some("file_1".to_string()),
            batch_id: Some("batch_1".to_string()),
            status: "in_progress".to_string(),
            output_file_id: None,
            error_file_id: None,
            output_file: None,
            error_file: None,
            completed_count: 0,
            failed_count: 0,
        });
        assert!(!needs_submit(&manifest));

        manifest.parts.push(BatchPart {
            index: 2,
            request_file: request_part_file_name(2),
            request_count: 1,
            request_bytes: 10,
            file_id: None,
            batch_id: None,
            status: "prepared".to_string(),
            output_file_id: None,
            error_file_id: None,
            output_file: None,
            error_file: None,
            completed_count: 0,
            failed_count: 0,
        });
        assert!(needs_submit(&manifest));
    }

    #[test]
    fn batch_wait_status_message_includes_remote_counts() {
        let mut manifest = BatchManifest {
            schema_version: BATCH_SCHEMA_VERSION,
            input_sha256: "sha".to_string(),
            provider: "openai".to_string(),
            model: DEFAULT_OPENAI_MODEL.to_string(),
            endpoint: "/v1/responses".to_string(),
            completion_window: "24h".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            active_elapsed_secs: 0,
            current_run_started_at: None,
            current_run_heartbeat_at: None,
            request_file: REQUESTS_FILE.to_string(),
            work_items_file: WORK_ITEMS_FILE.to_string(),
            request_count: 10,
            parts: Vec::new(),
            file_id: None,
            batch_id: Some("batch_1".to_string()),
            status: "in_progress".to_string(),
            output_file_id: None,
            error_file_id: None,
            output_file: None,
            error_file: None,
            imported_count: 0,
            failed_count: 1,
            rejected_count: 0,
        };
        manifest.parts.push(BatchPart {
            index: 1,
            request_file: request_part_file_name(1),
            request_count: 10,
            request_bytes: 100,
            file_id: Some("file_1".to_string()),
            batch_id: Some("batch_1".to_string()),
            status: "in_progress".to_string(),
            output_file_id: None,
            error_file_id: None,
            output_file: None,
            error_file: None,
            completed_count: 4,
            failed_count: 1,
        });

        let message =
            batch_wait_status_message(&manifest, 3, Duration::from_secs(65), Some(30));

        assert!(message.contains("poll #3"));
        assert!(message.contains("elapsed 00:01:05"));
        assert!(message.contains("status in_progress"));
        assert!(message.contains("parts in_progress:1"));
        assert!(message.contains("remote 4/10 done, 1 failed"));
        assert!(message.contains("next check in 30s"));
    }

    #[test]
    fn batch_manifest_run_recovery_uses_last_heartbeat() {
        let mut manifest = BatchManifest {
            schema_version: BATCH_SCHEMA_VERSION,
            input_sha256: "sha".to_string(),
            provider: "openai".to_string(),
            model: DEFAULT_OPENAI_MODEL.to_string(),
            endpoint: "/v1/responses".to_string(),
            completion_window: "24h".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            active_elapsed_secs: 10,
            current_run_started_at: Some("2026-01-01T00:00:00Z".to_string()),
            current_run_heartbeat_at: Some("2026-01-01T00:02:00Z".to_string()),
            request_file: REQUESTS_FILE.to_string(),
            work_items_file: WORK_ITEMS_FILE.to_string(),
            request_count: 1,
            parts: Vec::new(),
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

        recover_open_batch_manifest_run(&mut manifest);

        assert_eq!(manifest.active_elapsed_secs, 130);
        assert!(manifest.current_run_started_at.is_none());
        assert!(manifest.current_run_heartbeat_at.is_none());
    }
}
