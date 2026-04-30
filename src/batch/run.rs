use super::*;

pub(super) fn batch_run(args: BatchRunArgs) -> Result<()> {
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

    let manifest = read_json_file::<BatchManifest>(&manifest_path)?;
    if needs_submit(&manifest) {
        println!("Batch run step: submit");
        remote::batch_submit(BatchSubmitArgs {
            input: args.input.clone(),
            force: false,
            common: args.common.clone(),
        })?;
    } else {
        println!("Batch run step: submit skipped; remote batch id is already recorded");
    }

    let manifest = wait_for_fetchable_status(&args)?;
    if !is_fetchable_status(&manifest.status) {
        println!(
            "Batch run paused: remote status is {}. Re-run with --wait or run `batch status` later.",
            manifest.status
        );
        return Ok(());
    }

    println!("Batch run step: fetch");
    remote::batch_fetch(BatchFetchArgs {
        input: args.input.clone(),
        force: args.force_fetch,
        common: args.common.clone(),
    })?;

    println!("Batch run step: import");
    local::batch_import(BatchImportArgs {
        input: args.input.clone(),
        output: None,
        common: args.common.clone(),
    })?;

    println!("Batch run step: health");
    report::batch_health(BatchHealthArgs {
        input: args.input.clone(),
        common: args.common.clone(),
    })?;

    if args.skip_verify {
        println!("Batch run step: verify skipped");
    } else {
        println!("Batch run step: verify");
        report::batch_verify(BatchVerifyArgs {
            input: args.input.clone(),
            common: args.common.clone(),
        })?;
    }
    if let Some(output) = args.output {
        println!("Batch run step: assemble EPUB");
        let mut common = args.common;
        common.partial_from_cache = true;
        common.keep_cache = true;
        common.usage_only = false;
        common.clear_cache = false;
        translate_command(TranslateArgs {
            input: args.input,
            output: Some(output),
            from: None,
            to: None,
            common,
        })?;
    }
    Ok(())
}

fn batch_manifest_path(input: &Path, common: &CommonArgs) -> Result<PathBuf> {
    let cache = CacheStore::from_args(input, common)?;
    Ok(cache.dir.join(BATCH_DIR).join(BATCH_MANIFEST_FILE))
}

fn needs_submit(manifest: &BatchManifest) -> bool {
    manifest.parts.is_empty() && manifest.batch_id.is_none()
        || manifest.parts.iter().any(|part| part.batch_id.is_none())
}

fn wait_for_fetchable_status(args: &BatchRunArgs) -> Result<BatchManifest> {
    let started = Instant::now();
    loop {
        println!("Batch run step: status");
        let (manifest, _) = remote::refresh_batch_status(&args.input, &args.common)?;
        if is_fetchable_status(&manifest.status) || !args.wait {
            return Ok(manifest);
        }
        if let Some(max_wait_secs) = args.max_wait_secs
            && started.elapsed() >= Duration::from_secs(max_wait_secs)
        {
            return Ok(manifest);
        }
        println!(
            "Batch run waiting: status {}. Next check in {} second(s).",
            manifest.status, args.poll_secs
        );
        std::thread::sleep(Duration::from_secs(args.poll_secs));
    }
}

fn is_fetchable_status(status: &str) -> bool {
    matches!(status, "completed" | "failed" | "expired" | "cancelled")
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
            failed_count: 0,
        });
        assert!(needs_submit(&manifest));
    }
}
