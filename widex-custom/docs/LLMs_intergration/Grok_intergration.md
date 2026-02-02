# Grok 集成（Widex 落地：VectorEngine + xAI 官方 / Chat Completions）

> 目标：在 **widex** 分支长期开发（运行/测试/使用都在这里），同时可周期性同步上游 `main`，并保留/演进 Grok 集成与 Widex 自定义配置。

本工作区（当前仓库）落地结果：

- 已在 `codex-rs/` 内加入内置 providers：
  - `grok-vectorengine`（VectorEngine 中转，OpenAI Chat Completions 兼容）
  - `grok-xai`（xAI 官方，OpenAI Chat Completions 兼容）
- 已将 `grok-*` 模型加入 picker 预设（例如 `grok-4.1` / `grok-4-1-fast-*`）。
- 已把“会话内切到 grok-* 模型时自动切 provider；切走时回到 openai/openai-proxy”落到 core。
- 已在 Chat Completions 请求构造层对 VectorEngine 的已知限制做 best-effort 兼容（图像输入降级为文本提示）。

安全边界（必须遵守）：

- 不要把任何真实 key 写进 git 管理的文件（含 `widex-custom/`、`.ralph/`、任何 YAML/TOML/JSON）。
- 推荐使用 env：`GROK_API_KEY`（VectorEngine）/ `XAI_API_KEY`（xAI 官方），并通过 API Switchover 映射到 `openai_api_key`（避免污染 `OPENAI_API_KEY`）。


## 1. 总体架构：把 Grok 当成一个 Chat Completions Provider

Codex 主干抽象大致是：

- Provider（模型提供方）负责：`base_url`、headers、重试、stream idle timeout 等
- Wire API（线协议）负责：请求/响应 JSON schema 与 streaming 解析
  - OpenAI Responses：`/v1/responses` + SSE
  - OpenAI Chat：`/v1/chat/completions` + SSE
  - Gemini：`/models/{api_model}:streamGenerateContent?alt=sse` + SSE（widex 自增 wire）

Grok 目前分两条线路（都走 Chat Completions wire）：

- `wire_api = "chat"`（复用既有 OpenAI Chat Completions wire；不新增 wire）
- 请求（VectorEngine / `grok-vectorengine`）：`POST https://api.vectorengine.ai/v1/chat/completions`
- 请求（xAI 官方 / `grok-xai`）：`POST https://api.x.ai/v1/chat/completions`
- streaming：`text/event-stream`，以 `data: {json}` 连续输出，最终以 `data: [DONE]` 结束

这样做的收益是：尽量复用 Codex 的 session/tool router/TUI，把变更集中在“provider 配置 + 少量兼容逻辑”层。


## 2. 本仓实现：按“配置层 -> 协议层 -> 请求构造 -> 流式解析 -> UI”列出落点

### 2.1 配置/Provider 层（内置 grok-vectorengine + grok-xai）

- `codex-rs/core/src/model_provider_info.rs`
  - `built_in_model_providers()` 新增内置 providers：
    - `grok-vectorengine`
      - `base_url`: `https://api.vectorengine.ai/v1`
      - `wire_api`: `chat`
      - `requires_openai_auth = true`（复用 OpenAI 认证槽位：`openai_api_key` -> `Authorization: Bearer ...`）
    - `grok-xai`
      - `base_url`: `https://api.x.ai/v1`
      - `wire_api`: `chat`
      - `requires_openai_auth = true`

### 2.2 模型预设层（picker 可见）

- `codex-rs/core/src/models_manager/model_presets.rs`
  - 新增模型预设（show_in_picker = true）：
    - `grok-4.1`
    - `grok-4-1-fast-reasoning`
    - `grok-4-1-fast-non-reasoning`

### 2.3 认证层（auth.json + env 优先级 + Switchover 推荐）

由于 `grok-vectorengine` / `grok-xai` 都走 OpenAI Chat Completions wire，当前使用的认证字段仍是 `openai_api_key`：

- 推荐使用：API Switchover 映射到 `openai_api_key`
  - VectorEngine：`GROK_API_KEY`
  - xAI 官方：`XAI_API_KEY`
  - 示例模板：`widex-custom/features/api-switchover/api_config.example.yaml`

