# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Hostile and round-trip tests for the synthetic scenario contract."""
from __future__ import annotations

import json
import unittest
from pathlib import Path

from scenario import ScenarioError, canonical_bytes, validate

FIXTURE = Path(__file__).parent / "fixtures/synthetic-scenario-v1.json"
class ScenarioTests(unittest.TestCase):
    def setUp(self) -> None: self.value = json.loads(FIXTURE.read_text(encoding="utf-8"))
    def test_fixture_is_canonical_and_reproducible(self) -> None:
        first = canonical_bytes(self.value); self.assertEqual(first, canonical_bytes(json.loads(first))); self.assertEqual("synthetic", validate(self.value)["evidence_origin"])
    def test_hostile_mutations_fail_closed(self) -> None:
        mutations = []
        for key, value in (("evidence_origin", "strict_replay"), ("protocol", "unknown"), ("api_key", "secret")):
            changed = json.loads(json.dumps(self.value)); changed[key] = value; mutations.append(changed)
        changed = json.loads(json.dumps(self.value)); changed["events"][1]["sequence"] = 0; mutations.append(changed)
        changed = json.loads(json.dumps(self.value)); changed["events"][0]["payload"] = {"url": "https://provider.example"}; mutations.append(changed)
        for changed in mutations:
            with self.subTest(changed=changed), self.assertRaises(ScenarioError): validate(changed)
    def test_bounds_and_session_isolation(self) -> None:
        changed = json.loads(json.dumps(self.value)); changed["events"] = changed["events"] * 11
        with self.assertRaises(ScenarioError): validate(changed)
        changed = json.loads(json.dumps(self.value)); changed["events"][0]["session"] = "Bad Session"
        with self.assertRaises(ScenarioError): validate(changed)
if __name__ == "__main__": unittest.main()
