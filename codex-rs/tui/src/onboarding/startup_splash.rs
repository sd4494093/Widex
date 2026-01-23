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

const MIN_ANIMATION_HEIGHT: u16 = 20;
const MIN_ANIMATION_WIDTH: u16 = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StartupSplashOutcome {
    Continue,
    Exit,
}

pub(crate) struct StartupSplashWidget {
    animation: AsciiAnimation,
    animations_enabled: bool,
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

        // Skip the animation entirely when the viewport is too small so we don't clip frames.
        let show_animation =
            area.height >= MIN_ANIMATION_HEIGHT && area.width >= MIN_ANIMATION_WIDTH;

        let mut lines: Vec<Line> = Vec::new();
        if show_animation {
            let frame = self.animation.current_frame();
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
