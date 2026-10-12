#!/usr/bin/env python3
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Fail-closed checks for the CLI status/progress output contract.

This is intentionally a small source/inventory check rather than a second
hand-maintained command list.  The public command inventory in ``human.rs``
and the generated route fixture are the two contract inputs; the check makes
drift visible before a command can merge.  Behavioural details (clock,
terminal, writer and cancellation cases) remain covered by the Rust tests.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path


def fail(message: str) -> None:
    raise ValueError(message)


def inventory(root: Path) -> set[str]:
    source = (root / "crates/asb-cli/src/human.rs").read_text(encoding="utf-8")
    match = re.search(
        r"pub\(crate\) const PUBLIC_COMMANDS: &\[&str\] = &\[(.*?)\];",
        source,
        re.DOTALL,
    )
    if not match:
        fail("PUBLIC_COMMANDS inventory is missing or no longer parseable")
    return set(re.findall(r'"([a-z][a-z0-9-]*)"', match.group(1)))


def routes(root: Path) -> list[dict[str, object]]:
    path = root / "crates/asb-cli/fixtures/human/command-surface-v1.json"
    try:
        document = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        fail(f"cannot read command surface fixture: {exc}")
    if document.get("schema_version") != 1:
        fail("command surface fixture has an unsupported schema version")
    values = document.get("routes")
    if not isinstance(values, list) or not values:
        fail("command surface fixture has no routes")
    return values


def check(root: Path) -> list[str]:
    public = inventory(root)
    entries = routes(root)
    route_roots = {str(entry.get("path", "")).split()[0] for entry in entries}
    missing = sorted(public - route_roots)
    if missing:
        fail("public commands missing from generated route inventory: " + ", ".join(missing))
    unknown = sorted(route_roots - public)
    if unknown:
        fail("generated route inventory contains unknown public commands: " + ", ".join(unknown))

    for entry in entries:
        path = entry.get("path")
        if not isinstance(path, str) or not path:
            fail("route has no stable path")
        if not isinstance(entry.get("human"), bool) or not isinstance(entry.get("json"), bool):
            fail(f"{path}: route must declare human and json output support")
        if not entry["human"] and not entry["json"]:
            fail(f"{path}: route cannot disable both output modes")
        # Shell completion is a deliberately raw text route; every other
        # public route must have a machine-readable contract.
        if path != "completion bash" and not entry["json"]:
            fail(f"{path}: eligible route has no JSON output contract")

    progress = (root / "crates/asb-cli/src/step_progress.rs").read_text(encoding="utf-8")
    for required in ("DELAY", "REFRESH", "StepStatus", "StepReporter", "terminal"):
        if required not in progress:
            fail(f"shared progress reporter is missing {required}")
    for token in ("Success", "Failure", "Cancelled", "Waiting"):
        if not re.search(rf"StepStatus::?\s*\{{?[^}}]*\b{token}\b|\b{token}\b", progress):
            fail(f"shared progress reporter has no {token} terminal state")

    # ANSI redraw and carriage-return bytes belong exclusively to the shared
    # renderer.  This catches a new direct progress writer without trying to
    # infer intent from arbitrary writeln! call sites.
    for source_path in (root / "crates/asb-cli/src").rglob("*.rs"):
        if source_path.name == "step_progress.rs":
            continue
        text = source_path.read_text(encoding="utf-8")
        if "\\x1b[" in text or 'write!(self.writer, "\\r' in text:
            fail(f"{source_path.relative_to(root)} bypasses the shared progress renderer")

    return [f"checked {len(public)} public command families and {len(entries)} routes"]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[2])
    args = parser.parse_args()
    try:
        for message in check(args.root):
            print(message)
    except ValueError as exc:
        print(f"status/progress completeness: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
