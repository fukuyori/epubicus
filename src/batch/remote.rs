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
    let mut manifest: BatchManifest = read_json_file(&manifest_path)?;
    validate_batch_manifest_for_input(&manifest, &cache)?;
    if manifest.provider != Provider::Openai.to_string() {
        bail!("batch submit currently supports openai manifests only");
    }
    if manifest.request_count == 0 {
        bail!("batch submit refused an empty request file");
    }
    ensure_manifest_parts(&mut manifest);
    let submit_indices = part_indices_to_submit(&manifest, args.force)?;
    let endpoint = manifest.endpoint.clone();
    let completion_window = manifest.completion_window.clone();
    for part_index in submit_indices {
        let part = manifest
            .parts
            .iter_mut()
            .find(|part| part.index == part_index)
            .with_context(|| format!("batch manifest is missing part {part_index}"))?;
        if part.batch_id.is_some() && args.force {
            part.file_id = None;
            part.batch_id = None;
            part.status = "prepared".to_string();
            part.output_file_id = None;
            part.error_file_id = None;
            part.output_file = None;
            part.error_file = None;
            part.failed_count = 0;
        }
        let requests_path = batch_dir.join(&part.request_file);
        if !requests_path.exists() {
            bail!("requests file does not exist: {}", requests_path.display());
        }
        let file = upload_batch_file(
            &client,
            &args.common.openai_base_url,
            &api_key,
            &requests_path,
        )
        .with_context(|| format!("failed to upload batch part {}", part.index))?;
        let remote = create_openai_batch(
            &client,
            &args.common.openai_base_url,
            &api_key,
            &endpoint,
            &completion_window,
            &file.id,
        )
        .with_context(|| format!("failed to create batch part {}", part.index))?;
        apply_remote_batch_to_part(part, &remote);
        part.file_id = Some(file.id);
    }
    sync_manifest_from_parts(&mut manifest);
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
        "Submitted batch: {} part(s) | status {} | dir: {}",
        manifest.parts.len(),
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
    let fetch_plan = plan_fetch_parts(&manifest, &batch_dir, args.force)?;
    for item in &fetch_plan.output_downloads {
        download_openai_file_content(
            &client,
            &args.common.openai_base_url,
            &api_key,
            &item.file_id,
            &item.path,
            args.force,
        )
        .with_context(|| format!("failed to fetch output for batch part {}", item.index))?;
        downloaded.push(item.path.clone());
    }
    for item in &fetch_plan.error_downloads {
        download_openai_file_content(
            &client,
            &args.common.openai_base_url,
            &api_key,
            &item.file_id,
            &item.path,
            args.force,
        )
        .with_context(|| format!("failed to fetch errors for batch part {}", item.index))?;
        downloaded.push(item.path.clone());
    }

    let mut aggregate_output = None;
    let mut aggregate_error = None;
    if !fetch_plan.output_paths.is_empty() {
        let output_path = batch_dir.join(OUTPUT_FILE);
        concatenate_jsonl_files(
            &output_path,
            fetch_plan
                .output_paths
                .iter()
                .map(|(_, path)| path.as_path()),
            true,
        )?;
        aggregate_output = Some(output_path.display().to_string());
        downloaded.push(output_path);
    }
    if !fetch_plan.error_paths.is_empty() {
        let error_path = batch_dir.join(REMOTE_ERRORS_FILE);
        concatenate_jsonl_files(
            &error_path,
            fetch_plan
                .error_paths
                .iter()
                .map(|(_, path)| path.as_path()),
            true,
        )?;
        aggregate_error = Some(error_path.display().to_string());
        downloaded.push(error_path);
    }
    if aggregate_output.is_some() || aggregate_error.is_some() {
        update_downloaded_files(
            &args.input,
            &args.common,
            fetch_plan.output_paths,
            fetch_plan.error_paths,
            aggregate_output,
            aggregate_error,
        )?;
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

pub(super) fn plan_fetch_parts(
    manifest: &BatchManifest,
    batch_dir: &Path,
    force: bool,
) -> Result<FetchPlan> {
    let mut plan = FetchPlan::default();
    for part in &manifest.parts {
        if let Some(file_id) = &part.output_file_id {
            let path = batch_dir.join(output_part_file_name(part.index));
            plan.output_paths.push((part.index, path.clone()));
            if force || !path.exists() {
                plan.output_downloads.push(FetchDownload {
                    index: part.index,
                    file_id: file_id.clone(),
                    path,
                });
            }
        }
        if let Some(file_id) = &part.error_file_id {
            let path = batch_dir.join(remote_error_part_file_name(part.index));
            plan.error_paths.push((part.index, path.clone()));
            if force || !path.exists() {
                plan.error_downloads.push(FetchDownload {
                    index: part.index,
                    file_id: file_id.clone(),
                    path,
                });
            }
        }
    }
    Ok(plan)
}

#[derive(Default)]
pub(super) struct FetchPlan {
    pub(super) output_downloads: Vec<FetchDownload>,
    pub(super) error_downloads: Vec<FetchDownload>,
    pub(super) output_paths: Vec<(usize, PathBuf)>,
    pub(super) error_paths: Vec<(usize, PathBuf)>,
}

pub(super) struct FetchDownload {
    pub(super) index: usize,
    pub(super) file_id: String,
    pub(super) path: PathBuf,
}

pub(super) fn concatenate_jsonl_files<'a>(
    output_path: &Path,
    paths: impl IntoIterator<Item = &'a Path>,
    force: bool,
) -> Result<()> {
    if output_path.exists() && !force {
        bail!(
            "{} already exists; use --force to overwrite",
            output_path.display()
        );
    }
    let tmp = tmp_path(output_path);
    let mut output =
        File::create(&tmp).with_context(|| format!("failed to create {}", tmp.display()))?;
    for path in paths {
        let text = fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        if !text.trim().is_empty() {
            output.write_all(text.trim_end().as_bytes())?;
            output.write_all(b"\n")?;
        }
    }
    output.flush()?;
    fs::rename(&tmp, output_path)
        .with_context(|| format!("failed to commit {}", output_path.display()))?;
    Ok(())
}

