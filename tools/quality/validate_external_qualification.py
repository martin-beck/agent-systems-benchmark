#!/usr/bin/env python3
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Validate opt-in native qualification evidence without acquiring artifacts."""

import argparse
import hashlib
import json
import re
import sys
from pathlib import Path
from typing import Any

SHA = re.compile(r"^[0-9a-f]{40}$")
SHA256 = re.compile(r"^[0-9a-f]{64}$")

TOP_KEYS = {"schema_version", "workload", "status", "source", "dataset", "evaluator", "adaptation", "contamination", "reset", "platform", "oracle"}
SOURCE_KEYS = {"repository", "commit", "archive_sha256", "license", "license_sha256"}
DATASET_KEYS = {"repository", "revision", "split", "window", "license", "license_sha256"}
EVALUATOR_KEYS = {"repository", "revision", "image_digest", "sbom_sha256", "license", "license_sha256"}
EVIDENCE_KEYS = {"status", "evidence"}


def fail(message: str) -> None:
    raise ValueError(f"external qualification: {message}")


def _keys(value: Any, allowed: set[str], label: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        fail(f"{label} must be an object")
    unknown = set(value) - allowed
    if unknown:
        fail(f"{label} has unknown fields: {sorted(unknown)}")
    return value


def _digest(value: Any, label: str) -> None:
    if not isinstance(value, str) or not SHA256.fullmatch(value):
        fail(f"{label} must be a lowercase SHA-256 digest")


def _image_digest(value: Any, label: str) -> None:
    if not isinstance(value, str) or not re.fullmatch(r"sha256:[0-9a-f]{64}", value):
        fail(f"{label} must be a sha256:<64 lowercase hex> digest")


def _revision(value: Any, label: str) -> None:
    if not isinstance(value, str) or not SHA.fullmatch(value):
        fail(f"{label} must be an immutable 40-character commit")


def _evidence(value: Any, label: str) -> None:
    item = _keys(value, EVIDENCE_KEYS, label)
    if item.get("status") != "verified" or not isinstance(item.get("evidence"), str) or not item["evidence"]:
        fail(f"{label} must contain verified evidence")


def validate_document(document: Any) -> dict[str, Any]:
    root = _keys(document, TOP_KEYS, "document")
    if root.get("schema_version") != 1 or root.get("status") != "qualified":
        fail("schema_version 1 and status qualified are required")
    if not isinstance(root.get("workload"), str) or not root["workload"]:
        fail("workload identifier is required")

    source = _keys(root.get("source"), SOURCE_KEYS, "source")
    dataset = _keys(root.get("dataset"), DATASET_KEYS, "dataset")
    evaluator = _keys(root.get("evaluator"), EVALUATOR_KEYS, "evaluator")
    for label, value in (("source.commit", source.get("commit")), ("dataset.revision", dataset.get("revision")), ("evaluator.revision", evaluator.get("revision"))):
        _revision(value, label)
    for label, value in (("source.archive_sha256", source.get("archive_sha256")), ("source.license_sha256", source.get("license_sha256")), ("dataset.license_sha256", dataset.get("license_sha256")), ("evaluator.sbom_sha256", evaluator.get("sbom_sha256")), ("evaluator.license_sha256", evaluator.get("license_sha256"))):
        _digest(value, label)
    _image_digest(evaluator.get("image_digest"), "evaluator.image_digest")
    for label, value in (("source.license", source.get("license")), ("dataset.license", dataset.get("license")), ("evaluator.license", evaluator.get("license")), ("dataset.split", dataset.get("split")), ("dataset.window", dataset.get("window"))):
        if not isinstance(value, str) or not value:
            fail(f"{label} is required")
    for label, value in (("source.repository", source.get("repository")), ("dataset.repository", dataset.get("repository")), ("evaluator.repository", evaluator.get("repository"))):
        if not isinstance(value, str) or not value.startswith("https://"):
            fail(f"{label} must be an HTTPS repository")
    _evidence(root.get("adaptation"), "adaptation")
    _evidence(root.get("contamination"), "contamination")
    _evidence(root.get("reset"), "reset")
    platform = _keys(root.get("platform"), EVIDENCE_KEYS | {"runner"}, "platform")
    if platform.get("runner") != "linux-x86_64" or platform.get("status") != "verified":
        fail("platform must be a verified native linux-x86_64 cell")
    if not isinstance(platform.get("evidence"), str) or not platform["evidence"]:
        fail("platform must contain verified evidence")
    oracle = _keys(root.get("oracle"), {"id", "revision", "evidence"}, "oracle")
    if not isinstance(oracle.get("id"), str) or not oracle["id"]:
        fail("oracle id is required")
    _revision(oracle.get("revision"), "oracle.revision")
    if not isinstance(oracle.get("evidence"), str) or not oracle["evidence"]:
        fail("oracle evidence is required")
    return root


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("qualification", type=Path)
    args = parser.parse_args()
    try:
        document = json.loads(args.qualification.read_text(encoding="utf-8"))
        validated = validate_document(document)
    except (OSError, json.JSONDecodeError, ValueError) as exc:
        print(str(exc), file=sys.stderr)
        return 2
    print(json.dumps({"schema_version": 1, "status": "qualified", "workload": validated["workload"], "evidence_sha256": hashlib.sha256(args.qualification.read_bytes()).hexdigest()}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
