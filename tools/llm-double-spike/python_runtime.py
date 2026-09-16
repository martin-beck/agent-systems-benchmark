# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Validate the pinned, credential-free Python fixture runtime identity."""
from __future__ import annotations

import json
import re
from pathlib import Path

MANIFEST = Path(__file__).with_name("python-runtime-v1.json")
IMAGE = "python@sha256:ed86c82274b3c69b52fb5820f358f0bd7df0b603332063cb5c6e32bd220c3e6e"
REQUIRED = {"schema_version", "runtime", "version", "architecture", "image", "network", "site_initialization", "source", "source_revision"}


class RuntimeError(ValueError):
    """The runtime manifest is malformed or not the reviewed identity."""


def load_manifest(path: Path = MANIFEST) -> dict[str, object]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if set(value) != REQUIRED:
        raise RuntimeError("runtime manifest fields are not closed")
    if value["schema_version"] != 1 or value["runtime"] != "python" or value["version"] != "3.13.15":
        raise RuntimeError("unsupported Python runtime")
    if value["source"] != "docker-library/python" or value["source_revision"] != "python:3.13.15-slim-bookworm":
        raise RuntimeError("runtime source provenance does not match the reviewed release")
    if value["architecture"] != "amd64" or value["image"] != IMAGE:
        raise RuntimeError("runtime image identity does not match the reviewed digest")
    if value["network"] != "none" or value["site_initialization"] != "disabled":
        raise RuntimeError("runtime isolation contract is incomplete")
    if not re.fullmatch(r"python@sha256:[0-9a-f]{64}", str(value["image"])):
        raise RuntimeError("runtime image is not an immutable digest")
    return value


if __name__ == "__main__":
    print(json.dumps(load_manifest(), sort_keys=True))
