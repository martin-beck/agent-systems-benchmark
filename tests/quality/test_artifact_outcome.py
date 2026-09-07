#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Failure and privacy fixtures for optional CI artifact classification."""

from __future__ import annotations

import contextlib
import importlib.util
import io
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
TOOL = ROOT / "tools/quality/artifact_outcome.py"
HEAD = "1" * 40
SPEC = importlib.util.spec_from_file_location("artifact_outcome", TOOL)
assert SPEC is not None and SPEC.loader is not None
ARTIFACT_OUTCOME = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = ARTIFACT_OUTCOME
SPEC.loader.exec_module(ARTIFACT_OUTCOME)


def run(*arguments: str) -> SimpleNamespace:
    stdout = io.StringIO()
    stderr = io.StringIO()
    with (
        mock.patch.object(sys, "argv", [str(TOOL), *arguments]),
        contextlib.redirect_stdout(stdout),
        contextlib.redirect_stderr(stderr),
    ):
        status = ARTIFACT_OUTCOME.main()
    return SimpleNamespace(
        returncode=status, stdout=stdout.getvalue(), stderr=stderr.getvalue()
    )


class ArtifactOutcomeTest(unittest.TestCase):
    def test_prepare_is_bounded_content_free_and_exclusive(self) -> None:
        with tempfile.TemporaryDirectory(prefix="asb-artifact-outcome-") as raw:
            root = Path(raw)
            output = root / "evidence.json"
            result = run(
                "prepare",
                "--repository",
                "martin-beck/agent-systems-benchmark",
                "--ref",
                "refs/pull/42/merge",
                "--head",
                HEAD,
                "--run-id",
                "1234",
                "--attempt",
                "2",
                "--output-root",
                str(root),
                "--output",
                str(output),
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(os.stat(output).st_mode & 0o777, 0o600)
            self.assertLessEqual(output.stat().st_size, 1024)
            self.assertEqual(
                json.loads(output.read_text(encoding="utf-8")),
                {
                    "check": "repository-quality",
                    "head": HEAD,
                    "repository": "martin-beck/agent-systems-benchmark",
                    "run_attempt": 2,
                    "run_id": 1234,
                    "schema_version": 1,
                },
            )
            retry = run(
                "prepare",
                "--repository",
                "martin-beck/agent-systems-benchmark",
                "--ref",
                "refs/heads/main",
                "--head",
                HEAD,
                "--run-id",
                "1234",
                "--attempt",
                "2",
                "--output-root",
                str(root),
                "--output",
                str(output),
            )
            self.assertNotEqual(retry.returncode, 0)

    def test_prepare_rejects_identity_bounds_and_redirection(self) -> None:
        with tempfile.TemporaryDirectory(prefix="asb-artifact-outcome-") as raw:
            root = Path(raw)
            outside = root.parent / f"{root.name}-outside"
            common = [
                "prepare",
                "--repository",
                "martin-beck/agent-systems-benchmark",
                "--ref",
                "refs/heads/main",
                "--head",
                HEAD,
                "--run-id",
                "1",
                "--attempt",
                "1",
                "--output-root",
                str(root),
                "--output",
                str(root / "evidence.json"),
            ]
            for index, bad in (
                (2, "private/repository"),
                (4, "refs/heads/feature"),
                (6, "short"),
                (8, "0"),
                (10, "1001"),
                (14, str(outside)),
            ):
                arguments = common.copy()
                arguments[index] = bad
                result = run(*arguments)
                self.assertNotEqual(result.returncode, 0, arguments)
                self.assertNotIn(str(root), result.stderr)

            target = root / "target"
            target.write_text("sentinel", encoding="utf-8")
            link = root / "evidence.json"
            link.symlink_to(target)
            redirected = run(*common)
            self.assertNotEqual(redirected.returncode, 0)
            self.assertEqual(target.read_text(encoding="utf-8"), "sentinel")

            unavailable_root = root / "absent"
            arguments = common.copy()
            arguments[12] = str(unavailable_root)
            arguments[14] = str(unavailable_root / "evidence.json")
            missing = run(*arguments)
            self.assertNotEqual(missing.returncode, 0)
            self.assertEqual(
                missing.stderr,
                "artifact evidence unavailable: filesystem boundary failed\n",
            )

    def test_optional_quota_like_failure_is_classified_without_retry(self) -> None:
        first = run(
            "classify", "--role", "optional", "--outcome", "failure", "--attempt", "1"
        )
        delayed = run(
            "classify", "--role", "optional", "--outcome", "failure", "--attempt", "2"
        )
        self.assertEqual(first.returncode, 0)
        self.assertEqual(delayed.returncode, 0)
        self.assertEqual(json.loads(first.stdout)["artifact"], "unavailable")
        self.assertEqual(
            json.loads(delayed.stdout)["retry"], "manual-after-provider-recalculation"
        )
        self.assertNotIn("quota", first.stdout)

    def test_required_failure_and_cancellation_remain_fail_closed(self) -> None:
        for role, outcome in (
            ("required", "failure"),
            ("required", "skipped"),
            ("optional", "cancelled"),
            ("optional", "skipped"),
        ):
            result = run(
                "classify", "--role", role, "--outcome", outcome, "--attempt", "1"
            )
            self.assertNotEqual(result.returncode, 0, (role, outcome))
        success = run(
            "classify", "--role", "optional", "--outcome", "success", "--attempt", "1"
        )
        self.assertEqual(success.returncode, 0)
        self.assertEqual(json.loads(success.stdout)["artifact"], "published")

    def test_internal_output_bounds_and_partial_write_fail_closed(self) -> None:
        with self.assertRaises(ValueError):
            ARTIFACT_OUTCOME.emit({"oversized": "x" * 2048})

        with tempfile.TemporaryDirectory(prefix="asb-artifact-outcome-") as raw:
            root = Path(raw)
            arguments = [
                "prepare",
                "--repository",
                "martin-beck/agent-systems-benchmark",
                "--ref",
                "refs/heads/main",
                "--head",
                HEAD,
                "--run-id",
                "1",
                "--attempt",
                "1",
                "--output-root",
                str(root),
                "--output",
                str(root / "evidence.json"),
            ]
            with mock.patch.object(ARTIFACT_OUTCOME.os, "write", return_value=0):
                partial = run(*arguments)
            self.assertNotEqual(partial.returncode, 0)
            self.assertFalse((root / "evidence.json").exists())

            with mock.patch.object(ARTIFACT_OUTCOME, "MAX_JSON_BYTES", 1):
                oversized = run(*arguments)
            self.assertNotEqual(oversized.returncode, 0)
            self.assertFalse((root / "evidence.json").exists())

    def test_file_creation_failure_is_generic_and_leaves_no_output(self) -> None:
        with tempfile.TemporaryDirectory(prefix="asb-artifact-outcome-") as raw:
            root = Path(raw)
            output = root / "evidence.json"
            real_open = ARTIFACT_OUTCOME.os.open
            calls = 0

            def fail_second_open(*args: object, **kwargs: object) -> int:
                nonlocal calls
                calls += 1
                if calls == 2:
                    raise PermissionError("private path must not escape")
                return real_open(*args, **kwargs)

            with mock.patch.object(
                ARTIFACT_OUTCOME.os, "open", side_effect=fail_second_open
            ):
                result = run(
                    "prepare",
                    "--repository",
                    "martin-beck/agent-systems-benchmark",
                    "--ref",
                    "refs/heads/main",
                    "--head",
                    HEAD,
                    "--run-id",
                    "1",
                    "--attempt",
                    "1",
                    "--output-root",
                    str(root),
                    "--output",
                    str(output),
                )
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(
                result.stderr,
                "artifact evidence unavailable: filesystem boundary failed\n",
            )
            self.assertNotIn("private", result.stderr)
            self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()
