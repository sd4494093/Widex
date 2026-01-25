use anyhow::Context;
use clap::Args;
use clap::Parser;
use clap::Subcommand;
use std::io;
use std::path::PathBuf;
use std::process::Stdio;

mod circuit_breaker;
mod loop_runner;
mod monitor;
mod ralph_status;
mod ralph_storage;
mod response_analysis;

#[derive(Debug, Parser)]
#[command(version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Initialize a .ralph/ folder in the current directory.
    Init(InitArgs),

    /// Run the autonomous loop.
    Run(RunArgs),

    /// Start the autonomous loop in the background (detached).
    Start(StartArgs),

    /// Request the running loop to stop (creates STOP and optionally sends SIGTERM).
    Stop(StopArgs),

    /// Print a one-shot status snapshot (like monitor, but exits immediately).
    Status(StatusArgs),

    /// Monitor .ralph/status.json and recent logs.
    Monitor(MonitorArgs),
}

#[derive(Debug, Args)]
pub struct InitArgs {
    /// Overwrite existing files under .ralph/ (default: false).
    #[arg(long = "overwrite", default_value_t = false)]
    pub overwrite: bool,
}

#[derive(Debug, Args, Clone)]
pub struct RunArgs {
    /// Max number of loops to run before exiting (unless completion phrase is seen).
    #[arg(long = "loops", default_value_t = 20)]
    pub loops: u64,

    /// Max calls per hour.
    #[arg(long = "calls", default_value_t = 100)]
    pub max_calls_per_hour: u64,

    /// A phrase that indicates completion. If the final message contains any of these, the loop
    /// exits early (status=completed). Repeatable.
    #[arg(long = "completion-phrase")]
    pub completion_phrases: Vec<String>,

    /// Enable the circuit breaker (off by default). When enabled, repeated no-progress or repeated
    /// same-error loops can stop execution early.
    #[arg(long = "enable-circuit-breaker", default_value_t = false)]
    pub enable_circuit_breaker: bool,

    /// Prompt file.
    #[arg(long = "prompt", default_value = ".ralph/PROMPT.md")]
    pub prompt_path: PathBuf,

    /// Timeout per `codex exec` call.
    #[arg(long = "timeout-minutes", default_value_t = 15)]
    pub timeout_minutes: u64,

    /// Disable session continuity.
    #[arg(long = "no-continue", default_value_t = false)]
    pub no_continue: bool,

    /// Session expiry (hours).
    #[arg(long = "session-expiry-hours", default_value_t = 24)]
    pub session_expiry_hours: u64,

    /// Allow running outside a Git repo.
    #[arg(long = "skip-git-repo-check", default_value_t = false)]
    pub skip_git_repo_check: bool,

    /// Do not pass --full-auto to `codex exec`.
    #[arg(long = "no-full-auto", default_value_t = false)]
    pub no_full_auto: bool,

    /// Pass `--dangerously-bypass-approvals-and-sandbox` to the child `widex exec`.
    ///
    /// This avoids interactive approval prompts that can cause the exec process to hang until
    /// `--timeout-minutes` is hit.
    #[arg(long = "exec-bypass-approvals-and-sandbox", default_value_t = false)]
    pub exec_bypass_approvals_and_sandbox: bool,

    /// Print extra debug info to stderr.
    #[arg(long = "verbose", default_value_t = false)]
    pub verbose: bool,

    /// Disable `--output-schema` for `codex exec`.
    ///
    /// By default, ralph-widex passes a JSON Schema so the agent can emit a structured
    /// Ralph status JSON output (more reliable exit detection).
    #[arg(long = "no-output-schema", default_value_t = false)]
    pub no_output_schema: bool,

    /// Disable MCP startup/bridging for the child `widex exec` process.
    ///
    /// This is useful when one of the configured MCP servers writes non-JSON output
    /// to stdout, which can break JSON-RPC framing and cause rmcp serde errors.
    #[arg(long = "disable-mcp", default_value_t = false)]
    pub disable_mcp: bool,

