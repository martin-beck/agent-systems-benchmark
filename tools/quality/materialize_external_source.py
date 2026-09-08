#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Materialize a verified external source archive without executing it."""

import argparse
import hashlib
import json
import tarfile
from pathlib import Path


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--destination", type=Path, required=True)
    args = parser.parse_args()
    if args.destination.exists():
        raise SystemExit("destination must not already exist")
    digest = hashlib.sha256(args.archive.read_bytes()).hexdigest()
    if digest != args.sha256:
        raise SystemExit("source archive digest mismatch")
    destination = args.destination.resolve()
    destination.mkdir(parents=True)
    try:
        with tarfile.open(args.archive, "r:*") as archive:
            members = archive.getmembers()
            for member in members:
                target = (destination / member.name).resolve()
                if destination not in target.parents and target != destination:
                    raise SystemExit("archive member escapes destination")
                if member.issym() or member.islnk() or member.isdev():
                    raise SystemExit("archive links and devices are not accepted")
            archive.extractall(destination, filter="data")
    except (tarfile.TarError, OSError) as exc:
        raise SystemExit(f"source archive could not be materialized: {exc}") from exc
    print(json.dumps({"sha256": digest, "destination": str(destination)}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
