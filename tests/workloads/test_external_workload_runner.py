# SPDX-License-Identifier: MIT
import subprocess
import sys
import json
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


def test_qualified_disposable_oracle_runs_with_bounded_command(tmp_path):
    registry = tmp_path / "registry.json"
    registry.write_text(json.dumps({"workloads": [{
        "id": "fixture",
        "evaluator": {"provenance": {"status": "qualified", "sbom_sha256": "a" * 64, "evidence": "fixture-evidence-v1"}, "image_digest": "sha256:" + "b" * 64},
    }]}))
    oracle = ROOT / "tests/workloads/fixtures/oracle.py"
    result = subprocess.run(
        [sys.executable, str(SCRIPT), "fixture", "--root", str(tmp_path), "--registry", str(registry), "--", sys.executable, str(oracle)],
        check=True,
        capture_output=True,
        text=True,
    )
    assert json.loads(result.stdout.splitlines()[-1]) == {"exit_code": 0, "workload": "fixture"}


def test_oracle_result_validator_accepts_only_bounded_exact_schema(tmp_path):
    result_file = tmp_path / "result.json"
    result_file.write_text(json.dumps({"oracle": "fixture-v1", "score": 1}))
    validator = ROOT / "tools/quality/validate_oracle_result.py"
    result = subprocess.run([sys.executable, str(validator), str(result_file)], check=True, capture_output=True, text=True)
    assert json.loads(result.stdout) == {"oracle": "fixture-v1", "score": 1}

    result_file.write_text(json.dumps({"oracle": "fixture-v1", "score": 2, "raw": "secret"}))
    rejected = subprocess.run([sys.executable, str(validator), str(result_file)], capture_output=True, text=True)
    assert rejected.returncode != 0
    assert "exact" in rejected.stderr
