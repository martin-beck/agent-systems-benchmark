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
