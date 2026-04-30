use super::*;

pub(super) struct PreparedItem {
    pub(super) work_item: WorkItem,
    pub(super) request: BatchRequestLine,
}

pub(super) struct BatchPartPlan {
    pub(super) part: BatchPart,
    pub(super) start: usize,
    pub(super) end: usize,
}

#[derive(Debug)]
pub(super) struct BatchHealth {
    pub(super) input_hash: String,
    pub(super) batch_dir: PathBuf,
    pub(super) manifest_status: Option<String>,
    pub(super) manifest_request_count: Option<usize>,
    pub(super) manifest_batch_id: Option<String>,
    pub(super) manifest_output_file_id: Option<String>,
    pub(super) manifest_error_file_id: Option<String>,
    pub(super) manifest_output_file: Option<String>,
    pub(super) manifest_failed_count: Option<usize>,
    pub(super) manifest_part_status_counts: BTreeMap<String, usize>,
    pub(super) request_count: usize,
    pub(super) work_item_count: usize,
    pub(super) state_counts: BTreeMap<String, usize>,
    pub(super) cache_backed_items: usize,
    pub(super) rejected_file_count: usize,
    pub(super) error_file_count: usize,
    pub(super) import_report: Option<ImportReport>,
    pub(super) oldest_pending_at: Option<String>,
    pub(super) oldest_pending_age_secs: Option<i64>,
}

#[derive(Debug)]
pub(super) struct BatchVerifyReport {
    pub(super) input_hash: String,
    pub(super) batch_dir: PathBuf,
    pub(super) checked_pages: usize,
    pub(super) expected_count: usize,
    pub(super) work_item_count: usize,
    pub(super) missing: Vec<VerifyFinding>,
    pub(super) stale: Vec<VerifyFinding>,
    pub(super) orphaned: Vec<VerifyFinding>,
    pub(super) cache_conflict: Vec<VerifyFinding>,
    pub(super) invalid_cache: Vec<VerifyFinding>,
}

impl BatchVerifyReport {
    pub(super) fn has_findings(&self) -> bool {
        !self.missing.is_empty()
            || !self.stale.is_empty()
            || !self.orphaned.is_empty()
            || !self.cache_conflict.is_empty()
            || !self.invalid_cache.is_empty()
    }
}

#[derive(Debug)]
pub(super) struct VerifyFinding {
    pub(super) page_index: usize,
    pub(super) block_index: usize,
    pub(super) cache_key: String,
    pub(super) reason: String,
}

impl VerifyFinding {
    pub(super) fn from_work_item(item: &WorkItem, reason: &str) -> Self {
        Self {
            page_index: item.page_index,
            block_index: item.block_index,
            cache_key: item.cache_key.clone(),
            reason: reason.to_string(),
        }
    }
}

#[derive(Deserialize, Serialize)]
pub(super) struct BatchManifest {
    pub(super) schema_version: u32,
    pub(super) input_sha256: String,
    pub(super) provider: String,
    pub(super) model: String,
    pub(super) endpoint: String,
    pub(super) completion_window: String,
    pub(super) created_at: String,
    pub(super) updated_at: String,
    pub(super) request_file: String,
    pub(super) work_items_file: String,
    pub(super) request_count: usize,
    #[serde(default)]
    pub(super) parts: Vec<BatchPart>,
    pub(super) file_id: Option<String>,
    pub(super) batch_id: Option<String>,
    pub(super) status: String,
    pub(super) output_file_id: Option<String>,
    pub(super) error_file_id: Option<String>,
    pub(super) output_file: Option<String>,
    pub(super) error_file: Option<String>,
    pub(super) imported_count: usize,
    pub(super) failed_count: usize,
    pub(super) rejected_count: usize,
}

#[derive(Clone, Deserialize, Serialize)]
pub(super) struct BatchPart {
    pub(super) index: usize,
    pub(super) request_file: String,
    pub(super) request_count: usize,
    pub(super) request_bytes: usize,
    pub(super) file_id: Option<String>,
    pub(super) batch_id: Option<String>,
    pub(super) status: String,
    pub(super) output_file_id: Option<String>,
    pub(super) error_file_id: Option<String>,
    pub(super) output_file: Option<String>,
    pub(super) error_file: Option<String>,
    pub(super) failed_count: usize,
}

#[derive(Deserialize, Serialize)]
pub(super) struct WorkItem {
    pub(super) custom_id: String,
    pub(super) cache_key: String,
    pub(super) page_index: usize,
    pub(super) block_index: usize,
    pub(super) href: String,
    pub(super) source_text: String,
    pub(super) source_hash: String,
    pub(super) prompt_hash: String,
    pub(super) source_chars: usize,
    pub(super) provider: String,
    pub(super) model: String,
    pub(super) state: String,
    pub(super) attempt: u32,
    pub(super) last_error: Option<String>,
    pub(super) updated_at: String,
}

#[derive(Clone, Deserialize, Serialize)]
pub(super) struct BatchRequestLine {
    pub(super) custom_id: String,
    pub(super) method: String,
    pub(super) url: String,
    pub(super) body: BatchResponsesBody,
}

#[derive(Clone, Deserialize, Serialize)]
pub(super) struct BatchResponsesBody {
    pub(super) model: String,
    pub(super) instructions: String,
    pub(super) input: String,
}

#[derive(Deserialize)]
pub(super) struct BatchOutputLine {
    pub(super) custom_id: String,
    pub(super) response: Option<BatchOutputResponse>,
    pub(super) error: Option<serde_json::Value>,
}

#[derive(Deserialize)]
pub(super) struct BatchOutputResponse {
    pub(super) status_code: u16,
    pub(super) body: serde_json::Value,
}

#[derive(Deserialize)]
pub(super) struct OpenAiFile {
    pub(super) id: String,
}

#[derive(Deserialize)]
pub(super) struct OpenAiBatch {
    pub(super) id: String,
    pub(super) status: String,
    pub(super) output_file_id: Option<String>,
    pub(super) error_file_id: Option<String>,
    pub(super) request_counts: Option<OpenAiBatchRequestCounts>,
}

#[derive(Deserialize)]
pub(super) struct OpenAiBatchRequestCounts {
    pub(super) failed: Option<usize>,
}

#[derive(Serialize)]
pub(super) struct RejectedLine {
    pub(super) custom_id: String,
    pub(super) cache_key: String,
    pub(super) source_hash: String,
    pub(super) error: String,
}

#[derive(Serialize)]
pub(super) struct ImportErrorLine {
    pub(super) custom_id: String,
    pub(super) error: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub(super) struct ImportReport {
    pub(super) imported_count: usize,
    #[serde(default)]
    pub(super) already_cached_count: usize,
    pub(super) rejected_count: usize,
    pub(super) error_count: usize,
    pub(super) output_file: String,
}
