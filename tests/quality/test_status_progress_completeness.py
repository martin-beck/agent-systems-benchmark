#!/usr/bin/env python3
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Controlled defects for the status/progress completeness gate."""

from __future__ import annotations

import json
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
CHECK = ROOT / "tools/quality/check_status_progress_completeness.py"


def run(root: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(CHECK), "--root", str(root)],
        cwd=root,
        text=True,
        capture_output=True,
        check=False,
    )


def copy_tree(tmp_path: Path) -> Path:
    destination = tmp_path / "repo"
    shutil.copytree(ROOT, destination, ignore=shutil.ignore_patterns("target", ".git"))
    return destination


class StatusProgressCompletenessTests(unittest.TestCase):
    def with_copy(self):
        temporary = tempfile.TemporaryDirectory()
        root = copy_tree(Path(temporary.name))
        self.addCleanup(temporary.cleanup)
        return root

    def test_inventory_and_renderer_contract_passes(self):
        result = run(ROOT)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("public command families", result.stdout)

    def test_missing_route_is_a_controlled_defect(self):
        root = self.with_copy()
        path = root / "crates/asb-cli/fixtures/human/command-surface-v1.json"
        document = json.loads(path.read_text())
        document["routes"] = [entry for entry in document["routes"] if entry["path"] != "run"]
        path.write_text(json.dumps(document))
        result = run(root)
        self.assertEqual(result.returncode, 1)
        self.assertIn("missing from generated route inventory", result.stderr)

    def test_direct_ansi_renderer_bypass_is_a_controlled_defect(self):
        root = self.with_copy()
        source = root / "crates/asb-cli/src/lib.rs"
        source.write_text(source.read_text() + '\nconst BAD_PROGRESS: &str = "\\x1b[2K";\n')
        result = run(root)
        self.assertEqual(result.returncode, 1)
        self.assertIn("bypasses the shared progress renderer", result.stderr)
