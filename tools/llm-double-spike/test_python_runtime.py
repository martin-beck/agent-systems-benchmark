# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Tests for the closed Python fixture-runtime provenance contract."""
from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from python_runtime import IMAGE, RuntimeError, load_manifest  # noqa: E402


class PythonRuntimeTests(unittest.TestCase):
    def test_reviewed_manifest_is_immutable_and_isolated(self) -> None:
        manifest = load_manifest()
        self.assertEqual(manifest["image"], IMAGE)
        self.assertEqual(manifest["network"], "none")
        self.assertEqual(manifest["site_initialization"], "disabled")

    def test_unknown_fields_and_digest_drift_fail_closed(self) -> None:
        original = json.loads(Path(__file__).with_name("python-runtime-v1.json").read_text())
        for change in ({"extra": True}, {"image": "python:latest"}):
            candidate = dict(original)
            candidate.update(change)
            with tempfile.NamedTemporaryFile(mode="w", suffix=".json") as stream:
                json.dump(candidate, stream)
                stream.flush()
                with self.assertRaises(RuntimeError):
                    load_manifest(Path(stream.name))


if __name__ == "__main__":
    unittest.main()
