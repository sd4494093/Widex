use crate::circuit_breaker;
use crate::ralph_status::LoopCompletionStatus;
use crate::ralph_status::RalphStatus;
use crate::ralph_status::parse_ralph_status_from_text;
use crate::ralph_storage::RALPH_DIR;
use crate::ralph_storage::RalphPaths;
use crate::ralph_storage::acquire_lock;
use crate::ralph_storage::append_log_line;
use crate::ralph_storage::remove_file_if_exists;
use crate::ralph_storage::write_json_atomic;
use crate::response_analysis::Analysis;
use crate::response_analysis::AnalysisFile;
use crate::response_analysis::ExitSignalsFile;
use crate::response_analysis::analyze_last_message;
use crate::widex_cmd_hint;
use anyhow::Context;
use chrono::Datelike;
use chrono::Local;
use chrono::Timelike;
use codex_exec::exec_events::PatchApplyStatus;
use codex_exec::exec_events::ThreadEvent;
use codex_exec::exec_events::ThreadItemDetails;
use serde_json::json;
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicU8;
use std::sync::atomic::Ordering;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncWriteExt;
use tokio::io::BufReader;
use tokio::process;
use tokio::time;
use tokio_util::sync::CancellationToken;

const TEMPLATE_PROMPT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../widex-custom/features/ralph-widex/templates/PROMPT.md"
));
const TEMPLATE_FIX_PLAN: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../widex-custom/features/ralph-widex/templates/fix_plan.md"
));
const TEMPLATE_AGENT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../widex-custom/features/ralph-widex/templates/AGENT.md"
));
const TEMPLATE_SPECS_GITKEEP: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../widex-custom/features/ralph-widex/templates/specs/.gitkeep"
));

#[derive(Debug, Clone)]
pub(crate) struct RunOptions {
    pub(crate) codex_cmd: PathBuf,
    pub(crate) prompt_path: PathBuf,
    pub(crate) max_calls_per_hour: u64,
    pub(crate) timeout_minutes: u64,
    pub(crate) use_continue: bool,
    pub(crate) session_expiry_hours: u64,
    pub(crate) skip_git_repo_check: bool,
    pub(crate) full_auto: bool,
    pub(crate) verbose: bool,
    pub(crate) use_output_schema: bool,
    pub(crate) disable_mcp: bool,
    pub(crate) retry_no_final_message: u8,
    pub(crate) exec_config_overrides: Vec<String>,
    pub(crate) exec_enable_features: Vec<String>,
    pub(crate) exec_disable_features: Vec<String>,
    pub(crate) exec_model: Option<String>,
}

#[derive(Debug, serde::Serialize)]
struct StatusFile {
    timestamp: String,
    loop_count: u64,
    calls_made_this_hour: u64,
    max_calls_per_hour: u64,
    last_action: String,
    status: String,
    exit_reason: String,
    next_reset: String,
}

#[derive(Debug, serde::Serialize)]
struct ProgressFile {
    status: String,
    elapsed_seconds: u64,
    last_output: String,
}

#[derive(Debug, serde::Serialize, serde::Deserialize, Default)]
struct SessionFile {
    session_id: String,
    created_at: String,
    last_used: String,
    reset_reason: String,
    created_at_epoch: i64,
    last_used_epoch: i64,
    expires_at_epoch: i64,
}

pub(crate) async fn init_in_place(cwd: &Path, no_overwrite: bool) -> anyhow::Result<()> {
    let ralph_dir = cwd.join(RALPH_DIR);

    tokio::fs::create_dir_all(ralph_dir.join("specs").join("stdlib")).await?;
    tokio::fs::create_dir_all(ralph_dir.join("examples")).await?;
    tokio::fs::create_dir_all(ralph_dir.join("logs")).await?;
    tokio::fs::create_dir_all(ralph_dir.join("docs").join("generated")).await?;
    tokio::fs::create_dir_all(cwd.join("src")).await?;

    let prompt_path = ralph_dir.join("PROMPT.md");
    if no_overwrite && tokio::fs::try_exists(&prompt_path).await? {
        anyhow::bail!(
            "{} already exists; refusing to overwrite.",
            prompt_path.display()
        );
    }

    tokio::fs::write(&prompt_path, TEMPLATE_PROMPT).await?;
    tokio::fs::write(ralph_dir.join("@fix_plan.md"), TEMPLATE_FIX_PLAN).await?;
    tokio::fs::write(ralph_dir.join("@AGENT.md"), TEMPLATE_AGENT).await?;

    let specs_dir = ralph_dir.join("specs");
    tokio::fs::create_dir_all(&specs_dir).await?;
    tokio::fs::write(specs_dir.join(".gitkeep"), TEMPLATE_SPECS_GITKEEP).await?;

    println!("Initialized {RALPH_DIR}/ in {}", cwd.display());
    println!(
        "Next: edit {RALPH_DIR}/PROMPT.md and {RALPH_DIR}/@fix_plan.md then run: {} ralph-widex run",
        widex_cmd_hint()
    );

    Ok(())
}

