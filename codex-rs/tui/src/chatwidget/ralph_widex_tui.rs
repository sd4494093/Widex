//! In-TUI Ralph autonomous loop (Widex).
//!
//! This module owns all Widex `/ralph-widex` state and behavior layered on top of
//! the upstream `ChatWidget`: loop bookkeeping (`RalphTuiState`), slash-command
//! dispatch, per-iteration prompt enqueueing, per-hour call limiting, per-turn
//! timeout watchdogs, and `.ralph/` progress/status file maintenance. Keeping it
//! here (instead of inside `chatwidget.rs`) minimizes the Widex overlay footprint
//! on upstream files.
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use chrono::Local;
use chrono::Timelike;
use codex_ralph_widex::CompletionMode as RalphOverlayCompletionMode;
use codex_ralph_widex::widex_overlay;

use crate::app_command::AppCommand;
use crate::app_event::AppEvent;
use crate::exec_command::escape_command;
use crate::history_cell;

use super::ChatWidget;

#[derive(Debug, Clone)]
pub(super) struct RalphTuiState {
    max_loops: u64,
    current_loop: u64,
    completion_phrases: Vec<String>,
    completion_mode: RalphCompletionMode,
    completion_regexes: Vec<regex_lite::Regex>,
    completion_regex_patterns: Vec<String>,
    in_flight: bool,
    // Optional per-hour limiter for the number of Ralph turns (not tool calls).
    max_calls_per_hour: Option<u64>,
    call_window_key: String,
    calls_made_in_window: u64,
    // Per-turn watchdog that interrupts the current turn after the timeout.
    timeout_minutes: Option<u64>,
    // Guards against stale timeout timers from previous turns.
    active_turn_nonce: u64,
    // Best-effort: why the current (or most recent) turn aborted, if known.
    last_abort_reason: Option<String>,
    // True if the per-turn watchdog fired for the current turn.
    turn_timed_out: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RalphCompletionMode {
    Contains,
    PromiseTag,
    Regex,
}

impl RalphCompletionMode {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "contains" => Some(Self::Contains),
            "promise-tag" => Some(Self::PromiseTag),
            "regex" => Some(Self::Regex),
            _ => None,
        }
    }

    fn as_overlay_mode(self) -> RalphOverlayCompletionMode {
        match self {
            Self::Contains => RalphOverlayCompletionMode::Contains,
            Self::PromiseTag => RalphOverlayCompletionMode::PromiseTag,
            Self::Regex => RalphOverlayCompletionMode::Regex,
        }
    }
}

#[derive(Debug, Clone)]
struct RalphTuiConfig {
    loops: u64,
    completion_phrases: Vec<String>,
    completion_mode: RalphCompletionMode,
    completion_regexes: Vec<String>,
    max_calls_per_hour: Option<u64>,
    timeout_minutes: Option<u64>,
    skip_git_repo_check: bool,
}

impl ChatWidget {
    pub(super) fn dispatch_ralph_tui_command(&mut self, args: &str) {
        let argv = shlex::split(args)
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| vec![args.to_string()]);

        if argv.iter().any(|a| a == "-h" || a == "--help") {
            self.add_info_message(widex_overlay::tui_help_text().to_string(), None);
            return;
        }

        let Some((first, rest)) = argv.split_first() else {
            self.start_ralph_tui(RalphTuiConfig {
                loops: widex_overlay::DEFAULT_TUI_LOOPS,
                completion_phrases: vec![widex_overlay::DEFAULT_TUI_COMPLETION_PHRASE.to_string()],
                completion_mode: RalphCompletionMode::Contains,
                completion_regexes: Vec::new(),
                max_calls_per_hour: None,
                timeout_minutes: None,
                skip_git_repo_check: false,
            });
            return;
        };

