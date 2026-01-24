use crate::ralph_status::parse_ralph_status_from_text;
use serde::Deserialize;
use serde::Serialize;

const COMPLETION_KEYWORDS: [&str; 6] = [
    "done",
    "complete",
    "finished",
    "all tasks complete",
    "project complete",
    "ready for review",
];

const NO_WORK_PATTERNS: [&str; 4] = [
    "nothing to do",
    "no changes",
    "already implemented",
    "up to date",
];

const TEST_ONLY_PATTERNS: [&str; 7] = [
    "npm test",
    "bats",
    "pytest",
    "jest",
    "cargo test",
    "go test",
    "running tests",
];

#[derive(Debug, Clone, Serialize)]
pub(crate) struct AnalysisFile {
    pub(crate) loop_number: u64,
    pub(crate) timestamp: String,
    pub(crate) output_file: String,
    pub(crate) output_format: String,
    pub(crate) analysis: Analysis,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct Analysis {
    pub(crate) has_completion_signal: bool,
    pub(crate) is_test_only: bool,
    pub(crate) is_stuck: bool,
    pub(crate) has_progress: bool,
    pub(crate) files_modified: u64,
    pub(crate) confidence_score: u64,
    pub(crate) exit_signal: bool,
    pub(crate) work_summary: String,
    pub(crate) output_length: u64,
    pub(crate) error_count: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct LoopSignals {
    pub(crate) has_completion_signal: bool,
    pub(crate) exit_signal: bool,
    pub(crate) is_test_only: bool,
    pub(crate) is_stuck: bool,
    pub(crate) confidence_score: u64,
    pub(crate) work_summary: String,
}

pub(crate) fn analyze_last_message(
    last_message: Option<&str>,
    files_modified: u64,
    error_count: u64,
) -> LoopSignals {
    let Some(text) = last_message else {
        return LoopSignals {
            has_completion_signal: false,
            exit_signal: false,
            is_test_only: false,
            is_stuck: false,
            confidence_score: 0,
            work_summary: "No last message".to_string(),
        };
    };

    if let Ok(status) = serde_json::from_str::<crate::ralph_status::RalphStatus>(text) {
        let gate = status.gate_satisfied();
        return LoopSignals {
            has_completion_signal: gate,
            exit_signal: gate,
            is_test_only: false,
            is_stuck: error_count > 5,
            confidence_score: 100,
            work_summary: "Structured Ralph status".to_string(),
        };
    }

    if let Some(status) = parse_ralph_status_from_text(text) {
        let gate = status.gate_satisfied();
        return LoopSignals {
            has_completion_signal: gate,
            exit_signal: gate,
            is_test_only: false,
            is_stuck: error_count > 5,
            confidence_score: 100,
            work_summary: "RALPH_STATUS block".to_string(),
        };
    }

    let lower = text.to_ascii_lowercase();

    let mut has_completion_signal = false;
    let mut confidence_score: u64 = 0;
    for kw in COMPLETION_KEYWORDS {
        if lower.contains(kw) {
            has_completion_signal = true;
            confidence_score = confidence_score.saturating_add(10);
            break;
        }
    }

    let mut no_work = false;
    for pat in NO_WORK_PATTERNS {
        if lower.contains(pat) {
            has_completion_signal = true;
            no_work = true;
            confidence_score = confidence_score.saturating_add(15);
            break;
        }
    }

    let test_commands = TEST_ONLY_PATTERNS
        .iter()
        .filter(|p| lower.contains(**p))
        .count() as u64;
    let likely_impl = [
        "implement",
        "creating",
        "writing",
        "adding",
        "function",
        "class",
    ]
    .iter()
    .any(|p| lower.contains(p));

    let is_test_only = test_commands > 0 && !likely_impl;
    let is_stuck = error_count > 5;

    // Conservative heuristic: only allow heuristic exit when we see an explicit "no work" pattern
    // and there were no file changes this loop.
    let exit_signal = no_work && files_modified == 0 && error_count == 0;

    let work_summary = if no_work {
        "No work remaining".to_string()
    } else if is_test_only {
        "Test execution only, no implementation".to_string()
    } else {
        "Output analyzed, no explicit status found".to_string()
    };

    LoopSignals {
        has_completion_signal,
        exit_signal,
        is_test_only,
        is_stuck,
        confidence_score,
        work_summary,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct ExitSignalsFile {
    pub(crate) test_only_loops: Vec<u64>,
    pub(crate) done_signals: Vec<u64>,
    pub(crate) completion_indicators: Vec<u64>,
}

impl ExitSignalsFile {
    pub(crate) fn update_for_loop(&mut self, loop_number: u64, analysis: &Analysis) {
        if analysis.is_test_only {
            self.test_only_loops.push(loop_number);
        } else if analysis.has_progress {
            self.test_only_loops.clear();
        }

        if analysis.has_completion_signal {
            self.done_signals.push(loop_number);
        }

        if analysis.confidence_score >= 60 {
            self.completion_indicators.push(loop_number);
        }

        truncate_signal_history(&mut self.test_only_loops);
        truncate_signal_history(&mut self.done_signals);
        truncate_signal_history(&mut self.completion_indicators);
    }
}

fn truncate_signal_history(values: &mut Vec<u64>) {
    const MAX: usize = 20;
    if values.len() > MAX {
        values.drain(0..values.len().saturating_sub(MAX));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn parses_ralph_status_block_sets_exit() {
        let text = r#"
---RALPH_STATUS---
STATUS: COMPLETE
TASKS_COMPLETED_THIS_LOOP: 1
FILES_MODIFIED: 0
TESTS_STATUS: NOT_RUN
WORK_TYPE: IMPLEMENTATION
EXIT_SIGNAL: true
RECOMMENDATION: done
---END_RALPH_STATUS---
"#;
        let sig = analyze_last_message(Some(text), 0, 0);
        assert_eq!(sig.exit_signal, true);
        assert_eq!(sig.confidence_score, 100);
    }
}
