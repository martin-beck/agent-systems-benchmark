#!/usr/bin/env python3
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Validate ASB tutorial contracts without executing tutorial commands."""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Any

MAX_BYTES = 256 * 1024
MAX_STEPS = 64
MAX_ARGS = 16
ID = re.compile(r"^[A-Za-z][A-Za-z0-9._-]{0,63}$")
SHA256 = re.compile(r"^[0-9a-f]{64}$")
SECRET = re.compile(r"(?i)(api[_-]?key|token|password|secret|bearer|sk-[a-z0-9])")
SHELL = set(";&|<>`$(){}!\\\n\r")


class ValidationError(ValueError):
    pass


def _object(value: Any, name: str, fields: set[str]) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise ValidationError(f"{name} must be an object")
    unknown = set(value) - fields
    if unknown:
        raise ValidationError(f"{name} has unknown field(s): {', '.join(sorted(unknown))}")
    return value


def _string(value: Any, name: str, maximum: int = 128) -> str:
    if not isinstance(value, str) or not value or len(value) > maximum:
        raise ValidationError(f"{name} must be a bounded non-empty string")
    if SECRET.search(value):
        raise ValidationError(f"{name} appears to contain secret material")
    return value


def _safe_arg(value: Any, name: str) -> str:
    value = _string(value, name, 256)
    if any(char in value for char in SHELL):
        raise ValidationError(f"{name} contains shell or control syntax")
    return value


def _path_arg(value: str, name: str) -> None:
    if (
        value.startswith("-")
        or value.startswith("/")
        or value.startswith("~")
        or ".." in value.split("/")
        or "://" in value
        or value.lower().startswith(("http:", "https:", "tcp:", "udp:"))
    ):
        raise ValidationError(f"{name} must be a repository-relative reference")


def _command(command: Any, metadata: dict[str, Any], step: str) -> None:
    if not isinstance(command, list) or not command or len(command) > MAX_ARGS:
        raise ValidationError(f"step {step} command must be a bounded argument array")
    args = [_safe_arg(item, f"step {step} command argument") for item in command]
    if args[0] != metadata["executable"]:
        raise ValidationError(f"step {step} must invoke the ASB executable symbolically")
    root = args[1] if len(args) > 1 else ""
    commands = metadata["commands"]
    if root not in commands:
        raise ValidationError(f"step {step} uses unknown ASB command")
    tail = args[2:]
    spec = commands[root]
    if root in {"compare", "report"}:
        if len(tail) < spec["min_args"]:
            raise ValidationError(f"step {step} has too few arguments for {root}")
        for value in tail:
            _path_arg(value, f"step {step} {root} path")
        return
    if root == "auth":
        if not tail or tail[0] not in spec["operations"]:
            raise ValidationError(f"step {step} uses an unknown auth operation")
        required = spec["operations"][tail[0]]
        actual = tail[1:]
        if len(actual) != len(required) * 2 or any(
            actual[index * 2] != option
            for index, option in enumerate(required)
        ):
            raise ValidationError(f"step {step} auth options are missing, unknown, or reordered")
        for index in range(len(required)):
            _safe_arg(actual[index * 2 + 1], f"step {step} auth option value")
        return
    for form in spec["forms"]:
        if len(tail) != len(form):
            continue
        valid = True
        for value, expected in zip(tail, form):
            if expected == "PATH":
                _path_arg(value, f"step {step} path")
            elif expected == "SHA256" and not SHA256.fullmatch(value):
                valid = False
            elif expected == "AGENT" and not ID.fullmatch(value):
                valid = False
            elif expected not in {"PATH", "SHA256", "AGENT"} and value != expected:
                valid = False
        if valid:
            return
    raise ValidationError(f"step {step} arguments do not match current ASB command metadata")


def validate_document(document: Any, metadata: dict[str, Any]) -> None:
    root = _object(document, "tutorial", {"schema_version", "tutorial_id", "title", "steps"})
    if root.get("schema_version") != 1:
        raise ValidationError("tutorial schema_version must be 1")
    tutorial_id = _string(root.get("tutorial_id"), "tutorial_id")
    if not ID.fullmatch(tutorial_id):
        raise ValidationError("tutorial_id has invalid syntax")
    _string(root.get("title"), "title", 256)
    steps = root.get("steps")
    if not isinstance(steps, list) or not 0 < len(steps) <= MAX_STEPS:
        raise ValidationError("steps must contain between 1 and 64 steps")
    seen: set[str] = set()
    for raw in steps:
        step = _object(raw, "step", {"id", "command", "expect", "references", "network", "credentials"})
        step_id = _string(step.get("id"), "step id")
        if not ID.fullmatch(step_id) or step_id in seen:
            raise ValidationError("step IDs must be unique and use stable syntax")
        seen.add(step_id)
        _command(step.get("command"), metadata, step_id)
        expect = _object(step.get("expect"), f"step {step_id} expect", {"exit_code", "stdout_shape"})
        if not isinstance(expect.get("exit_code"), int) or not 0 <= expect["exit_code"] <= 125:
            raise ValidationError(f"step {step_id} has invalid expected exit code")
        _string(expect.get("stdout_shape"), f"step {step_id} stdout_shape", 64)
        if step.get("network", "denied") != "denied" or step.get("credentials", "none") != "none":
            raise ValidationError(f"step {step_id} must be offline and credential-free")
        references = step.get("references", [])
        if not isinstance(references, list) or len(references) > 16:
            raise ValidationError(f"step {step_id} references are unbounded")
        for reference in references:
            item = _object(reference, f"step {step_id} reference", {"path", "kind"})
            path = _string(item.get("path"), "reference path", 256)
            _path_arg(path, "reference path")
            if item.get("kind") not in {"input", "output", "config", "fixture"}:
                raise ValidationError(f"step {step_id} has an invalid reference kind")


def validate_metadata(metadata: Any) -> dict[str, Any]:
    """Validate the checked-in metadata before using it as executable grammar."""
    root = _object(metadata, "command metadata", {"schema_version", "executable", "commands"})
    if root.get("schema_version") != 1 or root.get("executable") != "asb":
        raise ValidationError("command metadata has an unsupported schema or executable")
    commands = root.get("commands")
    if not isinstance(commands, dict) or not commands:
        raise ValidationError("command metadata commands must be a non-empty object")
    for name, raw in commands.items():
        if not isinstance(name, str) or not ID.fullmatch(name):
            raise ValidationError("command metadata contains an invalid command name")
        spec = _object(raw, f"metadata command {name}", {"forms", "min_args", "operations"})
        present = [key for key in ("forms", "min_args", "operations") if key in spec]
        if len(present) != 1:
            raise ValidationError(f"metadata command {name} must define exactly one grammar form")
        if "forms" in spec:
            forms = spec["forms"]
            if not isinstance(forms, list) or not forms:
                raise ValidationError(f"metadata command {name} forms are invalid")
            for form in forms:
                if not isinstance(form, list) or len(form) > MAX_ARGS:
                    raise ValidationError(f"metadata command {name} has an invalid form")
                for value in form:
                    if not isinstance(value, str) or value not in {"PATH", "SHA256", "AGENT", "bash", "json", "--format", "--format=json", "--provider-selection", "--offline", "--dry-run", "--launch", "--provider", "--endpoint-digest", "--credential-digest", "--credential-reference-sha256", "--catalog-sha256", "--provider-profile", "--agent", "--model", "--output", "--idempotency-key", "launch", "status", "doctor", "remove", "install", "upgrade"}:
                        raise ValidationError(f"metadata command {name} has an unknown grammar token")
        elif "min_args" in spec:
            if not isinstance(spec["min_args"], int) or spec["min_args"] < 1 or spec["min_args"] > MAX_ARGS:
                raise ValidationError(f"metadata command {name} has an invalid minimum argument count")
        else:
            operations = spec["operations"]
            if not isinstance(operations, dict) or not operations:
                raise ValidationError(f"metadata command {name} operations are invalid")
            for operation, required in operations.items():
                if not ID.fullmatch(operation) or not isinstance(required, list) or not required:
                    raise ValidationError(f"metadata command {name} has an invalid operation")
                if any(not isinstance(option, str) or not option.startswith("--") for option in required):
                    raise ValidationError(f"metadata command {name} has an invalid option")
    return root


def validate_comparison_fixture(document: Any) -> None:
    """Validate a bounded comparison result without running a benchmark."""
    root = _object(
        document,
        "comparison",
        {
            "schema_version", "comparison_id", "benchmark", "measurement",
            "configuration_digest", "runs", "denominator", "comparable", "reason",
        },
    )
    if root.get("schema_version") != 1:
        raise ValidationError("comparison schema_version must be 1")
    _string(root.get("comparison_id"), "comparison_id")
    benchmark = _object(root.get("benchmark"), "comparison benchmark", {"id", "revision"})
    measurement = _object(
        root.get("measurement"), "comparison measurement", {"id", "revision", "unit"}
    )
    for name, value in [
        ("benchmark id", benchmark.get("id")),
        ("benchmark revision", benchmark.get("revision")),
        ("measurement id", measurement.get("id")),
        ("measurement revision", measurement.get("revision")),
        ("measurement unit", measurement.get("unit")),
    ]:
        _string(value, name)
    digest = root.get("configuration_digest")
    if not isinstance(digest, str) or not SHA256.fullmatch(digest):
        raise ValidationError("comparison configuration_digest must be SHA-256")
    runs = root.get("runs")
    if not isinstance(runs, list) or len(runs) < 2 or len(runs) > MAX_STEPS:
        raise ValidationError("comparison runs must contain between 2 and 64 runs")
    failed = completed = 0
    identities = set()
    for raw in runs:
        run = _object(
            raw,
            "comparison run",
            {"run_id", "agent", "terminal_state", "score", "benchmark"},
        )
        _string(run.get("run_id"), "comparison run_id")
        _string(run.get("agent"), "comparison agent")
        state = run.get("terminal_state")
        if state not in {"completed", "failed", "cancelled", "needs_reconciliation"}:
            raise ValidationError("comparison run has invalid terminal_state")
        if state == "failed":
            failed += 1
        if state == "completed":
            completed += 1
            if not isinstance(run.get("score"), (int, float)) or isinstance(
                run.get("score"), bool
            ):
                raise ValidationError("completed comparison run must have a numeric score")
        run_benchmark = run.get("benchmark")
        if run_benchmark is not None:
            candidate = _object(
                run_benchmark, "comparison run benchmark", {"id", "revision"}
            )
            identity = (candidate.get("id"), candidate.get("revision"))
            if identity != (benchmark["id"], benchmark["revision"]):
                identities.add(identity)
    denominator = _object(
        root.get("denominator"),
        "comparison denominator",
        {"attempted", "completed", "failed", "eligible"},
    )
    counts = {
        name: denominator.get(name)
        for name in ("attempted", "completed", "failed", "eligible")
    }
    if any(
        not isinstance(value, int) or isinstance(value, bool) or value < 0
        for value in counts.values()
    ):
        raise ValidationError("comparison denominator counts must be non-negative integers")
    if (
        counts["attempted"] != len(runs)
        or counts["completed"] != completed
        or counts["failed"] != failed
    ):
        raise ValidationError("comparison denominator must retain every terminal run")
    comparable = root.get("comparable")
    if not isinstance(comparable, bool):
        raise ValidationError("comparison comparable must be boolean")
    if identities or (comparable and failed) or (comparable and counts["eligible"] != completed):
        raise ValidationError("comparison silently compares incompatible or failed runs")
    if not comparable and not root.get("reason"):
        raise ValidationError("non-comparable comparison needs a reason")


def load(path: Path) -> Any:
    if path.is_symlink() or not path.is_file() or path.stat().st_size > MAX_BYTES:
        raise ValidationError("tutorial must be a bounded regular file")
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        raise ValidationError("tutorial is not valid bounded UTF-8 JSON") from exc


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("tutorial", type=Path)
    parser.add_argument("--metadata", type=Path, default=Path(__file__).with_name("command_metadata_v1.json"))
    args = parser.parse_args(argv)
    try:
        metadata = validate_metadata(load(args.metadata))
        validate_document(load(args.tutorial), metadata)
    except ValidationError as exc:
        print(f"tutorial validation failed: {exc}", file=sys.stderr)
        return 1
    print(f"valid tutorial: {args.tutorial}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