pub(crate) async fn run_loop(cwd: &Path, opts: RunOptions) -> anyhow::Result<()> {
    let paths = RalphPaths::new(cwd);
    paths.ensure_dirs().await?;

    let _lock = acquire_lock(&paths).await?;
    let _pid_guard = PidGuard::write(&paths).await?;
    circuit_breaker::ensure_initialized(&paths).await?;
    if opts.use_output_schema {
        ensure_output_schema_file(&paths).await?;
    }

    let shutdown = Shutdown::new();
    tokio::spawn(listen_for_shutdown(shutdown.clone()));

    if !tokio::fs::try_exists(&opts.prompt_path).await? {
        let cmd = widex_cmd_hint();
        anyhow::bail!(
            "Prompt file not found: {} (run `{cmd} ralph-widex init` first)",
            opts.prompt_path.display(),
        );
    }

    init_call_tracking(&paths).await?;
    append_log_line(
        &paths,
        "INFO",
        &format!("Starting ralph loop in {}", paths.cwd.display()),
    )
    .await?;

    let mut loop_count: u64 = 0;
    loop {
        loop_count += 1;
        init_call_tracking(&paths).await?;

        if shutdown.is_cancelled() {
            let reason = shutdown.reason();
            update_status(
                &paths,
                loop_count,
                read_calls_made(&paths).await?,
                opts.max_calls_per_hour,
                "shutdown",
                "exited",
                &format!("Interrupted: {reason:?}"),
            )
            .await?;
            append_log_line(
                &paths,
                "WARN",
                &format!("Stopping loop due to shutdown: {reason:?}"),
            )
            .await?;
            return Ok(());
        }

        if tokio::fs::try_exists(&paths.stop_file).await? {
            shutdown.trigger(ShutdownReason::StopFile);
            update_status(
                &paths,
                loop_count,
                read_calls_made(&paths).await?,
                opts.max_calls_per_hour,
                "stop_file",
                "exited",
                "STOP file present",
            )
            .await?;
            append_log_line(&paths, "WARN", "STOP file present; stopping loop.").await?;
            return Ok(());
        }

        if !circuit_breaker::can_execute(&paths).await? {
            let state = circuit_breaker::read_state(&paths).await?;
            update_status(
                &paths,
                loop_count,
                read_calls_made(&paths).await?,
                opts.max_calls_per_hour,
                "circuit_breaker",
                "exited",
                &state.reason,
            )
            .await?;
            append_log_line(
                &paths,
                "ERROR",
                &format!("Circuit breaker open: {}", state.reason),
            )
            .await?;
            anyhow::bail!("Circuit breaker open: {}", state.reason);
        }

        let calls_made = read_calls_made(&paths).await?;
        if calls_made >= opts.max_calls_per_hour {
            update_status(
                &paths,
                loop_count,
                calls_made,
                opts.max_calls_per_hour,
                "rate_limited",
                "waiting",
                "",
            )
            .await?;
            append_log_line(
                &paths,
                "WARN",
                &format!(
                    "Rate limit reached ({calls_made}/{max}). Waiting for reset...",
                    max = opts.max_calls_per_hour
                ),
            )
            .await?;
            wait_for_reset(&paths, &shutdown).await?;
            if shutdown.is_cancelled() {
                let reason = shutdown.reason();
                update_status(
                    &paths,
                    loop_count,
                    read_calls_made(&paths).await?,
                    opts.max_calls_per_hour,
                    "shutdown",
                    "exited",
                    &format!("Interrupted: {reason:?}"),
                )
                .await?;
                append_log_line(
                    &paths,
                    "WARN",
                    &format!("Stopping loop due to shutdown: {reason:?}"),
                )
                .await?;
                return Ok(());
            }
            continue;
        }

        let mut calls_made = increment_calls_made(&paths).await?;
        update_status(
            &paths,
            loop_count,
            calls_made,
            opts.max_calls_per_hour,
            "codex_exec",
            "running",
            "",
        )
        .await?;
        append_log_line(
            &paths,
            "LOOP",
            &format!(
                "Loop #{loop_count} (call {calls_made}/{max})",
                max = opts.max_calls_per_hour
            ),
        )
        .await?;

        let max_attempts = opts.retry_no_final_message.saturating_add(1);
        let mut attempt: u8 = 0;
        let exec = loop {
            attempt = attempt.saturating_add(1);
            let exec = match codex_exec_once(&paths, loop_count, &opts, &shutdown).await {
                Ok(exec) => exec,
                Err(err) => {
                    update_status(
                        &paths,
                        loop_count,
                        calls_made,
                        opts.max_calls_per_hour,
                        "codex_exec",
                        "exited",
                        &format!("codex exec failed: {err:#}"),
                    )
                    .await?;
                    append_log_line(&paths, "ERROR", &format!("codex exec failed: {err:#}"))
                        .await?;
                    return Err(err);
                }
            };

            let should_retry =
                !exec.interrupted && exec.exit_code == 0 && exec.last_message.is_none();
            if should_retry && attempt < max_attempts {
                append_log_line(
                    &paths,
                    "WARN",
                    &format!(
                        "codex exec produced no final message; retrying ({attempt}/{max_attempts})",
                    ),
                )
                .await?;

                // Each retry is another `codex exec` invocation, so it should count toward the
                // per-hour call cap.
                let current_calls = read_calls_made(&paths).await?;
                if current_calls >= opts.max_calls_per_hour {
                    update_status(
                        &paths,
                        loop_count,
                        current_calls,
                        opts.max_calls_per_hour,
                        "rate_limited",
                        "waiting",
                        "",
                    )
                    .await?;
                    append_log_line(
                        &paths,
                        "WARN",
                        &format!(
                            "Rate limit reached ({current_calls}/{max}). Waiting for reset...",
                            max = opts.max_calls_per_hour
                        ),
                    )
                    .await?;
                    wait_for_reset(&paths, &shutdown).await?;
                }

                calls_made = increment_calls_made(&paths).await?;
                update_status(
                    &paths,
                    loop_count,
                    calls_made,
                    opts.max_calls_per_hour,
                    "codex_exec",
                    "running",
                    "",
                )
                .await?;
                append_log_line(
                    &paths,
                    "LOOP",
                    &format!(
                        "Retrying: Loop #{loop_count} (call {calls_made}/{max})",
                        max = opts.max_calls_per_hour
                    ),
                )
                .await?;
                continue;
            }

            break exec;
        };

        if let Some(thread_id) = exec.thread_id.as_deref()
            && opts.use_continue
        {
            update_session(&paths, thread_id, opts.session_expiry_hours).await?;
        }

        if opts.use_continue {
            if let Some(reason) = should_clear_session_after_exec(&exec) {
                remove_file_if_exists(&paths.session_file).await?;
                append_log_line(
                    &paths,
                    "WARN",
                    &format!("Clearing session for next loop: {reason}"),
                )
                .await?;
            }
        }

        if exec.interrupted {
            update_status(
                &paths,
                loop_count,
                calls_made,
                opts.max_calls_per_hour,
                "shutdown",
                "exited",
                &exec.interrupt_reason,
            )
            .await?;
            append_log_line(
                &paths,
                "WARN",
                &format!("Stopped: {}", exec.interrupt_reason),
            )
            .await?;
            return Ok(());
        }

        let output_length = exec
            .last_message
            .as_deref()
            .map(|s| s.len() as u64)
            .unwrap_or(0);

        let ralph_status = parse_ralph_status_from_last_message(exec.last_message.as_deref());

        let signals = analyze_last_message(
            exec.last_message.as_deref(),
            exec.files_changed,
            exec.error_count,
        );

        let work_summary = if let Some(status) = &ralph_status {
            match status.status {
                LoopCompletionStatus::Blocked => {
                    let recommendation =
                        sanitize_recommendation_for_exit_reason(status.recommendation.as_str());
                    format!("Blocked: {recommendation}")
                }
                LoopCompletionStatus::Complete | LoopCompletionStatus::InProgress => {
                    "Structured Ralph status".to_string()
                }
            }
        } else {
            signals.work_summary.clone()
        };

        let analysis = Analysis {
            has_completion_signal: signals.has_completion_signal,
            is_test_only: signals.is_test_only,
            is_stuck: signals.is_stuck,
            has_progress: exec.files_changed > 0,
            files_modified: exec.files_changed,
            confidence_score: signals.confidence_score,
            exit_signal: signals.exit_signal,
            work_summary,
            output_length,
            error_count: exec.error_count,
        };

        let analysis_file = AnalysisFile {
            loop_number: loop_count,
            timestamp: chrono::Utc::now().to_rfc3339(),
            output_file: exec.last_message_path.display().to_string(),
            output_format: "text".to_string(),
            analysis: analysis.clone(),
        };

        write_json_atomic(&paths.response_analysis_file, &analysis_file).await?;

        let mut exit_signals: ExitSignalsFile = tokio::fs::read(&paths.exit_signals_file)
            .await
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        exit_signals.update_for_loop(loop_count, &analysis);
        write_json_atomic(&paths.exit_signals_file, &exit_signals).await?;

        if let Some(status) = &ralph_status
            && status.status == LoopCompletionStatus::Blocked
        {
            let recommendation =
                sanitize_recommendation_for_exit_reason(status.recommendation.as_str());
            let blocked_reason = format!("Blocked: {recommendation}");
            update_status(
                &paths,
                loop_count,
                calls_made,
                opts.max_calls_per_hour,
                "blocked",
                "exited",
                &blocked_reason,
            )
            .await?;
            append_log_line(&paths, "WARN", &blocked_reason).await?;
            println!("{blocked_reason}");
            return Ok(());
        }

        let files_changed_for_circuit_breaker = if analysis.is_test_only {
            1
        } else {
            exec.files_changed
        };
        let cb_outcome = circuit_breaker::record_loop_result(
            &paths,
            loop_count,
            files_changed_for_circuit_breaker,
            exec.error_signature.as_deref(),
        )
        .await?;

        if cb_outcome.opened {
            update_status(
                &paths,
                loop_count,
                calls_made,
                opts.max_calls_per_hour,
                "circuit_breaker",
                "exited",
                &cb_outcome.state.reason,
            )
            .await?;
            append_log_line(
                &paths,
                "ERROR",
                &format!("Stopping: {}", cb_outcome.state.reason),
            )
            .await?;
            anyhow::bail!("Stopping: {}", cb_outcome.state.reason);
        }

        if analysis.is_test_only
            && analysis.files_modified == 0
            && analysis.error_count == 0
            && has_consecutive_loop_suffix(&exit_signals.test_only_loops, 3, loop_count)
        {
            update_status(
                &paths,
                loop_count,
                calls_made,
                opts.max_calls_per_hour,
                "test_only",
                "completed",
                "3 consecutive test-only loops",
            )
            .await?;
            append_log_line(
                &paths,
                "SUCCESS",
                "Stopping after 3 consecutive test-only loops (likely stable).",
            )
            .await?;
            println!("Stopping after 3 consecutive test-only loops");
            return Ok(());
        }

        if signals.exit_signal && signals.has_completion_signal {
            update_status(
                &paths,
                loop_count,
                calls_made,
                opts.max_calls_per_hour,
                "complete",
                "completed",
                "EXIT_SIGNAL gate satisfied",
            )
            .await?;
            append_log_line(&paths, "SUCCESS", "Exit conditions met; stopping loop.").await?;
            println!("EXIT_SIGNAL gate satisfied");
            return Ok(());
        }

        if exec.exit_code != 0 {
            append_log_line(
                &paths,
                "WARN",
                &format!("codex exec failed with exit code {}", exec.exit_code),
            )
            .await?;
        }

        tokio::select! {
            _ = time::sleep(time::Duration::from_secs(1)) => {}
            _ = shutdown.token.cancelled() => {
                let reason = shutdown.reason();
                update_status(
                    &paths,
                    loop_count,
                    calls_made,
                    opts.max_calls_per_hour,
                    "shutdown",
                    "exited",
                    &format!("Interrupted: {reason:?}"),
                ).await?;
                append_log_line(
                    &paths,
                    "WARN",
                    &format!("Stopping loop due to shutdown: {reason:?}"),
                )
                .await?;
                return Ok(());
            }
        }
    }
}

