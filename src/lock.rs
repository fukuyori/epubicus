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
    pub(crate) command: Option<String>,
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
                    if remove_stale_lock(path)? {
                        announced = false;
                        continue;
                    }
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
    let Some(process) = process_snapshot(pid) else {
        return true;
    };
    if let Some(created_at) = metadata
        .created_at
        .as_deref()
        .and_then(parse_rfc3339_utc)
    {
        if process.started_at > created_at + chrono::TimeDelta::seconds(2) {
            return true;
        }
    }
    if let Some(command) = metadata.command.as_deref() {
        let command = command.to_ascii_lowercase();
        if command.contains("epubicus")
            && !process.command_line.to_ascii_lowercase().contains("epubicus")
        {
            return true;
        }
    }
    false
}

fn parse_rfc3339_utc(value: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
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
            "command" => metadata.command = Some(value.to_string()),
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
fn process_snapshot(pid: u32) -> Option<ProcessSnapshot> {
    let script = format!(
        "$p = Get-CimInstance Win32_Process -Filter \"ProcessId = {pid}\" -ErrorAction SilentlyContinue; \
if ($null -eq $p) {{ exit 1 }}; \
$start = $null; \
try {{ $start = (Get-Process -Id {pid} -ErrorAction Stop).StartTime.ToUniversalTime().ToString('o') }} catch {{}}; \
[pscustomobject]@{{ start = $start; command = $p.CommandLine }} | ConvertTo-Json -Compress"
    );
    let Ok(output) = Command::new("powershell")
        .args(["-NoProfile", "-Command", &script])
        .output()
    else {
        return process_snapshot_fallback(pid);
    };
    if !output.status.success() {
        return process_snapshot_fallback(pid);
    }
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    let started_at = value["start"]
        .as_str()
        .and_then(parse_rfc3339_utc)
        .unwrap_or_else(chrono::Utc::now);
    let command_line = value["command"].as_str().unwrap_or_default().to_string();
    Some(ProcessSnapshot {
        started_at,
        command_line,
    })
}

#[cfg(windows)]
fn process_snapshot_fallback(pid: u32) -> Option<ProcessSnapshot> {
    let filter = format!("PID eq {pid}");
    let Ok(output) = Command::new("tasklist")
        .args(["/FI", &filter, "/FO", "CSV", "/NH"])
        .output()
    else {
        return None;
    };
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text.lines().next()?.trim();
    if line.is_empty() || line.starts_with("INFO:") {
        return None;
    }
    Some(ProcessSnapshot {
        started_at: chrono::Utc::now(),
        command_line: line.to_string(),
    })
}

#[cfg(not(windows))]
fn process_snapshot(pid: u32) -> Option<ProcessSnapshot> {
    let path = Path::new("/proc").join(pid.to_string());
    path.exists().then(|| ProcessSnapshot {
        started_at: chrono::Utc::now(),
        command_line: String::new(),
    })
}

struct ProcessSnapshot {
    started_at: chrono::DateTime<chrono::Utc>,
    command_line: String,
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
    fn acquire_replaces_stale_lock_before_waiting() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("test.lock");
        fs::write(
            &path,
            format!(
                "pid=999999\nhostname={}\npurpose=stale\ncreated_at=old\nheartbeat_at=old\n",
                current_hostname()
            ),
        )?;

        let _lock = FileLock::acquire(&path, "test")?;
        let data = fs::read_to_string(&path)?;

        assert!(data.contains(&format!("pid={}", std::process::id())));
        assert!(data.contains("purpose=test"));
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
