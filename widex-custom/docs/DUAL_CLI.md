# Dual CLI: official `codex` + widex `widex`

Goal

- Keep the official npm CLI as `codex` on your PATH.
- Run the widex fork as a separate command (`widex`) so it can evolve independently (Gemini wire support, switcher, custom config), while still merging upstream into `main` and then into `widex`.

Why

- The npm CLI currently rejects `wire_api = "gemini"` (it only supports `responses`, `responses_websocket`, `chat`).
- The widex fork adds `wire_api = "gemini"` plus Gemini request/SSE support.
- If both CLIs share the same `CODEX_HOME` (default: `~/.codex`), one will eventually break the other.

Solution

1) Separate `CODEX_HOME`

- Official npm codex: `~/.codex`
- Widex codex: `~/.widex-codex`

This repo now ships a wrapper script at `widex-custom/bin/widex` that:

- sets `CODEX_HOME=~/.widex-codex` by default
- runs the repo-built Rust binary (`codex-rs/target/release/codex`)
- builds `codex-rs/target/release/codex` automatically on first run (`cargo build --release`)

2) Install the wrapper on PATH

Pick one:

- Symlink into `~/.local/bin`:

  ```bash
  ln -sf /home/will/data/codex/widex-custom/bin/widex ~/.local/bin/widex
  ```

- Or call it directly from the repo:

  ```bash
  /home/will/data/codex/widex-custom/bin/widex
  ```

3) Build widex codex

```bash
cd /home/will/data/codex/codex-rs
cargo build -p codex-cli --bin codex --release
```

4) Usage

- Official:

  ```bash
  codex
  ```

- Widex:

  ```bash
  widex
  ```

Notes

- The widex TUI will look for the API switchover YAML at `${CODEX_HOME}/api_switchover.yaml`.
- Keep secrets out of the repo: API keys should live in `${CODEX_HOME}/auth.json` or environment variables.
