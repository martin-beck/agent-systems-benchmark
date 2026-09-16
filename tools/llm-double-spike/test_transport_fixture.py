# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Positive and negative bounded transport fixture tests."""
from __future__ import annotations
import json, subprocess, sys, unittest
from pathlib import Path
ROOT = Path(__file__).parent

class TransportFixtureTests(unittest.TestCase):
    def test_ordering_and_cleanup_are_real_fixture_operations(self) -> None:
        result = subprocess.run([sys.executable, str(ROOT / "transport_fixture.py")], capture_output=True, text=True, check=True)
        report = json.loads(result.stdout)
        self.assertEqual(report["tool_result_order"], "verified")
        self.assertEqual(report["cancellation_cleanup"], "verified")
        self.assertEqual(report["outbound"], "unavailable-outside-isolation")
    def test_non_executable_consumer_is_rejected(self) -> None:
        result = subprocess.run([sys.executable, str(ROOT / "transport_fixture.py"), "--executable", str(ROOT / "README.md")], check=False)
        self.assertNotEqual(result.returncode, 0)

if __name__ == "__main__": unittest.main()
