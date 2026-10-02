import importlib.util
import os
from pathlib import Path


ROOT = Path(__file__).parents[2]
SPEC = importlib.util.spec_from_file_location(
    "content_addressed_qualification", ROOT / "tools/run-content-addressed-qualification.py"
)
RUNNER = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(RUNNER)


def test_local_capture_is_public_and_fixture_bound():
    fixture = ROOT / "crates/asb-replay/fixtures/v1/buffered.json"
    capture = RUNNER.fixture_capture(fixture, "codex", RUNNER.PROFILE)
    assert capture["network"] == "loopback_only"
    assert capture["estimated_cost_minor"] == 0
    assert "synthetic-credential" not in str(capture)
    assert capture["contents"]["cassette_id"] == "fixture-buffered"


def test_subprocess_environment_strips_credentials_and_denies_network(monkeypatch):
    monkeypatch.setenv("OPENAI_API_KEY", "not-forwarded")
    monkeypatch.setenv("PROVIDER_TOKEN", "not-forwarded")
    monkeypatch.setenv("SSH_PRIVATE_KEY", "not-forwarded")
    environment = RUNNER.sanitized_environment()
    assert "OPENAI_API_KEY" not in environment
    assert "PROVIDER_TOKEN" not in environment
    assert "SSH_PRIVATE_KEY" not in environment
    assert environment["ASB_NETWORK"] == "denied"
    assert environment["ASB_LOCAL_MOCK"] == "1"


def test_receipt_digest_is_canonical_and_selection_is_sorted():
    document = {"selection": {"workloads": ["b", "a"], "agents": ["codex"]}, "network": "denied"}
    digest = RUNNER.digest_json(document)
    assert digest == RUNNER.digest_json(document)
    assert RUNNER.select(["codex"], RUNNER.ALL_AGENTS, RUNNER.DEFAULT_AGENTS, False) == ["codex"]
