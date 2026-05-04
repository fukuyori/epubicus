use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

pub(crate) const DEFAULT_MODEL: &str = "qwen3:14b";
pub(crate) const DEFAULT_OLLAMA_HOST: &str = "http://localhost:11434";
pub(crate) const DEFAULT_OPENAI_MODEL: &str = "gpt-5-mini";
pub(crate) const DEFAULT_CLAUDE_MODEL: &str = "claude-sonnet-4-5";
pub(crate) const DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
pub(crate) const DEFAULT_CLAUDE_BASE_URL: &str = "https://api.anthropic.com/v1";
pub(crate) const ANTHROPIC_VERSION: &str = "2023-06-01";
pub(crate) const DEFAULT_MAX_CHARS_PER_REQUEST: usize = 3500;
pub(crate) const DEFAULT_CONCURRENCY: usize = 1;
pub(crate) const DEFAULT_BATCH_MAX_REQUESTS_PER_FILE: usize = 50_000;
pub(crate) const DEFAULT_BATCH_MAX_BYTES_PER_FILE: usize = 200_000_000;

#[derive(Parser)]
#[command(name = "epubicus")]
#[command(version)]
#[command(about = "Translate English EPUB files to Japanese with Ollama, OpenAI, or Claude")]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Commands,
}

#[derive(Subcommand)]
pub(crate) enum Commands {
    /// Create a translated EPUB.
    Translate(TranslateArgs),
    /// Translate a spine page range and print the translation to stdout.
    Test(TestArgs),
    /// Inspect EPUB spine order.
    Inspect(InspectArgs),
    /// Show EPUB table of contents.
    Toc(TocArgs),
    /// Extract glossary candidates from an EPUB.
    Glossary(GlossaryArgs),
    /// Inspect or maintain translation caches.
    Cache(CacheArgs),
    /// Prepare and manage asynchronous batch translation artifacts.
    Batch(BatchArgs),
    /// Retry recoverable translation errors recorded in a recovery log.
    Recover(RecoverArgs),
    /// Scan a rebuilt EPUB and write recovery logs for suspicious untranslated blocks.
    ScanRecovery(ScanRecoveryArgs),
    /// Remove a stale input-use flag for an EPUB.
    Unlock(UnlockArgs),
}

