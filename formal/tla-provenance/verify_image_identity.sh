#!/usr/bin/env bash
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
set -euo pipefail

if [[ $# != 3 ]]; then
    echo "usage: verify_image_identity.sh NAME_AT_DIGEST OS ARCH" >&2
    exit 64
fi

python3 -c '
import json
import re
import sys


def reject():
    print("OCI image identity rejected", file=sys.stderr)
    raise SystemExit(65)


def closed_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            reject()
        result[key] = value
    return result


try:
    expected, expected_os, expected_arch = sys.argv[1:]
    if not re.fullmatch(r"[a-z0-9][a-z0-9._/-]{0,127}@sha256:[0-9a-f]{64}", expected):
        reject()
    raw = sys.stdin.buffer.read(16_385)
    if not raw or len(raw) > 16_384:
        reject()
    document = json.loads(raw.decode("utf-8", errors="strict"), object_pairs_hook=closed_object)
    if not isinstance(document, dict) or set(document) != {
        "config_id", "repo_digests", "os", "architecture"
    }:
        reject()
    config_id = document["config_id"]
    repo_digests = document["repo_digests"]
    if not isinstance(config_id, str) or not re.fullmatch(r"sha256:[0-9a-f]{64}", config_id):
        reject()
    if not isinstance(repo_digests, list) or len(repo_digests) != 1:
        reject()
    if repo_digests[0] != expected:
        reject()
    if document["os"] != expected_os or document["architecture"] != expected_arch:
        reject()
except (UnicodeDecodeError, json.JSONDecodeError, TypeError, ValueError):
    reject()
' "$1" "$2" "$3"
