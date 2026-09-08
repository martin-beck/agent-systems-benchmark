#!/usr/bin/env python3
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Render a privacy-safe external result comparison report."""

import argparse
import json
from pathlib import Path


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("comparison", type=Path)
    parser.add_argument("--limitations", nargs="*", default=[])
    args = parser.parse_args()
    comparison = json.loads(args.comparison.read_text(encoding="utf-8"))
    if comparison.get("status") == "comparable":
        lines = [f"## External workload comparison ({comparison['oracle']})", "", f"Evaluator: `{comparison['evaluator']}`", "", "| Workload | Score |", "|---|---:|"]
        lines.extend(f"| {row['workload']} | {row['score']} |" for row in comparison["ranking"])
    elif comparison.get("status") == "non-comparable":
        lines = ["## External workload comparison", "", "**Non-comparable:** " + comparison.get("reason", "comparison contract rejected")]
    else:
        raise SystemExit("comparison status is invalid")
    if args.limitations:
        lines.extend(["", "Limitations:", *[f"- {item}" for item in args.limitations]])
    print("\n".join(lines))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
