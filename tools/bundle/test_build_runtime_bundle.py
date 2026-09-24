#!/usr/bin/env python3
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Bounded, offline tests for runtime-bundle assembly primitives."""

import importlib.util
import tempfile
import unittest
from argparse import Namespace
from pathlib import Path


MODULE = Path(__file__).with_name("build_runtime_bundle.py")
SPEC = importlib.util.spec_from_file_location("build_runtime_bundle", MODULE)
assert SPEC and SPEC.loader
BUILDER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BUILDER)


class BundleBuilderTests(unittest.TestCase):
    def test_manifest_profile_status_is_truthful(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "bin").mkdir()
            (root / "bin" / "payload").write_bytes(b"payload")
            args = Namespace(
                profile="unsigned-development",
                bundle_id="fixture",
                bundle_version="1.0.0",
                os="linux",
                arch="x86_64",
                libc="glibc",
                libc_version="2.39",
            )
            BUILDER.make_manifest(root, args)
            manifest = __import__("json").loads((root / "manifest.json").read_text())
            self.assertEqual(manifest["profile"], "unsigned-development")
            self.assertEqual(manifest["signature_status"], "unsigned")

    def test_inventory_excludes_signed_metadata_and_is_sorted(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "bin").mkdir()
            for name in ("manifest.json", "manifest.json.sig", "sbom.spdx.json", "sbom.cdx.json"):
                (root / name).write_text("metadata")
            (root / "bin" / "sidecar").write_bytes(b"sidecar")
            (root / "LICENSE").write_bytes(b"MIT")
            inventory = BUILDER.artifacts(root)
            self.assertEqual([item["path"] for item in inventory], ["LICENSE", "bin/sidecar"])

    def test_archive_is_reproducible_and_refuses_overwrite(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "stage"
            root.mkdir()
            (root / "payload").write_bytes(b"bounded payload")
            first = Path(directory) / "first.tar.gz"
            second = Path(directory) / "second.tar.gz"
            BUILDER.archive(root, first)
            BUILDER.archive(root, second)
            self.assertEqual(first.read_bytes(), second.read_bytes())
            with self.assertRaises(SystemExit):
                BUILDER.archive(root, first)


if __name__ == "__main__":
    unittest.main()
