# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Fail-closed repository-wide contract registry and conformance runner."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path
from typing import Any

MAX_JSON_BYTES = 1024 * 1024
MAX_CONTRACTS = 128
MAX_FIXTURES_PER_CONTRACT = 128
IDENTIFIER = re.compile(r"[a-z0-9][a-z0-9.-]{0,127}\Z")
CAPABILITY_NAME = re.compile(r"[a-z][a-z0-9_]{0,63}\Z")
SCHEMA_ROOTS = (
    "crates/asb-cli/schema/v1",
    "crates/asb-protocol/schema/v1",
    "crates/asb-control/schema/v1",
    "crates/asb-control/schema/v1.2",
    "crates/asb-control/schema/v1.3",
    "crates/asb-replay/schema/v1",
    "crates/asb-bundle/schema/v1",
    "crates/asb-workloads/registry/v1",
)
REQUIRED_COMMANDS = (
    ("cargo", "test", "-p", "asb-cli", "--test", "capability_contract", "--locked"),
    ("cargo", "test", "-p", "asb-protocol", "--test", "schema_conformance", "--locked"),
    ("cargo", "test", "-p", "asb-control", "--test", "schema_conformance", "--locked"),
    ("cargo", "test", "-p", "asb-replay", "--test", "schema_conformance", "--locked"),
    ("cargo", "test", "-p", "asb-bundle", "--test", "schema_conformance", "--locked"),
    ("cargo", "test", "-p", "asb-workloads", "--test", "validity_registry", "--locked"),
    ("cargo", "test", "-p", "asb-control", "--test", "control", "--locked"),
)
CAPABILITY_SNAPSHOT = "crates/asb-protocol/fixtures/v1/provider-capabilities.json"


class ContractError(RuntimeError):
    """A contract registry invariant failed."""


def read_json(path: Path) -> Any:
    with path.open("rb") as source:
        data = source.read(MAX_JSON_BYTES + 1)
    if len(data) > MAX_JSON_BYTES:
        raise ContractError("oversized JSON")
    try:
        return json.loads(data)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ContractError("invalid JSON") from error


def relative_path(root: Path, value: object) -> Path:
    if not isinstance(value, str) or not value or "\x00" in value:
        raise ContractError("invalid relative path")
    path = Path(value)
    if path.is_absolute() or ".." in path.parts or path.as_posix() != value:
        raise ContractError("unsafe relative path")
    resolved = (root / path).resolve()
    if not resolved.is_relative_to(root.resolve()):
        raise ContractError("path escapes repository")
    return resolved


def validate_capability_snapshot(snapshot: object) -> None:
    expected_capability_fields = {
        "minimum_version",
        "maximum_version",
        "providers",
        "endpoint_classes",
        "credential_sources",
        "settings",
        "transport_ceiling",
    }
    if not isinstance(snapshot, dict) or set(snapshot) != expected_capability_fields:
        raise ContractError("capability snapshot is not closed")
    for version_name in ("minimum_version", "maximum_version"):
        version = snapshot[version_name]
        if (
            not isinstance(version, dict)
            or set(version) != {"major", "minor"}
            or not all(
                isinstance(value, int) and not isinstance(value, bool) and value >= 0
                for value in version.values()
            )
        ):
            raise ContractError("capability version is not closed")
    for list_name in ("providers", "endpoint_classes", "credential_sources"):
        values = snapshot[list_name]
        if (
            not isinstance(values, list)
            or not values
            or len(values) > 64
            or any(
                not isinstance(value, str) or not CAPABILITY_NAME.fullmatch(value)
                for value in values
            )
        ):
            raise ContractError("capability vocabulary is not bounded")
    settings = snapshot["settings"]
    if not isinstance(settings, dict) or not settings or len(settings) > 64:
        raise ContractError("capability settings are not bounded")
    for name, support in settings.items():
        if (
            not isinstance(name, str)
            or not CAPABILITY_NAME.fullmatch(name)
            or not isinstance(support, dict)
            or set(support) != {"exact_value", "explicit_omission"}
            or any(not isinstance(value, bool) for value in support.values())
        ):
            raise ContractError("capability setting support is not closed")
    transport = snapshot["transport_ceiling"]
    if (
        not isinstance(transport, dict)
        or not transport
        or len(transport) > 64
        or any(
            not isinstance(name, str)
            or not CAPABILITY_NAME.fullmatch(name)
            or not isinstance(value, int)
            or isinstance(value, bool)
            or value <= 0
            for name, value in transport.items()
        )
    ):
        raise ContractError("transport capability bounds are not closed")


