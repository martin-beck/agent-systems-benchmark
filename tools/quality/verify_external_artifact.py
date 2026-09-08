#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Verify an explicitly acquired external artifact before execution.

This helper only reads a caller-selected file and manifest; it never downloads,
extracts, executes, or silently substitutes an artifact.
"""

import argparse
import hashlib
import json
from pathlib import Path


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("artifact_id", help="manifest artifact identity")
    args = parser.parse_args()
    document = json.loads(args.manifest.read_text(encoding="utf-8"))
    matches = [item for item in document.get("artifacts", []) if item.get("id") == args.artifact_id]
    if len(matches) != 1:
        raise SystemExit("artifact identity is missing or duplicated")
    item = matches[0]
    relative = Path(item.get("relative_path", ""))
    if not relative.parts or relative.is_absolute() or ".." in relative.parts:
        raise SystemExit("artifact path is unsafe")
    target = (args.root / relative).resolve()
    root = args.root.resolve()
    if root not in target.parents:
        raise SystemExit("artifact escaped acquisition root")
    if not target.is_file():
        raise SystemExit("acquired artifact is absent")
    actual = hashlib.sha256(target.read_bytes()).hexdigest()
    expected = item.get("sha256")
    if not isinstance(expected, str) or actual != expected:
        raise SystemExit("acquired artifact digest mismatch")
    print(json.dumps({"artifact": args.artifact_id, "sha256": actual}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
