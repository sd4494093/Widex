# Gemini 集成：参考实现解析 + Widex 分支落地/维护策略

> 目标：在 **widex** 分支长期开发（运行/测试/使用都在这里），同时可周期性同步上游 `main`，并保留/演进 Gemini 集成与 Widex 自定义配置。

本文基于参考实现仓库：

- 参考代码路径：`/home/will/data/backups/codex_gemini/codex-with-gemini-integration`
- 该实现的核心思路：在 Codex 的“Provider + Wire API”抽象之上，新增 **Gemini 原生 JSON API** 的 wire protocol，并把 Gemini 的 SSE 流转换为 Codex 内部的 `ResponseEvent` 流（从而复用现有 TUI/CLI/工具调用链路）。


## 1. 总体架构（在 Codex 里把 Gemini 当成第三条 Wire API）

上游 Codex 的主干通常是：

- Provider（模型提供方）负责：`base_url`、headers、重试、stream idle timeout 等
- Wire API（线协议）负责：请求/响应 JSON schema 与 streaming 解析
  - OpenAI Responses：`/v1/responses` + SSE
  - OpenAI Chat：`/v1/chat/completions` + SSE（或聚合）

Gemini 集成把它扩展为：

- `wire_api = "gemini"`
- 请求走 Gemini 的 `:streamGenerateContent?alt=sse`
- 把 Gemini SSE 的增量 `parts`（text/thought/functionCall/inlineData）映射成 Codex 的：
  - `ResponseEvent::OutputTextDelta`
  - `ResponseEvent::ReasoningContentDelta`
  - `ResponseItem::FunctionCall`
  - `ResponseItem::Message`（最终落盘）
  - `ResponseEvent::Completed`

这样做的收益是：**不需要重写 Codex 的 session、tool router、TUI 渲染**；只要把“Gemini 的 wire”适配到既有内部事件流即可。


## 2. 参考实现的关键改动点（按模块拆解）

下面的“文件路径/行号”均指参考仓库 `/home/will/data/backups/codex_gemini/codex-with-gemini-integration`（不是当前 widex 工作区）。

### 2.1 配置/Provider：新增 Gemini provider + `WireApi::Gemini`

- `codex-rs/core/src/model_provider_info.rs:41`
  - `enum WireApi { Responses, Chat, Gemini }`
- `codex-rs/core/src/model_provider_info.rs:300`
  - `built_in_model_providers()` 内置 `gemini` provider
  - 默认 `base_url = https://api.ppchat.vip/v1beta`（也可用 `GEMINI_BASE_URL` 覆盖）
  - `env_http_headers` 支持：
    - `X-Goog-Api-Key <- GEMINI_API_KEY`
    - `Cookie <- GEMINI_COOKIE`
  - `auth_json_key = "GEMINI_API_KEY"`（从 auth.json 里读 Gemini key）
  - `requires_openai_auth = true` 用于触发“可复用 OpenAI 的 auth.json key”这个 fallback 逻辑

**要点**：Gemini provider 本身只负责“怎么连、怎么鉴权、有什么默认值”。真正的请求/解析逻辑由 core client 的 `stream_gemini()` 实现。


### 2.2 认证：auth.json 支持 `GEMINI_API_KEY`，并能回退复用 `OPENAI_API_KEY`

- `codex-rs/core/src/auth/storage.rs:34`
  - `AuthDotJson` 增加 `GEMINI_API_KEY` 字段
- `codex-rs/core/src/auth.rs:311`
  - `read_gemini_api_key_from_auth_json(...)`：优先 `GEMINI_API_KEY`，否则回退到 `OPENAI_API_KEY`
- `codex-rs/core/src/client.rs:486`
  - `stream_gemini()` 中，优先 env 的 `GEMINI_API_KEY`，否则从 auth.json 读（再回退共享 OpenAI key）

**设计取舍**：
- 让 Gemini 可以“独立 key”（环境变量）
- 也允许“复用 Codex 登录/已有 key”（auth.json 里的 OPENAI_API_KEY）


### 2.3 会话内 provider 自动切换（Gemini <-> OpenAI）

参考实现的目标是：用户在 TUI/CLI 里换模型时，不用手动切换 provider。

- `codex-rs/core/src/config/mod.rs:441`
  - `preferred_model_provider_id_for_model(current_provider_id, model)`
  - 规则：
    - 选 `gemini-*` → 尽量切到 `gemini` provider
    - 从 `gemini` 切到非 gemini → 切回 `openai-proxy`（优先）或 `openai`
- `codex-rs/core/src/codex.rs:451`
  - `SessionConfiguration::apply()` 调用上面的规则
  - 额外逻辑：从 gemini 切走时清掉 history 中的 images（避免把巨大 data-url 带到 OpenAI 侧导致 429）


