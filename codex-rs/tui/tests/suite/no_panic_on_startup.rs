use codex_core::AuthManager;
use codex_core::CodexAuth;
use codex_core::ThreadManager;
use codex_core::config::ConfigBuilder;
use codex_protocol::protocol::SessionSource;
use pretty_assertions::assert_eq;

/// Regression test for https://github.com/openai/codex/issues/8803.
#[tokio::test]
#[ignore = "TODO(mbolin): flaky"]
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

    let args = vec!["-c".to_string(), "analytics.enabled=false".to_string()];
    let spawned = codex_utils_pty::spawn_pty_process(
        codex_cli.to_string_lossy().as_ref(),
        &args,
        cwd.as_ref(),
        &env,
        &None,
        codex_utils_pty::TerminalSize::default(),
    )
    .await?;
    let mut output = Vec::new();
    let codex_utils_pty::SpawnedProcess {
        session,
        stdout_rx,
        stderr_rx,
        exit_rx,
    } = spawned;
    let mut output_rx = codex_utils_pty::combine_output_receivers(stdout_rx, stderr_rx);
    let mut exit_rx = exit_rx;
    let writer_tx = session.writer_sender();
    let exit_code_result = timeout(Duration::from_secs(10), async {
        // Read PTY output until the process exits while replying to cursor
        // position queries so the TUI can initialize without a real terminal.
        loop {
            select! {
                result = output_rx.recv() => match result {
                    Ok(chunk) => {
                        // The TUI asks for the cursor position via ESC[6n.
                        // Respond with a valid position to unblock startup.
                        if chunk.windows(4).any(|window| window == b"\x1b[6n") {
                            let _ = writer_tx.send(b"\x1b[1;1R".to_vec()).await;
                        }
                        output.extend_from_slice(&chunk);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break exit_rx.await,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                },
                result = &mut exit_rx => break result,
            }
        }
    })
    .await;
    let exit_code = match exit_code_result {
        Ok(Ok(code)) => code,
        Ok(Err(err)) => return Err(err.into()),
        Err(_) => {
            session.terminate();
            anyhow::bail!("timed out waiting for codex CLI to exit");
        }
    };
    // Drain any output that raced with the exit notification.
    while let Ok(chunk) = output_rx.try_recv() {
        output.extend_from_slice(&chunk);
    }

    let output = String::from_utf8_lossy(&output);
    Ok(CodexCliOutput {
        exit_code,
        output: output.to_string(),
    })
}
