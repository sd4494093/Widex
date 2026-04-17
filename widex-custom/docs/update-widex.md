# Widex 快速跟随上游（openai/codex）更新指南

## 当前产品重点（2026-04）

后续 Widex 维护策略统一按下面执行：

- 第一优先级：持续跟随 upstream 的稳定 release tag（例如 `rust-v0.122.0`），不要默认追 `upstream/main`。
- 第二优先级：保持 `ralph-widex` 主线能力稳定，包括：
  - TUI `/ralph-widex ...`
  - CLI `widex ralph-widex ...`
  - `.ralph/` 状态/监控/恢复链路
- 多 LLM 集成（例如 Gemini / Grok / 其它非上游默认 provider）目前**不再作为 Widex 主产品线持续演进目标**。
- 当前 `widex` 主线已经按这个原则继续收口：
  - 仓库内文档继续保留
  - 但运行时主链路不再默认接入 Gemini / Grok / api switchover 这类多 LLM 集成
- 相关文档、设计记录、历史实现说明继续保留在仓库中，作为后续可能重新接回这些能力时的参考资料；但在新的 upstream 跟随过程中，不再默认以“必须继续扩展/保活这些多 LLM 集成”为目标。

本仓库有两条主线分支：

- `main`：本地集成分支，用来在本机对齐 upstream 稳定 release tag，方便后续合并到 `widex`
- `widex`：你们日常开发 / 运行 / 发布分支（包含 Widex 的定制）

## 先说结论

**默认只推 `origin/widex`，不要自动推 `origin/main`。**

也就是说：

- 可以在**本地**把 `main` 更新到 `upstream/main`
- 可以在**本地**用 `main` 作为“上游镜像 / 中转分支”
- 但默认不要执行 `git push origin main`
- 对外发布 / 协作，统一推送 `origin/widex`

这样做的好处：

- 不会意外改动 fork 上的 `origin/main`
- 不需要把 fork 的 `main` 也维护成“严格等于 upstream/main”
- 团队协作时更清楚：**真正要看的分支是 `origin/widex`**

如果将来你们明确想把 fork 的 `main` 也维护成上游镜像，再单独执行 `push origin main` 即可；但这不应作为默认流程。

目标：上游更新频繁时，用最少命令把**本地** `main` 同步到 upstream 稳定 release tag，再把 `main` 合入 `widex`，最后只推 `origin/widex`。

## 0) 前置检查

在仓库根目录执行：

```bash
git remote -v
```

确保至少有：

- `origin` 指向你们 fork（例如 `git@github-sd4494093:sd4494093/Widex.git`）
- `upstream` 指向上游（例如 `git@github.com:openai/codex.git`）

注：如果你们用“多 GitHub 账号 / 多 SSH key”方案，`github-sd4494093` 这类 host alias 取决于
`~/.ssh/config`；如果没做 alias，也可以用标准的 `git@github.com:sd4494093/Widex.git`。

如果缺少 `upstream`，先补上：

```bash
git remote add upstream https://github.com/openai/codex.git
```

并且工作区要干净：

```bash
git status --porcelain=v1
```

若有本地未提交变更，请先提交或 stash。

## 1) 先把本地 main 对齐到 upstream 稳定 release tag

```bash
git fetch upstream
git fetch origin

git checkout main
# 先确认最新稳定 release tag
git tag -l 'rust-v0.*' | sort -V | tail

# 例：追到 rust-v0.122.0
git merge --ff-only rust-v0.122.0
```

如果本地没有 `main`，可从 `origin/main` 建一个本地分支：

```bash
git checkout -b main origin/main
```

### 如果仓库是浅克隆（很常见）

有时 `git merge-base` / `git merge` 结果异常，或者历史看起来“断掉了”，多半是浅克隆导致。
先执行：

```bash
git rev-parse --is-shallow-repository
```

如果输出是 `true`，先补全历史：

```bash
git fetch --unshallow origin
```

再继续后面的合并。

### 不要默认 push origin main

**默认到这里就停，不要执行：**

