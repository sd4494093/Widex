# Widex upstream merge and npm release playbook

本文档记录 Widex 长期跟随 `openai/codex` upstream release tag、通过 GitHub Actions 生产 npm artifacts、再发布 npm 包的实操流程。它也总结了 2026-05-14 到 2026-05-15 这次同步 `rust-v0.130.0` 时踩到的问题和修复。

## 目标和边界

- Widex 长期跟随 upstream 稳定 release tag，例如 `rust-v0.130.0`，不要默认追 `upstream/main`。
- 工作分支是 Widex 仓库的 `widex` 分支，推送目标是 `origin/widex`。
- 不要把 Widex 分支推回 upstream `openai/codex`。
- 保留 Widex 产品层差异，尽量减少和 upstream 的长期漂移。
- `widex-npm-artifacts` workflow 只构建 native artifacts、stage npm tarballs、上传 artifact；它不发布 npm。
- 当前 `rust-release.yml` 对 `widex-rust-v*` / `widex-v*` tag 设置 `should_publish=false`，所以打 Widex tag 也不会自动 npm publish。

## 每次合并 upstream 的标准流程

1. 确认分支和远端。

   ```bash
   git remote -v
   git status --short --branch
   git switch widex
   ```

2. 获取 upstream tags，并选择最新稳定 Rust release tag。

   ```bash
   git fetch upstream --tags
   git tag -l 'rust-v0.*' | sort -V | tail
   ```

3. 合并 tag 到 `widex`。

   ```bash
   git merge rust-vX.Y.Z
   ```

   冲突处理原则：

   - 优先保留 upstream 的结构和实现。
   - 只在 Widex 产品要求处重新挂接 Widex 行为。
   - 不复活旧的多 LLM runtime 集成，除非明确要求。
   - 重点检查 `widex-custom/features/ralph-widex/overlay/`、Widex/WellAU startup/auth 行为、TUI `/ralph-widex` 路径。

4. 本地验证。

   ```bash
   cd codex-rs
   just fmt
   cargo test -p <changed-crate>
   cargo build -p codex-cli --bin codex --profile widex-release
   ```

   如果改了 TUI 或可见文本，必须跑相关 `insta` snapshot 测试并明确接受 snapshot。

5. 提交并推送。

   ```bash
   git status --short
   git commit
   git push origin widex
   ```

## `widex-npm-artifacts` workflow 做什么

触发方式：

- push 到 `widex`，且改动路径匹配：
  - `.github/workflows/widex-npm-artifacts.yml`
  - `codex-cli/**`
  - `codex-rs/**`
  - `widex-custom/**`
- 手动 `workflow_dispatch`，需要输入 `release_version`。

并发策略：

- `concurrency.group` 是 workflow + branch。
- `cancel-in-progress: true`，新推送会取消旧的同分支 artifact run，避免旧 commit 长时间占队列。

Native matrix：

| package target | runner | native binaries built by workflow |
| --- | --- | --- |
| `x86_64-apple-darwin` | `macos-15-intel` | `codex` |
| `aarch64-apple-darwin` | `macos-15` | `codex` |
| `x86_64-unknown-linux-gnu` | `ubuntu-24.04` | `codex`, `bwrap` |
| `x86_64-unknown-linux-musl` | `ubuntu-24.04` | `codex`, `bwrap` |
| `aarch64-unknown-linux-musl` | `ubuntu-24.04-arm` | `codex`, `bwrap` |
| `x86_64-pc-windows-msvc` | `windows-2025` | `codex.exe`, `codex-windows-sandbox-setup.exe`, `codex-command-runner.exe` |
| `aarch64-pc-windows-msvc` | `windows-11-arm` | `codex.exe`, `codex-windows-sandbox-setup.exe`, `codex-command-runner.exe` |

Native artifacts 都会压成 `.zst` 后上传，每个 target 一个 artifact。

`Stage npm tarballs` job 会：

1. 安装 `zstd`、Node 22、pnpm。
2. `pnpm install --frozen-lockfile`。
3. 运行：

   ```bash
   ./scripts/stage_npm_packages.py \
     --release-version "$RELEASE_VERSION" \
     --workflow-url "$GITHUB_SERVER_URL/$GITHUB_REPOSITORY/actions/runs/$GITHUB_RUN_ID" \
     --package widex
   ```

4. 输出 `dist/npm/*.tgz`。
5. 上传 `widex-npm-tarballs-${release_version}` artifact。

push 触发时没有 `inputs.release_version`，artifact 名字会变成 `widex-npm-tarballs-`。这是正常现象，不代表 staging 失败。

## npm 包和 native payload 对应关系

`scripts/stage_npm_packages.py --package widex` 会展开为 root package 和 6 个平台包：

