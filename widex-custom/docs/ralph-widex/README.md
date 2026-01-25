# ralph-widex 使用文档（Widex 原生版）

`ralph-widex` 是 Widex 内置的“自治开发循环”功能：它会在你的项目目录中创建 `.ralph/`，然后反复调用 `widex exec`（JSON 模式）来执行 Ralph 式的迭代开发，并把状态/进度/日志写入 `.ralph/` 供监控。

重要约束：Widex 中 **只使用 Rust 原生实现**（`widex ralph-widex ...`）。`widex-custom/features/ralph-widex/bin` 与 `lib` 的 shell 脚本只保留作历史参考。


## 1) 快速开始

在任意项目目录（建议是 git repo 根）中：

```bash
# 初始化 .ralph/
widex ralph-widex init

# 编辑提示词和计划
$EDITOR .ralph/PROMPT.md
$EDITOR .ralph/@fix_plan.md

# 启动循环（推荐：后台启动，不阻塞终端）
widex ralph-widex start

# 另开一个终端监控状态
widex ralph-widex monitor
```

推荐：在 `.ralph/PROMPT.md` 里加入“输出纪律”，避免 `cat` 大文件（尤其是 `.ralph/logs/codex_events_*.jsonl`）导致上下文暴涨和超时（`ralph-widex init` 的模板已内置该段落）。

一次性查看状态（不进入刷新循环）：

```bash
widex ralph-widex status
```

TUI 内置命令（等价）：

- `/ralph-widex init`
- `/ralph-widex`（默认 start，后台启动）
- `/ralph-widex monitor`


## 2) 运行期目录与文件（`.ralph/`）

`ralph-widex` 会读写以下文件（这些文件是稳定“外部接口”，用于 debug/可观测）：

- `.ralph/PROMPT.md`：Ralph 主提示词（你会改它）
- `.ralph/@fix_plan.md`：当前待办/计划（你会改它）
- `.ralph/@AGENT.md`：辅助说明（可选维护）

状态与控制：

- `.ralph/status.json`：loop 状态（loop 计数、calls/hour、last_action、exit_reason、next_reset 等）
- `.ralph/progress.json`：`widex exec` 执行中按秒刷新（不执行时通常不存在）
- `.ralph/ralph_widex.pid`：当前 ralph-widex 进程 PID（运行时由 Rust 写入；正常退出会自动删除；崩溃/被强杀时可能残留；`widex ralph-widex status` 会标记 `(stale)`，可用 `widex ralph-widex stop` 清理）
- `.ralph/STOP`：存在即请求停止；Rust 版会在 **exec 运行中** / **rate-limit 等待中** 也检测并尽快退出

退出检测/分析：

- `.ralph/ralph_output_schema.json`：结构化输出 schema（默认会传给 `widex exec --output-schema`）
- `.ralph/.response_analysis`：每轮分析结果（退出信号/完成信号/错误计数等）
- `.ralph/.exit_signals`：累计信号（例如 test-only/done/confidence）

熔断器：

- `.ralph/.circuit_breaker_state`：熔断器状态（含 same-error 统计与原因）
- `.ralph/.circuit_breaker_history`：熔断器历史

日志：

- `.ralph/logs/ralph.log`：高层日志
- `.ralph/logs/codex_events_<ts>.jsonl`：`widex exec --json` 的事件流（逐行 JSON；文件名沿用历史命名）
- `.ralph/logs/codex_stderr_<ts>.log`：子进程 stderr
- `.ralph/logs/codex_last_message_<ts>.txt`：本轮 `--output-last-message` 的落盘内容


## 3) 停机与 graceful shutdown

支持三种“尽快退出”方式：

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
- STOP 在 **exec 运行中**、**rate-limit 等待中** 都会被检测；检测到后会尽快中止当前 `widex exec` 并退出 loop。

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

## 5.1) BLOCKED（自动停机，不触发熔断）

当 `widex exec` 输出结构化的 `RALPH_STATUS` 且 `STATUS: BLOCKED` 时，`ralph-widex` 会认为“需要外部输入/人工决策”，并**立即退出**（`status.json.last_action = "blocked"`），避免无意义地消耗 calls/hour。

典型场景：

- 需要你提供离线快照/截图/密钥/权限
- 需要你确认是否保留某些“意外出现”的文件（例如外部工具生成的新测试）

处理方式：

1) 按 recommendation 补齐输入/做出决策
2) 再次运行：

```bash
rm -f .ralph/STOP .ralph/.circuit_breaker_state
widex ralph-widex run --disable-mcp --skip-git-repo-check
```


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
  --prompt .ralph/PROMPT.md \
  --timeout-minutes 15 \
  --calls 100 \
  --retry-no-final-message 1 \
  --session-expiry-hours 24 \
  --disable-mcp \
  --exec-bypass-approvals-and-sandbox \
  --skip-git-repo-check \
  --verbose
```

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