def load_catalog(root: Path, catalog_path: Path) -> dict[str, Any]:
    raw = read_json(catalog_path)
    expected = {
        "schema_version",
        "contracts",
        "conformance_commands",
        "generated_document",
        "capability_snapshot",
    }
    if not isinstance(raw, dict) or set(raw) != expected:
        raise ContractError("catalog shape is not closed")
    if (
        raw["schema_version"] != 1
        or not isinstance(raw["contracts"], list)
        or not raw["contracts"]
        or len(raw["contracts"]) > MAX_CONTRACTS
    ):
        raise ContractError("unsupported catalog version")
    ids: set[str] = set()
    schemas: set[str] = set()
    for entry in raw["contracts"]:
        fields = {"id", "schema", "fixtures", "rust_test"}
        if not isinstance(entry, dict) or set(entry) != fields:
            raise ContractError("contract entry shape is not closed")
        identifier = entry["id"]
        if not isinstance(identifier, str) or not IDENTIFIER.fullmatch(identifier):
            raise ContractError("invalid contract identifier")
        if identifier in ids:
            raise ContractError("duplicate contract identifier")
        ids.add(identifier)
        schema = relative_path(root, entry["schema"])
        schema_name = schema.relative_to(root).as_posix()
        if schema_name in schemas:
            raise ContractError("duplicate schema registration")
        schemas.add(schema_name)
        read_json(schema)
        fixtures = entry["fixtures"]
        if (
            not isinstance(fixtures, list)
            or not fixtures
            or len(fixtures) > MAX_FIXTURES_PER_CONTRACT
            or any(not isinstance(fixture, str) for fixture in fixtures)
            or len(fixtures) != len(set(fixtures))
        ):
            raise ContractError("missing positive fixture")
        for fixture in fixtures:
            read_json(relative_path(root, fixture))
        rust_test = entry["rust_test"]
        if not isinstance(rust_test, str) or "/" not in rust_test:
            raise ContractError("missing Rust round-trip gate")
    discovered = {
        path.relative_to(root).as_posix()
        for directory in SCHEMA_ROOTS
        for path in (root / directory).glob("*.schema.json")
    }
    if schemas != discovered:
        raise ContractError(
            f"schema registry mismatch: missing={sorted(discovered - schemas)}, "
            f"extra={sorted(schemas - discovered)}"
        )
    commands = raw["conformance_commands"]
    if commands != [list(command) for command in REQUIRED_COMMANDS]:
        raise ContractError("conformance commands are not the closed allowlist")
    registered_tests = {entry["rust_test"] for entry in raw["contracts"]}
    executed_tests = {f"{command[3]}/{command[5]}" for command in REQUIRED_COMMANDS[:-1]}
    if registered_tests != executed_tests:
        raise ContractError("Rust round-trip registrations and commands differ")
    relative_path(root, raw["generated_document"])
    if raw["capability_snapshot"] != CAPABILITY_SNAPSHOT:
        raise ContractError("capability snapshot is not the canonical fixture")
    snapshot = read_json(relative_path(root, CAPABILITY_SNAPSHOT))
    validate_capability_snapshot(snapshot)
    if CAPABILITY_SNAPSHOT not in {
        fixture for entry in raw["contracts"] for fixture in entry["fixtures"]
    }:
        raise ContractError("capability snapshot is not schema-enrolled")
    return raw


