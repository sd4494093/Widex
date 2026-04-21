# Widex npm 发版流程

目标：让客户可直接执行：

```bash
npm install -g @wellau/widex
widex
```

## 1. 发布前确认

- 当前分支已完成要发布的代码合并
- `origin/widex` 已推送最新提交
- Rust release 二进制已能通过本地验证
- 你拥有 npm scope `@wellau` 的发布权限

建议先确认：

```bash
git status --short --branch
npm whoami
npm access ls-packages <your-npm-user-or-org>
```

## 2. 本地基础验证

先验证 npm 元包本身可以正确打包：

```bash
cd /home/will/data/widex/codex-cli
npm pack --pack-destination /tmp/widex-npm-pack
```

再验证 Widex 当前 release 二进制：

```bash
cd /home/will/data/widex/codex-rs
cargo build -p codex-cli --bin codex --profile widex-release
```

## 3. 生成 npm staging 包

如果已经有对应版本的 release workflow 产物，可直接执行：

```bash
cd /home/will/data/widex
./scripts/stage_npm_packages.py \
  --release-version <VERSION> \
  --package widex
```

产物默认输出到：

```text
dist/npm/
```

正常情况下会得到：

- `widex-npm-<VERSION>.tgz`
- `widex-npm-linux-x64-<VERSION>.tgz`
- `widex-npm-linux-arm64-<VERSION>.tgz`
- `widex-npm-darwin-x64-<VERSION>.tgz`
- `widex-npm-darwin-arm64-<VERSION>.tgz`
- `widex-npm-win32-x64-<VERSION>.tgz`
- `widex-npm-win32-arm64-<VERSION>.tgz`

如果不想自动查 workflow，也可以先准备好 native vendor 目录，再单独调用：

```bash
cd /home/will/data/widex
python3 codex-cli/scripts/build_npm_package.py \
  --package widex \
  --release-version <VERSION> \
  --staging-dir /tmp/widex-stage \
  --pack-output /tmp/widex-npm-<VERSION>.tgz
```

## 4. 发布顺序

先发平台包，再发元包。

原因：`@wellau/widex` 元包依赖这些平台包别名；如果元包先发布，客户在安装时可能会先撞到 optional dependency 缺失。

示例：

```bash
npm publish dist/npm/widex-npm-linux-x64-<VERSION>.tgz --access public
npm publish dist/npm/widex-npm-linux-arm64-<VERSION>.tgz --access public
npm publish dist/npm/widex-npm-darwin-x64-<VERSION>.tgz --access public
npm publish dist/npm/widex-npm-darwin-arm64-<VERSION>.tgz --access public
npm publish dist/npm/widex-npm-win32-x64-<VERSION>.tgz --access public
npm publish dist/npm/widex-npm-win32-arm64-<VERSION>.tgz --access public
npm publish dist/npm/widex-npm-<VERSION>.tgz --access public
```

## 5. 发布后验证

建议在一台干净机器或干净用户环境验证：

```bash
npm install -g @wellau/widex@<VERSION>
widex --version
widex
```

重点确认：

- 安装命令是 `npm install -g @wellau/widex`
- 启动命令是 `widex`
- Widex 使用的是 `~/.widex-codex/`
- 无 key 时，启动页显示 `Input Widex Key (WillAU API Key)` / Quit
- 有 key 时，启动页允许直接继续

## 6. 当前链路约定

当前 Widex npm 包已经收口为：

- 包名：`@wellau/widex`
- CLI 命令：`widex`
- 平台包：`@wellau/widex-<platform>`
- npm 包页 README：`codex-cli/README.md`
- 元包 staging 脚本入口：`./scripts/stage_npm_packages.py --package widex`
