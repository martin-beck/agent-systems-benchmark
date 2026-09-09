# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Hostile and deterministic tests for the bounded GGUF metadata reader."""
from __future__ import annotations

import struct
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from gguf_tokenizer_digest import GgufError, tokenizer_digest


def _string(value: str) -> bytes:
    encoded = value.encode()
    return struct.pack("<Q", len(encoded)) + encoded


def _metadata(key: str, value: str) -> bytes:
    return _string(key) + struct.pack("<I", 8) + _string(value)


def _fixture() -> bytes:
    metadata = _metadata("tokenizer.ggml.model", "gpt2")
    metadata += _metadata("tokenizer.ggml.pre", "qwen2")
    return b"GGUF" + struct.pack("<IQQ", 3, 1, 2) + metadata


class GgufTokenizerDigestTests(unittest.TestCase):
    def test_digest_is_deterministic_and_ignores_non_tokenizer_keys(self) -> None:
        first = _fixture()
        second = first.replace(b"tokenizer.ggml.pre", b"general.architectu")
        with tempfile.TemporaryDirectory() as directory:
            one = Path(directory) / "one.gguf"
            two = Path(directory) / "two.gguf"
            one.write_bytes(first)
            two.write_bytes(second)
            self.assertNotEqual(tokenizer_digest(one), tokenizer_digest(two))
            self.assertEqual(tokenizer_digest(one), tokenizer_digest(one))

    def test_truncation_and_magic_fail_closed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "bad.gguf"
            path.write_bytes(b"bad")
            with self.assertRaises(GgufError):
                tokenizer_digest(path)
            path.write_bytes(_fixture()[:-1])
            with self.assertRaises(GgufError):
                tokenizer_digest(path)


if __name__ == "__main__":
    unittest.main()