pub(super) fn refresh_batch_status(
    input: &Path,
    common: &CommonArgs,
) -> Result<(BatchManifest, PathBuf)> {
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
    ensure_manifest_parts(&mut manifest);
    for part in &mut manifest.parts {
        let batch_id = part.batch_id.clone().with_context(|| {
            format!(
                "batch part {} has no batch_id; run batch submit first",
                part.index
            )
        })?;
        let remote = get_openai_batch(&client, &common.openai_base_url, api_key, &batch_id)
            .with_context(|| format!("failed to refresh batch part {}", part.index))?;
        apply_remote_batch_to_part(part, &remote);
    }
    sync_manifest_from_parts(&mut manifest);
    manifest.updated_at = chrono::Utc::now().to_rfc3339();
    write_json_pretty_atomic(&manifest_path, &manifest)?;
    Ok((manifest, batch_dir))
}

fn update_downloaded_files(
    input: &Path,
    common: &CommonArgs,
    output_parts: Vec<(usize, PathBuf)>,
    error_parts: Vec<(usize, PathBuf)>,
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
    for (index, path) in output_parts {
        if let Some(part) = manifest.parts.iter_mut().find(|part| part.index == index) {
            part.output_file = Some(path.display().to_string());
        }
    }
    for (index, path) in error_parts {
        if let Some(part) = manifest.parts.iter_mut().find(|part| part.index == index) {
            part.error_file = Some(path.display().to_string());
        }
    }
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
    if manifest.parts.len() > 1 {
        println!("parts:");
        for part in &manifest.parts {
            println!(
                "  part {}: {} request(s) | status {} | batch_id {}",
                part.index,
                part.request_count,
                part.status,
                part.batch_id.as_deref().unwrap_or("(missing)")
            );
        }
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
    endpoint: &str,
    completion_window: &str,
    file_id: &str,
) -> Result<OpenAiBatch> {
    let payload = serde_json::json!({
        "input_file_id": file_id,
        "endpoint": endpoint,
        "completion_window": completion_window,
    });
    request_json(
        client
            .post(format!("{}/batches", base_url.trim_end_matches('/')))
            .bearer_auth(api_key)
            .json(&payload),
    )
    .context("failed to create OpenAI batch")
}

pub(super) fn part_indices_to_submit(manifest: &BatchManifest, force: bool) -> Result<Vec<usize>> {
    let indices = manifest
        .parts
        .iter()
        .filter(|part| force || part.batch_id.is_none())
        .map(|part| part.index)
        .collect::<Vec<_>>();
    if indices.is_empty() {
        bail!(
            "batch already has recorded batch_id values for every part; use --force only if you intend to replace them"
        );
    }
    Ok(indices)
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
    let response = request.send().context("failed to send OpenAI request")?;
    let status = response.status();
    if !status.is_success() {
        let body = response
            .text()
            .unwrap_or_else(|err| format!("<failed to read error body: {err}>"));
        bail!(
            "OpenAI API returned {status}: {}",
            summarize_openai_error(&body)
        );
    }
    response
        .json::<T>()
        .context("failed to parse OpenAI JSON response")
}

fn summarize_openai_error(body: &str) -> String {
    let trimmed = body.trim();
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed)
        && let Some(error) = value.get("error")
    {
        let message = error
            .get("message")
            .and_then(|value| value.as_str())
            .unwrap_or(trimmed);
        let error_type = error.get("type").and_then(|value| value.as_str());
        let param = error.get("param").and_then(|value| value.as_str());
        let code = error.get("code").and_then(|value| value.as_str());
        let mut parts = vec![message.to_string()];
        if let Some(error_type) = error_type {
            parts.push(format!("type={error_type}"));
        }
        if let Some(param) = param {
            parts.push(format!("param={param}"));
        }
        if let Some(code) = code {
            parts.push(format!("code={code}"));
        }
        return parts.join(" | ");
    }
    const MAX_ERROR_BODY_CHARS: usize = 1000;
    let mut chars = trimmed.chars();
    let summary = chars
        .by_ref()
        .take(MAX_ERROR_BODY_CHARS)
        .collect::<String>();
    if chars.next().is_some() {
        format!("{summary}...")
    } else {
        summary
    }
}