struct PidGuard {
    path: PathBuf,
}

impl PidGuard {
    async fn write(paths: &RalphPaths) -> anyhow::Result<Self> {
        let pid = std::process::id();
        tokio::fs::write(&paths.pid_file, format!("{pid}\n")).await?;
        Ok(Self {
            path: paths.pid_file.clone(),
        })
    }
}

impl Drop for PidGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn parse_ralph_status_from_last_message(text: Option<&str>) -> Option<RalphStatus> {
    let text = text?;
    serde_json::from_str::<RalphStatus>(text)
        .ok()
        .or_else(|| parse_ralph_status_from_text(text))
}

fn sanitize_recommendation_for_exit_reason(recommendation: &str) -> String {
    let prefix = recommendation
        .split_once("---RALPH_STATUS---")
        .map(|(pre, _)| pre)
        .unwrap_or(recommendation);

    // Collapse whitespace/newlines into a single line so status.json stays readable.
    let mut one_line = String::new();
    for token in prefix.split_whitespace() {
        if !one_line.is_empty() {
            one_line.push(' ');
        }
        one_line.push_str(token);
    }

    truncate(&one_line, 240)
}

fn normalize_output_last_message(
    exit_code: i32,
    raw: Option<String>,
    error_count: &mut u64,
    error_messages: &mut Vec<String>,
) -> Option<String> {
    match raw {
        Some(msg) if !msg.trim().is_empty() => Some(msg),
        _ => {
            // If the child exited successfully but did not emit a final assistant message, treat it as an
            // actionable error so the circuit breaker can stop infinite "no-progress" loops.
            if exit_code == 0 {
                *error_count = error_count.saturating_add(1);
                error_messages.push("codex exec produced no final message".to_string());
            }
            None
        }
    }
}

