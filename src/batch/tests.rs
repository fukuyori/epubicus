use super::local::{batch_import, batch_prepare};
use super::*;
use crate::config::{
    CommonArgs, DEFAULT_CLAUDE_BASE_URL, DEFAULT_CONCURRENCY, DEFAULT_MAX_CHARS_PER_REQUEST,
    DEFAULT_OLLAMA_HOST, DEFAULT_OPENAI_BASE_URL,
};
use zip::{
    CompressionMethod, ZipWriter,
    write::{FileOptions, SimpleFileOptions},
};

#[test]
fn text_hash_is_stable_and_short() {
    assert_eq!(hash_text("abc"), hash_text("abc"));
    assert_eq!(hash_text("abc").len(), 32);
    assert_ne!(hash_text("abc"), hash_text("abcd"));
}

#[test]
fn batch_range_defaults_to_full_spine() -> Result<()> {
    let range = normalize_batch_range(None, None, 3)?;
    assert_eq!(range.collect::<Vec<_>>(), vec![1, 2, 3]);
    Ok(())
}

#[test]
fn batch_prepare_writes_manifest_work_items_and_requests() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let input = dir.path().join("minimal.epub");
    write_minimal_epub(&input)?;
    let args = BatchPrepareArgs {
        input: input.clone(),
        from: Some(1),
        to: Some(1),
        common: common_args(dir.path().join("cache")),
    };

    batch_prepare(args)?;

    let cache = CacheStore::from_args(&input, &common_args(dir.path().join("cache")))?;
    let batch_dir = cache.dir.join(BATCH_DIR);
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(batch_dir.join(BATCH_MANIFEST_FILE))?)?;
    assert_eq!(manifest["provider"], "openai");
    assert_eq!(manifest["model"], DEFAULT_OPENAI_MODEL);
    assert_eq!(manifest["endpoint"], "/v1/responses");
    assert_eq!(manifest["request_count"], 2);

    let work_items = read_jsonl_values(&batch_dir.join(WORK_ITEMS_FILE))?;
    let requests = read_jsonl_values(&batch_dir.join(REQUESTS_FILE))?;
    assert_eq!(work_items.len(), 2);
    assert_eq!(requests.len(), 2);
    assert_eq!(work_items[0]["state"], "prepared");
    assert_eq!(work_items[0]["page_index"], 1);
    assert_eq!(work_items[0]["block_index"], 1);
    assert_eq!(requests[0]["method"], "POST");
    assert_eq!(requests[0]["url"], "/v1/responses");
    assert_eq!(requests[0]["body"]["model"], DEFAULT_OPENAI_MODEL);
    assert_eq!(requests[0]["custom_id"], work_items[0]["custom_id"]);
    assert!(
        requests[0]["body"]["input"]
            .as_str()
            .unwrap()
            .contains("<source>")
    );
    Ok(())
}

#[test]
fn batch_health_reports_local_workspace_state() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let input = dir.path().join("minimal.epub");
    write_minimal_epub(&input)?;
    let cache_root = dir.path().join("cache");
    prepare_minimal_batch(&input, &cache_root)?;

    let health = report::collect_batch_health(&BatchHealthArgs {
        input,
        common: common_args(cache_root),
    })?;

    assert_eq!(health.manifest_status.as_deref(), Some("prepared"));
    assert_eq!(health.manifest_request_count, Some(2));
    assert_eq!(health.request_count, 2);
    assert_eq!(health.work_item_count, 2);
    assert_eq!(health.state_counts.get("prepared"), Some(&2));
    assert_eq!(health.cache_backed_items, 0);
    assert_eq!(health.rejected_file_count, 0);
    assert_eq!(health.error_file_count, 0);
    Ok(())
}

