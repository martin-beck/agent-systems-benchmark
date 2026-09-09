# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Tests for the isolated deterministic-double conformance spike."""

from __future__ import annotations

import json
import unittest
from pathlib import Path

from conformance import suite


class ConformanceTests(unittest.TestCase):
    def test_protocol_fault_and_repeat_evidence(self) -> None:
        report = suite()
        self.assertEqual("synthetic", report["evidence_class"])
        self.assertEqual("loopback-only", report["network"])
        self.assertEqual("pass", report["cases"]["rate_limit"])
        self.assertEqual("pass", report["cases"]["truncated_stream"])
        self.assertEqual("pass", report["cases"]["unmatched_route"])
        self.assertRegex(report["cases"]["openai_responses_sse"], r"^[0-9a-f]{64}$")
        self.assertEqual(report["cases"]["openai_chat_buffered"], report["cases"]["openai_chat_buffered"])

    def test_all_candidates_remain_explicitly_unqualified(self) -> None:
        report = suite()
        self.assertEqual(4, len(report["candidates"]))
        self.assertTrue(all(candidate["status"] == "untested" for candidate in report["candidates"]))
        self.assertTrue(all(candidate["reason"] for candidate in report["candidates"]))

    def test_manifest_has_immutable_revisions_and_licenses(self) -> None:
        manifest = json.loads((Path(__file__).parent / "manifest.json").read_text(encoding="utf-8"))
        self.assertEqual(1, manifest["schema_version"])
        self.assertTrue(all(len(candidate["revision"]) == 40 for candidate in manifest["candidates"]))
        self.assertEqual({"Apache-2.0", "MIT"}, {candidate["license"] for candidate in manifest["candidates"]})


if __name__ == "__main__":
    unittest.main()