struct ExecResult {
    exit_code: i32,
    thread_id: Option<String>,
    last_message: Option<String>,
    last_message_path: PathBuf,
    files_changed: u64,
    error_count: u64,
    error_signature: Option<String>,
    saw_thread_compaction_warning: bool,
    interrupted: bool,
    interrupt_reason: String,
}

fn should_clear_session_after_exec(exec: &ExecResult) -> Option<&'static str> {
    if exec.interrupted {
        return None;
    }

    if exec.saw_thread_compaction_warning {
        return Some("thread_compaction_warning");
    }

    if exec.exit_code == 124 {
        return Some("timeout");
    }

    if exec.exit_code == 0 && exec.last_message.is_none() {
        return Some("no_final_message");
    }

    None
}

async fn codex_exec_once(
    paths: &RalphPaths,
    loop_count: u64,
    opts: &RunOptions,
    shutdown: &Shutdown,
) -> anyhow::Result<ExecResult> {
    let ts = Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();

    let events_path = paths.logs_dir.join(format!("codex_events_{ts}.jsonl"));
    let stderr_path = paths.logs_dir.join(format!("codex_stderr_{ts}.log"));
    let last_message_path = paths.logs_dir.join(format!("codex_last_message_{ts}.txt"));

    let session_id = if opts.use_continue {
        read_session_id_if_valid(paths, opts.session_expiry_hours).await?
    } else {
        None
    };

    let mut cmd = process::Command::new(&opts.codex_cmd);
    cmd.current_dir(&paths.cwd);
    if opts.disable_mcp {
        // `-c mcp_servers={}` does not remove existing server entries because config layers
        // are merged (deep-merge for tables). Instead, proactively disable any servers we
        // can discover from common config locations.
        for server_name in detect_mcp_server_names(&paths.cwd).await {
            cmd.arg("-c");
            cmd.arg(format!("mcp_servers.{server_name}.enabled=false"));
        }
    }
    cmd.arg("exec");
    if let Some(model) = opts.exec_model.as_deref() {
        cmd.arg("-m");
        cmd.arg(model);
    }
    for kv in &opts.exec_config_overrides {
        cmd.arg("-c");
        cmd.arg(kv);
    }
    for feature in &opts.exec_enable_features {
        cmd.arg("--enable");
        cmd.arg(feature);
    }
    for feature in &opts.exec_disable_features {
        cmd.arg("--disable");
        cmd.arg(feature);
    }
    cmd.arg("--json");
    cmd.arg("--output-last-message");
    cmd.arg(&last_message_path);
    if opts.use_output_schema {
        cmd.arg("--output-schema");
        cmd.arg(&paths.output_schema_file);
    }
    if opts.skip_git_repo_check {
        cmd.arg("--skip-git-repo-check");
    }
    if opts.full_auto {
        cmd.arg("--full-auto");
    }
    if let Some(session_id) = &session_id {
        cmd.arg("resume");
        cmd.arg(session_id);
    }
    cmd.arg("-");

    let prompt_file = tokio::fs::File::open(&opts.prompt_path)
        .await
        .with_context(|| format!("Failed to open {}", opts.prompt_path.display()))?;
    let prompt_file = prompt_file.into_std().await;
    cmd.stdin(std::process::Stdio::from(prompt_file));
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    append_log_line(
        paths,
        "LOOP",
        &format!(
            "Running: {} exec (timeout: {}m)",
            opts.codex_cmd.display(),
            opts.timeout_minutes
        ),
    )
    .await?;

    let mut child = cmd.spawn().context("Failed to spawn codex exec")?;

    let stdout = child.stdout.take().context("Missing stdout")?;
    let stderr = child.stderr.take().context("Missing stderr")?;
    let mut stdout_lines = BufReader::new(stdout).lines();
    let mut stderr_lines = BufReader::new(stderr).lines();

    let mut events_file = tokio::fs::File::create(&events_path)
        .await
        .with_context(|| format!("Failed to create {}", events_path.display()))?;
    let mut stderr_file = tokio::fs::File::create(&stderr_path)
        .await
        .with_context(|| format!("Failed to create {}", stderr_path.display()))?;

    let start = time::Instant::now();
    let mut last_output = String::new();
    let mut thread_id: Option<String> = None;
    let mut last_agent_message: Option<String> = None;
    let mut files_changed: u64 = 0;
    let mut error_count: u64 = 0;
    let mut error_messages: Vec<String> = Vec::new();
    let mut saw_thread_compaction_warning = false;
    let timeout = time::sleep(time::Duration::from_secs(opts.timeout_minutes * 60));
    tokio::pin!(timeout);
    let mut progress_tick = time::interval(time::Duration::from_secs(1));
    progress_tick.set_missed_tick_behavior(time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = shutdown.token.cancelled() => {
                let reason = shutdown.reason();
                let (exit_code, interrupt_reason) = graceful_shutdown_child(&mut child, reason).await?;
                remove_file_if_exists(&paths.progress_file).await?;
                let last_message = tokio::fs::read_to_string(&last_message_path).await.ok();
                append_log_line(paths, "WARN", &format!("codex exec interrupted: {interrupt_reason}")).await?;
                let error_signature = compute_error_signature(&error_messages);
                return Ok(ExecResult{
                    exit_code,
                    thread_id,
                    last_message,
                    last_message_path: last_message_path.clone(),
                    files_changed,
                    error_count,
                    error_signature,
                    saw_thread_compaction_warning,
                    interrupted: true,
                    interrupt_reason,
                });
            }
            _ = progress_tick.tick() => {
                if tokio::fs::try_exists(&paths.stop_file).await? {
                    shutdown.trigger(ShutdownReason::StopFile);
                }
                let progress = ProgressFile {
                    status: "executing".to_string(),
                    elapsed_seconds: start.elapsed().as_secs(),
                    last_output: last_output.clone(),
                };
                write_json_atomic(&paths.progress_file, &progress).await?;
            }
            res = stdout_lines.next_line() => {
                let Some(line) = res.context("Failed reading stdout")? else {
                    break;
                };
                events_file.write_all(line.as_bytes()).await?;
                events_file.write_all(b"\n").await?;

                if let Ok(ev) = serde_json::from_str::<ThreadEvent>(&line) {
                    match ev {
                        ThreadEvent::ThreadStarted(ev) => {
                            thread_id = Some(ev.thread_id);
                            last_output = format!("thread started (loop {loop_count})");
                        }
                        ThreadEvent::ItemCompleted(ev) => {
                            match ev.item.details {
                                ThreadItemDetails::AgentMessage(item) => {
                                    last_agent_message = Some(item.text.clone());
                                    last_output = "agent message received".to_string();
                                }
                                ThreadItemDetails::FileChange(item)
                                    if item.status == PatchApplyStatus::Completed =>
                                {
                                    let paths_changed: HashSet<_> =
                                        item.changes.into_iter().map(|c| c.path).collect();
                                    files_changed =
                                        files_changed.saturating_add(paths_changed.len() as u64);
                                    last_output = format!("file changes: {}", paths_changed.len());
                                }
                                ThreadItemDetails::Error(item) => {
                                    if item
                                        .message
                                        .to_ascii_lowercase()
                                        .contains("long threads and multiple compactions")
                                    {
                                        saw_thread_compaction_warning = true;
                                    }
                                    error_count = error_count.saturating_add(1);
                                    error_messages.push(item.message.clone());
                                    last_output = format!("error: {}", truncate(&item.message, 120));
                                }
                                _ => {}
                            }
                        }
                        ThreadEvent::Error(err) => {
                            if err
                                .message
                                .to_ascii_lowercase()
                                .contains("long threads and multiple compactions")
                            {
                                saw_thread_compaction_warning = true;
                            }
                            error_count = error_count.saturating_add(1);
                            error_messages.push(err.message.clone());
                            last_output = format!("error: {}", truncate(&err.message, 120));
                        }
                        _ => {}
                    }
                }
            }
            res = stderr_lines.next_line() => {
                let Some(line) = res.context("Failed reading stderr")? else {
                    continue;
                };
                stderr_file.write_all(line.as_bytes()).await?;
                stderr_file.write_all(b"\n").await?;
                if looks_like_error(&line) && !is_ignorable_error_line(&line) {
                    error_messages.push(line.clone());
                }
                if !is_ignorable_error_line(&line) {
                    last_output = format!("stderr: {}", truncate(&line, 120));
                }
                if opts.verbose {
                    eprintln!("codex exec: {line}");
                }
            }
            status = child.wait() => {
                let status = status.context("Failed to wait for child")?;
                let exit_code = status.code().unwrap_or(1);
                remove_file_if_exists(&paths.progress_file).await?;
                let last_message = normalize_output_last_message(
                    exit_code,
                    tokio::fs::read_to_string(&last_message_path).await.ok(),
                    &mut error_count,
                    &mut error_messages,
                );
                let last_message = last_message.or_else(|| last_agent_message.clone());
                append_log_line(paths, "INFO", &format!("codex exec exit code: {exit_code}")).await?;
                let error_signature = compute_error_signature(&error_messages);
                return Ok(ExecResult{
                    exit_code,
                    thread_id,
                    last_message,
                    last_message_path: last_message_path.clone(),
                    files_changed,
                    error_count,
                    error_signature,
                    saw_thread_compaction_warning,
                    interrupted: false,
                    interrupt_reason: String::new(),
                });
            }
            _ = &mut timeout => {
                let _ = child.kill().await;
                remove_file_if_exists(&paths.progress_file).await?;
                let msg = format!("codex exec timed out after {} minutes", opts.timeout_minutes);
                error_count = error_count.saturating_add(1);
                error_messages.push(msg.clone());
                append_log_line(paths, "ERROR", "codex exec timed out").await?;
                let error_signature = compute_error_signature(&error_messages);
                return Ok(ExecResult{
                    exit_code: 124,
                    thread_id,
                    last_message: None,
                    last_message_path: last_message_path.clone(),
                    files_changed,
                    error_count,
                    error_signature,
                    saw_thread_compaction_warning,
                    interrupted: false,
                    interrupt_reason: msg,
                });
            }
        }
    }

    // stdout closed; wait for child.
    let status = child.wait().await.context("Failed to wait for child")?;
    let exit_code = status.code().unwrap_or(1);
    remove_file_if_exists(&paths.progress_file).await?;
    let last_message = normalize_output_last_message(
        exit_code,
        tokio::fs::read_to_string(&last_message_path).await.ok(),
        &mut error_count,
        &mut error_messages,
    );
    let last_message = last_message.or_else(|| last_agent_message);
    append_log_line(paths, "INFO", &format!("codex exec exit code: {exit_code}")).await?;
    let error_signature = compute_error_signature(&error_messages);

    Ok(ExecResult {
        exit_code,
        thread_id,
        last_message,
        last_message_path,
        files_changed,
        error_count,
        error_signature,
        saw_thread_compaction_warning,
        interrupted: false,
        interrupt_reason: String::new(),
    })
}

