#!/usr/bin/env python3
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Bounded, offline tests for runtime-bundle assembly primitives."""

import importlib.util
import json
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

    def test_staging_handoff_is_signed_profile_but_has_no_signature(self) -> None:
        handoff_module = Path(__file__).with_name("prepare_signing_handoff.py")
        handoff_spec = importlib.util.spec_from_file_location("prepare_signing_handoff", handoff_module)
        assert handoff_spec and handoff_spec.loader
        handoff = importlib.util.module_from_spec(handoff_spec)
        handoff_spec.loader.exec_module(handoff)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            supervisor = root / "supervisor"
            sidecar = root / "sidecar"
            supervisor.write_bytes(b"supervisor")
            sidecar.write_bytes(b"sidecar")
            stage = root / "stage"
            handoff_file = root / "handoff.json"
            handoff.stage(
                Namespace(
                    bundle_id="fixture",
                    bundle_version="1.0.0",
                    os="linux",
                    arch="x86_64",
                    libc="glibc",
                    libc_version="2.39",
                    principal="asb-release",
                    ssh_keygen_sha256="a" * 64,
                    supervisor=supervisor,
                    sidecar=sidecar,
                    stage_output=stage,
                    handoff_output=handoff_file,
                )
            )
            manifest = json.loads((stage / "manifest.json").read_text())
            record = json.loads(handoff_file.read_text())
            self.assertEqual(manifest["profile"], "signed")
            self.assertEqual(manifest["signature_status"], "signed")
            self.assertFalse((stage / "manifest.json.sig").exists())
            self.assertTrue(record["verification"]["signature_must_be_supplied_externally"])
            self.assertEqual(record["required_signature"]["namespace"], "asb-runtime-bundle-v1")

    def test_staging_refuses_existing_output_and_bad_helpers(self) -> None:
        handoff_module = Path(__file__).with_name("prepare_signing_handoff.py")
        handoff_spec = importlib.util.spec_from_file_location("prepare_signing_handoff", handoff_module)
        assert handoff_spec and handoff_spec.loader
        handoff = importlib.util.module_from_spec(handoff_spec)
        handoff_spec.loader.exec_module(handoff)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            helper = root / "helper"
            helper.write_bytes(b"helper")
            stage = root / "stage"
            stage.mkdir()
            args = Namespace(
                bundle_id="fixture", bundle_version="1.0.0", os="linux", arch="x86_64",
                libc="glibc", libc_version="2.39", principal="asb-release",
                ssh_keygen_sha256="a" * 64, supervisor=helper, sidecar=helper,
                stage_output=stage, handoff_output=root / "handoff.json",
            )
            with self.assertRaises(SystemExit):
                handoff.stage(args)

    def test_staging_rejects_malformed_external_authority_inputs(self) -> None:
        handoff_module = Path(__file__).with_name("prepare_signing_handoff.py")
        handoff_spec = importlib.util.spec_from_file_location("prepare_signing_handoff", handoff_module)
        assert handoff_spec and handoff_spec.loader
        handoff = importlib.util.module_from_spec(handoff_spec)
        handoff_spec.loader.exec_module(handoff)
        with self.assertRaises(SystemExit):
            handoff.validate_authority_inputs("operator principal", "a" * 64)
        with self.assertRaises(SystemExit):
            handoff.validate_authority_inputs("asb-release", "A" * 64)

    def test_staging_accepts_bounded_operator_authority_shape(self) -> None:
        handoff_module = Path(__file__).with_name("prepare_signing_handoff.py")
        handoff_spec = importlib.util.spec_from_file_location("prepare_signing_handoff", handoff_module)
        assert handoff_spec and handoff_spec.loader
        handoff = importlib.util.module_from_spec(handoff_spec)
        handoff_spec.loader.exec_module(handoff)
        handoff.validate_authority_inputs("operator@example", "a" * 64)


if __name__ == "__main__":
    unittest.main()