```bash
# 默认不要这样做
git push origin main
```

`origin/main` 在你们当前流程里不是主要协作分支；真正要推的是 `origin/widex`。

## 2) 把本地 main 合入 widex

```bash
git checkout widex
git merge --ff-only origin/widex   # 先把本地 widex 跟到远端最新
git merge main
# 如有冲突：解决冲突 -> git add -> git commit
```

合并完成后，推送：

```bash
git push origin widex
```

## 3) 建议验证

建议合并完成后至少跑一次关键测试（按你们实际改动的 crate 取舍）：

```bash
cd codex-rs
just fmt
cargo test -p codex-tui
```

如果这次上游更新涉及 `core/protocol/common` 等基础 crate，建议再补：

```bash
cargo test -p codex-core --lib
```

（完整 `cargo test --all-features` 通常更慢，按需要在 CI 或专门窗口执行。）

## 4) 本地重建 Widex（底层 codex 二进制）（可选，但推荐）

你们的 `widex` wrapper 会在需要时重建它依赖的 `codex` 二进制；默认用 `widex-release`
profile（更快），通常只需要：

```bash
# 触发自动构建（若缺 binary 或你想强制 rebuild）
WIDEX_FORCE_REBUILD=1 widex --version
```

或手动构建：

```bash
cd codex-rs
cargo build -p codex-cli --bin codex --profile widex-release
```

### 4.1 重要经验：不要用旧的 `widex-release` 二进制做验收

`widex-custom/bin/widex` 默认直接执行：

```bash
codex-rs/target/widex-release/codex
```

如果这个二进制已经存在，wrapper **不会因为源码变了就自动重编**。所以每次改了
`codex-rs/tui/`、`codex-rs/cli/`、启动页 / onboarding / 登录链路之后，正式验收前要先执行：

```bash
cd codex-rs
cargo build -p codex-cli --bin codex --profile widex-release
```

否则非常容易出现：

- `cargo test -p codex-tui` 已经通过
- 源码看起来也对
- 但 `widex` 实际跑出来还是旧行为

后续把这一步当成 Widex 启动链路验收前的固定动作。

### 4.2 重要经验：真实 `~/.widex-codex/config.toml` 也要自动迁到 Widex 标准主配置

不要只满足“新用户首次生成 config 正确”。

真实生产环境里，很多用户的 `~/.widex-codex/config.toml` 可能已经被旧的 API switcher、
ppchat 配置、Gemini/Grok 试验配置等污染过。如果 wrapper 只是“补缺省”，那么用户机器上仍可能继续跑旧的：

- `model_provider = "gpt-ppchat"`
- 非 `file` 的 CLI auth store
- 非 WillAU 的主 provider

当前 Widex 标准已经改成：

```toml
model_provider = "custom"
model = "gpt-5.4"
model_reasoning_effort = "high"
disable_response_storage = true
personality = "pragmatic"
cli_auth_credentials_store = "file"

[model_providers.custom]
name = "custom"
wire_api = "responses"
requires_openai_auth = true
base_url = "https://api.wellau.com/v1"
```

注意这里必须是：

```toml
base_url = "https://api.wellau.com/v1"
```

不是：

```toml
base_url = "https://wellau.com/v1"
```

实测结果是：

- `https://api.wellau.com/v1/models` 返回正常 JSON 模型列表
- `https://wellau.com/v1/models` 返回站点 HTML
- `https://wellau.com/v1/responses` 返回 `405 Not Allowed`

所以后续不要再把 Widex 默认 API 根地址改成 `https://wellau.com/v1`。

所以后续维护原则是：

- wrapper 不只负责初始化新 config
- 也必须把已有 `~/.widex-codex/config.toml` 迁移回上述 Widex 标准主配置
- 迁移时保留 `.bak` 备份
- 迁移逻辑必须是**幂等**的，不能每次执行 `widex` 都重复改写 config

## 4.3 生产 smoke 的判定顺序

后续每次追 `upstream` 或修改启动/鉴权链路时，按下面顺序验收：

