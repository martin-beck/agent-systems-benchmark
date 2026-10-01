#!/usr/bin/env python3
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
import json
import shlex
import tempfile
import time
import unittest
from pathlib import Path
from unittest import mock

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
            with mock.patch.object(run, "isolated_command", side_effect=lambda command: command):
                result = run.run_case(Path("/bin/true"), {
                "name": "doctor", "kind": "recovery", "argv": [], "timeout": 1.0,
                "expected_exit": 0, "cleanup": [], "development_warning_only": False,
                }, root)
            self.assertEqual(result["classification"], "passed")
            self.assertTrue(result["cleanup_ok"])
            self.assertFalse(root.exists())

    def test_network_isolation_failure_is_typed_and_cleans_root(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "case"
            with mock.patch.object(run, "isolated_command", side_effect=RuntimeError("secret host detail")):
                result = run.run_case(Path("/bin/true"), {
                    "name": "network", "kind": "setup", "argv": [], "timeout": 1.0,
                    "expected_exit": 0, "cleanup": [], "development_warning_only": False,
                }, root)
            self.assertEqual(result["classification"], "runner_unavailable")
            self.assertTrue(result["cleanup_ok"])
            self.assertNotIn("secret", json.dumps(result))
            self.assertFalse(root.exists())

    def test_streaming_output_is_capped_and_process_group_is_reaped(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "case"
            with mock.patch.object(run, "isolated_command", side_effect=lambda command: command):
                result = run.run_case(Path("/bin/sh"), {
                    "name": "noisy", "kind": "record",
                    "argv": ["-c", "yes x | head -c 1000000"], "timeout": 2.0,
                    "expected_exit": 0, "cleanup": [], "development_warning_only": False,
                }, root)
            self.assertEqual(result["classification"], "output_exceeded")
            self.assertLessEqual(len(result["output"].encode()), run.MAX_OUTPUT)
            self.assertTrue(result["cleanup_ok"])
            self.assertFalse(root.exists())

    def test_timeout_kills_process_group_and_removes_private_root(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "case"
            with mock.patch.object(run, "isolated_command", side_effect=lambda command: command):
                result = run.run_case(Path("/bin/sh"), {
                    "name": "hang", "kind": "recovery", "argv": ["-c", "sleep 30"],
                    "timeout": 0.1, "expected_exit": 0, "cleanup": [],
                    "development_warning_only": False,
                }, root)
            self.assertEqual(result["classification"], "timeout")
            self.assertTrue(result["cleanup_ok"])
            self.assertFalse(root.exists())

    def test_leader_exit_does_not_skip_descendant_group_cleanup(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "case"
            marker = Path(directory) / "escaped-marker"
            command = f"sleep 0.5; echo escaped > {shlex.quote(str(marker))}"
            with mock.patch.object(run, "isolated_command", side_effect=lambda command: command):
                result = run.run_case(Path("/bin/sh"), {
                    "name": "orphan", "kind": "recovery",
                    "argv": ["-c", f"({command}) >/dev/null 2>&1 & exit 0"], "timeout": 2.0,
                    "expected_exit": 0, "cleanup": [], "development_warning_only": False,
                }, root)
            self.assertEqual(result["classification"], "descendants_survived")
            self.assertTrue(result["cleanup_ok"])
            self.assertFalse(root.exists())
            time.sleep(0.7)
            self.assertFalse(marker.exists())

    def test_warning_requires_typed_code(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "case"
            case = {"name": "warn", "kind": "setup", "argv": ["-c", "printf '{\"status\":\"warning\",\"code\":\"auth_unavailable\"}'"],
                    "timeout": 1.0, "expected_exit": 0, "cleanup": [],
                    "development_warning_only": True, "warning_code": "auth_unavailable"}
            with mock.patch.object(run, "isolated_command", side_effect=lambda command: command):
                result = run.run_case(Path("/bin/sh"), case, root)
            self.assertEqual(result["classification"], "warning")

    def test_manifest_rejects_network_fallback(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "matrix.json"
            path.write_text(json.dumps({"schema_version": 1, "cases": [{
                "name": "bad", "kind": "setup", "argv": ["x"], "network": "host"
            }]}), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "network=none"):
                run.load_manifest(path)


if __name__ == "__main__":
    unittest.main()