> Widex 会把第一次切换时读到的 key 缓存进 `${CODEX_HOME}/auth.json`（`WIDEX_SAVED_API_KEYS`），后续可以 unset env 仍可切换。

### 2.4 会话层（切模型自动切 provider；切走自动回退）

- `codex-rs/core/src/codex.rs`
  - 当 model 为 `grok-4-1-fast-*` 且当前 provider 是 `openai/openai-proxy` 时：自动切换到 `grok-xai`
  - 当 model 以 `grok-` 开头（非 fast 变体）且当前 provider 是 `openai/openai-proxy` 时：自动切换到 `grok-vectorengine`
  - 从 `grok-*` 切回非 `grok-*` 时：若当前 provider 为 `grok-vectorengine`/`grok-xai`，自动回退到 `openai-proxy`（若存在）否则回退到 `openai`

### 2.5 请求构造层（Prompt/Tools -> Chat Completions JSON；图像降级）

- `codex-rs/codex-api/src/requests/chat.rs`
  - 复用 Chat Completions 的 messages/tools 构造逻辑
  - VectorEngine 的 Grok 端点当前对 `content: [{type:\"image_url\", ...}]` payload 可能会忽略（文本-only）。因此当 model 以 `grok-` 开头且输入包含图片时：
    - 不发送多模态结构化 `image_url` 数组
    - 追加 best-effort 文本提示：`[image_url: ...]`（避免整条用户消息“被吃掉”）

### 2.6 流式解析层（Chat SSE -> ResponseEvent）

- `codex-rs/codex-api/src/sse/chat.rs`
  - 解析 Chat Completions SSE：
    - `choices[].delta.content`（string 或 array） -> `ResponseEvent::OutputTextDelta`
    - `choices[].delta.reasoning`（若存在） -> `ResponseEvent::ReasoningContentDelta`
    - `choices[].delta.tool_calls` + `finish_reason = tool_calls` -> `ResponseItem::FunctionCall`
    - `data: [DONE]` 或连接 close -> `ResponseEvent::Completed`

### 2.7 UI 支持（现状）

- picker 中可直接选择 `grok-*` 预设模型
- 图像输入在 `grok-*` 上会降级为文本提示（见 2.5）；如需真正的多模态，需要 VectorEngine 侧支持并在请求构造层放开


## 3. 使用方式（最小可用）

推荐方式（和 OpenAI 官方 key 解耦）：

- 配置 `${CODEX_HOME}/api_switchover.yaml`（可从 `widex-custom/features/api-switchover/api_config.example.yaml` 拷贝）
- 设置 key（两选一）：
  - VectorEngine（`grok-4.1`）：`GROK_API_KEY`
  - xAI 官方（`grok-4-1-fast-*`）：`XAI_API_KEY`
- 在 TUI 中使用 `/model <grok-model>` 触发 switchover
  - 切换成功后可 unset env；widex 会使用缓存的 `WIDEX_SAVED_API_KEYS`（见 2.3）

可选（非交互）：

- `cd codex-rs && cargo run -p codex-api-switchover -- --config ${CODEX_HOME}/api_switchover.yaml apply --model grok-4-1-fast-non-reasoning --set-model`
  - 作用：把 switchover 的 provider + key 写入 `${CODEX_HOME}`（不在仓库里落盘任何 secret）

### 3.1 TUI / CLI 冒烟验证（推荐）

目标：用最少步骤确认 **模型切换 + 流式输出** 正常工作。

1) 启动 TUI（两种方式任选其一）：

- 已安装二进制：运行 `codex`
- 源码运行：`cd codex-rs && cargo run --bin codex --`

2) 在 TUI 输入框中执行：

- `/model grok-4-1-fast-non-reasoning`（或 `grok-4.1` / 其它 `grok-*`）
- 然后发送一句话，例如：`ping`

3) 验收标准：

- 能看到逐步流式输出（而不是卡住/一次性返回）。
- 结束后能正常回到可输入状态（流式完成）。

可选（非交互）：

- `codex exec "ping"`（用于快速验证能跑通一轮请求；不覆盖 TUI 的渲染/交互路径）


## 4. 测试/验证（本仓）

