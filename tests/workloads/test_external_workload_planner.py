# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).parents[2]
SCRIPT = ROOT / "tools/quality/plan_external_workload.py"


def test_planner_fails_closed_for_unqualified_evaluator():
    result = subprocess.run(
        [sys.executable, str(SCRIPT), "swe-bench"],
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 2
    assert json.loads(result.stdout) == {
        "workload": "swe-bench",
        "status": "unqualified",
        "reason": "evaluator provenance is not qualified",
    }


def test_terminal_bench_planner_fails_closed_before_native_qualification():
    result = subprocess.run(
        [sys.executable, str(SCRIPT), "terminal-bench"],
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 2
    assert json.loads(result.stdout) == {
        "workload": "terminal-bench",
        "status": "unqualified",
        "reason": "evaluator provenance is not qualified",
    }


def test_performance_and_reproducibility_candidates_fail_closed():
    for workload in ["swe-perf", "swe-fficiency", "core-bench"]:
        result = subprocess.run(
            [sys.executable, str(SCRIPT), workload],
            capture_output=True,
            text=True,
            check=False,
        )
        assert result.returncode == 2
        assert json.loads(result.stdout) == {
            "workload": workload,
            "status": "unqualified",
            "reason": "evaluator provenance is not qualified",
        }


def test_planner_rejects_unknown_workload():
    result = subprocess.run(
        [sys.executable, str(SCRIPT), "missing"],
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode != 0
    assert "unknown" in result.stderr


def test_planner_accepts_only_matching_reviewed_qualification(tmp_path):
    registry = json.loads((ROOT / "crates/asb-workloads/registry/v1/external-workloads.json").read_text())
    item = next(item for item in registry["workloads"] if item["id"] == "terminal-bench")
    item["evaluator"]["image_digest"] = "sha256:" + "a" * 64
    item["evaluator"]["provenance"] = {"status": "qualified", "sbom_sha256": "b" * 64, "evidence": "fixture"}
    registry_path = tmp_path / "registry.json"
    registry_path.write_text(json.dumps(registry))
    fixture = ROOT / "tests/workloads/fixtures/external-qualification.json"
    result = subprocess.run(
        [sys.executable, str(SCRIPT), "terminal-bench", "--registry", str(registry_path), "--qualification", str(fixture)],
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 0
    assert json.loads(result.stdout)["status"] == "planned"

    wrong = json.loads(fixture.read_text())
    wrong["workload"] = "swe-bench"
    wrong_path = tmp_path / "wrong.json"
    wrong_path.write_text(json.dumps(wrong))
    result = subprocess.run(
        [sys.executable, str(SCRIPT), "terminal-bench", "--registry", str(registry_path), "--qualification", str(wrong_path)],
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode != 0
    assert "does not match" in result.stderr
