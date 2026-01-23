# API Switchover (widex)

This folder contains a YAML-driven "API/provider switcher" for widex that can be used in two ways:

1) Inside the TUI: selecting a model via `/model` will also (optionally) switch:
- the active `model_provider` (via `Op::OverrideTurnContext.model_provider_id`)
- the stored API keys in `CODEX_HOME/auth.json` (writes OPENAI_API_KEY / GEMINI_API_KEY)

2) As a standalone CLI:
- `cargo run -p codex-api-switchover -- --config <path> list`
- `cargo run -p codex-api-switchover -- --config <path> resolve <model>`
- `cargo run -p codex-api-switchover -- --config <path> apply --model <model> --set-model`

## Config Location

The TUI searches for the config in this order:

1. `$CODEX_API_SWITCHER_CONFIG`
2. `$WIDEX_API_SWITCHER_CONFIG`
3. `$CODEX_HOME/api_switchover.yaml`
4. `<cwd>/widex-custom/features/api-switchover/api_config.yaml` (useful for repo-local dev)

## Setup

1) Copy the template:

```bash
cp widex-custom/features/api-switchover/api_config.example.yaml ~/.codex/api_switchover.yaml
```

2) Export keys (recommended) or fill literals in your local yaml (do not commit secrets):

```bash
export OPENAI_API_KEY='sk-...'
export GEMINI_API_KEY='...'
```

3) Ensure the provider ids referenced by the yaml exist in `~/.codex/config.toml` under
`model_providers` (built-ins: `openai`, `gemini`, `ollama`, `lmstudio`).

## Notes

- Widex stores multiple keys in `auth.json` under `WIDEX_SAVED_API_KEYS` so switching profiles does
  not lose previous values. The active key is still written to `OPENAI_API_KEY` / `GEMINI_API_KEY`.

- If a profile references an env var (e.g. `GROK_API_KEY`) and that env var is missing at runtime,
  the switchover will fall back to the last saved key for that profile (if any).

- Grok (via VectorEngine) uses an OpenAI-compatible Chat Completions endpoint (`/v1/chat/completions`).
  In the switcher YAML, configure Grok profiles using `auth.openai_api_key` (recommended from `GROK_API_KEY` env).
