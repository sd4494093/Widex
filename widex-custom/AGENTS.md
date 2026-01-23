# widex-custom

本 AGENTS.md 的作用域：`widex-custom/` 目录树。

## 目标

- `widex` 分支是日常开发/运行/测试/使用的工作分支。
- `main` 分支尽量保持“接近上游”，只在同步上游时更新。
- `widex-custom/` 用来承载 **Widex 的自定义资产**（文档、配置模板、模型列表、feature 约定），尽量减少对上游代码的侵入，以降低与上游合并时的冲突。

## 约束（后续接入其它模型也要遵循）

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

### C. 最小侵入上游代码（强制）

- 优先把 Widex 特性做成：配置（profiles/config 模板）+ 新增模块文件 + 小范围接入点（分流/注册）。
- 避免把 provider 专用逻辑堆到上游的巨型文件里；尽量以新增文件/模块承载。

### D. 不在这里放 Rust 代码规范（说明）

- `widex-custom/` 主要承载文档/配置资产。
- Rust 代码（`codex-rs/`）的风格/测试/工具约束以仓库根部 `AGENTS.md` 为准（不要在这里复制一份产生分叉）。
