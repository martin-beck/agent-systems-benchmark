#!/usr/bin/env python3
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Prove required quality gates reject controlled defects."""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def workflow_script(path: Path, step_name: str) -> str:
    text = path.read_text(encoding="utf-8")
    step = text.split(f"      - name: {step_name}\n", 1)[1]
    block = step.split("        run: |\n", 1)[1]
    lines: list[str] = []
    for line in block.splitlines():
        if not line.startswith("          "):
            break
        lines.append(line[10:])
    if not lines:
        raise RuntimeError(f"{path} has no script for {step_name}")
    return "\n".join(lines) + "\n"


def must_fail(name: str, command: list[str], cwd: Path = ROOT) -> None:
    result = subprocess.run(command, cwd=cwd, capture_output=True, text=True, check=False)
    if result.returncode == 0:
        raise RuntimeError(f"{name} accepted its deliberately bad fixture")
    print(f"negative fixture passed: {name}")


def git(repository: Path, *args: str) -> None:
    subprocess.run(["git", *args], cwd=repository, check=True, capture_output=True)


def init_git(repository: Path) -> None:
    git(repository, "init", "-q")
    git(repository, "config", "user.name", "Fixture")
    git(repository, "config", "user.email", "fixture@example.invalid")
    git(repository, "config", "commit.gpgsign", "false")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--bin-dir", type=Path, required=True)
    args = parser.parse_args()
    actionlint = str(args.bin_dir / "actionlint")
    zizmor = str(args.bin_dir / "zizmor")
    gitleaks = str(args.bin_dir / "gitleaks")

    with tempfile.TemporaryDirectory(prefix="asb-quality-negative-") as raw:
        temp = Path(raw)
        canary_root = temp / "public-canary-root"
        canary_root.mkdir(mode=0o700)
        canary = workflow_script(
            ROOT / ".github/workflows/development-host-canary.yml",
            "Verify qualified runtime and disposable workspace",
        )
        canary_env = {
            "PATH": os.environ["PATH"],
            "GITHUB_EVENT_NAME": "workflow_dispatch",
            "GITHUB_REPOSITORY": "martin-beck/agent-systems-benchmark",
            "GITHUB_REF": "refs/heads/main",
            "GITHUB_RUN_ID": "1234",
            "GITHUB_RUN_ATTEMPT": "1",
            "RUNNER_OS": "Linux",
            "RUNNER_ARCH": "X64",
            "RUNNER_TEMP": str(canary_root),
        }
        canary_result = subprocess.run(
            ["bash"], input=canary, env=canary_env, capture_output=True, text=True, check=False
        )
        if (
            canary_result.returncode != 0
            or canary_result.stdout != "ASB development-host canary passed\n"
            or canary_result.stderr
            or list(canary_root.iterdir())
        ):
            raise RuntimeError("canary success output or cleanup is not fixed and private")
        private_path = temp / "private-host-path"
        failed_env = canary_env | {"RUNNER_TEMP": str(private_path)}
        failed_result = subprocess.run(
            ["bash"], input=canary, env=failed_env, capture_output=True, text=True, check=False
        )
        if (
            failed_result.returncode == 0
            or failed_result.stdout
            or failed_result.stderr != "ASB development-host canary failed\n"
            or str(private_path) in failed_result.stderr
        ):
            raise RuntimeError("canary failure output disclosed its private path")
        print("positive fixture passed: privacy-safe canary output and cleanup")

        bad_workflow = temp / "bad.yml"
        bad_workflow.write_text(
            "name: bad\n"
            "\"on\": [push]\n"
            "jobs:\n"
            "  test:\n"
            "    runs-on: ubuntu-latest\n"
            "    steps:\n"
            "      - uses: actions/checkout@main\n"
            "      - run: ${{ github.event.issue.title }}\n",
            encoding="utf-8",
        )
        must_fail("actionlint", [actionlint, str(bad_workflow)])
        must_fail("zizmor", [zizmor, "--pedantic", str(bad_workflow)])

        leaked = temp / "leaked"
        leaked.mkdir()
        init_git(leaked)
        (leaked / "secret.txt").write_text(
            "service_api_key = "
            "\"7f4c8b2e91a6d305c7e8f1420b9a6d3e"
            + "5c7f8a1b2d4e6f8091a3c5e7b9d2f4a6\"\n",
            encoding="utf-8",
        )
        git(leaked, "add", "secret.txt")
        git(leaked, "commit", "-qm", "bad fixture")
        must_fail("gitleaks", [gitleaks, "git", "--no-banner", str(leaked)])

        missing_dco = temp / "missing-dco"
        shutil.copytree(ROOT, missing_dco, ignore=shutil.ignore_patterns(".git", "target"))
        init_git(missing_dco)
        git(missing_dco, "add", ".")
        git(missing_dco, "commit", "-qm", "missing DCO fixture")
        must_fail(
            "DCO policy",
            ["python3", str(missing_dco / "tools/quality/repository_policy.py"), "--head", "HEAD"],
            cwd=missing_dco,
        )

        misplaced_dco = temp / "misplaced-dco"
        misplaced_dco.mkdir()
        init_git(misplaced_dco)
        (misplaced_dco / "change").write_text("change\n", encoding="utf-8")
        git(misplaced_dco, "add", "change")
        git(
            misplaced_dco,
            "commit",
            "-qm",
            "misplaced DCO fixture\n\nSigned-off-by: Fixture <fixture@example.invalid>\n\nnon-trailer prose",
        )
        must_fail(
            "misplaced DCO trailer",
            ["python3", str(ROOT / "tools/quality/check_dco.py"), "--root", str(misplaced_dco), "--head", "HEAD"],
        )

        unsigned = temp / "unsigned"
        shutil.copytree(ROOT, unsigned, ignore=shutil.ignore_patterns(".git", "target"))
        init_git(unsigned)
        git(unsigned, "add", ".")
        git(unsigned, "commit", "-qsm", "unsigned fixture")
        must_fail(
            "SSH signature policy",
            ["python3", str(unsigned / "tools/quality/repository_policy.py"), "--head", "HEAD"],
            cwd=unsigned,
        )

        merge_range = temp / "synthetic-merge-range"
        merge_range.mkdir()
        init_git(merge_range)
        (merge_range / "base").write_text("base\n", encoding="utf-8")
        git(merge_range, "add", "base")
        git(merge_range, "commit", "-qsm", "base fixture")
        base = subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=merge_range, text=True
        ).strip()
        git(merge_range, "checkout", "-qb", "contributor")
        (merge_range / "change").write_text("change\n", encoding="utf-8")
        git(merge_range, "add", "change")
        git(merge_range, "commit", "-qsm", "contributor fixture")
        pr_head = subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=merge_range, text=True
        ).strip()
        git(merge_range, "checkout", "-q", "master")
        git(merge_range, "merge", "--no-ff", "contributor", "-qm", "synthetic merge")
        merge_head = subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=merge_range, text=True
        ).strip()
        checker = str(ROOT / "tools/quality/check_dco.py")
        must_fail(
            "synthetic merge exclusion",
            ["python3", checker, "--root", str(merge_range), "--base", base, "--head", merge_head],
        )
        subprocess.run(
            ["python3", checker, "--root", str(merge_range), "--base", base, "--head", pr_head],
            check=True,
        )
        print("positive fixture passed: pull request base..head DCO range")

        broken = temp / "mutable-action"
        shutil.copytree(ROOT, broken, ignore=shutil.ignore_patterns(".git", "target"))
        workflow = broken / ".github/workflows/verify.yml"
        workflow.write_text(
            workflow.read_text(encoding="utf-8").replace(
                "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1",
                "actions/checkout@main",
            ),
            encoding="utf-8",
        )
        init_git(broken)
        git(broken, "add", ".")
        must_fail(
            "immutable action pins",
            ["python3", str(broken / "tools/quality/repository_policy.py"), "--skip-commits"],
            cwd=broken,
        )

        persistent = temp / "persistent-runner"
        shutil.copytree(ROOT, persistent, ignore=shutil.ignore_patterns(".git", "target"))
        workflow = persistent / ".github/workflows/verify.yml"
        workflow.write_text(
            workflow.read_text(encoding="utf-8").replace(
                "runner: [ubuntu-24.04, ubuntu-24.04-arm]", "runner: [self-hosted]"
            ),
            encoding="utf-8",
        )
        init_git(persistent)
        git(persistent, "add", ".")
        must_fail(
            "disposable public PR runners",
            ["python3", str(persistent / "tools/quality/repository_policy.py"), "--skip-commits"],
            cwd=persistent,
        )

        automatic_canary = temp / "automatic-canary"
        shutil.copytree(ROOT, automatic_canary, ignore=shutil.ignore_patterns(".git", "target"))
        canary = automatic_canary / ".github/workflows/development-host-canary.yml"
        canary.write_text(
            canary.read_text(encoding="utf-8").replace("workflow_dispatch:", "pull_request:"),
            encoding="utf-8",
        )
        init_git(automatic_canary)
        git(automatic_canary, "add", ".")
        must_fail(
            "automatic persistent canary",
            [
                "python3",
                str(automatic_canary / "tools/quality/repository_policy.py"),
                "--skip-commits",
            ],
            cwd=automatic_canary,
        )

        privileged_canary = temp / "privileged-canary"
        shutil.copytree(ROOT, privileged_canary, ignore=shutil.ignore_patterns(".git", "target"))
        canary = privileged_canary / ".github/workflows/development-host-canary.yml"
        canary.write_text(
            canary.read_text(encoding="utf-8").replace(
                "  contents: read\n", "  contents: read\n  issues: write\n"
            ),
            encoding="utf-8",
        )
        init_git(privileged_canary)
        git(privileged_canary, "add", ".")
        must_fail(
            "read-only persistent canary",
            [
                "python3",
                str(privileged_canary / "tools/quality/repository_policy.py"),
                "--skip-commits",
            ],
            cwd=privileged_canary,
        )

        executable_canary = temp / "executable-canary"
        shutil.copytree(ROOT, executable_canary, ignore=shutil.ignore_patterns(".git", "target"))
        canary = executable_canary / ".github/workflows/development-host-canary.yml"
        canary.write_text(
            canary.read_text(encoding="utf-8").replace(
                "steps:\n", "steps:\n      - uses: actions/checkout@main\n"
            ),
            encoding="utf-8",
        )
        init_git(executable_canary)
        git(executable_canary, "add", ".")
        must_fail(
            "action-free persistent canary",
            [
                "python3",
                str(executable_canary / "tools/quality/repository_policy.py"),
                "--skip-commits",
            ],
            cwd=executable_canary,
        )

        alternate_ref_canary = temp / "alternate-ref-canary"
        shutil.copytree(
            ROOT, alternate_ref_canary, ignore=shutil.ignore_patterns(".git", "target")
        )
        canary = alternate_ref_canary / ".github/workflows/development-host-canary.yml"
        canary.write_text(
            canary.read_text(encoding="utf-8").replace(
                "github.ref == 'refs/heads/main'", "github.ref == 'refs/heads/release'"
            ),
            encoding="utf-8",
        )
        init_git(alternate_ref_canary)
        git(alternate_ref_canary, "add", ".")
        must_fail(
            "alternate-ref persistent canary",
            ["python3", str(alternate_ref_canary / "tools/quality/repository_policy.py"), "--skip-commits"],
            cwd=alternate_ref_canary,
        )

        fork_canary = temp / "fork-canary"
        shutil.copytree(ROOT, fork_canary, ignore=shutil.ignore_patterns(".git", "target"))
        canary = fork_canary / ".github/workflows/development-host-canary.yml"
        canary.write_text(
            canary.read_text(encoding="utf-8").replace(
                "github.repository == 'martin-beck/agent-systems-benchmark'",
                "github.repository != ''",
            ),
            encoding="utf-8",
        )
        init_git(fork_canary)
        git(fork_canary, "add", ".")
        must_fail(
            "fork persistent canary",
            ["python3", str(fork_canary / "tools/quality/repository_policy.py"), "--skip-commits"],
            cwd=fork_canary,
        )

        identity_canary = temp / "identity-output-canary"
        shutil.copytree(
            ROOT, identity_canary, ignore=shutil.ignore_patterns(".git", "target")
        )
        canary = identity_canary / ".github/workflows/development-host-canary.yml"
        canary.write_text(
            canary.read_text(encoding="utf-8").replace(
                "          set -euo pipefail\n",
                "          set -euo pipefail\n          printf '%s\\n' \"${RUNNER_NAME}\"\n",
                1,
            ),
            encoding="utf-8",
        )
        init_git(identity_canary)
        git(identity_canary, "add", ".")
        must_fail(
            "explicit persistent runner identity output",
            ["python3", str(identity_canary / "tools/quality/repository_policy.py"), "--skip-commits"],
            cwd=identity_canary,
        )

        unsanitized_canary = temp / "unsanitized-canary"
        shutil.copytree(
            ROOT, unsanitized_canary, ignore=shutil.ignore_patterns(".git", "target")
        )
        canary = unsanitized_canary / ".github/workflows/development-host-canary.yml"
        canary.write_text(
            canary.read_text(encoding="utf-8").replace(
                "<<'ASB_CANARY' >/dev/null 2>&1", "<<'ASB_CANARY'"
            ),
            encoding="utf-8",
        )
        init_git(unsanitized_canary)
        git(unsanitized_canary, "add", ".")
        must_fail(
            "unsanitized persistent canary failures",
            ["python3", str(unsanitized_canary / "tools/quality/repository_policy.py"), "--skip-commits"],
            cwd=unsanitized_canary,
        )

        privileged_trusted = temp / "privileged-trusted"
        shutil.copytree(
            ROOT, privileged_trusted, ignore=shutil.ignore_patterns(".git", "target")
        )
        trusted = privileged_trusted / ".github/workflows/development-host-trusted.yml"
        trusted.write_text(
            trusted.read_text(encoding="utf-8").replace(
                "  contents: read\n", "  contents: read\n  actions: write\n"
            ),
            encoding="utf-8",
        )
        init_git(privileged_trusted)
        git(privileged_trusted, "add", ".")
        must_fail(
            "widened trusted-runner permissions",
            ["python3", str(privileged_trusted / "tools/quality/repository_policy.py"), "--skip-commits"],
            cwd=privileged_trusted,
        )

        job_permissions_canary = temp / "job-permissions-canary"
        shutil.copytree(
            ROOT, job_permissions_canary, ignore=shutil.ignore_patterns(".git", "target")
        )
        canary = job_permissions_canary / ".github/workflows/development-host-canary.yml"
        canary.write_text(
            canary.read_text(encoding="utf-8").replace(
                "    name: Verify isolated development host\n",
                "    name: Verify isolated development host\n    permissions:\n      actions: write\n",
            ),
            encoding="utf-8",
        )
        init_git(job_permissions_canary)
        git(job_permissions_canary, "add", ".")
        must_fail(
            "job-level persistent-runner permissions",
            ["python3", str(job_permissions_canary / "tools/quality/repository_policy.py"), "--skip-commits"],
            cwd=job_permissions_canary,
        )

        pull_request_trusted = temp / "pull-request-trusted-runner"
        shutil.copytree(
            ROOT, pull_request_trusted, ignore=shutil.ignore_patterns(".git", "target")
        )
        trusted = pull_request_trusted / ".github/workflows/development-host-trusted.yml"
        trusted.write_text(
            trusted.read_text(encoding="utf-8").replace(
                "workflow_dispatch:", "pull_request:"
            ),
            encoding="utf-8",
        )
        init_git(pull_request_trusted)
        git(pull_request_trusted, "add", ".")
        must_fail(
            "public pull request persistent routing",
            [
                "python3",
                str(pull_request_trusted / "tools/quality/repository_policy.py"),
                "--skip-commits",
            ],
            cwd=pull_request_trusted,
        )

        unguarded_trusted = temp / "unguarded-trusted-runner"
        shutil.copytree(
            ROOT, unguarded_trusted, ignore=shutil.ignore_patterns(".git", "target")
        )
        trusted = unguarded_trusted / ".github/workflows/development-host-trusted.yml"
        trusted.write_text(
            trusted.read_text(encoding="utf-8").replace(
                "github.ref == 'refs/heads/main'", "github.ref != ''"
            ),
            encoding="utf-8",
        )
        init_git(unguarded_trusted)
        git(unguarded_trusted, "add", ".")
        must_fail(
            "unguarded persistent routing",
            [
                "python3",
                str(unguarded_trusted / "tools/quality/repository_policy.py"),
                "--skip-commits",
            ],
            cwd=unguarded_trusted,
        )

        partial_label = temp / "partial-label-trusted-runner"
        shutil.copytree(
            ROOT, partial_label, ignore=shutil.ignore_patterns(".git", "target")
        )
        trusted = partial_label / ".github/workflows/development-host-trusted.yml"
        trusted.write_text(
            trusted.read_text(encoding="utf-8").replace(
                "asb-development-v1-x86_64-ubuntu2404", "asb-development-v1"
            ),
            encoding="utf-8",
        )
        init_git(partial_label)
        git(partial_label, "add", ".")
        must_fail(
            "partial persistent runner label",
            [
                "python3",
                str(partial_label / "tools/quality/repository_policy.py"),
                "--skip-commits",
            ],
            cwd=partial_label,
        )

        required_optional = temp / "required-optional-artifact"
        shutil.copytree(
            ROOT, required_optional, ignore=shutil.ignore_patterns(".git", "target")
        )
        workflow = required_optional / ".github/workflows/quality.yml"
        workflow.write_text(
            workflow.read_text(encoding="utf-8").replace(
                "        continue-on-error: true\n", "        continue-on-error: false\n"
            ),
            encoding="utf-8",
        )
        init_git(required_optional)
        git(required_optional, "add", ".")
        must_fail(
            "optional artifact classification",
            [
                "python3",
                str(required_optional / "tools/quality/repository_policy.py"),
                "--skip-commits",
            ],
            cwd=required_optional,
        )

        missing_optional = temp / "missing-optional-artifact-boundary"
        shutil.copytree(
            ROOT, missing_optional, ignore=shutil.ignore_patterns(".git", "target")
        )
        workflow = missing_optional / ".github/workflows/quality.yml"
        workflow.write_text(
            workflow.read_text(encoding="utf-8").replace(
                "        continue-on-error: true\n", ""
            ),
            encoding="utf-8",
        )
        init_git(missing_optional)
        git(missing_optional, "add", ".")
        must_fail(
            "missing optional artifact boundary",
            [
                "python3",
                str(missing_optional / "tools/quality/repository_policy.py"),
                "--skip-commits",
            ],
            cwd=missing_optional,
        )

        weakened_required = temp / "weakened-required-check"
        shutil.copytree(
            ROOT, weakened_required, ignore=shutil.ignore_patterns(".git", "target")
        )
        workflow = weakened_required / ".github/workflows/quality.yml"
        workflow.write_text(
            workflow.read_text(encoding="utf-8").replace(
                "      - name: Enforce coverage floors\n",
                "      - name: Enforce coverage floors\n        continue-on-error: true\n",
            ),
            encoding="utf-8",
        )
        init_git(weakened_required)
        git(weakened_required, "add", ".")
        must_fail(
            "required check remains fail closed",
            [
                "python3",
                str(weakened_required / "tools/quality/repository_policy.py"),
                "--skip-commits",
            ],
            cwd=weakened_required,
        )

        broken_docs = temp / "broken-docs"
        shutil.copytree(ROOT, broken_docs, ignore=shutil.ignore_patterns(".git", "target"))
        (broken_docs / "BROKEN.md").write_text("[missing](does-not-exist.md)\n", encoding="utf-8")
        init_git(broken_docs)
        git(broken_docs, "add", ".")
        must_fail(
            "documentation links",
            ["python3", str(broken_docs / "tools/quality/repository_policy.py"), "--skip-commits"],
            cwd=broken_docs,
        )

        lockfile = temp / "Cargo.lock"
        lockfile.write_text(
            'version = 3\n\n[[package]]\nname = "openssl"\nversion = "0.9.0"\n'
            'source = "registry+https://github.com/rust-lang/crates.io-index"\n',
            encoding="utf-8",
        )
        must_fail(
            "cargo-audit",
            ["cargo", "audit", "--deny", "warnings", "--file", str(lockfile)],
        )

    must_fail(
        "cargo-deny",
        [
            "cargo",
            "deny",
            "--manifest-path",
            str(ROOT / "Cargo.toml"),
            "--config",
            str(ROOT / "tests/quality/deny-bad.toml"),
            "check",
            "bans",
        ],
    )
    must_fail(
        "coverage floor",
        ["cargo", "llvm-cov", "--workspace", "--fail-under-lines", "101"],
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
