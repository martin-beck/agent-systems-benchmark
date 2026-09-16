#!/usr/bin/env python3
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Offline regression tests for the Rust verification workflow's DCO routing."""

from __future__ import annotations

import re
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github/workflows/verify.yml"


def dco_step() -> str:
    text = WORKFLOW.read_text(encoding="utf-8")
    match = re.search(
        r"      - name: DCO certification\n(?P<body>.*?)(?=      - name: Clean generated state\n)",
        text,
        re.DOTALL,
    )
    if match is None:
        raise AssertionError("verify workflow has no DCO certification step")
    return match.group("body")


class VerifyWorkflowTests(unittest.TestCase):
    def test_canonical_main_push_uses_protected_policy(self) -> None:
        body = dco_step()
        self.assertIn('EVENT_REF: ${{ github.ref }}', body)
        self.assertIn('"$EVENT_NAME" == push', body)
        self.assertIn('"$EVENT_REF" == refs/heads/main', body)
        self.assertIn('python3 tools/quality/repository_policy.py \\', body)
        self.assertIn('--mode protected-main', body)
        self.assertIn('--event push', body)
        self.assertIn('--ref refs/heads/main', body)
        self.assertIn('--base "$base"', body)
        self.assertIn('--head "$head"', body)

    def test_strict_checker_is_reserved_for_noncanonical_contexts(self) -> None:
        body = dco_step()
        protected, strict = body.split("          else\n", 1)
        self.assertNotIn("check_dco.py", protected)
        self.assertIn('python3 tools/quality/check_dco.py "${args[@]}"', strict)

    def test_route_cannot_regress_to_strict_checker_on_main(self) -> None:
        body = dco_step()
        self.assertLess(
            body.index("--mode protected-main"), body.index("check_dco.py")
        )
        self.assertIn(
            '"$EVENT_NAME" == push &&\n'
            '                "$EVENT_REF" == refs/heads/main &&',
            body,
        )


if __name__ == "__main__":
    unittest.main()
