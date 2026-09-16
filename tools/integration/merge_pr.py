#!/usr/bin/env python3
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Construct and optionally publish an exact, signed pull-request merge."""

from __future__ import annotations

import argparse
import os
import re
import signal
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path

FULL_OID = re.compile(r"^[0-9a-f]{40}$")
REMOTE_NAME = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,100}$")
TARGET_REF = re.compile(r"^refs/heads/[A-Za-z0-9][A-Za-z0-9._/-]{0,200}$")
PR_REF = re.compile(r"^refs/pull/[1-9][0-9]{0,9}/head$")
MAX_OUTPUT_BYTES = 64 * 1024
COMMAND_TIMEOUT_SECONDS = 60


@dataclass(frozen=True)
class CommandResult:
    returncode: int
    stdout: str


def bounded_command(
    root: Path, *args: str, input_text: str | None = None
) -> CommandResult:
    with (
        tempfile.TemporaryFile() as stdin,
        tempfile.TemporaryFile() as stdout,
        tempfile.TemporaryFile() as stderr,
    ):
        if input_text is not None:
            stdin.write(input_text.encode())
        stdin.seek(0)
        environment = os.environ.copy()
        environment["LC_ALL"] = "C"
        process = subprocess.Popen(
            list(args),
            cwd=root,
            stdin=stdin,
            stdout=stdout,
            stderr=stderr,
            start_new_session=True,
            env=environment,
        )
        deadline = time.monotonic() + COMMAND_TIMEOUT_SECONDS
        while process.poll() is None:
            if time.monotonic() >= deadline:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
                raise ValueError("bounded subprocess timed out")
            if (
                os.fstat(stdout.fileno()).st_size > MAX_OUTPUT_BYTES
                or os.fstat(stderr.fileno()).st_size > MAX_OUTPUT_BYTES
            ):
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
                raise ValueError("bounded subprocess output exceeded limit")
            time.sleep(0.01)
        if (
            os.fstat(stdout.fileno()).st_size > MAX_OUTPUT_BYTES
            or os.fstat(stderr.fileno()).st_size > MAX_OUTPUT_BYTES
        ):
            raise ValueError("bounded subprocess output exceeded limit")
        stdout.seek(0)
        try:
            output = stdout.read().decode("utf-8")
        except UnicodeDecodeError as error:
            raise ValueError("bounded subprocess emitted invalid UTF-8") from error
        return CommandResult(process.returncode, output)


def run(root: Path, *args: str, input_text: str | None = None) -> str:
    result = bounded_command(root, *args, input_text=input_text)
    if result.returncode:
        raise ValueError("bounded subprocess failed")
    return result.stdout.strip()


def require_oid(name: str, value: str) -> str:
    if not FULL_OID.fullmatch(value):
        raise ValueError(f"{name} must be a full lowercase Git object ID")
    return value


def remote_oids(root: Path, remote: str, *references: str) -> dict[str, str]:
    lines = run(root, "git", "ls-remote", "--refs", remote, *references).splitlines()
    found: dict[str, str] = {}
    for line in lines:
        fields = line.split()
        if len(fields) != 2 or fields[1] not in references or fields[1] in found:
            raise ValueError("remote reference result is malformed or ambiguous")
        found[fields[1]] = require_oid("remote object ID", fields[0])
    if set(found) != set(references):
        raise ValueError("required remote reference is missing")
    return found


def commit_identity(root: Path, revision: str) -> tuple[str, str]:
    raw = run(
        root, "git", "show", "-s", "--format=%an%x00%ae%x00%cn%x00%ce%x00%B", revision
    )
    fields = raw.split("\0", 4)
    if len(fields) != 5:
        raise ValueError("commit identity metadata is malformed")
    author_name, author_email, committer_name, committer_email, body = fields
    if (
        not author_name
        or not author_email
        or author_name != committer_name
        or author_email != committer_email
    ):
        raise ValueError("commit author and committer identities do not match")
    trailers = run(
        root, "git", "interpret-trailers", "--parse", input_text=body
    ).splitlines()
    expected = f"Signed-off-by: {author_name} <{author_email}>"
    if trailers.count(expected) != 1:
        raise ValueError("commit DCO identity is absent, duplicated, or mismatched")
    return author_name, author_email


def verify_commit(root: Path, revision: str) -> None:
    allowed = root / "config/allowed_signers"
    signature = run(
        root,
        "git",
        "-c",
        "gpg.format=ssh",
        "-c",
        f"gpg.ssh.allowedSignersFile={allowed}",
        "log",
        "-1",
        "--format=%G?%n%GS",
        revision,
    ).splitlines()
    _, email = commit_identity(root, revision)
    if signature != ["G", email]:
        raise ValueError(
            "commit signer principal does not match its certified identity"
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
        initial = remote_oids(root, args.remote, args.target_ref, args.pr_ref)
        if initial[args.target_ref] != base:
            raise ValueError("remote target no longer equals the approved base")
        if initial[args.pr_ref] != head:
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
            before = remote_oids(root, args.remote, args.target_ref, args.pr_ref)
            if before[args.target_ref] != base:
                raise ValueError("remote target changed before publication")
            if before[args.pr_ref] != head:
                raise ValueError("remote pull-request head changed before publication")
            push = bounded_command(
                root,
                "git",
                "push",
                args.remote,
                f"{merge}:{args.target_ref}",
                f"--force-with-lease={args.target_ref}:{base}",
            )
            after = remote_oids(root, args.remote, args.target_ref, args.pr_ref)
            after_target = after[args.target_ref]
            after_pr = after[args.pr_ref]
            if after_pr != head:
                if after_target == merge:
                    print("PUBLISHED=1")
                    print("PUBLICATION=accepted-with-pr-ref-drift")
                raise ValueError("remote pull-request head changed during publication")
            if after_target == merge:
                print("PUBLISHED=1")
                print(
                    "PUBLICATION=accepted-after-error"
                    if push.returncode
                    else "PUBLICATION=accepted"
                )
                return 0
            if after_target == base and push.returncode:
                raise ValueError("publication failed without a remote target change")
            if after_target == base:
                raise ValueError(
                    "publication reported success without a remote target change"
                )
            raise ValueError(
                "remote target changed to an unexpected object during publication"
            )
        else:
            print("PUBLISHED=0")
            print("PUBLICATION=not-requested")
        return 0
    except OSError:
        print("merge integrity: bounded local operation failed", file=sys.stderr)
        return 1
    except ValueError as error:
        print(f"merge integrity: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
