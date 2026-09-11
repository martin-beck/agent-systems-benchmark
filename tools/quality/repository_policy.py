#!/usr/bin/env python3
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Check repository policy without network access."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

if __package__:
    from .check_dco import commit_range, validate_dco
else:
    from check_dco import commit_range, validate_dco

ROOT = Path(__file__).resolve().parents[2]
FULL_SHA = re.compile(r"^[0-9a-f]{40}$")
MARKDOWN_LINK = re.compile(r"!?\[[^\]]*\]\(([^)]+)\)")
SEMVER = re.compile(r"^\d+\.\d+\.\d+$")
DISPOSABLE_RUNNERS = {"ubuntu-24.04", "ubuntu-24.04-arm"}
CANARY_WORKFLOW = Path(".github/workflows/development-host-canary.yml")
CANARY_LABEL = "asb-development-v1-x86_64-ubuntu2404"
TRUSTED_WORKFLOW = Path(".github/workflows/development-host-trusted.yml")
QUALITY_WORKFLOW = Path(".github/workflows/quality.yml")
EMULATED_AARCH64_WORKFLOW = Path(".github/workflows/emulated-aarch64.yml")
HUAWEI_COPYRIGHT = "Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved."
SPDX_MIT = "SPDX-License-Identifier: MIT"
EXTENSIONLESS_SOURCES = frozenset({Path("tools/awq")})
TLA_MODULE = re.compile(r"^---- MODULE (?P<name>[A-Za-z][A-Za-z0-9_]*) ----$")
ALLOY_MODULE = re.compile(
    r"^module (?P<name>[A-Za-z][A-Za-z0-9_]*(?:/[A-Za-z][A-Za-z0-9_]*)*)$"
)
OPTIONAL_ARTIFACT_ACTION = (
    "actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02"
)
PROTECTED_CONDITION = (
    "github.repository == 'martin-beck/agent-systems-benchmark' "
    "&& github.ref == 'refs/heads/main'"
)
PRIVATE_OUTPUT_PATTERNS = (
    re.compile(
        r"(?:echo|printf)\b[^\n]*(?:RUNNER_(?:NAME|TEMP|WORKSPACE)|HOSTNAME|runner\.(?:name|temp))",
        re.IGNORECASE | re.MULTILINE,
    ),
    re.compile(
        r"(?:^|[;&|]\s*)(?:hostname|printenv|env|uname\s+-n)\s*(?:$|[|;&])",
        re.MULTILINE,
    ),
    re.compile(r"(?:RUNNER_NAME|HOSTNAME|runner\.name|runner\.temp|/etc/hostname)"),
    re.compile(r"^\s*set\s+-(?:[^\n]*x|[^\n]*o\s+xtrace)\s*$", re.MULTILINE),
)
GITHUB_WEB_FLOW_KEY = ROOT / "config/github_web_flow.gpg"
GITHUB_WEB_FLOW_KEY_SHA256 = (
    "6e8af687f60cf3f403151c8fb1b26e95e6f9e424ca60cc8f3787bd4466a3ef84"
)
GITHUB_WEB_FLOW_FINGERPRINT = "968479A1AFF927E37D1A566BB5690EEEBB952194"
GITHUB_COMMITTER = "GitHub <noreply@github.com>"
PROTECTED_EVENT = "push"
PROTECTED_REF = "refs/heads/main"
QUALITY_SIGNATURE_STEP = """      - name: Enforce repository and commit policy
        env:
          EVENT_NAME: ${{ github.event_name }}
          EVENT_REF: ${{ github.ref }}
          RANGE_BASE: ${{ steps.range.outputs.base }}
          RANGE_HEAD: ${{ steps.range.outputs.head }}
        run: |
          mode=ssh-only
          if [[ "$EVENT_NAME" == push && "$EVENT_REF" == refs/heads/main ]]; then
            mode=protected-main
          fi
          args=(--head "$RANGE_HEAD" --mode "$mode" --event "$EVENT_NAME" --ref "$EVENT_REF")
          if [[ -n "$RANGE_BASE" ]]; then
            args+=(--base "$RANGE_BASE")
          fi
          python3 tools/quality/repository_policy.py "${args[@]}"
"""


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
        is_trusted = relative == TRUSTED_WORKFLOW
        is_quality = relative == QUALITY_WORKFLOW
        is_emulated_aarch64 = relative == EMULATED_AARCH64_WORKFLOW
        is_protected = is_canary or is_trusted
        if "pull_request_target:" in text:
            fail(f"{relative} uses pull_request_target")
        if "ubuntu-24.04-arm" in text and ("  pull_request:\n" in text or "  push:\n" in text):
            fail(f"{relative} makes native ARM64 an automatic development gate")
        if is_emulated_aarch64:
            required_emulated_fragments = (
                '"on":\n  push:\n    branches: [main]\n  pull_request:\n',
                "runs-on: ubuntu-24.04\n",
                "QEMU_LD_PREFIX:",
                "trap cleanup EXIT",
            )
            if any(fragment not in text for fragment in required_emulated_fragments):
                fail(f"{relative} is not a required bounded QEMU AArch64 gate")
        if is_protected:
            triggers = re.search(r"^on:\n((?:  [^\n]*\n)*)", text, re.MULTILINE)
            if triggers is None or triggers.group(1) != "  workflow_dispatch:\n":
                fail(f"{relative} is not workflow_dispatch-only")
            if is_canary and "uses:" in text:
                fail(f"{relative} may not execute repository or third-party actions")
            permission_headers = re.findall(
                r"^[ \t]*permissions:[ \t]*$", text, re.MULTILINE
            )
            permissions = re.search(
                r"^permissions:\n((?:  [^\n]*\n)*)", text, re.MULTILINE
            )
            if (
                permission_headers != ["permissions:"]
                or permissions is None
                or permissions.group(1) != "  contents: read\n"
            ):
                fail(f"{relative} lacks exact read-only permissions")
            if "secrets." in text or "github.event.inputs" in text or "inputs." in text:
                fail(f"{relative} consumes secret or caller-controlled input")
            conditions = re.findall(r"^\s+if:\s*(.+?)\s*$", text, re.MULTILINE)
            if conditions != [PROTECTED_CONDITION]:
                fail(f"{relative} lacks the exact repository and main-ref trust guard")
            if any(pattern.search(text) for pattern in PRIVATE_OUTPUT_PATTERNS):
                fail(f"{relative} explicitly emits private runner identity")
            if is_canary and (
                "if bash -euo pipefail <<'ASB_CANARY' >/dev/null 2>&1" not in text
                or "\n          ASB_CANARY\n          then\n" not in text
                or text.count("'ASB development-host canary passed'") != 1
                or text.count("'ASB development-host canary failed'") != 1
            ):
                fail(f"{relative} lacks the fixed privacy-safe output boundary")
            if is_trusted and (
                "persist-credentials: false" not in text
                or "ref: ${{ github.sha }}" not in text
            ):
                fail(f"{relative} does not bind checkout to the protected revision")
        elif "self-hosted" in text:
            fail(f"{relative} uses a persistent runner")
        if "continue-on-error:" in text and (
            not is_quality
            or text.count("continue-on-error:") != 1
            or text.count("continue-on-error: true") != 1
        ):
            fail(f"{relative} weakens a required check")
        if is_quality:
            signature_step = text.split(
                "      - name: Enforce repository and commit policy\n", 1
            )
            if len(signature_step) != 2:
                fail(
                    f"{relative} lacks the exact protected-main signature-policy binding"
                )
            actual_signature_step = (
                "      - name: Enforce repository and commit policy\n"
                + signature_step[1].split("      - name:", 1)[0]
            )
            if actual_signature_step != QUALITY_SIGNATURE_STEP:
                fail(
                    f"{relative} lacks the exact protected-main signature-policy binding"
                )
            required_optional = (
                "      - name: Publish optional quality evidence\n"
                "        id: optional_evidence_upload\n"
                "        continue-on-error: true\n"
                f"        uses: {OPTIONAL_ARTIFACT_ACTION} # v4.6.2\n"
            )
            if (
                required_optional not in text
                or text.count(OPTIONAL_ARTIFACT_ACTION) != 1
            ):
                fail(
                    f"{relative} does not narrowly isolate the optional artifact failure"
                )
            required_fragments = (
                "          retention-days: 1\n",
                "          compression-level: 0\n",
                "          if-no-files-found: error\n",
                "        if: ${{ !cancelled() }}\n",
                "            --role optional \\\n",
                '            --outcome "$ARTIFACT_OUTCOME" \\\n',
            )
            if any(fragment not in text for fragment in required_fragments):
                fail(f"{relative} lacks bounded optional artifact classification")
            if text.index("- name: Verify clean tree") > text.index(
                "- name: Prepare bounded optional quality evidence"
            ):
                fail(f"{relative} publishes evidence before required checks finish")
        runners = re.findall(r"^\s*runs-on:\s*(.+?)\s*$", text, re.MULTILINE)
        if is_protected and runners != [f"[{CANARY_LABEL}]"]:
            fail(f"{relative} lacks the exact protected canary label")
        for raw_runner in runners:
            runner = raw_runner.split(" #", 1)[0].strip(" '\"")
            if runner == "${{ matrix.runner }}":
                if "runner: [ubuntu-24.04]" not in text:
                    fail(f"{relative} has an unbounded runner matrix")
            elif (
                not (is_protected and runner == f"[{CANARY_LABEL}]")
                and runner not in DISPOSABLE_RUNNERS
            ):
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