        match first.as_str() {
            "init" => {
                // Delegate to the Rust-native CLI init (creates .ralph/ in the current directory).
                let mut argv = vec!["init".to_string()];
                argv.extend(rest.iter().cloned());
                self.run_ralph_widex(argv);
            }
            "stop" => self.stop_ralph_tui("stopped by user"),
            "status" => self.print_ralph_tui_status(),
            "daemon" => {
                self.add_error_message(
                    "Unsupported Ralph subcommand. Use `/ralph-widex start`.".to_string(),
                );
            }
            "start" | "run" => {
                let cfg = parse_ralph_tui_args(rest);
                self.start_ralph_tui(cfg);
            }
            _ => {
                // Treat unknown subcommand as a shorthand for `start ...`.
                let mut tokens = vec![first.clone()];
                tokens.extend_from_slice(rest);
                let cfg = parse_ralph_tui_args(&tokens);
                self.start_ralph_tui(cfg);
            }
        }
    }

    fn start_ralph_tui(&mut self, mut cfg: RalphTuiConfig) {
        if self.ralph_tui.is_some() {
            self.add_error_message(
                "Ralph loop already running. Use /ralph-widex stop first.".to_string(),
            );
            return;
        }

        if cfg.completion_phrases.is_empty() {
            cfg.completion_phrases
                .push(widex_overlay::DEFAULT_TUI_COMPLETION_PHRASE.to_string());
        }

        let completion_regexes = if cfg.completion_mode == RalphCompletionMode::Regex {
            if cfg.completion_regexes.is_empty() {
                self.add_error_message(
                    "Missing --completion-regex (required when --completion-mode regex)."
                        .to_string(),
                );
                return;
            }

            let mut compiled = Vec::new();
            for pattern in &cfg.completion_regexes {
                match regex_lite::Regex::new(pattern) {
                    Ok(re) => compiled.push(re),
                    Err(err) => {
                        self.add_error_message(format!(
                            "Invalid --completion-regex {pattern}: {err}"
                        ));
                        return;
                    }
                }
            }
            compiled
        } else {
            Vec::new()
        };

        let ralph_dir = self.config.cwd.join(".ralph");
        let prompt_path = ralph_dir.join("PROMPT.md");
        if !prompt_path.exists() {
            self.add_error_message(format!(
                "Missing {} (run `/ralph-widex init` first).",
                prompt_path.display()
            ));
            return;
        }

        if !cfg.skip_git_repo_check && !self.config.cwd.join(".git").exists() {
            self.add_error_message(
                "Not inside a git repository. Use --skip-git-repo-check to override.".to_string(),
            );
            return;
        }

        // Best-effort: ensure expected files exist so the prompt can reliably reference them.
        let _ = std::fs::create_dir_all(ralph_dir.join("logs"));
        let _ = widex_overlay::ensure_fix_progress_file(&ralph_dir);
        // Clear STOP so a previous run doesn't immediately stop the loop.
        let _ = std::fs::remove_file(ralph_dir.join("STOP"));

        let now = Local::now();
        let call_window_key = now.format("%Y%m%d%H").to_string();

        let state = RalphTuiState {
            max_loops: cfg.loops,
            current_loop: 1,
            completion_phrases: cfg.completion_phrases.clone(),
            completion_mode: cfg.completion_mode,
            completion_regexes,
            completion_regex_patterns: cfg.completion_regexes.clone(),
            in_flight: false,
            max_calls_per_hour: cfg.max_calls_per_hour,
            call_window_key,
            calls_made_in_window: 0,
            timeout_minutes: cfg.timeout_minutes,
            active_turn_nonce: 0,
            last_abort_reason: None,
            turn_timed_out: false,
        };
        self.ralph_tui = Some(state);

        // Best-effort: write the Ralph status file so TUI status reporting stays consistent while
        // the loop is running.
        if let Some(state) = self.ralph_tui.as_ref() {
            let status_path = ralph_dir.join("status.json");
            let log_path = ralph_dir.join("logs").join("ralph.log");
            let _ = write_ralph_tui_status(&status_path, state, "starting", "running", "");
            let _ = append_ralph_tui_log_line(&log_path, "INFO", "starting");
        }

        let max = if cfg.loops == 0 {
            "infinite".to_string()
        } else {
            cfg.loops.to_string()
        };
        let calls = cfg
            .max_calls_per_hour
            .map(|n| n.to_string())
            .unwrap_or_else(|| "unlimited".to_string());
        let timeout = cfg
            .timeout_minutes
            .map(|n| format!("{n}m"))
            .unwrap_or_else(|| "none".to_string());

        let completion = match cfg.completion_mode {
            RalphCompletionMode::Contains => cfg.completion_phrases.join(" | "),
            RalphCompletionMode::PromiseTag => {
                format!("promise-tag: {}", cfg.completion_phrases.join(" | "))
            }
            RalphCompletionMode::Regex => format!("regex: {}", cfg.completion_regexes.join(" | ")),
        };
        self.add_info_message(
            format!(
                "Ralph loop started (loops={max}, calls/hour={calls}, timeout={timeout}, completion={completion})."
            ),
            Some(
                "It will keep iterating inside this Widex session. Use /ralph-widex stop to stop."
                    .to_string(),
            ),
        );

        if let Some(header) = self.ralph_status_header() {
            self.set_status_header(header);
        }

        self.enqueue_ralph_tui_iteration();
    }

    fn stop_ralph_tui(&mut self, reason: &str) {
        let Some(mut state) = self.ralph_tui.take() else {
            self.add_info_message("Ralph loop is not running.".to_string(), None);
            return;
        };

        // If the loop is in-flight, interrupt the current turn so stopping takes effect quickly.
        if state.in_flight && self.bottom_pane.is_task_running() {
            self.submit_op(AppCommand::interrupt());
        }
        state.in_flight = false;

        let ralph_dir = self.config.cwd.join(".ralph");
        let status_path = ralph_dir.join("status.json");
        let log_path = ralph_dir.join("logs").join("ralph.log");
        let last_action = "stopped by user";
        let _ = write_ralph_tui_status(&status_path, &state, last_action, "stopped", reason);
        let _ = append_ralph_tui_log_line(&log_path, "INFO", &format!("stopped: {reason}"));

        self.add_info_message(format!("Ralph loop stopped: {reason}."), None);
        self.restore_reasoning_status_header();
    }

    fn print_ralph_tui_status(&mut self) {
        let Some(state) = self.ralph_tui.as_ref() else {
            self.add_info_message("Ralph loop: inactive.".to_string(), None);
            return;
        };
        let now = Local::now();
        let max = if state.max_loops == 0 {
            "infinite".to_string()
        } else {
            state.max_loops.to_string()
        };

        let calls_max = state
            .max_calls_per_hour
            .filter(|n| *n != 0)
            .map(|n| n.to_string())
            .unwrap_or_else(|| "unlimited".to_string());
        let calls = state.calls_made_in_window;

        let reset_in_secs = now
            .with_minute(0)
            .and_then(|t| t.with_second(0))
            .and_then(|t| t.with_nanosecond(0))
            .map(|base| {
                (base + chrono::Duration::hours(1) - now)
                    .num_seconds()
                    .max(0) as u64
            });

        let timeout = state
            .timeout_minutes
            .filter(|n| *n != 0)
            .map(|n| format!("{n}m"))
            .unwrap_or_else(|| "none".to_string());

        let completion = match state.completion_mode {
            RalphCompletionMode::Contains => {
                format!("contains: {}", state.completion_phrases.join(" | "))
            }
            RalphCompletionMode::PromiseTag => {
                format!("promise-tag: {}", state.completion_phrases.join(" | "))
            }
            RalphCompletionMode::Regex => {
                format!("regex: {}", state.completion_regex_patterns.join(" | "))
            }
        };

        let stop_file_present = self.config.cwd.join(".ralph").join("STOP").exists();

        let mut msg = String::new();
        msg.push_str("Ralph-Widex (TUI)\n\n");
        msg.push_str(&format!(
            "Status: active (loop {}/{max}, in_flight={}).\n",
            state.current_loop, state.in_flight
        ));
        msg.push_str(&format!("Calls/hour: {calls}/{calls_max}.\n"));
        if let Some(reset_in_secs) = reset_in_secs {
            msg.push_str(&format!("Next reset: in {reset_in_secs}s.\n"));
        }
        msg.push_str(&format!("Timeout: {timeout}.\n"));
        msg.push_str(&format!("Completion: {completion}.\n"));
        msg.push_str(&format!(
            "STOP file: {}.\n",
            if stop_file_present {
                "present"
            } else {
                "absent"
            }
        ));

        self.add_info_message(msg, None);
    }

    fn enqueue_ralph_tui_iteration(&mut self) {
        let now = Local::now();
        let ralph_dir = self.config.cwd.join(".ralph");

        let mut wait_for_reset: Option<(u64, u64)> = None; // (wait_secs, max)
        let mut next_prompt: Option<(String, Option<u64>, u64)> = None; // (prompt, timeout_minutes, nonce)
        let mut fix_progress_event: Option<RalphFixProgressEvent> = None;

        {
            let Some(state) = self.ralph_tui.as_mut() else {
                return;
            };
            if state.in_flight {
                return;
            }

            // Reset call window if the hour changed.
            let window = now.format("%Y%m%d%H").to_string();
            if state.call_window_key != window {
                state.call_window_key = window;
                state.calls_made_in_window = 0;
            }

            if let Some(max) = state.max_calls_per_hour
                && max != 0
                && state.calls_made_in_window >= max
            {
                let Some(base) = now
                    .with_minute(0)
                    .and_then(|t| t.with_second(0))
                    .and_then(|t| t.with_nanosecond(0))
                else {
                    return;
                };
                let next_hour = base + chrono::Duration::hours(1);
                let wait_secs = (next_hour - now).num_seconds().max(1) as u64;
                wait_for_reset = Some((wait_secs, max));
            } else {
                let text = widex_overlay::render_tui_loop_prompt(
                    state.current_loop,
                    state.max_loops,
                    state.completion_mode.as_overlay_mode(),
                    &state.completion_phrases,
                    &state.completion_regex_patterns,
                );

                state.in_flight = true;
                state.calls_made_in_window = state.calls_made_in_window.saturating_add(1);
                state.active_turn_nonce = state.active_turn_nonce.saturating_add(1);
                state.last_abort_reason = None;
                state.turn_timed_out = false;
                let nonce = state.active_turn_nonce;
                next_prompt = Some((text, state.timeout_minutes, nonce));
                fix_progress_event = Some(RalphFixProgressEvent::start(state.current_loop));
            }
        }

        if let Some((wait_secs, max)) = wait_for_reset {
            let calls_made = self
                .ralph_tui
                .as_ref()
                .map(|s| s.calls_made_in_window)
                .unwrap_or_default();
            self.add_info_message(
                format!(
                    "Ralph reached calls/hour limit ({calls_made}/{max}); waiting {wait_secs}s for reset."
                ),
                None,
            );
            let tx = self.app_event_tx.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(wait_secs)).await;
                tx.send(AppEvent::RalphTuiWake);
            });
            return;
        }

        let Some((text, timeout_minutes, nonce)) = next_prompt else {
            return;
        };

        if let Some(event) = fix_progress_event
            && let Err(err) = write_ralph_fix_progress_event(&ralph_dir, event)
        {
            self.add_error_message(format!("Failed to update .ralph/@fix_progress.md: {err}"));
        }

        if let Some(state) = self.ralph_tui.as_ref() {
            let status_path = ralph_dir.join("status.json");
            let log_path = ralph_dir.join("logs").join("ralph.log");
            let last_action = format!("loop {} start", state.current_loop);
            let _ = write_ralph_tui_status(&status_path, state, &last_action, "running", "");
            let _ = append_ralph_tui_log_line(&log_path, "INFO", &last_action);
        }

        self.queue_user_message(text.into());

        if let Some(timeout) = timeout_minutes
            && timeout != 0
        {
            let tx = self.app_event_tx.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(timeout.saturating_mul(60))).await;
                tx.send(AppEvent::RalphTuiTimeout { nonce });
            });
        }
    }

    pub(crate) fn on_ralph_tui_wake(&mut self) {
        if self.ralph_tui.is_none() {
            return;
        }
        self.enqueue_ralph_tui_iteration();
        self.maybe_send_next_queued_input();
        self.request_redraw();
    }

    pub(crate) fn on_ralph_tui_timeout(&mut self, nonce: u64) {
        let Some(state) = self.ralph_tui.as_mut() else {
            return;
        };
        if !state.in_flight {
            return;
        }
        if state.active_turn_nonce != nonce {
            return;
        }
        if !self.bottom_pane.is_task_running() {
            return;
        }

        state.turn_timed_out = true;
        if state.last_abort_reason.is_none() {
            state.last_abort_reason = Some("timeout".to_string());
        }

        self.add_to_history(history_cell::new_error_event(
            "Ralph turn timed out; interrupting this turn and continuing.".to_string(),
        ));
        self.submit_op(AppCommand::interrupt());
        self.request_redraw();
    }

    pub(super) fn on_ralph_tui_task_complete(&mut self, last_agent_message: Option<&str>) {
        let stop_file = self.config.cwd.join(".ralph").join("STOP");
        let stop_file_present = stop_file.exists();
        let mut stop_reason: Option<&'static str> = None;
        let mut next_prompt: Option<String> = None;

        let fix_progress_event = {
            let Some(state) = self.ralph_tui.as_mut() else {
                return;
            };
            if !state.in_flight {
                return;
            }

            state.in_flight = false;

            let completion_seen = completion_signal_seen(
                last_agent_message,
                state.completion_mode,
                &state.completion_phrases,
                &state.completion_regexes,
            );
            let loop_num = state.current_loop;
            let max_loops = state.max_loops;

            // Stop precedence: STOP file > completion signal > max loops.
            if stop_file_present {
                stop_reason = Some("STOP file");
            } else if completion_seen {
                stop_reason = Some("completion signal seen");
            } else if max_loops != 0 && loop_num >= max_loops {
                stop_reason = Some("reached max loops");
            } else {
                state.current_loop = state.current_loop.saturating_add(1);

                next_prompt = Some(widex_overlay::render_tui_loop_prompt(
                    state.current_loop,
                    state.max_loops,
                    state.completion_mode.as_overlay_mode(),
                    &state.completion_phrases,
                    &state.completion_regex_patterns,
                ));

                state.in_flight = true;
            }

            let aborted = last_agent_message.is_none();
            RalphFixProgressEvent::end(
                loop_num,
                completion_seen,
                stop_reason,
                aborted,
                state.last_abort_reason.as_deref(),
                state.turn_timed_out,
            )
        };

        let ralph_dir = self.config.cwd.join(".ralph");
        let loop_num = fix_progress_event.loop_number;
        if let Err(err) = write_ralph_fix_progress_event(&ralph_dir, fix_progress_event) {
            self.add_error_message(format!("Failed to update .ralph/@fix_progress.md: {err}"));
        }

        if let Some(reason) = stop_reason {
            if let Some(state) = self.ralph_tui.as_ref() {
                let status_path = ralph_dir.join("status.json");
                let log_path = ralph_dir.join("logs").join("ralph.log");

                let (status, exit_reason) = match reason {
                    "STOP file" => ("stopped", "STOP file"),
                    "completion signal seen" => ("completed", "completion signal seen"),
                    "reached max loops" => ("exited", "reached max loops"),
                    _ => ("exited", reason),
                };
                let last_action = format!("loop {loop_num} end");
                let _ =
                    write_ralph_tui_status(&status_path, state, &last_action, status, exit_reason);
                let _ = append_ralph_tui_log_line(
                    &log_path,
                    "INFO",
                    &format!("Stopping: {exit_reason}"),
                );
            }

            self.ralph_tui = None;
            self.add_info_message(format!("Ralph loop stopped: {reason}."), None);
            self.restore_reasoning_status_header();
            return;
        }

        if let Some(text) = next_prompt {
            if let Some(state) = self.ralph_tui.as_ref() {
                let status_path = ralph_dir.join("status.json");
                let log_path = ralph_dir.join("logs").join("ralph.log");
                let last_action = format!("loop {loop_num} end");
                let _ = write_ralph_tui_status(&status_path, state, &last_action, "running", "");
                let _ = append_ralph_tui_log_line(&log_path, "INFO", &last_action);
            }
            self.queue_user_message(text.into());
        }
    }

    fn run_ralph_widex(&mut self, argv: Vec<String>) {
        if let Ok(cmd) = std::env::var("WIDEX_CMD")
            && !cmd.trim().is_empty()
        {
            let mut cmd: Vec<String> = vec![cmd, "ralph-widex".to_string()];
            cmd.extend(argv);
            let command = escape_command(&cmd);
            self.submit_op(AppCommand::run_user_shell_command(command));
            return;
        }

        // Prefer the current Widex/Codex executable so packaged Windows installs keep routing
        // Ralph back through the same binary instead of falling through to an unrelated `codex`
        // executable on PATH.
        let codex_cmd = ralph_widex_command_for_current_exe(std::env::current_exe().ok());

        let mut cmd: Vec<String> = vec![codex_cmd, "ralph-widex".to_string()];
        cmd.extend(argv);

        let command = escape_command(&cmd);
        self.submit_op(AppCommand::run_user_shell_command(command));
    }
    pub(super) fn ralph_status_header(&self) -> Option<String> {
        let state = self.ralph_tui.as_ref()?;
        let max = if state.max_loops == 0 {
            "inf".to_string()
        } else {
            state.max_loops.to_string()
        };
        Some(format!("Ralph {}/{}", state.current_loop, max))
    }

    pub(super) fn ralph_working_status_header(&self) -> Option<String> {
        let state = self.ralph_tui.as_ref()?;
        let max = if state.max_loops == 0 {
            "infinite".to_string()
        } else {
            state.max_loops.to_string()
        };
        Some(format!("Working (Ralph {}/{max})", state.current_loop))
    }

    /// Blocks quit requests while the Ralph loop is active, pointing the user
    /// at `/ralph-widex stop`. Returns true when the quit was blocked.
    pub(super) fn ralph_tui_block_quit(&mut self) -> bool {
        let Some(state) = self.ralph_tui.as_ref() else {
            return false;
        };
        let max = if state.max_loops == 0 {
            "infinite".to_string()
        } else {
            state.max_loops.to_string()
        };
        self.add_info_message(
            format!(
                "Ralph loop is active ({}/{max}). Use /ralph-widex stop before exiting.",
                state.current_loop
            ),
            None,
        );
        true
    }

    /// Ralph-aware handling for an interrupted turn.
    ///
    /// Returns true when the interruption was consumed by the Ralph loop (the
    /// loop records the abort reason and advances to the next iteration), in
    /// which case the caller must skip the regular interrupted-turn flow.
    pub(super) fn on_ralph_tui_interrupted_turn(&mut self, reason: super::TurnAbortReason) -> bool {
        if !self.ralph_tui.as_ref().is_some_and(|state| state.in_flight) {
            return false;
        }
        if let Some(state) = self.ralph_tui.as_mut()
            && state.in_flight
            && state.last_abort_reason.is_none()
        {
            state.last_abort_reason = Some(format!("{reason:?}"));
        }
        self.add_to_history(history_cell::new_error_event(
            "Conversation interrupted.".to_owned(),
        ));
        self.on_ralph_tui_task_complete(None);
        self.maybe_send_next_queued_input();
        self.request_redraw();
        true
    }

    /// While the Ralph loop is active, Ctrl+C must never arm the quit
    /// shortcut; it only interrupts the current turn. Returns true when the
    /// key press was consumed here.
    pub(super) fn ralph_tui_handle_ctrl_c(&mut self) -> bool {
        if self.ralph_tui.is_none() {
            return false;
        }
        self.quit_shortcut_expires_at = None;
        self.quit_shortcut_key = None;
        self.bottom_pane.clear_quit_shortcut_hint();

        let _ = self.bottom_pane.on_ctrl_c();
        if self.is_cancellable_work_active() {
            self.submit_op(AppCommand::interrupt());
        }
        true
    }

    /// While the Ralph loop is active, Ctrl+D must not quit. Returns true
    /// when the key press was consumed here.
    pub(super) fn ralph_tui_handle_ctrl_d(&mut self) -> bool {
        if self.ralph_tui.is_none() {
            return false;
        }
        self.bottom_pane.clear_quit_shortcut_hint();
        self.quit_shortcut_expires_at = None;
        self.quit_shortcut_key = None;
        true
    }

    /// UX guard: if a `/ralph-widex ...` command reached the model input path
    /// (e.g. pasted with leading whitespace, or typed behind a `!` shell
    /// escape), handle it locally instead of sending it to the model. Returns
    /// true when the text was consumed as a Ralph command.
    pub(super) fn ralph_tui_intercept_user_text(&mut self, text: &str) -> bool {
        let trimmed = text.trim_start();
        let candidate = if trimmed == "/ralph-widex" || trimmed.starts_with("/ralph-widex ") {
            Some(trimmed)
        } else if let Some(stripped) = trimmed.strip_prefix('!') {
            let shell_cmd = stripped.trim_start();
            (shell_cmd == "/ralph-widex" || shell_cmd.starts_with("/ralph-widex "))
                .then_some(shell_cmd)
        } else {
            None
        };
        let Some(command_text) = candidate else {
            return false;
        };
        let rest = command_text
            .strip_prefix("/ralph-widex")
            .unwrap_or(command_text)
            .trim_start();
        self.dispatch_ralph_tui_command(rest);
        true
    }
}

