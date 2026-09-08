# SPDX-License-Identifier: MIT
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).parents[2]
SCRIPT = ROOT / "tools/quality/run_external_workload.py"


def test_unqualified_evaluator_starts_no_subprocess(tmp_path):
    result = subprocess.run(
        [sys.executable, str(SCRIPT), "swe-bench", "--root", str(tmp_path), "--timeout-seconds", "5", "--", sys.executable, "-c", "raise SystemExit(99)"],
        capture_output=True,
        text=True,
    )
    assert result.returncode != 0
    assert "not qualified" in result.stderr


def test_unknown_evaluator_is_rejected(tmp_path):
    result = subprocess.run(
        [sys.executable, str(SCRIPT), "unknown", "--root", str(tmp_path), "--timeout-seconds", "5", "--", sys.executable, "-c", "raise SystemExit(99)"],
        capture_output=True,
        text=True,
    )
    assert result.returncode != 0
    assert "unknown" in result.stderr
