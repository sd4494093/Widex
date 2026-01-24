use anyhow::Context;
use chrono::Utc;
use serde::Serialize;
use std::ffi::OsStr;
use std::path::Path;
use std::path::PathBuf;
use tokio::fs;
use tokio::io::AsyncWriteExt;

pub(crate) const RALPH_DIR: &str = ".ralph";

#[derive(Debug, Clone)]
pub(crate) struct RalphPaths {
    pub(crate) cwd: PathBuf,
    pub(crate) ralph_dir: PathBuf,
    pub(crate) logs_dir: PathBuf,
    pub(crate) docs_generated_dir: PathBuf,
    pub(crate) status_file: PathBuf,
    pub(crate) progress_file: PathBuf,
    pub(crate) response_analysis_file: PathBuf,
    pub(crate) exit_signals_file: PathBuf,
    pub(crate) circuit_breaker_state_file: PathBuf,
    pub(crate) circuit_breaker_history_file: PathBuf,
    pub(crate) stop_file: PathBuf,
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
            status_file: ralph_dir.join("status.json"),
            progress_file: ralph_dir.join("progress.json"),
            response_analysis_file: ralph_dir.join(".response_analysis"),
            exit_signals_file: ralph_dir.join(".exit_signals"),
            circuit_breaker_state_file: ralph_dir.join(".circuit_breaker_state"),
            circuit_breaker_history_file: ralph_dir.join(".circuit_breaker_history"),
            stop_file: ralph_dir.join("STOP"),
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
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);

    let mut file = opts.open(&paths.lock_file).await.with_context(|| {
        format!(
            "Failed to acquire lock at {} (is ralph already running?)",
            paths.lock_file.display()
        )
    })?;

    let pid = std::process::id();
    let now = Utc::now().to_rfc3339();
    let content = format!("pid={pid}\nstarted_at={now}\n");
    file.write_all(content.as_bytes()).await?;
    file.flush().await?;

    Ok(LockGuard {
        path: paths.lock_file.clone(),
    })
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
