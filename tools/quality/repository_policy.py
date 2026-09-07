#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Check repository policy without network access."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

from check_dco import commit_range, validate_dco

ROOT = Path(__file__).resolve().parents[2]
FULL_SHA = re.compile(r"^[0-9a-f]{40}$")
MARKDOWN_LINK = re.compile(r"!?\[[^\]]*\]\(([^)]+)\)")
SEMVER = re.compile(r"^\d+\.\d+\.\d+$")
DISPOSABLE_RUNNERS = {"ubuntu-24.04", "ubuntu-24.04-arm"}
CANARY_WORKFLOW = Path(".github/workflows/development-host-canary.yml")
CANARY_LABEL = "asb-development-v1-x86_64-ubuntu2404"


def fail(message: str) -> None:
    raise ValueError(message)


def tracked_files() -> list[Path]:
    output = subprocess.check_output(
        ["git", "-C", str(ROOT), "ls-files", "-z"], text=False
    )
    return [ROOT / item.decode() for item in output.rstrip(b"\0").split(b"\0") if item]


def validate_manifest() -> dict[str, object]:
    path = ROOT / "config/quality-tools.json"
    data = json.loads(path.read_text(encoding="utf-8"))
    if data.get("schema_version") != 1:
        fail("quality tool manifest schema_version must be 1")
    for name, version in data["rust"].items():
        if not isinstance(version, str) or not SEMVER.fullmatch(version):
            fail(f"Rust tool {name} does not have an exact semantic version")
    coverage = data["coverage"]
    for floor in ("workspace_lines", "critical_lines"):
        if not isinstance(coverage[floor], int) or not 0 <= coverage[floor] <= 100:
            fail(f"coverage floor {floor} is outside 0..100")
    if coverage["workspace_lines"] != 90 or coverage["critical_lines"] != 95:
        fail("coverage floors must remain 90% workspace and 95% critical")
    if coverage["critical_packages"] != ["asb-core", "asb-protocol", "asb-replay"]:
        fail("critical coverage package set changed")
    for name, value in data["actions"].items():
        if not isinstance(value, str) or not FULL_SHA.fullmatch(value):
            fail(f"action {name} is not pinned to a full commit")
    for tool, record in data["external"].items():
        if not SEMVER.fullmatch(record["version"]):
            fail(f"{tool} does not have an exact semantic version")
        for platform in ("linux_x86_64", "linux_aarch64"):
            digest = record[platform]["sha256"]
            if not re.fullmatch(r"[0-9a-f]{64}", digest):
                fail(f"{tool} {platform} does not have a SHA-256 pin")
    return data


def validate_workflows(manifest: dict[str, object]) -> None:
    expected = set(manifest["actions"].values())
    found: set[str] = set()
    for workflow in sorted((ROOT / ".github/workflows").glob("*.y*ml")):
        text = workflow.read_text(encoding="utf-8")
        relative = workflow.relative_to(ROOT)
        is_canary = relative == CANARY_WORKFLOW
        if "pull_request_target:" in text:
            fail(f"{relative} uses pull_request_target")
        if is_canary:
            triggers = re.search(r"^on:\n((?:  [^\n]*\n)*)", text, re.MULTILINE)
            if triggers is None or triggers.group(1) != "  workflow_dispatch:\n":
                fail(f"{relative} is not workflow_dispatch-only")
            if "uses:" in text:
                fail(f"{relative} may not execute repository or third-party actions")
            permissions = re.search(r"^permissions:\n((?:  [^\n]*\n)*)", text, re.MULTILINE)
            if permissions is None or permissions.group(1) != "  contents: read\n":
                fail(f"{relative} lacks exact read-only permissions")
        elif "self-hosted" in text:
            fail(f"{relative} uses a persistent runner")
        runners = re.findall(r"^\s*runs-on:\s*(.+?)\s*$", text, re.MULTILINE)
        if is_canary and runners != [f"[{CANARY_LABEL}]"]:
            fail(f"{relative} lacks the exact protected canary label")
        for raw_runner in runners:
            runner = raw_runner.split(" #", 1)[0].strip(" '\"")
            if runner == "${{ matrix.runner }}":
                if "runner: [ubuntu-24.04, ubuntu-24.04-arm]" not in text:
                    fail(f"{relative} has an unbounded runner matrix")
            elif not (is_canary and runner == f"[{CANARY_LABEL}]") and runner not in DISPOSABLE_RUNNERS:
                fail(f"{relative} uses non-disposable runner {runner}")
        for reference in re.findall(r"\buses:\s*[^\s@]+@([^\s#]+)", text):
            if not FULL_SHA.fullmatch(reference):
                fail(f"{relative} has mutable action pin {reference}")
            found.add(reference)
    missing = expected - found
    if missing:
        fail(f"manifested action pins are unused: {sorted(missing)}")
    unexpected = found - expected
    if unexpected:
        fail(f"workflow action pins are absent from the manifest: {sorted(unexpected)}")


def validate_sources(files: list[Path]) -> None:
    for path in files:
        if path.suffix == ".rs":
            first = path.read_text(encoding="utf-8").splitlines()[0]
            if first != "// SPDX-License-Identifier: MIT":
                fail(f"{path.relative_to(ROOT)} lacks the Rust SPDX header")
        elif path.suffix in {".py", ".sh"}:
            leading = path.read_text(encoding="utf-8").splitlines()[:2]
            if "# SPDX-License-Identifier: MIT" not in leading:
                fail(f"{path.relative_to(ROOT)} lacks a script SPDX header")


def validate_markdown(files: list[Path]) -> None:
    for path in files:
        if path.suffix.lower() != ".md":
            continue
        for target in MARKDOWN_LINK.findall(path.read_text(encoding="utf-8")):
            target = target.strip()
            if (
                not target
                or target.startswith(("#", "http://", "https://", "mailto:"))
                or "://" in target
            ):
                continue
            clean = target.split("#", 1)[0]
            if clean and not (path.parent / clean).resolve().exists():
                fail(f"{path.relative_to(ROOT)} has missing local link {target}")


def validate_commits(base: str | None, head: str) -> None:
    allowed = ROOT / "config/allowed_signers"
    revisions = commit_range(ROOT, base, head)
    if not revisions:
        fail("commit policy received an empty revision range")
    validate_dco(ROOT, revisions)
    for revision in revisions:
        result = subprocess.run(
            [
                "git",
                "-C",
                str(ROOT),
                "-c",
                "gpg.format=ssh",
                "-c",
                f"gpg.ssh.allowedSignersFile={allowed}",
                "verify-commit",
                revision,
            ],
            text=True,
            capture_output=True,
            check=False,
        )
        if result.returncode:
            fail(f"{revision} lacks an allowed SSH signature: {result.stderr.strip()}")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base")
    parser.add_argument("--head", default="HEAD")
    parser.add_argument("--skip-commits", action="store_true")
    args = parser.parse_args()
    try:
        manifest = validate_manifest()
        files = tracked_files()
        validate_workflows(manifest)
        validate_sources(files)
        validate_markdown(files)
        if not args.skip_commits:
            validate_commits(args.base, args.head)
    except (OSError, subprocess.CalledProcessError, TypeError, ValueError, KeyError) as error:
        print(f"repository policy: {error}", file=sys.stderr)
        return 1
    print("repository policy: all checks passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
