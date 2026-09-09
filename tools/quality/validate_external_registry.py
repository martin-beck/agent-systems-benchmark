#!/usr/bin/env python3
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Validate the opt-in external workload provenance registry without acquiring data."""

import argparse
import hashlib
import json
import re
import sys
from pathlib import Path
from typing import Any

SHA = re.compile(r"^[0-9a-f]{40}$")
SHA256 = re.compile(r"^[0-9a-f]{64}$")
TERMINAL_BENCH_SOURCE_COMMIT = "452bf305c6daa62fc59061d22133a7cbc7c1572e"
TERMINAL_BENCH_SOURCE_ARCHIVE = (
    "390ee198a0f02fcdf140ac21420e106ce98f26d5f028b3d16b73e2d5137ff392"
)
TERMINAL_BENCH_DATASET_MANIFEST = (
    "ecd296ba053840bd4c0068e8f84e8a6fa829d184d0fd9852becdc19f4c895fcf"
)
HARBOR_COMMIT = "4407eb5227a2ff4f0d3f16b2eb48849382fdf276"
HARBOR_ARCHIVE = "04ec6b077d610896d75ed85b6b5ff88a9a241da6d528419acca66d2307329a21"
APACHE_LICENSE = "c71d239df91726fc519c6eb72d318ec65820627232b2f796219e87dcf35d0ab4"
PERFORMANCE_PINS = {
    "swe-perf": {
        "source": "9e8fed285ed640b51baaf66b13773706dc95aa45",
        "archive": "eae21438a7ed072f5a0befd3849f40d6c92d947dea532a2ede55b71487d1f1dd",
        "dataset": "6298f128c550e92ac25af17653cf092224046516",
    },
    "swe-fficiency": {
        "source": "12d32a2d6800824a7d84bdb6797b5708e7b7957f",
        "archive": "f8d0c3b895dd67d46da8e534092af2986694cb2de494c61547963006964a1945",
        "dataset": "7fbfe6c2f1f409afb17acc4c3f737991ec6e1c27",
    },
    "core-bench": {
        "source": "e32a2980e72fe6eb04ee04eb749458f570625663",
        "archive": "1be7112b9c06e19cf0f44051200dca73d8107ec43537d185a2008d9f82c2aa68",
        "dataset": "18ac8edf2532d9edb9d13ae71f715410de6ee5a0",
    },
}
ADDITIONAL_SUITE_PINS = {
    "swe-bench-pro": {
        "version": "main-ca10a60a",
        "source": "ca10a60a5fcae51e6948ffe1485d4153d421e6c5",
        "archive": "c1058143eba29553d02fd8cb9c55ec2f0c04aa7093f4ba6fc5af43b57f5d73e0",
        "dataset": "7ab5114912baf22bb098818e604c02fe7ad2c11f",
    },
    "bigcodebench": {
        "version": "v0.2.4-9059fb84",
        "source": "9059fb84d1188c02edeac4995361656a2fdecbef",
        "archive": "b95cc1e4adc1d1d1f888785ce5614930d11097805a3e6925e2b948f1b93683b1",
        "dataset": "b74c0d0bf70d2c0bc459be537895cca163007f1a",
    },
    "evalplus": {
        "version": "v0.3.1-e5d0ed0b",
        "source": "e5d0ed0bab96280b60b637ec7f15b5e4841b0cb2",
        "archive": "490f08728db1da8a73df89ada48f8ad0270fbd667895838348e64506095a2d49",
        "dataset": "d32357cf319e50e9c8d8dab5ea876c72b0fd321b",
    },
    "livecodebench": {
        "version": "release-v6-28fef95e",
        "source": "28fef95ea8c9f7a547c8329f2cd3d32b92c1fa24",
        "archive": "cd2b592e51ef9aec0f8e92ebb2b8b579f1e519fbcaaeb9e63e4f0ffbf3f18bb1",
        "dataset": "0fe84c3912ea0c4d4a78037083943e8f0c4dd505",
    },
}
REGISTRY = (
    Path(__file__).parents[2]
    / "crates/asb-workloads/registry/v1/external-workloads.json"
)


def fail(message: str) -> None:
    raise SystemExit(f"external registry: {message}")


def validate_terminal_bench(item: dict[str, Any]) -> None:
    source = item.get("source", {})
    dataset = item.get("dataset", {})
    evaluator = item.get("evaluator", {})
    if item.get("version") != "v4.0.0-452bf305":
        fail("terminal-bench: exact v4 source version is required")
    if source.get("commit") != TERMINAL_BENCH_SOURCE_COMMIT:
        fail("terminal-bench: exact source commit is required")
    if source.get("archive_sha256") != TERMINAL_BENCH_SOURCE_ARCHIVE:
        fail("terminal-bench: exact source archive digest is required")
    if source.get("license_sha256") != APACHE_LICENSE:
        fail("terminal-bench: exact source license digest is required")
    if dataset.get("manifest_sha256") != TERMINAL_BENCH_DATASET_MANIFEST:
        fail("terminal-bench: exact dataset manifest digest is required")
    if dataset.get("task_count") != 66:
        fail("terminal-bench: dataset task count must match the pinned manifest")
    if dataset.get("task_reference_kind") != "harbor-package-sha256":
        fail("terminal-bench: Harbor package digest semantics are required")
    if dataset.get("source_tree_matches_packages") is not False:
        fail("terminal-bench: source checkout must not stand in for package bytes")
    if (
        evaluator.get("entrypoint") != "harbor run"
        or evaluator.get("version") != HARBOR_COMMIT
    ):
        fail("terminal-bench: exact Harbor harness identity is required")
    if evaluator.get("archive_sha256") != HARBOR_ARCHIVE:
        fail("terminal-bench: exact Harbor archive digest is required")
    if evaluator.get("license_sha256") != APACHE_LICENSE:
        fail("terminal-bench: exact Harbor license digest is required")
    if evaluator.get("image_digest") is not None or evaluator.get("provenance") != {
        "status": "planned",
        "sbom_sha256": None,
        "evidence": None,
    }:
        fail(
            "terminal-bench: evaluator must remain unqualified without native evidence"
        )
    if item.get("network") != {"status": "unqualified-upstream-default-public"}:
        fail("terminal-bench: unresolved public network default must remain explicit")
    if item.get("reset") != {"status": "unverified"}:
        fail("terminal-bench: reset must remain unverified without native evidence")


def validate_performance_candidate(item: dict[str, Any]) -> None:
    ident = item["id"]
    pins = PERFORMANCE_PINS[ident]
    source = item.get("source", {})
    dataset = item.get("dataset", {})
    evaluator = item.get("evaluator", {})
    if (
        source.get("commit") != pins["source"]
        or source.get("archive_sha256") != pins["archive"]
    ):
        fail(f"{ident}: exact source and archive identities are required")
    if dataset.get("revision") != pins["dataset"]:
        fail(f"{ident}: exact dataset revision is required")
    if evaluator.get("image_digest") is not None or evaluator.get("provenance") != {
        "status": "planned",
        "sbom_sha256": None,
        "evidence": None,
    }:
        fail(f"{ident}: evaluator must remain unqualified without native evidence")
    expected_timing = "not-applicable" if ident == "core-bench" else "unqualified"
    if item.get("performance") != {
        "correctness": "unqualified",
        "timing": expected_timing,
        "paired_trials": 0,
        "uncertainty_method": None,
        "hardware_control": "unqualified",
    }:
        fail(f"{ident}: performance and reproducibility evidence must remain explicit")
    if (
        ident == "swe-perf"
        and source.get("license_status") != "missing-at-pinned-revision"
    ):
        fail("swe-perf: missing source license must remain explicit")
    if ident == "swe-fficiency" and dataset.get("license") != "NOASSERTION":
        fail("swe-fficiency: missing dataset license must remain explicit")
    if (
        ident == "core-bench"
        and evaluator.get("license_status") != "missing-at-pinned-revision"
    ):
        fail("core-bench: missing recommended evaluator license must remain explicit")


def validate_additional_suite(item: dict[str, Any]) -> None:
    ident = item["id"]
    pins = ADDITIONAL_SUITE_PINS[ident]
    source = item.get("source", {})
    dataset = item.get("dataset", {})
    evaluator = item.get("evaluator", {})
    if item.get("version") != pins["version"]:
        fail(f"{ident}: exact suite version is required")
    if source.get("commit") != pins["source"]:
        fail(f"{ident}: exact source commit is required")
    if source.get("archive_sha256") != pins["archive"]:
        fail(f"{ident}: exact source archive digest is required")
    if dataset.get("revision") != pins["dataset"]:
        fail(f"{ident}: exact dataset revision is required")
    if evaluator.get("image_digest") is not None or evaluator.get("provenance") != {
        "status": "planned",
        "sbom_sha256": None,
        "evidence": None,
    }:
        fail(f"{ident}: evaluator must remain unqualified without native evidence")
    if source.get("license") not in {"MIT", "Apache-2.0"}:
        fail(f"{ident}: source license must be an explicit SPDX license")
    if dataset.get("vendored") is not False or dataset.get("acquisition") != "explicit-download":
        fail(f"{ident}: dataset must remain explicit-download and non-vendored")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--registry", type=Path, default=REGISTRY)
    args = parser.parse_args()
    try:
        data = json.loads(args.registry.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        fail(f"cannot parse registry: {exc}")
    if data.get("schema_version") != 1 or not isinstance(data.get("workloads"), list):
        fail("schema_version 1 and workloads list are required")
    seen = set()
    for item in data["workloads"]:
        ident = item.get("id")
        if not isinstance(ident, str) or not ident or ident in seen:
            fail(f"duplicate or invalid workload id: {ident!r}")
        seen.add(ident)
        source = item.get("source", {})
        if source.get("archive_status") not in {
            "verified",
            "unverified",
            "unavailable-at-pinned-revision",
        }:
            fail(f"{ident}: archive_status must be explicit")
        if source.get("archive_status") == "verified":
            archive_digest = source.get("archive_sha256")
            if isinstance(archive_digest, dict):
                if not archive_digest or any(
                    not SHA256.fullmatch(value) for value in archive_digest.values()
                ):
                    fail(
                        f"{ident}: every verified repository archive requires a SHA-256 identity"
                    )
            elif not SHA256.fullmatch(archive_digest or ""):
                fail(f"{ident}: verified archive requires a SHA-256 identity")
        commits = (
            [source.get("commit")]
            if source.get("commit")
            else [x.split("@", 1)[1] for x in source.get("repositories", [])]
        )
        if not commits or any(not SHA.fullmatch(commit) for commit in commits):
            fail(
                f"{ident}: every source revision must be a 40-character lowercase commit"
            )
        dataset = item.get("dataset", {})
        if (
            dataset.get("vendored") is not False
            or dataset.get("acquisition") != "explicit-download"
        ):
            fail(
                f"{ident}: external datasets must be explicit-download and non-vendored"
            )
        evaluator = item.get("evaluator", {})
        if not evaluator.get("version") or not evaluator.get("entrypoint"):
            fail(f"{ident}: evaluator identity is incomplete")
        if evaluator.get("image_digest") is not None and not re.fullmatch(
            r"sha256:[0-9a-f]{64}", str(evaluator["image_digest"])
        ):
            fail(f"{ident}: image_digest must be sha256:... or null while planned")
        provenance = evaluator.get("provenance", {})
        if provenance.get("status") not in {"planned", "qualified"}:
            fail(f"{ident}: evaluator provenance status is invalid")
        if provenance.get("status") == "qualified" and (
            not evaluator.get("image_digest")
            or not provenance.get("sbom_sha256")
            or not provenance.get("evidence")
        ):
            fail(
                f"{ident}: qualified evaluator requires image, SBOM, and evidence identities"
            )
        if not item.get("limitations"):
            fail(f"{ident}: limitations must be explicit")
        if ident == "terminal-bench":
            validate_terminal_bench(item)
        if ident in PERFORMANCE_PINS:
            validate_performance_candidate(item)
        if ident in ADDITIONAL_SUITE_PINS:
            validate_additional_suite(item)
    digest = hashlib.sha256(args.registry.read_bytes()).hexdigest()
    print(
        json.dumps(
            {"schema_version": 1, "workloads": len(seen), "registry_sha256": digest},
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