async fn ensure_output_schema_file(paths: &RalphPaths) -> anyhow::Result<()> {
    let schema = json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "status": { "type": "string", "enum": ["IN_PROGRESS", "COMPLETE", "BLOCKED"] },
            "tasks_completed_this_loop": { "type": "integer", "minimum": 0 },
            "files_modified": { "type": "integer", "minimum": 0 },
            "tests_status": { "type": "string", "enum": ["PASSING", "FAILING", "NOT_RUN"] },
            "work_type": { "type": "string", "enum": ["IMPLEMENTATION", "TESTING", "DOCUMENTATION", "REFACTORING", "DEBUGGING"] },
            "exit_signal": { "type": "boolean" },
            "recommendation": { "type": "string" }
        },
        "required": ["status", "tasks_completed_this_loop", "files_modified", "tests_status", "work_type", "exit_signal", "recommendation"]
    });

    write_json_atomic(&paths.output_schema_file, &schema).await
}

async fn detect_mcp_server_names(cwd: &Path) -> Vec<String> {
    let mut names = HashSet::<String>::new();

    // System config layer.
    names.extend(mcp_server_names_from_toml_file(Path::new("/etc/codex/config.toml")).await);

    // User config layer.
    let codex_home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".codex")));
    if let Some(codex_home) = codex_home {
        names.extend(mcp_server_names_from_toml_file(&codex_home.join("config.toml")).await);
    }

    // Project config layers (best-effort).
    names.extend(mcp_server_names_from_toml_file(&cwd.join(".codex").join("config.toml")).await);
    if let Some(git_root) = git_root(cwd).await {
        names.extend(
            mcp_server_names_from_toml_file(&git_root.join(".codex").join("config.toml")).await,
        );
    }

    let mut names: Vec<String> = names.into_iter().collect();
    names.sort();
    names
}

