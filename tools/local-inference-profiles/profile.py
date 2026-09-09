# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Fail-closed validator for local inference profile evidence."""
from __future__ import annotations

import argparse
import hashlib
import json
import re
from pathlib import Path
from typing import Any

MAX_BYTES = 1_048_576
HEX40 = re.compile(r"^[0-9a-f]{40}$")
HEX64 = re.compile(r"^[0-9a-f]{64}$")
IDENTITY = re.compile(r"^[a-z][a-z0-9._-]{0,63}$")
LICENSES = {"Apache-2.0", "MIT", "BSD-3-Clause"}
ROUTES = {"openai_chat", "openai_responses", "anthropic_messages"}
STATUSES = {"qualified", "unqualified"}
PRIVATE_MARKERS = (
    "/home/",
    "/private/",
    "BEGIN OPENSSH",
    "BEGIN RSA",
    "ghp_",
    "sk-",
)


class ProfileError(ValueError):
    """A profile manifest is malformed, private, or overclaims evidence."""


def _require_text(value: Any, field: str, pattern: re.Pattern[str]) -> None:
    if not isinstance(value, str) or not pattern.fullmatch(value):
        raise ProfileError(f"invalid {field}")


def _walk(value: Any) -> None:
    if isinstance(value, dict):
        if len(value) > 32:
            raise ProfileError("object has too many fields")
        for child in value.values():
            _walk(child)
    elif isinstance(value, list):
        if len(value) > 64:
            raise ProfileError("array is unbounded")
        for child in value:
            _walk(child)
    elif isinstance(value, str):
        if len(value.encode()) > 16_384:
            raise ProfileError("string exceeds bounded field size")
        if any(marker in value for marker in PRIVATE_MARKERS):
            raise ProfileError("private or credential-like value")


def _validate_artifact(value: Any, field: str, *, allow_missing_tree: bool = False) -> None:
    if not isinstance(value, dict) or set(value) != {
        "name",
        "version",
        "source",
        "revision",
        "tree",
        "license",
    }:
        raise ProfileError(f"{field} artifact fields are not closed")
    for name in ("name", "version", "source"):
        if not isinstance(value[name], str) or not value[name]:
            raise ProfileError(f"invalid {field}.{name}")
    _require_text(value["revision"], f"{field}.revision", HEX40)
    if value["tree"] is None and allow_missing_tree:
        pass
    else:
        _require_text(value["tree"], f"{field}.tree", HEX40)
    if value["license"] not in LICENSES:
        raise ProfileError(f"unsupported {field}.license")
    if not value["source"].startswith("https://github.com/"):
        raise ProfileError(f"{field}.source is not an official HTTPS source")