#[derive(Parser, Clone)]
pub(crate) struct CommonArgs {
    /// Translation provider.
    #[arg(short = 'p', long, value_enum, env = "EPUBICUS_PROVIDER", default_value_t = Provider::Ollama)]
    pub(crate) provider: Provider,
    /// Model name. Defaults depend on --provider.
    #[arg(short, long, env = "EPUBICUS_MODEL")]
    pub(crate) model: Option<String>,
    /// Provider to use only when the primary provider returns a refusal/explanation response.
    #[arg(long, value_enum, env = "EPUBICUS_FALLBACK_PROVIDER")]
    pub(crate) fallback_provider: Option<Provider>,
    /// Model name for --fallback-provider. Defaults depend on --fallback-provider.
    #[arg(long, env = "EPUBICUS_FALLBACK_MODEL")]
    pub(crate) fallback_model: Option<String>,
    /// Ollama endpoint.
    #[arg(long, env = "EPUBICUS_OLLAMA_HOST", default_value = DEFAULT_OLLAMA_HOST)]
    pub(crate) ollama_host: String,
    /// OpenAI API base URL.
    #[arg(long, env = "EPUBICUS_OPENAI_BASE_URL", default_value = DEFAULT_OPENAI_BASE_URL)]
    pub(crate) openai_base_url: String,
    /// Claude/Anthropic API base URL.
    #[arg(long, env = "EPUBICUS_CLAUDE_BASE_URL", default_value = DEFAULT_CLAUDE_BASE_URL)]
    pub(crate) claude_base_url: String,
    /// OpenAI API key. Prefer OPENAI_API_KEY or --prompt-api-key for interactive use.
    #[arg(long)]
    pub(crate) openai_api_key: Option<String>,
    /// Anthropic API key. Prefer ANTHROPIC_API_KEY or --prompt-api-key for interactive use.
    #[arg(long)]
    pub(crate) anthropic_api_key: Option<String>,
    /// Prompt for the provider API key at runtime without echoing it.
    #[arg(long)]
    pub(crate) prompt_api_key: bool,
    /// Sampling temperature.
    #[arg(short = 'T', long, env = "EPUBICUS_TEMPERATURE", default_value_t = 0.3)]
    pub(crate) temperature: f32,
    /// Context window size passed to Ollama.
    #[arg(short = 'n', long, env = "EPUBICUS_NUM_CTX", default_value_t = 8192)]
    pub(crate) num_ctx: u32,
    /// HTTP timeout per translation request, in seconds.
    #[arg(
        short = 't',
        long,
        env = "EPUBICUS_TIMEOUT_SECS",
        default_value_t = 900
    )]
    pub(crate) timeout_secs: u64,
    /// Number of retries after the initial request for timeout, connection, rate limit, server errors, or validation failures.
    #[arg(short = 'r', long, env = "EPUBICUS_RETRIES", default_value_t = 3)]
    pub(crate) retries: u32,
    /// Maximum source characters per provider request. Long blocks are split at sentence boundaries.
    #[arg(short = 'x', long, env = "EPUBICUS_MAX_CHARS_PER_REQUEST", default_value_t = DEFAULT_MAX_CHARS_PER_REQUEST)]
    pub(crate) max_chars_per_request: usize,
    /// Maximum number of uncached provider requests to run in parallel. Automatically reduced after retryable errors and slowly restored after successful requests.
    #[arg(short = 'j', long, env = "EPUBICUS_CONCURRENCY", default_value_t = DEFAULT_CONCURRENCY)]
    pub(crate) concurrency: usize,
    /// Style preset: novel, novel-polite, tech, essay, academic, business.
    #[arg(short = 's', long, env = "EPUBICUS_STYLE", default_value = "essay")]
    pub(crate) style: String,
    /// Do not call the translation provider; emit source text instead.
    #[arg(short = 'd', long)]
    pub(crate) dry_run: bool,
    /// Glossary JSON file used to force consistent terms.
    #[arg(short = 'g', long)]
    pub(crate) glossary: Option<PathBuf>,
    /// Override the cache root directory. Per-EPUB caches are stored under <cache_root>/<input_hash>/.
    /// Defaults to OS-standard cache (Windows: %LOCALAPPDATA%\epubicus\cache, Unix: ~/.cache/epubicus).
    #[arg(long)]
    pub(crate) cache_root: Option<PathBuf>,
    /// Disable translation cache.
    #[arg(long)]
    pub(crate) no_cache: bool,
    /// Clear this input EPUB's cache before translating.
    #[arg(long)]
    pub(crate) clear_cache: bool,
    /// Keep the cache after a successful completion (default: cache is auto-deleted on completion).
    #[arg(short = 'k', long)]
    pub(crate) keep_cache: bool,
    /// Estimate API requests and tokens, then exit without translating.
    #[arg(short = 'u', long)]
    pub(crate) usage_only: bool,
    /// Create a partial EPUB from cached translations and keep cache misses unchanged.
    #[arg(long = "partial-from-cache")]
    pub(crate) partial_from_cache: bool,
    /// After validation retries are exhausted, keep the original block instead of aborting.
    #[arg(long, env = "EPUBICUS_PASSTHROUGH_ON_VALIDATION_FAILURE")]
    pub(crate) passthrough_on_validation_failure: bool,
    /// Show detailed warnings while processing.
    #[arg(long, env = "EPUBICUS_VERBOSE")]
    pub(crate) verbose: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum Provider {
    Ollama,
    Openai,
    Claude,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum BatchPriority {
    PageOrder,
    FailedFirst,
    HardFirst,
    ShortFirst,
    OldestFirst,
}

impl std::fmt::Display for BatchPriority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BatchPriority::PageOrder => write!(f, "page-order"),
            BatchPriority::FailedFirst => write!(f, "failed-first"),
            BatchPriority::HardFirst => write!(f, "hard-first"),
            BatchPriority::ShortFirst => write!(f, "short-first"),
            BatchPriority::OldestFirst => write!(f, "oldest-first"),
        }
    }
}

impl std::fmt::Display for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Provider::Ollama => write!(f, "ollama"),
            Provider::Openai => write!(f, "openai"),
            Provider::Claude => write!(f, "claude"),
        }
    }
}

#[derive(Parser)]
pub(crate) struct TranslateArgs {
    /// Input EPUB.
    pub(crate) input: PathBuf,
    /// Output EPUB.
    #[arg(short, long)]
    pub(crate) output: Option<PathBuf>,
    /// First spine page to translate, 1-based.
    #[arg(long)]
    pub(crate) from: Option<usize>,
    /// Last spine page to translate, 1-based and inclusive.
    #[arg(long)]
    pub(crate) to: Option<usize>,
    #[command(flatten)]
    pub(crate) common: CommonArgs,
}

