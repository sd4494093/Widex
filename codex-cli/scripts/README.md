# npm releases

Use the staging helper in the repo root to generate npm tarballs for a release. For
example, to stage the Widex CLI, responses proxy, and SDK packages for version `0.6.0`:

```bash
./scripts/stage_npm_packages.py \
  --release-version 0.6.0 \
  --package widex \
  --package codex-responses-api-proxy \
  --package codex-sdk
```

This downloads the native artifacts once, hydrates `vendor/` for each package, and writes
tarballs to `dist/npm/`.

When `--package widex` is provided, the staging helper builds the lightweight
`@wellau/widex` meta package plus all platform-native `@wellau/widex` variants.
For Linux x64, the staged platform package carries both `x86_64-unknown-linux-gnu`
and `x86_64-unknown-linux-musl` vendor trees so standard npm installs can fall back
cleanly on older glibc hosts.

If you need to invoke `build_npm_package.py` directly, run
`codex-cli/scripts/install_native_deps.py` first and pass `--vendor-src` pointing to the
directory that contains the populated `vendor/` tree.