def render(catalog: dict[str, Any]) -> str:
    lines = [
        "# Contract catalog",
        "",
        "This file is generated by tools/quality/contract_consistency.py.",
        "",
        "| Contract | Canonical schema | Positive examples | Rust round-trip gate |",
        "| --- | --- | ---: | --- |",
    ]
    for entry in sorted(catalog["contracts"], key=lambda item: item["id"]):
        lines.append(
            f"| {entry['id']} | {entry['schema']} | {len(entry['fixtures'])} | "
            f"{entry['rust_test']} |"
        )
    lines.extend(
        [
            "",
            "## Provider capability support",
            "",
            "This table is generated from the schema-validated provider capability fixture.",
            "",
            "| Setting | Exact value | Explicit omission |",
            "| --- | --- | --- |",
        ]
    )
    capabilities = catalog["_capabilities"]
    for name, support in sorted(capabilities["settings"].items()):
        exact = "yes" if support["exact_value"] else "no"
        omission = "yes" if support["explicit_omission"] else "no"
        lines.append(f"| {name} | {exact} | {omission} |")
    lines.extend(
        [
            "",
            f"Providers: {', '.join(capabilities['providers'])}.",
            f"Endpoint classes: {', '.join(capabilities['endpoint_classes'])}.",
            f"Credential sources: {', '.join(capabilities['credential_sources'])}.",
            "",
            "The registry is closed. Unregistered schemas, duplicate identifiers, unsafe paths,",
            "malformed or oversized JSON, missing examples, and missing Rust gates fail validation.",
            "A checked-in v1 schema is immutable relative to the integration base; changed contracts",
            "must use a new versioned schema directory instead of silently breaking old consumers.",
            "Stateful control conformance covers negotiation, bounds, cancellation, and errors.",
            "",
        ]
    )
    return "\n".join(lines)


def ensure_v1_compatibility(
    root: Path, catalog: dict[str, Any], baseline_ref: str | None
) -> None:
    if baseline_ref is None:
        return
    if not re.fullmatch(r"[0-9a-f]{40}", baseline_ref):
        raise ContractError("baseline ref must be an exact commit")
    try:
        subprocess.run(
            ["git", "cat-file", "-e", f"{baseline_ref}^{{commit}}"],
            cwd=root,
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
    except subprocess.CalledProcessError as error:
        raise ContractError("baseline commit is unavailable") from error
    for entry in catalog["contracts"]:
        schema = entry["schema"]
        present = subprocess.run(
            ["git", "cat-file", "-e", f"{baseline_ref}:{schema}"],
            cwd=root,
            check=False,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        if present.returncode != 0:
            continue
        prior_bytes = subprocess.run(
            ["git", "show", f"{baseline_ref}:{schema}"],
            cwd=root,
            check=True,
            stdout=subprocess.PIPE,
        ).stdout
        try:
            prior = json.loads(prior_bytes)
            current = read_json(relative_path(root, schema))
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            raise ContractError("baseline schema is invalid JSON") from error
        if prior != current:
            raise ContractError(f"in-place v1 schema change: {schema}")


def check(
    root: Path,
    catalog_path: Path,
    run_tests: bool,
    write: bool = False,
    baseline_ref: str | None = None,
) -> None:
    catalog = load_catalog(root, catalog_path)
    catalog["_capabilities"] = read_json(
        relative_path(root, catalog["capability_snapshot"])
    )
    document = relative_path(root, catalog["generated_document"])
    expected = render(catalog)
    if write:
        document.parent.mkdir(parents=True, exist_ok=True)
        document.write_text(expected, encoding="utf-8")
    elif document.read_text(encoding="utf-8") != expected:
        raise ContractError("generated contract catalog differs")
    ensure_v1_compatibility(root, catalog, baseline_ref)
    if run_tests:
        for command in catalog["conformance_commands"]:
            subprocess.run(command, cwd=root, check=True)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--root", type=Path, default=Path(__file__).resolve().parents[2]
    )
    parser.add_argument("--catalog", type=Path)
    parser.add_argument("--run-tests", action="store_true")
    parser.add_argument("--write", action="store_true")
    parser.add_argument("--baseline-ref")
    args = parser.parse_args()
    root = args.root.resolve()
    catalog_path = args.catalog or root / "contracts/v1/catalog.json"
    try:
        check(root, catalog_path, args.run_tests, args.write, args.baseline_ref)
    except (ContractError, OSError, subprocess.CalledProcessError) as error:
        print(f"contract consistency failed: {error}", file=sys.stderr)
        return 1
    print("contract consistency: all registered artifacts agree")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