#[test]
fn batch_health_reports_remote_manifest_and_pending_age() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let input = dir.path().join("minimal.epub");
    write_minimal_epub(&input)?;
    let cache_root = dir.path().join("cache");
    let (batch_dir, mut work_items) = prepare_minimal_batch(&input, &cache_root)?;

    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(batch_dir.join(BATCH_MANIFEST_FILE))?)?;
    manifest["batch_id"] = serde_json::Value::String("batch_123".to_string());
    manifest["output_file_id"] = serde_json::Value::String("file_output".to_string());
    manifest["error_file_id"] = serde_json::Value::String("file_error".to_string());
    manifest["failed_count"] = serde_json::Value::Number(2.into());
    fs::write(
        batch_dir.join(BATCH_MANIFEST_FILE),
        serde_json::to_vec_pretty(&manifest)?,
    )?;

    let old_update = (chrono::Utc::now() - chrono::Duration::hours(2)).to_rfc3339();
    work_items[0]["updated_at"] = serde_json::Value::String(old_update);
    write_jsonl_values(&batch_dir.join(WORK_ITEMS_FILE), &work_items)?;

    let health = report::collect_batch_health(&BatchHealthArgs {
        input,
        common: common_args(cache_root),
    })?;

    assert_eq!(health.manifest_batch_id.as_deref(), Some("batch_123"));
    assert_eq!(
        health.manifest_output_file_id.as_deref(),
        Some("file_output")
    );
    assert_eq!(health.manifest_error_file_id.as_deref(), Some("file_error"));
    assert_eq!(health.manifest_failed_count, Some(2));
    assert!(health.oldest_pending_age_secs.unwrap_or_default() >= 7_000);
    Ok(())
}

#[test]
fn batch_health_reports_imported_cache_entries() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let input = dir.path().join("minimal.epub");
    write_minimal_epub(&input)?;
    let cache_root = dir.path().join("cache");
    let (_batch_dir, work_items) = prepare_minimal_batch(&input, &cache_root)?;
    let output_path = dir.path().join("output.jsonl");
    write_fixture_output(&output_path, &work_items, |source| {
        if source.contains("⟦E") {
            "こんにちは、⟦E1⟧世界⟦/E1⟧。".to_string()
        } else {
            "これは有効な日本語訳です。".to_string()
        }
    })?;
    batch_import(BatchImportArgs {
        input: input.clone(),
        output: Some(output_path),
        common: common_args(cache_root.clone()),
    })?;

    let health = report::collect_batch_health(&BatchHealthArgs {
        input,
        common: common_args(cache_root),
    })?;

    assert_eq!(health.manifest_status.as_deref(), Some("imported"));
    assert_eq!(health.state_counts.get("imported"), Some(&2));
    assert_eq!(health.cache_backed_items, 2);
    let report = health.import_report.context("missing import report")?;
    assert_eq!(report.imported_count, 2);
    assert_eq!(report.rejected_count, 0);
    Ok(())
}

#[test]
fn batch_verify_accepts_prepared_workspace() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let input = dir.path().join("minimal.epub");
    write_minimal_epub(&input)?;
    let cache_root = dir.path().join("cache");
    prepare_minimal_batch(&input, &cache_root)?;

    let report = report::collect_batch_verify(&BatchVerifyArgs {
        input,
        common: common_args(cache_root),
    })?;

    assert_eq!(report.expected_count, 2);
    assert_eq!(report.work_item_count, 2);
    assert!(!report.has_findings());
    Ok(())
}

#[test]
fn batch_verify_detects_stale_work_item_hashes() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let input = dir.path().join("minimal.epub");
    write_minimal_epub(&input)?;
    let cache_root = dir.path().join("cache");
    let (batch_dir, mut work_items) = prepare_minimal_batch(&input, &cache_root)?;
    work_items[0]["source_hash"] = serde_json::Value::String("stale".to_string());
    write_jsonl_values(&batch_dir.join(WORK_ITEMS_FILE), &work_items)?;

    let report = report::collect_batch_verify(&BatchVerifyArgs {
        input,
        common: common_args(cache_root),
    })?;

    assert_eq!(report.stale.len(), 1);
    assert!(report.stale[0].reason.contains("source_hash changed"));
    Ok(())
}

