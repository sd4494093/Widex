#!/usr/bin/env node
// Unified entry point for the Widex CLI.

import { spawn, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "fs";
import { createRequire } from "node:module";
import os from "os";
import path from "path";
import { fileURLToPath } from "url";

// __dirname equivalent in ESM
const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const require = createRequire(import.meta.url);

const PLATFORM_PACKAGE_BY_TARGET = {
  "x86_64-unknown-linux-gnu": "@wellau/widex-linux-x64-gnu",
  "x86_64-unknown-linux-musl": "@wellau/widex-linux-x64-musl",
  "aarch64-unknown-linux-musl": "@wellau/widex-linux-arm64",
  "x86_64-apple-darwin": "@wellau/widex-darwin-x64",
  "aarch64-apple-darwin": "@wellau/widex-darwin-arm64",
  "x86_64-pc-windows-msvc": "@wellau/widex-win32-x64",
  "aarch64-pc-windows-msvc": "@wellau/widex-win32-arm64",
};

const { platform, arch } = process;

let targetTriples = [];
switch (platform) {
  case "linux":
  case "android":
    switch (arch) {
      case "x64":
        targetTriples = [
          "x86_64-unknown-linux-gnu",
          "x86_64-unknown-linux-musl",
        ];
        break;
      case "arm64":
        targetTriples = ["aarch64-unknown-linux-musl"];
        break;
      default:
        break;
    }
    break;
  case "darwin":
    switch (arch) {
      case "x64":
        targetTriples = ["x86_64-apple-darwin"];
        break;
      case "arm64":
        targetTriples = ["aarch64-apple-darwin"];
        break;
      default:
        break;
    }
    break;
  case "win32":
    switch (arch) {
      case "x64":
        targetTriples = ["x86_64-pc-windows-msvc"];
        break;
      case "arm64":
        targetTriples = ["aarch64-pc-windows-msvc"];
        break;
      default:
        break;
    }
    break;
  default:
    break;
}

if (targetTriples.length === 0) {
  throw new Error(`Unsupported platform: ${platform} (${arch})`);
}

const codexBinaryName = process.platform === "win32" ? "codex.exe" : "codex";
const localVendorRoot = path.join(__dirname, "..", "vendor");

function codexBinaryPath(vendorRoot, targetTriple) {
  return path.join(vendorRoot, targetTriple, "codex", codexBinaryName);
}

function resolveTargetTriple(vendorRoot) {
  return targetTriples.find((targetTriple) =>
    existsSync(codexBinaryPath(vendorRoot, targetTriple)),
  );
}

function resolveInstalledPlatforms() {
  const platforms = [];
  for (const targetTriple of targetTriples) {
    const platformPackage = PLATFORM_PACKAGE_BY_TARGET[targetTriple];
    if (!platformPackage) {
      continue;
    }

    try {
      const packageJsonPath = require.resolve(`${platformPackage}/package.json`);
      const candidateVendorRoot = path.join(path.dirname(packageJsonPath), "vendor");
      if (existsSync(codexBinaryPath(candidateVendorRoot, targetTriple))) {
        platforms.push({
          vendorRoot: candidateVendorRoot,
          resolvedTargetTriple: targetTriple,
          platformPackage,
        });
      }
    } catch {
      // Optional platform package not installed.
    }
  }

  const resolvedTargetTriple = resolveTargetTriple(localVendorRoot);
  if (resolvedTargetTriple) {
    platforms.push({
      vendorRoot: localVendorRoot,
      resolvedTargetTriple,
      platformPackage: PLATFORM_PACKAGE_BY_TARGET[resolvedTargetTriple],
    });
  }

  return platforms;
}

function missingPlatformPackagesMessage() {
  const platformPackages = [
    ...new Set(
      targetTriples
        .map((targetTriple) => PLATFORM_PACKAGE_BY_TARGET[targetTriple])
        .filter(Boolean),
    ),
  ];
  const packageManager = detectPackageManager();
  const updateCommand =
    packageManager === "bun"
      ? "bun install -g @wellau/widex@latest"
      : "npm install -g @wellau/widex@latest";
  return `Missing optional dependencies (${platformPackages.join(", ")}). Reinstall Widex: ${updateCommand}`;
}

const installedPlatforms = resolveInstalledPlatforms();
if (installedPlatforms.length === 0) {
  throw new Error(missingPlatformPackagesMessage());
}

function getUpdatedPath(newDirs) {
  const pathSep = process.platform === "win32" ? ";" : ":";
  const existingPath = process.env.PATH || "";
  const updatedPath = [
    ...newDirs,
    ...existingPath.split(pathSep).filter(Boolean),
  ].join(pathSep);
  return updatedPath;
}

/**
 * Use heuristics to detect the package manager that was used to install Widex
 * in order to give the user a hint about how to update it.
 */
function detectPackageManager() {
  const userAgent = process.env.npm_config_user_agent || "";
  if (/\bbun\//.test(userAgent)) {
    return "bun";
  }

  const execPath = process.env.npm_execpath || "";
  if (execPath.includes("bun")) {
    return "bun";
  }

  if (
    __dirname.includes(".bun/install/global") ||
    __dirname.includes(".bun\\install\\global")
  ) {
    return "bun";
  }

  return userAgent ? "npm" : null;
}

function widexDefaultConfig() {
  return `model_provider = "custom"
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

[features]
apps = false
child_agents_md = true
codex_git_commit = true
js_repl = true
memories = true
multi_agent = true
prevent_idle_sleep = true
responses_websockets = true
responses_websockets_v2 = true
skill_env_var_dependency_prompt = true
sqlite = true
undo = true
`;
}

function ensureRootSetting(configText, key, value) {
  const pattern = new RegExp(`^\\s*${key}\\s*=`, "m");
  if (pattern.test(configText)) {
    return configText;
  }

  return `${key} = ${value}\n${configText}`;
}

function ensureBlock(configText, header, block) {
  const escapedHeader = header.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const pattern = new RegExp(`^\\s*${escapedHeader}\\s*$`, "m");
  if (pattern.test(configText)) {
    return configText;
  }

  return `${configText.replace(/\s*$/, "")}\n\n${block}\n`;
}

function ensureWidexConfig(codexHome) {
  mkdirSync(codexHome, { recursive: true });
  const configPath = path.join(codexHome, "config.toml");
  if (!existsSync(configPath)) {
    writeFileSync(configPath, widexDefaultConfig(), { mode: 0o600 });
    return;
  }

  let configText = readFileSync(configPath, "utf8");
  configText = ensureRootSetting(configText, "model_provider", '"custom"');
  configText = ensureRootSetting(configText, "model", '"gpt-5.4"');
  configText = ensureRootSetting(configText, "model_reasoning_effort", '"high"');
  configText = ensureRootSetting(configText, "disable_response_storage", "true");
  configText = ensureRootSetting(configText, "personality", '"pragmatic"');
  configText = ensureRootSetting(configText, "cli_auth_credentials_store", '"file"');
  configText = ensureBlock(
    configText,
    "[model_providers.custom]",
    `[model_providers.custom]
name = "custom"
wire_api = "responses"
requires_openai_auth = true
base_url = "https://api.wellau.com/v1"`,
  );
  configText = ensureBlock(
    configText,
    "[features]",
    `[features]
apps = false
child_agents_md = true
codex_git_commit = true
js_repl = true
memories = true
multi_agent = true
prevent_idle_sleep = true
responses_websockets = true
responses_websockets_v2 = true
skill_env_var_dependency_prompt = true
sqlite = true
undo = true`,
  );
  writeFileSync(configPath, configText);
}

function resolveWidexCodexHome() {
  const defaultCodexHome = path.join(os.homedir(), ".widex");
  const explicitWidexHome = process.env.WIDEX_CODEX_HOME;
  if (explicitWidexHome && explicitWidexHome.length > 0) {
    return explicitWidexHome;
  }

  const inheritedCodexHome = process.env.CODEX_HOME;
  if (inheritedCodexHome && inheritedCodexHome !== defaultCodexHome) {
    // eslint-disable-next-line no-console
    console.error(
      `widex: ignoring inherited CODEX_HOME='${inheritedCodexHome}' and using '${defaultCodexHome}'`,
    );
  }

  return defaultCodexHome;
}

const codexHome = resolveWidexCodexHome();
ensureWidexConfig(codexHome);

function executionContextForPlatform(installedPlatform) {
  const { vendorRoot, resolvedTargetTriple } = installedPlatform;
  const archRoot = path.join(vendorRoot, resolvedTargetTriple);
  const binaryPath = path.join(archRoot, "codex", codexBinaryName);

  const additionalDirs = [];
  const pathDir = path.join(archRoot, "path");
  if (existsSync(pathDir)) {
    additionalDirs.push(pathDir);
  }

  const env = {
    ...process.env,
    PATH: getUpdatedPath(additionalDirs),
    CODEX_HOME: codexHome,
    WIDEX_CMD: process.env.WIDEX_CMD || "widex",
    CODEX_CMD: process.env.CODEX_CMD || "widex",
  };
  const packageManagerEnvVar =
    detectPackageManager() === "bun"
      ? "CODEX_MANAGED_BY_BUN"
      : "CODEX_MANAGED_BY_NPM";
  env[packageManagerEnvVar] = "1";

  return { binaryPath, env };
}

function selectRunnablePlatform() {
  const failures = [];
  for (const installedPlatform of installedPlatforms) {
    const { binaryPath, env } = executionContextForPlatform(installedPlatform);
    const result = spawnSync(binaryPath, ["--version"], {
      env,
      encoding: "utf8",
    });
    if (result.status === 0) {
      return { ...installedPlatform, binaryPath, env };
    }

    const message = [result.error?.message, result.stderr, result.stdout]
      .filter(Boolean)
      .join("\n")
      .trim();
    failures.push(
      `${installedPlatform.resolvedTargetTriple}: ${message || `exit ${result.status ?? "unknown"}`}`,
    );
  }

  throw new Error(
    `No runnable Widex native binary found for ${platform} (${arch}).\n${failures.join("\n")}`,
  );
}

const { binaryPath, env } = selectRunnablePlatform();

const child = spawn(binaryPath, process.argv.slice(2), {
  stdio: "inherit",
  env,
});

child.on("error", (err) => {
  // Typically triggered when the binary is missing or not executable.
  // Re-throwing here will terminate the parent with a non-zero exit code
  // while still printing a helpful stack trace.
  // eslint-disable-next-line no-console
  console.error(err);
  process.exit(1);
});

// Forward common termination signals to the child so that it shuts down
// gracefully. In the handler we temporarily disable the default behavior of
// exiting immediately; once the child has been signaled we simply wait for
// its exit event which will in turn terminate the parent (see below).
const forwardSignal = (signal) => {
  if (child.killed) {
    return;
  }
  try {
    child.kill(signal);
  } catch {
    /* ignore */
  }
};

["SIGINT", "SIGTERM", "SIGHUP"].forEach((sig) => {
  process.on(sig, () => forwardSignal(sig));
});

// When the child exits, mirror its termination reason in the parent so that
// shell scripts and other tooling observe the correct exit status.
// Wrap the lifetime of the child process in a Promise so that we can await
// its termination in a structured way. The Promise resolves with an object
// describing how the child exited: either via exit code or due to a signal.
const childResult = await new Promise((resolve) => {
  child.on("exit", (code, signal) => {
    if (signal) {
      resolve({ type: "signal", signal });
    } else {
      resolve({ type: "code", exitCode: code ?? 1 });
    }
  });
});

if (childResult.type === "signal") {
  // Re-emit the same signal so that the parent terminates with the expected
  // semantics (this also sets the correct exit code of 128 + n).
  process.kill(process.pid, childResult.signal);
} else {
  process.exit(childResult.exitCode);
}