    /// Retry `codex exec` if it exits successfully but produces no final agent message.
    ///
    /// Some providers/edge-cases can yield a "no last agent message" warning even with
    /// `--output-schema` enabled. Retrying once is usually enough to recover.
    #[arg(long = "retry-no-final-message", default_value_t = 1)]
    pub retry_no_final_message: u8,

    /// Additional config overrides forwarded to the child `widex exec` invocation.
    ///
    /// This is useful when you want the ralph loop to run with a different provider/model or
    /// networking settings without editing `${CODEX_HOME}/config.toml`.
    ///
    /// Example:
    /// `widex ralph-widex run --exec-config 'model=\"gpt-5.2\"' --exec-config 'model_providers.openai.base_url=\"...\"'`
    #[arg(long = "exec-config")]
    pub exec_config_overrides: Vec<String>,

    /// Enable feature flags for the child `widex exec` invocation (repeatable).
    #[arg(long = "exec-enable")]
    pub exec_enable_features: Vec<String>,

    /// Disable feature flags for the child `widex exec` invocation (repeatable).
    #[arg(long = "exec-disable")]
    pub exec_disable_features: Vec<String>,

    /// Override the model used by the child `widex exec` invocation.
    #[arg(long = "exec-model")]
    pub exec_model: Option<String>,

    /// Override the Codex binary used for `codex exec`.
    ///
    /// Defaults to $CODEX_CMD if set, otherwise the currently running executable.
    #[arg(long = "codex-cmd")]
    pub codex_cmd: Option<PathBuf>,
}

impl Default for RunArgs {
    fn default() -> Self {
        Self {
            loops: 20,
            max_calls_per_hour: 100,
            completion_phrases: Vec::new(),
            enable_circuit_breaker: false,
            prompt_path: PathBuf::from(".ralph/PROMPT.md"),
            timeout_minutes: 15,
            no_continue: false,
            session_expiry_hours: 24,
            skip_git_repo_check: false,
            no_full_auto: false,
            exec_bypass_approvals_and_sandbox: false,
            verbose: false,
            no_output_schema: false,
            disable_mcp: false,
            retry_no_final_message: 1,
            exec_config_overrides: Vec::new(),
            exec_enable_features: Vec::new(),
            exec_disable_features: Vec::new(),
            exec_model: None,
            codex_cmd: None,
        }
    }
}

#[derive(Debug, Args)]
pub struct MonitorArgs {
    /// Refresh interval in seconds.
    #[arg(long = "interval", default_value_t = 2)]
    pub interval_secs: u64,
}

#[derive(Debug, Args)]
pub struct StatusArgs {
    /// Number of log lines to show from `.ralph/logs/ralph.log`.
    #[arg(long = "tail", default_value_t = 12)]
    pub tail_lines: usize,
}

#[derive(Debug, Args, Clone)]
pub struct StartArgs {
    #[command(flatten)]
    pub run: RunArgs,

    /// Do not detach (useful for debugging start behavior).
    #[arg(long = "no-detach", default_value_t = false)]
    pub no_detach: bool,
}

#[derive(Debug, Args, Clone)]
pub struct StopArgs {
    /// Do not send SIGTERM; only create STOP.
    #[arg(long = "no-sigterm", default_value_t = false)]
    pub no_sigterm: bool,

    /// Wait this many seconds for the process to exit after signaling.
    #[arg(long = "wait-seconds", default_value_t = 5)]
    pub wait_seconds: u64,

    /// If the process does not exit after the wait, send SIGKILL (Unix only).
    #[arg(long = "force", default_value_t = false)]
    pub force: bool,
}

