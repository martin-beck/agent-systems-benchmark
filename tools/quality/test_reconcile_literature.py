# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Positive and hostile coverage for the literature parity contract."""

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).parents[2]
SPEC = importlib.util.spec_from_file_location(
    "reconcile_literature", ROOT / "tools/quality/reconcile_literature.py"
)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class LiteratureParityTests(unittest.TestCase):
    def setUp(self) -> None:
        self.registry = json.loads(
            (ROOT / "crates/asb-workloads/registry/v1/external-workloads.json").read_text()
        )
        self.documents = {
            name: (ROOT / "docs" / name).read_text() for name in MODULE.DOCS
        }

    def test_every_documented_identity_has_one_canonical_registry_record(self) -> None:
        parity = MODULE.build_parity(self.registry, self.documents)
        self.assertEqual(len(parity["entries"]), 25)
        self.assertEqual(
            {entry["registry_id"] for entry in parity["entries"]}
            - {record["id"] for record in self.registry["workloads"]},
            set(),
        )
        frameworks = [entry for entry in parity["entries"] if entry["kind"] == "framework"]
        self.assertTrue(frameworks)
        self.assertTrue(all(not entry["selectable"] for entry in frameworks))

    def test_missing_documentation_fails_closed(self) -> None:
        documents = dict(self.documents)
        documents = {name: text.replace("Terminal-Bench", "") for name, text in documents.items()}
        with self.assertRaisesRegex(ValueError, "documentation mention is missing"):
            MODULE.build_parity(self.registry, documents)

    def test_duplicate_registry_identity_fails_closed(self) -> None:
        registry = json.loads(json.dumps(self.registry))
        registry["workloads"].append(dict(registry["workloads"][0]))
        with self.assertRaisesRegex(ValueError, "duplicate or invalid registry identity"):
            MODULE.build_parity(registry, self.documents)

    def test_framework_activation_is_rejected(self) -> None:
        registry = json.loads(json.dumps(self.registry))
        harbor = next(record for record in registry["workloads"] if record["id"] == "harbor")
        harbor["selection"] = "executable-candidate"
        with self.assertRaisesRegex(ValueError, "framework classification disagrees"):
            MODULE.build_parity(registry, self.documents)


if __name__ == "__main__":
    unittest.main()