#[test]
fn batch_verify_detects_imported_state_without_cache() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let input = dir.path().join("minimal.epub");
    write_minimal_epub(&input)?;
    let cache_root = dir.path().join("cache");
    let (batch_dir, mut work_items) = prepare_minimal_batch(&input, &cache_root)?;
    work_items[0]["state"] = serde_json::Value::String("imported".to_string());
    write_jsonl_values(&batch_dir.join(WORK_ITEMS_FILE), &work_items)?;

    let report = report::collect_batch_verify(&BatchVerifyArgs {
        input,
        common: common_args(cache_root),
    })?;

    assert_eq!(report.cache_conflict.len(), 1);
    assert!(
        report.cache_conflict[0]
            .reason
            .contains("cache entry is missing")
    );
    Ok(())
}

#[test]
fn remote_batch_response_updates_manifest_ids_and_status() {
    let mut manifest = fixture_manifest();
    let remote = OpenAiBatch {
        id: "batch_123".to_string(),
        status: "completed".to_string(),
        output_file_id: Some("file_output".to_string()),
        error_file_id: Some("file_error".to_string()),
        request_counts: Some(OpenAiBatchRequestCounts { failed: Some(3) }),
    };

    remote::apply_remote_batch(&mut manifest, &remote);

    assert_eq!(manifest.batch_id.as_deref(), Some("batch_123"));
    assert_eq!(manifest.status, "completed");
    assert_eq!(manifest.output_file_id.as_deref(), Some("file_output"));
    assert_eq!(manifest.error_file_id.as_deref(), Some("file_error"));
    assert_eq!(manifest.failed_count, 3);
}

#[test]
fn batch_import_writes_valid_output_to_cache() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let input = dir.path().join("minimal.epub");
    write_minimal_epub(&input)?;
    let cache_root = dir.path().join("cache");
    let (batch_dir, work_items) = prepare_minimal_batch(&input, &cache_root)?;
    let output_path = dir.path().join("output.jsonl");
    let mut output = File::create(&output_path)?;
    for item in &work_items {
        let custom_id = item["custom_id"].as_str().context("missing custom_id")?;
        let source_text = item["source_text"]
            .as_str()
            .context("missing source_text")?;
        let translated = if source_text.contains("⟦E") {
            "こんにちは、⟦E1⟧世界⟦/E1⟧。"
        } else {
            "これは有効な日本語訳です。"
        };
        writeln!(
            output,
            "{}",
            serde_json::json!({
                "custom_id": custom_id,
                "response": {
                    "status_code": 200,
                    "body": {
                        "output_text": translated
                    }
                },
                "error": null
            })
        )?;
    }
    output.flush()?;

    batch_import(BatchImportArgs {
        input: input.clone(),
        output: Some(output_path),
        common: common_args(cache_root.clone()),
    })?;

    let imported_cache = CacheStore::from_args(&input, &common_args(cache_root))?;
    for item in &work_items {
        let key = item["cache_key"].as_str().context("missing cache_key")?;
        assert!(imported_cache.peek(key).is_some());
    }
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(batch_dir.join(BATCH_MANIFEST_FILE))?)?;
    assert_eq!(manifest["imported_count"], 2);
    assert_eq!(manifest["rejected_count"], 0);
    assert!(batch_dir.join(IMPORT_REPORT_FILE).exists());
    Ok(())
}

#[test]
fn batch_import_accepts_reordered_output() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let input = dir.path().join("minimal.epub");
    write_minimal_epub(&input)?;
    let cache_root = dir.path().join("cache");
    let (batch_dir, mut work_items) = prepare_minimal_batch(&input, &cache_root)?;
    work_items.reverse();
    let output_path = dir.path().join("output.jsonl");
    write_fixture_output(&output_path, &work_items, |source| {
        if source.contains("⟦E") {
            "こんにちは、⟦E1⟧世界⟦/E1⟧。".to_string()
        } else {
            "これは有効な日本語訳です。".to_string()
        }
    })?;

    batch_import(BatchImportArgs {
        input: input.clone(),
        output: Some(output_path),
        common: common_args(cache_root.clone()),
    })?;

    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(batch_dir.join(BATCH_MANIFEST_FILE))?)?;
    assert_eq!(manifest["imported_count"], 2);
    assert_eq!(manifest["rejected_count"], 0);
    Ok(())
}

