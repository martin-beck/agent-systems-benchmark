# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Negative-path tests for the MockAgents qualification boundary."""
from __future__ import annotations

import io
import tarfile
import tempfile
import unittest
from pathlib import Path

from qualify_mockagents import QualificationError, load_lock, verify_archive


class QualificationTests(unittest.TestCase):
    def test_lock_is_closed_and_exact(self) -> None:
        lock = load_lock()
        self.assertEqual(lock["source"]["commit"], "6ddb03e54a14484e5929a19673f0cfd8a1975f07")

    def test_archive_rejects_traversal(self) -> None:
        lock = load_lock()
        with tempfile.TemporaryDirectory() as directory:
            archive = Path(directory) / "bad.tar.gz"
            with tarfile.open(archive, "w:gz") as tar:
                for name, content, mode in (("LICENSE", b"bad", 0o644), ("README.md", b"readme", 0o644), ("mockagents", b"x", 0o755), ("../escape", b"x", 0o644)):
                    info = tarfile.TarInfo(name); info.size = len(content); info.mode = mode
                    tar.addfile(info, io.BytesIO(content))
            with self.assertRaises(QualificationError):
                verify_archive(archive, lock, Path(directory) / "out")

    def test_archive_rejects_duplicate(self) -> None:
        lock = load_lock()
        with tempfile.TemporaryDirectory() as directory:
            archive = Path(directory) / "bad.tar.gz"
            with tarfile.open(archive, "w:gz") as tar:
                for name in ("LICENSE", "README.md", "mockagents", "README.md"):
                    info = tarfile.TarInfo(name); info.size = 1; info.mode = 0o755 if name == "mockagents" else 0o644
                    tar.addfile(info, io.BytesIO(b"x"))
            with self.assertRaises(QualificationError):
                verify_archive(archive, lock, Path(directory) / "out")


if __name__ == "__main__":
    unittest.main()
