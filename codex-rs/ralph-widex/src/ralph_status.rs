use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LoopCompletionStatus {
    InProgress,
    Complete,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TestsStatus {
    Passing,
    Failing,
    NotRun,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WorkType {
    Implementation,
    Testing,
    Documentation,
    Refactoring,
    Debugging,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RalphStatus {
    pub status: LoopCompletionStatus,
    pub tasks_completed_this_loop: u64,
    pub files_modified: u64,
    pub tests_status: TestsStatus,
    pub work_type: WorkType,
    pub exit_signal: bool,
    pub recommendation: String,
}

impl RalphStatus {
    pub fn gate_satisfied(&self) -> bool {
        self.exit_signal && self.status == LoopCompletionStatus::Complete
    }
}

pub fn parse_ralph_status_from_text(text: &str) -> Option<RalphStatus> {
    let block = extract_status_block(text)?;
    parse_key_values(&block)
}

fn extract_status_block(text: &str) -> Option<String> {
    let start = text.find("---RALPH_STATUS---")?;
    let after_start = &text[start + "---RALPH_STATUS---".len()..];
    let end_rel = after_start.find("---END_RALPH_STATUS---")?;
    Some(after_start[..end_rel].to_string())
}

fn parse_key_values(block: &str) -> Option<RalphStatus> {
    let mut status: Option<LoopCompletionStatus> = None;
    let mut tasks_completed: Option<u64> = None;
    let mut files_modified: Option<u64> = None;
    let mut tests_status: Option<TestsStatus> = None;
    let mut work_type: Option<WorkType> = None;
    let mut exit_signal: Option<bool> = None;
    let mut recommendation: Option<String> = None;

    for raw_line in block.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }

        let (k, v) = match line.split_once(':') {
            Some((k, v)) => (k.trim(), v.trim()),
            None => continue,
        };

        match k.to_ascii_uppercase().as_str() {
            "STATUS" => {
                status = parse_enum(v);
            }
            "TASKS_COMPLETED_THIS_LOOP" => {
                tasks_completed = v.parse().ok();
            }
            "FILES_MODIFIED" => {
                files_modified = v.parse().ok();
            }
            "TESTS_STATUS" => {
                tests_status = parse_enum(v);
            }
            "WORK_TYPE" => {
                work_type = parse_enum(v);
            }
            "EXIT_SIGNAL" => {
                exit_signal = parse_bool(v);
            }
            "RECOMMENDATION" => {
                recommendation = Some(v.to_string());
            }
            _ => {}
        }
    }

    Some(RalphStatus {
        status: status?,
        tasks_completed_this_loop: tasks_completed.unwrap_or(0),
        files_modified: files_modified.unwrap_or(0),
        tests_status: tests_status.unwrap_or(TestsStatus::NotRun),
        work_type: work_type.unwrap_or(WorkType::Implementation),
        exit_signal: exit_signal.unwrap_or(false),
        recommendation: recommendation.unwrap_or_default(),
    })
}

fn parse_bool(v: &str) -> Option<bool> {
    match v.trim().to_ascii_lowercase().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn parse_enum<T>(v: &str) -> Option<T>
where
    for<'de> T: Deserialize<'de>,
{
    let json = format!("\"{}\"", v.trim().to_ascii_uppercase());
    serde_json::from_str(&json).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn parses_status_block() {
        let text = r#"
hello

---RALPH_STATUS---
STATUS: COMPLETE
TASKS_COMPLETED_THIS_LOOP: 2
FILES_MODIFIED: 3
TESTS_STATUS: PASSING
WORK_TYPE: IMPLEMENTATION
EXIT_SIGNAL: true
RECOMMENDATION: ship it
---END_RALPH_STATUS---
"#;

        let status = parse_ralph_status_from_text(text).expect("status");
        assert_eq!(status.status, LoopCompletionStatus::Complete);
        assert_eq!(status.exit_signal, true);
        assert_eq!(status.files_modified, 3);
        assert_eq!(status.tests_status, TestsStatus::Passing);
        assert_eq!(status.work_type, WorkType::Implementation);
        assert_eq!(status.recommendation, "ship it");
    }
}