def validate_sources(files: list[Path], *, root: Path = ROOT) -> None:
    for path in files:
        relative = path.relative_to(root)
        lines = path.read_text(encoding="utf-8").splitlines()
        if path.suffix == ".rs":
            expected = [f"// {HUAWEI_COPYRIGHT}", f"// {SPDX_MIT}"]
            offset = 0
        elif path.suffix in {".py", ".sh"} or relative in EXTENSIONLESS_SOURCES:
            expected = [f"# {HUAWEI_COPYRIGHT}", f"# {SPDX_MIT}"]
            offset = int(bool(lines and lines[0].startswith("#!")))
        elif path.suffix == ".tla":
            declaration = TLA_MODULE.fullmatch(lines[0]) if lines else None
            if declaration is None or declaration.group("name") != path.stem:
                fail(
                    f"{relative} must begin with a TLA+ module declaration "
                    "matching its filename"
                )
            expected = [f"\\* {HUAWEI_COPYRIGHT}", f"\\* {SPDX_MIT}"]
            offset = 1
        elif path.suffix == ".als":
            declaration = ALLOY_MODULE.fullmatch(lines[0]) if lines else None
            if declaration is None or declaration.group("name").rsplit("/", 1)[-1] != path.stem:
                fail(
                    f"{relative} must begin with an Alloy module declaration "
                    "matching its filename"
                )
            expected = [f"// {HUAWEI_COPYRIGHT}", f"// {SPDX_MIT}"]
            offset = 1
        else:
            continue
        if lines[offset : offset + 2] != expected:
            fail(
                f"{relative} lacks the exact adjacent Huawei 2026 and SPDX MIT "
                f"source header at lines {offset + 1}-{offset + 2}"
            )
        pair_count = sum(
            lines[index : index + 2] == expected
            for index in range(max(len(lines) - 1, 0))
        )
        if pair_count != 1:
            fail(
                f"{relative} must contain exactly one canonical adjacent "
                "Huawei/MIT header pair"
            )


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


