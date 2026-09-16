# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Positive and negative bounded transport fixture tests."""
from __future__ import annotations
import json, subprocess, sys, unittest
from pathlib import Path
ROOT = Path(__file__).parent
sys.path.insert(0, str(ROOT))
from transport_fixture import pinned_digest  # noqa: E402

class TransportFixtureTests(unittest.TestCase):
    def test_ordering_and_cleanup_are_real_fixture_operations(self) -> None:
        result = subprocess.run([sys.executable, str(ROOT / "transport_fixture.py")], capture_output=True, text=True, check=True)
        report = json.loads(result.stdout)
        self.assertEqual(report["tool_result_order"], "verified")
        self.assertEqual(report["cancellation_cleanup"], "verified")
        self.assertEqual(report["repeat_clean_state"], "verified")
        self.assertEqual(report["outbound"], "unavailable-outside-isolation")
    def test_non_executable_consumer_is_rejected(self) -> None:
        result = subprocess.run([sys.executable, str(ROOT / "transport_fixture.py"), "--executable", str(ROOT / "README.md")], check=False)
        self.assertNotEqual(result.returncode, 0)

    def test_platform_artifact_pins_are_distinct_and_closed(self) -> None:
        amd64 = pinned_digest("linux-amd64")
        arm64 = pinned_digest("linux-arm64")
        self.assertEqual(len(amd64), 64)
        self.assertEqual(len(arm64), 64)
        self.assertNotEqual(amd64, arm64)

if __name__ == "__main__": unittest.main()
