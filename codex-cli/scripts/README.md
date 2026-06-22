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

This downloads the required native package archive artifacts, hydrates `vendor/` for
each package, and writes tarballs to `dist/npm/`.

When `--package widex` is provided, the staging helper builds the lightweight
`@wellau/widex` meta package plus all platform-native `@wellau/widex` variants.
Linux x64 ships as two optional packages (`@wellau/widex-linux-x64-gnu` and
`@wellau/widex-linux-x64-musl`) so glibc and musl hosts each install only one
native payload. The launcher prefers the gnu package on standard glibc distros
and falls back to musl.

Direct `build_npm_package.py` invocations are still useful for package-specific
debugging, but native packages expect `--vendor-src` to point at a prehydrated
`vendor/` tree. Release packaging should use `scripts/stage_npm_packages.py`.
