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
import stat
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
SCHEMA_SHA256 = {
    "hosted-portability.schema.json": "93d47a8042c6c872c6854e60293ca0c56d09fdddc4ffab3e9e47e9d6ae5cc0f9",
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
    if "const" in schema and value != schema["const"]:
        raise PortabilityError("platform evidence does not match its closed schema")
    if "enum" in schema and value not in cast(list[Any], schema["enum"]):
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
        if schema.get("uniqueItems") and len({json.dumps(item, sort_keys=True) for item in value}) != len(value):
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
        selected, absent, schema_name, expected = {
            "native-qualification": (
                native_path,
                hosted_path,
                "native-evidence.schema.json",
                ("native-run", "native-functional"),
            ),
            "hosted-portability": (
                hosted_path,
                native_path,
                "hosted-portability.schema.json",
                ("hosted-portability", "functional-portability-only"),
            ),
        }[route]
        if absent.exists() or absent.is_symlink():
            raise PortabilityError("more than one platform evidence kind exists")
        report = _bounded_json(selected)
        schema = _bounded_json(
            source / "platforms/v1" / schema_name, SCHEMA_SHA256[schema_name]
        )
        _validate_schema(report, schema, schema)
        if (report.get("kind"), report.get("qualification")) != expected:
            raise PortabilityError("platform evidence route and claim differ")
        return route
    except PortabilityError:
        for candidate in candidates:
            _remove_candidate(candidate, output_root)
        raise


def release_route(release: dict[str, str], root: Path = Path("/")) -> str:
    """Select the exact-native or rolling-hosted route without conflating them."""
    profile = native.PLATFORMS["ubuntu-24.04"]
    if release.get("ID") != profile["id"] or release.get("VERSION_ID") != profile["version_id"]:
        raise PortabilityError("hosted distribution does not match the declared runner family")
    version = release.get("VERSION", "")
    if version == profile["version"]:
        try:
            native.validate_platform("ubuntu-24.04", release, root)
        except native.EvidenceError as error:
            raise PortabilityError("exact native release evidence is inconsistent") from error
        return "native-qualification"
    rolling = ROLLING_VERSION.fullmatch(version)
    if rolling is None or int(rolling.group(1)) <= 4:
        raise PortabilityError("hosted distribution release is malformed or outside the rolling family")
    return "hosted-portability"


def _source(source: Path, base_commit: str) -> tuple[str, str]:
    try:
        return cast(tuple[str, str], native.source_identity(source, base_commit))
    except native.EvidenceError as error:
        raise PortabilityError("source identity is not immutable") from error


def _run(argv: Sequence[str], source: Path) -> dict[str, Any]:
    try:
        return cast(dict[str, Any], native.run_check(argv, source))
    except native.EvidenceError as error:
        raise PortabilityError("hosted portability check did not pass") from error


def collect(
    runner_label: str,
    expected_architecture: str,
    run_id: str,
    source: Path,
    base_commit: str,
    checks: list[tuple[str, list[str]]],
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
    if release_route(release, root) != "hosted-portability":
        raise PortabilityError("exact reviewed release requires the native qualification route")
    if [name for name, _ in checks] != ["process", "metrics", "sandbox"]:
        raise PortabilityError("checks must be exactly process, metrics, and sandbox")
    if len({name for name, _ in checks}) != len(checks):
        raise PortabilityError("hosted check names must be unique")
    commit, tree = _source(source, base_commit)
    results = {name: _run(argv, source) for name, argv in checks}
    if _source(source, base_commit) != (commit, tree):
        raise PortabilityError("source identity changed while checks ran")
    return {
        "format_version": 1,
        "kind": "hosted-portability",
        "qualification": "functional-portability-only",
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
        "limitations": LIMITATIONS,
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
    emit = subparsers.add_parser("collect")
    emit.add_argument("--runner-label", required=True)
    emit.add_argument("--architecture", required=True)
    emit.add_argument("--run-id", required=True)
    emit.add_argument("--source", type=Path, required=True)
    emit.add_argument("--base-commit", required=True)
    emit.add_argument("--output", type=Path, required=True)
    emit.add_argument("--output-root", type=Path, required=True)
    emit.add_argument("--root", type=Path, default=Path("/"))
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
            print(release_route(_read_release(args.root), args.root))
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