### 2.4 Core：`stream_gemini()`（请求构造 + 发送 + 重试）

- 分流入口：`codex-rs/core/src/client.rs:294`
  - `ModelClient::stream()` 根据 `provider.wire_api` 分流
- Gemini 主逻辑：`codex-rs/core/src/client.rs:359`
  - endpoint：`/models/{api_model}:streamGenerateContent?alt=sse`
  - 模型名适配：对 `-codex/-germini/-gemini` 后缀 strip，得到真实 `api_model`
  - 构造 `GeminiRequest { system_instruction, contents, tools, tool_config, generation_config, safety_settings }`
  - `generation_config` 对 Gemini 3 thinkingLevel/thinkingBudget 做兼容（并对 image 模型省略 thinkingConfig）
  - HTTP：使用 `reqwest` 直接发起 POST
  - 重试：对 429/5xx/网络错误做指数退避（最多 3 次）


### 2.5 Core：工具声明/Schema 适配（ToolSpec -> functionDeclarations）

- `codex-rs/core/src/client.rs:1773`
  - `build_gemini_tools(tools: &[ToolSpec]) -> Option<Vec<GeminiTool>>`
  - 把 Codex 的 `ToolSpec::Function` 转成 Gemini `functionDeclarations`
  - 递归删除 JSON Schema 中的 `additionalProperties`（Gemini function schema 不认该字段）

此外还做了一个“首回合强制只允许读工具”的控制：

- `codex-rs/core/src/client.rs:173`
  - `GEMINI_READ_ONLY_TOOL_NAMES` 白名单
- `codex-rs/core/src/client.rs:186`
  - `build_gemini_tool_config(...)` 生成 `function_calling_config`：
    - `mode = any + allowed_function_names = [read tools...]`
    - 触发条件可通过 `CODEX_GEMINI_FORCE_READ_TOOLS_FIRST_TURN` 控制


### 2.6 Core：SSE 流转换（Gemini SSE -> Codex ResponseEvent）

- `codex-rs/core/src/client.rs:922`
  - `spawn_gemini_sse_stream(...)` + `process_gemini_sse(...)`
- `process_gemini_sse` 的核心映射：
  - `part.text`（非 thought）→ `ResponseEvent::OutputTextDelta`
  - `part.thought == true` 且文本有意义 → `ResponseEvent::ReasoningContentDelta`
  - `part.function_call` → 累积成 `ResponseItem::FunctionCall`（生成稳定的 call_id）
  - `part.inline_data`（图片）→ 最终在 `ResponseItem::Message` 里作为 `ContentItem::InputImage { image_url: data:... }`

最终回合结束时发：
- `ResponseEvent::OutputItemDone(ResponseItem::Message { ... })`
- `ResponseEvent::OutputItemDone(ResponseItem::FunctionCall { ... })`（逐个）
- `ResponseEvent::Completed { ... }`


### 2.7 “thoughtSignature”兼容（Gemini 3 工具调用的关键坑）

Gemini 3 在多轮工具调用链路里，常要求 function call 带 `thoughtSignature`（否则可能 400/429）。参考实现做了两层处理：

1) **在 Codex 内部对象里保留 thought_signature**，但不要把它发给非 Gemini provider
- `codex-rs/protocol/src/models.rs:102`
  - `ResponseItem::FunctionCall` 增加 `thought_signature: Option<String>`
  - 对 `Message` 也保留一个内部 thought_signature（用于 replay）

2) **对 Gemini 请求补齐/合成 thoughtSignature**
- `codex-rs/core/src/client.rs:1591`
  - `ensure_active_loop_has_thought_signatures(contents)`：
    - 对 functionCall 的第一个 part 缺失时补一个合成 signature
    - 对 inline_data（图片）part 也补齐

3) **对非 Gemini provider 剥离 thought_signature**
- `codex-rs/core/src/client.rs:1829`
  - `strip_thought_signatures_from_input(input)`：避免把未知字段发给 OpenAI Responses 等


### 2.8 TUI：Gemini 图片输出落盘 + `/open-image`

Gemini 图片模型可能以 `data:<mime>;base64,<data>` 的形式把图片塞进 message content。

- `codex-rs/tui/src/chatwidget.rs:3307`
  - 监听 assistant message 的 `ContentItem::InputImage`
  - 保存到 `~/.codex/images/<conversation_id>/000000.<ext>`
  - 提示用户用 `/open-image` 打开最近生成的图片


## 3. Widex 分支：如何“跟随上游更新”同时“保留 Gemini 集成 + 自定义配置”

我们当前的分支策略（已写在 `widex-custom/docs/BRANCH_GUIDE.md`）：

- **日常开发一直在 `widex` 分支**（运行/测试/使用都在这里）
- **`main` 分支保持尽量干净**：只在同步上游时切换到 `main`