const RALPH_FIX_PROGRESS_AUTOLOG_JSONL: &str = ".fix_progress.autolog.jsonl";
const RALPH_FIX_PROGRESS_AUTOLOG_START: &str = widex_overlay::FIX_PROGRESS_AUTOLOG_START;
const RALPH_FIX_PROGRESS_AUTOLOG_END: &str = widex_overlay::FIX_PROGRESS_AUTOLOG_END;
const RALPH_FIX_PROGRESS_AUTOLOG_MAX_EVENTS: usize = 200;
const RALPH_FIX_PROGRESS_REGENERATE_MAX_ATTEMPTS: usize = 10;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum RalphFixProgressEventKind {
    Start,
    End,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct RalphFixProgressEvent {
    ts: String,
    loop_number: u64,
    kind: RalphFixProgressEventKind,
    completion_seen: Option<bool>,
    stop_reason: Option<String>,
    interrupted: Option<bool>,
    abort_reason: Option<String>,
    timed_out: Option<bool>,
}

impl RalphFixProgressEvent {
    fn start(loop_number: u64) -> Self {
        Self {
            ts: Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            loop_number,
            kind: RalphFixProgressEventKind::Start,
            completion_seen: None,
            stop_reason: None,
            interrupted: None,
            abort_reason: None,
            timed_out: None,
        }
    }

    fn end(
        loop_number: u64,
        completion_seen: bool,
        stop_reason: Option<&str>,
        aborted: bool,
        abort_reason: Option<&str>,
        timed_out: bool,
    ) -> Self {
        Self {
            ts: Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            loop_number,
            kind: RalphFixProgressEventKind::End,
            completion_seen: Some(completion_seen),
            stop_reason: stop_reason.map(std::string::ToString::to_string),
            interrupted: Some(aborted),
            abort_reason: abort_reason.map(std::string::ToString::to_string),
            timed_out: Some(timed_out),
        }
    }
}

fn write_ralph_fix_progress_event(
    ralph_dir: &Path,
    event: RalphFixProgressEvent,
) -> std::io::Result<()> {
    let autolog_path = ralph_dir.join(RALPH_FIX_PROGRESS_AUTOLOG_JSONL);
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&autolog_path)?;
    let line = serde_json::to_string(&event).map_err(std::io::Error::other)?;
    writeln!(file, "{line}")?;

    regenerate_ralph_fix_progress_md(ralph_dir)
}

