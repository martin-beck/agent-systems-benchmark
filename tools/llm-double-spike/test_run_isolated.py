# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Regression tests for the isolated qualification runner contract."""
from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from run_isolated import build_command  # noqa: E402


class RunnerContractTests(unittest.TestCase):
    def setUp(self) -> None:
        self.artifact = Path("/srv/data/projects/.asb-ar1249-artifact-audit/mockagents.tar.gz")

    def test_command_is_network_and_mount_restricted(self) -> None:
        command = build_command(self.artifact, ["/bin/true"])
        self.assertIn("--network", command)
        self.assertEqual(command[command.index("--network") + 1], "none")
        self.assertIn("--read-only", command)
        self.assertIn("--cap-drop", command)
        self.assertEqual(command[command.index("--cap-drop") + 1], "ALL")
        self.assertNotIn("--privileged", command)
        self.assertEqual(sum(part.startswith("type=bind") for part in command), 1)

    def test_shell_vectors_are_rejected(self) -> None:
        with self.assertRaises(ValueError):
            build_command(self.artifact, ["/bin/sh"])
        with self.assertRaises(ValueError):
            build_command(self.artifact, ["-c", "true"])


if __name__ == "__main__":
    unittest.main()
