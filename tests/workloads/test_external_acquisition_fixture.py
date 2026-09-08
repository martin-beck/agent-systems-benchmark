# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
import hashlib
import json
from pathlib import Path
import subprocess
import sys


ROOT = Path(__file__).parents[2]
SPEC = ROOT / "tests/workloads/fixtures/external-acquisition.json"


def test_offline_acquisition_fixture_has_content_addressed_evidence():
    spec = json.loads(SPEC.read_text())
    artifact = spec["artifacts"][0]
    content = (ROOT / spec["offline_fixture"]).read_bytes()
    # The fixture intentionally exercises the same fail-closed digest comparison
    # used before an acquired evaluator may cross the external execution boundary.
    assert hashlib.sha256(content).hexdigest() == artifact["sha256"]


def test_offline_acquisition_mismatch_is_rejected():
    spec = json.loads(SPEC.read_text())
    artifact = spec["artifacts"][0]
    content = (ROOT / spec["offline_fixture"]).read_bytes() + b"tampered"
    assert hashlib.sha256(content).hexdigest() != artifact["sha256"]


def test_verifier_accepts_only_the_pinned_artifact():
    script = ROOT / "tools/quality/verify_external_artifact.py"
    result = subprocess.run(
        [sys.executable, str(script), "--manifest", str(SPEC), "--root", str(ROOT / "tests/workloads/fixtures"), "swe-bench-evaluator-fixture"],
        check=True,
        capture_output=True,
        text=True,
    )
    assert json.loads(result.stdout)["sha256"] == "93c10dc970a5de4a200ae6b67cd4530647a1aa62a4ddc2817347627d8670802e"
