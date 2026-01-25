# ralph-widex 使用文档（Widex 原生版）

`ralph-widex` 是 Widex 内置的“自治开发循环”功能，用于实现 Ralph 式迭代开发（同一提示词/同一计划反复迭代，依靠文件与 git 历史实现自反馈）。

重要约束：Widex 中 **只使用 Rust 原生实现**（`widex ralph-widex ...`）。`widex-custom/features/ralph-widex/bin` 与 `lib` 的 shell 脚本只保留作历史参考。

## 两种运行方式（推荐 TUI 前台模式）

1) **TUI 前台模式（推荐）**：在 Widex TUI 内使用 `/ralph-widex ...` 启动循环。每一轮都是一个正常的 Codex/Widex turn，因此你能在终端里看到完整的工具调用/执行过程；当一轮结束（或被中断）后，会自动触发下一轮，直到达到 `--loops` 或命中完成词（`--completion-phrase`）。

   - `Esc`：中断当前 turn，然后继续下一轮（不停止 Ralph）
   - `Ctrl+C`：中断当前 turn，然后继续下一轮（不停止 Ralph；不会触发退出快捷键）
   - `/ralph-widex stop`：停止整个 Ralph loop（不再进入下一轮；停止后才允许退出 Widex）
   - 支持的参数（TUI 模式）：
     - `--loops N`：最多迭代 N 轮（`0` 表示无限）
     - `--completion-phrase TEXT`：完成词（可重复传多个；命中任意一个即可提前停机）
     - `--completion-mode MODE`：完成检测模式（`contains` / `promise-tag` / `regex`）
       - 推荐：`promise-tag`（更少误触发），要求最终消息包含 `<promise>...</promise>`
     - `--completion-regex PATTERN`：当 `--completion-mode regex` 时生效（可重复）
     - `--timeout-minutes N`：每个 turn 的 watchdog（超时会自动 `Interrupt` 该 turn 并继续下一轮）
     - `--calls N`：每小时最多跑 N 个 Ralph turn；到达后会等待到整点自动继续
     - `--skip-git-repo-check`：允许在非 git repo 目录运行（默认要求在 git repo 内）

2) **CLI 监督者模式（可选）**：使用 `widex ralph-widex run/start` 运行一个外部 supervisor 进程，按轮调用 `widex exec` 并将状态/日志写入 `.ralph/`（更适合 headless/无人值守场景）。


## 1) 快速开始

在任意项目目录（建议是 git repo 根）中：

```bash
# 初始化 .ralph/
widex ralph-widex init

# 编辑提示词和计划（以及可选的完成词）
$EDITOR .ralph/PROMPT.md
$EDITOR .ralph/@fix_plan.md

# 进入 Widex TUI（必须在项目目录里启动）
widex --ask-for-approval never --sandbox danger-full-access -m gpt-5.2

# 在 TUI 内启动循环（前台可见每轮交互）
/ralph-widex start --loops 20 --completion-phrase "所有任务已完成"
```

更可靠的“完成词”写法（避免正文里偶然出现“任务完成”导致误触发）：

```text
/ralph-widex start --loops 20 --completion-mode promise-tag --completion-phrase "任务完成"
```

然后要求模型在**最终消息**里输出（精确文本）：

```text
<promise>任务完成</promise>
```

推荐：在 `.ralph/PROMPT.md` 里加入“输出纪律”，避免 `cat` 大文件（尤其是 `.ralph/logs/codex_events_*.jsonl`）导致上下文暴涨和超时（`ralph-widex init` 的模板已内置该段落）。

停止循环：

- TUI 内：`/ralph-widex stop`
- 或在项目目录：`touch .ralph/STOP`（会在当前 turn 结束/中断时生效）
- 仅中断当前 turn（继续下一轮）：按 `Esc`


## 2) 运行期目录与文件（`.ralph/`）

`ralph-widex` 会读写以下文件（这些文件是稳定“外部接口”，用于 debug/可观测）：

