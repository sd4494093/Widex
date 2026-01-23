use std::fs;
use std::io;
use std::path::Path;
use std::path::PathBuf;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

const INSTALL_VERSION: &str = "0.1.0";

const BIN_RALPH_WIDEX: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../widex-custom/features/ralph-widex/bin/ralph-widex"
));
const BIN_RALPH_WIDEX_MONITOR: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../widex-custom/features/ralph-widex/bin/ralph-widex-monitor"
));

const LIB_DATE_UTILS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../widex-custom/features/ralph-widex/lib/date_utils.sh"
));
const LIB_TIMEOUT_UTILS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../widex-custom/features/ralph-widex/lib/timeout_utils.sh"
));
const LIB_RESPONSE_ANALYZER: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../widex-custom/features/ralph-widex/lib/response_analyzer.sh"
));
const LIB_CIRCUIT_BREAKER: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../widex-custom/features/ralph-widex/lib/circuit_breaker.sh"
));

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

const FEATURE_README: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../widex-custom/features/ralph-widex/README.md"
));
const UPSTREAM_LICENSE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../widex-custom/features/ralph-widex/LICENSE.upstream"
));

pub(crate) fn install_dir(codex_home: &Path) -> PathBuf {
    codex_home.join("features").join("ralph-widex")
}

pub(crate) fn ensure_installed(codex_home: &Path) -> io::Result<PathBuf> {
    let root = install_dir(codex_home);
    let version_file = root.join("VERSION");

    if let Ok(existing) = fs::read_to_string(&version_file)
        && existing.trim() == INSTALL_VERSION
    {
        return Ok(root);
    }

    fs::create_dir_all(root.join("bin"))?;
    fs::create_dir_all(root.join("lib"))?;
    fs::create_dir_all(root.join("templates").join("specs"))?;

    write_file(&root.join("VERSION"), format!("{INSTALL_VERSION}\n"))?;

    write_file(&root.join("README.md"), FEATURE_README)?;
    write_file(&root.join("LICENSE.upstream"), UPSTREAM_LICENSE)?;

    write_file(&root.join("bin").join("ralph-widex"), BIN_RALPH_WIDEX)?;
    write_file(
        &root.join("bin").join("ralph-widex-monitor"),
        BIN_RALPH_WIDEX_MONITOR,
    )?;

    write_file(&root.join("lib").join("date_utils.sh"), LIB_DATE_UTILS)?;
    write_file(
        &root.join("lib").join("timeout_utils.sh"),
        LIB_TIMEOUT_UTILS,
    )?;
    write_file(
        &root.join("lib").join("response_analyzer.sh"),
        LIB_RESPONSE_ANALYZER,
    )?;
    write_file(
        &root.join("lib").join("circuit_breaker.sh"),
        LIB_CIRCUIT_BREAKER,
    )?;

    write_file(&root.join("templates").join("PROMPT.md"), TEMPLATE_PROMPT)?;
    write_file(
        &root.join("templates").join("fix_plan.md"),
        TEMPLATE_FIX_PLAN,
    )?;
    write_file(&root.join("templates").join("AGENT.md"), TEMPLATE_AGENT)?;
    write_file(
        &root.join("templates").join("specs").join(".gitkeep"),
        TEMPLATE_SPECS_GITKEEP,
    )?;

    set_executable(&root.join("bin").join("ralph-widex"))?;
    set_executable(&root.join("bin").join("ralph-widex-monitor"))?;

    Ok(root)
}

fn write_file(path: &Path, content: impl AsRef<[u8]>) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)
}

#[cfg(unix)]
fn set_executable(path: &Path) -> io::Result<()> {
    let mut perm = fs::metadata(path)?.permissions();
    perm.set_mode(0o755);
    fs::set_permissions(path, perm)
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::process::Command;
    use tempfile::tempdir;

    #[cfg(unix)]
    fn is_executable(path: &Path) -> io::Result<bool> {
        let mode = fs::metadata(path)?.permissions().mode();
        Ok(mode & 0o111 != 0)
    }

    #[cfg(not(unix))]
    fn is_executable(_path: &Path) -> io::Result<bool> {
        Ok(true)
    }

    #[test]
    fn ensure_installed_is_idempotent_and_writes_expected_files() -> io::Result<()> {
        let codex_home = tempdir()?;
        let install_root = ensure_installed(codex_home.path())?;

        let version = fs::read_to_string(install_root.join("VERSION"))?;
        assert_eq!(version, format!("{INSTALL_VERSION}\n"));

        let ralph = install_root.join("bin").join("ralph-widex");
        assert_eq!(ralph.exists(), true);
        assert_eq!(is_executable(&ralph)?, true);

        let monitor = install_root.join("bin").join("ralph-widex-monitor");
        assert_eq!(monitor.exists(), true);
        assert_eq!(is_executable(&monitor)?, true);

        let content = fs::read_to_string(&ralph)?;
        assert_eq!(content.starts_with("#!/usr/bin/env bash"), true);

        // A second install should be a no-op.
        let install_root2 = ensure_installed(codex_home.path())?;
        assert_eq!(install_root2, install_root);

        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn installed_ralph_can_init_project() -> io::Result<()> {
        let codex_home = tempdir()?;
        let install_root = ensure_installed(codex_home.path())?;
        let ralph = install_root.join("bin").join("ralph-widex");

        let project = tempdir()?;
        let status = Command::new(&ralph)
            .current_dir(project.path())
            .arg("init")
            .status()?;
        assert_eq!(status.success(), true);

        assert_eq!(
            project.path().join(".ralph").join("PROMPT.md").exists(),
            true
        );
        assert_eq!(
            project.path().join(".ralph").join("@AGENT.md").exists(),
            true
        );
        assert_eq!(
            project.path().join(".ralph").join("@fix_plan.md").exists(),
            true
        );

        Ok(())
    }
}