fn regenerate_ralph_fix_progress_md(ralph_dir: &Path) -> std::io::Result<()> {
    let md_path = ralph_dir.join("@fix_progress.md");
    for attempt in 0..RALPH_FIX_PROGRESS_REGENERATE_MAX_ATTEMPTS {
        let mut original = match std::fs::read_to_string(&md_path) {
            Ok(v) => v,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                widex_overlay::ensure_fix_progress_file(ralph_dir)?;
                std::fs::read_to_string(&md_path)?
            }
            Err(err) => return Err(err),
        };

        if original.trim().is_empty() {
            widex_overlay::ensure_fix_progress_file(ralph_dir)?;
            original = std::fs::read_to_string(&md_path)?;
        }
        let mut contents = original.clone();

        if !contents.contains(RALPH_FIX_PROGRESS_AUTOLOG_START)
            || !contents.contains(RALPH_FIX_PROGRESS_AUTOLOG_END)
        {
            contents.push_str("\n\n");
            contents.push_str(RALPH_FIX_PROGRESS_AUTOLOG_START);
            contents.push('\n');
            contents.push('\n');
            contents.push_str(RALPH_FIX_PROGRESS_AUTOLOG_END);
            contents.push('\n');
        }

        let (before, after) = match contents.split_once(RALPH_FIX_PROGRESS_AUTOLOG_START) {
            Some((before, rest)) => match rest.split_once(RALPH_FIX_PROGRESS_AUTOLOG_END) {
                Some((_old, after)) => (before.to_string(), after.to_string()),
                None => (contents.clone(), String::new()),
            },
            None => (contents.clone(), String::new()),
        };

        let events = read_ralph_fix_progress_events(ralph_dir);
        let autolog = render_ralph_fix_progress_autolog(&events);
        let new_contents = format!(
            "{before}{RALPH_FIX_PROGRESS_AUTOLOG_START}\n{autolog}\n{RALPH_FIX_PROGRESS_AUTOLOG_END}{after}"
        );

        // Optimistic concurrency: if the user/agent is appending Notes while we're regenerating the
        // Auto log section, retry instead of overwriting their changes.
        let current = std::fs::read_to_string(&md_path).unwrap_or_default();
        if current != original {
            if attempt + 1 < RALPH_FIX_PROGRESS_REGENERATE_MAX_ATTEMPTS {
                std::thread::sleep(Duration::from_millis(5));
                continue;
            }
            return Err(std::io::Error::other(
                "failed to regenerate @fix_progress.md: file changed during update",
            ));
        }

        codex_utils_path::write_atomically(&md_path, &new_contents)?;
        return Ok(());
    }

    Err(std::io::Error::other(
        "failed to regenerate @fix_progress.md: too many retries",
    ))
}

