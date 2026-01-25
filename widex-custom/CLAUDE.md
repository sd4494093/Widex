# Widex：模型接入与长期维护约束（给人/AI 的统一说明）

本仓库采用“上游跟随 + widex 定制”策略：

- **widex**：日常开发/运行/测试/使用都在这里
- **main**：尽量保持干净，只用于接收上游更新

同步上游（手动流程，5 分钟级）：

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

## Dual CLI（强制遵守）

目标：官方 npm `codex` 与 widex fork 共存，互不干扰。

- 官方 npm `codex`：默认使用 `~/.codex`
- widex：使用 `widex-custom/bin/widex` 启动，默认使用 `~/.widex-codex`

原因：widex 增加了官方 npm CLI 不识别的配置扩展（例如 `wire_api = "gemini"`）。共用同一个 `CODEX_HOME`
会互相“读坏配置”。

更多说明见：`widex-custom/docs/DUAL_CLI.md`。


## 统一约束：新增任何模型/Provider 必须按层实现

接入新模型（Gemini/Grok/Claude/OpenRouter/本地模型等）时，必须按下面的顺序拆解，不允许把逻辑散落在各处：

1) 配置层（Provider）
- 增加/扩展 `wire_api`（如果需要新的线协议）
- 增加/注册 provider 默认值（base_url / headers / retry / idle timeout）

2) 协议层（Protocol）
- 只有当 wire 需要携带“跨回合状态”时才扩展协议类型（例如 Gemini 的 `thoughtSignature`）
- **不允许**把 provider 私有字段泄漏到其它 wire_api 的请求里；必要时在序列化层跳过

3) 认证层（Auth）
- 定义 key 的来源与优先级：env -> auth.json -> 兼容回退（如复用 OPENAI_API_KEY）
- 明确“专用 key”与“共享 key”的行为，并在 docs 写清楚

4) 会话层（Model <-> Provider 自动切换）
- 如果 UI/用户会在会话中切模型，必须定义是否自动切 provider
- 若不同 provider 的 payload 形态差异大（如包含 data-url 图片），需要定义切换时的历史清理策略

5) 请求构造层（Prompt/Tools 适配）
- 把 Codex 内部 `Prompt` / `ToolSpec` 适配成 provider 的请求结构
- 如 provider 对 schema 有限制（例如不支持 `additionalProperties`），必须在这一层做清洗

6) 流式解析层（Streaming -> ResponseEvent）
- 把 provider 的 streaming 事件转换为 Codex 内部统一的 `ResponseEvent`（OutputTextDelta/FunctionCall/Completed 等）
- 这是让 TUI/CLI/工具链路复用的关键

7) UI 支持层（必要时补齐）
- 图片/文件类输出需要落盘时：定义保存路径、命令入口（如 /open-image）、以及用户提示
- UI 变更需要更新 snapshot（若项目使用 insta）

## 密钥/鉴权（强制遵守）

- 永远不要把任何真实 key/令牌写进 git（包括 `sk-...`、cookies、`auth.json`、包含 key 的本地 yaml/toml）。
- switchover 允许在 `auth.json` 缓存多份 key：
  - `OPENAI_API_KEY` / `GEMINI_API_KEY`：当前生效 key（历史上一直存在）
  - `WIDEX_SAVED_API_KEYS`：按 profile 缓存多份 key，避免切换时丢失
- switchover YAML 中推荐使用 `env: XXX_API_KEY`，并允许：
  - env 缺失时回退到 `WIDEX_SAVED_API_KEYS` 中之前保存的 key（若存在）


## Rust 约束来源

- Rust 代码（`codex-rs/`）的风格/测试/工具约束以仓库根部 `AGENTS.md` 为准。

## Ralph Widex（本仓）

- TUI 内置命令：`/ralph-widex`（可带参数：`/ralph-widex init|run|monitor ...`）。
- 实现形态：只使用 Rust 原生实现（不安装/不执行 shell 脚本；shell 目录仅作历史参考）。
- 运行约定：在项目根目录生成 `.ralph/`；可用 `.ralph/STOP` 或 `/ralph-widex stop` 请求停机。
- 交互默认：TUI 的 `/ralph-widex start ...` 在**当前 Widex 会话内前台迭代**（每轮可见完整交互/工具调用），直到跑满轮次或命中完成词。
  - `Esc`/`Ctrl+C`：只中断当前 turn，继续下一轮
  - `/ralph-widex stop`：停止整个循环


## Gemini（本仓）落地检查清单（按层读代码）