#[test]
fn batch_import_defaults_to_fetched_output_file() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let input = dir.path().join("minimal.epub");
    write_minimal_epub(&input)?;
    let cache_root = dir.path().join("cache");
    let (batch_dir, work_items) = prepare_minimal_batch(&input, &cache_root)?;
    let output_path = batch_dir.join(OUTPUT_FILE);
    write_fixture_output(&output_path, &work_items, |source| {
        if source.contains("⟦E") {
            "こんにちは、⟦E1⟧世界⟦/E1⟧。".to_string()
        } else {
            "これは有効な日本語訳です。".to_string()
        }
    })?;

    batch_import(BatchImportArgs {
        input: input.clone(),
        output: None,
        common: common_args(cache_root.clone()),
    })?;

    let imported_cache = CacheStore::from_args(&input, &common_args(cache_root))?;
    for item in &work_items {
        let key = item["cache_key"].as_str().context("missing cache_key")?;
        assert!(imported_cache.peek(key).is_some());
    }
    Ok(())
}

#[test]
fn batch_reroute_local_marks_selected_state() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let input = dir.path().join("minimal.epub");
    write_minimal_epub(&input)?;
    let cache_root = dir.path().join("cache");
    let (batch_dir, mut work_items) = prepare_minimal_batch(&input, &cache_root)?;
    work_items[0]["state"] = serde_json::Value::String("rejected".to_string());
    write_jsonl_values(&batch_dir.join(WORK_ITEMS_FILE), &work_items)?;

    let summary = reroute::reroute_local_items(&BatchRerouteLocalArgs {
        input,
        states: vec!["rejected".to_string()],
        remaining: false,
        endgame_threshold: None,
        limit: None,
        priority: BatchPriority::PageOrder,
        common: common_args(cache_root),
    })?;

    assert_eq!(summary.selected_count, 1);
    let updated = read_jsonl_values(&batch_dir.join(WORK_ITEMS_FILE))?;
    assert_eq!(updated[0]["state"], "local_pending");
    assert_eq!(updated[1]["state"], "prepared");
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(batch_dir.join(BATCH_MANIFEST_FILE))?)?;
    assert_eq!(manifest["status"], "local_pending");
    Ok(())
}

#[test]
fn batch_reroute_local_respects_endgame_threshold() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let input = dir.path().join("minimal.epub");
    write_minimal_epub(&input)?;
    let cache_root = dir.path().join("cache");
    let (batch_dir, _work_items) = prepare_minimal_batch(&input, &cache_root)?;

    let summary = reroute::reroute_local_items(&BatchRerouteLocalArgs {
        input,
        states: Vec::new(),
        remaining: false,
        endgame_threshold: Some(1),
        limit: None,
        priority: BatchPriority::PageOrder,
        common: common_args(cache_root),
    })?;

    assert_eq!(summary.selected_count, 0);
    let updated = read_jsonl_values(&batch_dir.join(WORK_ITEMS_FILE))?;
    assert_eq!(updated[0]["state"], "prepared");
    assert_eq!(updated[1]["state"], "prepared");
    Ok(())
}

#[test]
fn batch_reroute_local_short_first_honors_limit() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let input = dir.path().join("minimal.epub");
    write_minimal_epub(&input)?;
    let cache_root = dir.path().join("cache");
    let (batch_dir, mut work_items) = prepare_minimal_batch(&input, &cache_root)?;
    work_items[0]["source_chars"] = serde_json::json!(500);
    work_items[1]["source_chars"] = serde_json::json!(1);
    write_jsonl_values(&batch_dir.join(WORK_ITEMS_FILE), &work_items)?;

    let summary = reroute::reroute_local_items(&BatchRerouteLocalArgs {
        input,
        states: Vec::new(),
        remaining: true,
        endgame_threshold: None,
        limit: Some(1),
        priority: BatchPriority::ShortFirst,
        common: common_args(cache_root),
    })?;

    assert_eq!(summary.selected_count, 1);
    let updated = read_jsonl_values(&batch_dir.join(WORK_ITEMS_FILE))?;
    assert_eq!(updated[0]["state"], "prepared");
    assert_eq!(updated[1]["state"], "local_pending");
    Ok(())
}