```bash
widex --version
widex --help
widex ralph-widex --help
```

然后检查真实 `~/.widex-codex/`：

- `config.toml` 是否已经回到 Widex/WillAU 标准主配置
- `auth.json` 是否存在 `OPENAI_API_KEY`

然后再做真实在线 smoke，例如：

```bash
widex exec --color never --sandbox read-only -c mcp.enabled=false -c mcp.auto_attach_all=false "Respond with exactly PONG and nothing else."
```

### 4.3.1 如何区分问题归因

- 如果返回 `401 INVALID_API_KEY`
  说明是 `auth.json` 里的当前 key 无效，先修 key，不要误判成 Widex 代码问题。
- 如果返回 `429 Too Many Requests`
  说明鉴权已通，但当前 key/租户额度或限流有问题。这是服务侧/账号侧问题，不是 Widex 启动链路、wrapper、TUI onboarding 的代码问题。
- 只有在 key 有效、额度正常的前提下，`widex exec` 仍失败，才继续查 Widex 代码或 upstream 兼容性。

## 5) 2026-03-06 这次更新的实际记录

本次升级是一次真实的“Widex 跟随上游”案例，遇到并确认了下面这些点：

### 5.1 实际分支状态

当时仓库状态大致是：

- `origin/main` **落后** `upstream/main`
- 主要协作分支是 `origin/widex`
- 因此本次只更新了**本地** `main`，然后把它合入 `widex`
- **没有推送 `origin/main`**
- 最终目标是推送 `origin/widex`

这也再次说明：

> 你们当前最合理的默认流程，是把 `main` 当作本地“上游 release 对齐分支”，而不是团队发布分支。

### 5.2 本次遇到的典型问题

1. **仓库是浅克隆**
   - 本地最初是 shallow clone，导致 merge-base / 历史关系判断不可靠
   - 需要先 `git fetch --unshallow origin`

2. **本地有未提交改动**
   - 升级前先 stash 更安全
   - 本次就是先 stash，再做 fetch / merge，最后再 pop 回来

3. **上游改动跨度很大，冲突集中在 Rust 核心层**
   - 主要冲突点在：
     - `codex-rs/core/`
     - `codex-rs/codex-api/`
     - `codex-rs/tui/`
   - 尤其是模型元数据、provider / wire API、TUI 事件流、session 设置更新等

4. **Widex 定制不能机械地“全选 ours / theirs”**
   - 要优先保留 Widex 的：
     - Ralph Widex TUI 流程
     - `widex` wrapper / 独立 `CODEX_HOME` 约束
     - `ralph-widex` 运行与监控链路
   - 同时也要接住上游的新字段 / 新事件 / 新结构

5. **本机验证可能被系统依赖卡住**
   - 本次 `cargo check -p codex-core` 通过
   - `just fmt` 也已执行
   - 但 `codex-tui` / 部分测试会被 Linux 机器缺少 `libcap.pc` 卡住
   - 这不是 merge 冲突本身，而是宿主机缺系统开发包

## 6) 后续其他 agent 更新时必须注意

### 6.1 分支策略

- **默认只推 `origin/widex`**
- 不要在没有明确要求的情况下推 `origin/main`
- 若只是“跟上游并合入 Widex”，只需要：
  - 更新本地 `main`
  - 合并到 `widex`
  - 推 `origin/widex`

### 6.2 升级前先做这 4 步

```bash
git status --porcelain=v1
git remote -v
git rev-parse --is-shallow-repository
git fetch upstream && git fetch origin
```

如果有本地改动：先 stash。
如果是 shallow clone：先 `git fetch --unshallow origin`。
如果没有 `upstream`：先 `git remote add upstream ...`。

### 6.3 冲突处理原则

- Widex 专属逻辑优先看这些目录：
  - `widex-custom/`
  - `codex-rs/core/`
  - `codex-rs/tui/`
  - `codex-rs/codex-api/`
