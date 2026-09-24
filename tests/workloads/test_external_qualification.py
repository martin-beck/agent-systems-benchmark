# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
import json
import subprocess
import sys
from copy import deepcopy
from pathlib import Path

ROOT = Path(__file__).parents[2]
FIXTURE = ROOT / "tests/workloads/fixtures/external-qualification.json"
SCRIPT = ROOT / "tools/quality/validate_external_qualification.py"


def _run(tmp_path, document):
    path = tmp_path / "qualification.json"
    path.write_text(json.dumps(document))
    return subprocess.run([sys.executable, str(SCRIPT), str(path)], capture_output=True, text=True, check=False)


def test_qualification_fixture_is_verified_and_deterministic():
    first = subprocess.check_output([sys.executable, str(SCRIPT), str(FIXTURE)], text=True)
    second = subprocess.check_output([sys.executable, str(SCRIPT), str(FIXTURE)], text=True)
    assert json.loads(first) == json.loads(second)
    assert json.loads(first)["status"] == "qualified"


def test_qualification_rejects_missing_evidence(tmp_path):
    document = json.loads(FIXTURE.read_text())
    document["reset"]["status"] = "unverified"
    result = _run(tmp_path, document)
    assert result.returncode == 2
    assert "reset must contain verified evidence" in result.stderr


def test_qualification_rejects_mutable_or_mixed_identity(tmp_path):
    document = json.loads(FIXTURE.read_text())
    document["dataset"]["revision"] = "main"
    result = _run(tmp_path, document)
    assert result.returncode == 2
    assert "dataset.revision" in result.stderr

    document = json.loads(FIXTURE.read_text())
    document["evaluator"]["image_digest"] = None
    result = _run(tmp_path, document)
    assert result.returncode == 2
    assert "evaluator.image_digest" in result.stderr


def test_qualification_rejects_unknown_fields(tmp_path):
    document = deepcopy(json.loads(FIXTURE.read_text()))
    document["oracle"]["score"] = 1
    result = _run(tmp_path, document)
    assert result.returncode == 2
    assert "unknown fields" in result.stderr
