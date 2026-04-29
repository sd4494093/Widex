# ralph-widex 使用文档（Widex TUI 版）

`ralph-widex` 是 Widex 内置的自治开发循环功能。用户入口统一只有 TUI slash 命令：`/ralph-widex ...`。

重要约束：

- 第一条命令固定是 `/ralph-widex init`
- 初始化后，编辑 `.ralph/PROMPT.md` 和 `.ralph/@fix_plan.md`
- 然后用 `/ralph-widex start ...` 启动循环
- 用户侧只保留 Widex TUI 内的 `/ralph-widex ...` 流程

## 1) 快速开始

在项目目录启动 Widex 后，按下面顺序操作：

```text
/ralph-widex init
```

然后编辑：

```text
.ralph/PROMPT.md
.ralph/@fix_plan.md
```

再启动循环：

```text
/ralph-widex start --loops 20 --completion-phrase "所有任务已完成"
```

常用控制：

- `/ralph-widex stop`：停止整个 Ralph loop
- `/ralph-widex status`：查看当前 Ralph loop 状态
- `Esc`：只中断当前 turn，然后继续下一轮
- `Ctrl+C`：只中断当前 turn，然后继续下一轮，不会退出 Widex

## 2) TUI 帮助口径

TUI 内帮助页应统一理解为：

```text
/ralph-widex init [--overwrite]
/ralph-widex start [--loops N] [--calls N] [--timeout-minutes N] [--skip-git-repo-check]
                  [--completion-phrase TEXT]... [--completion-mode MODE]
                  [--completion-regex PATTERN]...
/ralph-widex stop
/ralph-widex status
```

说明：

- `start` 是用户侧唯一的启动命令
- `run`、`daemon`、`monitor` 不属于 Widex 用户流程
- 初始化成功后的下一步提示应始终是 `/ralph-widex start`

## 3) 运行期目录与文件（`.ralph/`）

`/ralph-widex init` 会创建：

- `.ralph/PROMPT.md`
- `.ralph/@fix_plan.md`
- `.ralph/@fix_progress.md`
- `.ralph/@AGENT.md`
- `.ralph/specs/.gitkeep`
- `.ralph/logs/`
- `.ralph/examples/`
- `.ralph/docs/generated/`

运行期间会用到：

- `.ralph/PROMPT.md`：每轮都会要求模型读取
- `.ralph/@fix_plan.md`：当前待办/计划
- `.ralph/@fix_progress.md`：进度记录
- `.ralph/.fix_progress.autolog.jsonl`：自动进度日志真源
- `.ralph/STOP`：停止请求文件
- `.ralph/status.json`：TUI best-effort 写入的状态文件，便于排查
- `.ralph/logs/ralph.log`：TUI best-effort 写入的日志文件，便于排查

## 4) 参数说明

`/ralph-widex start` 支持：

- `--loops N`：最多迭代 N 轮，`0` 表示无限
- `--completion-phrase TEXT`：完成词，可重复
- `--completion-mode MODE`：`contains` / `promise-tag` / `regex`
- `--completion-regex PATTERN`：当 `--completion-mode regex` 时生效，可重复
- `--timeout-minutes N`：每轮 watchdog 超时
- `--calls N`：每小时最多 N 个 Ralph turn
- `--skip-git-repo-check`：允许在非 git repo 目录运行

推荐：

- 优先用 `--completion-mode promise-tag`
- 最终消息里输出唯一完成标记，例如：

```text
<promise>任务完成</promise>
```

## 5) 停机语义

- 停止整个循环：`/ralph-widex stop`
- 中断当前 turn 但继续下一轮：`Esc` 或 `Ctrl+C`
- 退出 Widex 前必须先 `/ralph-widex stop`
- `.ralph/STOP` 也可作为内部停止信号文件，但用户侧首选 `/ralph-widex stop`

## 6) 常见排查

- 看不到 Ralph 文件：先确认已经执行过 `/ralph-widex init`
- `Missing .ralph/PROMPT.md`：说明还没 init，先 `/ralph-widex init`
- `bash: /ralph-widex: No such file or directory`：你把 `/ralph-widex ...` 当成了 shell 命令；它是 Widex TUI slash 命令，必须在 Widex 输入框里直接输入
- Ralph 正在运行却想退出：先 `/ralph-widex stop`
- 需要看状态：用 `/ralph-widex status`

## 7) Overlay 维护原则

Ralph 的 Widex 自定义层统一落在：

- `widex-custom/features/ralph-widex/overlay/`
- `widex-custom/features/ralph-widex/templates/`

后续追 upstream 时：

- 用户侧文案、TUI help、loop prompt 优先只改 overlay
- `.ralph/` 初始化模板优先只改 templates
- `codex-rs/ralph-widex` 只做共享 adapter / 执行引擎接线
- 尽量不要再回到 `codex-rs/tui/src/chatwidget.rs` 直接改大段 Ralph 文案

## 8) 当前产品结论

`ralph-widex` 的用户侧定义已经固定：

- 只有 TUI 入口
- 只有 `/ralph-widex init/start/stop/status`
- 第一条命令必须是 `/ralph-widex init`
- 初始化成功后的下一步必须是 `/ralph-widex start`
