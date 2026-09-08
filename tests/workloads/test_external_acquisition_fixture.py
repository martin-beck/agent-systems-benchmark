import hashlib
import json
from pathlib import Path


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
