# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).parents[2]
GENERATOR = ROOT / "tools/quality/generate_workload_catalog.py"
CHECKER = ROOT / "tools/quality/check_workload_catalog_parity.py"
EXPECTED = ROOT / "docs/generated/workload-catalog-v1.json"


def test_generated_catalog_is_current():
    result = subprocess.run(
        [sys.executable, str(GENERATOR)], capture_output=True, text=True, check=False
    )
    assert result.returncode == 0, result.stderr


def test_cli_catalog_checker_rejects_drift(tmp_path):
    value = json.loads(EXPECTED.read_text())
    value["entries"][0]["evidence"] = "qualified"
    value.update({"command": "workload-catalog", "ok": True})
    drifted = tmp_path / "catalog.json"
    drifted.write_text(json.dumps(value))
    result = subprocess.run(
        [sys.executable, str(CHECKER), str(drifted)],
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode != 0
    assert "diverges" in result.stderr
