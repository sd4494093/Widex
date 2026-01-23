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


## Rust 约束来源

- Rust 代码（`codex-rs/`）的风格/测试/工具约束以仓库根部 `AGENTS.md` 为准。


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
