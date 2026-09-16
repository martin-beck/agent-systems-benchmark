# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Regression tests for the isolated qualification runner contract."""
from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from run_isolated import PYTHON_IMAGE, build_command, verify_artifact  # noqa: E402


class RunnerContractTests(unittest.TestCase):
    def setUp(self) -> None:
        self.artifact = Path("/srv/data/projects/.asb-ar1249-artifact-audit/mockagents.tar.gz")

    def test_command_is_network_and_mount_restricted(self) -> None:
        command = build_command(self.artifact, ["/bin/true"])
        self.assertIn("--rm", command)
        self.assertIn("--network", command)
        self.assertEqual(command[command.index("--network") + 1], "none")
        self.assertIn("--read-only", command)
        self.assertIn("--cap-drop", command)
        self.assertEqual(command[command.index("--cap-drop") + 1], "ALL")
        self.assertNotIn("--privileged", command)
        mounts = [part for part in command if part.startswith("type=bind")]
        self.assertEqual(mounts, ["type=bind,src=/srv/data/projects/.asb-ar1249-artifact-audit/mockagents.tar.gz,dst=/input/artifact,readonly"])
        self.assertNotIn("--volume", command)

    def test_cleanup_is_container_owned(self) -> None:
        command = build_command(self.artifact, ["/bin/true"], "asb-ar1252-123")
        self.assertEqual(command[command.index("--rm")], "--rm")
        self.assertEqual(command[command.index("--name") + 1], "asb-ar1252-123")
        self.assertEqual(command[command.index("--mount") + 1].split(",")[-1], "readonly")

    def test_container_name_is_internal_and_bounded(self) -> None:
        with self.assertRaises(ValueError):
            build_command(self.artifact, ["/bin/true"], "arbitrary-name")

    def test_only_reviewed_python_digest_can_be_selected(self) -> None:
        command = build_command(self.artifact, ["/bin/true"], "asb-ar1252-123", PYTHON_IMAGE)
        self.assertIn(PYTHON_IMAGE, command)
        with self.assertRaises(ValueError):
            build_command(self.artifact, ["/bin/true"], "asb-ar1252-123", "python:latest")

    def test_shell_vectors_are_rejected(self) -> None:
        with self.assertRaises(ValueError):
            build_command(self.artifact, ["/bin/sh"])
        with self.assertRaises(ValueError):
            build_command(self.artifact, ["-c", "true"])

    def test_artifact_digest_mismatch_is_rejected(self) -> None:
        with tempfile.NamedTemporaryFile() as artifact:
            artifact.write(b"pinned fixture")
            artifact.flush()
            with self.assertRaises(ValueError):
                verify_artifact(Path(artifact.name), "0" * 64)


if __name__ == "__main__":
    unittest.main()