#[test]
fn batch_translate_local_marks_cached_pending_items_imported() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let input = dir.path().join("minimal.epub");
    write_minimal_epub(&input)?;
    let cache_root = dir.path().join("cache");
    let (batch_dir, mut work_items) = prepare_minimal_batch(&input, &cache_root)?;
    for item in &mut work_items {
        item["state"] = serde_json::Value::String("local_pending".to_string());
    }
    write_jsonl_values(&batch_dir.join(WORK_ITEMS_FILE), &work_items)?;
    let mut cache = CacheStore::from_args(&input, &common_args(cache_root.clone()))?;
    let key = work_items[0]["cache_key"]
        .as_str()
        .context("missing cache_key")?;
    cache.insert(crate::cache::CacheRecord {
        key: key.to_string(),
        translated: "これは有効な日本語訳です。".to_string(),
        provider: "ollama".to_string(),
        model: DEFAULT_OPENAI_MODEL.to_string(),
        at: chrono::Utc::now().to_rfc3339(),
    })?;

    reroute::batch_translate_local(BatchTranslateLocalArgs {
        input,
        limit: Some(1),
        priority: BatchPriority::PageOrder,
        common: common_args(cache_root),
    })?;

    let updated = read_jsonl_values(&batch_dir.join(WORK_ITEMS_FILE))?;
    assert_eq!(updated[0]["state"], "local_imported");
    assert_eq!(updated[1]["state"], "local_pending");
    Ok(())
}

#[test]
fn batch_translate_local_short_first_honors_limit() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let input = dir.path().join("minimal.epub");
    write_minimal_epub(&input)?;
    let cache_root = dir.path().join("cache");
    let (batch_dir, mut work_items) = prepare_minimal_batch(&input, &cache_root)?;
    for item in &mut work_items {
        item["state"] = serde_json::Value::String("local_pending".to_string());
    }
    work_items[0]["source_chars"] = serde_json::json!(500);
    work_items[1]["source_chars"] = serde_json::json!(1);
    write_jsonl_values(&batch_dir.join(WORK_ITEMS_FILE), &work_items)?;

    let mut cache = CacheStore::from_args(&input, &common_args(cache_root.clone()))?;
    for item in &work_items {
        let key = item["cache_key"].as_str().context("missing cache_key")?;
        cache.insert(crate::cache::CacheRecord {
            key: key.to_string(),
            translated: "これは有効な日本語訳です。".to_string(),
            provider: "ollama".to_string(),
            model: DEFAULT_OPENAI_MODEL.to_string(),
            at: chrono::Utc::now().to_rfc3339(),
        })?;
    }

    reroute::batch_translate_local(BatchTranslateLocalArgs {
        input,
        limit: Some(1),
        priority: BatchPriority::ShortFirst,
        common: common_args(cache_root),
    })?;

    let updated = read_jsonl_values(&batch_dir.join(WORK_ITEMS_FILE))?;
    assert_eq!(updated[0]["state"], "local_pending");
    assert_eq!(updated[1]["state"], "local_imported");
    Ok(())
}

#[test]
fn batch_import_rejects_invalid_translation_without_caching() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let input = dir.path().join("minimal.epub");
    write_minimal_epub(&input)?;
    let cache_root = dir.path().join("cache");
    let (batch_dir, work_items) = prepare_minimal_batch(&input, &cache_root)?;
    let output_path = dir.path().join("output.jsonl");
    write_fixture_output(&output_path, &work_items, |source| source.to_string())?;

    batch_import(BatchImportArgs {
        input: input.clone(),
        output: Some(output_path),
        common: common_args(cache_root.clone()),
    })?;

    let imported_cache = CacheStore::from_args(&input, &common_args(cache_root))?;
    for item in &work_items {
        let key = item["cache_key"].as_str().context("missing cache_key")?;
        assert!(imported_cache.peek(key).is_none());
    }
    let rejected = read_jsonl_values(&batch_dir.join(REJECTED_FILE))?;
    assert_eq!(rejected.len(), 2);
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(batch_dir.join(BATCH_MANIFEST_FILE))?)?;
    assert_eq!(manifest["imported_count"], 0);
    assert_eq!(manifest["rejected_count"], 2);
    Ok(())
}