1) 配置/Provider：新增 `wire_api = "gemini"` + 内置 `gemini` provider
- `codex-rs/core/src/model_provider_info.rs`
  - `WireApi` 新增 `Gemini`
  - `built_in_model_providers()` 内置 `gemini` provider（默认 `GEMINI_BASE_URL`；headers：`GEMINI_API_KEY` -> `X-Goog-Api-Key`，可选 `GEMINI_COOKIE`）

2) 认证：`auth.json` 支持 `GEMINI_API_KEY`，并可回退复用 `OPENAI_API_KEY`
- `codex-rs/core/src/auth/storage.rs`: `AuthDotJson` 增加 `GEMINI_API_KEY`
- `codex-rs/core/src/auth.rs`: 读取优先级：env `GEMINI_API_KEY` -> auth.json `GEMINI_API_KEY` -> auth.json `OPENAI_API_KEY`

3) 会话层：模型切换时自动切 provider + 清理历史图片
- `codex-rs/core/src/codex.rs`: `gemini-*` 触发切 provider；从 gemini 切走时清理历史 data-url 图片
- `codex-rs/core/src/context_manager/history.rs`: `replace_all_images(...)`

4) 核心实现：Gemini 请求构造 + 发送 + SSE 转换
- 分流入口：`codex-rs/core/src/client.rs`（`WireApi::Gemini => crate::gemini::stream_gemini(...)`）
- Gemini wire：`codex-rs/core/src/gemini.rs`
  - URL：`/models/{api_model}:streamGenerateContent?alt=sse`
  - `ToolSpec::Function` -> Gemini `functionDeclarations`
  - 递归删除 JSON Schema 里的 `additionalProperties`
  - SSE parts（text/thought/functionCall/inlineData）-> `ResponseEvent`

5) `thoughtSignature` 兼容
- `codex-rs/protocol/src/models.rs`: `ResponseItem::FunctionCall.thought_signature`
- `codex-rs/core/src/gemini.rs`: 在 SSE 解析/请求构造中保留并回填

6) UI
- 目前 core 会产出 `ContentItem::InputImage { image_url: "data:..." }`
- TUI 的“落盘 + /open-image”尚未加入（后续按需补齐）

## Grok（本仓）落地检查清单（按层读代码）

Grok（通过 VectorEngine 中转）目前按 OpenAI Chat Completions 兼容接入：

1) 配置/Provider：新增内置 provider `grok-vectorengine`
- `codex-rs/core/src/model_provider_info.rs`
  - 内置 `grok-vectorengine`
  - `base_url = https://api.vectorengine.ai/v1`
  - `wire_api = "chat"`（`/v1/chat/completions` + SSE）

2) 模型预设（picker 可见）
- `codex-rs/core/src/models_manager/model_presets.rs`
  - `grok-4.1`
  - `grok-4-1-fast-reasoning`
  - `grok-4-1-fast-non-reasoning`

3) switchover 规则（/model 切换时自动换 provider + key）
- YAML 模板：`widex-custom/features/api-switchover/api_config.example.yaml`
  - `grok-` 前缀 -> `grok-vectorengine` profile
  - profile 的 `auth.openai_api_key` 推荐从 `GROK_API_KEY` env 读取（首次切换后会缓存进 `WIDEX_SAVED_API_KEYS`）


## 测试建议（本仓）

- `cd codex-rs && just fmt`
- `cd codex-rs && just write-config-schema`（如 wire_api/schema 有变）
- `cd codex-rs && just fix -p codex-core`
- `cd codex-rs && cargo test -p codex-protocol`
- `cd codex-rs && cargo test -p codex-api`
- `cd codex-rs && cargo test -p codex-core --lib`

说明：

- `codex-core` 的集成测试集合在 `core/tests/all.rs`，其中包含会修改进程环境变量的测试；并发执行容易互相干扰。
- 若要跑它，建议：`cd codex-rs && cargo test -p codex-core --test all -- --test-threads=1`。


## 下一步：Grok “更多能力”（先明确范围）

按同一模板继续推进时，请先确认你希望优先补哪些能力（可多选）：

- tools/function calling：tool schema、tool-choice、tool-call streaming delta 的兼容性与回填策略
- 多模态：image input/output（若 VectorEngine 端支持）
- reasoning 参数映射：effort/temperature/top_p 等的映射规则与默认值
- token/usage 解析：非 stream + stream 情况下 usage 的提取与展示
- 错误码/重试策略：429/5xx、SSE 断流重连、请求超时与错误归因
