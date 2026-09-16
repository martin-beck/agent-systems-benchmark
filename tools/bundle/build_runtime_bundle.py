#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Build, sign, verify, and reproducibly archive the ASB runtime helpers."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import os
import pathlib
import shutil
import stat
import subprocess
import tarfile
import tempfile
from typing import Any


ROOT = pathlib.Path(__file__).resolve().parents[2]
NAMESPACE = "asb-runtime-bundle-v1"


def digest(path: pathlib.Path) -> str:
    hasher = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            hasher.update(chunk)
    return hasher.hexdigest()


def canonical_json(value: Any) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()


def artifacts(root: pathlib.Path) -> list[dict[str, Any]]:
    result = []
    excluded = {"manifest.json", "manifest.json.sig", "sbom.spdx.json", "sbom.cdx.json"}
    for path in sorted(
        p for p in root.rglob("*")
        if p.is_file() and p.relative_to(root).as_posix() not in excluded
    ):
        relative = path.relative_to(root).as_posix()
        mode = stat.S_IMODE(path.stat().st_mode)
        item = {
            "path": relative,
            "size": path.stat().st_size,
            "sha256": digest(path),
            "mode": mode,
            "license_expression": "MIT",
            "license_evidence": ["LICENSE"],
        }
        if relative == "bin/asb_loopback_supervisor":
            item["role"] = "supervisor"
        elif relative == "bin/asb_loopback_sidecar":
            item["role"] = "sidecar"
        result.append(item)
    return result


def write_sboms(root: pathlib.Path, inventory: list[dict[str, Any]]) -> tuple[str, str]:
    spdx = {
        "SPDXID": "SPDXRef-DOCUMENT",
        "spdxVersion": "SPDX-2.3",
        "name": "ASB runtime bundle",
        "files": [
            {
                "SPDXID": f"SPDXRef-{item['path'].replace('/', '-')}",
                "fileName": item["path"],
                "checksums": [{"algorithm": "SHA256", "checksumValue": item["sha256"]}],
                "licenseConcluded": item["license_expression"],
            }
            for item in inventory
        ],
    }
    cyclonedx = {
        "bomFormat": "CycloneDX",
        "specVersion": "1.6",
        "version": 1,
        "components": [
            {
                "type": "file",
                "bom-ref": item["path"],
                "hashes": [{"alg": "SHA-256", "content": item["sha256"]}],
                "licenses": [{"expression": item["license_expression"]}],
            }
            for item in inventory
        ],
    }
    spdx_path = root / "sbom.spdx.json"
    cdx_path = root / "sbom.cdx.json"
    spdx_path.write_bytes(canonical_json(spdx))
    cdx_path.write_bytes(canonical_json(cyclonedx))
    return digest(spdx_path), digest(cdx_path)


def make_manifest(root: pathlib.Path, args: argparse.Namespace) -> None:
    inventory = artifacts(root)
    spdx_sha, cdx_sha = write_sboms(root, inventory)
    manifest = {
        "schema_version": 1,
        "bundle_id": args.bundle_id,
        "bundle_version": args.bundle_version,
        "target": {
            "operating_system": args.os,
            "architecture": args.arch,
            "libc": args.libc,
            "libc_version": args.libc_version,
        },
        "entrypoint": "bin/asb_loopback_supervisor",
        "artifacts": inventory,
        "runtime_components": {
            "supervisor": "bin/asb_loopback_supervisor",
            "sidecar": "bin/asb_loopback_sidecar",
        },
        "content_sha256": content_digest(inventory),
        "spdx": {"path": "sbom.spdx.json", "sha256": spdx_sha},
        "cyclonedx": {"path": "sbom.cdx.json", "sha256": cdx_sha},
    }
    (root / "manifest.json").write_bytes(canonical_json(manifest))