#[derive(Parser)]
pub(crate) struct TestArgs {
    /// Input EPUB.
    pub(crate) input: PathBuf,
    /// First spine page to translate, 1-based.
    #[arg(long)]
    pub(crate) from: usize,
    /// Last spine page to translate, 1-based and inclusive.
    #[arg(long)]
    pub(crate) to: usize,
    #[command(flatten)]
    pub(crate) common: CommonArgs,
}

#[derive(Parser)]
pub(crate) struct InspectArgs {
    /// Input EPUB.
    pub(crate) input: PathBuf,
}

#[derive(Parser)]
pub(crate) struct TocArgs {
    /// Input EPUB.
    pub(crate) input: PathBuf,
}

#[derive(Parser)]
pub(crate) struct UnlockArgs {
    /// Input EPUB whose input-use flag should be removed.
    pub(crate) input: PathBuf,
    /// Remove the flag even if the recorded process still appears to be running.
    #[arg(long)]
    pub(crate) force: bool,
}

#[derive(Parser)]
pub(crate) struct RecoverArgs {
    /// Recovery JSONL written next to a partial output EPUB.
    pub(crate) log: Option<PathBuf>,
    /// Resolve the newest recovery.jsonl from a cached run by input EPUB path or cache hash prefix.
    #[arg(long = "cache", conflicts_with = "log")]
    pub(crate) cache_target: Option<String>,
    /// Input EPUB. Defaults to the first input_epub recorded in the log.
    #[arg(long)]
    pub(crate) input: Option<PathBuf>,
    /// Maximum number of log items to retry.
    #[arg(long)]
    pub(crate) limit: Option<usize>,
    /// List matching recovery log items without translating.
    #[arg(long)]
    pub(crate) list: bool,
    /// Only include records for this spine page number.
    #[arg(long)]
    pub(crate) page: Option<usize>,
    /// Only include records for this block index.
    #[arg(long)]
    pub(crate) block: Option<usize>,
    /// Only include records with this reason. Can be repeated.
    #[arg(long = "reason")]
    pub(crate) reasons: Vec<String>,
    /// Failed-item JSONL path. Defaults to <log stem>.failed.jsonl.
    #[arg(long)]
    pub(crate) failed_log: Option<PathBuf>,
    /// Rebuild the EPUB from cache after every selected item is recovered.
    #[arg(long)]
    pub(crate) rebuild: bool,
    /// Output EPUB path for --rebuild. Defaults to the output_epub recorded in the log.
    #[arg(short, long)]
    pub(crate) output: Option<PathBuf>,
    #[command(flatten)]
    pub(crate) common: CommonArgs,
}

#[derive(Parser)]
pub(crate) struct ScanRecoveryArgs {
    /// Original input EPUB.
    pub(crate) input: PathBuf,
    /// Translated or partially translated EPUB to inspect.
    pub(crate) output: PathBuf,
    /// Maximum number of suspicious blocks to record.
    #[arg(long)]
    pub(crate) limit: Option<usize>,
    /// After writing the recovery log, retry the suspicious blocks immediately.
    #[arg(long)]
    pub(crate) recover: bool,
    /// Rebuild the inspected EPUB after --recover succeeds.
    #[arg(long, requires = "recover")]
    pub(crate) rebuild: bool,
    /// Failed-item JSONL path for --recover. Defaults to <recovery log directory>\failed.jsonl.
    #[arg(long, requires = "recover")]
    pub(crate) failed_log: Option<PathBuf>,
    #[command(flatten)]
    pub(crate) common: CommonArgs,
}

#[derive(Parser)]
pub(crate) struct GlossaryArgs {
    /// Input EPUB.
    pub(crate) input: PathBuf,
    /// Output glossary JSON.
    #[arg(short, long, default_value = "glossary.json")]
    pub(crate) output: PathBuf,
    /// Minimum occurrences required for a candidate.
    #[arg(long, default_value_t = 3)]
    pub(crate) min_occurrences: usize,
    /// Maximum number of entries to output.
    #[arg(long, default_value_t = 200)]
    pub(crate) max_entries: usize,
    /// Write a Markdown prompt for reviewing the glossary with ChatGPT or Claude.
    #[arg(long)]
    pub(crate) review_prompt: Option<PathBuf>,
}

#[derive(Parser)]
pub(crate) struct CacheArgs {
    /// Override the cache root directory. Defaults to OS-standard cache location.
    #[arg(long, global = true)]
    pub(crate) cache_root: Option<PathBuf>,
    #[command(subcommand)]
    pub(crate) command: CacheCommand,
}

