use std::{
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, BufWriter, Cursor, Write},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use indicatif::{ProgressBar, ProgressStyle};
use quick_xml::{
    Reader, Writer,
    events::{BytesEnd, BytesStart, BytesText, Event},
    name::QName,
};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use walkdir::WalkDir;
use zip::{
    CompressionMethod, ZipArchive, ZipWriter,
    write::{FileOptions, SimpleFileOptions},
};

const DEFAULT_MODEL: &str = "qwen3:14b";
const DEFAULT_OLLAMA_HOST: &str = "http://localhost:11434";
const DEFAULT_OPENAI_MODEL: &str = "gpt-5-mini";
const DEFAULT_CLAUDE_MODEL: &str = "claude-sonnet-4-5";
const DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
const DEFAULT_CLAUDE_BASE_URL: &str = "https://api.anthropic.com/v1";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const ETA_MIN_MODEL_BLOCKS: usize = 5;
const ETA_MIN_ELAPSED_SECS: u64 = 30;

#[derive(Parser)]
#[command(name = "epubicus")]
#[command(about = "Translate English EPUB files to Japanese with Ollama, OpenAI, or Claude")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
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
}

#[derive(Parser, Clone)]
struct CommonArgs {
    /// Translation provider.
    #[arg(long, value_enum, default_value_t = Provider::Ollama)]
    provider: Provider,
    /// Model name. Defaults depend on --provider.
    #[arg(short, long)]
    model: Option<String>,
    /// Ollama endpoint.
    #[arg(long, default_value = DEFAULT_OLLAMA_HOST)]
    ollama_host: String,
    /// OpenAI API base URL.
    #[arg(long, default_value = DEFAULT_OPENAI_BASE_URL)]
    openai_base_url: String,
    /// Claude/Anthropic API base URL.
    #[arg(long, default_value = DEFAULT_CLAUDE_BASE_URL)]
    claude_base_url: String,
    /// OpenAI API key. Prefer OPENAI_API_KEY or --prompt-api-key for interactive use.
    #[arg(long)]
    openai_api_key: Option<String>,
    /// Anthropic API key. Prefer ANTHROPIC_API_KEY or --prompt-api-key for interactive use.
    #[arg(long)]
    anthropic_api_key: Option<String>,
    /// Prompt for the provider API key at runtime without echoing it.
    #[arg(long)]
    prompt_api_key: bool,
    /// Sampling temperature.
    #[arg(long, default_value_t = 0.3)]
    temperature: f32,
    /// Context window size passed to Ollama.
    #[arg(long, default_value_t = 8192)]
    num_ctx: u32,
    /// HTTP timeout per translation request, in seconds.
    #[arg(long, default_value_t = 900)]
    timeout_secs: u64,
    /// Number of retries for timeout, connection, rate limit, or server errors.
    #[arg(long, default_value_t = 2)]
    retries: u32,
    /// Style preset: novel, novel-polite, tech, essay, academic, business.
    #[arg(long, default_value = "essay")]
    style: String,
    /// Do not call the translation provider; emit source text instead.
    #[arg(long)]
    dry_run: bool,
    /// Glossary JSON file used to force consistent terms.
    #[arg(long)]
    glossary: Option<PathBuf>,
    /// Override the cache root directory. Per-EPUB caches are stored under <cache_root>/<input_hash>/.
    /// Defaults to OS-standard cache (Windows: %LOCALAPPDATA%\epubicus\cache, Unix: ~/.cache/epubicus).
    #[arg(long)]
    cache_root: Option<PathBuf>,
    /// Disable translation cache.
    #[arg(long)]
    no_cache: bool,
    /// Clear this input EPUB's cache before translating.
    #[arg(long)]
    clear_cache: bool,
    /// Keep the cache after a successful completion (default: cache is auto-deleted on completion).
    #[arg(long)]
    keep_cache: bool,
    /// Create a partial EPUB from cached translations and keep cache misses unchanged.
    #[arg(long = "partial-from-cache")]
    partial_from_cache: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Provider {
    Ollama,
    Openai,
    Claude,
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
struct TranslateArgs {
    /// Input EPUB.
    input: PathBuf,
    /// Output EPUB.
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// First spine page to translate, 1-based.
    #[arg(long)]
    from: Option<usize>,
    /// Last spine page to translate, 1-based and inclusive.
    #[arg(long)]
    to: Option<usize>,
    #[command(flatten)]
    common: CommonArgs,
}

#[derive(Parser)]
struct TestArgs {
    /// Input EPUB.
    input: PathBuf,
    /// First spine page to translate, 1-based.
    #[arg(long)]
    from: usize,
    /// Last spine page to translate, 1-based and inclusive.
    #[arg(long)]
    to: usize,
    #[command(flatten)]
    common: CommonArgs,
}

#[derive(Parser)]
struct InspectArgs {
    /// Input EPUB.
    input: PathBuf,
}

#[derive(Parser)]
struct TocArgs {
    /// Input EPUB.
    input: PathBuf,
}

#[derive(Parser)]
struct GlossaryArgs {
    /// Input EPUB.
    input: PathBuf,
    /// Output glossary JSON.
    #[arg(short, long, default_value = "glossary.json")]
    output: PathBuf,
    /// Minimum occurrences required for a candidate.
    #[arg(long, default_value_t = 3)]
    min_occurrences: usize,
    /// Maximum number of entries to output.
    #[arg(long, default_value_t = 200)]
    max_entries: usize,
    /// Write a Markdown prompt for reviewing the glossary with ChatGPT or Claude.
    #[arg(long)]
    review_prompt: Option<PathBuf>,
}

#[derive(Parser)]
struct CacheArgs {
    /// Override the cache root directory. Defaults to OS-standard cache location.
    #[arg(long, global = true)]
    cache_root: Option<PathBuf>,
    #[command(subcommand)]
    command: CacheCommand,
}

#[derive(Subcommand)]
enum CacheCommand {
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

#[derive(Debug)]
struct EpubBook {
    work_dir: TempDir,
    opf_path: PathBuf,
    manifest: Vec<ManifestItem>,
    spine: Vec<SpineItem>,
}

#[derive(Debug, Clone)]
struct ManifestItem {
    id: String,
    href: String,
    abs_path: PathBuf,
    media_type: String,
    properties: Vec<String>,
}

#[derive(Debug, Clone)]
struct SpineItem {
    idref: String,
    href: String,
    abs_path: PathBuf,
    media_type: String,
    linear: bool,
}

#[derive(Debug)]
struct SpineRef {
    idref: String,
    linear: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Write,
    Stdout,
}

#[derive(Debug, Default)]
struct Stats {
    pages_seen: usize,
    pages_translated: usize,
    blocks_translated: usize,
}

#[derive(Debug, Clone, Default)]
struct InlineEntry {
    start: Option<Vec<u8>>,
    end: Option<Vec<u8>>,
    empty: Option<Vec<u8>>,
}

#[derive(Debug, Default)]
struct InlineMap {
    entries: HashMap<u32, InlineEntry>,
}

#[derive(Debug, Deserialize)]
struct OllamaResponse {
    message: OllamaMessage,
}

#[derive(Debug, Deserialize)]
struct OllamaMessage {
    content: String,
}

#[derive(Debug, Deserialize)]
struct ClaudeResponse {
    content: Vec<ClaudeContent>,
}

#[derive(Debug, Deserialize)]
struct ClaudeContent {
    #[serde(rename = "type")]
    kind: String,
    text: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct GlossaryFile {
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_lang: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_lang: Option<String>,
    entries: Vec<GlossaryEntry>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct GlossaryEntry {
    src: String,
    dst: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
}

#[derive(Debug)]
struct GlossaryCandidate {
    term: String,
    count: usize,
    kind: String,
}

const CACHE_SCHEMA_VERSION: u32 = 1;
const MANIFEST_FILE: &str = "manifest.json";
const TRANSLATIONS_FILE: &str = "translations.jsonl";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheRecord {
    key: String,
    translated: String,
    provider: String,
    model: String,
    at: String,
}

#[derive(Debug, Default)]
struct CacheStats {
    hits: usize,
    misses: usize,
    writes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Manifest {
    schema_version: u32,
    input: ManifestInput,
    params: ManifestParams,
    timestamps: ManifestTimestamps,
    #[serde(default)]
    last_output_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ManifestInput {
    sha256: String,
    path_when_started: String,
    size_bytes: u64,
    mtime: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ManifestParams {
    provider: String,
    model: String,
    prompt_version: String,
    style_id: String,
    glossary_sha: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ManifestTimestamps {
    started_at: String,
    last_updated_at: String,
}

struct CacheStore {
    enabled: bool,
    /// Per-EPUB cache directory: <cache_root>/<input_hash>/. Always populated, even when enabled=false,
    /// so callers can refer to it for diagnostics.
    dir: PathBuf,
    translations_path: PathBuf,
    manifest_path: PathBuf,
    /// Short hex (first 16 bytes) of the input EPUB hash, used as the directory name.
    #[allow(dead_code)]
    input_hash: String,
    /// Full SHA-256 hex of the input EPUB.
    input_sha256: String,
    entries: HashMap<String, String>,
    stats: CacheStats,
    keep_cache: bool,
}

impl CacheStore {
    fn from_args(input: &Path, args: &CommonArgs) -> Result<Self> {
        if args.partial_from_cache && args.no_cache {
            bail!("--partial-from-cache cannot be used with --no-cache");
        }
        let (input_sha256, input_hash) = compute_input_hash(input)?;
        let root = resolve_cache_root(args.cache_root.as_deref())?;
        let dir = root.join(&input_hash);
        let translations_path = dir.join(TRANSLATIONS_FILE);
        let manifest_path = dir.join(MANIFEST_FILE);
        if args.clear_cache && dir.exists() {
            fs::remove_dir_all(&dir)
                .with_context(|| format!("failed to clear cache {}", dir.display()))?;
        }
        if args.no_cache {
            return Ok(Self {
                enabled: false,
                dir,
                translations_path,
                manifest_path,
                input_hash,
                input_sha256,
                entries: HashMap::new(),
                stats: CacheStats::default(),
                keep_cache: args.keep_cache,
            });
        }
        fs::create_dir_all(&dir)
            .with_context(|| format!("failed to create cache dir {}", dir.display()))?;
        let entries = read_cache_entries(&translations_path)?;
        Ok(Self {
            enabled: true,
            dir,
            translations_path,
            manifest_path,
            input_hash,
            input_sha256,
            entries,
            stats: CacheStats::default(),
            keep_cache: args.keep_cache,
        })
    }

    fn get(&mut self, key: &str) -> Option<String> {
        if !self.enabled {
            self.stats.misses += 1;
            return None;
        }
        match self.entries.get(key) {
            Some(value) => {
                self.stats.hits += 1;
                Some(value.clone())
            }
            None => {
                self.stats.misses += 1;
                None
            }
        }
    }

    fn contains_key(&self, key: &str) -> bool {
        self.enabled && self.entries.contains_key(key)
    }

    fn insert(&mut self, record: CacheRecord) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }
        if self.entries.contains_key(&record.key) {
            return Ok(());
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.translations_path)
            .with_context(|| {
                format!("failed to open cache {}", self.translations_path.display())
            })?;
        serde_json::to_writer(&mut file, &record)?;
        writeln!(file)?;
        file.flush()?;
        self.entries
            .insert(record.key.clone(), record.translated.clone());
        self.stats.writes += 1;
        Ok(())
    }

    /// Read or create the manifest, persisting it after fields are filled in.
    fn upsert_manifest(
        &self,
        input: &Path,
        params: ManifestParams,
        last_output_path: Option<&Path>,
    ) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }
        let now = chrono::Utc::now().to_rfc3339();
        let existing: Option<Manifest> = if self.manifest_path.exists() {
            let data = fs::read_to_string(&self.manifest_path).with_context(|| {
                format!("failed to read manifest {}", self.manifest_path.display())
            })?;
            serde_json::from_str(&data).ok()
        } else {
            None
        };
        let metadata = fs::metadata(input).ok();
        let size_bytes = metadata.as_ref().map(|m| m.len()).unwrap_or(0);
        let mtime = metadata
            .as_ref()
            .and_then(|m| m.modified().ok())
            .and_then(|t| {
                chrono::DateTime::<chrono::Utc>::from(t)
                    .to_rfc3339()
                    .into()
            });
        let manifest = match existing {
            Some(mut m) => {
                m.params = params;
                m.timestamps.last_updated_at = now;
                if let Some(p) = last_output_path {
                    m.last_output_path = Some(p.display().to_string());
                }
                m.input.path_when_started = input.display().to_string();
                m.input.size_bytes = size_bytes;
                m.input.mtime = mtime;
                m
            }
            None => Manifest {
                schema_version: CACHE_SCHEMA_VERSION,
                input: ManifestInput {
                    sha256: self.input_sha256.clone(),
                    path_when_started: input.display().to_string(),
                    size_bytes,
                    mtime,
                },
                params,
                timestamps: ManifestTimestamps {
                    started_at: now.clone(),
                    last_updated_at: now,
                },
                last_output_path: last_output_path.map(|p| p.display().to_string()),
            },
        };
        write_manifest(&self.manifest_path, &manifest)
    }

    /// Delete the cache directory unless --keep-cache was set. Idempotent: does nothing if disabled.
    fn finalize_completion(&self) -> Result<()> {
        if !self.enabled || self.keep_cache {
            return Ok(());
        }
        if self.dir.exists() {
            fs::remove_dir_all(&self.dir).with_context(|| {
                format!(
                    "failed to remove cache directory {}",
                    self.dir.display()
                )
            })?;
        }
        Ok(())
    }
}

/// Compute (full SHA-256 hex, first-16-byte hex) of the input file.
fn compute_input_hash(input: &Path) -> Result<(String, String)> {
    let file = File::open(input)
        .with_context(|| format!("failed to open input {}", input.display()))?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = std::io::Read::read(&mut reader, &mut buf)
            .with_context(|| format!("failed to read {}", input.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let digest = hasher.finalize();
    let full: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    let short: String = digest[..16].iter().map(|b| format!("{b:02x}")).collect();
    Ok((full, short))
}

/// Resolve the cache root directory, preferring the user override.
/// Default per platform:
///   Windows: %LOCALAPPDATA%\epubicus\cache (fallback %APPDATA%\epubicus\cache)
///   Unix:    $XDG_CACHE_HOME/epubicus or ~/.cache/epubicus
fn resolve_cache_root(override_root: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = override_root {
        return Ok(p.to_path_buf());
    }
    if cfg!(windows) {
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            return Ok(PathBuf::from(local).join("epubicus").join("cache"));
        }
        if let Some(appdata) = std::env::var_os("APPDATA") {
            return Ok(PathBuf::from(appdata).join("epubicus").join("cache"));
        }
    } else if let Some(xdg) = std::env::var_os("XDG_CACHE_HOME") {
        return Ok(PathBuf::from(xdg).join("epubicus"));
    } else if let Some(home) = std::env::var_os("HOME") {
        return Ok(PathBuf::from(home).join(".cache").join("epubicus"));
    }
    bail!("cannot determine default cache root; set --cache-root explicitly");
}

fn write_manifest(path: &Path, manifest: &Manifest) -> Result<()> {
    let tmp = path.with_extension("json.tmp");
    let data = serde_json::to_vec_pretty(manifest)
        .context("failed to serialize cache manifest")?;
    fs::write(&tmp, &data)
        .with_context(|| format!("failed to write manifest {}", tmp.display()))?;
    fs::rename(&tmp, path)
        .with_context(|| format!("failed to commit manifest {}", path.display()))?;
    Ok(())
}

fn glossary_sha(entries: &[GlossaryEntry]) -> String {
    if entries.is_empty() {
        return String::new();
    }
    let mut hasher = Sha256::new();
    for e in entries {
        hasher.update(e.src.as_bytes());
        hasher.update(b"=>");
        hasher.update(e.dst.as_bytes());
        hasher.update(b"\n");
    }
    let digest = hasher.finalize();
    digest[..16].iter().map(|b| format!("{b:02x}")).collect()
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Translate(args) => translate_command(args),
        Commands::Test(args) => test_command(args),
        Commands::Inspect(args) => inspect_command(args),
        Commands::Toc(args) => toc_command(args),
        Commands::Glossary(args) => glossary_command(args),
        Commands::Cache(args) => cache_command(args),
    }
}

fn translate_command(args: TranslateArgs) -> Result<()> {
    let output = args
        .output
        .unwrap_or_else(|| default_output_path(&args.input));
    let book = unpack_epub(&args.input)?;
    let range = normalize_range(args.from, args.to, book.spine.len())?;
    let partial_from_cache = args.common.partial_from_cache;
    let cache = CacheStore::from_args(&args.input, &args.common)?;
    let mut translator = Translator::new(args.common, cache)?;
    if !partial_from_cache {
        let params = translator.manifest_params();
        translator
            .cache
            .upsert_manifest(&args.input, params, Some(&output))?;
    }
    let mut stats = translate_book(&book, range, &mut translator, Mode::Write, true)?;
    update_opf_metadata(&book.opf_path, &translator.model)?;
    pack_epub(book.work_dir.path(), &output)?;
    stats.pages_seen = book.spine.len();
    let cache_dir_display = if translator.cache.enabled {
        translator.cache.dir.display().to_string()
    } else {
        "disabled".to_string()
    };
    let pack_succeeded = true;
    let full_range_translated =
        stats.pages_translated == book.spine.len() && !partial_from_cache;
    let cache_was_kept_or_partial =
        partial_from_cache || translator.cache.keep_cache || !full_range_translated;
    if pack_succeeded && !cache_was_kept_or_partial {
        translator.cache.finalize_completion()?;
    }
    let cache_status = if !translator.cache.enabled {
        "disabled".to_string()
    } else if pack_succeeded && !cache_was_kept_or_partial {
        format!("auto-deleted ({cache_dir_display})")
    } else {
        cache_dir_display.clone()
    };
    eprintln!(
        "Done. Output: {} | pages translated: {} | blocks translated: {} | provider: {} | model: {} | cache hits: {} | misses: {} | writes: {} | cache: {}",
        output.display(),
        stats.pages_translated,
        stats.blocks_translated,
        translator.provider,
        translator.model,
        translator.cache.stats.hits,
        translator.cache.stats.misses,
        translator.cache.stats.writes,
        cache_status,
    );
    Ok(())
}

fn test_command(args: TestArgs) -> Result<()> {
    let book = unpack_epub(&args.input)?;
    let range = normalize_range(Some(args.from), Some(args.to), book.spine.len())?;
    let cache = CacheStore::from_args(&args.input, &args.common)?;
    let mut translator = Translator::new(args.common, cache)?;
    let stats = translate_book(&book, range, &mut translator, Mode::Stdout, false)?;
    eprintln!(
        "Translated {} spine pages, {} blocks.",
        stats.pages_translated, stats.blocks_translated
    );
    Ok(())
}

fn cache_command(args: CacheArgs) -> Result<()> {
    let root = resolve_cache_root(args.cache_root.as_deref())?;
    match args.command {
        CacheCommand::List => cache_list(&root),
        CacheCommand::Show { target } => cache_show(&root, &target),
        CacheCommand::Prune {
            older_than,
            yes,
            dry_run,
        } => cache_prune(&root, older_than, yes, dry_run),
        CacheCommand::Clear {
            hash,
            all,
            yes,
            dry_run,
        } => cache_clear(&root, hash.as_deref(), all, yes, dry_run),
    }
}

#[derive(Debug)]
struct CacheEntryInfo {
    hash: String,
    dir: PathBuf,
    manifest: Option<Manifest>,
    cached_segments: usize,
    size_bytes: u64,
}

fn collect_cache_entries(root: &Path) -> Result<Vec<CacheEntryInfo>> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(root)
        .with_context(|| format!("failed to read cache root {}", root.display()))?
    {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let dir = entry.path();
        let hash = entry.file_name().to_string_lossy().to_string();
        let manifest_path = dir.join(MANIFEST_FILE);
        let manifest: Option<Manifest> = if manifest_path.exists() {
            let data = fs::read_to_string(&manifest_path).ok();
            data.and_then(|s| serde_json::from_str(&s).ok())
        } else {
            None
        };
        let translations_path = dir.join(TRANSLATIONS_FILE);
        let cached_segments = if translations_path.exists() {
            count_jsonl_lines(&translations_path).unwrap_or(0)
        } else {
            0
        };
        let size_bytes = directory_size(&dir).unwrap_or(0);
        out.push(CacheEntryInfo {
            hash,
            dir,
            manifest,
            cached_segments,
            size_bytes,
        });
    }
    out.sort_by(|a, b| {
        let aa = a
            .manifest
            .as_ref()
            .map(|m| m.timestamps.last_updated_at.clone())
            .unwrap_or_default();
        let bb = b
            .manifest
            .as_ref()
            .map(|m| m.timestamps.last_updated_at.clone())
            .unwrap_or_default();
        bb.cmp(&aa)
    });
    Ok(out)
}

fn count_jsonl_lines(path: &Path) -> Result<usize> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut count = 0usize;
    for line in reader.lines() {
        let line = line?;
        if !line.trim().is_empty() {
            count += 1;
        }
    }
    Ok(count)
}

fn directory_size(dir: &Path) -> Result<u64> {
    let mut total = 0u64;
    for entry in WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
        if entry.file_type().is_file() {
            if let Ok(meta) = entry.metadata() {
                total += meta.len();
            }
        }
    }
    Ok(total)
}

fn human_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

fn cache_list(root: &Path) -> Result<()> {
    let entries = collect_cache_entries(root)?;
    println!("Cache root: {}", root.display());
    if entries.is_empty() {
        println!("(no cached runs)");
        return Ok(());
    }
    println!();
    println!(
        "{:<32}  {:>8}  {:>10}  {:<25}  {}",
        "Hash", "Segs", "Size", "Last Updated", "Input"
    );
    println!("{}", "-".repeat(110));
    for e in &entries {
        let last = e
            .manifest
            .as_ref()
            .map(|m| m.timestamps.last_updated_at.as_str())
            .unwrap_or("-");
        let input = e
            .manifest
            .as_ref()
            .map(|m| m.input.path_when_started.as_str())
            .unwrap_or("(no manifest)");
        println!(
            "{:<32}  {:>8}  {:>10}  {:<25}  {}",
            e.hash,
            e.cached_segments,
            human_bytes(e.size_bytes),
            last,
            input,
        );
    }
    println!();
    let total: u64 = entries.iter().map(|e| e.size_bytes).sum();
    println!(
        "Total: {} run(s), {}",
        entries.len(),
        human_bytes(total)
    );
    Ok(())
}

fn cache_show(root: &Path, target: &str) -> Result<()> {
    let entry = resolve_cache_target(root, target)?;
    println!("Hash:       {}", entry.hash);
    println!("Directory:  {}", entry.dir.display());
    println!("Size:       {}", human_bytes(entry.size_bytes));
    println!("Segments:   {}", entry.cached_segments);
    if let Some(manifest) = &entry.manifest {
        println!();
        let pretty = serde_json::to_string_pretty(manifest)?;
        println!("{pretty}");
    } else {
        println!("(manifest.json missing)");
    }
    Ok(())
}

fn resolve_cache_target(root: &Path, target: &str) -> Result<CacheEntryInfo> {
    let target_path = Path::new(target);
    let entries = collect_cache_entries(root)?;
    if target_path.exists() && target_path.is_file() {
        let (_, short) = compute_input_hash(target_path)?;
        return entries
            .into_iter()
            .find(|e| e.hash == short)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "no cached run found for input {} (hash {})",
                    target_path.display(),
                    short
                )
            });
    }
    let matches: Vec<_> = entries
        .into_iter()
        .filter(|e| e.hash.starts_with(target))
        .collect();
    match matches.len() {
        0 => bail!("no cached run matches '{target}'"),
        1 => Ok(matches.into_iter().next().unwrap()),
        n => bail!("'{target}' is ambiguous: matches {n} cached runs"),
    }
}

