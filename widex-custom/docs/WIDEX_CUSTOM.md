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

说明：

- `templates/`：Rust 原生实现会直接复用这里的模板内容生成 `.ralph/`
- `bin/`、`lib/`：早期 shell 版实现遗留（不作为运行路径，只保留作参考）

运行期文件（Rust 版会写入/读取）：

- `.ralph/status.json`：loop 状态（含 rate limit 信息）
- `.ralph/progress.json`：当前 `widex exec` 的实时进度（执行中才存在）
- `.ralph/.response_analysis`：每轮分析结果（退出检测/进展/错误）
- `.ralph/.exit_signals`：用于观测“test-only / completion 信号”的累积记录
- `.ralph/.circuit_breaker_state` / `.ralph/.circuit_breaker_history`：熔断器状态与历史
- `.ralph/STOP`：若存在，loop 会尽快退出；Rust 版也会在 `widex exec` 运行中或 rate-limit 等待期间检测到 STOP，并触发 graceful shutdown
- `.ralph/ralph_output_schema.json`：Ralph 结构化输出的 JSON Schema（用于 `widex exec --output-schema`）

用途：

- 在 TUI 内通过 `/ralph-widex` 启动一个“自主开发循环”（Ralph loop），底层会反复调用 `widex exec`
  并使用当前 repo 的 `.ralph/PROMPT.md` 作为提示词。
- 支持 `/ralph-widex init` 初始化当前目录的 `.ralph/` 结构（模板来源于 `widex-custom/features/ralph-widex/templates/`）。
- 支持 `/ralph-widex monitor` 在终端查看 `.ralph/status.json` 的实时状态面板。

现状（widex 分支）：

- TUI 的 `/ralph-widex` **只调用 Rust 原生实现**：`widex ralph-widex ...`（无需安装 shell 脚本）。
- `widex-custom/features/ralph-widex/` 的 shell 版仅作为历史参考保留（不再作为兜底/回退路径；不保证可用性）。

### 3.3 Ralph（生产级 Rust 重构规划）

背景：`ralph-widex` 的上游原型来自 `/home/will/data/backups/ralph-claude-code`（面向 `claude-code` 的 shell 插件）。
我们已在 Widex 中完成“可用移植版”（shell + jq + grep 解析），但它天然存在如下风险：`set -euo pipefail`
下的管道退出、跨平台 `date/timeout` 差异、以及对模型输出格式漂移的脆弱依赖。

Widex 的生产级目标：把 Ralph 做成 **原生 Widex 功能**（原生 slash 命令 + Rust 实现），稳定、可测试、
可演进，同时尽量保持 `.ralph/` 目录约定向后兼容。

#### Rust 原生版的设计原则（强制）

- **不依赖 shell 工具链**：不需要 `jq/timeout/grep`；用 Rust（`serde_json` + `tokio`）实现所有逻辑。
- **结构化驱动**：以 `widex exec --json` 的 JSONL 事件为主要信号源，而不是基于纯文本 grep。
- **可恢复/可观测**：保留 `.ralph/status.json`、`.ralph/progress.json`、`.ralph/logs/*` 作为稳定外部接口。
- **可控退出**：优先通过 `--output-schema` 强制模型输出结构化 “Ralph 状态” JSON（失败时才 fallback）。
- **单实例锁**：同一 repo 同一时间只允许一个 loop 在跑（避免并发写状态文件/重复调用）。
- **安全边界**：不写入/回显任何 API key；不触碰 `CODEX_SANDBOX_*` 相关逻辑；不在 repo 内放 `CODEX_HOME`。

#### 迁移策略（分阶段，避免破坏）

阶段 0（历史）：早期移植曾以 shell 版脚本验证链路可用性（仅用于参考，不再作为运行路径）。

阶段 1（进行中）：新增 Rust 实现（以 `widex ralph-widex ...` 子命令形式暴露）：

- `widex ralph-widex init`：生成 `.ralph/`（复用 templates，但由 Rust 写入）
- `widex ralph-widex run`：运行 loop（内部调用 `widex exec --json ...`；写 status/progress/log）
- `widex ralph-widex start`：后台启动 loop（不阻塞 TUI/终端；会写入 `.ralph/ralph_widex.pid`）
- `widex ralph-widex stop`：请求停机（创建 `.ralph/STOP`，并 best-effort 发送 SIGTERM）
- `widex ralph-widex monitor`：读取 `.ralph/status.json` + `.ralph/logs/ralph.log`（先 CLI 版，后续可内置到 TUI）
  - `run` 运行时会写入 `.ralph/ralph_widex.pid`（PID 文件；正常退出会删除）

阶段 2：TUI `/ralph-widex` 只调用 Rust 版（不再安装/执行 shell 脚本，也不提供 shell fallback）。

阶段 3：把 monitor/状态面板做成 TUI 内置视图（不再需要独立 `monitor` 进程）。

#### 生产级细节（Rust 版）

- same error 签名检测：loop 会从 `widex exec --json` 的 error item（以及部分 stderr）提取 error 文本并做归一化
  （数字→`<n>`、UUID→`<uuid>`、长 hex/0x→`<hex>`、压缩空白并 lower-case），用于区分“连续同一错误” vs “不同错误”。
  连续同一错误达到阈值会触发 circuit breaker。
- structured output：默认会为每次 `widex exec` 自动附带 `--output-schema .ralph/ralph_output_schema.json`，让模型更倾向输出可解析 JSON。
  可通过 `widex ralph-widex run --no-output-schema` 关闭。
- no last agent message 自动重试：当 `widex exec` exit code 为 0 但 `--output-last-message` 产出空内容时，
  `ralph-widex` 会在同一 loop 内自动重试（默认重试 1 次，可通过 `--retry-no-final-message` 调整；重试会计入 calls/hour）。
- 超时不致命：当单次 `widex exec` 触发 `--timeout-minutes` 超时，Rust 版会将其视为本轮失败（exit code 124）并继续下一轮；
  若持续超时/持续同错，会被熔断器收敛（避免无限消耗）。
- MCP 排查：如遇 rmcp serde / JsonRpcMessage 类错误，可用 `widex ralph-widex run --disable-mcp` 临时禁用已配置的 MCP servers（设置 `mcp_servers.<name>.enabled=false`），避免 JSON-RPC framing 被破坏。
- 继续（resume）旧会话时，Codex 可能会在 stderr 输出 `Custom tool call output is missing for call id: ...` 之类的内部修复日志；ralph-widex 会忽略这类日志，不会将其计入同错熔断。
- graceful shutdown：支持 Ctrl-C（SIGINT）与 SIGTERM；会尝试向子进程发送对应信号并在超时后强制终止，
  同时更新 `.ralph/status.json` 为 `shutdown/exited`。
- progress/monitor：Rust 版会按秒刷新 `.ralph/progress.json`（即使 stdout 没有持续输出），monitor 也会响应 Ctrl-C/SIGTERM 退出。
- 单实例锁（stale lock）：若上一次异常退出遗留 `.ralph/.lock`，Rust 版会尝试判断 PID 是否仍存在；若已不存在会清理 stale lock 并继续。


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
