use anyhow::Context;
use clap::Args;
use clap::Parser;
use clap::Subcommand;
use std::path::PathBuf;

mod loop_runner;
mod monitor;
mod ralph_status;
mod ralph_storage;

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
    /// Max calls per hour.
    #[arg(long = "calls", default_value_t = 100)]
    pub max_calls_per_hour: u64,

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

    /// Print extra debug info to stderr.
    #[arg(long = "verbose", default_value_t = false)]
    pub verbose: bool,

    /// Override the Codex binary used for `codex exec`.
    ///
    /// Defaults to $CODEX_CMD if set, otherwise the currently running executable.
    #[arg(long = "codex-cmd")]
    pub codex_cmd: Option<PathBuf>,
}

impl Default for RunArgs {
    fn default() -> Self {
        Self {
            max_calls_per_hour: 100,
            prompt_path: PathBuf::from(".ralph/PROMPT.md"),
            timeout_minutes: 15,
            no_continue: false,
            session_expiry_hours: 24,
            skip_git_repo_check: false,
            no_full_auto: false,
            verbose: false,
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
                max_calls_per_hour: args.max_calls_per_hour,
                timeout_minutes: args.timeout_minutes,
                use_continue: !args.no_continue,
                session_expiry_hours: args.session_expiry_hours,
                skip_git_repo_check: args.skip_git_repo_check,
                full_auto: !args.no_full_auto,
                verbose: args.verbose,
            };

            loop_runner::run_loop(&cwd, opts).await
        }
        Command::Monitor(args) => {
            let cwd = std::env::current_dir().context("Failed to read current directory")?;
            monitor::run_monitor(&cwd, args.interval_secs).await
        }
    }
}
