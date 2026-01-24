use crate::ralph_storage::RalphPaths;
use crate::ralph_storage::write_json_atomic;
use anyhow::Context;
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;

const NO_PROGRESS_THRESHOLD: u64 = 3;
const SAME_ERROR_THRESHOLD: u64 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum CircuitState {
    Closed,
    HalfOpen,
    Open,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CircuitBreakerState {
    pub(crate) state: CircuitState,
    pub(crate) last_change: String,
    pub(crate) consecutive_no_progress: u64,
    pub(crate) consecutive_same_error: u64,
    pub(crate) last_progress_loop: u64,
    pub(crate) total_opens: u64,
    pub(crate) reason: String,
    pub(crate) current_loop: u64,
}

impl Default for CircuitBreakerState {
    fn default() -> Self {
        Self {
            state: CircuitState::Closed,
            last_change: Utc::now().to_rfc3339(),
            consecutive_no_progress: 0,
            consecutive_same_error: 0,
            last_progress_loop: 0,
            total_opens: 0,
            reason: String::new(),
            current_loop: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CircuitTransition {
    timestamp: String,
    loop_number: u64,
    from_state: CircuitState,
    to_state: CircuitState,
    reason: String,
}

pub(crate) async fn ensure_initialized(paths: &RalphPaths) -> anyhow::Result<()> {
    if !tokio::fs::try_exists(&paths.circuit_breaker_state_file).await? {
        write_json_atomic(
            &paths.circuit_breaker_state_file,
            &CircuitBreakerState::default(),
        )
        .await?;
    }
    if !tokio::fs::try_exists(&paths.circuit_breaker_history_file).await? {
        write_json_atomic(
            &paths.circuit_breaker_history_file,
            &Vec::<CircuitTransition>::new(),
        )
        .await?;
    }
    Ok(())
}

pub(crate) async fn read_state(paths: &RalphPaths) -> anyhow::Result<CircuitBreakerState> {
    ensure_initialized(paths).await?;
    let bytes = tokio::fs::read(&paths.circuit_breaker_state_file)
        .await
        .with_context(|| {
            format!(
                "Failed to read {}",
                paths.circuit_breaker_state_file.display()
            )
        })?;
    serde_json::from_slice(&bytes).context("Failed to parse circuit breaker state")
}

pub(crate) async fn can_execute(paths: &RalphPaths) -> anyhow::Result<bool> {
    Ok(read_state(paths).await?.state != CircuitState::Open)
}

pub(crate) struct RecordOutcome {
    pub(crate) state: CircuitBreakerState,
    pub(crate) opened: bool,
}

pub(crate) async fn record_loop_result(
    paths: &RalphPaths,
    loop_number: u64,
    files_changed: u64,
    has_errors: bool,
) -> anyhow::Result<RecordOutcome> {
    let mut state = read_state(paths).await?;
    let current_state = state.state;

    let has_progress = files_changed > 0;
    if has_progress {
        state.consecutive_no_progress = 0;
        state.last_progress_loop = loop_number;
    } else {
        state.consecutive_no_progress = state.consecutive_no_progress.saturating_add(1);
    }

    if has_errors {
        state.consecutive_same_error = state.consecutive_same_error.saturating_add(1);
    } else {
        state.consecutive_same_error = 0;
    }

    state.current_loop = loop_number;

    let mut new_state = current_state;
    let mut reason = String::new();

    match current_state {
        CircuitState::Closed => {
            if state.consecutive_no_progress >= NO_PROGRESS_THRESHOLD {
                new_state = CircuitState::Open;
                reason = format!(
                    "No progress detected in {} consecutive loops",
                    state.consecutive_no_progress
                );
            } else if state.consecutive_same_error >= SAME_ERROR_THRESHOLD {
                new_state = CircuitState::Open;
                reason = format!(
                    "Same error repeated in {} consecutive loops",
                    state.consecutive_same_error
                );
            } else if state.consecutive_no_progress >= 2 {
                new_state = CircuitState::HalfOpen;
                reason = format!(
                    "Monitoring: {} loops without progress",
                    state.consecutive_no_progress
                );
            }
        }
        CircuitState::HalfOpen => {
            if has_progress {
                new_state = CircuitState::Closed;
                reason = "Progress detected, circuit recovered".to_string();
            } else if state.consecutive_no_progress >= NO_PROGRESS_THRESHOLD {
                new_state = CircuitState::Open;
                reason = format!(
                    "No recovery, opening circuit after {} loops",
                    state.consecutive_no_progress
                );
            }
        }
        CircuitState::Open => {
            new_state = CircuitState::Open;
            reason = "Circuit breaker is open, execution halted".to_string();
        }
    }

    let opened = new_state == CircuitState::Open && current_state != CircuitState::Open;
    if opened {
        state.total_opens = state.total_opens.saturating_add(1);
    }

    state.state = new_state;
    state.reason = reason.clone();
    state.last_change = Utc::now().to_rfc3339();

    write_json_atomic(&paths.circuit_breaker_state_file, &state).await?;
    if new_state != current_state {
        append_transition(paths, current_state, new_state, &reason, loop_number).await?;
    }

    Ok(RecordOutcome { state, opened })
}

async fn append_transition(
    paths: &RalphPaths,
    from_state: CircuitState,
    to_state: CircuitState,
    reason: &str,
    loop_number: u64,
) -> anyhow::Result<()> {
    ensure_initialized(paths).await?;
    let bytes = tokio::fs::read(&paths.circuit_breaker_history_file)
        .await
        .unwrap_or_default();
    let mut history: Vec<CircuitTransition> = serde_json::from_slice(&bytes).unwrap_or_default();

    history.push(CircuitTransition {
        timestamp: Utc::now().to_rfc3339(),
        loop_number,
        from_state,
        to_state,
        reason: reason.to_string(),
    });

    write_json_atomic(&paths.circuit_breaker_history_file, &history).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    #[tokio::test]
    async fn opens_after_three_no_progress_loops() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let paths = RalphPaths::new(dir.path());
        paths.ensure_dirs().await?;

        let r1 = record_loop_result(&paths, 1, 0, false).await?;
        assert_eq!(r1.state.state, CircuitState::Closed);
        let r2 = record_loop_result(&paths, 2, 0, false).await?;
        assert_eq!(r2.state.state, CircuitState::HalfOpen);
        let r3 = record_loop_result(&paths, 3, 0, false).await?;
        assert_eq!(r3.state.state, CircuitState::Open);
        assert_eq!(r3.opened, true);

        Ok(())
    }
}