fn cache_prune(root: &Path, older_than_days: u64, yes: bool, dry_run: bool) -> Result<()> {
    let entries = collect_cache_entries(root)?;
    if entries.is_empty() {
        println!("(no cached runs)");
        return Ok(());
    }
    let cutoff = chrono::Utc::now() - chrono::Duration::days(older_than_days as i64);
    let mut victims = Vec::new();
    for e in entries {
        let Some(manifest) = e.manifest.as_ref() else {
            continue;
        };
        let Ok(last) =
            chrono::DateTime::parse_from_rfc3339(&manifest.timestamps.last_updated_at)
        else {
            continue;
        };
        if last.with_timezone(&chrono::Utc) < cutoff {
            victims.push(e);
        }
    }
    if victims.is_empty() {
        println!("No cached runs older than {older_than_days} day(s).");
        return Ok(());
    }
    println!(
        "About to delete {} cached run(s) older than {} day(s):",
        victims.len(),
        older_than_days
    );
    for v in &victims {
        let last = v
            .manifest
            .as_ref()
            .map(|m| m.timestamps.last_updated_at.as_str())
            .unwrap_or("-");
        println!(
            "  - {} ({}, last updated {})",
            v.hash,
            human_bytes(v.size_bytes),
            last
        );
    }
    if dry_run {
        println!("(dry run; nothing deleted)");
        return Ok(());
    }
    if !yes {
        eprint!("Type 'yes' to confirm: ");
        std::io::stderr().flush().ok();
        let mut buf = String::new();
        std::io::stdin().read_line(&mut buf)?;
        if buf.trim() != "yes" {
            println!("Aborted.");
            return Ok(());
        }
    }
    let mut freed = 0u64;
    for v in &victims {
        fs::remove_dir_all(&v.dir).with_context(|| {
            format!("failed to remove cache dir {}", v.dir.display())
        })?;
        freed += v.size_bytes;
    }
    println!(
        "Deleted {} run(s); freed {}.",
        victims.len(),
        human_bytes(freed)
    );
    Ok(())
}

