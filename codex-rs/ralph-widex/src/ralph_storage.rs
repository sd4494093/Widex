use anyhow::Context;
use chrono::Utc;
use serde::Serialize;
use std::ffi::OsStr;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use tokio::fs;
use tokio::io::AsyncWriteExt;

pub(crate) const RALPH_DIR: &str = ".ralph";

#[derive(Debug, Clone)]
pub(crate) struct RalphPaths {
    pub(crate) cwd: PathBuf,
    pub(crate) ralph_dir: PathBuf,
    pub(crate) logs_dir: PathBuf,
    pub(crate) docs_generated_dir: PathBuf,
    pub(crate) pid_file: PathBuf,
    pub(crate) status_file: PathBuf,
    pub(crate) progress_file: PathBuf,
    pub(crate) response_analysis_file: PathBuf,
    pub(crate) exit_signals_file: PathBuf,
    pub(crate) circuit_breaker_state_file: PathBuf,
    pub(crate) circuit_breaker_history_file: PathBuf,
    pub(crate) stop_file: PathBuf,
    pub(crate) output_schema_file: PathBuf,
    pub(crate) call_count_file: PathBuf,
    pub(crate) last_reset_file: PathBuf,
    pub(crate) session_file: PathBuf,
    pub(crate) lock_file: PathBuf,
}

impl RalphPaths {
    pub(crate) fn new(cwd: &Path) -> Self {
        let ralph_dir = cwd.join(RALPH_DIR);
        let logs_dir = ralph_dir.join("logs");
        let docs_generated_dir = ralph_dir.join("docs").join("generated");

        Self {
            cwd: cwd.to_path_buf(),
            ralph_dir: ralph_dir.clone(),
            logs_dir,
            docs_generated_dir,
            pid_file: ralph_dir.join("ralph_widex.pid"),
            status_file: ralph_dir.join("status.json"),
            progress_file: ralph_dir.join("progress.json"),
            response_analysis_file: ralph_dir.join(".response_analysis"),
            exit_signals_file: ralph_dir.join(".exit_signals"),
            circuit_breaker_state_file: ralph_dir.join(".circuit_breaker_state"),
            circuit_breaker_history_file: ralph_dir.join(".circuit_breaker_history"),
            stop_file: ralph_dir.join("STOP"),
            output_schema_file: ralph_dir.join("ralph_output_schema.json"),
            call_count_file: ralph_dir.join(".call_count"),
            last_reset_file: ralph_dir.join(".last_reset"),
            session_file: ralph_dir.join(".widex_session.json"),
            lock_file: ralph_dir.join(".lock"),
        }
    }

    pub(crate) async fn ensure_dirs(&self) -> anyhow::Result<()> {
        fs::create_dir_all(&self.logs_dir)
            .await
            .with_context(|| format!("Failed to create {}", self.logs_dir.display()))?;
        fs::create_dir_all(&self.docs_generated_dir)
            .await
            .with_context(|| format!("Failed to create {}", self.docs_generated_dir.display()))?;
        Ok(())
    }
}

pub(crate) struct LockGuard {
    path: PathBuf,
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub(crate) async fn acquire_lock(paths: &RalphPaths) -> anyhow::Result<LockGuard> {
    fs::create_dir_all(&paths.ralph_dir).await?;
    let stale_cleanup_attempted = AtomicBool::new(false);

    loop {
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create_new(true);

        match opts.open(&paths.lock_file).await {
            Ok(mut file) => {
                let pid = std::process::id();
                let now = Utc::now().to_rfc3339();
                let content = format!("pid={pid}\nstarted_at={now}\n");
                file.write_all(content.as_bytes()).await?;
                file.flush().await?;

                return Ok(LockGuard {
                    path: paths.lock_file.clone(),
                });
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                // If the previous process crashed, the lock file can be left behind.
                // Try a single stale-lock cleanup pass to avoid trapping the user.
                if stale_cleanup_attempted.swap(true, Ordering::SeqCst) {
                    return Err(err).with_context(|| {
                        format!(
                            "Failed to acquire lock at {} (is ralph already running?)",
                            paths.lock_file.display()
                        )
                    });
                }

                if let Some(pid) = read_lock_pid(&paths.lock_file).await?
                    && !process_is_running(pid)
                {
                    let _ = fs::remove_file(&paths.lock_file).await;
                    continue;
                }

                return Err(err).with_context(|| {
                    format!(
                        "Failed to acquire lock at {} (is ralph already running?)",
                        paths.lock_file.display()
                    )
                });
            }
            Err(err) => {
                return Err(err).with_context(|| {
                    format!("Failed to acquire lock at {}", paths.lock_file.display())
                });
            }
        }
    }
}

pub(crate) async fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }

    let tmp_path = tmp_path_for(path);
    let data = serde_json::to_vec_pretty(value).context("Failed to serialize JSON")?;

    {
        let mut file = fs::File::create(&tmp_path)
            .await
            .with_context(|| format!("Failed to create {}", tmp_path.display()))?;
        file.write_all(&data).await?;
        file.write_all(b"\n").await?;
        file.flush().await?;
    }

    fs::rename(&tmp_path, path).await.with_context(|| {
        format!(
            "Failed to move {} to {}",
            tmp_path.display(),
            path.display()
        )
    })?;

    Ok(())
}

pub(crate) async fn remove_file_if_exists(path: &Path) -> anyhow::Result<()> {
    match fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).with_context(|| format!("Failed to remove {}", path.display())),
    }
}

pub(crate) async fn append_log_line(
    paths: &RalphPaths,
    level: &str,
    message: &str,
) -> anyhow::Result<()> {
    let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let line = format!("[{ts}] [{level}] {message}\n");
    let log_path = paths.logs_dir.join("ralph.log");

    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .await
        .with_context(|| format!("Failed to open {}", log_path.display()))?;

    file.write_all(line.as_bytes()).await?;
    file.flush().await?;
    Ok(())
}

fn tmp_path_for(path: &Path) -> PathBuf {
    let mut p = path.to_path_buf();

    // Keep the extension stable for editor/tooling friendliness.
    let ext = path.extension().unwrap_or_else(|| OsStr::new(""));
    let stem = path
        .file_stem()
        .unwrap_or_else(|| OsStr::new("tmp"))
        .to_string_lossy();

    let pid = std::process::id();
    let new_name = if ext.is_empty() {
        format!("{stem}.tmp.{pid}")
    } else {
        format!("{stem}.tmp.{pid}.{}", ext.to_string_lossy())
    };

    p.set_file_name(new_name);
    p
}

async fn read_lock_pid(path: &Path) -> anyhow::Result<Option<u32>> {
    let contents = match fs::read_to_string(path).await {
        Ok(v) => v,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).with_context(|| format!("Failed to read {}", path.display())),
    };

    for line in contents.lines() {
        if let Some(v) = line.strip_prefix("pid=")
            && let Ok(pid) = v.trim().parse::<u32>()
        {
            return Ok(Some(pid));
        }
    }

    Ok(None)
}

#[cfg(unix)]
pub(crate) fn process_is_running(pid: u32) -> bool {
    unsafe {
        // SAFETY: `kill(pid, 0)` does not actually send a signal; it checks process existence.
        // We treat EPERM as "running but not permitted".
        if libc::kill(pid as i32, 0) == 0 {
            return true;
        }
        !matches!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(code) if code == libc::ESRCH
        )
    }
}

#[cfg(not(unix))]
pub(crate) fn process_is_running(_pid: u32) -> bool {
    // Best-effort only; on non-Unix platforms we avoid adding extra dependencies.
    true
}
