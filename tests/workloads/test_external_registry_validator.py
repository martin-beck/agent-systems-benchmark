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
    assert result["workloads"] == 4
    assert len(result["registry_sha256"]) == 64


def _terminal_document():
    return json.loads(
        (ROOT / "crates/asb-workloads/registry/v1/external-workloads.json").read_text()
    )


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
    document["workloads"][-1]["dataset"]["source_tree_matches_packages"] = True
    _rejects(tmp_path, document, "must not stand in")


def test_terminal_bench_rejects_missing_manifest_pin(tmp_path):
    document = _terminal_document()
    document["workloads"][-1]["dataset"]["manifest_sha256"] = "0" * 63
    _rejects(tmp_path, document, "manifest digest")


def test_terminal_bench_rejects_network_or_reset_upgrade_without_evidence(tmp_path):
    document = _terminal_document()
    network_upgrade = deepcopy(document)
    network_upgrade["workloads"][-1]["network"]["status"] = "isolated"
    _rejects(tmp_path, network_upgrade, "network default")
    document["workloads"][-1]["reset"]["status"] = "verified"
    _rejects(tmp_path, document, "reset must remain unverified")
