use super::*;
use codex_ralph_widex::widex_overlay;

fn submit_composer_text(chat: &mut ChatWidget, text: &str) {
    chat.bottom_pane
        .set_composer_text(text.to_string(), Vec::new(), Vec::new());
    chat.handle_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    chat.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
}

#[tokio::test]
async fn ralph_help_uses_shared_overlay_copy() {
    let (mut chat, mut rx, _op_rx) = make_chatwidget_manual(/*model_override*/ None).await;

    submit_composer_text(&mut chat, "/ralph-widex --help");

    let cells = drain_insert_history(&mut rx);
    let rendered = cells
        .iter()
        .map(|cell| lines_to_single_string(cell))
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains(widex_overlay::tui_help_text()));
    assert!(rendered.contains("/ralph-widex start"));
}