async fn git_root(cwd: &Path) -> Option<PathBuf> {
    let out = process::Command::new("git")
        .arg("rev-parse")
        .arg("--show-toplevel")
        .current_dir(cwd)
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    Some(PathBuf::from(s))
}

async fn mcp_server_names_from_toml_file(path: &Path) -> HashSet<String> {
    let mut names = HashSet::<String>::new();
    let Ok(contents) = tokio::fs::read_to_string(path).await else {
        return names;
    };
    let Ok(value) = toml::from_str::<toml::Value>(&contents) else {
        return names;
    };
    let Some(mcp_servers) = value.get("mcp_servers").and_then(toml::Value::as_table) else {
        return names;
    };

    names.extend(mcp_servers.keys().cloned());
    names
}

#[derive(Debug, Clone, Copy)]
enum ShutdownReason {
    CtrlC,
    Sigterm,
    StopFile,
}

impl ShutdownReason {
    fn as_code(self) -> u8 {
        match self {
            Self::CtrlC => 1,
            Self::Sigterm => 2,
            Self::StopFile => 3,
        }
    }

    fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::CtrlC),
            2 => Some(Self::Sigterm),
            3 => Some(Self::StopFile),
            _ => None,
        }
    }
}

#[derive(Clone)]
struct Shutdown {
    token: CancellationToken,
    reason: Arc<AtomicU8>,
}

impl Shutdown {
    fn new() -> Self {
        Self {
            token: CancellationToken::new(),
            reason: Arc::new(AtomicU8::new(0)),
        }
    }

    fn trigger(&self, reason: ShutdownReason) {
        let _ =
            self.reason
                .compare_exchange(0, reason.as_code(), Ordering::SeqCst, Ordering::SeqCst);
        self.token.cancel();
    }

    fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }

    fn reason(&self) -> ShutdownReason {
        let code = self.reason.load(Ordering::SeqCst);
        ShutdownReason::from_code(code).unwrap_or(ShutdownReason::CtrlC)
    }
}

async fn listen_for_shutdown(shutdown: Shutdown) {
    #[cfg(unix)]
    {
        use tokio::signal::unix;
        use tokio::signal::unix::SignalKind;

        match unix::signal(SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => shutdown.trigger(ShutdownReason::CtrlC),
                    _ = term.recv() => shutdown.trigger(ShutdownReason::Sigterm),
                }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
                shutdown.trigger(ShutdownReason::CtrlC);
            }
        }
    }

    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
        shutdown.trigger(ShutdownReason::CtrlC);
    }
}

async fn graceful_shutdown_child(
    child: &mut process::Child,
    reason: ShutdownReason,
) -> anyhow::Result<(i32, String)> {
    let (sig, msg) = match reason {
        ShutdownReason::CtrlC => (SignalToSend::Interrupt, "Ctrl-C"),
        ShutdownReason::Sigterm => (SignalToSend::Terminate, "SIGTERM"),
        ShutdownReason::StopFile => (SignalToSend::Terminate, "STOP file"),
    };

    send_signal(child, sig).await?;

    let status = time::timeout(time::Duration::from_secs(5), child.wait()).await;
    let status = match status {
        Ok(status) => status?,
        Err(_) => {
            let _ = child.kill().await;
            child.wait().await?
        }
    };

    let code = status.code().unwrap_or(130);
    Ok((code, msg.to_string()))
}