- 处理冲突时不要只图“先编过”，要检查：
  - 是否尽量保持对 upstream 的低漂移
  - Ralph Widex 是否仍能在当前会话里前台工作
  - `widex` wrapper / 独立 `CODEX_HOME` / 启动链路是否仍正常
  - model picker / slash command / onboarding 是否还保留 Widex 主线行为

### 6.3.1 多 LLM 集成的当前处理原则

- `widex-custom/docs/LLMs_intergration/` 以及相关设计文档继续保留，不删除。
- 这些文档现在属于“历史能力 / 可回收设计资料”，不是当前版本的主维护目标。
- 如果上游更新与 Gemini / Grok / 其它多 LLM 定制发生冲突，默认优先：
  - upstream 对齐
  - `ralph-widex` 稳定性
  - Widex 主线启动、会话、TUI 交互稳定性
- 除非有明确新需求，不再为了保住多 LLM 集成而扩大对上游核心代码的长期分叉面。

### 6.3.1 版本号同步要求

- `widex` 启动页和 `widex --version` 使用的是 Rust workspace 版本，也就是 `codex-rs/Cargo.toml` 里的 `[workspace.package].version`
- 上游仓库源码里这个值经常保持 `0.0.0`，因为官方 release 流水线会在发布时再注入正式版本
- 但 Widex 是长期运行源码构建版，所以**每次完成一次 upstream 对齐后，都应把这个版本手工更新为本次对齐的 Codex 版本**
- 这样你们一眼就能看出：当前 Widex 对齐到哪个 upstream 版本、是否落后，以及 TUI 里的 update 提示是否真的有意义
- 本次 2026-03-06 升级后，Widex 版本已对齐为 `0.111.0`

### 6.4 验证优先级

推荐按下面顺序验证：

```bash
cd codex-rs
just fmt
cargo test -p codex-model-provider-info
cargo test -p codex-tui
env -i PATH="$PATH" HOME="$HOME" USER="$USER" SHELL="$SHELL" TERM="${TERM:-xterm-256color}" LANG="C.UTF-8" LC_ALL="C.UTF-8" CARGO_HOME="$HOME/.cargo" RUSTUP_HOME="$HOME/.rustup" TMPDIR="${TMPDIR:-/tmp}" cargo test -p codex-core
```

说明：

- `codex-core` 的 package 级测试建议放在**隔离环境**里执行，否则当前 agent 进程注入的开发者说明、skills、environment context 可能污染请求/事件断言，制造大量假失败。
- 如果某一步被系统依赖卡住，**要在提交说明里明确写出来**，不要默默跳过。

### 6.4.1 Widex / Ralph 最小封板烟测

上面 Rust 测试通过后，再做下面这组最小可运行验证：

```bash
widex --version
widex --help
widex ralph-widex --help
```

如果要补做一个**不依赖真实模型调用**的 Ralph 本地烟测，可在临时目录执行：

```bash
tmpdir="$(mktemp -d)"
cd "$tmpdir"
widex ralph-widex init
widex ralph-widex status
```

通过标准：

- `widex --version` 正常返回版本号
- `widex --help` 正常输出，且仍可见 `ralph-widex`
- `widex ralph-widex --help` 正常输出 `init/run/start/stop/status/monitor`
- `widex ralph-widex init` 能正确生成 `.ralph/` 模板目录
- `widex ralph-widex status` 至少能正常读取/提示 `.ralph/` 状态，而不是命令直接报错退出

说明：

- 这组烟测的目标是确认 **Widex 启动链路** 和 **Ralph 自定义入口** 没被上游合并打断。
- 它**不等价于**跑一轮真实 `ralph-widex run`；后者依赖实际模型、认证、网络和任务上下文，属于更高成本的业务验收。

### 6.4.2 生产环境金标准收口定义

一次 upstream 跟随完成后，只有同时满足下面几点，才算真正收口：