def _validate_profile(value: Any) -> None:
    required = {
        "id",
        "status",
        "selectable",
        "frontend",
        "backend",
        "model",
        "routes",
        "configuration",
        "qualification",
    }
    if not isinstance(value, dict) or set(value) != required:
        raise ProfileError("profile fields are not closed")
    _require_text(value["id"], "profile.id", IDENTITY)
    if value["status"] not in STATUSES or not isinstance(value["selectable"], bool):
        raise ProfileError("invalid profile status or selection flag")
    if value["status"] == "qualified" and not value["selectable"]:
        raise ProfileError("qualified profile must be selectable")
    if value["status"] == "unqualified" and value["selectable"]:
        raise ProfileError("unqualified profile must not be selectable")
    _validate_artifact(value["frontend"], "frontend", allow_missing_tree=value["status"] == "unqualified")
    if value["backend"] is not None:
        _validate_artifact(value["backend"], "backend", allow_missing_tree=value["status"] == "unqualified")
    if value["status"] == "qualified" and value["frontend"]["tree"] is None:
        raise ProfileError("qualified profile lacks frontend tree evidence")
    model = value["model"]
    if not isinstance(model, dict) or set(model) != {"name", "sha256", "tokenizer_sha256"}:
        raise ProfileError("model fields are not closed")
    if not isinstance(model["name"], str) or not model["name"]:
        raise ProfileError("invalid model.name")
    for field in ("sha256", "tokenizer_sha256"):
        if model[field] is not None:
            _require_text(model[field], f"model.{field}", HEX64)
    if not isinstance(value["routes"], list) or not value["routes"]:
        raise ProfileError("profile routes are empty")
    if any(route not in ROUTES for route in value["routes"]):
        raise ProfileError("profile has an unsupported route")
    if value["routes"] != sorted(set(value["routes"])):
        raise ProfileError("profile routes must be sorted and unique")
    configuration = value["configuration"]
    if not isinstance(configuration, dict) or set(configuration) != {
        "template",
        "parser",
        "sampling",
        "runtime",
        "isolation",
    }:
        raise ProfileError("configuration fields are not closed")
    for field in ("template", "parser", "sampling", "runtime"):
        if not isinstance(configuration[field], str) or not configuration[field]:
            raise ProfileError(f"invalid configuration.{field}")
    if configuration["isolation"] != {
        "network": "loopback_only",
        "credentials": "stripped",
        "resource_limits": "bounded",
        "teardown": "verified",
    }:
        raise ProfileError("profile isolation policy is not exact")
    qualification = value["qualification"]
    if not isinstance(qualification, dict) or set(qualification) != {
        "hardware",
        "runtime",
        "trials",
        "variability",
        "reason",
    }:
        raise ProfileError("qualification fields are not closed")
    if not isinstance(qualification["hardware"], str) or not qualification["hardware"]:
        raise ProfileError("qualification hardware is absent")
    if not isinstance(qualification["runtime"], str) or not qualification["runtime"]:
        raise ProfileError("qualification runtime is absent")
    trials = qualification["trials"]
    if not isinstance(trials, int) or trials < 0 or trials > 10_000:
        raise ProfileError("qualification trials are invalid")
    if not isinstance(qualification["variability"], str) or not qualification["variability"]:
        raise ProfileError("qualification variability is absent")
    reason = qualification["reason"]
    if value["status"] == "unqualified":
        if not isinstance(reason, str) or not reason:
            raise ProfileError("unqualified profile needs an explicit reason")
        if model["tokenizer_sha256"] is not None:
            raise ProfileError("unqualified profile cannot claim tokenizer evidence")
    elif reason is not None or model["sha256"] is None or model["tokenizer_sha256"] is None:
        raise ProfileError("qualified profile lacks complete model evidence")


def validate(value: Any) -> dict[str, Any]:
    if not isinstance(value, dict) or set(value) != {"schema_version", "profiles"}:
        raise ProfileError("manifest fields are not closed")
    if value["schema_version"] != 1:
        raise ProfileError("unsupported manifest schema")
    profiles = value["profiles"]
    if not isinstance(profiles, list) or not profiles or len(profiles) > 8:
        raise ProfileError("profile list is empty or unbounded")
    for profile in profiles:
        _validate_profile(profile)
    ids = [profile["id"] for profile in profiles]
    if ids != sorted(set(ids)):
        raise ProfileError("profile ids must be sorted and unique")
    _walk(value)
    canonical = json.dumps(value, sort_keys=True, separators=(",", ":")).encode()
    if len(canonical) > MAX_BYTES:
        raise ProfileError("manifest exceeds 1 MiB")
    return value


def canonical_bytes(value: Any) -> bytes:
    validate(value)
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode() + b"\n"


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("manifest", type=Path)
    args = parser.parse_args()
    payload = canonical_bytes(json.loads(args.manifest.read_text(encoding="utf-8")))
    print(json.dumps({"schema_version": 1, "sha256": hashlib.sha256(payload).hexdigest(), "bytes": len(payload)}, sort_keys=True))


if __name__ == "__main__":
    main()
