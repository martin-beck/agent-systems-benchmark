# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
import json
from pathlib import Path

REGISTRY = (
    Path(__file__).parents[2]
    / "crates/asb-workloads/registry/v1/external-workloads.json"
)


def test_external_registry_is_pinned_and_non_vendored():
    data = json.loads(REGISTRY.read_text())
    assert data["schema_version"] == 1
    ids = [item["id"] for item in data["workloads"]]
    assert ids == [
        "swe-bench",
        "aider-polyglot",
        "exercism-tracks",
        "terminal-bench",
        "swe-perf",
        "swe-fficiency",
        "core-bench",
        "swe-bench-pro",
        "bigcodebench",
        "evalplus",
        "livecodebench",
        "swe-lancer",
        "swe-rebench",
    ]
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

    terminal = next(
        item for item in data["workloads"] if item["id"] == "terminal-bench"
    )
    assert terminal["dataset"]["task_count"] == 66
    assert terminal["dataset"]["task_reference_kind"] == "harbor-package-sha256"
    assert terminal["dataset"]["source_tree_matches_packages"] is False
    assert terminal["evaluator"]["provenance"]["status"] == "planned"
    assert terminal["network"]["status"] == "unqualified-upstream-default-public"
    assert terminal["reset"]["status"] == "unverified"

    for item in data["workloads"]:
        if item["id"] not in {"swe-perf", "swe-fficiency", "core-bench"}:
            continue
        assert item["evaluator"]["provenance"]["status"] == "planned"
        assert item["performance"]["correctness"] == "unqualified"
        assert item["performance"]["paired_trials"] == 0
        assert item["performance"]["uncertainty_method"] is None

    for item in [
        item for item in data["workloads"]
        if item["id"] in {"swe-bench-pro", "bigcodebench", "evalplus", "livecodebench"}
    ]:
        assert item["source"]["archive_status"] == "verified"
        assert len(item["source"]["commit"]) == 40
        assert len(item["source"]["archive_sha256"]) == 64
        assert len(item["dataset"]["revision"]) == 40
        assert item["evaluator"]["provenance"]["status"] == "planned"
        assert item["platforms"]["linux-aarch64"] == "unsupported-until-native-evidence"

    evolving = {
        item["id"]: item
        for item in data["workloads"]
        if item["id"] in {"swe-lancer", "swe-rebench"}
    }
    assert evolving["swe-lancer"]["source"]["archive_status"] == "unavailable-at-pinned-revision"
    assert evolving["swe-rebench"]["dataset"]["license"] == "CC-BY-4.0"
