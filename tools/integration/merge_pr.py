#!/usr/bin/env python3
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Construct and optionally publish an exact, signed pull-request merge."""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

FULL_OID = re.compile(r"^[0-9a-f]{40}$")
REMOTE_NAME = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,100}$")
TARGET_REF = re.compile(r"^refs/heads/[A-Za-z0-9][A-Za-z0-9._/-]{0,200}$")
PR_REF = re.compile(r"^refs/pull/[1-9][0-9]{0,9}/head$")


def run(root: Path, *args: str, input_text: str | None = None) -> str:
    result = subprocess.run(
        list(args),
        cwd=root,
        input=input_text,
        text=True,
        capture_output=True,
        check=False,
    )
    if result.returncode:
        detail = result.stderr.strip() or result.stdout.strip() or "command failed"
        raise ValueError(f"{args[0]} failed: {detail}")
    return result.stdout.strip()


def require_oid(name: str, value: str) -> str:
    if not FULL_OID.fullmatch(value):
        raise ValueError(f"{name} must be a full lowercase Git object ID")
    return value


def remote_oid(root: Path, remote: str, reference: str) -> str:
    fields = run(root, "git", "ls-remote", "--refs", remote, reference).split()
    if len(fields) != 2 or fields[1] != reference:
        raise ValueError(f"remote reference {reference} is missing or ambiguous")
    return require_oid("remote object ID", fields[0])


def verify_commit(root: Path, revision: str) -> None:
    allowed = root / "config/allowed_signers"
    run(
        root,
        "git",
        "-c",
        "gpg.format=ssh",
        "-c",
        f"gpg.ssh.allowedSignersFile={allowed}",
        "verify-commit",
        revision,
    )


def verify_range(root: Path, base: str, head: str) -> None:
    revisions = run(
        root, "git", "rev-list", "--reverse", f"{base}..{head}"
    ).splitlines()
    if not revisions:
        raise ValueError("approved pull-request range is empty")
    run(
        root,
        "python3",
        str(root / "tools/quality/check_dco.py"),
        "--root",
        str(root),
        "--base",
        base,
        "--head",
        head,
    )
    for revision in revisions:
        verify_commit(root, revision)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=Path.cwd())
    parser.add_argument("--remote", default="origin")
    parser.add_argument("--target-ref", default="refs/heads/main")
    parser.add_argument("--pr-ref", required=True)
    parser.add_argument("--expected-base", required=True)
    parser.add_argument("--expected-head", required=True)
    parser.add_argument("--expected-tree", required=True)
    parser.add_argument("--subject", required=True)
    parser.add_argument("--push", action="store_true")
    args = parser.parse_args()

    try:
        root = args.root.resolve(strict=True)
        base = require_oid("expected base", args.expected_base)
        head = require_oid("expected head", args.expected_head)
        tree = require_oid("expected tree", args.expected_tree)
        if not REMOTE_NAME.fullmatch(args.remote):
            raise ValueError("remote must be a configured remote name")
        if not PR_REF.fullmatch(args.pr_ref):
            raise ValueError("pr-ref must be refs/pull/<number>/head")
        if not TARGET_REF.fullmatch(args.target_ref):
            raise ValueError("target-ref is not a bounded branch reference")
        if not args.subject or "\n" in args.subject or len(args.subject.encode()) > 160:
            raise ValueError("subject must be one bounded line")
        if run(root, "git", "status", "--porcelain"):
            raise ValueError("integration worktree is not clean")
        if run(root, "git", "rev-parse", "HEAD") != base:
            raise ValueError("integration worktree is not at the approved base")
        if remote_oid(root, args.remote, args.target_ref) != base:
            raise ValueError("remote target no longer equals the approved base")
        if remote_oid(root, args.remote, args.pr_ref) != head:
            raise ValueError(
                "remote pull-request head no longer equals the approved head"
            )

        run(root, "git", "fetch", "--no-tags", args.remote, base, head)
        if run(root, "git", "merge-base", base, head) != base:
            raise ValueError("approved head is not based on the exact approved base")
        if run(root, "git", "rev-parse", f"{head}^{{tree}}") != tree:
            raise ValueError("approved head tree does not match the expected tree")
        verify_range(root, base, head)

        name = run(root, "git", "config", "user.name")
        email = run(root, "git", "config", "user.email")
        if not name or not email or "\n" in name or "\n" in email:
            raise ValueError("Git author identity is missing or malformed")
        message = f"{args.subject}\n\nSigned-off-by: {name} <{email}>\n"
        merge = require_oid(
            "merge commit",
            run(
                root,
                "git",
                "commit-tree",
                "-S",
                "-p",
                base,
                "-p",
                head,
                tree,
                input_text=message,
            ),
        )
        parents = run(root, "git", "show", "-s", "--format=%P", merge).split()
        if parents != [base, head]:
            raise ValueError("constructed merge has unexpected parents")
        if run(root, "git", "rev-parse", f"{merge}^{{tree}}") != tree:
            raise ValueError("constructed merge changed the approved tree")
        verify_commit(root, merge)
        run(
            root,
            "python3",
            str(root / "tools/quality/check_dco.py"),
            "--root",
            str(root),
            "--base",
            head,
            "--head",
            merge,
        )

        print(f"BASE={base}")
        print(f"HEAD={head}")
        print(f"TREE={tree}")
        print(f"MERGE={merge}")
        if args.push:
            if remote_oid(root, args.remote, args.target_ref) != base:
                raise ValueError("remote target changed before publication")
            run(
                root,
                "git",
                "push",
                args.remote,
                f"{merge}:{args.target_ref}",
                f"--force-with-lease={args.target_ref}:{base}",
            )
            if remote_oid(root, args.remote, args.target_ref) != merge:
                raise ValueError("remote target does not contain the constructed merge")
            print("PUBLISHED=1")
        else:
            print("PUBLISHED=0")
        return 0
    except (OSError, ValueError) as error:
        print(f"merge integrity: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
