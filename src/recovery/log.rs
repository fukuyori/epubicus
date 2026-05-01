use std::{
    fs::File,
    io::{BufRead, BufReader, Write},
    path::Path,
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RecoveryRecord {
    pub(crate) kind: String,
    pub(crate) reason: String,
    pub(crate) input_epub: String,
    pub(crate) output_epub: String,
    pub(crate) cache_root: String,
    pub(crate) provider: String,
    pub(crate) model: String,
    pub(crate) style: String,
    pub(crate) page_no: usize,
    pub(crate) block_index: usize,
    pub(crate) href: String,
    pub(crate) cache_key: String,
    pub(crate) source_hash: String,
    pub(crate) source_text: String,
    pub(crate) error: Option<String>,
    pub(crate) suggested_action: String,
    pub(crate) at: String,
}

pub(crate) fn hash_text(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    let digest = hasher.finalize();
    digest[..16].iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) fn read_recovery_records(path: &Path) -> Result<Vec<RecoveryRecord>> {
    let file = File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut out = Vec::new();
    for (idx, line) in reader.lines().enumerate() {
        let line = line.with_context(|| format!("failed to read line {}", idx + 1))?;
        if line.trim().is_empty() {
            continue;
        }
        let record = serde_json::from_str(&line)
            .with_context(|| format!("failed to parse {} line {}", path.display(), idx + 1))?;
        out.push(record);
    }
    Ok(out)
}

pub(crate) fn write_recovery_records(path: &Path, records: &[RecoveryRecord]) -> Result<()> {
    let mut file =
        File::create(path).with_context(|| format!("failed to create {}", path.display()))?;
    for record in records {
        serde_json::to_writer(&mut file, record).context("failed to serialize recovery record")?;
        writeln!(file).context("failed to write recovery record newline")?;
    }
    Ok(())
}