def content_digest(inventory: list[dict[str, Any]]) -> str:
    hasher = hashlib.sha256()
    for item in inventory:
        for value in (
            item["path"],
            str(item["size"]),
            item["sha256"],
            str(item["mode"]),
            item["license_expression"],
        ):
            encoded = value.encode()
            hasher.update(len(encoded).to_bytes(8, "big"))
            hasher.update(encoded)
        evidence = item["license_evidence"]
        hasher.update(len(evidence).to_bytes(8, "big"))
        for value in evidence:
            encoded = value.encode()
            hasher.update(len(encoded).to_bytes(8, "big"))
            hasher.update(encoded)
    return hasher.hexdigest()


def archive(root: pathlib.Path, destination: pathlib.Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    if destination.exists() or destination.is_symlink():
        raise SystemExit("refusing to overwrite an existing bundle archive")
    temporary = destination.with_name(f".{destination.name}.tmp-{os.getpid()}")
    try:
        with temporary.open("wb") as raw:
            with gzip.GzipFile(fileobj=raw, mode="wb", filename="", mtime=0) as compressed:
                with tarfile.open(fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT) as output:
                    for path in sorted(root.rglob("*")):
                        relative = path.relative_to(root).as_posix()
                        info = output.gettarinfo(str(path), arcname=relative)
                        info.uid = info.gid = 0
                        info.uname = info.gname = ""
                        info.mtime = 0
                        if path.is_file():
                            with path.open("rb") as source:
                                output.addfile(info, source)
                        else:
                            output.addfile(info)
        os.replace(temporary, destination)
    finally:
        temporary.unlink(missing_ok=True)


def parse() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bundle-id", default="asb-runtime")
    parser.add_argument("--bundle-version", required=True)
    parser.add_argument("--os", required=True)
    parser.add_argument("--arch", required=True)
    parser.add_argument("--libc", required=True)
    parser.add_argument("--libc-version", required=True)
    parser.add_argument("--key", type=pathlib.Path, required=True)
    parser.add_argument("--allowed-signers", type=pathlib.Path, required=True)
    parser.add_argument("--principal", required=True)
    parser.add_argument("--ssh-keygen", type=pathlib.Path, default=pathlib.Path("/usr/bin/ssh-keygen"))
    parser.add_argument("--ssh-keygen-sha256", required=True)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    parser.add_argument("--supervisor", type=pathlib.Path, required=True)
    parser.add_argument("--sidecar", type=pathlib.Path, required=True)
    return parser.parse_args()


def main() -> int:
    args = parse()
    if not args.key.is_file() or not args.allowed_signers.is_file():
        raise SystemExit("signing key and allowed-signers file must be regular files")
    for helper in (args.supervisor, args.sidecar):
        if not helper.is_file() or helper.is_symlink():
            raise SystemExit("supervisor and sidecar must be regular files")
    with tempfile.TemporaryDirectory(prefix="asb-runtime-bundle-") as temporary:
        stage = pathlib.Path(temporary)
        (stage / "bin").mkdir()
        for name, source in (
            ("asb_loopback_supervisor", args.supervisor),
            ("asb_loopback_sidecar", args.sidecar),
        ):
            destination = stage / "bin" / name
            shutil.copyfile(source, destination)
            destination.chmod(0o755)
        shutil.copyfile(ROOT / "LICENSE", stage / "LICENSE")
        (stage / "LICENSE").chmod(0o644)
        make_manifest(stage, args)
        subprocess.run([str(args.ssh_keygen), "-Y", "sign", "-f", str(args.key), "-n", NAMESPACE, str(stage / "manifest.json")], check=True)
        verifier = ROOT / "target" / "release" / "asb-bundle-verify"
        command = [str(verifier), str(stage), str(args.allowed_signers), args.principal, str(args.ssh_keygen), args.ssh_keygen_sha256, args.os, args.arch, args.libc, args.libc_version]
        subprocess.run(command, check=True)
        archive(stage, args.output)
    print(args.output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
