#!/usr/bin/env python3
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Run a bounded, deterministic ASB setup/record/replay fault matrix."""

from __future__ import annotations

import argparse
import json
import os
import selectors
import signal
import shutil
import subprocess
import tempfile
import time
from pathlib import Path
from typing import Any

MAX_CASES = 128
MAX_OUTPUT = 64 * 1024
MAX_WORKSPACE = 64 * 1024 * 1024
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
        warning_code = case.get("warning_code")
        if warning_code is not None and (not isinstance(warning_code, str) or not warning_code or len(warning_code) > 128):
            raise ValueError(f"matrix case {name!r} warning_code is invalid")
        network = case.get("network", "none")
        if network != "none":
            raise ValueError(f"matrix case {name!r} must declare network=none")
        development_warning_only = case.get("development_warning_only", False)
        if not isinstance(development_warning_only, bool):
            raise ValueError(f"matrix case {name!r} development_warning_only is invalid")
        result.append({"name": name, "kind": kind, "argv": argv, "timeout": float(timeout),
                       "expected_exit": expected_exit, "cleanup": cleanup,
                       "development_warning_only": development_warning_only,
                       "warning_code": warning_code, "network": network})
    return result


def isolated_command(command: list[str]) -> list[str]:
    """Return a command with an enforced private network namespace.

    A missing or unusable isolation primitive is a typed runner-unavailable
    result, never an implicit fallback to the host network.
    """
    if os.name != "posix" or not shutil.which("unshare"):
        raise RuntimeError("network isolation unavailable")
    return ["unshare", "--net", "--", *command]


def workspace_bytes(root: Path) -> int:
    total = 0
    for path in root.rglob("*"):
        if path.is_file() and not path.is_symlink():
            try:
                total += path.stat().st_size
            except OSError:
                return MAX_WORKSPACE + 1
            if total > MAX_WORKSPACE:
                return total
    return total


def process_group_has_members(pgid: int) -> bool:
    """Detect descendants left behind after a session leader exits."""
    if os.name != "posix":
        return False
    proc = Path("/proc")
    try:
        entries = list(proc.iterdir())
    except OSError:
        return False
    for entry in entries:
        if not entry.name.isdigit():
            continue
        try:
            stat = (entry / "stat").read_text(encoding="utf-8")
            fields = stat[stat.rfind(")") + 2 :].split()
            # After the comm field: state, ppid, pgrp.
            if len(fields) > 2 and int(fields[2]) == pgid:
                return True
        except (OSError, ValueError):
            continue
    return False


def run_case(binary: Path, case: dict[str, Any], root: Path) -> dict[str, Any]:
    command = [str(binary), *case["argv"]]
    env = {"PATH": "/usr/bin:/bin", "HOME": str(root / "home"), "ASB_MATRIX_ROOT": str(root)}
    root.joinpath("home").mkdir(parents=True)
    process: subprocess.Popen[str] | None = None
    output = ""
    classification = "runner_unavailable"
    exit_code: int | None = None
    network = case.get("network", "none")
    if network != "none":
        shutil.rmtree(root, ignore_errors=True)
        return {"name": case["name"], "kind": case["kind"], "classification": "invalid_matrix",
                "exit_code": None, "expected_exit": case["expected_exit"], "output": "",
                "cleanup_ok": True}
    selector: selectors.BaseSelector | None = None
    isolation_attempted = False
    try:
        command = isolated_command(command)
        isolation_attempted = True
        process = subprocess.Popen(command, cwd=root, env=env, text=False,
                                   stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                   start_new_session=True)
        selector = selectors.DefaultSelector()
        assert process.stdout is not None
        selector.register(process.stdout, selectors.EVENT_READ)
        chunks: list[bytes] = []
        size = 0
        deadline = time.monotonic() + case["timeout"]
        while process.poll() is None or selector.get_map():
            if time.monotonic() >= deadline:
                classification = "timeout"
                break
            if workspace_bytes(root) > MAX_WORKSPACE:
                classification = "workspace_exceeded"
                break
            for key, _ in selector.select(min(0.05, max(0.0, deadline - time.monotonic()))):
                data = os.read(key.fileobj.fileno(), min(8192, MAX_OUTPUT - size + 1))
                if not data:
                    selector.unregister(key.fileobj)
                    continue
                size += len(data)
                if size > MAX_OUTPUT:
                    classification = "output_exceeded"
                    break
                chunks.append(data)
            if classification == "output_exceeded":
                break
        output = b"".join(chunks).decode("utf-8", "replace")[:MAX_OUTPUT]
        if classification == "runner_unavailable":
            exit_code = process.wait(timeout=1)
            if exit_code == case["expected_exit"]:
                if case["development_warning_only"]:
                    try:
                        warning = json.loads(output)
                    except json.JSONDecodeError:
                        classification = "warning_invalid"
                    else:
                        classification = "warning" if warning.get("status") == "warning" and isinstance(warning.get("code"), str) and (case["warning_code"] is None or warning["code"] == case["warning_code"]) else "warning_invalid"
                else:
                    classification = "passed"
            else:
                if isolation_attempted and "operation not permitted" in output.lower():
                    classification = "runner_unavailable"
                else:
                    classification = "failed"
    except (OSError, RuntimeError):
        classification = "runner_unavailable"
    finally:
        if selector is not None:
            selector.close()
        cleanup_ok = True
        group_alive = process is not None and process_group_has_members(process.pid)
        if group_alive and classification in {"passed", "warning", "runner_unavailable"}:
            classification = "descendants_survived"
        must_kill_group = (
            process is not None
            and (process.poll() is None or group_alive
                 or classification in {"timeout", "output_exceeded", "workspace_exceeded"})
        )
        if must_kill_group and process is not None:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            except OSError:
                cleanup_ok = False
        if process is not None:
            try:
                process.wait(timeout=2)
            except subprocess.TimeoutExpired:
                cleanup_ok = False
            if process.stdout is not None:
                process.stdout.close()
        if group_alive:
            deadline = time.monotonic() + 2
            while process_group_has_members(process.pid) and time.monotonic() < deadline:
                time.sleep(0.02)
            if process_group_has_members(process.pid):
                cleanup_ok = False
        for relative in case["cleanup"]:
            target = (root / relative).resolve()
            if root not in target.parents:
                cleanup_ok = False
                continue
            if target.exists() or target.is_symlink():
                cleanup_ok = False
        try:
            shutil.rmtree(root)
        except OSError:
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
    except (ValueError, json.JSONDecodeError):
        payload = {"schema_version": 1, "runner": "asb-fault-matrix-v1", "passed": False,
                   "classification": "invalid_matrix"}
        print(json.dumps(payload, sort_keys=True) if args.machine else "matrix: invalid_matrix")
        return 2
    except (OSError, RuntimeError):
        payload = {"schema_version": 1, "runner": "asb-fault-matrix-v1", "passed": False,
                   "classification": "runner_unavailable"}
        print(json.dumps(payload, sort_keys=True) if args.machine else "matrix: runner_unavailable")
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
