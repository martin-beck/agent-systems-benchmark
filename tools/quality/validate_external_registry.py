#!/usr/bin/env python3
"""Validate the opt-in external workload provenance registry without acquiring data."""

import hashlib
import json
import re
import sys
from pathlib import Path


SHA = re.compile(r"^[0-9a-f]{40}$")
REGISTRY = Path(__file__).parents[2] / "crates/asb-workloads/registry/v1/external-workloads.json"


def fail(message: str) -> None:
    raise SystemExit(f"external registry: {message}")


def main() -> int:
    try:
        data = json.loads(REGISTRY.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        fail(f"cannot parse registry: {exc}")
    if data.get("schema_version") != 1 or not isinstance(data.get("workloads"), list):
        fail("schema_version 1 and workloads list are required")
    seen = set()
    for item in data["workloads"]:
        ident = item.get("id")
        if not isinstance(ident, str) or not ident or ident in seen:
            fail(f"duplicate or invalid workload id: {ident!r}")
        seen.add(ident)
        source = item.get("source", {})
        commits = [source.get("commit")] if source.get("commit") else [x.split("@", 1)[1] for x in source.get("repositories", [])]
        if not commits or any(not SHA.fullmatch(commit) for commit in commits):
            fail(f"{ident}: every source revision must be a 40-character lowercase commit")
        dataset = item.get("dataset", {})
        if dataset.get("vendored") is not False or dataset.get("acquisition") != "explicit-download":
            fail(f"{ident}: external datasets must be explicit-download and non-vendored")
        evaluator = item.get("evaluator", {})
        if not evaluator.get("version") or not evaluator.get("entrypoint"):
            fail(f"{ident}: evaluator identity is incomplete")
        if evaluator.get("image_digest") is not None and not str(evaluator["image_digest"]).startswith("sha256:"):
            fail(f"{ident}: image_digest must be sha256:... or null while planned")
        if not item.get("limitations"):
            fail(f"{ident}: limitations must be explicit")
    digest = hashlib.sha256(REGISTRY.read_bytes()).hexdigest()
    print(json.dumps({"schema_version": 1, "workloads": len(seen), "registry_sha256": digest}, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
