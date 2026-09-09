# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
import json
import subprocess
import sys
from copy import deepcopy
from pathlib import Path

ROOT = Path(__file__).parents[2]
SCRIPT = ROOT / "tools/quality/validate_external_registry.py"


def test_external_registry_validator_reports_stable_digest():
    first = subprocess.check_output([sys.executable, str(SCRIPT)], text=True)
    second = subprocess.check_output([sys.executable, str(SCRIPT)], text=True)
    result = json.loads(first)
    assert result == json.loads(second)
    assert result["schema_version"] == 1
    assert result["workloads"] == 13
    assert len(result["registry_sha256"]) == 64


def _terminal_document():
    return json.loads(
        (ROOT / "crates/asb-workloads/registry/v1/external-workloads.json").read_text()
    )


def _item(document, workload_id):
    return next(item for item in document["workloads"] if item["id"] == workload_id)


def _rejects(tmp_path, document, message):
    registry = tmp_path / "registry.json"
    registry.write_text(json.dumps(document))
    result = subprocess.run(
        [sys.executable, str(SCRIPT), "--registry", str(registry)],
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode != 0
    assert message in result.stderr


def test_terminal_bench_rejects_source_checkout_as_package_bytes(tmp_path):
    document = _terminal_document()
    _item(document, "terminal-bench")["dataset"]["source_tree_matches_packages"] = True
    _rejects(tmp_path, document, "must not stand in")


def test_terminal_bench_rejects_missing_manifest_pin(tmp_path):
    document = _terminal_document()
    _item(document, "terminal-bench")["dataset"]["manifest_sha256"] = "0" * 63
    _rejects(tmp_path, document, "manifest digest")


def test_terminal_bench_rejects_changed_source_or_harness_pin(tmp_path):
    document = _terminal_document()
    changed_source = deepcopy(document)
    _item(changed_source, "terminal-bench")["source"]["commit"] = "1" * 40
    _rejects(tmp_path, changed_source, "source commit")
    changed_harness = deepcopy(document)
    _item(changed_harness, "terminal-bench")["evaluator"]["archive_sha256"] = "2" * 64
    _rejects(tmp_path, changed_harness, "Harbor archive")


def test_terminal_bench_cannot_be_marked_qualified_without_native_evidence(tmp_path):
    document = _terminal_document()
    evaluator = _item(document, "terminal-bench")["evaluator"]
    evaluator["image_digest"] = "sha256:" + "1" * 64
    evaluator["provenance"] = {
        "status": "qualified",
        "sbom_sha256": "2" * 64,
        "evidence": "synthetic-evidence",
    }
    _rejects(tmp_path, document, "must remain unqualified")


def test_terminal_bench_rejects_network_or_reset_upgrade_without_evidence(tmp_path):
    document = _terminal_document()
    network_upgrade = deepcopy(document)
    _item(network_upgrade, "terminal-bench")["network"]["status"] = "isolated"
    _rejects(tmp_path, network_upgrade, "network default")
    _item(document, "terminal-bench")["reset"]["status"] = "verified"
    _rejects(tmp_path, document, "reset must remain unverified")


def test_performance_candidates_reject_missing_pins_and_false_qualification(tmp_path):
    document = _terminal_document()
    by_id = {item["id"]: item for item in document["workloads"]}
    for ident in ["swe-perf", "swe-fficiency", "core-bench"]:
        changed = deepcopy(document)
        changed_by_id = {item["id"]: item for item in changed["workloads"]}
        changed_by_id[ident]["dataset"]["revision"] = "0" * 40
        _rejects(tmp_path, changed, "dataset revision")

        changed = deepcopy(document)
        changed_by_id = {item["id"]: item for item in changed["workloads"]}
        evaluator = changed_by_id[ident]["evaluator"]
        evaluator["image_digest"] = "sha256:" + "1" * 64
        evaluator["provenance"] = {
            "status": "qualified",
            "sbom_sha256": "2" * 64,
            "evidence": "synthetic-evidence",
        }
        _rejects(tmp_path, changed, "must remain unqualified")

        changed = deepcopy(document)
        changed_by_id = {item["id"]: item for item in changed["workloads"]}
        changed_by_id[ident]["performance"]["correctness"] = "qualified"
        _rejects(tmp_path, changed, "must remain explicit")

    assert by_id["swe-perf"]["source"]["license"] == "NOASSERTION"
    assert by_id["swe-fficiency"]["dataset"]["license"] == "NOASSERTION"
    assert by_id["core-bench"]["evaluator"]["license"] == "NOASSERTION"


def test_additional_suites_reject_pin_or_qualification_drift(tmp_path):
    document = _terminal_document()
    for ident in ["swe-bench-pro", "bigcodebench", "evalplus", "livecodebench"]:
        changed = deepcopy(document)
        changed_by_id = {item["id"]: item for item in changed["workloads"]}
        changed_by_id[ident]["dataset"]["revision"] = "0" * 40
        _rejects(tmp_path, changed, f"{ident}: exact dataset revision")

        changed = deepcopy(document)
        changed_by_id = {item["id"]: item for item in changed["workloads"]}
        evaluator = changed_by_id[ident]["evaluator"]
        evaluator["image_digest"] = "sha256:" + "1" * 64
        evaluator["provenance"] = {
            "status": "qualified",
            "sbom_sha256": "2" * 64,
            "evidence": "synthetic-evidence",
        }
        _rejects(tmp_path, changed, f"{ident}: evaluator must remain unqualified")
