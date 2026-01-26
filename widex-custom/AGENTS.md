# widex-custom

本 AGENTS.md 的作用域：`widex-custom/` 目录树。

## 目标

- `widex` 分支是日常开发/运行/测试/使用的工作分支。
- `main` 分支尽量保持“接近上游”，只在同步上游时更新。
- `widex-custom/` 用来承载 **Widex 的自定义资产**（文档、配置模板、模型列表、feature 约定），尽量减少对上游代码的侵入，以降低与上游合并时的冲突。

## 约束（后续接入其它模型也要遵循）

### 0. 安全边界（强制）

- **禁止提交任何密钥/令牌**：包括 `sk-...`、cookies、`auth.json`、以及包含真实 key 的 yaml/toml/json。
- `CODEX_HOME` 默认应在仓库外（推荐：官方 `~/.codex`；widex `~/.widex-codex`），不要把 `CODEX_HOME` 指到仓库目录。
- switchover 会把 key 缓存进 `${CODEX_HOME}/auth.json` 的 `WIDEX_SAVED_API_KEYS`（这是 secrets 数据），只能存在于用户机器本地。

### A. 按层拆解（强制）

接入任何新模型/新 Provider，都必须按下面的层次去设计和实现（不要把逻辑散落在各处）：

1) 配置层（Provider / wire_api / 默认值）
2) 认证层（key 的来源与优先级；auth.json / env / 兼容回退）
3) 会话层（模型切换时是否需要自动切 provider；是否需要清理历史以避免 payload 过大）
4) 请求构造层（把 Codex 内部 Prompt/ToolSpec 适配成该 provider 的请求结构）
5) 流式解析层（把 provider 的 streaming 事件适配成 Codex 内部 ResponseEvent）
6) UI 支持层（必要时补齐：图片落盘、命令、状态展示等）

### B. 文档同步（强制）

- 对外可用行为/配置一旦变更，必须在 `widex-custom/docs/` 补齐说明与示例。
- 对“如何同步上游”的流程更新，必须同步更新 `widex-custom/docs/BRANCH_GUIDE.md`。
- 对“切换器（api_switchover.yaml）”的规则/行为变更，必须同步更新：
  - `widex-custom/features/api-switchover/README.md`
  - `widex-custom/features/api-switchover/api_config.example.yaml`

### C. 最小侵入上游代码（强制）

- 优先把 Widex 特性做成：配置（profiles/config 模板）+ 新增模块文件 + 小范围接入点（分流/注册）。
- 避免把 provider 专用逻辑堆到上游的巨型文件里；尽量以新增文件/模块承载。

### D. 官方 codex 与 widex 分离（强制）

- 继续保留官方 npm `codex`，不要让 widex 的配置/schema 破坏它。
- widex 运行请使用 `widex-custom/bin/widex`（它会默认隔离 `CODEX_HOME=~/.widex-codex`，并在缺 binary 时自动 release 构建）。
- MCP 配置也随 `CODEX_HOME` 隔离：widex 用 `~/.widex-codex/config.toml`；官方 npm codex 用 `~/.codex/config.toml`（不要混用/互改）。

### E. Ralph Widex（强制）

- `/ralph-widex` 在 widex 中**只走 Rust 原生实现**；不要再引入 shell 兜底路径。
- `widex-custom/features/ralph-widex/bin/` 与 `lib/` 仅作为历史参考保留（不保证可用性）；任何新能力都应落在 `codex-rs/ralph-widex/`。
- 交互体验：TUI 的 `/ralph-widex start ...` 必须在**当前 Widex 会话内前台迭代**（每轮可见工具调用/输出），不要再默认做成后台 supervisor 进程。
  - `Esc`/`Ctrl+C`：只中断当前 turn，继续下一轮
  - `/ralph-widex stop`：停止整个循环
- 进度落盘：supervisor 每轮 start/end 会写入 `.ralph/.fix_progress.autolog.jsonl` 并重建 `@fix_progress.md` 的 auto log 段落；agent 只应在 Notes 区追加内容。
- 完成词建议：优先使用 `--completion-mode promise-tag` 并要求最终消息输出 `<promise>...</promise>`，减少误触发。

### F. 不在这里放 Rust 代码规范（说明）

- `widex-custom/` 主要承载文档/配置资产。
- Rust 代码（`codex-rs/`）的风格/测试/工具约束以仓库根部 `AGENTS.md` 为准（不要在这里复制一份产生分叉）。
