use crate::CompletionMode;
use std::path::Path;

pub const DEFAULT_TUI_LOOPS: u64 = 20;
pub const DEFAULT_TUI_COMPLETION_PHRASE: &str = "所有任务已完成";
pub const FIX_PROGRESS_AUTOLOG_START: &str = "<!-- RALPH_WIDEX_AUTOLOG_START -->";
pub const FIX_PROGRESS_AUTOLOG_END: &str = "<!-- RALPH_WIDEX_AUTOLOG_END -->";

const FIX_PROGRESS_TEMPLATE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../widex-custom/features/ralph-widex/templates/fix_progress.md"
));
const TUI_HELP_TEXT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../widex-custom/features/ralph-widex/overlay/tui_help.txt"
));
const TUI_LOOP_PROMPT_TEMPLATE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../widex-custom/features/ralph-widex/overlay/tui_loop_prompt.txt"
));

pub fn fix_progress_template() -> &'static str {
    FIX_PROGRESS_TEMPLATE
}

pub fn tui_help_text() -> &'static str {
    TUI_HELP_TEXT.trim_end()
}

pub fn ensure_fix_progress_file(ralph_dir: &Path) -> std::io::Result<()> {
    let md_path = ralph_dir.join("@fix_progress.md");
    if md_path.exists()
        && let Ok(existing) = std::fs::read_to_string(&md_path)
        && !existing.trim().is_empty()
    {
        return Ok(());
    }

    std::fs::write(md_path, fix_progress_template())
}

pub fn render_tui_loop_prompt(
    current_loop: u64,
    max_loops: u64,
    completion_mode: CompletionMode,
    completion_phrases: &[String],
    completion_regexes: &[String],
) -> String {
    let max_loops = if max_loops == 0 {
        "infinite".to_string()
    } else {
        max_loops.to_string()
    };
    let completion_instruction =
        render_completion_instruction(completion_mode, completion_phrases, completion_regexes);

    TUI_LOOP_PROMPT_TEMPLATE
        .replace("{current_loop}", &current_loop.to_string())
        .replace("{max_loops}", &max_loops)
        .replace("{completion_instruction}", &completion_instruction)
}

pub fn render_completion_instruction(
    completion_mode: CompletionMode,
    completion_phrases: &[String],
    completion_regexes: &[String],
) -> String {
    match completion_mode {
        CompletionMode::Contains => {
            if completion_phrases.is_empty() {
                String::new()
            } else {
                let phrases = completion_phrases.join(" | ");
                format!(
                    "If all tasks are complete, print one of these completion phrases in your FINAL assistant message (exact text):\n{phrases}\n"
                )
            }
        }
        CompletionMode::PromiseTag => {
            if completion_phrases.is_empty() {
                String::new()
            } else {
                let tags = completion_phrases
                    .iter()
                    .map(|phrase| {
                        let phrase = phrase.trim();
                        format!("<promise>{phrase}</promise>")
                    })
                    .collect::<Vec<_>>()
                    .join(" | ");
                format!(
                    "If all tasks are complete, print ONE of these completion promise tags in your FINAL assistant message (exact text):\n{tags}\n"
                )
            }
        }
        CompletionMode::Regex => {
            if completion_regexes.is_empty() {
                String::new()
            } else {
                let patterns = completion_regexes.join(" | ");
                format!(
                    "If all tasks are complete, ensure your FINAL assistant message matches one of these regex patterns:\n{patterns}\n"
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn loop_prompt_uses_overlay_assets() {
        let prompt = render_tui_loop_prompt(
            2,
            8,
            CompletionMode::PromiseTag,
            &["任务完成".to_string()],
            &[],
        );

        assert!(prompt.contains("Ralph-Widex loop 2/8."));
        assert!(prompt.contains(".ralph/@fix_progress.md"));
        assert!(prompt.contains("<promise>任务完成</promise>"));
    }

    #[test]
    fn ensure_fix_progress_file_recovers_empty_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ralph_dir = tmp.path().join(".ralph");
        std::fs::create_dir_all(&ralph_dir).expect("create .ralph");
        std::fs::write(ralph_dir.join("@fix_progress.md"), "").expect("write empty file");

        ensure_fix_progress_file(&ralph_dir).expect("recover template");
        let contents =
            std::fs::read_to_string(ralph_dir.join("@fix_progress.md")).expect("read template");
        assert!(contents.contains("## Notes (editable)"));
        assert!(contents.contains(FIX_PROGRESS_AUTOLOG_START));
        assert!(contents.contains(FIX_PROGRESS_AUTOLOG_END));
    }

    #[test]
    fn tui_help_text_matches_overlay_copy() {
        assert_eq!(tui_help_text().lines().next(), Some("Ralph-Widex (TUI)"));
        assert!(tui_help_text().contains("/ralph-widex init [--overwrite]"));
        assert!(tui_help_text().contains("/ralph-widex status"));
    }
}
