use crate::ralph_storage::RalphPaths;
use anyhow::Context;
use serde::Deserialize;
use tokio::time;

#[derive(Debug, Deserialize)]
struct StatusFile {
    mode: Option<String>,
    status: Option<String>,
    in_flight: Option<bool>,
    loop_current: Option<u64>,
    max_loops: Option<u64>,
    loop_count: Option<u64>,
    calls_made_this_hour: Option<u64>,
    max_calls_per_hour: Option<u64>,
    next_reset_in_seconds: Option<u64>,
    last_action: Option<String>,
    exit_reason: Option<String>,
    next_reset: Option<String>,
    timeout_minutes: Option<u64>,
    completion_mode: Option<String>,
    completion_phrases: Option<Vec<String>>,
    completion_regexes: Option<Vec<String>>,
    last_abort_reason: Option<String>,
    timed_out: Option<bool>,
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

async fn read_pid(paths: &RalphPaths) -> Option<u32> {
    let s = tokio::fs::read_to_string(&paths.pid_file).await.ok()?;
    let pid = s.trim().parse::<u32>().ok()?;
    Some(pid)
}

pub(crate) async fn run_monitor(cwd: &std::path::Path, interval_secs: u64) -> anyhow::Result<()> {
    let paths = RalphPaths::new(cwd);
    let interval_secs = interval_secs.max(1);

    let mut ticker = time::interval(time::Duration::from_secs(interval_secs));

    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .context("Failed to install SIGTERM handler")?;

        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    println!("\nralph-widex-monitor stopped.");
                    return Ok(());
                },
                _ = term.recv() => {
                    println!("\nralph-widex-monitor stopped (SIGTERM).");
                    return Ok(());
                },
                _ = ticker.tick() => {
                    render_once(&paths, interval_secs).await?;
                },
            }
        }
    }

    #[cfg(not(unix))]
    {
        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    println!("\nralph-widex-monitor stopped.");
                    return Ok(());
                },
                _ = ticker.tick() => {
                    render_once(&paths, interval_secs).await?;
                },
            }
        }
    }
}