| package | npm name | native components |
| --- | --- | --- |
| `widex` | `@wellau/widex` | 无 native payload；通过 optional dependencies 指向平台包 |
| `widex-linux-x64` | `@wellau/widex-linux-x64` | `bwrap`, `codex`, `rg` |
| `widex-linux-arm64` | `@wellau/widex-linux-arm64` | `bwrap`, `codex`, `rg` |
| `widex-darwin-x64` | `@wellau/widex-darwin-x64` | `codex`, `rg` |
| `widex-darwin-arm64` | `@wellau/widex-darwin-arm64` | `codex`, `rg` |
| `widex-win32-x64` | `@wellau/widex-win32-x64` | `codex`, `rg`, `codex-windows-sandbox-setup`, `codex-command-runner` |
| `widex-win32-arm64` | `@wellau/widex-win32-arm64` | `codex`, `rg`, `codex-windows-sandbox-setup`, `codex-command-runner` |

注意：

- `rg` 不是 workflow native build artifact；它由 `codex-cli/bin/rg` manifest 通过 `install_native_deps.py` 下载。
- Linux 的 `bwrap` 是 workflow 必须构建和上传的 native artifact。
- `widex-linux-x64` 同时需要 `x86_64-unknown-linux-gnu` 和 `x86_64-unknown-linux-musl` vendor source target。不要只保留 musl。

## 本次事故时间线

1. 合并 upstream。

   - 合并 tag：`rust-v0.130.0`
   - merge commit：`ff481b2b73 Merge tag 'rust-v0.130.0' into widex`
   - 推送到：`origin/widex`

2. 第一次 artifact run 失败。

   - run：`25867861572`
   - 结果：native builds 基本完成，但 `Stage npm tarballs` 失败。
   - 复现命令：

     ```bash
     python3 scripts/stage_npm_packages.py \
       --release-version 0.128.4-test.0 \
       --workflow-url https://github.com/sd4494093/Widex/actions/runs/25867861572 \
       --package widex \
       --output-dir /tmp/widex-npm-repro
     ```

   - 错误核心：

     ```text
     Expected artifact not found: .../x86_64-unknown-linux-gnu/bwrap-x86_64-unknown-linux-gnu.zst
     ```

   - 根因：Widex Linux 平台包需要 `bwrap`，但 workflow Linux job 当时只构建并上传了 `codex`。

3. 修复 Linux `bwrap` artifact。

   - commit：`d88d31594c Fix Widex npm artifact staging`
   - 改动：
     - Linux target 构建 `--bin codex --bin bwrap`。
     - Linux staging 时复制 `bwrap-${target}`。
     - 对 native artifact 目录里的所有文件统一 zstd 压缩。

4. 第二次 run 遇到外部下载失败。

   - run：`25897158150`
   - 失败 job：`x86_64-apple-darwin`
   - 失败步骤：`Cargo build`
   - 真实日志：

     ```text
     failed to run custom build command for `v8 v146.4.0`
     Downloading https://github.com/denoland/rusty_v8/releases/download/v146.4.0/librusty_v8_release_x86_64-apple-darwin.a.gz
     HTTP Error 504: Gateway Time-out
     HTTP Error 503: Service Unavailable
     assertion failed: status.success()
     ```

   - 根因：`rusty_v8` 预编译库从 GitHub release/CDN 下载时连续 `503/504`，不是代码编译错误。

5. 修复外部下载抖动和队列阻塞。

   - commit：`b2f7745bdc Retry Widex native cargo builds`
     - Unix 和 Windows `Cargo build` 都增加最多 3 次重试。
   - commit：`33aa73d97f Cancel stale Widex artifact runs`
     - `cancel-in-progress: true`，新推送自动取消旧 run。

6. 最终成功。

   - run：`25899340377`
   - URL：https://github.com/sd4494093/Widex/actions/runs/25899340377
   - 结果：`completed / success`
   - 成功 job：
     - `x86_64-pc-windows-msvc`
     - `aarch64-pc-windows-msvc`
     - `x86_64-apple-darwin`
     - `aarch64-apple-darwin`
     - `x86_64-unknown-linux-gnu`
     - `x86_64-unknown-linux-musl`
     - `aarch64-unknown-linux-musl`
     - `Stage npm tarballs`
   - 上传 artifacts：
     - `widex-npm-tarballs-`
     - 7 个 native target artifacts

## 排障命令

查 workflow runs：

```bash
curl -s 'https://api.github.com/repos/sd4494093/Widex/actions/workflows/widex-npm-artifacts.yml/runs?branch=widex&per_page=5' \
  | jq -r '.workflow_runs[] | [.id, .status, .conclusion, .head_sha, .created_at, .html_url] | @tsv'
```

查某个 run 的 jobs：

```bash
curl -s 'https://api.github.com/repos/sd4494093/Widex/actions/runs/<run_id>/jobs?per_page=100' \
  | jq -r '.jobs[] | [.name, .status, .conclusion, .html_url] | @tsv'
```

查某个 job 的 steps：

```bash
curl -s 'https://api.github.com/repos/sd4494093/Widex/actions/jobs/<job_id>' \
  | jq -r '.steps[] | [.number, .name, .status, .conclusion, .started_at, .completed_at] | @tsv'
```