#[derive(Subcommand)]
pub(crate) enum CacheCommand {
    /// List all cached runs.
    List,
    /// Show details for a specific run (by hash prefix or input EPUB path).
    Show {
        /// Cache hash (full or prefix) or path to an input EPUB.
        target: String,
    },
    /// Delete cache directories that have not been updated for the given number of days.
    Prune {
        /// Delete entries with last_updated_at older than N days.
        #[arg(long)]
        older_than: u64,
        /// Skip the confirmation prompt.
        #[arg(long, short = 'y')]
        yes: bool,
        /// Show what would be deleted without deleting.
        #[arg(long)]
        dry_run: bool,
    },
    /// Delete a single cached run, or all of them.
    Clear {
        /// Cache hash (full or prefix). Mutually exclusive with --all.
        #[arg(long, conflicts_with = "all")]
        hash: Option<String>,
        /// Delete every cached run. Requires confirmation unless --yes is set.
        #[arg(long, conflicts_with = "hash")]
        all: bool,
        /// Skip the confirmation prompt for --all.
        #[arg(long, short = 'y')]
        yes: bool,
        /// Show what would be deleted without deleting.
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Parser)]
pub(crate) struct BatchArgs {
    #[command(subcommand)]
    pub(crate) command: BatchCommand,
}

#[derive(Subcommand)]
pub(crate) enum BatchCommand {
    /// Prepare local Batch API request files without submitting them.
    Prepare(BatchPrepareArgs),
    /// Prepare, submit, wait optionally, fetch, import, and verify a batch.
    Run(BatchRunArgs),
    /// Write retry_requests.jsonl for failed or rejected batch items.
    RetryRequests(BatchRetryArgs),
    /// Import a local Batch API output JSONL file into the translation cache.
    Import(BatchImportArgs),
    /// Show local batch workspace, item, and cache state.
    Health(BatchHealthArgs),
    /// Verify local batch artifacts against the current EPUB and cache.
    Verify(BatchVerifyArgs),
    /// Upload request part files and create remote OpenAI batches.
    Submit(BatchSubmitArgs),
    /// Refresh remote OpenAI batch status into the local manifest.
    Status(BatchStatusArgs),
    /// Download remote batch output/error files.
    Fetch(BatchFetchArgs),
    /// Mark selected unfinished batch items for local translation.
    RerouteLocal(BatchRerouteLocalArgs),
    /// Translate local_pending batch items through the normal provider backend.
    TranslateLocal(BatchTranslateLocalArgs),
}

#[derive(Parser)]
pub(crate) struct BatchPrepareArgs {
    /// Input EPUB.
    pub(crate) input: PathBuf,
    /// First spine page to include, 1-based.
    #[arg(long)]
    pub(crate) from: Option<usize>,
    /// Last spine page to include, 1-based and inclusive.
    #[arg(long)]
    pub(crate) to: Option<usize>,
    /// Maximum request lines per Batch API input file.
    #[arg(long, default_value_t = DEFAULT_BATCH_MAX_REQUESTS_PER_FILE)]
    pub(crate) max_requests_per_file: usize,
    /// Maximum JSONL bytes per Batch API input file.
    #[arg(long, default_value_t = DEFAULT_BATCH_MAX_BYTES_PER_FILE)]
    pub(crate) max_bytes_per_file: usize,
    #[command(flatten)]
    pub(crate) common: CommonArgs,
}

#[derive(Parser)]
pub(crate) struct BatchRunArgs {
    /// Input EPUB.
    pub(crate) input: PathBuf,
    /// First spine page to include, 1-based.
    #[arg(long)]
    pub(crate) from: Option<usize>,
    /// Last spine page to include, 1-based and inclusive.
    #[arg(long)]
    pub(crate) to: Option<usize>,
    /// Maximum request lines per Batch API input file.
    #[arg(long, default_value_t = DEFAULT_BATCH_MAX_REQUESTS_PER_FILE)]
    pub(crate) max_requests_per_file: usize,
    /// Maximum JSONL bytes per Batch API input file.
    #[arg(long, default_value_t = DEFAULT_BATCH_MAX_BYTES_PER_FILE)]
    pub(crate) max_bytes_per_file: usize,
    /// Recreate local request artifacts before submitting.
    #[arg(long)]
    pub(crate) force_prepare: bool,
    /// Poll until the remote batch reaches a fetchable terminal state.
    #[arg(long)]
    pub(crate) wait: bool,
    /// Seconds between status polls when --wait is set.
    #[arg(long, default_value_t = 180)]
    pub(crate) poll_secs: u64,
    /// Maximum seconds to wait before exiting. Defaults to no limit.
    #[arg(long)]
    pub(crate) max_wait_secs: Option<u64>,
    /// Overwrite existing fetched output/error files.
    #[arg(long)]
    pub(crate) force_fetch: bool,
    /// Skip batch verify after import.
    #[arg(long)]
    pub(crate) skip_verify: bool,
    /// Assemble an EPUB from the imported cache after the batch finishes.
    #[arg(short, long)]
    pub(crate) output: Option<PathBuf>,
    #[command(flatten)]
    pub(crate) common: CommonArgs,
}

