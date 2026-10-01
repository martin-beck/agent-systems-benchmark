#!/usr/bin/env python3
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
import json
import tempfile
import unittest
from pathlib import Path

import run


class FaultMatrixTests(unittest.TestCase):
    def test_manifest_is_closed_and_bounded(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "matrix.json"
            path.write_text(json.dumps({"schema_version": 1, "cases": [
                {"name": "doctor", "kind": "recovery", "argv": ["doctor"], "timeout_seconds": 1}
            ]}), encoding="utf-8")
            self.assertEqual(run.load_manifest(path)[0]["kind"], "recovery")

    def test_invalid_kind_and_timeout_fail_closed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "matrix.json"
            path.write_text(json.dumps({"schema_version": 1, "cases": [
                {"name": "bad", "kind": "other", "argv": [], "timeout_seconds": 1}
            ]}), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "unsupported kind"):
                run.load_manifest(path)

    def test_runner_reports_expected_success(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "case"
            result = run.run_case(Path("/bin/true"), {
                "name": "doctor", "kind": "recovery", "argv": [], "timeout": 1.0,
                "expected_exit": 0, "cleanup": [], "development_warning_only": False,
            }, root)
            self.assertEqual(result["classification"], "passed")
            self.assertTrue(result["cleanup_ok"])


if __name__ == "__main__":
    unittest.main()
