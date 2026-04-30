#!/usr/bin/env python3
"""Install Codex native binaries (Rust CLI plus ripgrep helpers)."""

import argparse
from contextlib import contextmanager
import json
import os
import re
import shutil
import subprocess
import tarfile
import tempfile
import zipfile
from dataclasses import dataclass
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path
import sys
from typing import Iterable, Sequence
from urllib.parse import urlparse
from urllib.request import urlopen

SCRIPT_DIR = Path(__file__).resolve().parent
CODEX_CLI_ROOT = SCRIPT_DIR.parent
DEFAULT_WORKFLOW_URL = "https://github.com/openai/codex/actions/runs/17952349351"  # rust-v0.40.0
DEFAULT_GITHUB_REPO = "openai/codex"
VENDOR_DIR_NAME = "vendor"
RG_MANIFEST = CODEX_CLI_ROOT / "bin" / "rg"
BINARY_TARGETS = (
    "x86_64-unknown-linux-gnu",
    "x86_64-unknown-linux-musl",
    "aarch64-unknown-linux-musl",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
    "x86_64-pc-windows-msvc",
    "aarch64-pc-windows-msvc",
)


@dataclass(frozen=True)
class BinaryComponent:
    artifact_prefix: str  # matches the artifact filename prefix (e.g. codex-<target>.zst)
    dest_dir: str  # directory under vendor/<target>/ where the binary is installed
    binary_basename: str  # executable name inside dest_dir (before optional .exe)
    targets: tuple[str, ...] | None = None  # limit installation to specific targets


WINDOWS_TARGETS = tuple(target for target in BINARY_TARGETS if "windows" in target)

BINARY_COMPONENTS = {
    "codex": BinaryComponent(
        artifact_prefix="codex",
        dest_dir="codex",
        binary_basename="codex",
    ),
    "codex-responses-api-proxy": BinaryComponent(
        artifact_prefix="codex-responses-api-proxy",
        dest_dir="codex-responses-api-proxy",
        binary_basename="codex-responses-api-proxy",
    ),
    "codex-windows-sandbox-setup": BinaryComponent(
        artifact_prefix="codex-windows-sandbox-setup",
        dest_dir="codex",
        binary_basename="codex-windows-sandbox-setup",
        targets=WINDOWS_TARGETS,
    ),
    "codex-command-runner": BinaryComponent(
        artifact_prefix="codex-command-runner",
        dest_dir="codex",
        binary_basename="codex-command-runner",
        targets=WINDOWS_TARGETS,
    ),
}

RG_TARGET_PLATFORM_PAIRS: list[tuple[str, str]] = [
    ("x86_64-unknown-linux-gnu", "linux-x86_64"),
    ("x86_64-unknown-linux-musl", "linux-x86_64"),
    ("aarch64-unknown-linux-musl", "linux-aarch64"),
    ("x86_64-apple-darwin", "macos-x86_64"),
    ("aarch64-apple-darwin", "macos-aarch64"),
    ("x86_64-pc-windows-msvc", "windows-x86_64"),
    ("aarch64-pc-windows-msvc", "windows-aarch64"),
]
RG_TARGET_TO_PLATFORM = {target: platform for target, platform in RG_TARGET_PLATFORM_PAIRS}
DEFAULT_RG_TARGETS = [target for target, _ in RG_TARGET_PLATFORM_PAIRS]

# urllib.request.urlopen() defaults to no timeout (can hang indefinitely), which is painful in CI.
DOWNLOAD_TIMEOUT_SECS = 60


def _gha_enabled() -> bool:
    # GitHub Actions supports "workflow commands" (e.g. ::group:: / ::error::) that make logs
    # much easier to scan: groups collapse noisy sections and error annotations surface the
    # failure in the UI without changing the actual exception/traceback output.
    return os.environ.get("GITHUB_ACTIONS") == "true"


def _gha_escape(value: str) -> str:
    # Workflow commands require percent/newline escaping.
    return value.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")


def _gha_error(*, title: str, message: str) -> None:
    # Emit a GitHub Actions error annotation. This does not replace stdout/stderr logs; it just
    # adds a prominent summary line to the job UI so the root cause is easier to spot.
    if not _gha_enabled():
        return
    print(
        f"::error title={_gha_escape(title)}::{_gha_escape(message)}",
        flush=True,
    )


