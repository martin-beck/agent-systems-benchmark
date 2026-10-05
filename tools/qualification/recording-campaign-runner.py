#!/usr/bin/env python3
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Run the deterministic selected/all-agent recording acceptance fixture.

This runner deliberately invokes only ``record-campaign --local-mock``.  It
never enables ``benchmark-live`` and never needs a provider credential.  The
generated captures exercise the complete selected-agent x current-workload
matrix and the CLI's atomic cassette publication boundary.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
from pathlib import Path


AGENTS = (
    "opencode",
    "opendesk",
    "aider",
    "codex",
    "gemini",
    "qwen_code",
    "goose",
    "mini_swe",
    "openhands",
)
SCHEMA_VERSION = 1
PROFILE_SHA256 = "a" * 64


def run_json(asb: Path, *args: str) -> dict:
    completed = subprocess.run(
        [str(asb), *args, "--json"],
        check=True,
        capture_output=True,
        text=True,
    )
    return json.loads(completed.stdout)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--asb", type=Path, required=True, help="development ASB executable")
    parser.add_argument("--fixture", type=Path, required=True, help="buffered cassette fixture")
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--agents", default=", ".join(AGENTS))
    parser.add_argument("--workloads", help="comma-separated IDs; omitted means all available")
    args = parser.parse_args()

    agents = [value.strip() for value in args.agents.split(",") if value.strip()]
    unknown = sorted(set(agents) - set(AGENTS))
    if not agents or unknown or len(agents) > len(AGENTS):
        parser.error(f"unknown or empty agent selection: {unknown or agents}")
    catalog = run_json(args.asb, "workload-catalog")
    available = sorted(
        entry["id"]
        for entry in catalog["entries"]
        if entry.get("platform", "").startswith("linux-x86_64:")
        and entry.get("availability") in {"fixture_only", "available"}
    )
    workloads = [
        value.strip() for value in args.workloads.split(",") if value.strip()
    ] if args.workloads else available
    if not workloads or any(value not in available for value in workloads):
        parser.error("workload selection contains unavailable or no fixture workloads")
    workloads = sorted(set(workloads))

    cassette = json.loads(args.fixture.read_text(encoding="utf-8"))
    contents = cassette.get("contents")
    if not isinstance(contents, dict):
        raise SystemExit("fixture does not contain cassette contents")
    output_dir = args.output_dir.resolve()
    capture_dir = output_dir / "captures"
    cassette_dir = output_dir / "cassettes"
    capture_dir.mkdir(parents=True, exist_ok=True)
    cassette_dir.mkdir(parents=True, exist_ok=True)
    entries = []
    for agent in agents:
        capture = {
            "schema_version": SCHEMA_VERSION,
            "provider_profile_sha256": PROFILE_SHA256,
            "agent_id": agent,
            "network": "loopback_only",
            "estimated_cost_minor": 0,
            "confirmation": {"record": True, "network": True, "cost": False},
            "contents": contents,
        }
        capture_path = capture_dir / f"{agent}.json"
        capture_path.write_text(json.dumps(capture, separators=(",", ":")), encoding="utf-8")
        for workload in workloads:
            token = hashlib.sha256(workload.encode()).hexdigest()[:12]
            entries.append(
                {
                    "workload_id": workload,
                    "capture_path": str(capture_path),
                    "cassette_path": str(cassette_dir / f"{agent}-{token}.json"),
                }
            )
    manifest = {
        "schema_version": SCHEMA_VERSION,
        "provider_profile_sha256": PROFILE_SHA256,
        "agent_ids": agents,
        "workload_ids": workloads,
        "entries": entries,
    }
    manifest_path = output_dir / "campaign.json"
    manifest_path.write_text(json.dumps(manifest, separators=(",", ":")), encoding="utf-8")
    result = subprocess.run(
        [str(args.asb), "--json", "record-campaign", str(manifest_path), "--local-mock"],
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        sys.stderr.write(result.stderr)
        sys.stderr.write(result.stdout)
        return result.returncode
    output = json.loads(result.stdout)
    expected = len(agents) * len(workloads)
    published = list(cassette_dir.glob("*.json"))
    if output.get("tuple_count") != expected or not output.get("offline_ready") or len(published) != expected:
        raise SystemExit("recording campaign did not publish an exact offline-ready matrix")
    print(json.dumps(
        {"ok": True, "mode": "development_local_mock", "agents": agents,
         "workloads": workloads, "tuple_count": expected, "offline_ready": True},
        separators=(",", ":"),
    ))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
