use crate::ralph_storage::RalphPaths;
use anyhow::Context;
use serde::Deserialize;
use tokio::time;

#[derive(Debug, Deserialize)]
struct StatusFile {
    status: Option<String>,
    loop_count: Option<u64>,
    calls_made_this_hour: Option<u64>,
    max_calls_per_hour: Option<u64>,
    last_action: Option<String>,
    next_reset: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ProgressFile {
    status: Option<String>,
    elapsed_seconds: Option<u64>,
    last_output: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CircuitBreakerStateFile {
    state: Option<String>,
    reason: Option<String>,
    consecutive_no_progress: Option<u64>,
    consecutive_same_error: Option<u64>,
}

pub(crate) async fn run_monitor(cwd: &std::path::Path, interval_secs: u64) -> anyhow::Result<()> {
    let paths = RalphPaths::new(cwd);
    let interval_secs = interval_secs.max(1);

    let mut ticker = time::interval(time::Duration::from_secs(interval_secs));

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                println!("\nralph-widex-monitor stopped.");
                return Ok(());
            }
            _ = ticker.tick() => {
                render_once(&paths, interval_secs).await?;
            }
        }
    }
}

async fn render_once(paths: &RalphPaths, interval_secs: u64) -> anyhow::Result<()> {
    print!("\x1b[H\x1b[2J\x1b[3J");
    println!("ralph-widex-monitor");
    println!("==================");
    println!();

    if let Ok(status) = read_json::<StatusFile>(&paths.status_file).await {
        let status_str = status.status.unwrap_or_else(|| "unknown".to_string());
        let loop_count = status.loop_count.unwrap_or(0);
        let calls_made = status.calls_made_this_hour.unwrap_or(0);
        let max_calls = status.max_calls_per_hour.unwrap_or(0);
        let next_reset = status.next_reset.unwrap_or_default();

        println!("Status:     {status_str}");
        println!("Loop:       {loop_count}");
        println!("Calls:      {calls_made}/{max_calls} (next reset: {next_reset})");
        if let Some(last) = status.last_action.filter(|v| !v.is_empty()) {
            println!("Last:       {last}");
        }
    } else {
        println!("Status file not found: {}", paths.status_file.display());
        println!("(Is ralph-widex running in this repo?)");
    }

    if let Ok(cb) = read_json::<CircuitBreakerStateFile>(&paths.circuit_breaker_state_file).await {
        let state = cb.state.unwrap_or_else(|| "UNKNOWN".to_string());
        let reason = cb.reason.unwrap_or_default();
        let no_progress = cb.consecutive_no_progress.unwrap_or(0);
        let same_error = cb.consecutive_same_error.unwrap_or(0);
        println!();
        println!("Circuit:    {state} (no_progress={no_progress}, same_error={same_error})");
        if !reason.is_empty() {
            println!("CB reason:  {reason}");
        }
    }

    if tokio::fs::try_exists(&paths.stop_file)
        .await
        .unwrap_or(false)
    {
        println!();
        println!("STOP:       present ({})", paths.stop_file.display());
    }

    if let Ok(progress) = read_json::<ProgressFile>(&paths.progress_file).await
        && progress.status.as_deref() == Some("executing")
    {
        let elapsed = progress.elapsed_seconds.unwrap_or(0);
        let last_output = progress.last_output.unwrap_or_default();
        println!();
        println!("Codex exec: running ({elapsed}s elapsed)");
        if !last_output.is_empty() {
            println!("Output:     {last_output}");
        }
    }

    println!();
    println!("Recent activity:");
    println!("----------------");

    let log_path = paths.logs_dir.join("ralph.log");
    match tokio::fs::read_to_string(&log_path).await {
        Ok(contents) => {
            for line in tail_lines(&contents, 12) {
                println!("{line}");
            }
        }
        Err(_) => {
            println!("No log file found: {}", log_path.display());
        }
    }

    println!();
    println!(
        "(refresh every {interval_secs}s)  {}",
        chrono::Local::now().format("%H:%M:%S")
    );

    Ok(())
}

async fn read_json<T: for<'de> Deserialize<'de>>(path: &std::path::Path) -> anyhow::Result<T> {
    let bytes = tokio::fs::read(path)
        .await
        .with_context(|| format!("Failed to read {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("Failed to parse {}", path.display()))
}

fn tail_lines(contents: &str, n: usize) -> Vec<&str> {
    let lines: Vec<&str> = contents.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].to_vec()
}