@contextmanager
def _gha_group(title: str):
    # Wrap a block in a collapsible log group on GitHub Actions. Outside of GHA this is a no-op
    # so local output remains unchanged.
    if _gha_enabled():
        print(f"::group::{_gha_escape(title)}", flush=True)
    try:
        yield
    finally:
        if _gha_enabled():
            print("::endgroup::", flush=True)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Install native CLI binaries.")
    parser.add_argument(
        "--workflow-url",
        help=(
            "GitHub Actions workflow URL that produced the artifacts. Defaults to a "
            "known good run when omitted."
        ),
    )
    parser.add_argument(
        "--repo",
        help=(
            "GitHub repository in OWNER/REPO format. When omitted, infer from --workflow-url, "
            "the current git checkout, or fall back to openai/codex."
        ),
    )
    parser.add_argument(
        "--component",
        dest="components",
        action="append",
        choices=tuple(list(BINARY_COMPONENTS) + ["rg"]),
        help=(
            "Limit installation to the specified components."
            " May be repeated. Defaults to codex, codex-windows-sandbox-setup,"
            " codex-command-runner, and rg."
        ),
    )
    parser.add_argument(
        "--target",
        dest="targets",
        action="append",
        choices=BINARY_TARGETS,
        help=(
            "Limit installation to the specified target triple."
            " May be repeated. Defaults to all supported targets."
        ),
    )
    parser.add_argument(
        "root",
        nargs="?",
        type=Path,
        help=(
            "Directory containing package.json for the staged package. If omitted, the "
            "repository checkout is used."
        ),
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()

    codex_cli_root = (args.root or CODEX_CLI_ROOT).resolve()
    vendor_dir = codex_cli_root / VENDOR_DIR_NAME
    vendor_dir.mkdir(parents=True, exist_ok=True)

    components = args.components or [
        "codex",
        "codex-windows-sandbox-setup",
        "codex-command-runner",
        "rg",
    ]
    targets = args.targets or list(BINARY_TARGETS)

    workflow_url = (args.workflow_url or DEFAULT_WORKFLOW_URL).strip()
    if not workflow_url:
        workflow_url = DEFAULT_WORKFLOW_URL

    workflow_id = workflow_url.rstrip("/").split("/")[-1]
    repo = resolve_github_repo(args.repo, workflow_url, codex_cli_root)
    print(f"Downloading native artifacts from {repo} workflow {workflow_id}...")

    with _gha_group(f"Download native artifacts from {repo} workflow {workflow_id}"):
        with tempfile.TemporaryDirectory(prefix="codex-native-artifacts-") as artifacts_dir_str:
            artifacts_dir = Path(artifacts_dir_str)
            _download_artifacts(repo, workflow_id, artifacts_dir, targets)
            install_binary_components(
                artifacts_dir,
                vendor_dir,
                [BINARY_COMPONENTS[name] for name in components if name in BINARY_COMPONENTS],
                targets,
            )

    if "rg" in components:
        with _gha_group("Fetch ripgrep binaries"):
            print("Fetching ripgrep binaries...")
            fetch_rg(vendor_dir, targets, manifest_path=RG_MANIFEST)

    print(f"Installed native dependencies into {vendor_dir}")
    return 0


def fetch_rg(
    vendor_dir: Path,
    targets: Sequence[str] | None = None,
    *,
    manifest_path: Path,
) -> list[Path]:
    """Download ripgrep binaries described by the DotSlash manifest."""

    if targets is None:
        targets = DEFAULT_RG_TARGETS

    if not manifest_path.exists():
        raise FileNotFoundError(f"DotSlash manifest not found: {manifest_path}")

    manifest = _load_manifest(manifest_path)
    platforms = manifest.get("platforms", {})

    vendor_dir.mkdir(parents=True, exist_ok=True)

    targets = list(targets)
    if not targets:
        return []

    task_configs: list[tuple[str, str, dict]] = []
    for target in targets:
        platform_key = RG_TARGET_TO_PLATFORM.get(target)
        if platform_key is None:
            raise ValueError(f"Unsupported ripgrep target '{target}'.")

        platform_info = platforms.get(platform_key)
        if platform_info is None:
            raise RuntimeError(f"Platform '{platform_key}' not found in manifest {manifest_path}.")

        task_configs.append((target, platform_key, platform_info))

    results: dict[str, Path] = {}
    max_workers = min(len(task_configs), max(1, (os.cpu_count() or 1)))

    print("Installing ripgrep binaries for targets: " + ", ".join(targets))

    with ThreadPoolExecutor(max_workers=max_workers) as executor:
        future_map = {
            executor.submit(
                _fetch_single_rg,
                vendor_dir,
                target,
                platform_key,
                platform_info,
                manifest_path,
            ): target
            for target, platform_key, platform_info in task_configs
        }

        for future in as_completed(future_map):
            target = future_map[future]
            try:
                results[target] = future.result()
            except Exception as exc:
                _gha_error(
                    title="ripgrep install failed",
                    message=f"target={target} error={exc!r}",
                )
                raise RuntimeError(f"Failed to install ripgrep for target {target}.") from exc
            print(f"  installed ripgrep for {target}")

    return [results[target] for target in targets]


def resolve_github_repo(repo_arg: str | None, workflow_url: str, root: Path) -> str:
    if repo_arg:
        return repo_arg

    workflow_repo = parse_repo_from_workflow_url(workflow_url)
    if workflow_repo:
        return workflow_repo

    checkout_repo = parse_repo_from_git_checkout(root)
    if checkout_repo:
        return checkout_repo

    return DEFAULT_GITHUB_REPO


def parse_repo_from_workflow_url(workflow_url: str) -> str | None:
    parsed = urlparse(workflow_url)
    parts = [part for part in parsed.path.split("/") if part]
    if len(parts) < 4:
        return None
    if parts[2] != "actions" or parts[3] != "runs":
        return None
    return f"{parts[0]}/{parts[1]}"


def parse_repo_from_git_checkout(root: Path) -> str | None:
    try:
        remote_url = subprocess.check_output(
            ["git", "remote", "get-url", "origin"],
            cwd=root,
            text=True,
            stderr=subprocess.DEVNULL,
        ).strip()
    except (OSError, subprocess.CalledProcessError):
        return None

    if remote_url.startswith("git@") and ":" in remote_url:
        remote_path = remote_url.split(":", 1)[1]
    else:
        parsed = urlparse(remote_url)
        remote_path = parsed.path.lstrip("/")

    if remote_path.endswith(".git"):
        remote_path = remote_path[:-4]
    parts = [part for part in remote_path.split("/") if part]
    if len(parts) < 2:
        return None
    return f"{parts[-2]}/{parts[-1]}"


def _download_artifacts(
    repo: str,
    workflow_id: str,
    dest_dir: Path,
    requested_targets: Sequence[str],
) -> None:
    cmd = [
        "gh",
        "run",
        "download",
        "--dir",
        str(dest_dir),
        "--repo",
        repo,
        workflow_id,
    ]
    if shutil.which("gh") is not None:
        result = subprocess.run(
            cmd,
            check=False,
            capture_output=True,
            text=True,
        )
        if result.returncode == 0:
            return
        stderr = result.stderr.strip()
        stdout = result.stdout.strip()
        message = stderr or stdout or f"exit code {result.returncode}"
        print(f"gh artifact download failed, falling back to public mirror: {message}")
    else:
        print("gh not found, falling back to public mirror download.")

    _download_artifacts_via_public_mirror(repo, workflow_id, dest_dir, requested_targets)


def _download_artifacts_via_public_mirror(
    repo: str,
    workflow_id: str,
    dest_dir: Path,
    requested_targets: Sequence[str],
) -> None:
    artifacts_url = (
        f"https://api.github.com/repos/{repo}/actions/runs/{workflow_id}/artifacts?per_page=100"
    )
    try:
        payload = _curl_json(artifacts_url)
        artifacts = payload.get("artifacts", [])
    except subprocess.CalledProcessError as exc:
        print(f"GitHub artifacts API failed, parsing run page instead: {exc}")
        artifacts = _artifact_listing_from_run_page(repo, workflow_id)
    if not artifacts:
        artifacts = _artifact_listing_from_run_page(repo, workflow_id)
    if not artifacts:
        raise RuntimeError(f"No artifacts found for workflow {workflow_id} in repo {repo}.")

    requested_target_set = set(requested_targets)
    selected_artifacts = [
        artifact
        for artifact in artifacts
        if artifact.get("id") and artifact.get("name") in requested_target_set
    ]

    missing_targets = sorted(
        requested_target_set - {artifact.get("name") for artifact in selected_artifacts}
    )
    if missing_targets:
        missing_list = ", ".join(missing_targets)
        raise RuntimeError(f"Missing required artifacts in public mirror listing: {missing_list}")

    max_workers = min(len(selected_artifacts), 4)
    with ThreadPoolExecutor(max_workers=max_workers) as executor:
        futures = {
            executor.submit(
                _download_and_extract_artifact,
                repo,
                workflow_id,
                int(artifact["id"]),
                str(artifact["name"]),
                dest_dir,
            ): str(artifact["name"])
            for artifact in selected_artifacts
        }
        for future in as_completed(futures):
            artifact_name = futures[future]
            future.result()
            print(f"  downloaded artifact {artifact_name}")


def _download_and_extract_artifact(
    repo: str,
    workflow_id: str,
    artifact_id: int,
    artifact_name: str,
    dest_dir: Path,
) -> None:
    download_url = f"https://nightly.link/{repo}/actions/artifacts/{artifact_id}.zip"
    cache_dir = Path(tempfile.gettempdir()) / "codex-artifact-cache" / repo.replace("/", "--") / workflow_id
    cache_dir.mkdir(parents=True, exist_ok=True)
    archive_path = cache_dir / f"{artifact_name}.zip"
    _curl_download(download_url, archive_path)

    artifact_dest = dest_dir / artifact_name
    if artifact_dest.exists():
        shutil.rmtree(artifact_dest)
    artifact_dest.mkdir(parents=True, exist_ok=True)

    with zipfile.ZipFile(archive_path) as archive:
        archive.extractall(artifact_dest)


def _curl_json(url: str) -> dict:
    stdout = _curl_text(url)
    try:
        return json.loads(stdout)
    except json.JSONDecodeError as exc:
        raise RuntimeError(f"Failed to parse JSON from {url}.") from exc


def _curl_text(url: str) -> str:
    return subprocess.check_output(
        [
            "curl",
            "--http1.1",
            "--max-time",
            "30",
            "--retry",
            "6",
            "--retry-all-errors",
            "--retry-delay",
            "2",
            "-fsSL",
            url,
        ],
        text=True,
    )


def _artifact_listing_from_run_page(repo: str, workflow_id: str) -> list[dict[str, str | int]]:
    run_page_url = f"https://github.com/{repo}/actions/runs/{workflow_id}"
    html = _curl_text(run_page_url)
    row_pattern = re.compile(
        r'<tr role="row" data-artifact-id="(?P<id>\d+)">.*?'
        r'<span class="text-bold color-fg-default" style="word-break: break-word;">\s*'
        r'(?P<name>[^<]+?)\s*</span>',
        re.S,
    )
    artifacts = [
        {"id": int(match["id"]), "name": match["name"].strip()}
        for match in row_pattern.finditer(html)
    ]
    if not artifacts:
        raise RuntimeError(f"Failed to parse artifacts from workflow run page: {run_page_url}")
    return artifacts


def _curl_download(url: str, dest: Path) -> None:
    dest.parent.mkdir(parents=True, exist_ok=True)
    subprocess.check_call(
        [
            "curl",
            "--http1.1",
            "--max-time",
            "0",
            "--retry",
            "6",
            "--retry-all-errors",
            "--retry-delay",
            "2",
            "--continue-at",
            "-",
            "-fL",
            "-o",
            str(dest),
            url,
        ]
    )


def install_binary_components(
    artifacts_dir: Path,
    vendor_dir: Path,
    selected_components: Sequence[BinaryComponent],
    requested_targets: Sequence[str],
) -> None:
    if not selected_components:
        return

    requested_targets_set = set(requested_targets)

    for component in selected_components:
        component_targets = [
            target
            for target in (component.targets or BINARY_TARGETS)
            if target in requested_targets_set
        ]
        if not component_targets:
            continue

        print(
            f"Installing {component.binary_basename} binaries for targets: "
            + ", ".join(component_targets)
        )
        max_workers = min(len(component_targets), max(1, (os.cpu_count() or 1)))
        with ThreadPoolExecutor(max_workers=max_workers) as executor:
            futures = {
                executor.submit(
                    _install_single_binary,
                    artifacts_dir,
                    vendor_dir,
                    target,
                    component,
                ): target
                for target in component_targets
            }
            for future in as_completed(futures):
                installed_path = future.result()
                print(f"  installed {installed_path}")


def _install_single_binary(
    artifacts_dir: Path,
    vendor_dir: Path,
    target: str,
    component: BinaryComponent,
) -> Path:
    artifact_subdir = artifacts_dir / target
    archive_name = _archive_name_for_target(component.artifact_prefix, target)
    archive_path = artifact_subdir / archive_name
    if not archive_path.exists():
        raise FileNotFoundError(f"Expected artifact not found: {archive_path}")

    dest_dir = vendor_dir / target / component.dest_dir
    dest_dir.mkdir(parents=True, exist_ok=True)

    binary_name = (
        f"{component.binary_basename}.exe" if "windows" in target else component.binary_basename
    )
    dest = dest_dir / binary_name
    dest.unlink(missing_ok=True)
    extract_archive(archive_path, "zst", None, dest)
    if "windows" not in target:
        dest.chmod(0o755)
    return dest


def _archive_name_for_target(artifact_prefix: str, target: str) -> str:
    if "windows" in target:
        return f"{artifact_prefix}-{target}.exe.zst"
    return f"{artifact_prefix}-{target}.zst"


def _fetch_single_rg(
    vendor_dir: Path,
    target: str,
    platform_key: str,
    platform_info: dict,
    manifest_path: Path,
) -> Path:
    providers = platform_info.get("providers", [])
    if not providers:
        raise RuntimeError(f"No providers listed for platform '{platform_key}' in {manifest_path}.")

    url = providers[0]["url"]
    archive_format = platform_info.get("format", "zst")
    archive_member = platform_info.get("path")
    digest = platform_info.get("digest")
    expected_size = platform_info.get("size")

    dest_dir = vendor_dir / target / "path"
    dest_dir.mkdir(parents=True, exist_ok=True)

    is_windows = platform_key.startswith("win")
    binary_name = "rg.exe" if is_windows else "rg"
    dest = dest_dir / binary_name

    with tempfile.TemporaryDirectory() as tmp_dir_str:
        tmp_dir = Path(tmp_dir_str)
        archive_filename = os.path.basename(urlparse(url).path)
        download_path = tmp_dir / archive_filename
        print(
            f"  downloading ripgrep for {target} ({platform_key}) from {url}",
            flush=True,
        )
        try:
            _download_file(url, download_path)
        except Exception as exc:
            _gha_error(
                title="ripgrep download failed",
                message=f"target={target} platform={platform_key} url={url} error={exc!r}",
            )
            raise RuntimeError(
                "Failed to download ripgrep "
                f"(target={target}, platform={platform_key}, format={archive_format}, "
                f"expected_size={expected_size!r}, digest={digest!r}, url={url}, dest={download_path})."
            ) from exc

        dest.unlink(missing_ok=True)
        try:
            extract_archive(download_path, archive_format, archive_member, dest)
        except Exception as exc:
            raise RuntimeError(
                "Failed to extract ripgrep "
                f"(target={target}, platform={platform_key}, format={archive_format}, "
                f"member={archive_member!r}, url={url}, archive={download_path})."
            ) from exc

    if not is_windows:
        dest.chmod(0o755)

    return dest


def _download_file(url: str, dest: Path) -> None:
    dest.parent.mkdir(parents=True, exist_ok=True)
    dest.unlink(missing_ok=True)

    with urlopen(url, timeout=DOWNLOAD_TIMEOUT_SECS) as response, open(dest, "wb") as out:
        shutil.copyfileobj(response, out)


def extract_archive(
    archive_path: Path,
    archive_format: str,
    archive_member: str | None,
    dest: Path,
) -> None:
    dest.parent.mkdir(parents=True, exist_ok=True)

    if archive_format == "zst":
        output_path = archive_path.parent / dest.name
        subprocess.check_call(
            ["zstd", "-f", "-d", str(archive_path), "-o", str(output_path)]
        )
        shutil.move(str(output_path), dest)
        return

    if archive_format == "tar.gz":
        if not archive_member:
            raise RuntimeError("Missing 'path' for tar.gz archive in DotSlash manifest.")
        with tarfile.open(archive_path, "r:gz") as tar:
            try:
                member = tar.getmember(archive_member)
            except KeyError as exc:
                raise RuntimeError(
                    f"Entry '{archive_member}' not found in archive {archive_path}."
                ) from exc
            tar.extract(member, path=archive_path.parent, filter="data")
        extracted = archive_path.parent / archive_member
        shutil.move(str(extracted), dest)
        return

    if archive_format == "zip":
        if not archive_member:
            raise RuntimeError("Missing 'path' for zip archive in DotSlash manifest.")
        with zipfile.ZipFile(archive_path) as archive:
            try:
                with archive.open(archive_member) as src, open(dest, "wb") as out:
                    shutil.copyfileobj(src, out)
            except KeyError as exc:
                raise RuntimeError(
                    f"Entry '{archive_member}' not found in archive {archive_path}."
                ) from exc
        return

    raise RuntimeError(f"Unsupported archive format '{archive_format}'.")


def _load_manifest(manifest_path: Path) -> dict:
    cmd = ["dotslash", "--", "parse", str(manifest_path)]
    stdout = subprocess.check_output(cmd, text=True)
    try:
        manifest = json.loads(stdout)
    except json.JSONDecodeError as exc:
        raise RuntimeError(f"Invalid DotSlash manifest output from {manifest_path}.") from exc

    if not isinstance(manifest, dict):
        raise RuntimeError(
            f"Unexpected DotSlash manifest structure for {manifest_path}: {type(manifest)!r}"
        )

    return manifest


if __name__ == "__main__":
    import sys

    sys.exit(main())