fn cache_clear(
    root: &Path,
    hash: Option<&str>,
    all: bool,
    yes: bool,
    dry_run: bool,
) -> Result<()> {
    if !all && hash.is_none() {
        bail!("specify --hash <HASH> or --all");
    }
    if all {
        let entries = collect_cache_entries(root)?;
        if entries.is_empty() {
            println!("(no cached runs)");
            return Ok(());
        }
        let total_size: u64 = entries.iter().map(|e| e.size_bytes).sum();
        println!(
            "About to delete all {} cached run(s) (total {}):",
            entries.len(),
            human_bytes(total_size)
        );
        for e in &entries {
            let last = e
                .manifest
                .as_ref()
                .map(|m| m.timestamps.last_updated_at.as_str())
                .unwrap_or("-");
            let input = e
                .manifest
                .as_ref()
                .map(|m| m.input.path_when_started.as_str())
                .unwrap_or("(no manifest)");
            println!(
                "  - {} ({}, last {}) {}",
                e.hash,
                human_bytes(e.size_bytes),
                last,
                input,
            );
        }
        println!("(output EPUB files are NOT touched)");
        if dry_run {
            println!("(dry run; nothing deleted)");
            return Ok(());
        }
        if !yes {
            eprint!("Type 'yes' to confirm: ");
            std::io::stderr().flush().ok();
            let mut buf = String::new();
            std::io::stdin().read_line(&mut buf)?;
            if buf.trim() != "yes" {
                println!("Aborted.");
                return Ok(());
            }
        }
        for e in &entries {
            fs::remove_dir_all(&e.dir).with_context(|| {
                format!("failed to remove cache dir {}", e.dir.display())
            })?;
        }
        println!("Deleted {} run(s); freed {}.", entries.len(), human_bytes(total_size));
        return Ok(());
    }
    let hash = hash.unwrap();
    let entry = resolve_cache_target(root, hash)?;
    if dry_run {
        println!(
            "Would delete {} ({})",
            entry.hash,
            human_bytes(entry.size_bytes)
        );
        return Ok(());
    }
    fs::remove_dir_all(&entry.dir).with_context(|| {
        format!("failed to remove cache dir {}", entry.dir.display())
    })?;
    println!("Deleted {} ({}).", entry.hash, human_bytes(entry.size_bytes));
    Ok(())
}

fn inspect_command(args: InspectArgs) -> Result<()> {
    let book = unpack_epub(&args.input)?;
    println!("OPF: {}", book.opf_path.display());
    println!();
    println!(
        "{:>4}  {:<6}  {:<8}  {:>10}  {:>7}  {:<22}  {}",
        "No", "Linear", "Exists", "Bytes", "Blocks", "Media Type", "Href"
    );
    println!("{}", "-".repeat(96));
    for (idx, item) in book.spine.iter().enumerate() {
        let metadata = fs::metadata(&item.abs_path).ok();
        let exists = metadata.is_some();
        let bytes = metadata
            .map(|m| m.len().to_string())
            .unwrap_or_else(|| "-".to_string());
        let blocks = if exists {
            count_xhtml_blocks(&item.abs_path)
                .map(|count| count.to_string())
                .unwrap_or_else(|_| "parseerr".to_string())
        } else {
            "-".to_string()
        };
        println!(
            "{:>4}  {:<6}  {:<8}  {:>10}  {:>7}  {:<22}  {}",
            idx + 1,
            if item.linear { "yes" } else { "no" },
            if exists { "yes" } else { "missing" },
            bytes,
            blocks,
            item.media_type,
            item.href
        );
        println!(
            "      idref={} path={}",
            item.idref,
            item.abs_path.display()
        );
    }
    Ok(())
}

fn toc_command(args: TocArgs) -> Result<()> {
    let book = unpack_epub(&args.input)?;
    println!("OPF: {}", book.opf_path.display());
    if let Some(nav) = find_nav_item(&book.manifest) {
        println!("TOC: EPUB3 nav ({})", nav.href);
        println!();
        let entries = read_nav_toc(&nav.abs_path)?;
        print_toc_entries(&entries);
        return Ok(());
    }
    if let Some(ncx) = find_ncx_item(&book.manifest) {
        println!("TOC: EPUB2 NCX ({})", ncx.href);
        println!();
        let entries = read_ncx_toc(&ncx.abs_path)?;
        print_toc_entries(&entries);
        return Ok(());
    }
    bail!("no EPUB3 nav.xhtml or EPUB2 NCX item found in OPF manifest")
}

fn glossary_command(args: GlossaryArgs) -> Result<()> {
    let book = unpack_epub(&args.input)?;
    let candidates = extract_glossary_candidates(&book, args.min_occurrences, args.max_entries)?;
    let glossary = GlossaryFile {
        model: None,
        source_lang: Some("en".to_string()),
        target_lang: Some("ja".to_string()),
        entries: candidates
            .into_iter()
            .map(|candidate| GlossaryEntry {
                src: candidate.term,
                dst: String::new(),
                kind: Some(candidate.kind),
                note: Some(format!("occurrences: {}", candidate.count)),
            })
            .collect(),
    };
    let json = serde_json::to_string_pretty(&glossary)?;
    fs::write(&args.output, json)
        .with_context(|| format!("failed to write {}", args.output.display()))?;
    if let Some(path) = &args.review_prompt {
        let prompt = glossary_review_prompt(&glossary);
        fs::write(path, prompt).with_context(|| format!("failed to write {}", path.display()))?;
        eprintln!("Wrote glossary review prompt to {}", path.display());
    }
    eprintln!(
        "Wrote {} glossary candidates to {}",
        glossary.entries.len(),
        args.output.display()
    );
    Ok(())
}

fn glossary_review_prompt(glossary: &GlossaryFile) -> String {
    let json = serde_json::to_string_pretty(glossary).unwrap_or_else(|_| "{}".to_string());
    format!(
        r#"# EPUB 翻訳用語集レビュー依頼

以下は、英語 EPUB から自動抽出した用語集候補です。
この文章全体を作業指示として読み、最後の JSON を修正してください。

## 作業目的

英日翻訳で表記ゆれを防ぐため、人名、地名、組織名、製品名、作品名、専門用語を整理した用語集 JSON を作成してください。

## 入力 JSON の見方

- `src`: 原文に出てきた英語表記です。
- `dst`: 日本語訳語です。空欄なので、自然で一貫した訳語を入れてください。
- `kind`: 候補の種類です。必要に応じて修正してください。
- `note`: 出現回数などのコメントです。判断材料として使い、必要なら短い補足コメントに直してください。

## 修正方針

- 重要な人名、地名、組織名、製品名、プロジェクト名、作品名、専門用語を残してください。
- 誤検出、章見出し、一般語、文頭に多いだけの単語は削除してください。
- 同じ対象を指す表記ゆれや重複は、最も標準的な `src` に統合してください。
- `dst` には、文脈上自然な日本語訳または一般的なカタカナ表記を入れてください。
- `kind` は次のいずれかにしてください: `person`, `place`, `organization`, `product`, `term`, `work`, `other`
- 判断に迷う候補は、残す場合だけ `note` に短い理由を入れてください。
- 出力は有効な JSON のみ。Markdown のコードフェンスや説明文は付けないでください。
- JSON の形は `source_lang`, `target_lang`, `entries` を維持してください。

## 修正対象 JSON

```json
{json}
```
"#
    )
}

fn default_output_path(input: &Path) -> PathBuf {
    let stem = input.file_stem().and_then(|s| s.to_str()).unwrap_or("book");
    input.with_file_name(format!("{stem}.ja.epub"))
}

fn normalize_range(
    from: Option<usize>,
    to: Option<usize>,
    len: usize,
) -> Result<std::ops::RangeInclusive<usize>> {
    if len == 0 {
        bail!("EPUB spine has no XHTML pages");
    }
    let from = from.unwrap_or(1);
    let to = to.unwrap_or(len);
    if from == 0 || to == 0 || from > to || to > len {
        bail!("invalid range {from}-{to}; valid spine page range is 1-{len}");
    }
    Ok(from..=to)
}

fn unpack_epub(input: &Path) -> Result<EpubBook> {
    let file = File::open(input).with_context(|| format!("failed to open {}", input.display()))?;
    let mut archive =
        ZipArchive::new(BufReader::new(file)).context("input is not a valid EPUB/ZIP")?;
    let work_dir = tempfile::tempdir().context("failed to create temp dir")?;
    archive
        .extract(work_dir.path())
        .context("failed to unpack EPUB")?;

    let container_path = work_dir.path().join("META-INF").join("container.xml");
    let opf_rel = read_container_rootfile(&container_path)?;
    let opf_path = work_dir.path().join(normalize_epub_path(&opf_rel));
    let opf_dir = opf_path.parent().unwrap_or(work_dir.path()).to_path_buf();
    let opf = read_opf(&opf_path, &opf_dir)?;
    Ok(EpubBook {
        work_dir,
        opf_path,
        manifest: opf.manifest,
        spine: opf.spine,
    })
}

