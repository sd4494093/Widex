use codex_core::AuthManager;
use codex_core::CodexAuth;
use codex_core::ThreadManager;
use codex_core::config::ConfigBuilder;
use codex_protocol::protocol::SessionSource;
use pretty_assertions::assert_eq;

/// Regression test for https://github.com/openai/codex/issues/8803.
#[tokio::test]
async fn malformed_rules_should_not_panic() -> anyhow::Result<()> {
    // Spawning interactive UIs under PTY is flaky on Windows due to PTY limitations.
    if cfg!(windows) {
        return Ok(());
    }

    let tmp = tempfile::tempdir()?;
    let codex_home = tmp.path().to_path_buf();

    // Execpolicy rules are expected to be loaded from a directory, so a regular file should yield
    // a user-facing error (not a panic).
    std::fs::write(
        codex_home.join("rules"),
        "rules should be a directory not a file",
    )?;

    // Force a local provider so the test doesn't need OpenAI auth.
    let cwd = std::env::current_dir()?;
    let config_contents = format!(
        r#"
model_provider = "ollama"

[projects]
"{cwd}" = {{ trust_level = "trusted" }}
"#,
        cwd = cwd.display()
    );
    std::fs::write(codex_home.join("config.toml"), config_contents)?;

    let config = ConfigBuilder::default()
        .codex_home(codex_home.clone())
        .fallback_cwd(Some(cwd.clone()))
        .build()
        .await?;

    let auth = CodexAuth::create_dummy_chatgpt_auth_for_testing();
    let auth_manager = AuthManager::from_auth_for_testing_with_home(auth, codex_home.clone());
    let manager = ThreadManager::new(codex_home, auth_manager, SessionSource::Cli);

    let err = manager
        .start_thread(config)
        .await
        .err()
        .expect("expected rules load error");
    let msg = err.to_string();
    assert_eq!(msg.contains("failed to load rules"), true);
    assert_eq!(msg.contains("failed to read rules files"), true);

    Ok(())
}
