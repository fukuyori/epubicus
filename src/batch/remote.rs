use super::*;
use reqwest::blocking::{Client, multipart};
use std::time::Duration;

pub(super) fn batch_submit(args: BatchSubmitArgs) -> Result<()> {
    if args.common.no_cache {
        bail!("batch submit requires cache; remove --no-cache");
    }
    if args.common.clear_cache {
        bail!("batch submit does not support --clear-cache");
    }
    let api_key = openai_api_key(&args.common)?;
    let client = openai_client(&args.common)?;
    let _run_lock = acquire_input_run_lock(&args.input, "submit batch input EPUB")?;
    let cache = CacheStore::from_args(&args.input, &args.common)?;
    let batch_dir = cache.dir.join(BATCH_DIR);
    let batch_lock_path = batch_lock_path(&cache);
    let _batch_lock = FileLock::acquire(&batch_lock_path, "submit batch")?;

    let manifest_path = batch_dir.join(BATCH_MANIFEST_FILE);
    let work_items_path = batch_dir.join(WORK_ITEMS_FILE);
    let requests_path = batch_dir.join(REQUESTS_FILE);
    let mut manifest: BatchManifest = read_json_file(&manifest_path)?;
    validate_batch_manifest_for_input(&manifest, &cache)?;
    if manifest.provider != Provider::Openai.to_string() {
        bail!("batch submit currently supports openai manifests only");
    }
    if manifest.request_count == 0 {
        bail!("batch submit refused an empty request file");
    }
    if manifest.batch_id.is_some() && !args.force {
        bail!(
            "batch already has a recorded batch_id; use --force only if you intend to replace it"
        );
    }
    if !requests_path.exists() {
        bail!("requests file does not exist: {}", requests_path.display());
    }

    let file = upload_batch_file(
        &client,
        &args.common.openai_base_url,
        &api_key,
        &requests_path,
    )?;
    let remote = create_openai_batch(
        &client,
        &args.common.openai_base_url,
        &api_key,
        &manifest,
        &file.id,
    )?;
    apply_remote_batch(&mut manifest, &remote);
    manifest.file_id = Some(file.id);
    manifest.updated_at = chrono::Utc::now().to_rfc3339();

    if work_items_path.exists() {
        let mut work_items: Vec<WorkItem> = read_jsonl_file(&work_items_path)?;
        let now = chrono::Utc::now().to_rfc3339();
        for item in &mut work_items {
            if item.state == "prepared" {
                item.state = "submitted".to_string();
                item.updated_at = now.clone();
            }
        }
        write_jsonl_atomic(&work_items_path, work_items.iter())?;
    }
    write_json_pretty_atomic(&manifest_path, &manifest)?;

    println!(
        "Submitted batch: file_id {} | batch_id {} | status {} | dir: {}",
        manifest.file_id.as_deref().unwrap_or("(missing)"),
        manifest.batch_id.as_deref().unwrap_or("(missing)"),
        manifest.status,
        batch_dir.display()
    );
    Ok(())
}

pub(super) fn batch_status(args: BatchStatusArgs) -> Result<()> {
    let (manifest, _batch_dir) = refresh_batch_status(&args.input, &args.common)?;
    print_remote_status(&manifest);
    Ok(())
}

pub(super) fn batch_fetch(args: BatchFetchArgs) -> Result<()> {
    if args.common.no_cache {
        bail!("batch fetch requires cache; remove --no-cache");
    }
    if args.common.clear_cache {
        bail!("batch fetch does not support --clear-cache");
    }
    let client = openai_client(&args.common)?;
    let api_key = openai_api_key(&args.common)?;
    let (manifest, batch_dir) =
        refresh_batch_status_with_client(&args.input, &args.common, &client, &api_key)?;

    let mut downloaded = Vec::new();
    let mut output_written = None;
    let mut error_written = None;
    if let Some(output_file_id) = &manifest.output_file_id {
        let output_path = batch_dir.join(OUTPUT_FILE);
        download_openai_file_content(
            &client,
            &args.common.openai_base_url,
            &api_key,
            output_file_id,
            &output_path,
            args.force,
        )?;
        output_written = Some(output_path.display().to_string());
        downloaded.push(output_path);
    }
    if let Some(error_file_id) = &manifest.error_file_id {
        let error_path = batch_dir.join(REMOTE_ERRORS_FILE);
        download_openai_file_content(
            &client,
            &args.common.openai_base_url,
            &api_key,
            error_file_id,
            &error_path,
            args.force,
        )?;
        error_written = Some(error_path.display().to_string());
        downloaded.push(error_path);
    }
    if output_written.is_some() || error_written.is_some() {
        update_downloaded_files(&args.input, &args.common, output_written, error_written)?;
    }
    if downloaded.is_empty() {
        println!(
            "No remote output/error files are available yet | status: {}",
            manifest.status
        );
    } else {
        for path in downloaded {
            println!("Downloaded {}", path.display());
        }
    }
    Ok(())
}