fn read_container_rootfile(container_path: &Path) -> Result<String> {
    let mut reader = Reader::from_file(container_path)
        .with_context(|| format!("failed to read {}", container_path.display()))?;
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) | Event::Empty(e) if local_name(e.name().as_ref()) == b"rootfile" => {
                for attr in e.attributes().with_checks(false) {
                    let attr = attr?;
                    if local_name(attr.key.as_ref()) == b"full-path" {
                        return Ok(attr
                            .decode_and_unescape_value(reader.decoder())?
                            .into_owned());
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    bail!("META-INF/container.xml does not contain a rootfile full-path")
}

struct OpfData {
    manifest: Vec<ManifestItem>,
    spine: Vec<SpineItem>,
}

fn read_opf(opf_path: &Path, opf_dir: &Path) -> Result<OpfData> {
    let mut reader = Reader::from_file(opf_path)
        .with_context(|| format!("failed to read {}", opf_path.display()))?;
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut manifest = Vec::new();
    let mut idrefs = Vec::new();

    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) | Event::Empty(e) if local_name(e.name().as_ref()) == b"item" => {
                let mut id = None;
                let mut href = None;
                let mut media_type = None;
                let mut properties = Vec::new();
                for attr in e.attributes().with_checks(false) {
                    let attr = attr?;
                    let value = attr
                        .decode_and_unescape_value(reader.decoder())?
                        .into_owned();
                    match local_name(attr.key.as_ref()) {
                        b"id" => id = Some(value),
                        b"href" => href = Some(value),
                        b"media-type" => media_type = Some(value),
                        b"properties" => {
                            properties = value.split_whitespace().map(str::to_string).collect()
                        }
                        _ => {}
                    }
                }
                if let (Some(id), Some(href), Some(media_type)) = (id, href, media_type) {
                    let abs_path = opf_dir.join(normalize_epub_path(&href));
                    manifest.push(ManifestItem {
                        id,
                        href,
                        abs_path,
                        media_type,
                        properties,
                    });
                }
            }
            Event::Start(e) | Event::Empty(e) if local_name(e.name().as_ref()) == b"itemref" => {
                let mut idref = None;
                let mut linear = true;
                for attr in e.attributes().with_checks(false) {
                    let attr = attr?;
                    let value = attr
                        .decode_and_unescape_value(reader.decoder())?
                        .into_owned();
                    match local_name(attr.key.as_ref()) {
                        b"idref" => idref = Some(value),
                        b"linear" => linear = value != "no",
                        _ => {}
                    }
                }
                if let Some(idref) = idref {
                    idrefs.push(SpineRef { idref, linear });
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    let mut spine = Vec::new();
    let manifest_by_id = manifest
        .iter()
        .map(|item| (item.id.as_str(), item))
        .collect::<HashMap<_, _>>();
    for spine_ref in idrefs {
        let Some(item) = manifest_by_id.get(spine_ref.idref.as_str()) else {
            continue;
        };
        if item.media_type == "application/xhtml+xml"
            || item.href.ends_with(".xhtml")
            || item.href.ends_with(".html")
        {
            spine.push(SpineItem {
                idref: spine_ref.idref,
                href: item.href.clone(),
                abs_path: item.abs_path.clone(),
                media_type: item.media_type.clone(),
                linear: spine_ref.linear,
            });
        }
    }
    Ok(OpfData { manifest, spine })
}

fn count_xhtml_blocks(path: &Path) -> Result<usize> {
    let source = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut reader = Reader::from_reader(Cursor::new(source));
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut count = 0usize;
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) if is_block_tag(e.name()) => count += 1,
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(count)
}

fn extract_glossary_candidates(
    book: &EpubBook,
    min_occurrences: usize,
    max_entries: usize,
) -> Result<Vec<GlossaryCandidate>> {
    let mut counts = HashMap::<String, usize>::new();
    for item in &book.spine {
        let text = extract_plain_text(&item.abs_path)?;
        for candidate in find_term_candidates(&text) {
            *counts.entry(candidate).or_default() += 1;
        }
    }
    let mut candidates = counts
        .into_iter()
        .filter(|(_, count)| *count >= min_occurrences)
        .filter(|(term, _)| !is_glossary_stopword(term))
        .map(|(term, count)| GlossaryCandidate {
            kind: infer_glossary_kind(&term).to_string(),
            term,
            count,
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.term.to_lowercase().cmp(&b.term.to_lowercase()))
    });
    candidates.truncate(max_entries);
    Ok(candidates)
}

fn extract_plain_text(path: &Path) -> Result<String> {
    let source = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut reader = Reader::from_reader(Cursor::new(source));
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut out = String::new();
    let mut skip_depth = 0usize;
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) if is_never_translate_tag(e.name().as_ref()) => skip_depth += 1,
            Event::End(e) if is_never_translate_tag(e.name().as_ref()) => {
                skip_depth = skip_depth.saturating_sub(1);
            }
            Event::Text(t) if skip_depth == 0 => {
                out.push_str(&t.decode()?);
                out.push(' ');
            }
            Event::CData(t) if skip_depth == 0 => {
                out.push_str(&String::from_utf8_lossy(t.as_ref()));
                out.push(' ');
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(out)
}

fn find_term_candidates(text: &str) -> Vec<String> {
    let words = text
        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '\'' && ch != '-')
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();
    let mut candidates = Vec::new();
    let mut i = 0usize;
    while i < words.len() {
        if !is_capitalized_term_word(words[i]) {
            i += 1;
            continue;
        }
        let start = i;
        i += 1;
        while i < words.len() && is_capitalized_term_word(words[i]) {
            i += 1;
        }
        let len = i - start;
        if len == 1 {
            let word = words[start].trim_matches('\'');
            if word.len() >= 4 && !is_common_sentence_start(word) {
                candidates.push(word.to_string());
            }
        } else {
            let term = words[start..i].join(" ");
            if term.len() >= 4 {
                candidates.push(term);
            }
        }
    }
    candidates
}

fn is_capitalized_term_word(word: &str) -> bool {
    let word = word.trim_matches('\'');
    let mut chars = word.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_uppercase()
        && chars.any(|ch| ch.is_ascii_lowercase())
        && !word.chars().all(|ch| ch.is_ascii_uppercase())
}

fn is_common_sentence_start(word: &str) -> bool {
    matches!(
        word,
        "The"
            | "This"
            | "That"
            | "These"
            | "Those"
            | "There"
            | "When"
            | "Where"
            | "While"
            | "After"
            | "Before"
            | "Because"
            | "Although"
            | "However"
            | "He"
            | "She"
            | "They"
            | "We"
            | "I"
            | "It"
            | "Its"
            | "His"
            | "Her"
            | "Their"
            | "Our"
            | "You"
            | "Your"
            | "What"
            | "Who"
            | "Why"
            | "How"
            | "Chapter"
            | "Part"
            | "Table"
            | "Figure"
    )
}

fn is_glossary_stopword(term: &str) -> bool {
    is_common_sentence_start(term)
        || matches!(
            term,
            "Title Page" | "Copyright" | "Contents" | "Table of Contents" | "Introduction"
        )
}

fn infer_glossary_kind(term: &str) -> &'static str {
    if term.split_whitespace().count() >= 2 {
        "proper-noun"
    } else {
        "term"
    }
}

#[derive(Debug)]
struct TocEntry {
    level: usize,
    label: String,
    href: Option<String>,
}

fn find_nav_item(manifest: &[ManifestItem]) -> Option<&ManifestItem> {
    manifest
        .iter()
        .find(|item| {
            item.media_type == "application/xhtml+xml"
                && item.properties.iter().any(|property| property == "nav")
        })
        .or_else(|| {
            manifest.iter().find(|item| {
                item.media_type == "application/xhtml+xml"
                    && (item.href.ends_with("nav.xhtml")
                        || item.href.ends_with("nav.html")
                        || item.href.ends_with("toc.xhtml")
                        || item.href.ends_with("toc.html"))
            })
        })
}

fn find_ncx_item(manifest: &[ManifestItem]) -> Option<&ManifestItem> {
    manifest
        .iter()
        .find(|item| item.media_type == "application/x-dtbncx+xml")
}

fn read_nav_toc(path: &Path) -> Result<Vec<TocEntry>> {
    let source = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut reader = Reader::from_reader(Cursor::new(source));
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut in_toc_nav = false;
    let mut nav_depth = 0usize;
    let mut list_depth = 0usize;
    let mut current_anchor: Option<(usize, String, String)> = None;
    let mut entries = Vec::new();

    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) if local_name(e.name().as_ref()) == b"nav" => {
                if is_toc_nav(&e, reader.decoder())? {
                    in_toc_nav = true;
                    nav_depth = 1;
                }
            }
            Event::Start(e) if in_toc_nav => {
                nav_depth += 1;
                match local_name(e.name().as_ref()) {
                    b"ol" | b"ul" => list_depth += 1,
                    b"a" => {
                        let href = attr_value(&e, reader.decoder(), b"href")?.unwrap_or_default();
                        current_anchor = Some((list_depth.max(1), href, String::new()));
                    }
                    _ => {}
                }
            }
            Event::Text(t) if current_anchor.is_some() => {
                if let Some((_, _, label)) = current_anchor.as_mut() {
                    label.push_str(&t.decode()?);
                }
            }
            Event::CData(t) if current_anchor.is_some() => {
                if let Some((_, _, label)) = current_anchor.as_mut() {
                    label.push_str(&String::from_utf8_lossy(t.as_ref()));
                }
            }
            Event::End(e) if in_toc_nav && local_name(e.name().as_ref()) == b"a" => {
                if let Some((level, href, label)) = current_anchor.take() {
                    let label = collapse_ws(&label);
                    if !label.is_empty() {
                        entries.push(TocEntry {
                            level,
                            label,
                            href: if href.is_empty() { None } else { Some(href) },
                        });
                    }
                }
            }
            Event::End(e) if in_toc_nav => {
                match local_name(e.name().as_ref()) {
                    b"ol" | b"ul" => list_depth = list_depth.saturating_sub(1),
                    b"nav" if nav_depth == 1 => break,
                    _ => {}
                }
                nav_depth = nav_depth.saturating_sub(1);
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(entries)
}

fn is_toc_nav(e: &BytesStart<'_>, decoder: quick_xml::encoding::Decoder) -> Result<bool> {
    let mut epub_type = None;
    let mut role = None;
    for attr in e.attributes().with_checks(false) {
        let attr = attr?;
        let value = attr.decode_and_unescape_value(decoder)?.into_owned();
        match local_name(attr.key.as_ref()) {
            b"type" => epub_type = Some(value),
            b"role" => role = Some(value),
            _ => {}
        }
    }
    Ok(epub_type
        .as_deref()
        .map(|value| value.split_whitespace().any(|part| part == "toc"))
        .unwrap_or(false)
        || role.as_deref() == Some("doc-toc"))
}

