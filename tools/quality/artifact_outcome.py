#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Create and classify bounded optional CI evidence without hiding test failures."""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
from pathlib import Path

MAX_JSON_BYTES = 1024
SHA = re.compile(r"^[0-9a-f]{40}$")
DIGITS = re.compile(r"^[1-9][0-9]{0,19}$")
REPOSITORY = "martin-beck/agent-systems-benchmark"
PULL_REF = re.compile(r"^refs/pull/[1-9][0-9]{0,9}/merge$")
BRANCH_REF = re.compile(r"^refs/heads/[A-Za-z0-9][A-Za-z0-9._/-]{0,199}$")


def fail(message: str) -> None:
    raise ValueError(message)


def bounded_uint(raw: str, name: str, ceiling: int) -> int:
    if not DIGITS.fullmatch(raw):
        fail(f"{name} is malformed")
    value = int(raw)
    if value > ceiling:
        fail(f"{name} exceeds its bound")
    return value


def approved_ref(value: str) -> bool:
    if PULL_REF.fullmatch(value):
        return True
    if not BRANCH_REF.fullmatch(value):
        return False
    name = value.removeprefix("refs/heads/")
    return not (
        name.endswith(("/", ".", ".lock"))
        or "//" in name
        or ".." in name
        or "@{" in name
        or any(
            part.startswith(".") or part.endswith(".lock") for part in name.split("/")
        )
    )


def emit(value: dict[str, object]) -> None:
    encoded = json.dumps(value, sort_keys=True, separators=(",", ":"))
    if len(encoded.encode("utf-8")) > MAX_JSON_BYTES:
        fail("artifact status exceeds its bound")
    print(encoded)


def prepare(args: argparse.Namespace) -> None:
    if args.repository != REPOSITORY:
        fail("repository is not the approved public repository")
    if not approved_ref(args.ref):
        fail("ref is not an approved immutable CI ref")
    if not SHA.fullmatch(args.head):
        fail("head is not a full commit ID")
    run_id = bounded_uint(args.run_id, "run ID", 2**63 - 1)
    attempt = bounded_uint(args.attempt, "run attempt", 1000)

    root = args.output_root.resolve(strict=True)
    output = args.output
    if (
        output.parent.resolve(strict=True) != root
        or output.is_symlink()
        or output.exists()
    ):
        fail("evidence output is not a new direct child of the output root")
    payload = {
        "check": "repository-quality",
        "head": args.head,
        "repository": args.repository,
        "run_attempt": attempt,
        "run_id": run_id,
        "schema_version": 1,
    }
    encoded = (
        json.dumps(payload, sort_keys=True, separators=(",", ":")) + "\n"
    ).encode()
    if len(encoded) > MAX_JSON_BYTES:
        fail("evidence document exceeds its bound")
    root_descriptor = os.open(root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    descriptor = -1
    try:
        descriptor = os.open(
            output.name,
            os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
            0o600,
            dir_fd=root_descriptor,
        )
        if os.write(descriptor, encoded) != len(encoded):
            fail("evidence document write was incomplete")
        os.fsync(descriptor)
        os.fsync(root_descriptor)
    except Exception:
        if descriptor >= 0:
            os.unlink(output.name, dir_fd=root_descriptor)
        raise
    finally:
        if descriptor >= 0:
            os.close(descriptor)
        os.close(root_descriptor)


def classify(args: argparse.Namespace) -> int:
    attempt = bounded_uint(args.attempt, "run attempt", 1000)
    if args.outcome == "success":
        emit({"artifact": "published", "run_attempt": attempt, "schema_version": 1})
        return 0
    if args.outcome == "failure" and args.role == "optional":
        emit(
            {
                "artifact": "unavailable",
                "cause": "provider-upload-failure",
                "retry": "manual-after-provider-recalculation",
                "run_attempt": attempt,
                "schema_version": 1,
            }
        )
        return 0
    emit(
        {
            "artifact": "required-publication-failed"
            if args.role == "required"
            else "publication-interrupted",
            "run_attempt": attempt,
            "schema_version": 1,
        }
    )
    return 1


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser()
    commands = result.add_subparsers(dest="command", required=True)
    create = commands.add_parser("prepare")
    create.add_argument("--repository", required=True)
    create.add_argument("--ref", required=True)
    create.add_argument("--head", required=True)
    create.add_argument("--run-id", required=True)
    create.add_argument("--attempt", required=True)
    create.add_argument("--output-root", type=Path, required=True)
    create.add_argument("--output", type=Path, required=True)
    outcome = commands.add_parser("classify")
    outcome.add_argument("--role", choices=("optional", "required"), required=True)
    outcome.add_argument(
        "--outcome",
        choices=("success", "failure", "cancelled", "skipped"),
        required=True,
    )
    outcome.add_argument("--attempt", required=True)
    return result


def main() -> int:
    args = parser().parse_args()
    try:
        if args.command == "prepare":
            prepare(args)
            return 0
        return classify(args)
    except OSError:
        print(
            "artifact evidence unavailable: filesystem boundary failed", file=sys.stderr
        )
        return 1
    except ValueError as error:
        print(f"artifact evidence unavailable: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":  # pragma: no cover - exercised by hosted workflow
    raise SystemExit(main())