pub async fn run_main(cli: Cli, default_codex_cmd: PathBuf) -> anyhow::Result<()> {
    match cli
        .command
        .unwrap_or_else(|| Command::Run(RunArgs::default()))
    {
        Command::Init(args) => {
            let cwd = std::env::current_dir().context("Failed to read current directory")?;
            loop_runner::init_in_place(&cwd, !args.overwrite).await?;
            Ok(())
        }
        Command::Run(args) => {
            let cwd = std::env::current_dir().context("Failed to read current directory")?;
            let codex_cmd = args
                .codex_cmd
                .or_else(|| std::env::var_os("CODEX_CMD").map(PathBuf::from))
                .unwrap_or(default_codex_cmd);

            let opts = loop_runner::RunOptions {
                codex_cmd,
                prompt_path: args.prompt_path,
                max_loops: args.loops,
                max_calls_per_hour: args.max_calls_per_hour,
                timeout_minutes: args.timeout_minutes,
                use_continue: !args.no_continue,
                session_expiry_hours: args.session_expiry_hours,
                skip_git_repo_check: args.skip_git_repo_check,
                full_auto: !args.no_full_auto,
                bypass_approvals_and_sandbox: args.exec_bypass_approvals_and_sandbox,
                verbose: args.verbose,
                use_output_schema: !args.no_output_schema,
                disable_mcp: args.disable_mcp,
                retry_no_final_message: args.retry_no_final_message,
                enable_circuit_breaker: args.enable_circuit_breaker,
                completion_phrases: args.completion_phrases,
                exec_config_overrides: args.exec_config_overrides,
                exec_enable_features: args.exec_enable_features,
                exec_disable_features: args.exec_disable_features,
                exec_model: args.exec_model,
            };

            loop_runner::run_loop(&cwd, opts).await
        }
        Command::Start(args) => {
            let cwd = std::env::current_dir().context("Failed to read current directory")?;
            start_background(&cwd, &default_codex_cmd, args).await
        }
        Command::Stop(args) => {
            let cwd = std::env::current_dir().context("Failed to read current directory")?;
            stop_background(&cwd, args).await
        }
        Command::Status(args) => {
            let cwd = std::env::current_dir().context("Failed to read current directory")?;
            monitor::print_status_once(&cwd, args.tail_lines).await
        }
        Command::Monitor(args) => {
            let cwd = std::env::current_dir().context("Failed to read current directory")?;
            monitor::run_monitor(&cwd, args.interval_secs).await
        }
    }
}

pub(crate) fn widex_cmd_hint() -> &'static str {
    // Widex fork: user-facing hints should always prefer `widex`.
    "widex"
}

async fn start_background(
    cwd: &std::path::Path,
    cmd: &PathBuf,
    args: StartArgs,
) -> anyhow::Result<()> {
    let paths = crate::ralph_storage::RalphPaths::new(cwd);
    paths.ensure_dirs().await?;

    // Make sure a previous STOP doesn't immediately kill the new run.
    let _ = tokio::fs::remove_file(&paths.stop_file).await;

    if let Ok(pid) = read_pid_file(&paths).await {
        if crate::ralph_storage::process_is_running(pid) {
            anyhow::bail!("ralph-widex already running (pid={pid})");
        }
        // Stale PID file (e.g., killed process); allow restart.
        let _ = tokio::fs::remove_file(&paths.pid_file).await;
    }

    // Spawn: `widex ralph-widex run ...`
    let mut child = std::process::Command::new(cmd);
    child.current_dir(cwd);
    child.arg("ralph-widex");
    child.arg("run");
    apply_run_args(&mut child, &args.run);

    // Best-effort: redirect output somewhere stable.
    let log_path = paths.logs_dir.join("ralph_widex_daemon.log");
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|err| {
            io::Error::new(
                err.kind(),
                format!("Failed to open {}: {err}", log_path.display()),
            )
        })?;
    let log2 = log.try_clone()?;
    child.stdin(Stdio::null());
    child.stdout(Stdio::from(log));
    child.stderr(Stdio::from(log2));

    if !args.no_detach {
        detach_unix(&mut child);
    }

    let spawned = child
        .spawn()
        .with_context(|| format!("Failed to spawn {}", cmd.display()))?;
    let pid = spawned.id();

    // Write immediately for observability. The run loop also writes (and deletes on normal exit).
    tokio::fs::write(&paths.pid_file, format!("{pid}\n")).await?;
    println!(
        "Started ralph-widex in background (pid={pid}). Log: {}",
        log_path.display()
    );

    Ok(())
}

