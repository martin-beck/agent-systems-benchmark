#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Validate matching DCO trailers over an explicit immutable commit range."""

from __future__ import annotations

import argparse
import subprocess
from pathlib import Path


def commit_range(root: Path, base: str | None, head: str) -> list[str]:
    expression = head if base is None else f"{base}..{head}"
    output = subprocess.check_output(
        ["git", "-C", str(root), "rev-list", "--reverse", expression], text=True
    )
    return output.split()


def validate_dco(root: Path, revisions: list[str]) -> None:
    if not revisions:
        raise ValueError("DCO policy received an empty revision range")
    for revision in revisions:
        author = subprocess.check_output(
            ["git", "-C", str(root), "show", "-s", "--format=%an <%ae>", revision],
            text=True,
        ).strip()
        body = subprocess.check_output(
            ["git", "-C", str(root), "show", "-s", "--format=%B", revision], text=True
        ).splitlines()
        if f"Signed-off-by: {author}" not in body:
            raise ValueError(f"{revision} lacks a matching Signed-off-by trailer")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=Path.cwd())
    parser.add_argument("--base")
    parser.add_argument("--head", required=True)
    args = parser.parse_args()
    try:
        validate_dco(args.root, commit_range(args.root, args.base, args.head))
    except (OSError, subprocess.CalledProcessError, ValueError) as error:
        print(f"DCO policy: {error}")
        return 1
    print("DCO policy: all commits certified")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