async fn render_once(paths: &RalphPaths, interval_secs: u64) -> anyhow::Result<()> {
    print!("\x1b[H\x1b[2J\x1b[3J");
    println!("ralph-widex-monitor");
    println!("==================");
    println!();

    let pid = read_pid(paths).await;
    let pid_is_running = pid
        .map(crate::ralph_storage::process_is_running)
        .unwrap_or(false);

    if let Ok(status) = read_json::<StatusFile>(&paths.status_file).await {
        let mut status_str = status.status.unwrap_or_else(|| "unknown".to_string());
        if status_str == "running" && pid.is_some() && !pid_is_running {
            status_str = "exited".to_string();
        }
        let loop_current = status.loop_current.or(status.loop_count).unwrap_or(0);
        let max_loops = status.max_loops.unwrap_or(0);
        let max_loops = if max_loops == 0 {
            "inf".to_string()
        } else {
            max_loops.to_string()
        };
        let in_flight = status.in_flight.unwrap_or(false);
        let calls_made = status.calls_made_this_hour.unwrap_or(0);
        let max_calls = status.max_calls_per_hour.unwrap_or(0);
        let max_calls = if max_calls == 0 {
            "unlimited".to_string()
        } else {
            max_calls.to_string()
        };
        let next_reset = status.next_reset.unwrap_or_default();
        let reset_in = status.next_reset_in_seconds.unwrap_or(0);
        let timeout = status.timeout_minutes.unwrap_or(0);
        let timed_out = status.timed_out.unwrap_or(false);

        println!("Status:     {status_str}");
        if let Some(mode) = status.mode.as_deref().filter(|v| !v.is_empty()) {
            println!("Mode:       {mode}");
        }
        println!("Loop:       {loop_current}/{max_loops}");
        println!("In flight:  {in_flight}");
        println!(
            "Calls/hour: {calls_made}/{max_calls} (reset in: {reset_in}s, next reset: {next_reset})"
        );
        if timeout == 0 {
            println!("Timeout:    none");
        } else {
            println!("Timeout:    {timeout}m");
        }
        if let Some(mode) = status.completion_mode.as_deref().filter(|v| !v.is_empty()) {
            let phrases: &[String] = status.completion_phrases.as_deref().unwrap_or(&[]);
            let regexes: &[String] = status.completion_regexes.as_deref().unwrap_or(&[]);
            let completion = if mode == "regex" {
                regexes.join(" | ")
            } else if mode == "promise-tag" {
                phrases.join(" | ")
            } else {
                phrases.join(" | ")
            };
            if !completion.is_empty() {
                println!("Completion: {mode} ({completion})");
            } else {
                println!("Completion: {mode}");
            }
        }
        if let Some(abort) = status
            .last_abort_reason
            .as_deref()
            .filter(|v| !v.is_empty())
        {
            println!("Last abort: {abort} (timed_out={timed_out})");
        } else if timed_out {
            println!("Last abort: timeout (timed_out=true)");
        }
        if let Some(last) = status.last_action.filter(|v| !v.is_empty()) {
            println!("Last:       {last}");
        }
        if let Some(exit_reason) = status.exit_reason.as_deref().filter(|v| !v.is_empty()) {
            println!("Exit:       {exit_reason}");
        }
    } else {
        println!("Status file not found: {}", paths.status_file.display());
        println!("(Is ralph-widex running in this repo?)");
    }

    if let Some(pid) = pid {
        if pid_is_running {
            println!("PID:        {pid}");
        } else {
            println!("PID:        {pid} (stale)");
        }
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
        && (pid.is_none() || pid_is_running)
    {
        let elapsed = progress.elapsed_seconds.unwrap_or(0);
        let last_output = progress.last_output.unwrap_or_default();
        println!();
        println!("Widex exec: running ({elapsed}s elapsed)");
        if !last_output.is_empty() {
            println!("Output:     {last_output}");
        }
    }

    println!();
    println!("Recent activity:");
    println!("----------------");

    let (log_path, contents) = read_log_contents(paths).await;
    match contents {
        Some(contents) => {
            println!("Log:        {}", log_path.display());
            for line in tail_lines(&contents, 12) {
                println!("{line}");
            }
        }
        None => println!("No log file found: {}", log_path.display()),
    }

    println!();
    println!(
        "(refresh every {interval_secs}s)  {}",
        chrono::Local::now().format("%H:%M:%S")
    );

    Ok(())
}

pub(crate) async fn print_status_once(
    cwd: &std::path::Path,
    tail_lines_count: usize,
) -> anyhow::Result<()> {
    let paths = RalphPaths::new(cwd);
    let tail_lines_count = tail_lines_count.max(1);

    println!("ralph-widex-status");
    println!("==================");
    println!();
    println!("CWD: {}", paths.cwd.display());
    println!("RALPH: {}", paths.ralph_dir.display());

    let pid = read_pid(&paths).await;
    let pid_is_running = pid
        .map(crate::ralph_storage::process_is_running)
        .unwrap_or(false);

    if let Ok(status) = read_json::<StatusFile>(&paths.status_file).await {
        let mut status_str = status.status.unwrap_or_else(|| "unknown".to_string());
        if status_str == "running" && pid.is_some() && !pid_is_running {
            status_str = "exited".to_string();
        }
        let loop_current = status.loop_current.or(status.loop_count).unwrap_or(0);
        let max_loops = status.max_loops.unwrap_or(0);
        let max_loops = if max_loops == 0 {
            "inf".to_string()
        } else {
            max_loops.to_string()
        };
        let in_flight = status.in_flight.unwrap_or(false);
        let calls_made = status.calls_made_this_hour.unwrap_or(0);
        let max_calls = status.max_calls_per_hour.unwrap_or(0);
        let max_calls = if max_calls == 0 {
            "unlimited".to_string()
        } else {
            max_calls.to_string()
        };
        let next_reset = status.next_reset.unwrap_or_default();
        let reset_in = status.next_reset_in_seconds.unwrap_or(0);
        let timeout = status.timeout_minutes.unwrap_or(0);
        let timed_out = status.timed_out.unwrap_or(false);

        println!();
        println!("Status:     {status_str}");
        if let Some(mode) = status.mode.as_deref().filter(|v| !v.is_empty()) {
            println!("Mode:       {mode}");
        }
        println!("Loop:       {loop_current}/{max_loops}");
        println!("In flight:  {in_flight}");
        println!(
            "Calls/hour: {calls_made}/{max_calls} (reset in: {reset_in}s, next reset: {next_reset})"
        );
        if timeout == 0 {
            println!("Timeout:    none");
        } else {
            println!("Timeout:    {timeout}m");
        }
        if let Some(mode) = status.completion_mode.as_deref().filter(|v| !v.is_empty()) {
            let phrases: &[String] = status.completion_phrases.as_deref().unwrap_or(&[]);
            let regexes: &[String] = status.completion_regexes.as_deref().unwrap_or(&[]);
            let completion = if mode == "regex" {
                regexes.join(" | ")
            } else if mode == "promise-tag" {
                phrases.join(" | ")
            } else {
                phrases.join(" | ")
            };
            if !completion.is_empty() {
                println!("Completion: {mode} ({completion})");
            } else {
                println!("Completion: {mode}");
            }
        }
        if let Some(abort) = status
            .last_abort_reason
            .as_deref()
            .filter(|v| !v.is_empty())
        {
            println!("Last abort: {abort} (timed_out={timed_out})");
        } else if timed_out {
            println!("Last abort: timeout (timed_out=true)");
        }
        if let Some(last) = status.last_action.filter(|v| !v.is_empty()) {
            println!("Last:       {last}");
        }
        if let Some(exit_reason) = status.exit_reason.as_deref().filter(|v| !v.is_empty()) {
            println!("Exit:       {exit_reason}");
        }
    } else {
        println!();
        println!("Status file not found: {}", paths.status_file.display());
        println!("Run: widex ralph-widex init (or /ralph-widex init in TUI)");
    }

    if let Some(pid) = pid {
        println!();
        if pid_is_running {
            println!("PID:        {pid}");
        } else {
            println!("PID:        {pid} (stale)");
        }
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
        && (pid.is_none() || pid_is_running)
    {
        let elapsed = progress.elapsed_seconds.unwrap_or(0);
        let last_output = progress.last_output.unwrap_or_default();
        println!();
        println!("Widex exec: running ({elapsed}s elapsed)");
        if !last_output.is_empty() {
            println!("Output:     {last_output}");
        }
    }

    println!();
    println!("Recent activity:");
    println!("----------------");

    let (log_path, contents) = read_log_contents(&paths).await;
    match contents {
        Some(contents) => {
            println!("Log:        {}", log_path.display());
            for line in tail_lines(&contents, tail_lines_count) {
                println!("{line}");
            }
        }
        None => println!("No log file found: {}", log_path.display()),
    }

    Ok(())
}

async fn read_log_contents(paths: &RalphPaths) -> (std::path::PathBuf, Option<String>) {
    let primary = paths.logs_dir.join("ralph.log");
    if let Ok(contents) = tokio::fs::read_to_string(&primary).await {
        return (primary, Some(contents));
    }

    // Back-compat: older daemon builds redirected stdout/stderr here.
    let daemon = paths.logs_dir.join("ralph_widex_daemon.log");
    if let Ok(contents) = tokio::fs::read_to_string(&daemon).await {
        return (daemon, Some(contents));
    }

    (primary, None)
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
