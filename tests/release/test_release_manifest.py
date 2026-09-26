#!/usr/bin/env python3
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
from __future__ import annotations

import hashlib
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).parents[2]
BUILDER = ROOT / "tools/release/build_manifest.py"
GATE = ROOT / "tools/quality/run_pinned_cargo_gates.py"


class ReleaseManifestTests(unittest.TestCase):
    def test_manifest_is_deterministic_and_privacy_safe(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / "asb"
            binary.write_bytes(b"fixture-release-binary\n")
            output = root / "bundle"
            revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
            subprocess.run([sys.executable, str(BUILDER), "--source-revision", revision, "--binary", str(binary), "--output", str(output)], cwd=ROOT, check=True)
            manifest = json.loads((output / "manifest.json").read_text())
            self.assertEqual(manifest["source_revision"], revision)
            self.assertEqual(manifest["provenance"]["network"], "none")
            self.assertNotIn(str(root), (output / "manifest.json").read_text())
            self.assertEqual((output / "asb").stat().st_mode & 0o777, 0o755)
            self.assertEqual(len(manifest["artifacts"]), 1)
            self.assertEqual(manifest["artifacts"][0]["sha256"], hashlib.sha256(binary.read_bytes()).hexdigest())

    def test_dirty_or_mismatched_inputs_fail_closed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "asb"
            binary.write_bytes(b"fixture\n")
            result = subprocess.run([sys.executable, str(BUILDER), "--source-revision", "0" * 40, "--binary", str(binary), "--output", str(Path(directory) / "out")], cwd=ROOT, capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)

    def test_pinned_gate_rejects_missing_tool_and_bad_manifest(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest = root / "tools.json"
            manifest.write_text(json.dumps({"schema_version": 1, "tools": {"cargo-deny": {"version": "0.20.2", "sha256": "0" * 64}}}))
            result = subprocess.run([sys.executable, str(GATE), "--tool-manifest", str(manifest), "--bin-dir", str(root / "bin")], capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)


if __name__ == "__main__":
    unittest.main()