fn read_ncx_toc(path: &Path) -> Result<Vec<TocEntry>> {
    let source = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut reader = Reader::from_reader(Cursor::new(source));
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut stack = Vec::<NcxNavPoint>::new();
    let mut in_text = false;
    let mut entries = Vec::new();

    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) if local_name(e.name().as_ref()) == b"navPoint" => {
                stack.push(NcxNavPoint::default());
            }
            Event::Start(e) if local_name(e.name().as_ref()) == b"text" && !stack.is_empty() => {
                in_text = true;
            }
            Event::Empty(e) | Event::Start(e)
                if local_name(e.name().as_ref()) == b"content" && !stack.is_empty() =>
            {
                if let Some(src) = attr_value(&e, reader.decoder(), b"src")? {
                    if let Some(current) = stack.last_mut() {
                        current.href = Some(src);
                    }
                }
            }
            Event::Text(t) if in_text => {
                if let Some(current) = stack.last_mut() {
                    current.label.push_str(&t.decode()?);
                }
            }
            Event::CData(t) if in_text => {
                if let Some(current) = stack.last_mut() {
                    current.label.push_str(&String::from_utf8_lossy(t.as_ref()));
                }
            }
            Event::End(e) if local_name(e.name().as_ref()) == b"text" => {
                in_text = false;
            }
            Event::End(e) if local_name(e.name().as_ref()) == b"navPoint" => {
                if let Some(point) = stack.pop() {
                    let label = collapse_ws(&point.label);
                    if !label.is_empty() {
                        entries.push(TocEntry {
                            level: stack.len() + 1,
                            label,
                            href: point.href,
                        });
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(entries)
}

#[derive(Default)]
struct NcxNavPoint {
    label: String,
    href: Option<String>,
}

fn attr_value(
    e: &BytesStart<'_>,
    decoder: quick_xml::encoding::Decoder,
    name: &[u8],
) -> Result<Option<String>> {
    for attr in e.attributes().with_checks(false) {
        let attr = attr?;
        if local_name(attr.key.as_ref()) == name {
            return Ok(Some(attr.decode_and_unescape_value(decoder)?.into_owned()));
        }
    }
    Ok(None)
}

fn print_toc_entries(entries: &[TocEntry]) {
    if entries.is_empty() {
        println!("(no TOC entries found)");
        return;
    }
    for entry in entries {
        let indent = "  ".repeat(entry.level.saturating_sub(1));
        match &entry.href {
            Some(href) => println!("{}- {} -> {}", indent, entry.label, href),
            None => println!("{}- {}", indent, entry.label),
        }
    }
}

struct ProgressReporter {
    bar: ProgressBar,
    total_blocks: u64,
    cached_blocks: u64,
    model_blocks: usize,
    started: Instant,
    page_message: String,
}

impl ProgressReporter {
    fn new(total_blocks: u64, cached_blocks: u64) -> Result<Self> {
        let bar = ProgressBar::new(total_blocks);
        bar.set_style(
            ProgressStyle::with_template(
                "{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos}/{len} blocks | {msg}",
            )?
            .progress_chars("=> "),
        );
        if cached_blocks > 0 {
            bar.set_position(cached_blocks);
        }
        let reporter = Self {
            bar,
            total_blocks,
            cached_blocks,
            model_blocks: 0,
            started: Instant::now(),
            page_message: if cached_blocks > 0 {
                format!("resuming: {cached_blocks}/{total_blocks} cached")
            } else {
                "preparing".to_string()
            },
        };
        reporter.refresh_message();
        Ok(reporter)
    }

    fn set_page(&mut self, page_no: usize, total_pages: usize, href: &str) {
        let prefix = if self.cached_blocks > 0 {
            format!("cached {}/{} | ", self.cached_blocks, self.total_blocks)
        } else {
            String::new()
        };
        self.page_message = format!("{prefix}page {page_no}/{total_pages} {href}");
        self.refresh_message();
    }

    fn inc_model_block(&mut self) {
        self.model_blocks += 1;
        self.bar.inc(1);
        self.refresh_message();
    }

    fn inc_passthrough_block(&mut self) {
        self.bar.inc(1);
        self.refresh_message();
    }

    fn finish(self, stats: &Stats) {
        self.bar.finish_with_message(format!(
            "done: {} pages, {} blocks",
            stats.pages_translated, stats.blocks_translated
        ));
    }

    fn refresh_message(&self) {
        self.bar
            .set_message(format!("{} | {}", self.eta_message(), self.page_message));
    }

    fn eta_message(&self) -> String {
        let pos = self.bar.position();
        let remaining = self.total_blocks.saturating_sub(pos);
        if remaining == 0 {
            return "ETA done".to_string();
        }
        if self.model_blocks < ETA_MIN_MODEL_BLOCKS {
            return format!(
                "ETA warming up ({}/{ETA_MIN_MODEL_BLOCKS} model blocks)",
                self.model_blocks
            );
        }
        let elapsed = self.started.elapsed();
        if elapsed < Duration::from_secs(ETA_MIN_ELAPSED_SECS) {
            return format!(
                "ETA warming up ({}s/{ETA_MIN_ELAPSED_SECS}s)",
                elapsed.as_secs()
            );
        }
        let seconds_per_block = elapsed.as_secs_f64() / self.model_blocks as f64;
        let eta = Duration::from_secs_f64(seconds_per_block * remaining as f64);
        format!("ETA {}", format_duration_hms(eta))
    }
}

fn format_duration_hms(duration: Duration) -> String {
    let total = duration.as_secs();
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let seconds = total % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

fn translate_book(
    book: &EpubBook,
    range: std::ops::RangeInclusive<usize>,
    translator: &mut Translator,
    mode: Mode,
    show_progress: bool,
) -> Result<Stats> {
    let selected: HashSet<usize> = range.collect();
    let mut stats = Stats::default();
    let mut progress = if show_progress {
        let total_blocks = count_selected_blocks(book, &selected)?;
        let cached_blocks = count_selected_cached_blocks(book, &selected, translator)?;
        Some(ProgressReporter::new(total_blocks, cached_blocks)?)
    } else {
        None
    };
    for (idx, item) in book.spine.iter().enumerate() {
        let page_no = idx + 1;
        if !selected.contains(&page_no) {
            continue;
        }
        stats.pages_translated += 1;
        if let Some(progress) = progress.as_mut() {
            progress.set_page(page_no, book.spine.len(), &item.href);
        }
        if mode == Mode::Stdout {
            println!("\n===== spine page {page_no}: {} =====\n", item.href);
        }
        let result = translate_xhtml_file(&item.abs_path, translator, mode, progress.as_mut())
            .with_context(|| format!("failed to translate spine page {page_no}: {}", item.href))?;
        stats.blocks_translated += result;
    }
    if let Some(progress) = progress {
        progress.finish(&stats);
    }
    Ok(stats)
}

fn count_selected_blocks(book: &EpubBook, selected: &HashSet<usize>) -> Result<u64> {
    let mut total = 0u64;
    for (idx, item) in book.spine.iter().enumerate() {
        if selected.contains(&(idx + 1)) {
            total += count_xhtml_blocks(&item.abs_path)? as u64;
        }
    }
    Ok(total)
}

fn count_selected_cached_blocks(
    book: &EpubBook,
    selected: &HashSet<usize>,
    translator: &Translator,
) -> Result<u64> {
    let mut total = 0u64;
    for (idx, item) in book.spine.iter().enumerate() {
        if selected.contains(&(idx + 1)) {
            total += count_cached_xhtml_blocks(&item.abs_path, translator)? as u64;
        }
    }
    Ok(total)
}

fn count_cached_xhtml_blocks(path: &Path, translator: &Translator) -> Result<usize> {
    let source = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut reader = Reader::from_reader(Cursor::new(source));
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut count = 0usize;

    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) if is_block_tag(e.name()) => {
                let end_name = e.name().as_ref().to_vec();
                let inner = collect_element_inner(&mut reader, &end_name)?;
                let (source_text, _) = encode_inline(&inner)?;
                if !source_text.trim().is_empty() && translator.has_cached_translation(&source_text)
                {
                    count += 1;
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(count)
}

fn translate_xhtml_file(
    path: &Path,
    translator: &mut Translator,
    mode: Mode,
    mut progress: Option<&mut ProgressReporter>,
) -> Result<usize> {
    let source = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut reader = Reader::from_reader(Cursor::new(source));
    reader.config_mut().trim_text(false);
    let mut writer = Writer::new(Vec::new());
    let mut buf = Vec::new();
    let mut blocks = 0usize;

    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) if is_block_tag(e.name()) => {
                let start = e.into_owned();
                let end_name = start.name().as_ref().to_vec();
                let inner = collect_element_inner(&mut reader, &end_name)?;
                let (source_text, inline_map) = encode_inline(&inner)?;
                if source_text.trim().is_empty() {
                    if mode == Mode::Write {
                        writer.write_event(Event::Start(start))?;
                        write_events(&mut writer, &inner)?;
                        writer.write_event(Event::End(BytesEnd::new(String::from_utf8_lossy(
                            &end_name,
                        ))))?;
                    }
                } else {
                    let translation = translator.translate(&source_text)?;
                    match translation {
                        Translation::Translated {
                            text: translated,
                            from_cache,
                        } => {
                            if !from_cache {
                                if let Some(progress) = progress.as_mut() {
                                    progress.inc_model_block();
                                }
                            }
                            if mode == Mode::Stdout {
                                blocks += 1;
                                println!("{}", translated.trim());
                                println!();
                            } else {
                                let (restored, used_translation) =
                                    restore_inline_or_original(&translated, &inline_map, &inner);
                                if used_translation || translator.dry_run {
                                    blocks += 1;
                                }
                                writer.write_event(Event::Start(start))?;
                                write_events(&mut writer, &restored)?;
                                writer.write_event(Event::End(BytesEnd::new(
                                    String::from_utf8_lossy(&end_name),
                                )))?;
                            }
                        }
                        Translation::Original => {
                            if let Some(progress) = progress.as_mut() {
                                progress.inc_passthrough_block();
                            }
                            if mode == Mode::Stdout {
                                println!("{}", source_text.trim());
                                println!();
                            } else {
                                writer.write_event(Event::Start(start))?;
                                write_events(&mut writer, &inner)?;
                                writer.write_event(Event::End(BytesEnd::new(
                                    String::from_utf8_lossy(&end_name),
                                )))?;
                            }
                        }
                    }
                }
            }
            Event::Eof => break,
            event => {
                if mode == Mode::Write {
                    writer.write_event(event.into_owned())?;
                }
            }
        }
        buf.clear();
    }

    if mode == Mode::Write {
        fs::write(path, writer.into_inner())
            .with_context(|| format!("failed to write {}", path.display()))?;
    }
    Ok(blocks)
}

fn collect_element_inner<R: std::io::BufRead>(
    reader: &mut Reader<R>,
    end_name: &[u8],
) -> Result<Vec<Event<'static>>> {
    let mut buf = Vec::new();
    let mut depth = 0usize;
    let mut events = Vec::new();
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) => {
                depth += 1;
                events.push(Event::Start(e.into_owned()));
            }
            Event::End(e) if depth == 0 && e.name().as_ref() == end_name => break,
            Event::End(e) => {
                depth = depth.saturating_sub(1);
                events.push(Event::End(e.into_owned()));
            }
            Event::Eof => bail!("unexpected EOF while reading XHTML block"),
            event => events.push(event.into_owned()),
        }
        buf.clear();
    }
    Ok(events)
}

fn encode_inline(events: &[Event<'static>]) -> Result<(String, InlineMap)> {
    let mut text = String::new();
    let mut map = InlineMap::default();
    let mut next_id = 1u32;
    let mut stack: Vec<(Vec<u8>, u32)> = Vec::new();
    for event in events {
        match event {
            Event::Text(t) => text.push_str(&t.decode()?),
            Event::CData(t) => text.push_str(&String::from_utf8_lossy(t.as_ref())),
            Event::Start(e) => {
                let id = next_id;
                next_id += 1;
                map.entries.entry(id).or_default().start = Some(serialize_event(event)?);
                stack.push((e.name().as_ref().to_vec(), id));
                text.push_str(&format!("⟦E{id}⟧"));
            }
            Event::End(e) => {
                let id = stack
                    .iter()
                    .rposition(|(name, _)| name.as_slice() == e.name().as_ref())
                    .map(|pos| stack.remove(pos).1)
                    .unwrap_or_else(|| {
                        let id = next_id;
                        next_id += 1;
                        id
                    });
                map.entries.entry(id).or_default().end = Some(serialize_event(event)?);
                text.push_str(&format!("⟦/E{id}⟧"));
            }
            Event::Empty(_) => {
                let id = next_id;
                next_id += 1;
                map.entries.entry(id).or_default().empty = Some(serialize_event(event)?);
                text.push_str(&format!("⟦S{id}⟧"));
            }
            _ => {}
        }
    }
    Ok((collapse_ws(&text), map))
}

fn restore_inline(translated: &str, map: &InlineMap) -> Result<Vec<Event<'static>>> {
    let tokens = tokenize_placeholders(translated);
    validate_placeholder_ids(&tokens, map)?;
    let mut events = Vec::new();
    for token in tokens {
        match token {
            Token::Text(s) if !s.is_empty() => {
                events.push(Event::Text(BytesText::new(&s).into_owned()))
            }
            Token::Text(_) => {}
            Token::Open(id) | Token::Close(id) | Token::SelfClose(id) => {
                let Some(entry) = map.entries.get(&id) else {
                    bail!("unknown placeholder id {id}");
                };
                let bytes = match token {
                    Token::Open(_) => entry.start.as_ref(),
                    Token::Close(_) => entry.end.as_ref(),
                    Token::SelfClose(_) => entry.empty.as_ref(),
                    Token::Text(_) => unreachable!(),
                };
                let Some(bytes) = bytes else {
                    bail!("placeholder kind mismatch for id {id}");
                };
                events.push(parse_single_event(bytes)?);
            }
        }
    }
    Ok(events)
}