#[derive(Debug, Clone, Copy)]
enum SignalToSend {
    Interrupt,
    Terminate,
}

async fn send_signal(child: &mut process::Child, signal: SignalToSend) -> anyhow::Result<()> {
    let Some(pid) = child.id() else {
        return Ok(());
    };

    #[cfg(unix)]
    unsafe {
        let sig = match signal {
            SignalToSend::Interrupt => libc::SIGINT,
            SignalToSend::Terminate => libc::SIGTERM,
        };
        libc::kill(pid as i32, sig);
        Ok(())
    }

    #[cfg(not(unix))]
    {
        let _ = signal;
        child.kill().await?;
        Ok(())
    }
}

fn compute_error_signature(errors: &[String]) -> Option<String> {
    let mut normalized: Vec<String> = errors.iter().map(|e| normalize_error(e)).collect();
    normalized.sort();
    normalized.dedup();
    let sig = normalized
        .into_iter()
        .take(3)
        .collect::<Vec<_>>()
        .join(" | ");
    if sig.is_empty() { None } else { Some(sig) }
}

fn normalize_error(message: &str) -> String {
    let lower = message.to_ascii_lowercase();
    let out = normalize_uuid_like(&lower);
    let out = normalize_hex_like(&out);
    let out = normalize_numbers(&out);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn normalize_numbers(message: &str) -> String {
    let mut out = String::with_capacity(message.len());
    let mut chars = message.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch.is_ascii_digit() {
            while chars.peek().is_some_and(char::is_ascii_digit) {
                chars.next();
            }
            out.push_str("<n>");
        } else {
            out.push(ch);
        }
    }

    out
}

fn normalize_uuid_like(message: &str) -> String {
    let bytes = message.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if let Some(len) = uuid_len_at(bytes, i) {
            out.extend_from_slice(b"<uuid>");
            i += len;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn uuid_len_at(bytes: &[u8], start: usize) -> Option<usize> {
    // UUID: 8-4-4-4-12 (36 chars total)
    const UUID_LEN: usize = 36;
    if start + UUID_LEN > bytes.len() {
        return None;
    }

    for i in 0..UUID_LEN {
        let b = bytes[start + i];
        match i {
            8 | 13 | 18 | 23 => {
                if b != b'-' {
                    return None;
                }
            }
            _ => {
                if !b.is_ascii_hexdigit() {
                    return None;
                }
            }
        }
    }

    Some(UUID_LEN)
}

fn normalize_hex_like(message: &str) -> String {
    let bytes = message.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'0'
            && bytes.get(i + 1) == Some(&b'x')
            && bytes.get(i + 2).is_some_and(u8::is_ascii_hexdigit)
        {
            out.extend_from_slice(b"<hex>");
            i += 2;
            while i < bytes.len() && bytes[i].is_ascii_hexdigit() {
                i += 1;
            }
            continue;
        }

        // Replace very long hex-like identifiers (often hashes) to stabilize signatures.
        if bytes[i].is_ascii_hexdigit() {
            let mut j = i;
            while j < bytes.len() && bytes[j].is_ascii_hexdigit() {
                j += 1;
            }
            let len = j - i;
            if len >= 16 {
                out.extend_from_slice(b"<hex>");
                i = j;
                continue;
            }
        }

        out.push(bytes[i]);
        i += 1;
    }

    String::from_utf8_lossy(&out).into_owned()
}

async fn init_call_tracking(paths: &RalphPaths) -> anyhow::Result<()> {
    paths.ensure_dirs().await?;

    let current_hour = current_hour_stamp();
    let last_reset = tokio::fs::read_to_string(&paths.last_reset_file).await.ok();

    if last_reset.as_deref().map(str::trim) != Some(&current_hour) {
        tokio::fs::write(&paths.call_count_file, b"0").await?;
        tokio::fs::write(&paths.last_reset_file, current_hour.as_bytes()).await?;
        append_log_line(
            paths,
            "INFO",
            &format!("Call counter reset for new hour: {current_hour}"),
        )
        .await?;
    }

    Ok(())
}

async fn read_calls_made(paths: &RalphPaths) -> anyhow::Result<u64> {
    let raw = tokio::fs::read_to_string(&paths.call_count_file)
        .await
        .unwrap_or_else(|_| "0".to_string());
    Ok(raw.trim().parse::<u64>().unwrap_or(0))
}

async fn increment_calls_made(paths: &RalphPaths) -> anyhow::Result<u64> {
    let calls = read_calls_made(paths).await? + 1;
    tokio::fs::write(&paths.call_count_file, calls.to_string()).await?;
    Ok(calls)
}

async fn wait_for_reset(paths: &RalphPaths, shutdown: &Shutdown) -> anyhow::Result<()> {
    let now = Local::now();
    let next = (now + chrono::Duration::hours(1))
        .with_minute(0)
        .and_then(|t| t.with_second(0))
        .and_then(|t| t.with_nanosecond(0))
        .unwrap_or_else(|| now + chrono::Duration::hours(1));

    let dur = (next - now)
        .to_std()
        .unwrap_or_else(|_| std::time::Duration::from_secs(60));

    append_log_line(
        paths,
        "INFO",
        &format!("Sleeping for {}s until next hour...", dur.as_secs()),
    )
    .await?;

    // Use a simple 1s tick loop so STOP/shutdown is reliably detected during long sleeps.
    let deadline = time::Instant::now() + dur;
    let mut tick = time::interval(time::Duration::from_secs(1));
    tick.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
    loop {
        if shutdown.is_cancelled() {
            return Ok(());
        }
        if tokio::fs::try_exists(&paths.stop_file).await? {
            shutdown.trigger(ShutdownReason::StopFile);
            return Ok(());
        }
        if time::Instant::now() >= deadline {
            break;
        }
        tick.tick().await;
    }

    if shutdown.is_cancelled() {
        return Ok(());
    }
    init_call_tracking(paths).await
}

async fn update_status(
    paths: &RalphPaths,
    loop_count: u64,
    calls_made: u64,
    max_calls: u64,
    last_action: &str,
    status: &str,
    exit_reason: &str,
) -> anyhow::Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    let next_reset = next_hour_boundary_string();

    let file = StatusFile {
        timestamp: now,
        loop_count,
        calls_made_this_hour: calls_made,
        max_calls_per_hour: max_calls,
        last_action: last_action.to_string(),
        status: status.to_string(),
        exit_reason: exit_reason.to_string(),
        next_reset,
    };

    write_json_atomic(&paths.status_file, &file).await
}

