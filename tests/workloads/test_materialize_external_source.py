# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
import hashlib
import io
import subprocess
import sys
import tarfile
from pathlib import Path


ROOT = Path(__file__).parents[2]
SCRIPT = ROOT / "tools/quality/materialize_external_source.py"


def make_archive(path: Path, name: str, payload: bytes):
    with tarfile.open(path, "w:gz") as archive:
        info = tarfile.TarInfo(name)
        info.size = len(payload)
        archive.addfile(info, io.BytesIO(payload))


def test_materializer_verifies_and_extracts(tmp_path):
    archive = tmp_path / "source.tar.gz"
    make_archive(archive, "source/README", b"pinned\n")
    destination = tmp_path / "out"
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    subprocess.run([sys.executable, str(SCRIPT), "--archive", str(archive), "--sha256", digest, "--destination", str(destination)], check=True)
    assert (destination / "source/README").read_bytes() == b"pinned\n"


def test_materializer_rejects_traversal(tmp_path):
    archive = tmp_path / "evil.tar.gz"
    make_archive(archive, "../escape", b"nope")
    destination = tmp_path / "out"
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    result = subprocess.run([sys.executable, str(SCRIPT), "--archive", str(archive), "--sha256", digest, "--destination", str(destination)], capture_output=True, text=True)
    assert result.returncode != 0
    assert "escapes" in result.stderr