- 说明：
  - **TUI 前台模式**：主要依赖 `.ralph/PROMPT.md` / `.ralph/@fix_plan.md` / `.ralph/@fix_progress.md`，并支持用 `.ralph/STOP` 请求停止（在当前 turn 结束/中断时生效）。
  - **CLI 监督者模式**：会额外维护 `.ralph/status.json` / `.ralph/progress.json` / `.ralph/logs/*` 等可观测文件（用于 `widex ralph-widex monitor`）。

- `.ralph/PROMPT.md`：Ralph 主提示词（你会改它）
- `.ralph/@fix_plan.md`：当前待办/计划（你会改它）
- `.ralph/@fix_progress.md`：进度记录（推荐 agent 在 Notes 里追加；supervisor 会在每轮 start/end 把 auto log 强制落盘）
- `.ralph/.fix_progress.autolog.jsonl`：auto log 的“真源”（append-only）；即使模型误改/覆盖了 `@fix_progress.md`，supervisor 也能重建 auto log 段落
- `.ralph/@AGENT.md`：辅助说明（可选维护）

状态与控制：

- `.ralph/STOP`：存在即请求停止
- （CLI 监督者模式）`.ralph/status.json`：loop 状态（loop 计数、calls/hour、last_action、exit_reason、next_reset 等）
- （CLI 监督者模式）`.ralph/progress.json`：`widex exec` 执行中按秒刷新（不执行时通常不存在）
- （CLI 监督者模式）`.ralph/ralph_widex.pid`：当前 ralph-widex 进程 PID（运行时由 Rust 写入；正常退出会自动删除；崩溃/被强杀时可能残留；`widex ralph-widex status` 会标记 `(stale)`，可用 `widex ralph-widex stop` 清理）

退出检测/分析：

- （CLI 监督者模式）`.ralph/ralph_output_schema.json`：结构化输出 schema（默认会传给 `widex exec --output-schema`）
- （CLI 监督者模式）`.ralph/.response_analysis`：每轮分析结果（退出信号/完成信号/错误计数等）
- （CLI 监督者模式）`.ralph/.exit_signals`：累计信号（例如 test-only/done/confidence）

熔断器：

- （CLI 监督者模式）`.ralph/.circuit_breaker_state`：熔断器状态（含 same-error 统计与原因）
- （CLI 监督者模式）`.ralph/.circuit_breaker_history`：熔断器历史

日志：

- （CLI 监督者模式）`.ralph/logs/ralph.log`：高层日志
- （CLI 监督者模式）`.ralph/logs/codex_events_<ts>.jsonl`：`widex exec --json` 的事件流（逐行 JSON；文件名沿用历史命名）
- （CLI 监督者模式）`.ralph/logs/codex_stderr_<ts>.log`：子进程 stderr
- （CLI 监督者模式）`.ralph/logs/codex_last_message_<ts>.txt`：本轮 `--output-last-message` 的落盘内容


## 3) 停机与 graceful shutdown

TUI 前台模式（推荐）：

- **停止整个 Ralph loop**：`/ralph-widex stop`
- **只中断当前 turn（继续下一轮）**：`Esc`（或 `Ctrl+C`）
- **禁止直接退出 Widex**：Ralph loop 运行中会阻止退出；如需退出，请先 `/ralph-widex stop`

CLI 监督者模式：支持三种“尽快退出”方式：

1) Ctrl-C（SIGINT）
2) SIGTERM（例如进程管理器发的）
3) 创建 STOP 文件：

```bash
touch .ralph/STOP
```

或直接用子命令（推荐，等价于创建 STOP，并 best-effort SIGTERM）：

```bash
widex ralph-widex stop
```

如需只创建 STOP（不发 SIGTERM）：

```bash
widex ralph-widex stop --no-sigterm
```

说明：

