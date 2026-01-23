# Widex Custom（宪法 / 边界 / 变更总览）

本文件是 widex 分支的“总纲”，用于让后续贡献者/Agent 快速理解：

- 我们在这个 fork 里做什么、不做什么
- 如何在不影响上游的前提下演进（Gemini / Grok / Sonnet 等）
- 配置、切换器、密钥存储的**安全边界**

作用域：`widex-custom/`（文档/配置模板/约定），以及它们如何映射到 `codex-rs/` 的实现点。


## 1) 分支策略（上游跟随 + widex 定制）

- `widex`：日常开发/运行/测试/使用都在这里
- `main`：尽量接近上游，只在需要同步上游时更新

同步上游（建议节奏：每周或每月一次）：

```bash
git checkout main
git fetch upstream
git merge upstream/main
git push origin main

git checkout widex
git merge main
# 解决冲突（如果有）
git push origin widex
```


## 2) Dual CLI：官方 npm `codex` 与 widex `widex` 共存

目标：

- 继续使用官方 npm 安装的 `codex`（配置默认 `~/.codex`）
- widex fork 使用单独的命令 `widex`（配置默认 `~/.widex-codex`）

原因：

- widex 增加了 `wire_api = "gemini"` 等官方 npm CLI 不识别的配置扩展
- 两者共用同一个 `CODEX_HOME` 会互相“读坏配置”

实现：

- wrapper：`widex-custom/bin/widex`
  - 默认 `CODEX_HOME=~/.widex-codex`
  - 若 `codex-rs/target/release/codex` 不存在，会自动 `cargo build --release`
  - 设置 `WIDEX_API_SWITCHER_CONFIG=${CODEX_HOME}/api_switchover.yaml` 便于 TUI 读取切换器配置

更多说明见：`widex-custom/docs/DUAL_CLI.md`。


## 3) API Switchover（YAML 驱动的 provider/key 快速切换）

目录：`widex-custom/features/api-switchover/`

用途：

- 在 TUI 中用 `/model` 切换模型时，按规则自动切换 `model_provider`，并更新本地鉴权信息
- 提供独立 CLI：`codex-rs/api-switchover`（crate：`codex-api-switchover`）

配置发现顺序（TUI）：

1. `CODEX_API_SWITCHER_CONFIG`
2. `WIDEX_API_SWITCHER_CONFIG`
3. `${CODEX_HOME}/api_switchover.yaml`
4. `<cwd>/widex-custom/features/api-switchover/api_config.yaml`（仅用于仓库本地开发）

安全策略（关键）：

- **不要把任何 key 写进 git 管理的文件**
- 推荐在 YAML 里用 `env: XXX_API_KEY`
- widex 会把第一次切换时读到的 key 缓存进 `${CODEX_HOME}/auth.json`

### 3.1 多 key 缓存（WIDEX_SAVED_API_KEYS）

为了解决“切换 profile 会覆盖 OPENAI_API_KEY / GEMINI_API_KEY 导致丢 key”的问题，widex 扩展了
`auth.json`：

- `WIDEX_SAVED_API_KEYS`：保存多份 key（按 profile 维度缓存）
- 切换时：
  - 若 env/literal 提供了 key：写入 `WIDEX_SAVED_API_KEYS` 并设置当前 `OPENAI_API_KEY`/`GEMINI_API_KEY`
  - 若 env 缺失：回退到该 profile 之前保存的 key（若存在）

这使得你可以：

1) 临时 export `GROK_API_KEY/GEMINI_API_KEY/...` 切换一次
2) 切换成功后 unset env
3) 之后依然可以在 widex 内来回切模型/provider，不会丢失之前保存的 key

### 3.2 Ralph for Widex（/ralph-widex）

目录：`widex-custom/features/ralph-widex/`

用途：

- 在 TUI 内通过 `/ralph-widex` 启动一个“自主开发循环”（Ralph loop），底层会反复调用 `codex exec`
  并使用当前 repo 的 `.ralph/PROMPT.md` 作为提示词。
- 支持 `/ralph-widex init` 初始化当前目录的 `.ralph/` 结构（模板位于 feature 目录，会自动安装到
  `${CODEX_HOME}/features/ralph-widex`）。
- 支持 `/ralph-widex monitor` 在终端查看 `.ralph/status.json` 的实时状态面板。


## 4) Gemini 集成（新增 Wire API）

定位：Gemini 不是 OpenAI-compatible Responses/Chat，所以 widex 增加了新的 wire：

- `wire_api = "gemini"`
- 请求：`/models/{api_model}:streamGenerateContent?alt=sse`
- SSE 解析：Gemini parts -> Codex `ResponseEvent`

推荐入口文档：

- `widex-custom/docs/Gemini_intergration/README.md`


## 5) Grok 接入（OpenAI Chat Completions 兼容）

Grok（通过 VectorEngine 中转）走 OpenAI Chat Completions：

- `POST https://api.vectorengine.ai/v1/chat/completions`
- `stream: true` 返回 `text/event-stream` SSE（`chat.completion.chunk` + `[DONE]`）

widex 当前落地：

- 内置 provider：`grok-vectorengine`（`wire_api = "chat"`）
- 预设模型（picker 可见）：
  - `grok-4.1`
  - `grok-4-1-fast-reasoning`
  - `grok-4-1-fast-non-reasoning`
- switchover 模板已包含 `grok-` 前缀规则示例（见 `widex-custom/features/api-switchover/api_config.example.yaml`）


## 6) UI/品牌化（Widex）

widex 的 TUI 会显示 Widex 标识，并在启动时显示动画 splash（可随配置关闭动画）。

注意：UI 的 snapshot 测试使用 `insta`，UI 改动需要同步更新快照。


## 7) 密钥与 Git 安全（必须遵守）

永远不要把以下内容提交到 git：

- `${CODEX_HOME}/auth.json`（包含 `OPENAI_API_KEY` / `GEMINI_API_KEY` / `WIDEX_SAVED_API_KEYS`）
- 任何包含真实 `sk-...` 的 yaml/toml/json

防呆：

- 仓库根 `.gitignore` 已忽略 repo 内的 `.codex/` 与 `.widex-codex/`，避免把 `CODEX_HOME` 指到仓库时意外提交鉴权文件。
- `widex-custom/features/api-switchover/api_config.yaml`（本地 secrets 文件）已在 `.gitignore` 中忽略。


## 8) 下一步：按同一模板继续演进（Grok “更多能力” / Sonnet）

按“配置层 → 协议层 → 请求构造 → 流式解析 → UI 支持”推进：

### A) Grok “更多能力”（待你确认优先级）

候选项（可多选）：

- tools/function calling：tool schema、tool choice、tool-call delta 的兼容性验证
- 多模态：image input / image output（如果中转支持）
- reasoning 参数映射：effort/temperature/top_p 等与 grok 参数如何对应
- token/usage 解析：stream/非 stream 情况下 usage 的提取与 UI 展示
- 错误码/重试策略：429/5xx 的指数退避、请求超时、SSE 断流重连等

### B) Sonnet 接入（先确定 API 形态）

- 如果走 OpenAI-compatible（Responses 或 Chat Completions）：通常只需新增 provider + presets + switchover 规则
- 如果走 Anthropic 原生 streaming：需要新增 `wire_api` + 新模块（仿照 Gemini 的拆分方式）
