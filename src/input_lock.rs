use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use sha2::{Digest, Sha256};

use crate::{
    cache::compute_input_hash,
    config::UnlockArgs,
    lock::{FileLock, read_lock_metadata, remove_lock_force, remove_lock_if_stale},
};

const RUN_LOCK_DIR: &str = "epubicus";

pub(crate) fn acquire_input_run_lock(input: &Path, purpose: &str) -> Result<FileLock> {
    let path = input_run_lock_path(input)?;
    FileLock::acquire_nowait(&path, purpose)
}

pub(crate) fn input_run_lock_path(input: &Path) -> Result<PathBuf> {
    let (_, input_hash) = compute_input_hash(input)?;
    let path_hash = hash_input_path(input);
    Ok(std::env::temp_dir()
        .join(RUN_LOCK_DIR)
        .join(".locks")
        .join(format!("{input_hash}.{path_hash}.run.lock")))
}

fn hash_input_path(input: &Path) -> String {
    let normalized = input
        .canonicalize()
        .unwrap_or_else(|_| input.to_path_buf())
        .display()
        .to_string()
        .to_lowercase();
    let mut hasher = Sha256::new();
    hasher.update(normalized.as_bytes());
    let digest = hasher.finalize();
    digest[..8].iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) fn unlock_command(args: UnlockArgs) -> Result<()> {
    let path = input_run_lock_path(&args.input)?;
    if !path.exists() {
        println!("No input-use flag exists: {}", path.display());
        return Ok(());
    }
    let metadata = read_lock_metadata(&path).ok();
    let removed = if args.force {
        remove_lock_force(&path)?
    } else {
        remove_lock_if_stale(&path)?
    };
    if removed {
        println!("Removed input-use flag: {}", path.display());
        return Ok(());
    }
    if let Some(metadata) = metadata {
        bail!(
            "input-use flag is still active; use --force only if you are sure no epubicus process is using this EPUB\nlock: {}\npid={}\nhostname={}\npurpose={}\ncreated_at={}",
            path.display(),
            metadata
                .pid
                .map(|pid| pid.to_string())
                .unwrap_or_else(|| "-".to_string()),
            metadata.hostname.as_deref().unwrap_or("-"),
            metadata.purpose.as_deref().unwrap_or("-"),
            metadata.created_at.as_deref().unwrap_or("-")
        );
    }
    bail!(
        "input-use flag exists but could not be verified as stale; use --force only if you are sure no epubicus process is using this EPUB\nlock: {}",
        path.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn input_run_lock_rejects_same_epub_without_waiting() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let input = dir.path().join("book.epub");
        fs::write(&input, dir.path().display().to_string())?;
        let _first = acquire_input_run_lock(&input, "first")?;

        let err = acquire_input_run_lock(&input, "second").unwrap_err();

        assert!(err.to_string().contains("already using this input"));
        Ok(())
    }
}
