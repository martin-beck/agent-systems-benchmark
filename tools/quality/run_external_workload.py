#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Run a qualified external evaluator with bounded, shell-free process effects."""

import argparse
import json
import subprocess
from pathlib import Path


DEFAULT_REGISTRY = Path(__file__).parents[2] / "crates/asb-workloads/registry/v1/external-workloads.json"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("workload_id")
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--timeout-seconds", type=float, default=60.0)
    parser.add_argument("--registry", type=Path, default=DEFAULT_REGISTRY)
    args, command = parser.parse_known_args()
    if not command or command[0] != "--":
        raise SystemExit("an evaluator command is required after --")
    if args.timeout_seconds <= 0 or args.timeout_seconds > 3600:
        raise SystemExit("timeout must be between 0 and 3600 seconds")
    records = [item for item in json.loads(args.registry.read_text())["workloads"] if item.get("id") == args.workload_id]
    if len(records) != 1:
        raise SystemExit("unknown or duplicate external workload")
    evaluator = records[0]["evaluator"]
    provenance = evaluator.get("provenance", {})
    if provenance.get("status") != "qualified" or not all((evaluator.get("image_digest"), provenance.get("sbom_sha256"), provenance.get("evidence"))):
        raise SystemExit("external evaluator is not qualified; no subprocess started")
    root = args.root.resolve()
    if not root.is_dir():
        raise SystemExit("materialized workload root is absent")
    try:
        completed = subprocess.run(command[1:], cwd=root, shell=False, check=False, timeout=args.timeout_seconds)
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise SystemExit(f"external evaluator failed boundedly: {exc}") from exc
    print(json.dumps({"workload": args.workload_id, "exit_code": completed.returncode}, sort_keys=True))
    return completed.returncode


if __name__ == "__main__":
    raise SystemExit(main())
