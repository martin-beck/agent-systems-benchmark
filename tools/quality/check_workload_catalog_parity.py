#!/usr/bin/env python3
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Check that the CLI workload inventory equals generated catalog output."""

import argparse
import json
from pathlib import Path

ROOT = Path(__file__).parents[2]
EXPECTED = ROOT / "docs/generated/workload-catalog-v1.json"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("cli_output", type=Path)
    args = parser.parse_args()
    expected = json.loads(EXPECTED.read_text(encoding="utf-8"))
    actual = json.loads(args.cli_output.read_text(encoding="utf-8"))
    if actual.get("command") != "workload-catalog" or actual.get("schema_version") != 1:
        raise SystemExit("CLI workload catalog envelope is invalid")
    if actual.get("entries") != expected.get("entries"):
        raise SystemExit("CLI workload catalog diverges from generated catalog")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