- STOP 文件路径永远是“项目目录下的 `.ralph/STOP`”（例如 `/home/will/data/wellau/.ralph/STOP`）。
- CLI 监督者模式：STOP 在 **exec 运行中**、**rate-limit 等待中** 都会被检测；检测到后会尽快中止当前 `widex exec` 并退出 loop。
- TUI 前台模式：STOP 会在当前 turn 结束/中断时生效（推荐直接用 `/ralph-widex stop` 立即停机）。

退出时会：

- 尝试向正在运行的 `widex exec` 子进程发送信号并在短超时后强制终止
- 清理 `.ralph/progress.json`
- 更新 `.ralph/status.json` 标记退出原因


## 4) structured output（默认开启）

默认每次 `widex exec` 会带：

- `--json`（事件流 JSONL）
- `--output-last-message <path>`（写入 `.ralph/logs/codex_last_message_*.txt`）
- `--output-schema .ralph/ralph_output_schema.json`（强制/引导模型输出可解析结构）

说明：schema 会限制 `recommendation` 的最大长度（当前为 240 字符），避免模型在“总结/复述日志”时输出过长导致超时。

如需关闭 schema（仅用于排查模型/代理不兼容）：

```bash
widex ralph-widex run --no-output-schema
```

### 4.1) “no last agent message” 自动重试（默认开启）

某些 provider/代理/边缘情况会出现：

- `widex exec` 退出码为 0
- 但 `--output-last-message` 写出了空文件（CLI 会在 stderr 打印 `Warning: no last agent message ...`）

这会导致“无进展”误判并触发熔断器。`ralph-widex` 默认会在这种情况下**自动重试一次**（同一 loop 内，计入 calls/hour）。

另外：当 `--output-last-message` 落盘为空，但 stdout 的 JSONL 事件流里能看到 `AgentMessage` 时，`ralph-widex` 会自动用事件流里的 `AgentMessage` 作为本轮 `last_message` 的兜底（减少误判）。

可调整重试次数（重试次数 N 表示“最多额外重试 N 次”，总尝试次数 = N + 1）：

```bash
widex ralph-widex run --retry-no-final-message 2
```


## 5) 限流（calls/hour）

默认每小时最多 100 次调用：

```bash
widex ralph-widex run --calls 60
```

触达限流会进入等待，直到下一个整点自动 reset；等待期间仍会响应 Ctrl-C/SIGTERM/STOP。

## 5.1) BLOCKED（信号，不自动停机）

当 `widex exec` 输出结构化的 `RALPH_STATUS` 且 `STATUS: BLOCKED` 时，代表“需要外部输入/人工决策”的信号。

Widex 的 ralph 核心目标是“按计划迭代到指定轮数，或命中完成词提前退出”，所以：

- **默认不会因为 BLOCKED 自动退出**（会继续下一轮）
- 若你希望遇到 BLOCKED 立即停机：可以用完成词策略（让 agent 在确认被阻塞时输出你指定的 completion phrase），或手动 `widex ralph-widex stop`

典型场景：

- 需要你提供离线快照/截图/密钥/权限
- 需要你确认是否保留某些“意外出现”的文件（例如外部工具生成的新测试）

处理方式：按 recommendation 补齐输入/做出决策后继续运行即可。


## 6) 会话连续性（continue/resume）

默认启用会话连续性：若本轮 `widex exec` 输出了 `thread_id`，会写入 `.ralph/.widex_session.json`，后续用 `widex exec resume <thread_id>` 继续。

- 关闭：

```bash
widex ralph-widex run --no-continue
```

- 会话过期（默认 24h）：

```bash
widex ralph-widex run --session-expiry-hours 6
```

### 6.1) 自动清理会话（避免长线程/卡死）

在以下情况，`ralph-widex` 会自动删除 `.ralph/.widex_session.json`（只影响“下一轮”是否 resume），以避免“长线程 compaction 导致的退化/卡死”：

- `widex exec` 单次超时（exit code 124）
- `widex exec` exit code 0 但没有最终 assistant 消息（`no last agent message`）
- stdout 事件里出现 “Long threads and multiple compactions ... Start a new thread ...” 的提示

日志里会出现类似：

```
[WARN] Clearing session for next loop: timeout
```