fn read_ralph_fix_progress_events(ralph_dir: &Path) -> Vec<RalphFixProgressEvent> {
    let autolog_path = ralph_dir.join(RALPH_FIX_PROGRESS_AUTOLOG_JSONL);
    let Ok(contents) = std::fs::read_to_string(autolog_path) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(ev) = serde_json::from_str::<RalphFixProgressEvent>(line) {
            out.push(ev);
        }
    }
    out
}

fn render_ralph_fix_progress_autolog(events: &[RalphFixProgressEvent]) -> String {
    let mut out = String::new();
    out.push_str("## Auto log (managed by ralph-widex; do not edit)\n\n");

    let start = events
        .len()
        .saturating_sub(RALPH_FIX_PROGRESS_AUTOLOG_MAX_EVENTS);
    for ev in &events[start..] {
        match ev.kind {
            RalphFixProgressEventKind::Start => {
                let ts = ev.ts.as_str();
                let loop_number = ev.loop_number;
                out.push_str(&format!("- [{ts}] Loop {loop_number} start\n"));
            }
            RalphFixProgressEventKind::End => {
                let ts = ev.ts.as_str();
                let loop_number = ev.loop_number;
                let completion_seen = ev.completion_seen.unwrap_or(false);
                let stop_reason = ev.stop_reason.as_deref().unwrap_or("");
                let interrupted = ev.interrupted.unwrap_or(false);
                let abort_reason = ev.abort_reason.as_deref().unwrap_or("");
                let timed_out = ev.timed_out.unwrap_or(false);
                out.push_str(&format!(
                    "- [{ts}] Loop {loop_number} end: completion_seen={completion_seen} stop_reason={stop_reason} interrupted={interrupted} abort_reason={abort_reason} timed_out={timed_out}\n"
                ));
            }
        }
    }

    out
}