fn restore_inline_or_original(
    translated: &str,
    map: &InlineMap,
    original: &[Event<'static>],
) -> (Vec<Event<'static>>, bool) {
    match restore_inline(translated, map) {
        Ok(events) => (events, true),
        Err(_) => (original.to_vec(), false),
    }
}

fn validate_placeholder_ids(tokens: &[Token], map: &InlineMap) -> Result<()> {
    let mut open_seen = HashSet::new();
    let mut close_seen = HashSet::new();
    let mut self_seen = HashSet::new();
    for token in tokens {
        match token {
            Token::Open(id) => {
                open_seen.insert(*id);
            }
            Token::Close(id) => {
                close_seen.insert(*id);
            }
            Token::SelfClose(id) => {
                self_seen.insert(*id);
            }
            Token::Text(_) => {}
        }
    }

    for (id, entry) in &map.entries {
        if entry.empty.is_some() {
            if !self_seen.contains(id) {
                bail!("missing self-closing placeholder S{id}");
            }
        } else {
            if entry.start.is_some() && !open_seen.contains(id) {
                bail!("missing opening placeholder E{id}");
            }
            if entry.end.is_some() && !close_seen.contains(id) {
                bail!("missing closing placeholder /E{id}");
            }
        }
    }
    Ok(())
}

#[derive(Debug)]
enum Token {
    Text(String),
    Open(u32),
    Close(u32),
    SelfClose(u32),
}

fn tokenize_placeholders(s: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut rest = s;
    while let Some(start) = rest.find('⟦') {
        let (before, after_start) = rest.split_at(start);
        tokens.push(Token::Text(before.to_string()));
        if let Some(end) = after_start.find('⟧') {
            let marker = &after_start['⟦'.len_utf8()..end];
            if let Some(num) = marker.strip_prefix("/E").and_then(|n| n.parse().ok()) {
                tokens.push(Token::Close(num));
            } else if let Some(num) = marker.strip_prefix('E').and_then(|n| n.parse().ok()) {
                tokens.push(Token::Open(num));
            } else if let Some(num) = marker.strip_prefix('S').and_then(|n| n.parse().ok()) {
                tokens.push(Token::SelfClose(num));
            } else {
                tokens.push(Token::Text(after_start[..end + '⟧'.len_utf8()].to_string()));
            }
            rest = &after_start[end + '⟧'.len_utf8()..];
        } else {
            tokens.push(Token::Text(after_start.to_string()));
            rest = "";
        }
    }
    if !rest.is_empty() {
        tokens.push(Token::Text(rest.to_string()));
    }
    tokens
}

fn serialize_event(event: &Event<'static>) -> Result<Vec<u8>> {
    let mut writer = Writer::new(Vec::new());
    writer.write_event(event.clone())?;
    Ok(writer.into_inner())
}

fn parse_single_event(bytes: &[u8]) -> Result<Event<'static>> {
    let mut reader = Reader::from_reader(Cursor::new(bytes));
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Eof => bail!("could not parse serialized inline event"),
            event => return Ok(event.into_owned()),
        }
    }
}

fn write_events<W: Write>(writer: &mut Writer<W>, events: &[Event<'static>]) -> Result<()> {
    for event in events {
        writer.write_event(event.clone())?;
    }
    Ok(())
}

fn collapse_ws(s: &str) -> String {
    let mut out = String::new();
    let mut last_space = false;
    for ch in s.chars() {
        if ch.is_whitespace() {
            if !last_space {
                out.push(' ');
                last_space = true;
            }
        } else {
            out.push(ch);
            last_space = false;
        }
    }
    out.trim().to_string()
}

struct Translator {
    provider: Provider,
    model: String,
    ollama_host: String,
    openai_base_url: String,
    claude_base_url: String,
    openai_api_key: Option<String>,
    anthropic_api_key: Option<String>,
    temperature: f32,
    num_ctx: u32,
    retries: u32,
    style: String,
    glossary: Vec<GlossaryEntry>,
    cache: CacheStore,
    partial_from_cache: bool,
    dry_run: bool,
    client: Client,
}

enum Translation {
    Translated { text: String, from_cache: bool },
    Original,
}

impl Translator {
    fn new(args: CommonArgs, cache: CacheStore) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(args.timeout_secs))
            .build()
            .context("failed to create HTTP client")?;
        let model = args.model.unwrap_or_else(|| match args.provider {
            Provider::Ollama => DEFAULT_MODEL.to_string(),
            Provider::Openai => DEFAULT_OPENAI_MODEL.to_string(),
            Provider::Claude => DEFAULT_CLAUDE_MODEL.to_string(),
        });
        let openai_api_key = read_api_key(
            args.provider,
            Provider::Openai,
            args.openai_api_key,
            "OPENAI_API_KEY",
            args.prompt_api_key,
        )?;
        let anthropic_api_key = read_api_key(
            args.provider,
            Provider::Claude,
            args.anthropic_api_key,
            "ANTHROPIC_API_KEY",
            args.prompt_api_key,
        )?;
        let glossary = match args.glossary {
            Some(path) => load_glossary(&path)?,
            None => Vec::new(),
        };
        Ok(Self {
            provider: args.provider,
            model,
            ollama_host: args.ollama_host.trim_end_matches('/').to_string(),
            openai_base_url: args.openai_base_url.trim_end_matches('/').to_string(),
            claude_base_url: args.claude_base_url.trim_end_matches('/').to_string(),
            openai_api_key,
            anthropic_api_key,
            temperature: args.temperature,
            num_ctx: args.num_ctx,
            retries: args.retries,
            style: args.style,
            glossary,
            cache,
            partial_from_cache: args.partial_from_cache,
            dry_run: args.dry_run,
            client,
        })
    }

    fn translate(&mut self, source: &str) -> Result<Translation> {
        if self.dry_run {
            return Ok(Translation::Translated {
                text: source.to_string(),
                from_cache: false,
            });
        }
        let glossary_subset = self.glossary_subset(source);
        let key = self.cache_key(source, &glossary_subset);
        if let Some(translated) = self.cache.get(&key) {
            return Ok(Translation::Translated {
                text: translated,
                from_cache: true,
            });
        }
        if self.partial_from_cache {
            return Ok(Translation::Original);
        }
        let prompt = user_prompt(source, &glossary_subset);
        match self.provider {
            Provider::Ollama => self.translate_ollama(&prompt),
            Provider::Openai => self.translate_openai(&prompt),
            Provider::Claude => self.translate_claude(&prompt),
        }
        .and_then(|translated| {
            let record = CacheRecord {
                key,
                translated: translated.clone(),
                provider: self.provider.to_string(),
                model: self.model.clone(),
                at: chrono::Utc::now().to_rfc3339(),
            };
            self.cache.insert(record)?;
            Ok(Translation::Translated {
                text: translated,
                from_cache: false,
            })
        })
    }

    fn has_cached_translation(&self, source: &str) -> bool {
        if self.dry_run {
            return false;
        }
        let glossary_subset = self.glossary_subset(source);
        let key = self.cache_key(source, &glossary_subset);
        self.cache.contains_key(&key)
    }

    fn cache_key(&self, source: &str, glossary_subset: &[GlossaryEntry]) -> String {
        cache_key(
            self.provider,
            &self.model,
            &self.style,
            source,
            glossary_subset,
        )
    }

    fn manifest_params(&self) -> ManifestParams {
        ManifestParams {
            provider: self.provider.to_string(),
            model: self.model.clone(),
            prompt_version: "v1".to_string(),
            style_id: self.style.clone(),
            glossary_sha: glossary_sha(&self.glossary),
        }
    }

    fn glossary_subset(&self, source: &str) -> Vec<GlossaryEntry> {
        let source_lower = source.to_lowercase();
        self.glossary
            .iter()
            .filter(|entry| !entry.src.trim().is_empty() && !entry.dst.trim().is_empty())
            .filter(|entry| source_lower.contains(&entry.src.to_lowercase()))
            .cloned()
            .collect()
    }

    fn translate_ollama(&self, user_prompt: &str) -> Result<String> {
        let payload = serde_json::json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": system_prompt(&self.style)},
                {"role": "user", "content": user_prompt}
            ],
            "stream": false,
            "options": {
                "temperature": self.temperature,
                "top_p": 0.9,
                "num_ctx": self.num_ctx,
                "seed": 42
            }
        });
        let response: OllamaResponse = self.request_json_with_retry("Ollama", || {
            self.client
                .post(format!("{}/api/chat", self.ollama_host))
                .json(&payload)
        })?;
        Ok(response.message.content.trim().to_string())
    }

    fn translate_openai(&self, user_prompt: &str) -> Result<String> {
        let api_key = self.openai_api_key.as_deref().context(
            "OpenAI provider requires OPENAI_API_KEY, --openai-api-key, or --prompt-api-key",
        )?;
        let payload = serde_json::json!({
            "model": self.model,
            "instructions": system_prompt(&self.style),
            "input": user_prompt
        });
        let value: serde_json::Value = self.request_json_with_retry("OpenAI", || {
            self.client
                .post(format!("{}/responses", self.openai_base_url))
                .bearer_auth(api_key)
                .json(&payload)
        })?;
        extract_openai_text(&value).context("OpenAI response did not contain output text")
    }

    fn translate_claude(&self, user_prompt: &str) -> Result<String> {
        let api_key = self.anthropic_api_key.as_deref().context(
            "Claude provider requires ANTHROPIC_API_KEY, --anthropic-api-key, or --prompt-api-key",
        )?;
        let payload = serde_json::json!({
            "model": self.model,
            "max_tokens": 2048,
            "temperature": self.temperature,
            "system": system_prompt(&self.style),
            "messages": [
                {"role": "user", "content": user_prompt}
            ]
        });
        let response: ClaudeResponse = self.request_json_with_retry("Claude", || {
            self.client
                .post(format!("{}/messages", self.claude_base_url))
                .header("x-api-key", api_key)
                .header("anthropic-version", ANTHROPIC_VERSION)
                .header("content-type", "application/json")
                .json(&payload)
        })?;
        let text = response
            .content
            .into_iter()
            .filter(|part| part.kind == "text")
            .filter_map(|part| part.text)
            .collect::<Vec<_>>()
            .join("");
        if text.trim().is_empty() {
            bail!("Claude response did not contain text content");
        }
        Ok(text.trim().to_string())
    }

    fn request_json_with_retry<T, F>(&self, provider: &str, build: F) -> Result<T>
    where
        T: DeserializeOwned,
        F: Fn() -> reqwest::blocking::RequestBuilder,
    {
        let attempts = self.retries.saturating_add(1).max(1);
        for attempt in 1..=attempts {
            let result = build()
                .send()
                .and_then(|response| response.error_for_status())
                .and_then(|response| response.json::<T>());
            match result {
                Ok(value) => return Ok(value),
                Err(err) if attempt < attempts && should_retry_request(&err) => {
                    let wait_secs = 2_u64.saturating_pow((attempt - 1).min(5));
                    eprintln!(
                        "warning: {provider} request failed on attempt {attempt}/{attempts}: {err}; retrying in {wait_secs}s"
                    );
                    thread::sleep(Duration::from_secs(wait_secs));
                }
                Err(err) => {
                    return Err(err).with_context(|| {
                        format!("failed to call {provider} after {attempt} attempt(s)")
                    });
                }
            }
        }
        bail!("failed to call {provider}")
    }
}