- 工作区干净：`git status --short --branch`
- 对目标 release tag 不落后：`git describe --tags --exact-match` 或明确记录当前 merge 基线
- 已推到团队主分支：`git rev-list --left-right --count origin/widex...widex`
- `widex` 可直接启动：`widex --version` / `widex --help`
- `ralph-widex` 入口仍然可用：`widex ralph-widex --help`
- `widex-custom/bin/widex` 能在空 `WIDEX_CODEX_HOME` 下生成 WellAU 标准配置，且不自动写入 `auth.json`
- `codex-core` package 级测试在隔离环境下通过，或剩余失败已确认是上游基线问题并单独建档

建议固定用下面这组命令做最终封板：

```bash
git status --short --branch
git describe --tags --always
git rev-list --left-right --count origin/widex...widex
widex --version
widex --help | sed -n '1,40p'
widex ralph-widex --help | sed -n '1,80p'
tmp_home="$(mktemp -d)"
WIDEX_CODEX_HOME="$tmp_home" widex --help >/dev/null 2>&1 || true
sed -n '1,80p' "$tmp_home/config.toml"
test ! -f "$tmp_home/auth.json"
```

### 6.4.3 以后如何把跟随 upstream 的周期压短

后续要有意识把流程压缩成“只保 Ralph，其它尽量回到 upstream”：

- 新定制优先放在 `widex-custom/`，不要轻易继续扩张 `codex-rs/core/` 和 `codex-rs/tui/` 的长期分叉面。
- Widex 默认 provider / 默认 feature 的产品化收口，优先放在 `widex-custom/bin/widex` 这类启动层做；不要再把这类“默认配置注入”绑死在 TUI onboarding 提交流程里。
- 不要把 Gemini / Grok / api switchover 这类历史多 LLM 能力重新接回默认运行时主链路，除非有明确的新需求单独立项。
- 每次 upstream 合并，只优先保三件事：
  - `widex` 启动链路正常
  - `ralph-widex` TUI / CLI 入口正常
  - `.ralph/` 状态、监控、恢复链路不回退
- 发版验证尽量固定在“隔离环境命令 + Widex wrapper smoke + Ralph smoke”三段式，不要直接继承当前 agent 会话环境做最终判定。
- 合并完成后尽快做“小提交 + 小推送”，不要把额外实验性改动混进同一轮 upstream 跟随里。
- 如果某个 Widex 定制不是为了 Ralph，也不是为了启动链路稳定性，就要优先考虑删除、回退到 upstream，或者移到文档/模板层保存，而不是继续挂在主产品线。

### 6.5 不要遗漏本地已有改动

如果升级前工作区不是干净的：

- 先 stash
- 升级完成后再 pop
- 检查这些本地改动是否需要：
  - 一并纳入本次提交
  - 还是继续保留为本地未提交修改

不要在升级过程中把原有本地改动意外覆盖掉。

## 7) 常见问题

- 合并冲突多：
  - 先保证**本地** `main` 是干净的 `upstream/main`，再合入 `widex`；冲突只会集中在 `widex` 侧。
- 合入后编译失败：
  - 多半是上游新增字段 / 协议变更导致（例如 struct 新字段、enum 新变体），按报错逐个补齐即可。
- 老用户升级后 `widex resume` / `widex` 启动时提示
  `unknown variant 'gemini', expected 'responses'`：
  - 这是旧 `~/.widex-codex/config.toml` 里遗留的历史 Widex provider 配置（例如 `wire_api = "gemini"` / `wire_api = "chat"`）导致。
  - 当前 wrapper 会在启动前自动把历史 `gemini*` / `grok-*` provider 的 `wire_api` 迁移成 `responses`，并在同目录生成
    `config.toml.pre-mainline-wire-api.bak` 备份。
  - 如果仍想彻底清理，直接手工删除这些历史 provider 块即可；当前主线不再默认维护它们。
- `origin/main` 要不要同步到最新上游：
  - **默认不要。** 除非你们明确决定把 fork 的 `main` 也维护成 upstream 镜像。
- 只想更新 `widex`：
  - 可以，做法就是：更新本地 `main` -> merge 到 `widex` -> push `origin/widex`。
