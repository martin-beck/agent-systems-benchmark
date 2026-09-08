import json
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).parents[2]
SCRIPT = ROOT / "tools/quality/validate_external_registry.py"


def test_external_registry_validator_reports_stable_digest():
    first = subprocess.check_output([sys.executable, str(SCRIPT)], text=True)
    second = subprocess.check_output([sys.executable, str(SCRIPT)], text=True)
    result = json.loads(first)
    assert result == json.loads(second)
    assert result["schema_version"] == 1
    assert result["workloads"] == 3
    assert len(result["registry_sha256"]) == 64
