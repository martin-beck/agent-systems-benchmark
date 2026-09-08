#!/usr/bin/env python3
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Apply the manifest-defined workspace and critical-crate coverage floors."""

from __future__ import annotations

import json
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def run_coverage(arguments: list[str], floor: int) -> None:
    subprocess.run(
        [
            "cargo",
            "llvm-cov",
            "--locked",
            *arguments,
            "--all-targets",
            "--fail-under-lines",
            str(floor),
        ],
        cwd=ROOT,
        check=True,
    )


def main() -> int:
    manifest = json.loads(
        (ROOT / "config/quality-tools.json").read_text(encoding="utf-8")
    )
    coverage = manifest["coverage"]
    metadata = json.loads(
        subprocess.check_output(
            ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"],
            cwd=ROOT,
            text=True,
        )
    )
    workspace_packages = {
        package["name"]
        for package in metadata["packages"]
        if package["id"] in metadata["workspace_members"]
    }

    run_coverage(["--workspace"], coverage["workspace_lines"])
    for package in coverage["critical_packages"]:
        if package in workspace_packages:
            run_coverage(["--package", package], coverage["critical_lines"])
        else:
            print(f"critical coverage deferred until package exists: {package}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
