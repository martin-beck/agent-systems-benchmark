#!/usr/bin/env python3
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Fail closed unless Alloy receipts contain the expected finite outcomes."""

import json
import pathlib
import sys


def commands(path: pathlib.Path) -> dict[str, object]:
    if path.stat().st_size > 1024 * 1024:
        raise ValueError("receipt exceeds the one MiB verification bound")
    value = json.loads(path.read_text(encoding="utf-8"))
    result = value.get("commands")
    if not isinstance(result, dict):
        raise ValueError("receipt commands must be an object")
    return result


def has_instance(command: object) -> bool:
    return isinstance(command, dict) and bool(command.get("solution"))


def main() -> int:
    if len(sys.argv) != 3:
        print("usage: verify_alloy_receipt.py POSITIVE NEGATIVE", file=sys.stderr)
        return 2
    positive = commands(pathlib.Path(sys.argv[1]))
    negative = commands(pathlib.Path(sys.argv[2]))
    expected = {
        "UniqueLease",
        "StaleLeaseFenced",
        "JournalProjectionPrefix",
        "NoDuplicateCompletion",
        "UncertainHasNoLease",
        "ReplayCursorsBounded",
    }
    if set(positive) != expected | {"Witness"}:
        raise ValueError("positive receipt command set changed")
    if not has_instance(positive["Witness"]):
        raise ValueError("positive witness is unsatisfiable")
    if any(has_instance(positive[name]) for name in expected):
        raise ValueError("positive assertion has a counterexample")
    if set(negative) != expected:
        raise ValueError("negative receipt command set changed")
    missing = sorted(name for name in expected if not has_instance(negative[name]))
    if missing:
        raise ValueError(f"negative mutations were not detected: {missing}")
    print("Alloy positive and mutation receipts passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
