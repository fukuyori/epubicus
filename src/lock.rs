use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::Duration,
};

use anyhow::{Context, Result, bail};

const LOCK_POLL_INTERVAL: Duration = Duration::from_millis(200);

#[derive(Debug, Default)]
pub(crate) struct LockMetadata {
    pub(crate) pid: Option<u32>,
    pub(crate) hostname: Option<String>,
    pub(crate) purpose: Option<String>,
    pub(crate) created_at: Option<String>,
    pub(crate) heartbeat_at: Option<String>,
}

#[derive(Debug)]
pub(crate) struct FileLock {
    path: PathBuf,
    _file: File,
}

impl FileLock {
    pub(crate) fn acquire(path: &Path, purpose: &str) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create lock dir {}", parent.display()))?;
        }
        let mut announced = false;
        loop {
            match Self::create(path, purpose) {
                Ok(lock) => return Ok(lock),
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                    if !announced {
                        eprintln!("waiting for lock {} ({purpose})", path.display());
                        announced = true;
                    }
                    thread::sleep(LOCK_POLL_INTERVAL);
                }
                Err(err) => {
                    return Err(err)
                        .with_context(|| format!("failed to acquire lock {}", path.display()));
                }
            }
        }
    }

    pub(crate) fn acquire_nowait(path: &Path, purpose: &str) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create lock dir {}", parent.display()))?;
        }
        match Self::create(path, purpose) {
            Ok(lock) => Ok(lock),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                if remove_stale_lock(path)? {
                    return Self::create(path, purpose)
                        .with_context(|| format!("failed to acquire lock {}", path.display()));
                }
                let holder = fs::read_to_string(path)
                    .unwrap_or_else(|_| "(failed to read lock holder metadata)".to_string());
                bail!(
                    "another epubicus process is already using this input; lock: {}\n{}",
                    path.display(),
                    holder.trim()
                );
            }
            Err(err) => {
                Err(err).with_context(|| format!("failed to acquire lock {}", path.display()))
            }
        }
    }

    fn create(path: &Path, purpose: &str) -> std::io::Result<Self> {
        let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
        let now = chrono::Utc::now().to_rfc3339();
        writeln!(
            file,
            "pid={}\nhostname={}\npurpose={purpose}\ncommand={}\ncreated_at={now}\nheartbeat_at={now}",
            std::process::id(),
            current_hostname(),
            std::env::args().collect::<Vec<_>>().join(" ")
        )?;
        file.flush()?;
        Ok(Self {
            path: path.to_path_buf(),
            _file: file,
        })
    }
}

pub(crate) fn read_lock_metadata(path: &Path) -> Result<LockMetadata> {
    parse_lock_metadata(&fs::read_to_string(path)?)
}

pub(crate) fn remove_lock_if_stale(path: &Path) -> Result<bool> {
    remove_stale_lock(path)
}

pub(crate) fn remove_lock_force(path: &Path) -> Result<bool> {
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err).with_context(|| format!("failed to remove lock {}", path.display())),
    }
}

fn remove_stale_lock(path: &Path) -> Result<bool> {
    let metadata = match read_lock_metadata(path) {
        Ok(metadata) => metadata,
        Err(_) => return Ok(false),
    };
    if !is_stale_lock(&metadata) {
        return Ok(false);
    }
    remove_lock_force(path)
}

fn is_stale_lock(metadata: &LockMetadata) -> bool {
    let Some(pid) = metadata.pid else {
        return false;
    };
    let Some(hostname) = metadata.hostname.as_deref() else {
        return false;
    };
    if !hostname.eq_ignore_ascii_case(&current_hostname()) {
        return false;
    }
    !process_is_running(pid)
}

fn parse_lock_metadata(data: &str) -> Result<LockMetadata> {
    let mut metadata = LockMetadata::default();
    for line in data.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key {
            "pid" => metadata.pid = value.parse().ok(),
            "hostname" => metadata.hostname = Some(value.to_string()),
            "purpose" => metadata.purpose = Some(value.to_string()),
            "created_at" => metadata.created_at = Some(value.to_string()),
            "heartbeat_at" => metadata.heartbeat_at = Some(value.to_string()),
            _ => {}
        }
    }
    Ok(metadata)
}

fn current_hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "unknown".to_string())
}

#[cfg(windows)]
fn process_is_running(pid: u32) -> bool {
    let script = format!(
        "if (Get-Process -Id {pid} -ErrorAction SilentlyContinue) {{ exit 0 }} else {{ exit 1 }}"
    );
    let Ok(output) = Command::new("powershell")
        .args(["-NoProfile", "-Command", &script])
        .output()
    else {
        return true;
    };
    output.status.success()
}

#[cfg(not(windows))]
fn process_is_running(pid: u32) -> bool {
    Path::new("/proc").join(pid.to_string()).exists()
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_lock_releases_on_drop() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("test.lock");
        {
            let _lock = FileLock::acquire(&path, "test")?;
            assert!(path.exists());
        }
        assert!(!path.exists());
        let _lock = FileLock::acquire(&path, "test")?;
        assert!(path.exists());
        Ok(())
    }

    #[test]
    fn acquire_nowait_replaces_stale_lock() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("test.lock");
        fs::write(
            &path,
            format!(
                "pid=999999\nhostname={}\npurpose=stale\ncreated_at=old\nheartbeat_at=old\n",
                current_hostname()
            ),
        )?;

        let _lock = FileLock::acquire_nowait(&path, "test")?;
        let data = fs::read_to_string(&path)?;

        assert!(data.contains(&format!("pid={}", std::process::id())));
        assert!(data.contains("purpose=test"));
        Ok(())
    }
}
