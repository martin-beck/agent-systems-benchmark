# SPDX-License-Identifier: MIT
"""Positive and deliberate-failure tests for platform manifests."""

from __future__ import annotations

import base64
import copy
import importlib.util
import json
import unittest
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "validate_manifests", ROOT / "tools/platforms/validate_manifests.py"
)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("cannot load manifest validator")
VALIDATOR = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(VALIDATOR)


def apply_mutations(document: dict[str, Any], mutations: list[dict[str, Any]]) -> None:
    """Apply a fixture mutation to an in-memory document."""
    for mutation in mutations:
        target: Any = document
        for component in mutation["path"][:-1]:
            target = target[component]
        target[mutation["path"][-1]] = mutation["value"]


class ManifestTests(unittest.TestCase):
    """Exercise canonical and known-invalid platform documents."""

    @classmethod
    def setUpClass(cls) -> None:
        cls.platforms = VALIDATOR.load(ROOT / "platforms/v1/platforms.json")
        cls.agents = VALIDATOR.load(ROOT / "platforms/v1/agents.json")

    def test_canonical_manifests_pass(self) -> None:
        self.assertEqual(VALIDATOR.validate(self.platforms, self.agents), [])

    def test_npm_and_pypi_integrity_formats_are_accepted(self) -> None:
        sri = "sha512-" + base64.b64encode(b"a" * 64).decode("ascii")
        self.assertTrue(VALIDATOR.valid_integrity(sri))
        self.assertTrue(VALIDATOR.valid_integrity("sha256:" + "a" * 64))

    def test_npm_integrity_rejects_bad_alphabet_padding_and_length(self) -> None:
        sri = "sha512-" + base64.b64encode(b"a" * 64).decode("ascii")
        self.assertFalse(VALIDATOR.valid_integrity("sha512-not*base64="))
        self.assertFalse(VALIDATOR.valid_integrity(sri.rstrip("=")))
        self.assertFalse(VALIDATOR.valid_integrity("sha512-YWJjZA=="))

    def test_failure_fixtures_are_rejected(self) -> None:
        fixtures = sorted((ROOT / "tests/platforms/fixtures").glob("*.json"))
        self.assertGreaterEqual(len(fixtures), 3)
        for path in fixtures:
            with self.subTest(path=path.name):
                fixture = json.loads(path.read_text(encoding="utf-8"))
                candidate = copy.deepcopy(self.platforms)
                apply_mutations(candidate, fixture["mutations"])
                errors = VALIDATOR.validate(candidate, self.agents)
                self.assertTrue(
                    any(fixture["expected_error"] in error for error in errors),
                    errors,
                )


if __name__ == "__main__":
    unittest.main()
