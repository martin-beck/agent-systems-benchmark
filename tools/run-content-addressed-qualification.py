#!/usr/bin/env python3
"""Run the credential-free content-addressed ASB qualification lane.

The runner is intentionally a thin coordinator around the canonical ``asb``
commands.  It creates only local/mock captures, seals them through
``record-campaign``, reopens every cassette through strict offline replay, and
writes a small receipt containing identities and digests (never capture
contents, credentials, or provider endpoints).
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any, Sequence

SCHEMA_VERSION = 1
WORKFLOW_SCHEMA_VERSION = 1
PROFILE = "a" * 64
DEFAULT_AGENTS = ("codex",)
DEFAULT_WORKLOADS = ("original.bug-fix",)
# These are the repository's fixture-only workload identities.  The runner
# never expands this list by downloading a dataset or contacting a provider.
ALL_WORKLOADS = ("agentbench", "original.bug-fix", "swe-bench", "terminal-bench")
ALL_AGENTS = ("aider", "codex", "gemini", "goose", "mini_swe", "openhands", "opencode", "opendesk", "qwen_code")


def canonical(value: Any) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True).encode()


def digest_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def digest_json(value: Any) -> str:
    return digest_bytes(canonical(value))


def git_revision(root: Path) -> str:
    result = subprocess.run(
        ["git", "rev-parse", "HEAD"], cwd=root, check=True, capture_output=True, text=True
    )
    return result.stdout.strip()


def sanitized_environment() -> dict[str, str]:
    """Return a subprocess environment that cannot forward credentials.

    The local/mock lane has no provider authority.  Keep ordinary execution
    variables needed to find the exact binary, but strip common credential and
    private-key spellings even when a developer has them in the shell.
    """
    forbidden = ("API_KEY", "TOKEN", "PASSWORD", "PASSWD", "SECRET", "CREDENTIAL", "PRIVATE_KEY")
    environment = {
        key: value for key, value in os.environ.items()
        if not any(word in key.upper() for word in forbidden)
    }
    environment.update({"ASB_NETWORK": "denied", "ASB_LOCAL_MOCK": "1"})
    return environment


def command(binary: Sequence[str], args: Sequence[str], root: Path) -> tuple[int, Any, str]:
    result = subprocess.run(
        [*binary, *args], cwd=root, check=False, capture_output=True, text=True,
        env=sanitized_environment(),
    )
    try:
        output = json.loads(result.stdout)
    except json.JSONDecodeError:
        output = None
    return result.returncode, output, result.stderr.strip()


def fixture_capture(fixture: Path, agent: str, profile: str) -> dict[str, Any]:
    cassette = json.loads(fixture.read_text(encoding="utf-8"))
    # The fixture is public synthetic material.  Keep its response and request
    # body intact, but bind the envelope to the selected local identity.
    return {
        "schema_version": WORKFLOW_SCHEMA_VERSION,
        "provider_profile_sha256": profile,
        "agent_id": agent,
        "network": "loopback_only",
        "estimated_cost_minor": 0,
        "confirmation": {"record": True, "network": True, "cost": False},
        "contents": cassette["contents"],
    }


def select(values: Sequence[str] | None, all_values: Sequence[str], defaults: Sequence[str], all_flag: bool) -> list[str]:
    selected = list(all_values if all_flag else (values or defaults))
    if not selected or len(set(selected)) != len(selected):
        raise ValueError("selection must be non-empty and duplicate-free")
    return sorted(selected)


def run_campaign(
    root: Path,
    binary: Sequence[str],
    fixture: Path,
    agents: Sequence[str],
    workloads: Sequence[str],
    workspace: Path,
) -> dict[str, Any]:
    captures = workspace / "captures"
    cassettes = workspace / "cassettes"
    captures.mkdir()
    cassettes.mkdir()
    entries = []
    for agent in agents:
        for workload in workloads:
            capture = captures / f"{agent}-{workload}.json"
            cassette = cassettes / f"{agent}-{workload}.json"
            capture.write_bytes(canonical(fixture_capture(fixture, agent, PROFILE)))
            entries.append({"workload_id": workload, "capture_path": str(capture), "cassette_path": str(cassette)})
    manifest = workspace / "campaign.json"
    manifest.write_bytes(canonical({
        "schema_version": WORKFLOW_SCHEMA_VERSION,
        "provider_profile_sha256": PROFILE,
        "agent_ids": list(agents),
        "workload_ids": list(workloads),
        "entries": entries,
    }))
    code, output, error = command(binary, ["record-campaign", str(manifest), "--local-mock", "--json"], root)
    if code != 0 or not isinstance(output, dict) or not output.get("complete_coverage"):
        raise RuntimeError(f"local-mock campaign failed: {error or output}")
    recordings = output.get("recordings", [])
    if len(recordings) != len(entries):
        raise RuntimeError("campaign returned incomplete recording metadata")
    replays = []
    for entry, metadata in zip(entries, recordings, strict=True):
        code, replay, error = command(
            binary,
            ["easy", "replay", entry["cassette_path"], PROFILE, next(
                agent for agent in agents if agent == metadata.get("agent_id")
            ), "--local-mock", "--json"],
            root,
        )
        if code != 0 or not isinstance(replay, dict) or replay.get("network") != "denied":
            raise RuntimeError(f"strict offline replay failed: {error or replay}")
        replays.append({
            "agent_id": metadata["agent_id"],
            "workload_id": entry["workload_id"],
            "cassette_sha256": metadata["cassette_sha256"],
            "result_digest": replay.get("result_digest"),
            "network": replay["network"],
        })
    return {
        "campaign_id": output["campaign_id"],
        "tuple_count": output["tuple_count"],
        "recordings": [{
            "agent_id": item["agent_id"],
            "workload_id": entry["workload_id"],
            "cassette_sha256": item["cassette_sha256"],
        } for item, entry in zip(recordings, entries, strict=True)],
        "replays": replays,
        "comparison": {
            "comparable": all(item["network"] == "denied" and item["result_digest"] for item in replays),
            "tuple_count": len(replays),
            "matching_cassette_digests": len({item["cassette_sha256"] for item in replays}) == len(replays),
        },
    }


def negative_incomplete(root: Path, binary: Sequence[str], fixture: Path, agents: Sequence[str], workloads: Sequence[str], workspace: Path) -> dict[str, Any]:
    # Prove the gate rejects a prefix, without publishing any cassette.
    capture = workspace / "negative-capture.json"
    cassette = workspace / "negative-cassette.json"
    capture.write_bytes(canonical(fixture_capture(fixture, agents[0], PROFILE)))
    manifest = workspace / "negative-campaign.json"
    manifest.write_bytes(canonical({
        "schema_version": WORKFLOW_SCHEMA_VERSION,
        "provider_profile_sha256": PROFILE,
        "agent_ids": list(agents),
        "workload_ids": list(workloads),
        "entries": [{"workload_id": workloads[0], "capture_path": str(capture), "cassette_path": str(cassette)}],
    }))
    code, output, error = command(binary, ["record-campaign", str(manifest), "--local-mock", "--json"], root)
    rejected = code == 0 and isinstance(output, dict) and output.get("complete_coverage") is False and not cassette.exists()
    if not rejected:
        raise RuntimeError(f"incomplete-coverage negative case did not fail closed: {error or output}")
    return {"rejected": True, "reason": output.get("unavailable_reason", "recording-coverage-incomplete")}


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--asb", type=Path, required=True, help="exact local asb binary")
    parser.add_argument("--fixture", type=Path, default=None)
    parser.add_argument("--agent", action="append", dest="agents")
    parser.add_argument("--workload", action="append", dest="workloads")
    parser.add_argument("--all", action="store_true", help="run every repository fixture-only local-mock tuple")
    parser.add_argument("--receipt", type=Path, required=True)
    args = parser.parse_args(argv)
    root = args.root.resolve()
    fixture = (args.fixture or root / "crates/asb-replay/fixtures/v1/buffered.json").resolve()
    if not args.asb.is_absolute() or not args.asb.is_file():
        parser.error("--asb must identify an existing exact binary")
    if not fixture.is_file():
        parser.error("fixture does not exist")
    agents = select(args.agents, ALL_AGENTS, DEFAULT_AGENTS, args.all)
    workloads = select(args.workloads, ALL_WORKLOADS, DEFAULT_WORKLOADS, args.all)
    binary = [str(args.asb.resolve())]
    with tempfile.TemporaryDirectory(prefix="asb-content-addressed-") as directory:
        workspace = Path(directory)
        campaign = run_campaign(root, binary, fixture, agents, workloads, workspace)
        negative = negative_incomplete(root, binary, fixture, agents, workloads, workspace)
    receipt = {
        "schema_version": SCHEMA_VERSION,
        "status": "qualified",
        "network": "denied",
        "credentials": "none",
        "source_revision": git_revision(root),
        "runner_sha256": digest_bytes(Path(__file__).read_bytes()),
        "fixture_sha256": digest_bytes(fixture.read_bytes()),
        "provider_profile_sha256": PROFILE,
        "selection": {"agents": agents, "workloads": workloads},
        "campaign": campaign,
        "negative_incomplete_coverage": negative,
    }
    receipt["receipt_sha256"] = digest_json(receipt)
    args.receipt.parent.mkdir(parents=True, exist_ok=True)
    temporary = args.receipt.with_name(f".{args.receipt.name}.tmp")
    temporary.write_bytes(canonical(receipt))
    os.replace(temporary, args.receipt)
    print(json.dumps(receipt, sort_keys=True))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, ValueError) as error:
        print(f"qualification unavailable: {error}", file=sys.stderr)
        raise SystemExit(2)