fn should_retry_request(err: &reqwest::Error) -> bool {
    if err.is_timeout() || err.is_connect() || err.is_request() {
        return true;
    }
    err.status()
        .map(|status| status.as_u16() == 429 || status.is_server_error())
        .unwrap_or(false)
}

fn read_api_key(
    active_provider: Provider,
    key_provider: Provider,
    explicit: Option<String>,
    env_name: &str,
    prompt: bool,
) -> Result<Option<String>> {
    if active_provider != key_provider {
        return Ok(None);
    }
    if explicit.is_some() {
        return Ok(explicit);
    }
    if let Ok(value) = std::env::var(env_name) {
        if !value.trim().is_empty() {
            return Ok(Some(value));
        }
    }
    if prompt {
        let value = rpassword::prompt_password(format!("{env_name}: "))
            .with_context(|| format!("failed to read {env_name} from prompt"))?;
        if !value.trim().is_empty() {
            return Ok(Some(value));
        }
    }
    Ok(None)
}

fn extract_openai_text(value: &serde_json::Value) -> Option<String> {
    if let Some(text) = value.get("output_text").and_then(|v| v.as_str()) {
        return Some(text.trim().to_string());
    }
    let mut parts = Vec::new();
    for item in value.get("output")?.as_array()? {
        for content in item
            .get("content")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
        {
            if let Some(text) = content.get("text").and_then(|v| v.as_str()) {
                parts.push(text);
            }
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("").trim().to_string())
    }
}

fn load_glossary(path: &Path) -> Result<Vec<GlossaryEntry>> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("failed to read glossary {}", path.display()))?;
    let glossary: GlossaryFile = serde_json::from_str(&text)
        .with_context(|| format!("failed to parse glossary {}", path.display()))?;
    Ok(glossary.entries)
}

fn read_cache_entries(path: &Path) -> Result<HashMap<String, String>> {
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let file =
        File::open(path).with_context(|| format!("failed to open cache {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut entries = HashMap::new();
    for (line_no, line) in reader.lines().enumerate() {
        let line = line.with_context(|| format!("failed to read cache line {}", line_no + 1))?;
        if line.trim().is_empty() {
            continue;
        }
        let record: CacheRecord = serde_json::from_str(&line)
            .with_context(|| format!("failed to parse cache line {}", line_no + 1))?;
        entries.insert(record.key, record.translated);
    }
    Ok(entries)
}

fn cache_key(
    provider: Provider,
    model: &str,
    style: &str,
    source: &str,
    glossary_subset: &[GlossaryEntry],
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"epubicus-cache-v1\n");
    hasher.update(provider.to_string().as_bytes());
    hasher.update(b"\n");
    hasher.update(model.as_bytes());
    hasher.update(b"\n");
    hasher.update(style.as_bytes());
    hasher.update(b"\n");
    for entry in glossary_subset {
        hasher.update(entry.src.as_bytes());
        hasher.update(b"=>");
        hasher.update(entry.dst.as_bytes());
        hasher.update(b"\n");
    }
    hasher.update(source.as_bytes());
    let digest = hasher.finalize();
    digest[..16].iter().map(|b| format!("{b:02x}")).collect()
}

fn user_prompt(source: &str, glossary_subset: &[GlossaryEntry]) -> String {
    let mut prompt = String::new();
    if !glossary_subset.is_empty() {
        prompt.push_str("<glossary>\n");
        for entry in glossary_subset {
            prompt.push_str("- ");
            prompt.push_str(entry.src.trim());
            prompt.push_str(" => ");
            prompt.push_str(entry.dst.trim());
            if let Some(kind) = entry.kind.as_deref().filter(|kind| !kind.trim().is_empty()) {
                prompt.push_str(" (");
                prompt.push_str(kind.trim());
                prompt.push(')');
            }
            prompt.push('\n');
        }
        prompt.push_str("</glossary>\n\n");
    }
    prompt.push_str("<source>\n");
    prompt.push_str(source);
    prompt.push_str("\n</source>");
    prompt
}

fn system_prompt(style: &str) -> String {
    format!(
        "あなたは英日翻訳の専門家です。出版物として通用する自然で読みやすい日本語に翻訳してください。\n\n\
【絶対遵守ルール】\n\
1. 入力中の ⟦…⟧ で囲まれたマーカは、形を一切変えずに訳文に含めてください。\n\
2. マーカの順序は日本語として自然になるように入れ替えて構いませんが、原文に現れた全てのマーカを過不足なく残してください。\n\
3. マーカの中身を改変・追加・削除しないでください。\n\
4. 翻訳のみを出力し、説明・前置き・括弧書きの注釈を一切付けないでください。\n\
5. <glossary> が与えられた場合、そこにある訳語を必ず使用し、表記を統一してください。\n\n{}",
        style_prompt(style)
    )
}

fn style_prompt(style: &str) -> &'static str {
    match style {
        "novel" => {
            "【文体】\n- 地の文: である調。一文を長くしすぎず、句読点でリズムを整えてください。\n- 会話文: 話者の人物像にふさわしい口語。\n- 章タイトル: 簡潔。体言止めを基本にしてください。"
        }
        "novel-polite" => {
            "【文体】\n- 地の文: です・ます調。児童にも読みやすい自然な日本語にしてください。\n- 漢字を控えめにし、会話文は話者に合わせてください。"
        }
        "tech" => {
            "【文体】\n- 地の文: である調。事実を淡々と述べる調子。\n- 専門用語は一般に流通している訳語を優先してください。\n- コードや識別子は翻訳しないでください。"
        }
        "academic" => {
            "【文体】\n- 地の文: 硬めのである調。訳語の厳密さを優先してください。\n- 章タイトルは名詞句を基本にしてください。"
        }
        "business" => {
            "【文体】\n- 地の文: です・ます調。過度にくだけず、実務書として自然な表現にしてください。"
        }
        _ => {
            "【文体】\n- 地の文: である調。原文の論旨と語り口を尊重しつつ、日本語として自然にしてください。\n- 章タイトル: 体言止めを基本にしてください。"
        }
    }
}

fn update_opf_metadata(opf_path: &Path, model: &str) -> Result<()> {
    let source = fs::read(opf_path)?;
    let mut reader = Reader::from_reader(Cursor::new(source));
    reader.config_mut().trim_text(false);
    let mut writer = Writer::new(Vec::new());
    let mut buf = Vec::new();
    let mut in_language = false;
    let mut wrote_contributor = false;

    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) if local_name(e.name().as_ref()) == b"language" => {
                in_language = true;
                writer.write_event(Event::Start(e.into_owned()))?;
            }
            Event::Text(_) if in_language => {
                writer.write_event(Event::Text(BytesText::new("ja").into_owned()))?;
            }
            Event::End(e) if local_name(e.name().as_ref()) == b"language" => {
                in_language = false;
                writer.write_event(Event::End(e.into_owned()))?;
            }
            Event::End(e) if local_name(e.name().as_ref()) == b"metadata" => {
                if !wrote_contributor {
                    let mut contributor = BytesStart::new("dc:contributor");
                    contributor.push_attribute(("id", "epubicus-translator"));
                    writer.write_event(Event::Start(contributor))?;
                    writer.write_event(Event::Text(
                        BytesText::new(&format!("epubicus (model: {model})")).into_owned(),
                    ))?;
                    writer.write_event(Event::End(BytesEnd::new("dc:contributor")))?;
                    let mut role = BytesStart::new("meta");
                    role.push_attribute(("refines", "#epubicus-translator"));
                    role.push_attribute(("property", "role"));
                    role.push_attribute(("scheme", "marc:relators"));
                    writer.write_event(Event::Start(role))?;
                    writer.write_event(Event::Text(BytesText::new("trl").into_owned()))?;
                    writer.write_event(Event::End(BytesEnd::new("meta")))?;
                    wrote_contributor = true;
                }
                writer.write_event(Event::End(e.into_owned()))?;
            }
            Event::Eof => break,
            event => writer.write_event(event.into_owned())?,
        }
        buf.clear();
    }
    fs::write(opf_path, writer.into_inner())?;
    Ok(())
}

fn pack_epub(root: &Path, output: &Path) -> Result<()> {
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    let file =
        File::create(output).with_context(|| format!("failed to create {}", output.display()))?;
    let mut zip = ZipWriter::new(BufWriter::new(file));
    let stored: SimpleFileOptions =
        FileOptions::default().compression_method(CompressionMethod::Stored);
    let deflated: SimpleFileOptions =
        FileOptions::default().compression_method(CompressionMethod::Deflated);

    let mimetype = root.join("mimetype");
    if mimetype.exists() {
        zip.start_file("mimetype", stored)?;
        zip.write_all(&fs::read(&mimetype)?)?;
    }

    let mut files = WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .collect::<Vec<_>>();
    files.sort();

    for path in files {
        let rel = path
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/");
        if rel == "mimetype" {
            continue;
        }
        zip.start_file(rel, deflated)?;
        zip.write_all(&fs::read(path)?)?;
    }
    zip.finish()?;
    Ok(())
}

fn normalize_epub_path(path: &str) -> PathBuf {
    path.split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect()
}

fn local_name(name: &[u8]) -> &[u8] {
    name.rsplit(|b| *b == b':').next().unwrap_or(name)
}

fn is_block_tag(name: QName<'_>) -> bool {
    matches!(
        local_name(name.as_ref()),
        b"p" | b"h1"
            | b"h2"
            | b"h3"
            | b"h4"
            | b"h5"
            | b"h6"
            | b"li"
            | b"blockquote"
            | b"figcaption"
            | b"aside"
            | b"dt"
            | b"dd"
            | b"caption"
            | b"td"
            | b"th"
            | b"summary"
    )
}