def commit_parents(root: Path, revision: str) -> list[str]:
    output = subprocess.check_output(
        ["git", "-C", str(root), "show", "-s", "--format=%P", revision],
        text=True,
    )
    return output.split()


def verify_ssh(root: Path, allowed: Path, revision: str) -> None:
    result = subprocess.run(
        [
            "git",
            "-C",
            str(root),
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


def verify_github_web_flow(
    root: Path,
    key: Path,
    expected_key_sha256: str,
    expected_fingerprint: str,
    revision: str,
) -> None:
    key_bytes = key.read_bytes()
    if hashlib.sha256(key_bytes).hexdigest() != expected_key_sha256:
        fail("pinned GitHub Web Flow key digest differs")
    with tempfile.TemporaryDirectory(prefix="asb-web-flow-gpg-") as raw_home:
        home = Path(raw_home)
        home.chmod(0o700)
        environment = os.environ.copy()
        environment["GNUPGHOME"] = str(home)
        imported = subprocess.run(
            ["gpg", "--batch", "--quiet", "--import", str(key)],
            env=environment,
            text=True,
            capture_output=True,
            check=False,
        )
        if imported.returncode:
            fail("pinned GitHub Web Flow key cannot be imported")
        verified = subprocess.run(
            [
                "git",
                "-C",
                str(root),
                "-c",
                "gpg.format=openpgp",
                "-c",
                "gpg.program=gpg",
                "-c",
                "gpg.openpgp.program=gpg",
                "verify-commit",
                "--raw",
                revision,
            ],
            env=environment,
            text=True,
            capture_output=True,
            check=False,
        )
    status = verified.stdout + verified.stderr
    fingerprints = {
        fields[2]
        for line in status.splitlines()
        if line.startswith("[GNUPG:] VALIDSIG ") and len(fields := line.split()) >= 3
    }
    if verified.returncode or fingerprints != {expected_fingerprint}:
        fail(f"{revision} lacks the pinned GitHub Web Flow signature")


def exact_commit(root: Path, revision: str) -> str:
    if not FULL_SHA.fullmatch(revision):
        fail("protected-main commit identities must be full lowercase SHA-1 values")
    resolved = subprocess.check_output(
        ["git", "-C", str(root), "rev-parse", "--verify", f"{revision}^{{commit}}"],
        text=True,
    ).strip()
    if resolved != revision:
        fail("protected-main commit identity did not resolve exactly")
    return resolved


def validate_commits(
    base: str | None,
    head: str,
    *,
    mode: str = "ssh-only",
    event: str | None = None,
    ref: str | None = None,
    root: Path = ROOT,
    allowed: Path | None = None,
    web_flow_key: Path | None = None,
    web_flow_key_sha256: str = GITHUB_WEB_FLOW_KEY_SHA256,
    web_flow_fingerprint: str = GITHUB_WEB_FLOW_FINGERPRINT,
) -> None:
    allowed = allowed or root / "config/allowed_signers"
    if mode not in {"ssh-only", "protected-main"}:
        fail("unknown commit signature policy mode")
    if mode == "protected-main":
        if event != PROTECTED_EVENT or ref != PROTECTED_REF:
            fail("protected-main mode requires the canonical push event and main ref")
        if base is None:
            fail("protected-main mode requires an immutable range base")
        exact_commit(root, base)
        exact_commit(root, head)
    revisions = commit_range(root, base, head)
    if not revisions:
        fail("commit policy received an empty revision range")
    validate_dco(root, revisions)
    if mode == "ssh-only":
        for revision in revisions:
            verify_ssh(root, allowed, revision)
        return
    if revisions[-1] != head:
        fail("protected-main range head is not the final introduced commit")
    merge_revisions = [
        revision for revision in revisions if len(commit_parents(root, revision)) != 1
    ]
    if merge_revisions != [head]:
        fail("protected-main range must contain one final two-parent merge")
    parents = commit_parents(root, head)
    if len(parents) != 2 or parents[0] != base:
        fail("protected-main merge topology or first parent differs")
    topic_revisions = commit_range(root, base, parents[1])
    if revisions != topic_revisions + [head]:
        fail("protected-main range contains commits outside the merged topic")
    committer = subprocess.check_output(
        ["git", "-C", str(root), "show", "-s", "--format=%cn <%ce>", head],
        text=True,
    ).strip()
    if committer != GITHUB_COMMITTER:
        fail("protected-main merge committer is not exact GitHub Web Flow")
    for revision in topic_revisions:
        verify_ssh(root, allowed, revision)
    verify_github_web_flow(
        root,
        web_flow_key or root / GITHUB_WEB_FLOW_KEY.relative_to(ROOT),
        web_flow_key_sha256,
        web_flow_fingerprint,
        head,
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base")
    parser.add_argument("--head", default="HEAD")
    parser.add_argument(
        "--mode", choices=("ssh-only", "protected-main"), default="ssh-only"
    )
    parser.add_argument("--event")
    parser.add_argument("--ref")
    parser.add_argument("--skip-commits", action="store_true")
    parser.add_argument("--source-headers-only", action="store_true")
    args = parser.parse_args()
    try:
        files = tracked_files()
        validate_sources(files)
        if args.source_headers_only:
            print("repository policy: source headers passed")
            return 0
        manifest = validate_manifest()
        validate_workflows(manifest)
        validate_markdown(files)
        if not args.skip_commits:
            validate_commits(
                args.base,
                args.head,
                mode=args.mode,
                event=args.event,
                ref=args.ref,
            )
    except (
        OSError,
        subprocess.CalledProcessError,
        TypeError,
        ValueError,
        KeyError,
    ) as error:
        print(f"repository policy: {error}", file=sys.stderr)
        return 1
    print("repository policy: all checks passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
