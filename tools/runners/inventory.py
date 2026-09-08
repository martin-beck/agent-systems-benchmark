# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Emit a bounded deterministic inventory of an immutable runner installation."""

from __future__ import annotations

import hashlib
import json
import os
import stat
import sys
from pathlib import Path

MAX_ENTRIES = 20_000
MAX_FILE_BYTES = 256 * 1024 * 1024
MAX_TOTAL_BYTES = 1024 * 1024 * 1024
MAX_RELATIVE_BYTES = 4096
SKIP_ROOT = {"_work", "_diag", ".runner", ".credentials", ".credentials_rsaparams"}


def fail() -> None:
    raise RuntimeError("installation inventory failed")


def digest_file(path: Path, before: os.stat_result) -> str:
    flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_CLOEXEC", 0)
    descriptor = os.open(path, flags)
    try:
        current = os.fstat(descriptor)
        if (
            current.st_dev,
            current.st_ino,
            current.st_mode,
            current.st_uid,
            current.st_gid,
            current.st_size,
        ) != (
            before.st_dev,
            before.st_ino,
            before.st_mode,
            before.st_uid,
            before.st_gid,
            before.st_size,
        ):
            fail()
        digest = hashlib.sha256()
        while chunk := os.read(descriptor, 1024 * 1024):
            digest.update(chunk)
        return digest.hexdigest()
    finally:
        os.close(descriptor)


def inventory(root: Path, operator_uid: int, job_gid: int) -> list[list[object]]:
    resolved_root = root.resolve(strict=True)
    records: list[list[object]] = []
    stack = [resolved_root]
    total_bytes = 0
    while stack:
        directory = stack.pop()
        with os.scandir(directory) as scanner:
            entries = sorted(
                scanner,
                key=lambda entry: os.fsencode(entry.name),
                reverse=True,
            )
        for entry in entries:
            path = Path(entry.path)
            relative = path.relative_to(resolved_root)
            if relative.parent == Path(".") and relative.name in SKIP_ROOT:
                continue
            encoded = os.fsencode(str(relative))
            if not encoded or len(encoded) > MAX_RELATIVE_BYTES or b"\0" in encoded:
                fail()
            metadata = entry.stat(follow_symlinks=False)
            if metadata.st_uid != operator_uid or metadata.st_gid != job_gid:
                fail()
            mode = stat.S_IMODE(metadata.st_mode)
            if len(records) >= MAX_ENTRIES:
                fail()
            if stat.S_ISLNK(metadata.st_mode):
                target = os.readlink(path)
                if os.path.isabs(target):
                    fail()
                resolved_target = path.resolve(strict=True)
                if not resolved_target.is_relative_to(resolved_root):
                    fail()
                records.append(
                    ["l", operator_uid, job_gid, f"{mode:o}", str(relative), target]
                )
            elif stat.S_ISDIR(metadata.st_mode):
                if mode & 0o022:
                    fail()
                records.append(["d", operator_uid, job_gid, f"{mode:o}", str(relative)])
                stack.append(path)
            elif stat.S_ISREG(metadata.st_mode):
                if mode & 0o022:
                    fail()
                if metadata.st_size > MAX_FILE_BYTES:
                    fail()
                total_bytes += metadata.st_size
                if total_bytes > MAX_TOTAL_BYTES:
                    fail()
                records.append(
                    [
                        "f",
                        operator_uid,
                        job_gid,
                        f"{mode:o}",
                        str(relative),
                        metadata.st_size,
                        digest_file(path, metadata),
                    ]
                )
            else:
                fail()
    records.sort(key=lambda record: os.fsencode(str(record[4])))
    return records


def main() -> int:
    if len(sys.argv) != 4:
        return 2
    try:
        root = Path(sys.argv[1])
        operator_uid = int(sys.argv[2])
        job_gid = int(sys.argv[3])
        if operator_uid < 0 or job_gid < 0:
            fail()
        for record in inventory(root, operator_uid, job_gid):
            print(json.dumps(record, ensure_ascii=True, separators=(",", ":")))
    except (OSError, RuntimeError, ValueError):
        print("ERROR: installation inventory failed", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
