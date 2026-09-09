#!/usr/bin/env python3
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Emit bounded rolling-hosted portability evidence, never native qualification."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import re
import selectors
import signal
import stat
import subprocess
import time
from collections.abc import Sequence
from pathlib import Path
from typing import Any, cast
from urllib.parse import urlparse

import native_evidence as native

MAX_EVIDENCE_BYTES = 1 << 20
ROLLING_VERSION = re.compile(r"^24[.]04[.]([0-9]+) LTS [(]Noble Numbat[)]$")
LIMITATIONS = [
    "rolling-hosted-image",
    "not-native-qualification",
    "not-performance-baseline",
    "native-aarch64-unqualified",
]
SANDBOX_UNAVAILABLE = b"native sandbox capability unavailable:"
SANDBOX_LIMITATION = "native-sandbox-unavailable"
SCHEMA_SHA256 = {
    "hosted-portability.schema.json": "50646c9648afc2963580ef55209a9c967277ed7023fe0bd546898767bbb6ed00",
    "native-evidence.schema.json": "332056435d69dd8cdd237e7b587289c8b31e75f514c04ecdec04bd75e4356017",
}


class PortabilityError(RuntimeError):
    """A hosted portability precondition or check failed."""


def _bounded_json(path: Path, expected_sha256: str | None = None) -> dict[str, Any]:
    """Read one bounded no-follow regular JSON object."""
    descriptor = -1
    try:
        descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
        metadata = os.fstat(descriptor)
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_size > MAX_EVIDENCE_BYTES:
            raise PortabilityError("platform evidence is not a bounded regular file")
        data = os.read(descriptor, MAX_EVIDENCE_BYTES + 1)
        if len(data) > MAX_EVIDENCE_BYTES or os.read(descriptor, 1):
            raise PortabilityError("platform evidence exceeds byte limit")
        if expected_sha256 is not None and hashlib.sha256(data).hexdigest() != expected_sha256:
            raise PortabilityError("platform evidence schema identity is not exact")
        decoded = json.loads(data.decode("utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise PortabilityError("platform evidence cannot be decoded safely") from error
    finally:
        if descriptor >= 0:
            os.close(descriptor)
    if not isinstance(decoded, dict):
        raise PortabilityError("platform evidence must be one object")
    return cast(dict[str, Any], decoded)


def _resolve_ref(root: dict[str, Any], reference: str) -> dict[str, Any]:
    """Resolve the closed local-reference form used by pinned platform schemas."""
    if not reference.startswith("#/$defs/") or "/" in reference.removeprefix("#/$defs/"):
        raise PortabilityError("platform evidence schema reference is unsupported")
    target = root.get("$defs", {}).get(reference.removeprefix("#/$defs/"))
    if not isinstance(target, dict):
        raise PortabilityError("platform evidence schema reference is invalid")
    return cast(dict[str, Any], target)


def _matches_schema(value: Any, schema: dict[str, Any], root: dict[str, Any]) -> bool:
    try:
        _validate_schema(value, schema, root)
    except PortabilityError:
        return False
    return True


def _json_equal(left: Any, right: Any) -> bool:
    """Compare JSON values without Python's bool-as-int equality leak."""
    if isinstance(left, bool) or isinstance(right, bool):
        return isinstance(left, bool) and isinstance(right, bool) and left == right
    if isinstance(left, (int, float)) or isinstance(right, (int, float)):
        return (
            isinstance(left, (int, float))
            and not isinstance(left, bool)
            and isinstance(right, (int, float))
            and not isinstance(right, bool)
            and left == right
        )
    if isinstance(left, list) or isinstance(right, list):
        return (
            isinstance(left, list)
            and isinstance(right, list)
            and len(left) == len(right)
            and all(_json_equal(a, b) for a, b in zip(left, right, strict=True))
        )
    if isinstance(left, dict) or isinstance(right, dict):
        return (
            isinstance(left, dict)
            and isinstance(right, dict)
            and left.keys() == right.keys()
            and all(_json_equal(left[key], right[key]) for key in left)
        )
    return type(left) is type(right) and left == right


def _validate_schema(value: Any, schema: dict[str, Any], root: dict[str, Any]) -> None:
    """Validate the exact, digest-pinned platform-schema keyword closure."""
    if "$ref" in schema:
        _validate_schema(value, _resolve_ref(root, cast(str, schema["$ref"])), root)
        return
    if "oneOf" in schema:
        alternatives = cast(list[dict[str, Any]], schema["oneOf"])
        if sum(_matches_schema(value, option, root) for option in alternatives) != 1:
            raise PortabilityError("platform evidence does not match its closed schema")
        return
    if "const" in schema and not _json_equal(value, schema["const"]):
        raise PortabilityError("platform evidence does not match its closed schema")
    if "enum" in schema and not any(
        _json_equal(value, option) for option in cast(list[Any], schema["enum"])
    ):
        raise PortabilityError("platform evidence does not match its closed schema")

    declared_type = cast(str | None, schema.get("type"))
    valid_type = declared_type is None or {
        "object": isinstance(value, dict),
        "array": isinstance(value, list),
        "string": isinstance(value, str),
        "integer": isinstance(value, int) and not isinstance(value, bool),
        "boolean": isinstance(value, bool),
    }.get(declared_type, False)
    if not valid_type:
        raise PortabilityError("platform evidence does not match its closed schema")

    if isinstance(value, dict) and declared_type == "object":
        properties = cast(dict[str, dict[str, Any]], schema.get("properties", {}))
        required = cast(list[str], schema.get("required", []))
        if any(key not in value for key in required):
            raise PortabilityError("platform evidence does not match its closed schema")
        if schema.get("additionalProperties") is False and any(
            key not in properties for key in value
        ):
            raise PortabilityError("platform evidence does not match its closed schema")
        for key, item in value.items():
            if key in properties:
                _validate_schema(item, properties[key], root)

    if isinstance(value, list) and declared_type == "array":
        minimum = cast(int, schema.get("minItems", 0))
        maximum = cast(int | None, schema.get("maxItems"))
        if len(value) < minimum or (maximum is not None and len(value) > maximum):
            raise PortabilityError("platform evidence does not match its closed schema")
        if schema.get("uniqueItems"):
            for index, item in enumerate(value):
                if any(_json_equal(item, other) for other in value[index + 1 :]):
                    raise PortabilityError("platform evidence does not match its closed schema")
        prefix = cast(list[dict[str, Any]], schema.get("prefixItems", []))
        for item, item_schema in zip(value, prefix, strict=False):
            _validate_schema(item, item_schema, root)
        remaining = value[len(prefix) :]
        items = schema.get("items")
        if items is False and remaining:
            raise PortabilityError("platform evidence does not match its closed schema")
        if isinstance(items, dict):
            for item in remaining if prefix else value:
                _validate_schema(item, cast(dict[str, Any], items), root)

    if isinstance(value, str) and declared_type == "string":
        minimum_length = cast(int, schema.get("minLength", 0))
        maximum_length = cast(int | None, schema.get("maxLength"))
        if len(value) < minimum_length or (
            maximum_length is not None and len(value) > maximum_length
        ):
            raise PortabilityError("platform evidence does not match its closed schema")
        pattern = schema.get("pattern")
        if isinstance(pattern, str) and re.search(pattern, value) is None:
            raise PortabilityError("platform evidence does not match its closed schema")
        if schema.get("format") == "uri":
            parsed = urlparse(value)
            if not parsed.scheme or not parsed.netloc:
                raise PortabilityError("platform evidence does not match its closed schema")

    if isinstance(value, int) and not isinstance(value, bool) and declared_type == "integer":
        minimum_value = cast(int | None, schema.get("minimum"))
        maximum_value = cast(int | None, schema.get("maximum"))
        if (minimum_value is not None and value < minimum_value) or (
            maximum_value is not None and value > maximum_value
        ):
            raise PortabilityError("platform evidence does not match its closed schema")


def _remove_candidate(path: Path, output_root: Path) -> None:
    """Remove only a direct candidate artifact beneath the trusted output root."""
    try:
        if path.absolute().parent != output_root.absolute():
            return
        metadata = path.lstat()
        if stat.S_ISREG(metadata.st_mode) or stat.S_ISLNK(metadata.st_mode):
            path.unlink()
    except FileNotFoundError:
        pass


def validate_artifact(
    route: str,
    native_path: Path,
    hosted_path: Path,
    output_root: Path,
    source: Path,
) -> str:
    """Validate exact route/schema exclusivity or erase candidate artifacts."""
    candidates = (native_path, hosted_path)
    try:
        if route not in {"native-qualification", "hosted-portability"}:
            raise PortabilityError("platform evidence route is invalid")
        if native_path == hosted_path or any(
            path.absolute().parent != output_root.absolute() for path in candidates
        ):
            raise PortabilityError("platform evidence paths are not isolated")
        selected, absent, schema_name = {
            "native-qualification": (
                native_path,
                hosted_path,
                "native-evidence.schema.json",
            ),
            "hosted-portability": (
                hosted_path,
                native_path,
                "hosted-portability.schema.json",
            ),
        }[route]
        if absent.exists() or absent.is_symlink():
            raise PortabilityError("more than one platform evidence kind exists")
        report = _bounded_json(selected)
        schema = _bounded_json(
            source / "platforms/v1" / schema_name, SCHEMA_SHA256[schema_name]
        )
        _validate_schema(report, schema, schema)
        if route == "native-qualification" and (
            report.get("kind"), report.get("qualification")
        ) != ("native-run", "native-functional"):
            raise PortabilityError("platform evidence route and claim differ")
        if route == "hosted-portability":
            sandbox = cast(dict[str, Any], cast(dict[str, Any], report["checks"])["sandbox"])
            unavailable = sandbox["status"] == "unavailable"
            expected_qualification = (
                "functional-portability-partial"
                if unavailable
                else "functional-portability-only"
            )
            limitations = cast(list[str], report["limitations"])
            if (
                report["kind"] != "hosted-portability"
                or report["qualification"] != expected_qualification
                or (SANDBOX_LIMITATION in limitations) != unavailable
            ):
                raise PortabilityError("platform evidence route and claim differ")
        return route
    except PortabilityError:
        for candidate in candidates:
            _remove_candidate(candidate, output_root)
        raise


def release_route(
    release: dict[str, str], execution_class: str, root: Path = Path("/")
) -> str:
    """Select a route from explicit execution provenance and release identity."""
    profile = native.PLATFORMS["ubuntu-24.04"]
    if release.get("ID") != profile["id"] or release.get("VERSION_ID") != profile["version_id"]:
        raise PortabilityError("hosted distribution does not match the declared runner family")
    version = release.get("VERSION", "")
    if execution_class == "trusted-native":
        try:
            native.validate_platform("ubuntu-24.04", release, root)
        except native.EvidenceError as error:
            raise PortabilityError("exact native release evidence is inconsistent") from error
        return "native-qualification"
    if execution_class != "github-hosted":
        raise PortabilityError("platform execution class is invalid")
    if version == profile["version"]:
        return "hosted-portability"
    rolling = ROLLING_VERSION.fullmatch(version)
    if rolling is None or int(rolling.group(1)) <= 4:
        raise PortabilityError("hosted distribution release is malformed or outside the rolling family")
    return "hosted-portability"


def _source(source: Path, base_commit: str) -> tuple[str, str]:
    try:
        return cast(tuple[str, str], native.source_identity(source, base_commit))
    except native.EvidenceError as error:
        raise PortabilityError("source identity is not immutable") from error


def _run(slot: str, argv: Sequence[str], source: Path) -> dict[str, Any]:
    try:
        return cast(dict[str, Any], native.run_check(argv, source))
    except native.EvidenceError as error:
        raise PortabilityError(f"hosted portability {slot} check did not pass") from error


def _check_projection(argv: Sequence[str], output: bytes, status: str) -> dict[str, Any]:
    """Project only fixed status and bounded digests, never subprocess text."""
    return {
        "argv_sha256": "sha256:"
        + hashlib.sha256(
            json.dumps(list(argv), separators=(",", ":"), ensure_ascii=True).encode()
        ).hexdigest(),
        "output_bytes": len(output),
        "output_sha256": "sha256:" + hashlib.sha256(output).hexdigest(),
        "status": status,
    }


def _run_sandbox(argv: Sequence[str], source: Path, timeout: int = 900) -> dict[str, Any]:
    """Run the sandbox checks and classify only their fixed unavailable marker."""
    if not argv or any(not isinstance(item, str) or not item for item in argv):
        raise PortabilityError("hosted portability sandbox check did not pass")
    try:
        process = subprocess.Popen(
            list(argv), cwd=source, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT, close_fds=True, start_new_session=True,
        )
    except OSError as error:
        raise PortabilityError("hosted portability sandbox check did not pass") from error
    assert process.stdout is not None
    output = bytearray()
    deadline = time.monotonic() + timeout
    selector = selectors.DefaultSelector()
    selector.register(process.stdout, selectors.EVENT_READ)
    failed = False
    try:
        while selector.get_map():
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                failed = True
                break
            events = selector.select(min(remaining, 1.0))
            if not events and process.poll() is not None:
                events = [(selector.get_key(process.stdout), selectors.EVENT_READ)]
            for key, _ in events:
                chunk = os.read(key.fd, 65536)
                if not chunk:
                    selector.unregister(key.fileobj)
                    continue
                output.extend(chunk)
                if len(output) > native.MAX_OUTPUT_BYTES:
                    failed = True
                    break
            if failed:
                break
        if failed:
            os.killpg(process.pid, signal.SIGKILL)
        returncode = process.wait(timeout=10)
    except (OSError, subprocess.TimeoutExpired) as error:
        try:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=10)
        except (OSError, subprocess.TimeoutExpired):
            pass
        raise PortabilityError("hosted portability sandbox check did not pass") from error
    finally:
        selector.close()
        process.stdout.close()
    if failed or returncode != 0:
        raise PortabilityError("hosted portability sandbox check did not pass")
    if SANDBOX_UNAVAILABLE in output:
        result = _check_projection(argv, bytes(output), "unavailable")
        result["limitation"] = SANDBOX_LIMITATION
        return result
    return _check_projection(argv, bytes(output), "passed")


def collect(
    runner_label: str,
    expected_architecture: str,
    run_id: str,
    source: Path,
    base_commit: str,
    checks: list[tuple[str, list[str]]],
    execution_class: str,
    root: Path = Path("/"),
) -> dict[str, Any]:
    """Run the closed functional set and return a privacy-safe projection."""
    if runner_label != "ubuntu-24.04":
        raise PortabilityError("runner label is not supported")
    if not native.RUN_ID.fullmatch(run_id):
        raise PortabilityError("run ID is empty, unsafe, or oversized")
    if expected_architecture != "x86_64" or platform.machine() != expected_architecture:
        raise PortabilityError("hosted architecture is not exact x86_64")
    try:
        release = native.parse_os_release(native.read_bounded(root / "etc/os-release"))
    except (OSError, native.EvidenceError) as error:
        raise PortabilityError("hosted release evidence is unavailable") from error
    if release_route(release, execution_class, root) != "hosted-portability":
        raise PortabilityError("trusted native execution requires the native qualification route")
    if [name for name, _ in checks] != ["process", "metrics", "sandbox"]:
        raise PortabilityError("checks must be exactly process, metrics, and sandbox")
    if len({name for name, _ in checks}) != len(checks):
        raise PortabilityError("hosted check names must be unique")
    commit, tree = _source(source, base_commit)
    results = {
        name: (_run_sandbox(argv, source) if name == "sandbox" else _run(name, argv, source))
        for name, argv in checks
    }
    if _source(source, base_commit) != (commit, tree):
        raise PortabilityError("source identity changed while checks ran")
    return {
        "format_version": 1,
        "kind": "hosted-portability",
        "qualification": (
            "functional-portability-partial"
            if results["sandbox"]["status"] == "unavailable"
            else "functional-portability-only"
        ),
        "performance_baseline": False,
        "runner_label": runner_label,
        "run_id": run_id,
        "source": {"base_commit": base_commit, "commit": commit, "tree": tree},
        "observed": {
            "architecture": expected_architecture,
            "distribution": {
                "id": release["ID"],
                "version_id": release["VERSION_ID"],
                "version": release["VERSION"],
                "exact_native_release": False,
            },
        },
        "checks": results,
        "limitations": LIMITATIONS
        + ([SANDBOX_LIMITATION] if results["sandbox"]["status"] == "unavailable" else []),
    }


def _read_release(root: Path) -> dict[str, str]:
    try:
        return cast(
            dict[str, str],
            native.parse_os_release(native.read_bounded(root / "etc/os-release")),
        )
    except (OSError, native.EvidenceError) as error:
        raise PortabilityError("hosted release evidence is unavailable") from error


def main() -> int:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)
    route = subparsers.add_parser("route")
    route.add_argument("--root", type=Path, default=Path("/"))
    route.add_argument(
        "--execution-class", choices=("github-hosted", "trusted-native"), required=True
    )
    emit = subparsers.add_parser("collect")
    emit.add_argument("--runner-label", required=True)
    emit.add_argument("--architecture", required=True)
    emit.add_argument("--run-id", required=True)
    emit.add_argument("--source", type=Path, required=True)
    emit.add_argument("--base-commit", required=True)
    emit.add_argument("--output", type=Path, required=True)
    emit.add_argument("--output-root", type=Path, required=True)
    emit.add_argument("--root", type=Path, default=Path("/"))
    emit.add_argument(
        "--execution-class", choices=("github-hosted", "trusted-native"), required=True
    )
    emit.add_argument("--check", action="append", type=native.parse_check, required=True)
    validate = subparsers.add_parser("validate-artifact")
    validate.add_argument("--route", required=True)
    validate.add_argument("--native-file", type=Path, required=True)
    validate.add_argument("--hosted-file", type=Path, required=True)
    validate.add_argument("--output-root", type=Path, required=True)
    validate.add_argument("--source", type=Path, required=True)
    args = parser.parse_args()
    try:
        if args.command == "route":
            print(release_route(_read_release(args.root), args.execution_class, args.root))
            return 0
        if args.command == "validate-artifact":
            print(
                validate_artifact(
                    args.route,
                    args.native_file,
                    args.hosted_file,
                    args.output_root.absolute(),
                    args.source.absolute(),
                )
            )
            return 0
        report = collect(
            args.runner_label,
            args.architecture,
            args.run_id,
            args.source.resolve(strict=True),
            args.base_commit,
            args.check,
            args.execution_class,
            args.root.resolve(strict=True),
        )
        encoded = (json.dumps(report, indent=2, sort_keys=True) + "\n").encode()
        if len(encoded) > MAX_EVIDENCE_BYTES:
            raise PortabilityError("hosted evidence exceeds byte limit")
        native.write_atomic(args.output, report, args.output_root.resolve(strict=True))
    except PortabilityError as error:
        print(f"ERROR: {error}")
        return 1
    except native.EvidenceError:
        print("ERROR: hosted evidence could not be written safely")
        return 1
    print("wrote non-qualification hosted portability evidence")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
