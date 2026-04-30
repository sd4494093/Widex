---
name: widwex-upstream-update-workflow
description: Follow the latest stable openai/codex release tag into Widex, reattach the Widex overlay with minimal drift, validate Ralph/TUI/startup behavior, rebuild widex-release, and prepare npm publication. Use when updating Widex against upstream or closing a production release.
---

# Widex Upstream Update Workflow

Use this workflow when Widex needs to catch up to upstream while keeping only the Widex product layer and `ralph-widex` overlay stable.

## Product Rules

- Track stable upstream release tags, not `upstream/main`, unless explicitly requested.
- Preserve Widex differentiators only in these areas:
  - `widex-custom/features/ralph-widex/overlay/`
  - Widex / WillAU startup and auth behavior
  - TUI `/ralph-widex` command path
- Do not revive old multi-LLM runtime integrations unless explicitly requested.
- Keep `widex-custom/docs/LLMs_intergration/` as documentation only.

## Update Steps

1. Confirm branch and remotes.
   - `git remote -v`
   - `git status --short --branch`
   - work on `widex`
2. Fetch upstream and identify the latest stable release tag.
   - `git fetch upstream --tags`
   - `git tag -l 'rust-v0.*' | sort -V | tail`
3. Merge the chosen stable tag into `widex`.
   - prefer upstream as the base
   - reattach Widex behavior only where product requirements demand it
4. Reapply overlay expectations.
   - check `codex-rs/ralph-widex/src/widex_overlay.rs`
   - check `widex-custom/features/ralph-widex/overlay/*`
   - check Widex onboarding/auth flow and hidden provider filtering
5. Validate code.
   - `cd codex-rs && just fmt`
   - run targeted tests for touched crates first
   - if TUI or user-visible text changed, accept/update snapshots intentionally
   - run `just fix -p ...` for touched crates before final closure
6. Validate runtime.
   - `cargo build -p codex-cli --bin codex --profile widex-release`
   - `widex --version`
   - `widex --help`
   - in TUI, verify `/ralph-widex --help`, `/ralph-widex init`, `/ralph-widex status`
   - verify startup splash behavior:
     - with existing auth key: continue prompt
     - without auth key: `Input Widex Key / Quit`
7. Validate clean-home behavior.
   - run Widex with empty `WIDEX_CODEX_HOME`
   - confirm standard WellAU config is created
   - confirm `auth.json` is not auto-created without user input
8. Release closure.
   - review `git status`
   - commit merge + Widex overlay fixes together
   - push `origin widex`
   - run `npm pack --dry-run`
   - publish when requested
   - for `widex-linux-x64`, ensure the vendor tree includes `x86_64-unknown-linux-gnu`; do not rely on musl-only vendor contents

## Merge Heuristics

- Prefer small adapter repairs over re-editing large upstream TUI files.
- If only wording or Ralph prompts changed, update overlay assets first.
- If protocol fields changed upstream, patch tests and adapters, not product behavior.
- Do not revert unrelated user deletions or local docs unless explicitly asked.

## Production Closure Checklist

- target upstream stable tag recorded explicitly
- `just fix` passes for touched crates
- targeted tests pass
- Widex release binary builds
- startup splash behavior matches Widex rules
- Ralph entrypoints work in TUI
- clean-home config generation matches WellAU defaults
- branch committed and pushed
- npm artifact verified before publish
- `widex-linux-x64` vendor payload verified to contain `x86_64-unknown-linux-gnu`
