# SPDX-License-Identifier: MIT
"""Failure tests for the closed contract registry."""

from __future__ import annotations

import copy
import importlib.util
import json
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "contract_consistency", ROOT / "tools/quality/contract_consistency.py"
)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class ContractConsistencyTests(unittest.TestCase):
    def setUp(self) -> None:
        self.catalog = json.loads((ROOT / "contracts/v1/catalog.json").read_text())

    def assert_rejected(self, transform: object) -> None:
        changed = copy.deepcopy(self.catalog)
        transform(changed)
        with tempfile.NamedTemporaryFile(
            mode="w", suffix=".json", dir=ROOT, delete=False
        ) as output:
            json.dump(changed, output)
            path = Path(output.name)
        try:
            with self.assertRaises(MODULE.ContractError):
                MODULE.load_catalog(ROOT, path)
        finally:
            path.unlink()

    def test_duplicate_and_unregistered_contracts_fail_closed(self) -> None:
        self.assert_rejected(
            lambda value: value["contracts"].append(value["contracts"][0])
        )
        self.assert_rejected(lambda value: value["contracts"].pop())

    def test_paths_unknown_fields_and_missing_roundtrip_fail_closed(self) -> None:
        self.assert_rejected(
            lambda value: value["contracts"][0].update(schema="../outside.schema.json")
        )
        self.assert_rejected(
            lambda value: value["contracts"][0].update(schema="unsafe\x00schema.json")
        )
        self.assert_rejected(lambda value: value.update(private_config=True))
        self.assert_rejected(lambda value: value["contracts"][0].update(rust_test=""))
        self.assert_rejected(lambda value: value["conformance_commands"].pop())
        self.assert_rejected(
            lambda value: value["contracts"][0]["fixtures"].append(
                value["contracts"][0]["fixtures"][0]
            )
        )
        self.assert_rejected(
            lambda value: value.update(capability_snapshot="../private")
        )

    def test_capability_snapshot_is_closed_and_boolean(self) -> None:
        self.assert_rejected(
            lambda value: value.update(
                capability_snapshot="crates/asb-control/fixtures/v1/success-response.json"
            )
        )
        snapshot = json.loads((ROOT / self.catalog["capability_snapshot"]).read_text())
        snapshot["settings"]["temperature"]["exact_value"] = "yes"
        with self.assertRaises(MODULE.ContractError):
            MODULE.validate_capability_snapshot(snapshot)

    def test_in_place_v1_schema_change_fails_closed(self) -> None:
        with self.assertRaises(MODULE.ContractError):
            MODULE.ensure_v1_compatibility(ROOT, self.catalog, "not-a-commit")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            schema = root / "schema/v1/example.schema.json"
            schema.parent.mkdir(parents=True)
            schema.write_text("{}\n")
            subprocess.run(["git", "init", "-q"], cwd=root, check=True)
            subprocess.run(["git", "add", "."], cwd=root, check=True)
            subprocess.run(
                [
                    "git",
                    "-c",
                    "user.name=Fixture",
                    "-c",
                    "user.email=fixture@example.invalid",
                    "commit",
                    "-q",
                    "-m",
                    "fixture",
                ],
                cwd=root,
                check=True,
            )
            baseline = subprocess.run(
                ["git", "rev-parse", "HEAD"],
                cwd=root,
                check=True,
                text=True,
                stdout=subprocess.PIPE,
            ).stdout.strip()
            schema.write_text('{"type":"object"}\n')
            with self.assertRaises(MODULE.ContractError):
                MODULE.ensure_v1_compatibility(
                    root,
                    {"contracts": [{"schema": "schema/v1/example.schema.json"}]},
                    baseline,
                )
            schema.write_text("{ }\n")
            MODULE.ensure_v1_compatibility(
                root,
                {"contracts": [{"schema": "schema/v1/example.schema.json"}]},
                baseline,
            )


if __name__ == "__main__":
    unittest.main()
