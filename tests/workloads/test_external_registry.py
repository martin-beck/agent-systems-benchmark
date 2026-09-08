# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
import json
from pathlib import Path


REGISTRY = Path(__file__).parents[2] / "crates/asb-workloads/registry/v1/external-workloads.json"


def test_external_registry_is_pinned_and_non_vendored():
    data = json.loads(REGISTRY.read_text())
    assert data["schema_version"] == 1
    ids = [item["id"] for item in data["workloads"]]
    assert ids == ["swe-bench", "aider-polyglot", "exercism-tracks"]
    for item in data["workloads"]:
        assert item["dataset"]["vendored"] is False
        assert item["dataset"]["acquisition"] == "explicit-download"
        assert item["evaluator"]["image_digest"] is None
        assert len(item["evaluator"]["version"]) >= 8
        assert item["platforms"]["linux-aarch64"] in {
            "planned",
            "planned-with-toolchain-evidence",
            "unsupported-until-native-evidence",
        }