#[test]
fn batch_import_reports_duplicate_output_custom_id() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let input = dir.path().join("minimal.epub");
    write_minimal_epub(&input)?;
    let cache_root = dir.path().join("cache");
    let (batch_dir, work_items) = prepare_minimal_batch(&input, &cache_root)?;
    let output_path = dir.path().join("output.jsonl");
    let duplicate = vec![work_items[0].clone(), work_items[0].clone()];
    write_fixture_output(&output_path, &duplicate, |_source| {
        "これは有効な日本語訳です。".to_string()
    })?;

    batch_import(BatchImportArgs {
        input: input.clone(),
        output: Some(output_path),
        common: common_args(cache_root),
    })?;

    let errors = read_jsonl_values(&batch_dir.join(ERRORS_FILE))?;
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0]["error"], "duplicate output custom_id");
    Ok(())
}

fn common_args(cache_root: PathBuf) -> CommonArgs {
    CommonArgs {
        provider: Provider::Openai,
        model: None,
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
    }
}

fn read_jsonl_values(path: &Path) -> Result<Vec<serde_json::Value>> {
    let text = fs::read_to_string(path)?;
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).map_err(Into::into))
        .collect()
}

fn write_jsonl_values(path: &Path, values: &[serde_json::Value]) -> Result<()> {
    let mut file = File::create(path)?;
    for value in values {
        serde_json::to_writer(&mut file, value)?;
        writeln!(file)?;
    }
    file.flush()?;
    Ok(())
}

fn prepare_minimal_batch(
    input: &Path,
    cache_root: &Path,
) -> Result<(PathBuf, Vec<serde_json::Value>)> {
    batch_prepare(BatchPrepareArgs {
        input: input.to_path_buf(),
        from: Some(1),
        to: Some(1),
        common: common_args(cache_root.to_path_buf()),
    })?;
    let cache = CacheStore::from_args(input, &common_args(cache_root.to_path_buf()))?;
    let batch_dir = cache.dir.join(BATCH_DIR);
    let work_items = read_jsonl_values(&batch_dir.join(WORK_ITEMS_FILE))?;
    Ok((batch_dir, work_items))
}

fn fixture_manifest() -> BatchManifest {
    BatchManifest {
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
    }
}

fn write_fixture_output<F>(
    path: &Path,
    work_items: &[serde_json::Value],
    translate: F,
) -> Result<()>
where
    F: Fn(&str) -> String,
{
    let mut output = File::create(path)?;
    for item in work_items {
        let custom_id = item["custom_id"].as_str().context("missing custom_id")?;
        let source_text = item["source_text"]
            .as_str()
            .context("missing source_text")?;
        writeln!(
            output,
            "{}",
            serde_json::json!({
                "custom_id": custom_id,
                "response": {
                    "status_code": 200,
                    "body": {
                        "output_text": translate(source_text)
                    }
                },
                "error": null
            })
        )?;
    }
    output.flush()?;
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
<rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>"#,
        )?;
    zip.start_file("OEBPS/content.opf", deflated)?;
    zip.write_all(
            br#"<?xml version="1.0" encoding="UTF-8"?>
<package version="3.0" unique-identifier="bookid" xmlns="http://www.idpf.org/2007/opf">
<metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Test Book</dc:title><dc:language>en</dc:language></metadata>
<manifest><item id="c1" href="chapter1.xhtml" media-type="application/xhtml+xml"/></manifest>
<spine><itemref idref="c1"/></spine>
</package>"#,
        )?;
    zip.start_file("OEBPS/chapter1.xhtml", deflated)?;
    zip.write_all(
        br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><body>
<h1>Chapter One</h1>
<p>Hello <em>world</em>.</p>
</body></html>"#,
    )?;
    zip.finish()?;
    Ok(())
}
