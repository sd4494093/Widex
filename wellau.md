# WellAU Widex Maintenance Notes

This repo is the working checkout for maintaining Widex under:

```text
git@github.com:sd4494093/Widex.git
```

Use this directory for Widex source changes:

```bash
cd /Users/butcher/Projects/Widex
```

Do not use `/Users/butcher/Projects/Wellau` for Widex source work.

## Local Git Setup

The local `origin` should point to the upstream Widex repository:

```bash
git remote -v
```

Expected:

```text
origin  git@github.com:sd4494093/Widex.git (fetch)
origin  git@github.com:sd4494093/Widex.git (push)
```

The normal development branch is:

```text
widex
```

Check:

```bash
git branch --show-current
git status --short --branch
```

The repository-local SSH config should use the Widex key:

```bash
git config --get core.sshCommand
```

Expected:

```text
ssh -i ~/.ssh/widex_ed25519 -o IdentitiesOnly=yes
```

## Daily Development Flow

```bash
cd /Users/butcher/Projects/Widex
git switch widex
git pull

# edit files

git status
git add <files>
git commit -m "your commit message"
git push
```

Pushing to `widex` triggers the manual-safe artifact workflow:

```text
.github/workflows/widex-npm-artifacts.yml
```

This workflow builds native artifacts and npm tarballs. It does not publish npm.

## GitHub Actions Artifact Workflow

Workflow:

```text
widex-npm-artifacts
```

Run page:

```text
https://github.com/sd4494093/Widex/actions
```

The workflow builds these native targets:

```text
x86_64-apple-darwin
aarch64-apple-darwin
x86_64-unknown-linux-gnu
x86_64-unknown-linux-musl
aarch64-unknown-linux-musl
x86_64-pc-windows-msvc
aarch64-pc-windows-msvc
```

The final artifact is named like:

```text
widex-npm-tarballs-
```

It contains:

```text
widex-npm-<version>.tgz
widex-npm-linux-x64-<version>.tgz
widex-npm-linux-arm64-<version>.tgz
widex-npm-darwin-x64-<version>.tgz
widex-npm-darwin-arm64-<version>.tgz
widex-npm-win32-x64-<version>.tgz
widex-npm-win32-arm64-<version>.tgz
```

## Checking Downloaded npm Tarballs

After downloading the artifact zip:

```bash
mkdir -p /tmp/widex-npm-check
unzip -q ~/Downloads/widex-npm-tarballs-.zip -d /tmp/widex-npm-check
ls -lh /tmp/widex-npm-check
```

Check package metadata:

```bash
cd /tmp/widex-npm-check

for f in *.tgz; do
  echo "=== $f"
  tar -xOf "$f" package/package.json | jq '{name, version, optionalDependencies, os, cpu, bin}'
done
```

Check package contents:

```bash
for f in *.tgz; do
  echo "=== $f"
  tar -tzf "$f" \
    | sed 's#^package/##' \
    | grep -E '^(bin/widex.js|vendor/.*/(codex|path)/(codex|codex.exe|rg|rg.exe|codex-windows-sandbox-setup.exe|codex-command-runner.exe)|package.json$)' \
    | sort
done
```

## npm Publishing Order

Always publish platform packages first, then publish the main package last.

Platform packages:

```text
@wellau/widex-darwin-arm64
@wellau/widex-darwin-x64
@wellau/widex-linux-arm64
@wellau/widex-linux-x64
@wellau/widex-win32-arm64
@wellau/widex-win32-x64
```

Main package:

```text
@wellau/widex
```

Reason: the main package declares optional dependencies on the platform packages for the same version.

For a test release, use a prerelease version such as:

```text
0.128.5-test.0
```

and publish with:

```bash
npm publish <tarball.tgz> --access public --tag test
```

For an official release, use a stable version such as:

```text
0.128.5
```

and publish with:

```bash
npm publish <tarball.tgz> --access public --tag latest
```

Check npm state:

```bash
npm view @wellau/widex dist-tags version --json --registry=https://registry.npmjs.org/
npm view @wellau/widex@<version> optionalDependencies --json --registry=https://registry.npmjs.org/
```

Users install the latest official release with:

```bash
npm install -g @wellau/widex
```

Users install the test release with:

```bash
npm install -g @wellau/widex@test
```

## Security Notes

Never commit npm tokens or SSH private keys.

If a token is pasted into chat or logs, revoke it in npm and create a new one.

Prefer temporary npm config files for one-off publishing:

```bash
umask 077
printf '%s\n' \
  'registry=https://registry.npmjs.org/' \
  '//registry.npmjs.org/:_authToken=<TOKEN>' \
  > /tmp/widex-npm-publish.npmrc
```

Use it with:

```bash
npm publish <tarball.tgz> \
  --userconfig /tmp/widex-npm-publish.npmrc \
  --registry=https://registry.npmjs.org/ \
  --access public \
  --tag latest
```

Then remove it:

```bash
rm -f /tmp/widex-npm-publish.npmrc
```