#[derive(Parser)]
pub(crate) struct BatchRetryArgs {
    /// Input EPUB used for batch prepare.
    pub(crate) input: PathBuf,
    /// Select items currently in this state. Defaults to failed and rejected.
    #[arg(long = "state")]
    pub(crate) states: Vec<String>,
    /// Maximum number of retry requests to write.
    #[arg(long)]
    pub(crate) limit: Option<usize>,
    /// Selection priority for retry request planning.
    #[arg(long, value_enum, default_value_t = BatchPriority::PageOrder)]
    pub(crate) priority: BatchPriority,
    #[command(flatten)]
    pub(crate) common: CommonArgs,
}

#[derive(Parser)]
pub(crate) struct BatchImportArgs {
    /// Input EPUB used for batch prepare.
    pub(crate) input: PathBuf,
    /// Local Batch API output JSONL file to import. Defaults to the fetched batch/output.jsonl.
    #[arg(short, long)]
    pub(crate) output: Option<PathBuf>,
    #[command(flatten)]
    pub(crate) common: CommonArgs,
}

#[derive(Parser)]
pub(crate) struct BatchHealthArgs {
    /// Input EPUB used for batch prepare.
    pub(crate) input: PathBuf,
    #[command(flatten)]
    pub(crate) common: CommonArgs,
}

#[derive(Parser)]
pub(crate) struct BatchVerifyArgs {
    /// Input EPUB used for batch prepare.
    pub(crate) input: PathBuf,
    #[command(flatten)]
    pub(crate) common: CommonArgs,
}

#[derive(Parser)]
pub(crate) struct BatchSubmitArgs {
    /// Input EPUB used for batch prepare.
    pub(crate) input: PathBuf,
    /// Allow replacing an existing recorded batch id.
    #[arg(long)]
    pub(crate) force: bool,
    #[command(flatten)]
    pub(crate) common: CommonArgs,
}

#[derive(Parser)]
pub(crate) struct BatchStatusArgs {
    /// Input EPUB used for batch prepare.
    pub(crate) input: PathBuf,
    #[command(flatten)]
    pub(crate) common: CommonArgs,
}

#[derive(Parser)]
pub(crate) struct BatchFetchArgs {
    /// Input EPUB used for batch prepare.
    pub(crate) input: PathBuf,
    /// Overwrite existing downloaded output/error files.
    #[arg(long)]
    pub(crate) force: bool,
    #[command(flatten)]
    pub(crate) common: CommonArgs,
}

#[derive(Parser)]
pub(crate) struct BatchRerouteLocalArgs {
    /// Input EPUB used for batch prepare.
    pub(crate) input: PathBuf,
    /// Select items currently in this state. Can be repeated.
    #[arg(long = "state")]
    pub(crate) states: Vec<String>,
    /// Select every item that is not already imported or cached.
    #[arg(long)]
    pub(crate) remaining: bool,
    /// Only reroute remaining items when their count is at or below this value.
    #[arg(long)]
    pub(crate) endgame_threshold: Option<usize>,
    /// Maximum number of items to mark local_pending.
    #[arg(long)]
    pub(crate) limit: Option<usize>,
    /// Selection priority for local fallback planning.
    #[arg(long, value_enum, default_value_t = BatchPriority::PageOrder)]
    pub(crate) priority: BatchPriority,
    #[command(flatten)]
    pub(crate) common: CommonArgs,
}

#[derive(Parser)]
pub(crate) struct BatchTranslateLocalArgs {
    /// Input EPUB used for batch prepare.
    pub(crate) input: PathBuf,
    /// Maximum number of local_pending items to process.
    #[arg(long)]
    pub(crate) limit: Option<usize>,
    /// Processing priority for local_pending items.
    #[arg(long, value_enum, default_value_t = BatchPriority::PageOrder)]
    pub(crate) priority: BatchPriority,
    #[command(flatten)]
    pub(crate) common: CommonArgs,
}
