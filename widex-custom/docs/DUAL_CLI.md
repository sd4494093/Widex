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

- Widex keeps its config isolated via `CODEX_HOME` (default: `~/.widex-codex`). Official `codex` uses `~/.codex`.
- API switchover config (Widex):
  - `$WIDEX_API_SWITCHER_CONFIG` (override)
  - `${CODEX_HOME}/api_switchover.yaml` (default)
- Startup behavior (Widex): before the first request, Widex resolves the switchover plan for the
  startup model and applies the same provider/key switching behavior as `/model` (see
  `codex-rs/tui/src/app.rs:957`).

How `config.toml` relates to `api_switchover.yaml`

1) `${CODEX_HOME}/config.toml` defines providers (how to connect)

- `model_providers.<provider_id>` controls connection details: `base_url`, `wire_api` (`responses`/`chat`/`gemini`), headers, retries, etc.
- It does NOT decide which provider to use for a given model, and it does NOT switch keys.
- By default it also does NOT load/store real API keys; it only points at env vars (`env_key`) or other explicit fields (e.g. `experimental_bearer_token`).

2) `${CODEX_HOME}/api_switchover.yaml` defines routing + keys (which provider/key for which model)

- When you run `/model <name>` in the TUI, Widex resolves a `profile` via `models` / `rules` / `default_profile`.
- The profile's `provider_id` must match a `model_providers.<provider_id>` entry in `config.toml` (or a built-in provider id).
- The profile's `auth.*` pulls keys from env (recommended) or saved cache and writes the active key into `${CODEX_HOME}/auth.json`.
  - Widex caches multiple keys per profile under `WIDEX_SAVED_API_KEYS` so switching does not lose keys.

If you do not want to export env vars each time, you can either:

- export once + `/model` once (so Widex caches into `${CODEX_HOME}/auth.json`), then unset, or
- pre-seed `${CODEX_HOME}/auth.json` with `OPENAI_API_KEY` / `GEMINI_API_KEY` and the per-profile
  `WIDEX_SAVED_API_KEYS` entries.

Keep secrets out of the repo: do not commit real keys in YAML/TOML; treat `auth.json` as a local
private file (recommended permissions: `0600`).