## 7) 常用参数

```bash
widex ralph-widex run \
  --loops 20 \
  --completion-phrase "所有任务已完成" \
  --prompt .ralph/PROMPT.md \
  --timeout-minutes 15 \
  --calls 100 \
  --retry-no-final-message 1 \
  --session-expiry-hours 24 \
  --exec-bypass-approvals-and-sandbox \
  --skip-git-repo-check \
  --verbose
```

说明：

- `--loops N`：最多迭代 N 轮（未命中 completion phrase 时到点退出）
- `--completion-phrase ...`：出现即提前退出（可重复指定多个）
- `--enable-circuit-breaker`：可选开启熔断器（默认关闭；会在持续同错/无进展时提前停机）

- `--codex-cmd <path>`：覆盖用于执行 `widex exec` 的二进制路径（默认用当前可执行文件；也可用 env `CODEX_CMD`）
- `--exec-bypass-approvals-and-sandbox`：对子进程 `widex exec` 传 `--dangerously-bypass-approvals-and-sandbox`，避免 approval prompt 导致 exec 卡住直到超时（适合“无人值守的自治循环”）。

### 7.1) 透传到子进程 `widex exec`（推荐用来快速切模型/Provider）

`ralph-widex run` 支持把参数/配置透传给内部的 `widex exec`，用于：

- 临时切换模型（不改 `${CODEX_HOME}/config.toml`）
- 临时覆盖 provider/base_url/timeout 等（排查代理/网络问题）

示例：

```bash
widex ralph-widex run \
  --exec-model gpt-5.2 \
  --exec-config 'model="gpt-5.2"' \
  --exec-config 'model_reasoning_effort="high"'
```

注意：`ralph-widex` 默认会给子进程加 `-c model_reasoning_summary="none"`（减少噪音/加速收敛）。如需开启，请显式传：

```bash
widex ralph-widex run --exec-config 'model_reasoning_summary="concise"'
```

也可以用 feature toggles（repeatable）：

```bash
widex ralph-widex run --exec-enable web_search_request
widex ralph-widex run --exec-disable web_search_request
```


## 8) 常见排查

- loop 没反应：看 `.ralph/status.json` 和 `.ralph/logs/ralph.log`
- 想看每次 `widex exec` 的详细事件：打开 `.ralph/logs/codex_events_*.jsonl`
- 熔断器打开：看 `.ralph/.circuit_breaker_state`（里面有 reason）；修复后重跑即可（必要时可删除 `.ralph/.circuit_breaker_state` 重置）
- 强制停止：`touch .ralph/STOP`，并确认进程退出后可删除 STOP 再次启动
- `widex exec` 超时：`ralph-widex` 会把它视为“本轮失败”（exit code 124）并继续下一轮；如果持续超时，可能会触发熔断器。通常处理方式是：
  - 调大 `--timeout-minutes`
  - 或解决导致卡住的根因（例如 MCP / 网络 / provider 侧响应）
- 出现 rmcp serde / JsonRpcMessage 类错误（通常是某个 MCP server 输出了非 JSON 内容，导致 JSON-RPC framing 破坏）：用 `--disable-mcp` 跑一轮确认问题是否消失（会把已配置的 `mcp_servers.*` 逐个 `enabled=false`，而不是依赖“清空表”的覆盖）：

```bash
widex ralph-widex run --disable-mcp
```

- 如果你在 `.ralph/logs/codex_stderr_*.log` 或 monitor 的 Output 里反复看到：
  `Custom tool call output is missing for call id: ...`
  这通常是“继续（resume）旧 thread 时历史里存在缺失的 tool output”导致的内部修复日志。
  ralph-widex 会忽略它，不会因此触发 circuit breaker；但如果你希望彻底消除它，可以重置会话：

```bash
# 方式 1：本次不续跑，直接开新 thread
widex ralph-widex run --no-continue

# 方式 2：删除 session 记录（下次 run 会自动新建）
rm -f .ralph/.widex_session.json
```
