# Grok 集成（Widex 落地：VectorEngine / Chat Completions）

> 目标：在 **widex** 分支长期开发（运行/测试/使用都在这里），同时可周期性同步上游 `main`，并保留/演进 Grok 集成与 Widex 自定义配置。

本工作区（当前仓库）落地结果：

- 已在 `codex-rs/` 内加入内置 provider：`grok-vectorengine`（VectorEngine 中转，OpenAI Chat Completions 兼容）。
- 已将 `grok-*` 模型加入 picker 预设（例如 `grok-4.1` / `grok-4-1-fast-*`）。
- 已把“会话内切到 grok-* 模型时自动切 provider；切走时回到 openai/openai-proxy”落到 core。
- 已在 Chat Completions 请求构造层对 VectorEngine 的已知限制做 best-effort 兼容（图像输入降级为文本提示）。

安全边界（必须遵守）：

- 不要把任何真实 key 写进 git 管理的文件（含 `widex-custom/`、`.ralph/`、任何 YAML/TOML/JSON）。
- 推荐使用 env：`GROK_API_KEY`，并通过 API Switchover 映射到 `openai_api_key`（避免污染 `OPENAI_API_KEY`）。


## 1. 总体架构：把 Grok 当成一个 Chat Completions Provider

Codex 主干抽象大致是：

- Provider（模型提供方）负责：`base_url`、headers、重试、stream idle timeout 等
- Wire API（线协议）负责：请求/响应 JSON schema 与 streaming 解析
  - OpenAI Responses：`/v1/responses` + SSE
  - OpenAI Chat：`/v1/chat/completions` + SSE
  - Gemini：`/models/{api_model}:streamGenerateContent?alt=sse` + SSE（widex 自增 wire）

Grok（通过 VectorEngine 中转）当前走的是：

- `wire_api = "chat"`（复用既有 OpenAI Chat Completions wire；不新增 wire）
- 请求：`POST https://api.vectorengine.ai/v1/chat/completions`
- streaming：`text/event-stream`，以 `data: {json}` 连续输出，最终以 `data: [DONE]` 结束

这样做的收益是：尽量复用 Codex 的 session/tool router/TUI，把变更集中在“provider 配置 + 少量兼容逻辑”层。


## 2. 本仓实现：按“配置层 -> 协议层 -> 请求构造 -> 流式解析 -> UI”列出落点

### 2.1 配置/Provider 层（内置 grok-vectorengine）

- `codex-rs/core/src/model_provider_info.rs`
  - `built_in_model_providers()` 新增内置 provider：`grok-vectorengine`
    - `base_url`: `https://api.vectorengine.ai/v1`
    - `wire_api`: `chat`
    - `requires_openai_auth = true`（复用 OpenAI 认证槽位：`openai_api_key` -> `Authorization: Bearer ...`）

### 2.2 模型预设层（picker 可见）

- `codex-rs/core/src/models_manager/model_presets.rs`
  - 新增模型预设（show_in_picker = true）：
    - `grok-4.1`
    - `grok-4-1-fast-reasoning`
    - `grok-4-1-fast-non-reasoning`

### 2.3 认证层（auth.json + env 优先级 + Switchover 推荐）

由于 `grok-vectorengine` 走 OpenAI Chat Completions wire，当前使用的认证字段仍是 `openai_api_key`：

- 推荐使用：API Switchover 用 `GROK_API_KEY` 映射到 `openai_api_key`（避免和 OpenAI 官方 key 混用）
  - 示例模板：`widex-custom/features/api-switchover/api_config.example.yaml`

> Widex 会把第一次切换时读到的 key 缓存进 `${CODEX_HOME}/auth.json`（`WIDEX_SAVED_API_KEYS`），后续可以 unset env 仍可切换。

### 2.4 会话层（切模型自动切 provider；切走自动回退）

- `codex-rs/core/src/codex.rs`
  - 当 model 以 `grok-` 开头，且当前 provider 是 `openai/openai-proxy` 时：自动切换到 `grok-vectorengine`
  - 从 `grok-*` 切回非 `grok-*` 时：若当前 provider 为 `grok-vectorengine`，自动回退到 `openai-proxy`（若存在）否则回退到 `openai`

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
- `export GROK_API_KEY=<VectorEngine key>`
- 在 TUI 中使用 `/model grok-4.1`（或其它 `grok-*`）触发自动切换
  - 切换成功后可 `unset GROK_API_KEY`；widex 会使用缓存的 `WIDEX_SAVED_API_KEYS`（见 2.3）

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