async fn stop_background(cwd: &std::path::Path, args: StopArgs) -> anyhow::Result<()> {
    let paths = crate::ralph_storage::RalphPaths::new(cwd);
    paths.ensure_dirs().await?;

    // The primary stop signal is the STOP file.
    tokio::fs::write(&paths.stop_file, b"").await?;

    let Some(pid) = read_pid_file(&paths).await.ok() else {
        println!("Stop requested (STOP file created).");
        return Ok(());
    };

    if !crate::ralph_storage::process_is_running(pid) {
        let _ = tokio::fs::remove_file(&paths.pid_file).await;
        println!("Stop requested (pid={pid}; already stopped).");
        return Ok(());
    }

    #[cfg(unix)]
    if !args.no_sigterm {
        unsafe {
            libc::kill(pid as i32, libc::SIGTERM);
        }
    }

    // Wait for the process to exit. If it doesn't, optionally force-kill.
    let wait = args.wait_seconds.max(1);
    for _ in 0..wait {
        if !crate::ralph_storage::process_is_running(pid) {
            println!("Stop requested (pid={pid}).");
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }

    #[cfg(unix)]
    if args.force {
        unsafe {
            libc::kill(pid as i32, libc::SIGKILL);
        }
        let _ = tokio::fs::remove_file(&paths.pid_file).await;
        println!("Force-stopped ralph-widex (pid={pid}).");
        return Ok(());
    }

    println!("Stop requested (pid={pid}) but process still running.");
    Ok(())
}

fn apply_run_args(cmd: &mut std::process::Command, args: &RunArgs) {
    cmd.arg("--calls").arg(args.max_calls_per_hour.to_string());
    cmd.arg("--prompt").arg(args.prompt_path.as_os_str());
    cmd.arg("--timeout-minutes")
        .arg(args.timeout_minutes.to_string());
    if args.no_continue {
        cmd.arg("--no-continue");
    }
    cmd.arg("--session-expiry-hours")
        .arg(args.session_expiry_hours.to_string());
    if args.skip_git_repo_check {
        cmd.arg("--skip-git-repo-check");
    }
    if args.no_full_auto {
        cmd.arg("--no-full-auto");
    }
    if args.exec_bypass_approvals_and_sandbox {
        cmd.arg("--exec-bypass-approvals-and-sandbox");
    }
    if args.verbose {
        cmd.arg("--verbose");
    }
    if args.no_output_schema {
        cmd.arg("--no-output-schema");
    }
    if args.disable_mcp {
        cmd.arg("--disable-mcp");
    }
    cmd.arg("--retry-no-final-message")
        .arg(args.retry_no_final_message.to_string());
    for kv in &args.exec_config_overrides {
        cmd.arg("--exec-config").arg(kv);
    }
    for feature in &args.exec_enable_features {
        cmd.arg("--exec-enable").arg(feature);
    }
    for feature in &args.exec_disable_features {
        cmd.arg("--exec-disable").arg(feature);
    }
    if let Some(model) = args.exec_model.as_deref() {
        cmd.arg("--exec-model").arg(model);
    }
    if let Some(codex_cmd) = args.codex_cmd.as_ref() {
        cmd.arg("--codex-cmd").arg(codex_cmd.as_os_str());
    }
}

#[cfg(unix)]
fn detach_unix(cmd: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;

    unsafe {
        cmd.pre_exec(|| {
            // Detach from the controlling terminal/session.
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(not(unix))]
fn detach_unix(_cmd: &mut std::process::Command) {}

async fn read_pid_file(paths: &crate::ralph_storage::RalphPaths) -> anyhow::Result<u32> {
    let s = tokio::fs::read_to_string(&paths.pid_file)
        .await
        .with_context(|| format!("Failed to read {}", paths.pid_file.display()))?;
    let pid = s.trim().parse::<u32>()?;
    Ok(pid)
}