fn current_hour_stamp() -> String {
    let now = Local::now();
    format!(
        "{:04}{:02}{:02}{:02}",
        now.year(),
        now.month(),
        now.day(),
        now.hour()
    )
}

fn next_hour_boundary_string() -> String {
    let now = Local::now();
    let next = (now + chrono::Duration::hours(1))
        .with_minute(0)
        .and_then(|t| t.with_second(0))
        .unwrap_or_else(|| now + chrono::Duration::hours(1));
    next.format("%H:%M:%S").to_string()
}

async fn read_session_id_if_valid(
    paths: &RalphPaths,
    expiry_hours: u64,
) -> anyhow::Result<Option<String>> {
    let bytes = match tokio::fs::read(&paths.session_file).await {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(err)
                .with_context(|| format!("Failed to read {}", paths.session_file.display()));
        }
    };

    let session: SessionFile = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };

    if session.session_id.trim().is_empty() {
        return Ok(None);
    }

    let now = chrono::Utc::now().timestamp();
    let expires_at = session.expires_at_epoch;
    if expires_at > 0 && now >= expires_at {
        return Ok(None);
    }

    // If expires_at_epoch was not set in older files, derive it.
    if expires_at <= 0 {
        let created = session.created_at_epoch;
        if created > 0 {
            let derived = created + (expiry_hours as i64 * 3600);
            if now >= derived {
                return Ok(None);
            }
        }
    }

    Ok(Some(session.session_id))
}

async fn update_session(
    paths: &RalphPaths,
    session_id: &str,
    expiry_hours: u64,
) -> anyhow::Result<()> {
    let now = chrono::Utc::now();
    let now_epoch = now.timestamp();
    let expires = now_epoch + (expiry_hours as i64 * 3600);

    let mut session = match tokio::fs::read(&paths.session_file).await {
        Ok(bytes) => serde_json::from_slice::<SessionFile>(&bytes).unwrap_or_default(),
        Err(_) => SessionFile::default(),
    };

    if session.session_id.trim().is_empty() {
        session.session_id = session_id.to_string();
        session.created_at = now.to_rfc3339();
        session.created_at_epoch = now_epoch;
        session.reset_reason = "created".to_string();
    }

    session.last_used = now.to_rfc3339();
    session.last_used_epoch = now_epoch;
    session.expires_at_epoch = expires;

    write_json_atomic(&paths.session_file, &session).await
}

fn truncate(s: &str, max: usize) -> String {
    let mut out = s.chars().take(max).collect::<String>();
    if s.chars().count() > max {
        out.push_str("...");
    }
    out
}

fn has_consecutive_loop_suffix(values: &[u64], n: usize, current_loop: u64) -> bool {
    if values.len() < n || n == 0 {
        return false;
    }

    // Expect the suffix to be: current_loop, current_loop-1, ..., current_loop-(n-1)
    for i in 0..n {
        let expected = current_loop.saturating_sub(i as u64);
        let actual = values[values.len() - 1 - i];
        if actual != expected {
            return false;
        }
    }

    true
}

fn looks_like_error(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    lower.contains("error")
        || lower.contains("panic")
        || lower.contains("failed")
        || lower.contains("exception")
        || lower.contains("traceback")
}

fn is_ignorable_error_line(line: &str) -> bool {
    // These indicate an internal invariant violation that Codex attempts to repair
    // by inserting synthetic outputs, but they are frequently observed in resumed
    // sessions and are not actionable for ralph-widex's circuit breaker.
    let lower = line.to_ascii_lowercase();
    lower.contains("custom tool call output is missing for call id:")
        || lower.contains("function call output is missing for call id:")
        || lower.contains("local shell call output is missing for call id:")
        || lower.contains("error opening display!")
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn normalizes_numbers_in_error_messages() {
        assert_eq!(
            normalize_error("Error code 123 at line 45"),
            "error code <n> at line <n>"
        );
    }

    #[test]
    fn ignores_missing_call_output_invariant_errors() {
        assert!(is_ignorable_error_line(
            "2026-01-01T00:00:00Z ERROR codex_core::util: Custom tool call output is missing for call id: call_ABC123"
        ));
        assert!(is_ignorable_error_line(
            "Function call output is missing for call id: call_ABC123"
        ));
        assert!(is_ignorable_error_line(
            "Local shell call output is missing for call id: call_ABC123"
        ));
        assert!(is_ignorable_error_line("Error opening display!"));
        assert!(!is_ignorable_error_line("ERROR: something actually failed"));
    }

    #[test]
    fn computes_stable_signature_across_variable_numbers() {
        let errors1 = vec!["Error 123".to_string(), "Oops 9".to_string()];
        let errors2 = vec!["Oops 10".to_string(), "Error 456".to_string()];

        assert_eq!(
            compute_error_signature(&errors1),
            compute_error_signature(&errors2)
        );
    }

    #[test]
    fn normalizes_uuid_like_error_ids() {
        assert_eq!(
            normalize_error("request 123e4567-e89b-12d3-a456-426614174000 failed"),
            "request <uuid> failed"
        );
    }

    #[test]
    fn normalizes_hex_literals() {
        assert_eq!(normalize_error("panic at 0xdeadbeef"), "panic at <hex>");
    }
}
