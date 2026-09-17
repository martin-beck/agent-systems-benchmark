#!/usr/bin/env python3
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Fail closed when a pull-request topic is based on stale protected main."""

from __future__ import annotations

import argparse
from pathlib import Path

if __package__:
    from .repository_policy import validate_current_topic_base
else:
    from repository_policy import validate_current_topic_base


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=Path.cwd())
    parser.add_argument("--base", required=True)
    parser.add_argument("--head", required=True)
    args = parser.parse_args()
    try:
        validate_current_topic_base(args.root.resolve(strict=True), args.base, args.head)
    except (OSError, ValueError):
        print(
            "topic base policy: topic is stale or its commit identities are invalid",
            flush=True,
        )
        return 1
    print("topic base policy: current protected base is included")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
