#!/usr/bin/env python3
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Prepare deterministic runtime-bundle staging for an external signer."""

from __future__ import annotations

import argparse
import json
import pathlib
import shutil
import stat
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(pathlib.Path(__file__).parent))
from build_runtime_bundle import digest, make_manifest  # noqa: E402

NAMESPACE = "asb-runtime-bundle-v1"


def stage(args: argparse.Namespace) -> None:
    destination = args.stage_output
    if destination.exists() or destination.is_symlink():
        raise SystemExit("refusing to overwrite an existing staging directory")
    for helper in (args.supervisor, args.sidecar):
        if not helper.is_file() or helper.is_symlink():
            raise SystemExit("supervisor and sidecar must be regular files")
    destination.mkdir(parents=True, mode=0o700)
    (destination / "bin").mkdir(mode=0o700)
    for name, source in (
        ("asb_loopback_supervisor", args.supervisor),
        ("asb_loopback_sidecar", args.sidecar),
    ):
        target = destination / "bin" / name
        shutil.copyfile(source, target)
        target.chmod(stat.S_IRUSR | stat.S_IWUSR | stat.S_IXUSR)
    license_path = ROOT / "LICENSE"
    shutil.copyfile(license_path, destination / "LICENSE")
    (destination / "LICENSE").chmod(stat.S_IRUSR | stat.S_IWUSR)
    make_manifest(
        destination,
        argparse.Namespace(
            profile="signed",
            bundle_id=args.bundle_id,
            bundle_version=args.bundle_version,
            os=args.os,
            arch=args.arch,
            libc=args.libc,
            libc_version=args.libc_version,
        ),
    )
    manifest = json.loads((destination / "manifest.json").read_text())
    handoff = {
        "schema_version": 1,
        "qualification": "non-production signing handoff",
        "bundle_id": manifest["bundle_id"],
        "bundle_version": manifest["bundle_version"],
        "target": manifest["target"],
        "manifest_sha256": digest(destination / "manifest.json"),
        "content_sha256": manifest["content_sha256"],
        "required_signature": {
            "path": "manifest.json.sig",
            "namespace": NAMESPACE,
            "principal": args.principal,
            "allowed_signers_filename": "ALLOWED_SIGNERS",
            "ssh_keygen_sha256": args.ssh_keygen_sha256,
        },
        "verification": {
            "signature_must_be_supplied_externally": True,
            "command": "asb-bundle-verify BUNDLE ALLOWED_SIGNERS PRINCIPAL SSH_KEYGEN SSH_KEYGEN_SHA256 OS ARCH LIBC LIBC_VERSION",
            "release_claim": "blocked until detached signature verification succeeds",
        },
    }
    args.handoff_output.parent.mkdir(parents=True, exist_ok=True)
    if args.handoff_output.exists() or args.handoff_output.is_symlink():
        raise SystemExit("refusing to overwrite an existing handoff document")
    args.handoff_output.write_text(json.dumps(handoff, sort_keys=True, indent=2) + "\n")
    print(args.stage_output)


def parse() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bundle-id", default="asb-runtime")
    parser.add_argument("--bundle-version", required=True)
    parser.add_argument("--os", required=True)
    parser.add_argument("--arch", required=True)
    parser.add_argument("--libc", required=True)
    parser.add_argument("--libc-version", required=True)
    parser.add_argument("--principal", required=True)
    parser.add_argument("--ssh-keygen-sha256", required=True)
    parser.add_argument("--supervisor", type=pathlib.Path, required=True)
    parser.add_argument("--sidecar", type=pathlib.Path, required=True)
    parser.add_argument("--stage-output", type=pathlib.Path, required=True)
    parser.add_argument("--handoff-output", type=pathlib.Path, required=True)
    return parser.parse_args()


if __name__ == "__main__":
    stage(parse())
