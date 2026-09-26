#!/usr/bin/env python3
"""Run cargo-deny and cargo-audit from an already provisioned tool directory."""
from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
from pathlib import Path


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--tool-manifest", type=Path, required=True)
    parser.add_argument("--bin-dir", type=Path, required=True)
    args = parser.parse_args()
    data = json.loads(args.tool_manifest.read_text(encoding="utf-8"))
    if set(data) != {"schema_version", "tools"} or data["schema_version"] != 1:
        raise SystemExit("tool manifest schema is not exact")
    for name, expected in data["tools"].items():
        if name not in {"cargo-deny", "cargo-audit"} or set(expected) != {"version", "sha256"}:
            raise SystemExit("tool manifest contains an unknown or incomplete tool")
        path = args.bin_dir / name
        if not path.is_file() or path.is_symlink():
            raise SystemExit(f"missing pinned tool: {name}")
        actual = hashlib.sha256(path.read_bytes()).hexdigest()
        if actual != expected["sha256"]:
            raise SystemExit(f"pinned tool digest mismatch: {name}")
        version = subprocess.run([str(path), "--version"], check=True, capture_output=True, text=True).stdout
        if expected["version"] not in version:
            raise SystemExit(f"pinned tool version mismatch: {name}")
    subprocess.run([str(args.bin_dir / "cargo-deny"), "check", "--locked"], check=True)
    subprocess.run([str(args.bin_dir / "cargo-audit"), "--deny", "warnings"], check=True)
    return 0


if __name__ == "__main__":
    main()