#[derive(Debug, serde::Serialize)]
struct RalphTuiStatusFile {
    timestamp: String,
    mode: String,
    loop_current: u64,
    max_loops: u64,
    in_flight: bool,
    loop_count: u64,
    calls_made_this_hour: u64,
    max_calls_per_hour: u64,
    next_reset_in_seconds: u64,
    timeout_minutes: u64,
    completion_mode: String,
    completion_phrases: Vec<String>,
    completion_regexes: Vec<String>,
    last_abort_reason: Option<String>,
    timed_out: bool,
    last_action: String,
    status: String,
    exit_reason: String,
    next_reset: String,
}

fn next_hour_boundary_string() -> String {
    let now = Local::now();
    let next = (now + chrono::Duration::hours(1))
        .with_minute(0)
        .and_then(|t| t.with_second(0))
        .and_then(|t| t.with_nanosecond(0))
        .unwrap_or_else(|| now + chrono::Duration::hours(1));
    next.format("%H:%M:%S").to_string()
}

fn seconds_until_next_hour_boundary() -> u64 {
    let now = Local::now();
    let next = (now + chrono::Duration::hours(1))
        .with_minute(0)
        .and_then(|t| t.with_second(0))
        .and_then(|t| t.with_nanosecond(0))
        .unwrap_or_else(|| now + chrono::Duration::hours(1));

    (next - now)
        .to_std()
        .unwrap_or_else(|_| std::time::Duration::from_secs(0))
        .as_secs()
}