Grok 集成通常只涉及 core/codex-api（不新增 wire）：

- `cd codex-rs && just fmt`
- `cd codex-rs && cargo test -p codex-api`
- `cd codex-rs && cargo test -p codex-core --test all list_models_returns -- --test-threads=1`

说明（Linux）：

- `codex-core` 的集成测试需要 workspace 二进制 `codex-linux-sandbox`；若本机尚未构建过，可先运行：
  - `cd codex-rs && cargo build -p codex-linux-sandbox --bin codex-linux-sandbox`


## 5. 后续：按同一模板继续演进（Grok “更多能力”）

按“配置层 -> 协议层 -> 请求构造 -> 流式解析 -> UI 支持”推进，候选项：

- tools/function calling：tool schema、tool choice、tool-call delta 的兼容性验证
- 多模态：image input / image output（若 VectorEngine 支持，需要解除 2.5 的降级逻辑）
- reasoning 参数映射：effort/temperature/top_p 等与 Grok 参数如何对应
- token/usage 解析：stream/非 stream 情况下 usage 的提取与 UI 展示
- 错误码/重试策略：429/5xx 的指数退避、请求超时、SSE 断流重连等


## 6. 现状确认：VectorEngine Grok（grok-4.1）工具调用需要使用 fast-reasoning 上游模型

结论（截至 2026-02-02，本仓 widex 线上实测）：

- VectorEngine 线路的函数调用示例里：即使“前台模型”写的是 `grok-4.1`，**代理侧用于 function calling 的模型是**
  `grok-4-fast-reasoning`，并且需要标准的 `tools` / `tool_choice` 结构。
- Widex 已按该约定做了兼容：当你选择 `grok-4.1` 且本轮存在 tools（MCP 工具）时，Widex 会保持 provider 为
  VectorEngine，但把本轮上游请求模型临时改为 `grok-4-fast-reasoning`，以触发 `tool_calls`（若 429，则回退到
  `grok-4-fast-non-reasoning` 再试一次）。
- 限制：VectorEngine 侧部分 grok-* “fast” 线路在部分账号/时段可能会频繁 429（见 6.2）。

实现点：

- OpenAI 标准 chat tools schema（兼容 `api.x.ai` 与 `api.vectorengine.ai`）：`codex-rs/core/src/tools/spec.rs`
- VectorEngine `grok-4.1` + tools 时上游模型切换：`codex-rs/core/src/client.rs`

### 6.1 xAI 官方线路（grok-4-1-fast-*）支持 tool_calls

对 `grok-4-1-fast-reasoning` / `grok-4-1-fast-non-reasoning`，Widex 会通过 `grok-xai` provider 走
`https://api.x.ai/v1/chat/completions`。该线路支持标准 Chat Completions 的 `tools` / `tool_calls`，因此能触发 Widex 的 MCP 工具调用。

### 6.2 常见症状：429 Too Many Requests

你可能会看到：

- `■ exceeded retry limit, last status: 429 Too Many Requests`
- 或者 TUI 中出现 `stream disconnected before completion ...`

这是 VectorEngine 侧 rate limit / quota 限制导致的，Widex 会做有限重试；超过重试上限后会报错退出该次请求。

建议：

- 如果 VectorEngine 线路频繁 429：
  - 降低并发/请求频率，或稍后再试
  - 若你持有 xAI 官方 key，优先使用 `grok-4-1-fast-*`（走 `https://api.x.ai/v1`）
- 工具调用场景（需要 filesystem/shell/MCP）优先选 `gemini-*` / `gpt-*`；VectorEngine 线路可能受 429 影响（见 6.2）

### 6.3 可选后续（如果必须让 Grok “也能用工具”）

如果你必须在 Grok 下使用 MCP 工具，有两条路线（都需要额外开发/或代理支持）：

1) 让 VectorEngine 提供支持 tool calling 的 Grok 端点（或支持 `/v1/responses` 的 function calling），Widex 侧仅需切换 provider/wire。
2) Widex 侧实现“文本协议工具调用”fallback：让 Grok 输出严格标记的 JSON（例如 `TOOL_CALL: {...}`），由 Widex 解析后执行 MCP，再把 tool 输出回填给模型继续推理。
