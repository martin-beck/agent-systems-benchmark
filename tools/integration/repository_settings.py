#!/usr/bin/env python3
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Audit or apply the repository settings that disable web-created merges."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys

REQUIRED = {
    "allow_merge_commit": False,
    "allow_squash_merge": False,
    "allow_rebase_merge": False,
    "allow_auto_merge": False,
    "web_commit_signoff_required": True,
}


def fetch(repository: str) -> dict[str, object]:
    result = subprocess.run(
        ["gh", "api", f"repos/{repository}"],
        text=True,
        capture_output=True,
        check=False,
    )
    if result.returncode:
        raise ValueError(result.stderr.strip() or "GitHub settings query failed")
    return json.loads(result.stdout)


def apply(repository: str) -> dict[str, object]:
    command = ["gh", "api", "--method", "PATCH", f"repos/{repository}"]
    for key, value in REQUIRED.items():
        command.extend(["-F", f"{key}={str(value).lower()}"])
    result = subprocess.run(command, text=True, capture_output=True, check=False)
    if result.returncode:
        raise ValueError(result.stderr.strip() or "GitHub settings update failed")
    return json.loads(result.stdout)


def validate(settings: dict[str, object]) -> None:
    for key, expected in REQUIRED.items():
        if settings.get(key) is not expected:
            raise ValueError(
                f"repository setting {key} must be {str(expected).lower()}"
            )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repository", default="martin-beck/agent-systems-benchmark")
    parser.add_argument(
        "--settings-json", help="offline fixture instead of a GitHub query"
    )
    parser.add_argument("--apply", action="store_true")
    args = parser.parse_args()
    try:
        if args.settings_json is not None and args.apply:
            raise ValueError("--settings-json cannot be combined with --apply")
        settings = (
            json.loads(args.settings_json)
            if args.settings_json is not None
            else apply(args.repository)
            if args.apply
            else fetch(args.repository)
        )
        if not isinstance(settings, dict):
            raise TypeError("repository settings response must be an object")
        validate(settings)
    except (OSError, TypeError, ValueError, json.JSONDecodeError) as error:
        print(f"merge settings: {error}", file=sys.stderr)
        return 1
    print("merge settings: incompatible web merge modes are disabled")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