fn write_ralph_tui_status(
    path: &Path,
    state: &RalphTuiState,
    last_action: &str,
    status: &str,
    exit_reason: &str,
) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let file = RalphTuiStatusFile {
        timestamp: chrono::Utc::now().to_rfc3339(),
        mode: "tui".to_string(),
        loop_current: state.current_loop,
        max_loops: state.max_loops,
        in_flight: state.in_flight,
        loop_count: state.current_loop,
        calls_made_this_hour: state.calls_made_in_window,
        max_calls_per_hour: state.max_calls_per_hour.unwrap_or(0),
        next_reset_in_seconds: seconds_until_next_hour_boundary(),
        timeout_minutes: state.timeout_minutes.unwrap_or(0),
        completion_mode: match state.completion_mode {
            RalphCompletionMode::Contains => "contains",
            RalphCompletionMode::PromiseTag => "promise-tag",
            RalphCompletionMode::Regex => "regex",
        }
        .to_string(),
        completion_phrases: state.completion_phrases.clone(),
        completion_regexes: state.completion_regex_patterns.clone(),
        last_abort_reason: state.last_abort_reason.clone(),
        timed_out: state.turn_timed_out,
        last_action: last_action.to_string(),
        status: status.to_string(),
        exit_reason: exit_reason.to_string(),
        next_reset: next_hour_boundary_string(),
    };

    let data = serde_json::to_vec_pretty(&file).map_err(std::io::Error::other)?;
    let tmp_path = path.with_extension("json.tmp");
    std::fs::write(&tmp_path, [&data[..], b"\n"].concat())?;
    std::fs::rename(&tmp_path, path)?;
    Ok(())
}

