use crate::ralph_status::parse_ralph_status_from_text;
use crate::ralph_storage::RALPH_DIR;
use crate::ralph_storage::RalphPaths;
use crate::ralph_storage::acquire_lock;
use crate::ralph_storage::append_log_line;
use crate::ralph_storage::remove_file_if_exists;
use crate::ralph_storage::write_json_atomic;
use anyhow::Context;
use chrono::Datelike;
use chrono::Local;
use chrono::Timelike;
use codex_exec::exec_events::PatchApplyStatus;
use codex_exec::exec_events::ThreadEvent;
use codex_exec::exec_events::ThreadItemDetails;
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncWriteExt;
use tokio::io::BufReader;
use tokio::process;
use tokio::time;

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
        "Next: edit {RALPH_DIR}/PROMPT.md and {RALPH_DIR}/@fix_plan.md then run: codex ralph-widex run"
    );

    Ok(())
}

pub(crate) async fn run_loop(cwd: &Path, opts: RunOptions) -> anyhow::Result<()> {
    let paths = RalphPaths::new(cwd);
    paths.ensure_dirs().await?;

    let _lock = acquire_lock(&paths).await?;

    if !tokio::fs::try_exists(&opts.prompt_path).await? {
        anyhow::bail!(
            "Prompt file not found: {} (run `codex ralph-widex init` first)",
            opts.prompt_path.display()
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
            wait_for_reset(&paths).await?;
            continue;
        }

        let calls_made = increment_calls_made(&paths).await?;
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

        let exec = codex_exec_once(&paths, loop_count, &opts).await?;

        if let Some(thread_id) = exec.thread_id.as_deref()
            && opts.use_continue
        {
            update_session(&paths, thread_id, opts.session_expiry_hours).await?;
        }

        let gate_satisfied = exec
            .last_message
            .as_deref()
            .and_then(parse_ralph_status_from_text)
            .is_some_and(|s| s.gate_satisfied());

        if gate_satisfied {
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
            update_status(
                &paths,
                loop_count,
                calls_made,
                opts.max_calls_per_hour,
                "error",
                "exited",
                "codex exec failed",
            )
            .await?;
            anyhow::bail!("codex exec failed with exit code {}", exec.exit_code);
        }

        time::sleep(time::Duration::from_secs(1)).await;
    }
}

struct ExecResult {
    exit_code: i32,
    thread_id: Option<String>,
    last_message: Option<String>,
}

async fn codex_exec_once(
    paths: &RalphPaths,
    loop_count: u64,
    opts: &RunOptions,
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
    cmd.arg("exec");
    cmd.arg("--json");
    cmd.arg("--output-last-message");
    cmd.arg(&last_message_path);
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
    let timeout = time::sleep(time::Duration::from_secs(opts.timeout_minutes * 60));
    tokio::pin!(timeout);

    loop {
        tokio::select! {
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
                            if let ThreadItemDetails::FileChange(item) = ev.item.details
                                && item.status == PatchApplyStatus::Completed {
                                    let paths_changed: HashSet<_> = item.changes.into_iter().map(|c| c.path).collect();
                                    last_output = format!("file changes: {}", paths_changed.len());
                                }
                        }
                        ThreadEvent::Error(err) => {
                            last_output = format!("error: {}", truncate(&err.message, 120));
                        }
                        _ => {}
                    }
                }

                let progress = ProgressFile {
                    status: "executing".to_string(),
                    elapsed_seconds: start.elapsed().as_secs(),
                    last_output: last_output.clone(),
                };
                write_json_atomic(&paths.progress_file, &progress).await?;
            }
            res = stderr_lines.next_line() => {
                let Some(line) = res.context("Failed reading stderr")? else {
                    continue;
                };
                stderr_file.write_all(line.as_bytes()).await?;
                stderr_file.write_all(b"\n").await?;
                if opts.verbose {
                    eprintln!("codex exec: {line}");
                }
            }
            status = child.wait() => {
                let status = status.context("Failed to wait for child")?;
                let exit_code = status.code().unwrap_or(1);
                remove_file_if_exists(&paths.progress_file).await?;
                let last_message = tokio::fs::read_to_string(&last_message_path).await.ok();
                append_log_line(paths, "INFO", &format!("codex exec exit code: {exit_code}")).await?;
                return Ok(ExecResult{ exit_code, thread_id, last_message });
            }
            _ = &mut timeout => {
                let _ = child.kill().await;
                remove_file_if_exists(&paths.progress_file).await?;
                append_log_line(paths, "ERROR", "codex exec timed out").await?;
                anyhow::bail!("codex exec timed out after {} minutes", opts.timeout_minutes);
            }
        }
    }

    // stdout closed; wait for child.
    let status = child.wait().await.context("Failed to wait for child")?;
    let exit_code = status.code().unwrap_or(1);
    remove_file_if_exists(&paths.progress_file).await?;
    let last_message = tokio::fs::read_to_string(&last_message_path).await.ok();
    append_log_line(paths, "INFO", &format!("codex exec exit code: {exit_code}")).await?;

    Ok(ExecResult {
        exit_code,
        thread_id,
        last_message,
    })
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

async fn wait_for_reset(paths: &RalphPaths) -> anyhow::Result<()> {
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

    time::sleep(dur).await;
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
