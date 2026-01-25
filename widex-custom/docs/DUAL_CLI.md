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
- runs the repo-built Rust binary (default: `codex-rs/target/widex-release/codex`)
- builds the binary automatically on first run
- defaults to a fast multi-core build profile, but can be forced back to upstream `--release`

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
cargo build -p codex-cli --bin codex --profile widex-release
```

Faster multi-core release builds

Upstream `--release` uses fat LTO + `codegen-units=1`, which is great for maximum runtime perf but makes local builds slow and mostly single-core. Widex adds a separate Cargo profile `widex-release` for local dev:

- `lto = "thin"`
- `codegen-units = 16`
- `incremental = true`
- `strip = "none"`

You can tune/override it:

```bash
# Faster (default) - uses the `widex-release` profile
widex --version

# Force a rebuild (if you already have a binary and want to rebuild anyway)
WIDEX_FORCE_REBUILD=1 widex --version

# Use upstream release profile exactly
WIDEX_BUILD_PROFILE=upstream widex --version

# Control build parallelism
WIDEX_CARGO_JOBS="$(nproc)" widex --version

# Optional compile cache for faster rebuilds (if installed)
WIDEX_USE_SCCACHE=1 widex --version
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