fn append_ralph_tui_log_line(path: &Path, level: &str, message: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let ts = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let line = format!("[{ts}] [{level}] {message}\n");
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(line.as_bytes())?;
    Ok(())
}

fn ralph_widex_command_for_current_exe(current_exe: Option<PathBuf>) -> String {
    current_exe
        .filter(|path| {
            path.file_stem()
                .and_then(|name| name.to_str())
                .is_some_and(|name| matches!(name, "codex" | "widex"))
        })
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "codex".to_string())
}

fn parse_ralph_tui_args(args: &[String]) -> RalphTuiConfig {
    let mut loops: Option<u64> = None;
    let mut completion_phrases: Vec<String> = Vec::new();
    let mut completion_mode = RalphCompletionMode::Contains;
    let mut completion_regexes: Vec<String> = Vec::new();
    let mut max_calls_per_hour: Option<u64> = None;
    let mut timeout_minutes: Option<u64> = None;
    let mut skip_git_repo_check = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--loops" => {
                if let Some(val) = args.get(i + 1) {
                    if let Ok(n) = val.parse::<u64>() {
                        loops = Some(n)
                    }
                    i += 2;
                    continue;
                }
            }
            "--completion-phrase" => {
                if let Some(val) = args.get(i + 1) {
                    completion_phrases.push(val.to_string());
                    i += 2;
                    continue;
                }
            }
            "--completion-mode" => {
                if let Some(val) = args.get(i + 1)
                    && let Some(mode) = RalphCompletionMode::parse(val)
                {
                    completion_mode = mode;
                    i += 2;
                    continue;
                }
            }
            "--completion-regex" => {
                if let Some(val) = args.get(i + 1) {
                    completion_regexes.push(val.to_string());
                    i += 2;
                    continue;
                }
            }
            "--calls" => {
                if let Some(val) = args.get(i + 1) {
                    if let Ok(n) = val.parse::<u64>() {
                        max_calls_per_hour = Some(n);
                    }
                    i += 2;
                    continue;
                }
            }
            "--timeout-minutes" => {
                if let Some(val) = args.get(i + 1) {
                    if let Ok(n) = val.parse::<u64>() {
                        timeout_minutes = Some(n);
                    }
                    i += 2;
                    continue;
                }
            }
            "--skip-git-repo-check" => {
                skip_git_repo_check = true;
                i += 1;
                continue;
            }
            other => {
                let _ = other;
            }
        }
        i += 1;
    }

    let loops = loops.unwrap_or(widex_overlay::DEFAULT_TUI_LOOPS);
    if completion_phrases.is_empty() {
        completion_phrases.push(widex_overlay::DEFAULT_TUI_COMPLETION_PHRASE.to_string());
    }
    RalphTuiConfig {
        loops,
        completion_phrases,
        completion_mode,
        completion_regexes,
        max_calls_per_hour,
        timeout_minutes,
        skip_git_repo_check,
    }
}

fn completion_signal_seen(
    message: Option<&str>,
    mode: RalphCompletionMode,
    phrases: &[String],
    regexes: &[regex_lite::Regex],
) -> bool {
    let Some(message) = message else {
        return false;
    };
    let message = message.trim();
    if message.is_empty() {
        return false;
    }

    match mode {
        RalphCompletionMode::Contains => {
            if phrases.is_empty() {
                return false;
            }
            // ASCII-only lowercasing avoids Unicode case-folding quirks while still keeping
            // non-ASCII completion phrases (e.g. Chinese) stable.
            let message_lower = message.to_ascii_lowercase();
            phrases.iter().any(|p| {
                let p = p.trim();
                !p.is_empty() && message_lower.contains(&p.to_ascii_lowercase())
            })
        }
        RalphCompletionMode::PromiseTag => completion_promise_tag_seen(message, phrases),
        RalphCompletionMode::Regex => regexes.iter().any(|re| re.is_match(message)),
    }
}

fn completion_promise_tag_seen(message: &str, phrases: &[String]) -> bool {
    if phrases.is_empty() {
        return false;
    }

    const OPEN: &str = "<promise>";
    const CLOSE: &str = "</promise>";

    let mut rest = message;
    while let Some(start) = rest.find(OPEN) {
        let after_open = &rest[start + OPEN.len()..];
        let Some(end) = after_open.find(CLOSE) else {
            return false;
        };
        let inner = after_open[..end].trim();
        if !inner.is_empty() && phrases.iter().any(|p| p.trim() == inner) {
            return true;
        }
        rest = &after_open[end + CLOSE.len()..];
    }
    false
}
