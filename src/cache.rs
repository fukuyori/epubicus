use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use crate::{
    config::{CacheArgs, CacheCommand, CommonArgs},
    glossary::GlossaryEntry,
};

const CACHE_SCHEMA_VERSION: u32 = 1;
const MANIFEST_FILE: &str = "manifest.json";
const TRANSLATIONS_FILE: &str = "translations.jsonl";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CacheRecord {
    pub(crate) key: String,
    pub(crate) translated: String,
    pub(crate) provider: String,
    pub(crate) model: String,
    pub(crate) at: String,
}

#[derive(Debug, Default)]
pub(crate) struct CacheStats {
    pub(crate) hits: usize,
    pub(crate) misses: usize,
    pub(crate) writes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Manifest {
    pub(crate) schema_version: u32,
    pub(crate) input: ManifestInput,
    pub(crate) params: ManifestParams,
    pub(crate) timestamps: ManifestTimestamps,
    #[serde(default)]
    pub(crate) last_output_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ManifestInput {
    pub(crate) sha256: String,
    pub(crate) path_when_started: String,
    pub(crate) size_bytes: u64,
    pub(crate) mtime: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ManifestParams {
    pub(crate) provider: String,
    pub(crate) model: String,
    pub(crate) prompt_version: String,
    pub(crate) style_id: String,
    pub(crate) glossary_sha: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ManifestTimestamps {
    pub(crate) started_at: String,
    pub(crate) last_updated_at: String,
}

pub(crate) struct CacheStore {
    pub(crate) enabled: bool,
    /// Per-EPUB cache directory: <cache_root>/<input_hash>/. Always populated, even when enabled=false,
    /// so callers can refer to it for diagnostics.
    pub(crate) dir: PathBuf,
    pub(crate) translations_path: PathBuf,
    pub(crate) manifest_path: PathBuf,
    /// Short hex (first 16 bytes) of the input EPUB hash, used as the directory name.
    #[allow(dead_code)]
    pub(crate) input_hash: String,
    /// Full SHA-256 hex of the input EPUB.
    pub(crate) input_sha256: String,
    pub(crate) entries: HashMap<String, String>,
    pub(crate) stats: CacheStats,
    pub(crate) keep_cache: bool,
}

impl CacheStore {
    pub(crate) fn from_args(input: &Path, args: &CommonArgs) -> Result<Self> {
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

    pub(crate) fn get(&mut self, key: &str) -> Option<String> {
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

    pub(crate) fn peek(&self, key: &str) -> Option<&str> {
        self.enabled
            .then(|| self.entries.get(key).map(String::as_str))
            .flatten()
    }

    pub(crate) fn invalidate(&mut self, key: &str) {
        if self.enabled {
            self.entries.remove(key);
        }
    }

    pub(crate) fn insert(&mut self, record: CacheRecord) -> Result<()> {
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
    pub(crate) fn upsert_manifest(
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
            .and_then(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339().into());
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
    pub(crate) fn finalize_completion(&self) -> Result<()> {
        if !self.enabled || self.keep_cache {
            return Ok(());
        }
        if self.dir.exists() {
            fs::remove_dir_all(&self.dir).with_context(|| {
                format!("failed to remove cache directory {}", self.dir.display())
            })?;
        }
        Ok(())
    }
}

/// Compute (full SHA-256 hex, first-16-byte hex) of the input file.
pub(crate) fn compute_input_hash(input: &Path) -> Result<(String, String)> {
    let file =
        File::open(input).with_context(|| format!("failed to open input {}", input.display()))?;
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
    let data = serde_json::to_vec_pretty(manifest).context("failed to serialize cache manifest")?;
    fs::write(&tmp, &data)
        .with_context(|| format!("failed to write manifest {}", tmp.display()))?;
    fs::rename(&tmp, path)
        .with_context(|| format!("failed to commit manifest {}", path.display()))?;
    Ok(())
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

pub(crate) fn glossary_sha(entries: &[GlossaryEntry]) -> String {
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

pub(crate) fn cache_command(args: CacheArgs) -> Result<()> {
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
    pub(crate) dir: PathBuf,
    manifest: Option<Manifest>,
    cached_segments: usize,
    pub(crate) size_bytes: u64,
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
    println!("Total: {} run(s), {}", entries.len(), human_bytes(total));
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
        let Ok(last) = chrono::DateTime::parse_from_rfc3339(&manifest.timestamps.last_updated_at)
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
        fs::remove_dir_all(&v.dir)
            .with_context(|| format!("failed to remove cache dir {}", v.dir.display()))?;
        freed += v.size_bytes;
    }
    println!(
        "Deleted {} run(s); freed {}.",
        victims.len(),
        human_bytes(freed)
    );
    Ok(())
}

fn cache_clear(root: &Path, hash: Option<&str>, all: bool, yes: bool, dry_run: bool) -> Result<()> {
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
            fs::remove_dir_all(&e.dir)
                .with_context(|| format!("failed to remove cache dir {}", e.dir.display()))?;
        }
        println!(
            "Deleted {} run(s); freed {}.",
            entries.len(),
            human_bytes(total_size)
        );
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
    fs::remove_dir_all(&entry.dir)
        .with_context(|| format!("failed to remove cache dir {}", entry.dir.display()))?;
    println!(
        "Deleted {} ({}).",
        entry.hash,
        human_bytes(entry.size_bytes)
    );
    Ok(())
}
