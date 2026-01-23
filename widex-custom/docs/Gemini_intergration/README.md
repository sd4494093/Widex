# Gemini 集成（Widex 落地 + 参考实现对照）

> 目标：在 **widex** 分支长期开发（运行/测试/使用都在这里），同时可周期性同步上游 `main`，并保留/演进 Gemini 集成与 Widex 自定义配置。

参考实现仓库（用于对照思路）：

- 参考代码路径：`/home/will/data/backups/codex_gemini/codex-with-gemini-integration`

本工作区（当前仓库）落地结果：

- 已在 `codex-rs/` 内实现 `WireApi::Gemini` + 内置 `gemini` provider + Gemini SSE -> Codex `ResponseEvent` 的适配。
- 已把“会话内切到 gemini-* 模型时自动切 provider；切走时清理 data-url 图片”落到 core。
- 已在 `widex-custom/` 补齐“长期维护约束”和“按层阅读/改造模板”，用于后续接入 Grok 等模型。


## 1. 总体架构：把 Gemini 当成一条新的 Wire API

Codex 主干抽象大致是：

- Provider（模型提供方）负责：`base_url`、headers、重试、stream idle timeout 等
- Wire API（线协议）负责：请求/响应 JSON schema 与 streaming 解析
  - OpenAI Responses：`/v1/responses` + SSE
  - OpenAI Chat：`/v1/chat/completions` + SSE
  - Websocket Responses：`/v1/responses` + websocket（本仓已有）

Gemini 集成新增：

- `wire_api = "gemini"`
- 请求走 Gemini JSON API：`/models/{api_model}:streamGenerateContent?alt=sse`
- 把 Gemini SSE 的 `parts`（text/thought/functionCall/inlineData）映射成 Codex 的内部事件流：
  - `ResponseEvent::OutputTextDelta`
  - `ResponseEvent::ReasoningContentDelta`
  - `ResponseItem::FunctionCall`
  - `ResponseItem::Message`（包含 `ContentItem::InputImage` data-url）
  - `ResponseEvent::Completed`

这样做的收益是：尽量复用 Codex 的 session/tool router/TUI，把变更集中在“wire 适配”层。


## 2. 本仓实现：按“配置层 -> 协议层 -> 请求构造 -> 流式解析 -> UI”列出落点

### 2.1 配置/Provider 层（wire_api + 内置 provider）

- `codex-rs/core/src/model_provider_info.rs`
  - `WireApi` 新增 `Gemini`
  - `built_in_model_providers()` 新增内置 provider：`gemini`
    - `GEMINI_BASE_URL`（默认 `https://generativelanguage.googleapis.com/v1beta`）
    - headers：
      - `X-Goog-Api-Key` <- `GEMINI_API_KEY`
      - `Cookie` <- `GEMINI_COOKIE`（可选）
- `codex-rs/core/config.schema.json`
  - 已更新（包含 `wire_api = "gemini"`）

### 2.2 协议层（thoughtSignature 跨回合保留，但不泄漏到其它 wire）

- `codex-rs/protocol/src/models.rs`
  - `ResponseItem::FunctionCall` 增加 `thought_signature: Option<String>`
  - 字段对外序列化被跳过（避免影响 OpenAI / 其它 provider 的请求体）

### 2.3 认证层（auth.json + env 优先级 + fallback）

- `codex-rs/core/src/auth/storage.rs`
  - `AuthDotJson` 增加 `GEMINI_API_KEY` 字段（serde rename）
- `codex-rs/core/src/auth.rs`
  - 读取优先级：`GEMINI_API_KEY`(env) -> `auth.json` 的 `GEMINI_API_KEY` -> `auth.json` 的 `OPENAI_API_KEY`
- `codex-rs/login/src/server.rs`
  - 写入 `AuthDotJson` 时补齐 `gemini_api_key: None`（保持结构完整）

### 2.4 会话层（切模型自动切 provider；切走时清理图片）

- `codex-rs/core/src/codex.rs`
  - 会话配置应用时：`gemini-*` 模型 -> 自动切换到 `gemini` provider
  - 从 `gemini-*` 切回非 gemini：清理历史中的 data-url 图片（减少 payload）
- `codex-rs/core/src/context_manager/history.rs`
  - `replace_all_images(...)`：替换历史里的 `InputImage` / tool output image items 为占位文本

### 2.5 请求构造层（Prompt/Tools -> Gemini JSON）

- `codex-rs/core/src/gemini.rs`
  - 构造 `system_instruction`、`contents`、`tools(functionDeclarations)`、`generation_config`
  - 对 JSON Schema 做清洗（递归移除 `additionalProperties`）
- `codex-rs/core/gemini_prompt.md`
  - Gemini 默认 system instructions

### 2.6 流式解析层（Gemini SSE -> ResponseEvent）

- `codex-rs/core/src/client.rs`
  - `ModelClientSession::stream()` 对 `WireApi::Gemini` 分流到 `crate::gemini::stream_gemini(...)`
- `codex-rs/core/src/gemini.rs`
  - SSE 解析：把 `parts` 逐段转换为 `ResponseEvent`，并在 `FunctionCall` 上保留/回填 `thought_signature`

### 2.7 UI 支持（现状）

- 当前 core 会把 Gemini inlineData image 映射为 `ContentItem::InputImage { image_url: "data:..." }`
- TUI 的“data-url 图片落盘 + /open-image”尚未补齐（后续可按参考实现加入；建议伴随 snapshot 测试一起做）


## 3. 使用方式（最小可用）

环境变量（推荐）：

- `GEMINI_API_KEY`：Gemini key
- `GEMINI_BASE_URL`：可选；默认 `https://generativelanguage.googleapis.com/v1beta`
- `GEMINI_COOKIE`：可选；用于某些代理/特殊场景

模型命名约定：

- `gemini-*` 前缀模型会触发会话内自动切换到 `gemini` provider


## 4. 测试/验证（本仓）

Gemini 集成涉及 core/protocol/codex-api：

- `cd codex-rs && just fmt`
- `cd codex-rs && just write-config-schema`
- `cd codex-rs && just fix -p codex-core`
- `cd codex-rs && cargo test -p codex-protocol`
- `cd codex-rs && cargo test -p codex-api`
- `cd codex-rs && cargo test -p codex-core --lib`

说明：

- `core/tests/all.rs` 包含大量会修改进程环境变量的集成测试；并发执行容易互相干扰。
- 若要运行它，建议：`cd codex-rs && cargo test -p codex-core --test all -- --test-threads=1`。


## 5. 后续：接入 Grok（同一模板）

接入 Grok 等其它模型时，仍按“配置层 -> 协议层 -> 请求构造 -> 流式解析 -> UI 支持”推进：

- 如果 Grok 提供 OpenAI-compatible `/v1/responses`：优先只新增 provider（不新增 wire）
- 如果 Grok 协议/streaming 与 OpenAI/Gemini 都不兼容：新增 `WireApi::Grok` + 新模块 `codex-rs/core/src/grok.rs`，仿照 `gemini.rs`
