use std::path::Path;
use std::path::PathBuf;

use codex_shell_command::parse_command::extract_shell_command;
use dirs::home_dir;
use shlex::try_join;

pub(crate) fn escape_command(command: &[String]) -> String {
    try_join(command.iter().map(String::as_str)).unwrap_or_else(|_| command.join(" "))
}

pub(crate) fn strip_bash_lc_and_escape(command: &[String]) -> String {
    let display = if let Some((_, script)) = extract_shell_command(command) {
        script.to_string()
    } else {
        escape_command(command)
    };
    canonicalize_tui_command_display(display)
}

fn canonicalize_tui_command_display(display: String) -> String {
    let Some(parts) = shlex::split(&display) else {
        return display;
    };
    let Some((program, rest)) = parts.split_first() else {
        return display;
    };
    let Some((subcommand, args)) = rest.split_first() else {
        return display;
    };
    if subcommand != "ralph-widex" {
        return display;
    }

    let Some(file_name) = Path::new(program)
        .file_name()
        .and_then(|name| name.to_str())
    else {
        return display;
    };
    if !matches!(file_name, "widex" | "codex") {
        return display;
    }

    let mut normalized = vec!["/ralph-widex".to_string()];
    normalized.extend(args.iter().cloned());
    escape_command(&normalized)
}

pub(crate) fn split_command_string(command: &str) -> Vec<String> {
    let Some(parts) = shlex::split(command) else {
        return vec![command.to_string()];
    };
    match shlex::try_join(parts.iter().map(String::as_str)) {
        Ok(round_trip)
            if round_trip == command
                || (!command.contains(":\\")
                    && shlex::split(&round_trip).as_ref() == Some(&parts)) =>
        {
            parts
        }
        _ => vec![command.to_string()],
    }
}

/// If `path` is absolute and inside $HOME, return the part *after* the home
/// directory; otherwise, return the path as-is. Note if `path` is the homedir,
/// this will return and empty path.
pub(crate) fn relativize_to_home<P>(path: P) -> Option<PathBuf>
where
    P: AsRef<Path>,
{
    let path = path.as_ref();
    if !path.is_absolute() {
        // If the path is not absolute, we can’t do anything with it.
        return None;
    }

    let home_dir = home_dir()?;
    let rel = path.strip_prefix(&home_dir).ok()?;
    Some(rel.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_command() {
        let args = vec!["foo".into(), "bar baz".into(), "weird&stuff".into()];
        let cmdline = escape_command(&args);
        assert_eq!(cmdline, "foo 'bar baz' 'weird&stuff'");
    }

    #[test]
    fn test_strip_bash_lc_and_escape() {
        // Test bash
        let args = vec!["bash".into(), "-lc".into(), "echo hello".into()];
        let cmdline = strip_bash_lc_and_escape(&args);
        assert_eq!(cmdline, "echo hello");

        // Test zsh
        let args = vec!["zsh".into(), "-lc".into(), "echo hello".into()];
        let cmdline = strip_bash_lc_and_escape(&args);
        assert_eq!(cmdline, "echo hello");

        // Test absolute path to zsh
        let args = vec!["/usr/bin/zsh".into(), "-lc".into(), "echo hello".into()];
        let cmdline = strip_bash_lc_and_escape(&args);
        assert_eq!(cmdline, "echo hello");

        // Test absolute path to bash
        let args = vec!["/bin/bash".into(), "-lc".into(), "echo hello".into()];
        let cmdline = strip_bash_lc_and_escape(&args);
        assert_eq!(cmdline, "echo hello");
    }

    #[test]
    fn strip_bash_lc_and_escape_normalizes_ralph_widex_commands() {
        let args = vec!["bash".into(), "-lc".into(), "widex ralph-widex init".into()];
        let cmdline = strip_bash_lc_and_escape(&args);
        assert_eq!(cmdline, "/ralph-widex init");

        let args = vec![
            "codex".into(),
            "ralph-widex".into(),
            "start".into(),
            "--loops".into(),
            "5".into(),
        ];
        let cmdline = strip_bash_lc_and_escape(&args);
        assert_eq!(cmdline, "/ralph-widex start --loops 5");

        let args = vec![
            "/home/will/.local/bin/widex".into(),
            "ralph-widex".into(),
            "init".into(),
            "--overwrite".into(),
        ];
        let cmdline = strip_bash_lc_and_escape(&args);
        assert_eq!(cmdline, "/ralph-widex init --overwrite");
    }

    #[test]
    fn split_command_string_round_trips_shell_wrappers() {
        let command =
            shlex::try_join(["/bin/zsh", "-lc", r#"python3 -c 'print("Hello, world!")'"#])
                .expect("round-trippable command");
        assert_eq!(
            split_command_string(&command),
            vec![
                "/bin/zsh".to_string(),
                "-lc".to_string(),
                r#"python3 -c 'print("Hello, world!")'"#.to_string(),
            ]
        );
    }

    #[test]
    fn split_command_string_preserves_non_roundtrippable_windows_commands() {
        let command = r#"C:\Program Files\Git\bin\bash.exe -lc "echo hi""#;
        assert_eq!(split_command_string(command), vec![command.to_string()]);
    }
}