fn is_never_translate_tag(name: &[u8]) -> bool {
    matches!(
        local_name(name),
        b"code" | b"pre" | b"kbd" | b"samp" | b"var" | b"tt" | b"script" | b"style" | b"math"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dry_run_roundtrips_minimal_epub() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let input = dir.path().join("minimal.epub");
        let output = dir.path().join("minimal.ja.epub");
        write_minimal_epub(&input)?;

        let book = unpack_epub(&input)?;
        assert_eq!(book.spine.len(), 2);

        let common = CommonArgs {
            provider: Provider::Ollama,
            model: Some(DEFAULT_MODEL.to_string()),
            ollama_host: DEFAULT_OLLAMA_HOST.to_string(),
            openai_base_url: DEFAULT_OPENAI_BASE_URL.to_string(),
            claude_base_url: DEFAULT_CLAUDE_BASE_URL.to_string(),
            openai_api_key: None,
            anthropic_api_key: None,
            prompt_api_key: false,
            temperature: 0.3,
            num_ctx: 8192,
            timeout_secs: 900,
            retries: 2,
            style: "essay".to_string(),
            glossary: None,
            cache_root: None,
            no_cache: true,
            clear_cache: false,
            keep_cache: false,
            partial_from_cache: false,
            dry_run: true,
        };
        let cache = CacheStore::from_args(&input, &common)?;
        let mut translator = Translator::new(common, cache)?;
        let stats = translate_book(&book, 1..=2, &mut translator, Mode::Write, false)?;
        assert_eq!(stats.pages_translated, 2);
        assert_eq!(stats.blocks_translated, 3);
        update_opf_metadata(&book.opf_path, &translator.model)?;
        pack_epub(book.work_dir.path(), &output)?;

        let repacked = unpack_epub(&output)?;
        assert_eq!(repacked.spine.len(), 2);
        Ok(())
    }

    #[test]
    fn placeholder_tokens_keep_marker_kinds() {
        let tokens = tokenize_placeholders("A ⟦E1⟧B⟦/E1⟧ ⟦S2⟧");
        assert!(matches!(tokens[1], Token::Open(1)));
        assert!(matches!(tokens[3], Token::Close(1)));
        assert!(matches!(tokens[5], Token::SelfClose(2)));
    }

    #[test]
    fn inline_restore_failure_keeps_original_link_markup() -> Result<()> {
        let source = br##"<p xmlns:epub="http://www.idpf.org/2007/ops"><a href="chapter.xhtml#note" epub:type="noteref">1</a> Footnote text.</p>"##;
        let mut reader = Reader::from_reader(Cursor::new(source));
        reader.config_mut().trim_text(false);
        let mut buf = Vec::new();
        let inner = loop {
            match reader.read_event_into(&mut buf)? {
                Event::Start(e) if local_name(e.name().as_ref()) == b"p" => {
                    let end_name = e.name().as_ref().to_vec();
                    break collect_element_inner(&mut reader, &end_name)?;
                }
                Event::Eof => bail!("missing test paragraph"),
                _ => {}
            }
            buf.clear();
        };
        let (_, inline_map) = encode_inline(&inner)?;
        let (restored, used_translation) =
            restore_inline_or_original("脚注本文だけでプレースホルダなし。", &inline_map, &inner);
        assert!(!used_translation);

        let mut writer = Writer::new(Vec::new());
        write_events(&mut writer, &restored)?;
        let restored_text = String::from_utf8(writer.into_inner())?;
        assert!(restored_text.contains("href=\"chapter.xhtml#note\""));
        assert!(restored_text.contains("epub:type=\"noteref\""));
        Ok(())
    }

    #[test]
    fn aside_is_translatable_block() {
        assert!(is_block_tag(QName(b"aside")));
    }

    #[test]
    fn reads_epub3_nav_toc() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let input = dir.path().join("minimal.epub");
        write_minimal_epub(&input)?;
        let book = unpack_epub(&input)?;
        let nav = find_nav_item(&book.manifest).context("missing nav item")?;
        let entries = read_nav_toc(&nav.abs_path)?;
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].label, "Chapter One");
        assert_eq!(entries[0].href.as_deref(), Some("chapter1.xhtml"));
        assert_eq!(entries[1].label, "Chapter Two");
        Ok(())
    }

    #[test]
    fn glossary_subset_is_injected_into_prompt() {
        let entries = vec![
            GlossaryEntry {
                src: "Horizon".to_string(),
                dst: "ホライゾン".to_string(),
                kind: Some("system".to_string()),
                note: None,
            },
            GlossaryEntry {
                src: "Unused".to_string(),
                dst: "未使用".to_string(),
                kind: None,
                note: None,
            },
        ];
        let subset = entries
            .iter()
            .filter(|entry| "Horizon failed.".contains(&entry.src))
            .cloned()
            .collect::<Vec<_>>();
        let prompt = user_prompt("Horizon failed.", &subset);
        assert!(prompt.contains("Horizon => ホライゾン (system)"));
        assert!(!prompt.contains("Unused"));
        assert!(prompt.contains("<source>\nHorizon failed.\n</source>"));
    }

    #[test]
    fn cache_key_changes_with_glossary() {
        let entry_a = GlossaryEntry {
            src: "Horizon".to_string(),
            dst: "ホライゾン".to_string(),
            kind: None,
            note: None,
        };
        let entry_b = GlossaryEntry {
            src: "Horizon".to_string(),
            dst: "ホライズン".to_string(),
            kind: None,
            note: None,
        };
        let key_a = cache_key(
            Provider::Ollama,
            "qwen3:14b",
            "essay",
            "Horizon",
            &[entry_a],
        );
        let key_b = cache_key(
            Provider::Ollama,
            "qwen3:14b",
            "essay",
            "Horizon",
            &[entry_b],
        );
        assert_ne!(key_a, key_b);
    }

    #[test]
    fn cache_store_roundtrips_jsonl() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let cache_root = dir.path().join("cache");
        let input = dir.path().join("book.epub");
        fs::write(&input, b"dummy")?;
        let args = CommonArgs {
            provider: Provider::Ollama,
            model: Some(DEFAULT_MODEL.to_string()),
            ollama_host: DEFAULT_OLLAMA_HOST.to_string(),
            openai_base_url: DEFAULT_OPENAI_BASE_URL.to_string(),
            claude_base_url: DEFAULT_CLAUDE_BASE_URL.to_string(),
            openai_api_key: None,
            anthropic_api_key: None,
            prompt_api_key: false,
            temperature: 0.3,
            num_ctx: 8192,
            timeout_secs: 900,
            retries: 2,
            style: "essay".to_string(),
            glossary: None,
            cache_root: Some(cache_root.clone()),
            no_cache: false,
            clear_cache: false,
            keep_cache: false,
            partial_from_cache: false,
            dry_run: false,
        };
        let mut cache = CacheStore::from_args(&input, &args)?;
        let dir_path = cache.dir.clone();
        let manifest_path = cache.manifest_path.clone();
        let translations_path = cache.translations_path.clone();
        cache.insert(CacheRecord {
            key: "abc".to_string(),
            translated: "訳文".to_string(),
            provider: "ollama".to_string(),
            model: DEFAULT_MODEL.to_string(),
            at: "2026-04-29T00:00:00Z".to_string(),
        })?;
        assert!(dir_path.starts_with(&cache_root));
        assert!(translations_path.exists());

        let mut loaded = CacheStore::from_args(&input, &args)?;
        assert_eq!(loaded.get("abc").as_deref(), Some("訳文"));
        assert_eq!(loaded.stats.hits, 1);

        let params = ManifestParams {
            provider: "ollama".to_string(),
            model: DEFAULT_MODEL.to_string(),
            prompt_version: "v1".to_string(),
            style_id: "essay".to_string(),
            glossary_sha: String::new(),
        };
        loaded.upsert_manifest(&input, params, Some(&input))?;
        assert!(manifest_path.exists());

        loaded.finalize_completion()?;
        assert!(!dir_path.exists());
        Ok(())
    }

    #[test]
    fn partial_from_cache_keeps_cache_misses_original() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let cache_root = dir.path().join("cache");
        let input = dir.path().join("book.epub");
        fs::write(&input, b"dummy")?;
        let args = CommonArgs {
            provider: Provider::Ollama,
            model: Some(DEFAULT_MODEL.to_string()),
            ollama_host: DEFAULT_OLLAMA_HOST.to_string(),
            openai_base_url: DEFAULT_OPENAI_BASE_URL.to_string(),
            claude_base_url: DEFAULT_CLAUDE_BASE_URL.to_string(),
            openai_api_key: None,
            anthropic_api_key: None,
            prompt_api_key: false,
            temperature: 0.3,
            num_ctx: 8192,
            timeout_secs: 900,
            retries: 2,
            style: "essay".to_string(),
            glossary: None,
            cache_root: Some(cache_root),
            no_cache: false,
            clear_cache: false,
            keep_cache: false,
            partial_from_cache: true,
            dry_run: false,
        };
        let cache = CacheStore::from_args(&input, &args)?;
        let mut translator = Translator::new(args, cache)?;

        match translator.translate("Hello")? {
            Translation::Original => {}
            Translation::Translated { text, .. } => bail!("unexpected translation: {text}"),
        }
        assert_eq!(translator.cache.stats.misses, 1);
        assert_eq!(translator.cache.stats.writes, 0);
        Ok(())
    }

    #[test]
    fn cache_dir_uses_input_hash_subdirectory() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let cache_root = dir.path().join("cache");
        let input = dir.path().join("book.epub");
        fs::write(&input, b"sample epub bytes")?;
        let args = CommonArgs {
            provider: Provider::Ollama,
            model: Some(DEFAULT_MODEL.to_string()),
            ollama_host: DEFAULT_OLLAMA_HOST.to_string(),
            openai_base_url: DEFAULT_OPENAI_BASE_URL.to_string(),
            claude_base_url: DEFAULT_CLAUDE_BASE_URL.to_string(),
            openai_api_key: None,
            anthropic_api_key: None,
            prompt_api_key: false,
            temperature: 0.3,
            num_ctx: 8192,
            timeout_secs: 900,
            retries: 2,
            style: "essay".to_string(),
            glossary: None,
            cache_root: Some(cache_root.clone()),
            no_cache: false,
            clear_cache: false,
            keep_cache: false,
            partial_from_cache: false,
            dry_run: false,
        };
        let cache = CacheStore::from_args(&input, &args)?;
        let (_, expected_short) = compute_input_hash(&input)?;
        assert_eq!(cache.input_hash, expected_short);
        assert_eq!(cache.input_hash.len(), 32); // 16 bytes hex
        assert_eq!(cache.dir, cache_root.join(&expected_short));
        Ok(())
    }

    #[test]
    fn finalize_completion_keeps_dir_when_keep_cache_set() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let cache_root = dir.path().join("cache");
        let input = dir.path().join("book.epub");
        fs::write(&input, b"another sample")?;
        let args = CommonArgs {
            provider: Provider::Ollama,
            model: Some(DEFAULT_MODEL.to_string()),
            ollama_host: DEFAULT_OLLAMA_HOST.to_string(),
            openai_base_url: DEFAULT_OPENAI_BASE_URL.to_string(),
            claude_base_url: DEFAULT_CLAUDE_BASE_URL.to_string(),
            openai_api_key: None,
            anthropic_api_key: None,
            prompt_api_key: false,
            temperature: 0.3,
            num_ctx: 8192,
            timeout_secs: 900,
            retries: 2,
            style: "essay".to_string(),
            glossary: None,
            cache_root: Some(cache_root),
            no_cache: false,
            clear_cache: false,
            keep_cache: true,
            partial_from_cache: false,
            dry_run: false,
        };
        let cache = CacheStore::from_args(&input, &args)?;
        let dir_path = cache.dir.clone();
        cache.finalize_completion()?;
        assert!(dir_path.exists());
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
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#,
        )?;
        zip.start_file("OEBPS/content.opf", deflated)?;
        zip.write_all(
            br#"<?xml version="1.0" encoding="UTF-8"?>
<package version="3.0" unique-identifier="bookid" xmlns="http://www.idpf.org/2007/opf">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="bookid">minimal</dc:identifier>
    <dc:title>Minimal</dc:title>
    <dc:language>en</dc:language>
  </metadata>
  <manifest>
    <item id="c1" href="chapter1.xhtml" media-type="application/xhtml+xml"/>
    <item id="c2" href="chapter2.xhtml" media-type="application/xhtml+xml"/>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
  </manifest>
  <spine>
    <itemref idref="c1"/>
    <itemref idref="c2"/>
  </spine>
</package>"#,
        )?;
        zip.start_file("OEBPS/chapter1.xhtml", deflated)?;
        zip.write_all(
            br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><body>
  <h1>Chapter One</h1>
  <p>This is <em>very</em> important.<br /></p>
</body></html>"#,
        )?;
        zip.start_file("OEBPS/chapter2.xhtml", deflated)?;
        zip.write_all(
            br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><body>
  <p>Second page.</p>
</body></html>"#,
        )?;
        zip.start_file("OEBPS/nav.xhtml", deflated)?;
        zip.write_all(
            br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
  <body>
    <nav epub:type="toc">
      <ol>
        <li><a href="chapter1.xhtml">Chapter One</a></li>
        <li><a href="chapter2.xhtml">Chapter Two</a></li>
      </ol>
    </nav>
  </body>
</html>"#,
        )?;
        zip.finish()?;
        Ok(())
    }
}