fn refresh_batch_status(input: &Path, common: &CommonArgs) -> Result<(BatchManifest, PathBuf)> {
    if common.no_cache {
        bail!("batch status requires cache; remove --no-cache");
    }
    if common.clear_cache {
        bail!("batch status does not support --clear-cache");
    }
    let api_key = openai_api_key(common)?;
    let client = openai_client(common)?;
    refresh_batch_status_with_client(input, common, &client, &api_key)
}

fn refresh_batch_status_with_client(
    input: &Path,
    common: &CommonArgs,
    client: &Client,
    api_key: &str,
) -> Result<(BatchManifest, PathBuf)> {
    let _run_lock = acquire_input_run_lock(input, "refresh batch input EPUB")?;
    let cache = CacheStore::from_args(input, common)?;
    let batch_dir = cache.dir.join(BATCH_DIR);
    let batch_lock_path = batch_lock_path(&cache);
    let _batch_lock = FileLock::acquire(&batch_lock_path, "refresh batch")?;
    let manifest_path = batch_dir.join(BATCH_MANIFEST_FILE);
    let mut manifest: BatchManifest = read_json_file(&manifest_path)?;
    validate_batch_manifest_for_input(&manifest, &cache)?;
    let batch_id = manifest
        .batch_id
        .clone()
        .context("batch manifest has no batch_id; run batch submit first")?;
    let remote = get_openai_batch(&client, &common.openai_base_url, &api_key, &batch_id)?;
    apply_remote_batch(&mut manifest, &remote);
    manifest.updated_at = chrono::Utc::now().to_rfc3339();
    write_json_pretty_atomic(&manifest_path, &manifest)?;
    Ok((manifest, batch_dir))
}

fn update_downloaded_files(
    input: &Path,
    common: &CommonArgs,
    output_file: Option<String>,
    error_file: Option<String>,
) -> Result<()> {
    let _run_lock = acquire_input_run_lock(input, "update fetched batch files")?;
    let cache = CacheStore::from_args(input, common)?;
    let batch_dir = cache.dir.join(BATCH_DIR);
    let batch_lock_path = batch_lock_path(&cache);
    let _batch_lock = FileLock::acquire(&batch_lock_path, "update fetched batch files")?;
    let manifest_path = batch_dir.join(BATCH_MANIFEST_FILE);
    let mut manifest: BatchManifest = read_json_file(&manifest_path)?;
    if let Some(output_file) = output_file {
        manifest.output_file = Some(output_file);
    }
    if let Some(error_file) = error_file {
        manifest.error_file = Some(error_file);
    }
    manifest.updated_at = chrono::Utc::now().to_rfc3339();
    write_json_pretty_atomic(&manifest_path, &manifest)
}

fn print_remote_status(manifest: &BatchManifest) {
    println!(
        "Batch status: {} | batch_id: {} | file_id: {}",
        manifest.status,
        manifest.batch_id.as_deref().unwrap_or("(missing)"),
        manifest.file_id.as_deref().unwrap_or("(missing)")
    );
    println!(
        "requests: {} | imported: {} | rejected: {} | failed: {}",
        manifest.request_count,
        manifest.imported_count,
        manifest.rejected_count,
        manifest.failed_count
    );
    if let Some(output_file_id) = &manifest.output_file_id {
        println!("output_file_id: {output_file_id}");
    }
    if let Some(error_file_id) = &manifest.error_file_id {
        println!("error_file_id: {error_file_id}");
    }
}

fn openai_client(common: &CommonArgs) -> Result<Client> {
    Client::builder()
        .timeout(Duration::from_secs(common.timeout_secs))
        .build()
        .context("failed to create HTTP client")
}

