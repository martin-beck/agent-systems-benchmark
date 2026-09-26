#!/usr/bin/env python3
"""Build a deterministic, unsigned first-customer release manifest.

The command is deliberately offline: it consumes an already-built executable,
the checked-out tree, and a caller-supplied exact revision.  It never downloads
tools or embeds host paths, credentials, or logs.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import tempfile
from pathlib import Path

HEX40 = re.compile(r"^[0-9a-f]{40}$")
HEX64 = re.compile(r"^[0-9a-f]{64}$")


def digest(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def die(message: str) -> "NoReturn":
    raise SystemExit(message)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--source-revision", required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--profile", choices=("unsigned-release",), default="unsigned-release")
    args = parser.parse_args()
    if not HEX40.fullmatch(args.source_revision):
        die("source revision must be a lowercase 40-character commit")
    if subprocess.run(["git", "status", "--porcelain"], check=False, capture_output=True, text=True).stdout:
        die("release input tree is dirty")
    observed = subprocess.run(["git", "rev-parse", "HEAD"], check=True, capture_output=True, text=True).stdout.strip()
    if observed != args.source_revision:
        die("source revision does not match HEAD")
    if not args.binary.is_file() or args.binary.is_symlink():
        die("release binary must be a regular file")
    cargo_toml = Path("Cargo.toml").read_text(encoding="utf-8")
    version_match = re.search(
        r"(?ms)^\[workspace\.package\].*?^version\s*=\s*\"([^\"]+)\"",
        cargo_toml,
    )
    if version_match is None:
        die("workspace package version is missing")
    version = version_match.group(1)
    output = args.output.resolve()
    if output.exists():
        die("refusing to overwrite an existing release output")
    output.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="asb-release-", dir=output.parent) as temporary:
        root = Path(temporary)
        payload = root / "asb"
        shutil.copyfile(args.binary, payload)
        os.chmod(payload, 0o755)
        binary_sha = digest(payload)
        spdx = {
            "spdxVersion": "SPDX-2.3",
            "dataLicense": "CC0-1.0",
            "SPDXID": "SPDXRef-DOCUMENT",
            "name": "asb-release",
            "documentNamespace": f"https://agent-systems-benchmark.invalid/spdx/{args.source_revision}",
            "creationInfo": {"created": "1970-01-01T00:00:00Z", "creators": ["Tool: asb-release-manifest-v1"]},
            "packages": [{
                "SPDXID": "SPDXRef-Package-asb",
                "name": "asb",
                "versionInfo": version,
                "downloadLocation": "NOASSERTION",
                "filesAnalyzed": False,
            }],
        }
        cyclonedx = {
            "bomFormat": "CycloneDX",
            "specVersion": "1.6",
            "serialNumber": f"urn:uuid:{args.source_revision}",
            "version": 1,
            "metadata": {"component": {"type": "application", "name": "asb", "version": version}},
            "components": [],
        }
        (root / "sbom.spdx.json").write_text(json.dumps(spdx, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8")
        (root / "sbom.cyclonedx.json").write_text(json.dumps(cyclonedx, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8")
        manifest = {
            "schema_version": 1,
            "profile": args.profile,
            "version": version,
            "source_revision": args.source_revision,
            "artifacts": [{"path": "asb", "size": payload.stat().st_size, "sha256": binary_sha}],
            "sbom": {"spdx": "sbom.spdx.json", "cyclonedx": "sbom.cyclonedx.json"},
            "provenance": {"builder": "asb-release-manifest-v1", "network": "none", "credentials": "none"},
        }
        (root / "manifest.json").write_text(json.dumps(manifest, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8")
        lines = [f"{digest(path)}  {path.name}" for path in sorted(root.iterdir()) if path.name != "SHA256SUMS"]
        (root / "SHA256SUMS").write_text("\n".join(lines) + "\n", encoding="utf-8")
        os.replace(root, output)
    return 0


if __name__ == "__main__":
    main()
