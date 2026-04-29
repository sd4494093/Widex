## Ralph-Widex Overlay

This directory is the Widex-owned overlay boundary for `ralph-widex`.

Purpose:

- keep Widex product policy, TUI copy, and Ralph workflow prompts outside upstream-heavy Rust files
- let upstream `codex-rs` changes land with minimal merge pressure
- make Ralph customization reviewable as small, isolated overlay assets

Ownership split:

- `codex-rs/ralph-widex/`: shared execution engine and adapter API
- `widex-custom/features/ralph-widex/overlay/`: Widex-owned product wording and TUI prompt assets
- `widex-custom/features/ralph-widex/templates/`: `.ralph/` initialization templates

Merge rule:

- when following upstream, do not re-edit large `chatwidget.rs` Ralph copy blocks
- update the shared overlay adapter only if upstream command wiring changes
- keep Widex wording changes in this directory whenever possible

Current assets:

- `tui_help.txt`: TUI `/ralph-widex --help` copy
- `tui_loop_prompt.txt`: per-loop prompt template injected into the active Widex session
