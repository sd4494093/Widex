# @wellau/widex

Widex is the WellAU-maintained coding engine distribution built from the Widex fork.

## Install

```bash
npm install -g @wellau/widex
```

Then start Widex with:

```bash
widex
```

## What gets installed

`@wellau/widex` is a lightweight launcher package. During installation it resolves the matching native Widex payload for your platform and exposes the `widex` command.

## Authentication

Widex uses its own isolated home directory by default and does not reuse the upstream Codex CLI state.

Default Widex runtime state:

```text
~/.widex-codex/
```

On first launch, if no Widex API key is available yet, the startup flow will prompt the user to input a Widex Key (WillAU API Key).

## Upgrade

```bash
npm install -g @wellau/widex@latest
```

## Uninstall

```bash
npm uninstall -g @wellau/widex
```
