# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Repository-wide tutorial freshness and no-execution regression tests."""

import ast
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from .check_freshness import validate_repository
from .validate import ValidationError, load, validate_document

ROOT = Path(__file__).parents[2]
TUTORIALS = ROOT / "tools" / "tutorials"


class TutorialFreshnessTests(unittest.TestCase):
    def test_repository_discovery_and_documentation_are_current(self) -> None:
        self.assertEqual(validate_repository(ROOT), [])

    def test_removed_command_is_rejected(self) -> None:
        metadata = load(TUTORIALS / "command_metadata_v1.json")
        metadata["commands"].pop("provider-catalog")
        with self.assertRaisesRegex(ValidationError, "unknown ASB command"):
            validate_document(load(TUTORIALS / "initial-setup-v1.json"), metadata)

    def test_renamed_option_is_rejected(self) -> None:
        metadata = load(TUTORIALS / "command_metadata_v1.json")
        metadata["commands"]["capabilities"]["forms"] = [["--renamed", "json"]]
        with self.assertRaises(ValidationError):
            validate_document(load(TUTORIALS / "initial-setup-v1.json"), metadata)

    def test_duplicate_metadata_keys_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "duplicate.json"
            path.write_text('{"a": 1, "a": 2}', encoding="utf-8")
            with self.assertRaisesRegex(ValidationError, "duplicate"):
                load(path)

    def test_diagnostics_are_deterministic(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "tools/tutorials").mkdir(parents=True)
            (root / "docs").mkdir()
            (root / "tools/tutorials/command_metadata_v1.json").write_text(
                "{}", encoding="utf-8"
            )
            self.assertEqual(validate_repository(root), validate_repository(root))

    def test_clean_home_subprocess_is_offline_and_does_not_execute_commands(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as home:
            env = {"PATH": os.environ["PATH"], "HOME": home, "PYTHONPATH": str(ROOT)}
            result = subprocess.run(
                [sys.executable, "tools/tutorials/check_freshness.py"],
                cwd=ROOT,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("contracts validated", result.stdout)

    def test_gate_has_no_process_or_network_imports(self) -> None:
        tree = ast.parse((TUTORIALS / "check_freshness.py").read_text(encoding="utf-8"))
        imports = {
            alias.name.split(".", 1)[0]
            for node in ast.walk(tree)
            if isinstance(node, ast.Import)
            for alias in node.names
        }
        self.assertNotIn("subprocess", imports)
        self.assertNotIn("socket", imports)


if __name__ == "__main__":
    unittest.main()