手动同步流程（你给出的版本）：

```bash
# 每周或每月同步一次上游（5分钟操作）

git checkout main
git fetch upstream
git merge upstream/main
git push origin main

git checkout widex
git merge main
# 解决冲突（如果有）
git push origin widex
```

### 3.1 建议：把 Gemini 集成拆成“相对独立的 commit/模块”，降低 merge 冲突成本

参考实现对 core/protocol/tui/config 都有改动，这些文件通常是上游也会频繁变动的冲突热点。

为了让 `widex` 能长期低成本跟随上游，建议把 Gemini 集成在 widex 里按下面方式组织：

- **尽量新增文件/模块，而不是在巨型文件里堆很多 Gemini 专用代码**
  - 例如把 Gemini wire 的请求结构体、SSE 解析、tool schema 适配拆到：
    - `codex-rs/core/src/gemini/`（新增目录）
  - 然后在 `ModelClient::stream()` 里只保留一层分流调用：`gemini::stream(...)`

- **把改动拆成可独立 cherry-pick 的 commit 序列**（方便以后从参考实现或旧版挪动/重做）：
  1. protocol：引入 `thought_signature`（内部字段）
  2. core/config：引入 `WireApi::Gemini`、provider 字段扩展（如需 auth_json_key）
  3. core：实现 `stream_gemini` + SSE 转换
  4. core：工具声明 schema 适配、read-tools-first-turn
  5. tui：图片落盘 + `/open-image`
  6. docs：补齐 config 示例、行为说明

- **Widex 自定义配置**尽量留在 `widex-custom/`（你们已经这么做了）
  - 这样同步上游时，`widex-custom/` 理论上几乎不冲突


### 3.2 建议：把“我们自己的定制配置”做成 profile + config layer（避免改上游默认值）

上游 Codex 通常允许在 `~/.codex/config.toml` 定义：
- `model` / `model_provider`
- `model_providers.<id>`
- `profiles.<name>`

Widex 的定制配置建议落在：
- `widex-custom/configs/`：提供模板/示例
- `widex-custom/features/`：记录启用的 feature flags
- `widex-custom/models/`：记录模型/别名/默认策略

然后在 docs 给出：
- `profiles.gemini`（直连 Gemini provider）
- `profiles.codex`（OpenAI 或 proxy）
- `profiles.safe`（更严格的 sandbox/approval）

这样“享受 Gemini 集成”主要靠 profile 切换，不必把默认行为写死在代码里。


## 4. 迁移落地清单（把参考实现移植到当前 widex 工作区时）

> 注意：当前 widex 工作区（`/home/will/data/codex`）的上游代码结构与参考实现不完全一致（例如 `WireApi` 目前包含 `ResponsesWebsocket`，且尚未出现 `WireApi::Gemini`）。移植时应以“最小侵入 + 新增模块”为原则。

建议按以下清单执行（便于逐步可用、逐步测试）：

1) **协议层**（protocol）
- 给 `ResponseItem::FunctionCall` 增加内部 `thought_signature` 字段（对 wire 不序列化/或仅 Gemini 路径使用）

2) **配置层**（core/config）
- `WireApi` 增加 `Gemini`
- provider 增加 Gemini 所需字段（如果希望从 auth.json 选 key：需要 `auth_json_key` 或等价机制）
- built-in providers 加 `gemini`

3) **认证层**（core/auth）
- auth.json schema 增加 `GEMINI_API_KEY`
- 读取优先级：env GEMINI_API_KEY → auth.json GEMINI_API_KEY → auth.json OPENAI_API_KEY

4) **请求/解析层**（core/client）
- 实现 `stream_gemini()`
- 实现 `process_gemini_sse()`（SSE -> ResponseEvent）
- tool declarations + schema 清洗（strip additionalProperties）

5) **TUI**（可选但强烈推荐）
- data-url 图片落盘 + `/open-image`

6) **测试**
- unit tests：schema 清洗、thought_signature 补齐、contents 构造
- tui snapshot：模型选择、状态栏等（若 UI 变更）


## 5. 相关文档入口

- 分支同步工作流：`widex-custom/docs/BRANCH_GUIDE.md`
- Widex 自定义功能总览：`widex-custom/docs/WIDEX_CUSTOM.md`


## 6. 备注：为什么本文仍然有价值（即使当前 widex 还没真正集成 Gemini）

- 它明确了“Gemini 集成需要改哪些层、为什么要改、哪里最容易踩坑（thoughtSignature）”。
- 它提供了一套“把大改动做成可维护 patch set”的组织方式，降低长期同步上游的成本。
- 你们后续要加的“其他定制配置”，也能沿用同一套思路：
  - 尽量把定制放进 `widex-custom/`
  - 把行为控制放进 profile/config，而不是把默认行为写死在 core