拉取 job 日志。公开 API 可能返回 403；本机 GitHub 凭据可读日志时用下面方式，注意不要打印 token：

```bash
cred="$(printf 'protocol=https\nhost=github.com\n\n' | git credential fill)"
user="$(printf '%s\n' "$cred" | awk -F= '$1=="username"{print $2}')"
token="$(printf '%s\n' "$cred" | awk -F= '$1=="password"{print $2}')"
curl -fsSL -u "$user:$token" \
  'https://api.github.com/repos/sd4494093/Widex/actions/jobs/<job_id>/logs' \
  -o /tmp/widex-job-<job_id>.log
```

快速定位失败：

```bash
rg -n 'error:|failed|HTTP Error|Process completed|timed out|Killed|exit code' /tmp/widex-job-<job_id>.log
tail -n 220 /tmp/widex-job-<job_id>.log
```

查 artifacts：

```bash
curl -s 'https://api.github.com/repos/sd4494093/Widex/actions/runs/<run_id>/artifacts?per_page=100' \
  | jq -r '.artifacts[] | [.name, .size_in_bytes, .expired] | @tsv'
```

## 发布 npm 前检查

1. 确认 artifact workflow 成功。

   - run status 必须是 `completed / success`。
   - 7 个 native target job 必须全部 `success`。
   - `Stage npm tarballs` 必须 `success`。
   - artifacts 里必须有 `widex-npm-tarballs-*`。

2. 下载并检查 tarballs。

   最少应包含 7 个 `.tgz`：

   - `widex-npm-<version>.tgz`
   - `widex-npm-linux-x64-<version>.tgz`
   - `widex-npm-linux-arm64-<version>.tgz`
   - `widex-npm-darwin-x64-<version>.tgz`
   - `widex-npm-darwin-arm64-<version>.tgz`
   - `widex-npm-win32-x64-<version>.tgz`
   - `widex-npm-win32-arm64-<version>.tgz`

3. 抽查 tarball 内容。

   ```bash
   npm pack --dry-run ./dist/npm/widex-npm-<version>.tgz
   tar -tzf ./dist/npm/widex-npm-linux-x64-<version>.tgz | rg 'vendor/.*/(codex|bwrap|rg)'
   tar -tzf ./dist/npm/widex-npm-win32-x64-<version>.tgz | rg 'codex-windows-sandbox-setup|codex-command-runner'
   ```

4. 发布顺序建议。

   先发布 6 个平台包，再发布 root `@wellau/widex`。root package 的 `optionalDependencies` 指向同版本平台包，先发平台包可以避免安装窗口期解析不到 optional dependency。

5. 当前不会自动 npm publish。

   如果要自动发布，需要单独设计发布 workflow，并配置 npm trusted publishing 或 npm token。不要误以为 `widex-npm-artifacts` 成功就已经发布到了 npm。

## 常见坑

- GitHub UI annotation 经常只显示 `Process completed with exit code 101`，不够定位；必须拉完整 job log。
- `rusty_v8`、`rg` 等外部下载可能偶发 `503/504`，native build 需要重试。
- Linux package 缺 `bwrap` 会在 `Stage npm tarballs` 才暴露，因为 native build job 本身可能是绿的。
- `rg` 由 manifest 下载，不要为了它额外改 native build matrix。
- 在 Apple Silicon 本机交叉编 `x86_64-apple-darwin` 可能拿到 arm64 的 LiveKit/WebRTC 静态库并链接失败；这不等价于 GitHub Intel runner 的失败。macOS x64 最可信的是 `macos-15-intel` CI 日志。
- 旧 run 如果不取消，会阻塞新 run。artifact workflow 应保持 `cancel-in-progress: true`。
- workflow push 触发时 `release_version` 默认还是测试版本逻辑；正式发布前要明确版本号，并确认 tarball 内 package.json 版本。
- 不要发布已经存在的 npm package/version。npm 版本不可覆盖。

## 最终发布 checklist

- [ ] 记录本次 upstream tag，例如 `rust-v0.130.0`。
- [ ] `widex` 分支干净，除明确忽略的本地文件外无未提交改动。
- [ ] merge commit 和 Widex overlay 修复已推送到 `origin/widex`。
- [ ] `widex-npm-artifacts` 最新 run 是 `completed / success`。
- [ ] 7 个 native target artifact 全部存在。
- [ ] `widex-npm-tarballs-*` artifact 存在，且包含 root + 6 个平台包。
- [ ] Linux x64 tarball 同时包含 gnu/musl 所需 payload，且包含 `bwrap`。
- [ ] Windows tarball 包含 sandbox setup 和 command runner。
- [ ] 本地或临时环境验证 `widex --version`、`widex --help`、启动 auth/splash 行为。
- [ ] 平台包先发布，root `@wellau/widex` 后发布。
- [ ] 发布完成后，用干净环境 `npm install -g @wellau/widex@<version>` 验证。
