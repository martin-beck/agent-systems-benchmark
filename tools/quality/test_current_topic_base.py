#!/usr/bin/env python3
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Positive and negative tests for current protected-base admission."""

from __future__ import annotations

import subprocess
import tempfile
import unittest
from pathlib import Path

from tools.quality.repository_policy import validate_current_topic_base


def git(root: Path, *args: str) -> str:
    return subprocess.check_output(["git", *args], cwd=root, text=True).strip()


class CurrentTopicBaseTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory(prefix="asb-topic-base-")
        self.root = Path(self.temp.name)
        git(self.root, "init", "-q", "-b", "main")
        git(self.root, "config", "user.name", "Fixture")
        git(self.root, "config", "user.email", "fixture@example.invalid")
        (self.root / "base").write_text("base\n", encoding="utf-8")
        git(self.root, "add", "base")
        git(self.root, "commit", "-q", "-m", "base")
        self.base = git(self.root, "rev-parse", "HEAD")
        git(self.root, "switch", "-q", "-c", "topic")
        (self.root / "topic").write_text("topic\n", encoding="utf-8")
        git(self.root, "add", "topic")
        git(self.root, "commit", "-q", "-m", "topic")
        self.topic = git(self.root, "rev-parse", "HEAD")

    def tearDown(self) -> None:
        self.temp.cleanup()

    def test_current_base_is_accepted(self) -> None:
        validate_current_topic_base(self.root, self.base, self.topic)

    def test_stale_topic_is_rejected(self) -> None:
        git(self.root, "switch", "-q", "main")
        (self.root / "main").write_text("advanced\n", encoding="utf-8")
        git(self.root, "add", "main")
        git(self.root, "commit", "-q", "-m", "advance protected main")
        current_base = git(self.root, "rev-parse", "HEAD")
        with self.assertRaisesRegex(ValueError, "not based on the current protected base"):
            validate_current_topic_base(self.root, current_base, self.topic)


if __name__ == "__main__":
    unittest.main()