## 6. 现状确认：VectorEngine Grok（Chat Completions）目前不支持 tools / MCP 工具调用

结论（截至 2026-02-02，本仓 widex 线上实测）：

- `https://api.vectorengine.ai/v1/chat/completions` 返回的 Grok 输出**不会产生** Chat Completions 的 `tool_calls`（也不会产生 legacy `function_call`）。
- 即使请求里显式传入 `tools` / `tool_choice`（强制指定 tool）或 `functions` / `function_call`，Grok 仍会以普通文本回答，并表现得像“根本没有拿到 tool 列表”（常见回复是 “I don't have a tool …”）。
- 因此：在当前 VectorEngine Grok 接口形态下，Widex 的 MCP 工具（filesystem/shell/…）无法通过“标准 function calling”被 Grok 触发；这不是 Widex 的工具路由 bug，而是上游端点能力缺失/被代理剥离。

验证方法（不写入任何 key；从 `${CODEX_HOME}/auth.json` 读取已保存的 key）：

```bash
python3 - <<'PY'
import json, os, requests

auth=json.load(open(os.path.expanduser("~/.widex-codex/auth.json"),"r",encoding="utf-8"))
key=auth["WIDEX_SAVED_API_KEYS"]["profile:grok-vectorengine:OPENAI_API_KEY"]
url="https://api.vectorengine.ai/v1/chat/completions"
headers={"Authorization":f"Bearer {key}","Content-Type":"application/json"}

params={"type":"object","properties":{"a":{"type":"integer"},"b":{"type":"integer"}},"required":["a","b"],"additionalProperties":False}
body={
  "model":"grok-4.1",
  "messages":[{"role":"user","content":"Use tool math_add with {a: 1, b: 2}. Do not answer normally."}],
  "temperature":0,
  "max_tokens":64,
  "stream":False,
  "tools":[{"type":"function","function":{"name":"math_add","description":"Add two integers.","parameters":params}}],
  "tool_choice":{"type":"function","function":{"name":"math_add"}},
}
r=requests.post(url,headers=headers,json=body,timeout=(10,40))
j=r.json()
choice=j.get("choices",[{}])[0]
msg=choice.get("message",{})
print("finish_reason=",choice.get("finish_reason"))
print("has_tool_calls=",bool(msg.get("tool_calls")))
print("has_function_call=",bool(msg.get("function_call")))
print("content_prefix=",repr((msg.get("content") or "")[:120]))
PY
```

如果 `has_tool_calls=False` 且模型以文本形式说“没有这个 tool”，则说明 `tools` 没有生效。

### 6.1 常见症状：429 Too Many Requests

你可能会看到：

- `■ exceeded retry limit, last status: 429 Too Many Requests`
- 或者 TUI 中出现 `stream disconnected before completion ...`

这是 VectorEngine 侧 rate limit / quota 限制导致的，Widex 会做有限重试；超过重试上限后会报错退出该次请求。

建议：

- 优先使用 `grok-4.1`（`grok-4-1-fast-*` 更容易触发 429，具体视账号配额而定）
- 等待一段时间后重试，或降低并发/请求频率
- 工具调用场景（需要 filesystem/shell/MCP）请切到 `gemini-*` 或 `gpt-*` 模型完成任务

### 6.2 可选后续（如果必须让 Grok “也能用工具”）

如果你必须在 Grok 下使用 MCP 工具，有两条路线（都需要额外开发/或代理支持）：

1) 让 VectorEngine 提供支持 tool calling 的 Grok 端点（或支持 `/v1/responses` 的 function calling），Widex 侧仅需切换 provider/wire。
2) Widex 侧实现“文本协议工具调用”fallback：让 Grok 输出严格标记的 JSON（例如 `TOOL_CALL: {...}`），由 Widex 解析后执行 MCP，再把 tool 输出回填给模型继续推理。
