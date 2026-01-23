# Ralph for Widex (Codex)

This feature ports the core ideas from `frankbria/ralph-claude-code` (MIT license; see
`LICENSE.upstream`) to Widex/Codex.

Goal: provide an autonomous development loop with exit detection, rate limiting, and monitoring,
triggered from within the Widex TUI via the `/ralph-widex` slash command.

## What you get

- `/ralph-widex` starts an autonomous loop that repeatedly runs `codex exec` in your current repo,
  using `.ralph/PROMPT.md` as the prompt.
- `/ralph-widex init` creates a `.ralph/` directory in the current repo from templates.
- `/ralph-widex setup <project>` creates a new project directory and initializes `.ralph/`.
- `/ralph-widex monitor` runs a simple terminal dashboard reading `.ralph/status.json`.

## Files and conventions

Ralph-specific files live in `.ralph/`:

- `.ralph/PROMPT.md` : loop instructions (must include a `---RALPH_STATUS---` block)
- `.ralph/@fix_plan.md` : prioritized tasks
- `.ralph/@AGENT.md` : build/test commands and project notes
- `.ralph/logs/` : loop logs
- `.ralph/status.json` : live status for the monitor

## Dependencies

- `bash`
- `jq`
- `git` (recommended)

## Notes

- The loop uses `codex exec` (headless mode) under the hood.
- Session continuity is implemented via `codex exec resume <session_id>` using the session id
  printed by `codex exec`.
