# Widex 快速跟随上游（openai/codex）更新指南

本仓库有两条主线分支：

- `main`：尽量保持“等于上游 upstream/main”（用于快速跟随上游）
- `widex`：你们日常开发/运行/发布分支（包含 Widex 的定制）

目标：上游更新频繁时，用最少命令把 `main` 同步到 `upstream/main`，再把 `main` 合入 `widex`。

## 0) 前置检查

在仓库根目录执行：

```bash
git remote -v
```

确保至少有：

- `origin` 指向你们 fork（例如 `git@github-sd4494093:sd4494093/Widex.git`）
- `upstream` 指向上游（例如 `git@github.com:openai/codex.git`）

注：如果你们用“多 GitHub 账号/多 SSH key”方案，`github-sd4494093` 这类 host alias 取决于
`~/.ssh/config`；如果没做 alias，也可以用标准的 `git@github.com:sd4494093/Widex.git`。

并且工作区干净：

```bash
git status --porcelain=v1
```

若有本地未提交变更，请先提交或 stash。

## 1) 快速同步 main 到 upstream/main（优先 fast-forward）

```bash
git fetch upstream
git fetch origin

git checkout main
# 理想情况：main 只做 fast-forward
git merge --ff-only upstream/main

git push origin main
```

如果 `--ff-only` 失败（说明 `main` 上有额外提交，不能快进），且你确认 `main` 应当“完全等于上游”，可用硬同步：

```bash
git checkout main
git reset --hard upstream/main
# 注意：这会改写 origin/main 历史
git push --force-with-lease origin main
```

## 2) 把 main 合入 widex

```bash
git checkout widex
git merge main
# 如有冲突：解决冲突 -> git add -> git commit

git push origin widex
```

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

## 3) 本地重建 Widex（底层 codex 二进制）（可选，但推荐）

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

## 4) 常见问题

- 合并冲突多：
  - 先保证 `main` 是干净的 upstream/main，再合入 `widex`；冲突只会发生在 `widex` 侧。
- 合入后编译失败：
  - 多半是上游新增字段/协议变更导致（例如 struct 新字段），按报错逐个补齐即可。
- 你不想改写 `origin/main`：
  - 那就不要用 `reset --hard + force push`，而是让 `main` 保留历史并手动 merge；但这会让 `main` 不再“等于上游”，后续同步会更难。