#[cfg(test)]
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

fn apply_remote_batch_to_part(part: &mut BatchPart, remote: &OpenAiBatch) {
    part.batch_id = Some(remote.id.clone());
    part.status = remote.status.clone();
    if let Some(output_file_id) = &remote.output_file_id {
        part.output_file_id = Some(output_file_id.clone());
    }
    if let Some(error_file_id) = &remote.error_file_id {
        part.error_file_id = Some(error_file_id.clone());
    }
    if let Some(counts) = &remote.request_counts {
        part.failed_count = counts.failed.unwrap_or(part.failed_count);
    }
}

fn ensure_manifest_parts(manifest: &mut BatchManifest) {
    if !manifest.parts.is_empty() {
        return;
    }
    manifest.parts.push(BatchPart {
        index: 1,
        request_file: manifest.request_file.clone(),
        request_count: manifest.request_count,
        request_bytes: 0,
        file_id: manifest.file_id.clone(),
        batch_id: manifest.batch_id.clone(),
        status: manifest.status.clone(),
        output_file_id: manifest.output_file_id.clone(),
        error_file_id: manifest.error_file_id.clone(),
        output_file: manifest.output_file.clone(),
        error_file: manifest.error_file.clone(),
        failed_count: manifest.failed_count,
    });
}

fn sync_manifest_from_parts(manifest: &mut BatchManifest) {
    let Some(first) = manifest.parts.first() else {
        return;
    };
    manifest.file_id = first.file_id.clone();
    manifest.batch_id = first.batch_id.clone();
    manifest.output_file_id = first.output_file_id.clone();
    manifest.error_file_id = first.error_file_id.clone();
    manifest.failed_count = manifest.parts.iter().map(|part| part.failed_count).sum();
    manifest.status = aggregate_part_status(&manifest.parts);
}

pub(super) fn aggregate_part_status(parts: &[BatchPart]) -> String {
    if parts.is_empty() {
        return "prepared".to_string();
    }
    if parts.iter().all(|part| part.status == "completed") {
        return "completed".to_string();
    }
    for status in ["failed", "expired", "cancelled"] {
        if parts.iter().any(|part| part.status == status) {
            return status.to_string();
        }
    }
    for status in ["cancelling", "finalizing", "in_progress", "validating"] {
        if parts.iter().any(|part| part.status == status) {
            return status.to_string();
        }
    }
    if parts.iter().all(|part| part.status == "submitted") {
        return "submitted".to_string();
    }
    "partial".to_string()
}

fn validate_batch_manifest_for_input(manifest: &BatchManifest, cache: &CacheStore) -> Result<()> {
    if manifest.input_sha256 != cache.input_sha256 {
        bail!("batch manifest input_sha256 does not match the current input EPUB");
    }
    Ok(())
}
