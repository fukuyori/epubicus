mod io;
mod local;
mod model;
mod remote;
mod report;
mod reroute;
mod run;
mod work;

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::{self, File},
    io::{Cursor, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use quick_xml::{Reader, events::Event};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use io::*;
use model::*;
use work::*;

use crate::{
    cache::CacheStore,
    config::{
        BatchArgs, BatchCommand, BatchFetchArgs, BatchHealthArgs, BatchImportArgs,
        BatchPrepareArgs, BatchPriority, BatchRerouteLocalArgs, BatchRetryArgs, BatchRunArgs,
        BatchStatusArgs, BatchSubmitArgs, BatchTranslateLocalArgs, BatchVerifyArgs, CommonArgs,
        DEFAULT_CLAUDE_MODEL, DEFAULT_MODEL, DEFAULT_OPENAI_MODEL, Provider, TranslateArgs,
    },
    epub::{EpubBook, is_block_tag, unpack_epub},
    glossary::{GlossaryEntry, load_glossary},
    input_lock::acquire_input_run_lock,
    lock::FileLock,
    prompt::{system_prompt, user_prompt},
    translate_command,
    translator::{cache_key, extract_openai_text, validate_translation_response},
    xhtml::{collect_element_inner, encode_inline},
};

pub(super) const BATCH_DIR: &str = "batch";
pub(super) const BATCH_MANIFEST_FILE: &str = "batch_manifest.json";
pub(super) const WORK_ITEMS_FILE: &str = "work_items.jsonl";
pub(super) const REQUESTS_FILE: &str = "requests.jsonl";
pub(super) const RETRY_REQUESTS_FILE: &str = "retry_requests.jsonl";
pub(super) const OUTPUT_FILE: &str = "output.jsonl";
pub(super) const REMOTE_ERRORS_FILE: &str = "remote_errors.jsonl";
pub(super) const IMPORT_REPORT_FILE: &str = "import_report.json";
pub(super) const REJECTED_FILE: &str = "rejected.jsonl";
pub(super) const ERRORS_FILE: &str = "errors.jsonl";
pub(super) const BATCH_SCHEMA_VERSION: u32 = 1;

pub(super) fn request_part_file_name(index: usize) -> String {
    format!("requests.part-{index:04}.jsonl")
}

pub(super) fn output_part_file_name(index: usize) -> String {
    format!("output.part-{index:04}.jsonl")
}

pub(super) fn remote_error_part_file_name(index: usize) -> String {
    format!("remote_errors.part-{index:04}.jsonl")
}

pub(crate) fn batch_command(args: BatchArgs) -> Result<()> {
    match args.command {
        BatchCommand::Prepare(args) => local::batch_prepare(args),
        BatchCommand::Run(args) => run::batch_run(args),
        BatchCommand::RetryRequests(args) => local::batch_retry_requests(args),
        BatchCommand::Import(args) => local::batch_import(args),
        BatchCommand::Health(args) => report::batch_health(args),
        BatchCommand::Verify(args) => report::batch_verify(args),
        BatchCommand::Submit(args) => remote::batch_submit(args),
        BatchCommand::Status(args) => remote::batch_status(args),
        BatchCommand::Fetch(args) => remote::batch_fetch(args),
        BatchCommand::RerouteLocal(args) => reroute::batch_reroute_local(args),
        BatchCommand::TranslateLocal(args) => reroute::batch_translate_local(args),
    }
}

#[cfg(test)]
mod tests;
