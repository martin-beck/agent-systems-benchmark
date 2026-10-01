#!/usr/bin/env python3
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Run a bounded, deterministic ASB setup/record/replay fault matrix."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import tempfile
from pathlib import Path
from typing import Any

MAX_CASES = 128
MAX_OUTPUT = 64 * 1024
MAX_TIMEOUT = 300.0
KINDS = {"setup", "record", "replay", "benchmark", "recovery"}


def load_manifest(path: Path) -> list[dict[str, Any]]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict) or value.get("schema_version") != 1:
        raise ValueError("matrix schema_version must be 1")
    cases = value.get("cases")
    if not isinstance(cases, list) or not cases or len(cases) > MAX_CASES:
        raise ValueError("matrix cases must contain 1..128 entries")
    result: list[dict[str, Any]] = []
    names: set[str] = set()
    for case in cases:
        if not isinstance(case, dict):
            raise ValueError("matrix case must be an object")
        name = case.get("name")
        kind = case.get("kind")
        argv = case.get("argv")
        if not isinstance(name, str) or not name or name in names:
            raise ValueError("matrix case names must be unique non-empty strings")
        if kind not in KINDS:
            raise ValueError(f"matrix case {name!r} has an unsupported kind")
        if not isinstance(argv, list) or not argv or not all(
            isinstance(item, str) and item for item in argv
        ):
            raise ValueError(f"matrix case {name!r} argv must be non-empty strings")
        timeout = case.get("timeout_seconds", 30.0)
        expected_exit = case.get("expected_exit", 0)
        if not isinstance(timeout, (int, float)) or not 0 < timeout <= MAX_TIMEOUT:
            raise ValueError(f"matrix case {name!r} timeout is out of bounds")
        if not isinstance(expected_exit, int) or not -255 <= expected_exit <= 255:
            raise ValueError(f"matrix case {name!r} expected_exit is invalid")
        cleanup = case.get("cleanup", [])
        if not isinstance(cleanup, list) or not all(isinstance(item, str) and item for item in cleanup):
            raise ValueError(f"matrix case {name!r} cleanup must be a string list")
        names.add(name)
        result.append({"name": name, "kind": kind, "argv": argv, "timeout": float(timeout),
                       "expected_exit": expected_exit, "cleanup": cleanup,
                       "development_warning_only": bool(case.get("development_warning_only", False))})
    return result


def run_case(binary: Path, case: dict[str, Any], root: Path) -> dict[str, Any]:
    command = [str(binary), *case["argv"]]
    env = {"PATH": "/usr/bin:/bin", "HOME": str(root / "home"), "ASB_MATRIX_ROOT": str(root)}
    root.joinpath("home").mkdir(parents=True)
    try:
        completed = subprocess.run(command, cwd=root, env=env, text=True,
                                   stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                   timeout=case["timeout"], check=False)
        output = completed.stdout[:MAX_OUTPUT]
        truncated = len(completed.stdout) > MAX_OUTPUT
        if truncated:
            classification = "output_exceeded"
        elif completed.returncode == case["expected_exit"]:
            classification = "warning" if case["development_warning_only"] and completed.returncode else "passed"
        else:
            classification = "failed"
        exit_code = completed.returncode
    except subprocess.TimeoutExpired as error:
        output = (error.stdout or "")[:MAX_OUTPUT] if isinstance(error.stdout, str) else ""
        classification, exit_code, truncated = "timeout", None, False
    finally:
        cleanup_ok = True
        for relative in case["cleanup"]:
            target = (root / relative).resolve()
            if root not in target.parents:
                cleanup_ok = False
                continue
            if target.exists() or target.is_symlink():
                cleanup_ok = False
    if not cleanup_ok and classification == "passed":
        classification = "cleanup_failed"
    return {"name": case["name"], "kind": case["kind"], "classification": classification,
            "exit_code": exit_code, "expected_exit": case["expected_exit"],
            "output": output, "cleanup_ok": cleanup_ok}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--binary", type=Path, default=Path("asb"))
    parser.add_argument("--json", action="store_true", dest="machine")
    args = parser.parse_args()
    try:
        cases = load_manifest(args.manifest)
        if not args.binary.is_absolute():
            binary = (Path.cwd() / args.binary).resolve()
        else:
            binary = args.binary
        if not binary.is_file() or not os.access(binary, os.X_OK):
            raise ValueError("binary must be an executable regular file")
        with tempfile.TemporaryDirectory(prefix="asb-fault-matrix-") as directory:
            results = [run_case(binary, case, Path(directory) / case["name"]) for case in cases]
        payload = {"schema_version": 1, "runner": "asb-fault-matrix-v1", "results": results,
                   "passed": all(result["classification"] in {"passed", "warning"} for result in results)}
        if args.machine:
            print(json.dumps(payload, sort_keys=True))
        else:
            for result in results:
                print(f'{result["kind"]}: {result["name"]}: {result["classification"]}')
            print("matrix: passed" if payload["passed"] else "matrix: failed")
        return 0 if payload["passed"] else 1
    except (OSError, ValueError, json.JSONDecodeError) as error:
        payload = {"schema_version": 1, "runner": "asb-fault-matrix-v1", "passed": False,
                   "classification": "invalid_matrix", "error": str(error)}
        print(json.dumps(payload, sort_keys=True) if args.machine else f"matrix: invalid_matrix: {error}")
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
