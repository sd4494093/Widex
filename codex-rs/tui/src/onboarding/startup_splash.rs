use crossterm::event::KeyCode;
use crossterm::event::KeyEventKind;
use crossterm::event::KeyModifiers;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::prelude::Widget;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::Clear;
use ratatui::widgets::Paragraph;
use ratatui::widgets::WidgetRef;
use ratatui::widgets::Wrap;

use color_eyre::eyre::Result;

use codex_login::load_auth_dot_json;
use codex_login::read_openai_api_key_from_env;

use crate::ascii_animation::AsciiAnimation;
use crate::legacy_core::config::Config;
use crate::tui::FrameRequester;
use crate::tui::Tui;
use crate::tui::TuiEvent;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StartupSplashOutcome {
    Continue,
    EnterApiKey,
    Exit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StartupSplashMode {
    ContinuePrompt,
    WidexKeyLoadedPrompt,
    WidexAuthPrompt,
}

pub(crate) struct StartupSplashWidget {
    animation: AsciiAnimation,
    animations_enabled: bool,
    mode: StartupSplashMode,
}

fn frame_dimensions(frame: &str) -> (u16, u16) {
    let frame_height = frame.lines().count() as u16;
    let frame_width = frame
        .lines()
        .map(|line| line.chars().count() as u16)
        .max()
        .unwrap_or(0);
    (frame_width, frame_height)
}

fn should_show_animation(area: Rect, animations_enabled: bool, frame: &str) -> bool {
    let (frame_width, frame_height) = frame_dimensions(frame);
    let required_height = frame_height.saturating_add(3);
    animations_enabled && area.height >= required_height && area.width >= frame_width
}

impl StartupSplashWidget {
    pub(crate) fn new(
        request_frame: FrameRequester,
        animations_enabled: bool,
        mode: StartupSplashMode,
    ) -> Self {
        Self {
            animation: AsciiAnimation::new(request_frame),
            animations_enabled,
            mode,
        }
    }
}

impl WidgetRef for &StartupSplashWidget {
    fn render_ref(&self, area: Rect, buf: &mut Buffer) {
        Clear.render(area, buf);
        if self.animations_enabled {
            self.animation.schedule_next_frame();
        }

        let frame = self.animation.current_frame();
        let show_animation = should_show_animation(area, self.animations_enabled, frame);

        let mut lines: Vec<Line> = Vec::new();
        if show_animation {
            lines.extend(frame.lines().map(Into::into));
            lines.push("".into());
        }

        lines.push(Line::from(vec![
            "  ".into(),
            "Welcome to ".into(),
            "Widex".bold(),
            ", the intelligent coding engine".into(),
        ]));
        match self.mode {
            StartupSplashMode::ContinuePrompt => {
                lines.push(Line::from(vec![
                    "  ".into(),
                    "Press any key to continue".dim(),
                    " ".into(),
                    "(Ctrl+C to quit)".dim(),
                ]));
            }
            StartupSplashMode::WidexKeyLoadedPrompt => {
                lines.push("".into());
                lines.push("  Detected an existing Widex Key.".into());
                lines.push(Line::from(vec![
                    "  ".into(),
                    "Press any key to continue".dim(),
                    " ".into(),
                    "(Ctrl+C to quit)".dim(),
                ]));
                lines.push("  Press e to replace the current Widex Key.".dim().into());
            }
            StartupSplashMode::WidexAuthPrompt => {
                lines.push("".into());
                lines.push(Line::from(vec![
                    "  ".into(),
                    "1. ".cyan(),
                    "Input Widex Key (WillAU API Key)".into(),
                ]));
                lines.push("     Press Enter, 1, or e to continue.".dim().into());
                lines.push("".into());
                lines.push(Line::from(vec!["  ".into(), "2. ".cyan(), "Quit".into()]));
                lines.push("     Press 2, q, or Ctrl+C to exit.".dim().into());
            }
        }

        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }
}

pub(crate) async fn run_startup_splash(
    tui: &mut Tui,
    animations_enabled: bool,
    mode: StartupSplashMode,
) -> Result<StartupSplashOutcome> {
    use tokio_stream::StreamExt;

    let widget = StartupSplashWidget::new(tui.frame_requester(), animations_enabled, mode);

    tui.draw(u16::MAX, |frame| {
        frame.render_widget_ref(&widget, frame.area());
    })?;

    let tui_events = tui.event_stream();
    tokio::pin!(tui_events);

    while let Some(event) = tui_events.next().await {
        match event {
            TuiEvent::Key(key_event) => {
                if key_event.kind != KeyEventKind::Press {
                    continue;
                }
                if (key_event.code == KeyCode::Char('c') || key_event.code == KeyCode::Char('d'))
                    && key_event.modifiers.contains(KeyModifiers::CONTROL)
                {
                    return Ok(StartupSplashOutcome::Exit);
                }
                match mode {
                    StartupSplashMode::ContinuePrompt => return Ok(StartupSplashOutcome::Continue),
                    StartupSplashMode::WidexKeyLoadedPrompt => match key_event.code {
                        KeyCode::Char('e') => return Ok(StartupSplashOutcome::EnterApiKey),
                        _ => return Ok(StartupSplashOutcome::Continue),
                    },
                    StartupSplashMode::WidexAuthPrompt => match key_event.code {
                        KeyCode::Enter | KeyCode::Char('1') | KeyCode::Char('e') => {
                            return Ok(StartupSplashOutcome::EnterApiKey);
                        }
                        KeyCode::Char('2') | KeyCode::Char('q') => {
                            return Ok(StartupSplashOutcome::Exit);
                        }
                        _ => continue,
                    },
                }
            }
            TuiEvent::Paste(_) => match mode {
                StartupSplashMode::ContinuePrompt | StartupSplashMode::WidexKeyLoadedPrompt => {
                    return Ok(StartupSplashOutcome::Continue);
                }
                StartupSplashMode::WidexAuthPrompt => continue,
            },
            TuiEvent::Draw => {
                let _ = tui.draw(u16::MAX, |frame| {
                    frame.render_widget_ref(&widget, frame.area());
                });
            }
            TuiEvent::Resize => {
                let _ = tui.draw(u16::MAX, |frame| {
                    frame.render_widget_ref(&widget, frame.area());
                });
            }
        }
    }

    Ok(StartupSplashOutcome::Continue)
}

/// True when the configured `codex_home` is a Widex-managed home directory.
pub(crate) fn is_widex_codex_home(config: &Config) -> bool {
    config
        .codex_home
        .file_name()
        .is_some_and(|name| name == ".widex" || name == ".widex-codex")
}

fn widex_api_key_present(config: &Config) -> bool {
    if read_openai_api_key_from_env().is_some() {
        return true;
    }

    match load_auth_dot_json(&config.codex_home, config.cli_auth_credentials_store_mode) {
        Ok(Some(auth)) => auth
            .openai_api_key
            .as_deref()
            .is_some_and(|api_key| !api_key.trim().is_empty()),
        Ok(None) => false,
        Err(err) => {
            tracing::warn!("failed to read auth state for startup splash: {err}");
            false
        }
    }
}

/// Select the Widex startup splash variant for the current auth state.
pub(crate) fn startup_splash_mode(config: &Config) -> StartupSplashMode {
    if is_widex_codex_home(config) && config.model_provider.requires_openai_auth {
        if widex_api_key_present(config) {
            StartupSplashMode::WidexKeyLoadedPrompt
        } else {
            StartupSplashMode::WidexAuthPrompt
        }
    } else {
        StartupSplashMode::ContinuePrompt
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use ratatui::layout::Rect;

    #[test]
    fn startup_splash_shows_animation_when_frame_fits() {
        let widget = StartupSplashWidget::new(
            FrameRequester::test_dummy(),
            true,
            StartupSplashMode::ContinuePrompt,
        );
        let frame = widget.animation.current_frame();
        let (frame_width, frame_height) = frame_dimensions(frame);
        let area = Rect::new(0, 0, frame_width, frame_height + 3);

        assert_eq!(should_show_animation(area, true, frame), true);
    }

    #[test]
    fn startup_splash_skips_animation_when_frame_does_not_fit() {
        let widget = StartupSplashWidget::new(
            FrameRequester::test_dummy(),
            true,
            StartupSplashMode::ContinuePrompt,
        );
        let frame = widget.animation.current_frame();
        let (frame_width, frame_height) = frame_dimensions(frame);
        let area = Rect::new(0, 0, frame_width, frame_height + 2);

        assert_eq!(should_show_animation(area, true, frame), false);
    }

    #[test]
    fn continue_prompt_renders_continue_copy() {
        let widget = StartupSplashWidget::new(
            FrameRequester::test_dummy(),
            false,
            StartupSplashMode::ContinuePrompt,
        );
        let area = Rect::new(0, 0, 80, 8);
        let mut buf = Buffer::empty(area);

        (&widget).render_ref(area, &mut buf);

        let rendered = (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rendered.contains("Press any key to continue"));
        assert!(rendered.contains("(Ctrl+C to quit)"));
    }

    #[test]
    fn widex_auth_prompt_renders_key_and_quit_choices() {
        let widget = StartupSplashWidget::new(
            FrameRequester::test_dummy(),
            false,
            StartupSplashMode::WidexAuthPrompt,
        );
        let area = Rect::new(0, 0, 80, 12);
        let mut buf = Buffer::empty(area);

        (&widget).render_ref(area, &mut buf);

        let rendered = (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rendered.contains("Input Widex Key (WillAU API Key)"));
        assert!(rendered.contains("Quit"));
    }

    #[test]
    fn widex_key_loaded_prompt_renders_continue_and_replace_copy() {
        let widget = StartupSplashWidget::new(
            FrameRequester::test_dummy(),
            false,
            StartupSplashMode::WidexKeyLoadedPrompt,
        );
        let area = Rect::new(0, 0, 80, 10);
        let mut buf = Buffer::empty(area);

        (&widget).render_ref(area, &mut buf);

        let rendered = (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rendered.contains("Detected an existing Widex Key."));
        assert!(rendered.contains("Press any key to continue"));
        assert!(rendered.contains("Press e to replace the current Widex Key."));
    }
    #[tokio::test]
    async fn widex_startup_splash_prompts_for_key_without_auth_json() -> std::io::Result<()> {
        let temp_dir = tempfile::TempDir::new()?;
        let config = build_widex_config(&temp_dir).await?;

        assert_eq!(
            startup_splash_mode(&config),
            StartupSplashMode::WidexAuthPrompt
        );
        Ok(())
    }

    #[tokio::test]
    async fn widex_startup_splash_continues_with_auth_json_key() -> std::io::Result<()> {
        let temp_dir = tempfile::TempDir::new()?;
        let config = build_widex_config(&temp_dir).await?;
        std::fs::write(
            config.codex_home.join("auth.json"),
            r#"{"OPENAI_API_KEY":"sk-test","auth_mode":"apikey"}"#,
        )?;

        assert_eq!(
            startup_splash_mode(&config),
            StartupSplashMode::WidexKeyLoadedPrompt
        );
        Ok(())
    }

    async fn build_widex_config(temp_dir: &tempfile::TempDir) -> std::io::Result<Config> {
        let codex_home = temp_dir.path().join(".widex");
        std::fs::create_dir_all(&codex_home)?;
        crate::legacy_core::config::ConfigBuilder::default()
            .codex_home(codex_home)
            .build()
            .await
    }
}
