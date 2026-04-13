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

use crate::ascii_animation::AsciiAnimation;
use crate::tui::FrameRequester;
use crate::tui::Tui;
use crate::tui::TuiEvent;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StartupSplashOutcome {
    Continue,
    Exit,
}

pub(crate) struct StartupSplashWidget {
    animation: AsciiAnimation,
    animations_enabled: bool,
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
    pub(crate) fn new(request_frame: FrameRequester, animations_enabled: bool) -> Self {
        Self {
            animation: AsciiAnimation::new(request_frame),
            animations_enabled,
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
        lines.push(Line::from(vec![
            "  ".into(),
            "Press any key to continue".dim(),
            " ".into(),
            "(Ctrl+C to quit)".dim(),
        ]));

        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }
}

pub(crate) async fn run_startup_splash(
    tui: &mut Tui,
    animations_enabled: bool,
) -> Result<StartupSplashOutcome> {
    use tokio_stream::StreamExt;

    let widget = StartupSplashWidget::new(tui.frame_requester(), animations_enabled);

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
                return Ok(StartupSplashOutcome::Continue);
            }
            TuiEvent::Paste(_) => return Ok(StartupSplashOutcome::Continue),
            TuiEvent::Draw => {
                let _ = tui.draw(u16::MAX, |frame| {
                    frame.render_widget_ref(&widget, frame.area());
                });
            }
        }
    }

    Ok(StartupSplashOutcome::Continue)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use ratatui::layout::Rect;

    #[test]
    fn startup_splash_shows_animation_when_frame_fits() {
        let widget = StartupSplashWidget::new(FrameRequester::test_dummy(), true);
        let frame = widget.animation.current_frame();
        let (frame_width, frame_height) = frame_dimensions(frame);
        let area = Rect::new(0, 0, frame_width, frame_height + 3);

        assert_eq!(should_show_animation(area, true, frame), true);
    }

    #[test]
    fn startup_splash_skips_animation_when_frame_does_not_fit() {
        let widget = StartupSplashWidget::new(FrameRequester::test_dummy(), true);
        let frame = widget.animation.current_frame();
        let (frame_width, frame_height) = frame_dimensions(frame);
        let area = Rect::new(0, 0, frame_width, frame_height + 2);

        assert_eq!(should_show_animation(area, true, frame), false);
    }
}