fn openai_api_key(common: &CommonArgs) -> Result<String> {
    if let Some(value) = &common.openai_api_key
        && !value.trim().is_empty()
    {
        return Ok(value.clone());
    }
    if let Ok(value) = std::env::var("OPENAI_API_KEY")
        && !value.trim().is_empty()
    {
        return Ok(value);
    }
    if common.prompt_api_key {
        let value = rpassword::prompt_password("OPENAI_API_KEY: ")
            .context("failed to read OPENAI_API_KEY from prompt")?;
        if !value.trim().is_empty() {
            return Ok(value);
        }
    }
    bail!("OpenAI Batch API requires OPENAI_API_KEY, --openai-api-key, or --prompt-api-key")
}

fn upload_batch_file(
    client: &Client,
    base_url: &str,
    api_key: &str,
    requests_path: &Path,
) -> Result<OpenAiFile> {
    let file_part = multipart::Part::file(requests_path)
        .with_context(|| format!("failed to open {}", requests_path.display()))?;
    let form = multipart::Form::new()
        .text("purpose", "batch")
        .part("file", file_part);
    request_json(
        client
            .post(format!("{}/files", base_url.trim_end_matches('/')))
            .bearer_auth(api_key)
            .multipart(form),
    )
    .context("failed to upload Batch API request file")
}

fn create_openai_batch(
    client: &Client,
    base_url: &str,
    api_key: &str,
    manifest: &BatchManifest,
    file_id: &str,
) -> Result<OpenAiBatch> {
    let payload = serde_json::json!({
        "input_file_id": file_id,
        "endpoint": manifest.endpoint,
        "completion_window": manifest.completion_window,
    });
    request_json(
        client
            .post(format!("{}/batches", base_url.trim_end_matches('/')))
            .bearer_auth(api_key)
            .json(&payload),
    )
    .context("failed to create OpenAI batch")
}

fn get_openai_batch(
    client: &Client,
    base_url: &str,
    api_key: &str,
    batch_id: &str,
) -> Result<OpenAiBatch> {
    request_json(
        client
            .get(format!(
                "{}/batches/{}",
                base_url.trim_end_matches('/'),
                batch_id
            ))
            .bearer_auth(api_key),
    )
    .context("failed to retrieve OpenAI batch")
}

fn download_openai_file_content(
    client: &Client,
    base_url: &str,
    api_key: &str,
    file_id: &str,
    output_path: &Path,
    force: bool,
) -> Result<()> {
    if output_path.exists() && !force {
        bail!(
            "{} already exists; use --force to overwrite",
            output_path.display()
        );
    }
    let bytes = client
        .get(format!(
            "{}/files/{}/content",
            base_url.trim_end_matches('/'),
            file_id
        ))
        .bearer_auth(api_key)
        .send()
        .and_then(|response| response.error_for_status())
        .and_then(|response| response.bytes())
        .with_context(|| format!("failed to download OpenAI file content {file_id}"))?;
    let tmp = tmp_path(output_path);
    fs::write(&tmp, bytes).with_context(|| format!("failed to write {}", tmp.display()))?;
    fs::rename(&tmp, output_path)
        .with_context(|| format!("failed to commit {}", output_path.display()))?;
    Ok(())
}

fn request_json<T: for<'de> Deserialize<'de>>(
    request: reqwest::blocking::RequestBuilder,
) -> Result<T> {
    request
        .send()
        .and_then(|response| response.error_for_status())
        .and_then(|response| response.json::<T>())
        .map_err(Into::into)
}

pub(super) fn apply_remote_batch(manifest: &mut BatchManifest, remote: &OpenAiBatch) {
    manifest.batch_id = Some(remote.id.clone());
    manifest.status = remote.status.clone();
    if let Some(output_file_id) = &remote.output_file_id {
        manifest.output_file_id = Some(output_file_id.clone());
    }
    if let Some(error_file_id) = &remote.error_file_id {
        manifest.error_file_id = Some(error_file_id.clone());
    }
    if let Some(counts) = &remote.request_counts {
        manifest.failed_count = counts.failed.unwrap_or(manifest.failed_count);
    }
}

fn validate_batch_manifest_for_input(manifest: &BatchManifest, cache: &CacheStore) -> Result<()> {
    if manifest.input_sha256 != cache.input_sha256 {
        bail!("batch manifest input_sha256 does not match the current input EPUB");
    }
    Ok(())
}
