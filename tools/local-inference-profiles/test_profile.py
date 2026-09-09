# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Hostile and round-trip tests for local inference profile evidence."""
from __future__ import annotations

import json
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from profile import ProfileError, canonical_bytes, validate

FIXTURE = Path(__file__).parent / "profiles-v1.json"


class ProfileTests(unittest.TestCase):
    def setUp(self) -> None:
        self.value = json.loads(FIXTURE.read_text(encoding="utf-8"))

    def test_fixture_is_canonical_and_reproducible(self) -> None:
        first = canonical_bytes(self.value)
        self.assertEqual(first, canonical_bytes(json.loads(first)))
        self.assertEqual(["llama_cpp", "localai", "ollama", "vllm"], [item["id"] for item in validate(self.value)["profiles"]])

    def test_unqualified_profiles_are_not_selectable(self) -> None:
        self.assertEqual(["ollama"], [item["id"] for item in self.value["profiles"] if item["selectable"]])
        ollama = self.value["profiles"][2]
        self.assertEqual("06c1097efce0431c2045fe7b2e5108366e43bee1b4603a7aded8f21689e90bca", ollama["model"]["sha256"])
        self.assertEqual("2773bb7a4d7d8f04b21e09d3f3c0b968792ea7ba1284156004cc0dc39ef3c7f3", ollama["model"]["tokenizer_sha256"])

    def test_hostile_mutations_fail_closed(self) -> None:
        mutations = []
        changed = json.loads(json.dumps(self.value)); changed["profiles"][0]["selectable"] = True; mutations.append(changed)
        changed = json.loads(json.dumps(self.value)); changed["profiles"][2]["model"]["sha256"] = "bad"; mutations.append(changed)
        changed = json.loads(json.dumps(self.value)); changed["profiles"][2]["frontend"]["source"] = "https://private.example/engine"; mutations.append(changed)
        changed = json.loads(json.dumps(self.value)); changed["profiles"][2]["configuration"]["isolation"]["network"] = "public"; mutations.append(changed)
        changed = json.loads(json.dumps(self.value)); changed["profiles"][2]["qualification"]["trials"] = -1; mutations.append(changed)
        for value in mutations:
            with self.subTest(value=value), self.assertRaises(ProfileError):
                validate(value)


if __name__ == "__main__":
    unittest.main()
