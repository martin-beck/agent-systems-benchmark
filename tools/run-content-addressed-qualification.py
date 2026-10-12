#!/usr/bin/env python3
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
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


def command(
    binary: Sequence[str], args: Sequence[str], root: Path,
    extra_environment: dict[str, str] | None = None,
) -> tuple[int, Any, str]:
    environment = sanitized_environment()
    if extra_environment:
        environment.update(extra_environment)
    result = subprocess.run(
        [*binary, *args], cwd=root, check=False, capture_output=True, text=True,
        env=environment,
    )
    try:
        output = json.loads(result.stdout)
    except json.JSONDecodeError:
        output = None
    return result.returncode, output, result.stderr.strip()


def plan_and_run_journey(
    root: Path, binary: Sequence[str], workspace: Path,
) -> dict[str, Any]:
    """Create, validate, execute, report, and compare two fresh plans."""
    user_root = workspace / "fresh-user"
    user_root.mkdir(mode=0o700)
    environment = {
        "HOME": str(user_root),
        "XDG_CONFIG_HOME": str(user_root / "config"),
        "XDG_STATE_HOME": str(user_root / "state"),
        "XDG_DATA_HOME": str(user_root / "data"),
    }
    setup_code, setup, error = command(
        binary,
        ["setup", "--agent", "codex", "--provider-profile", "openrouter",
         "--model", "cohere/north-mini-code:free", "--persist", "--json"],
        root,
        environment,
    )
    if setup_code != 0 or not isinstance(setup, dict):
        raise RuntimeError(f"fresh-user setup failed: {error or setup}")
    agent = workspace / "offline-agent"
    agent.write_text(
        "#!/bin/sh\nprintf '%s\\n' 'def parse_line(line):' '    return line' > parser.py\n",
        encoding="utf-8",
    )
    agent.chmod(0o700)
    plans: list[dict[str, Any]] = []
    reports: list[dict[str, Any]] = []
    run_dirs: list[Path] = []
    for index in (1, 2):
        plan_root = workspace / f"plan-{index}"
        plan_root.mkdir(mode=0o700)
        result_root = plan_root / "results"
        plan_path = plan_root / "experiment.toml"
        create_code, created, error = command(
            binary,
            ["plan", "create", "--workload", "original.bug-fix", "--agent", "codex",
             "--agent-executable", str(agent), "--run-id", "qualification",
             "--result-root", str(result_root), "--work-root", str(plan_root / "work"),
            "--output", str(plan_path), "--use-config", "--json"],
            root,
            environment,
        )
        if create_code != 0 or not isinstance(created, dict):
            raise RuntimeError(f"plan create failed: {error or created}")
        plan_code, planned, error = command(
            binary, ["plan", str(plan_path), "--use-config", "--json"], root, environment
        )
        if plan_code != 0 or not isinstance(planned, dict):
            raise RuntimeError(f"plan validation failed: {error or planned}")
        run_code, run_output, error = command(
            binary, ["run", str(plan_path), "--use-config", "--local-mock", "--json"], root, environment
        )
        if run_code != 0 or not isinstance(run_output, dict):
            raise RuntimeError(f"local-mock run failed: {error or run_output}")
        run_dir = result_root / "runs" / "qualification"
        report_code, report, error = command(
            binary, ["report", str(run_dir), "--json"], root, environment
        )
        if report_code != 0 or not isinstance(report, dict):
            raise RuntimeError(f"run report failed: {error or report}")
        plans.append({
            "experiment_sha256": created.get("experiment_sha256"),
            "validated": planned.get("ok") is True,
        })
        reports.append({
            "run_id": "qualification",
            "terminal_state": report.get("runs", [{}])[0].get("terminal_state"),
        })
        run_dirs.append(run_dir)
    compare_code, comparison, error = command(
        binary,
        ["compare", *(str(path) for path in run_dirs), "--json"],
        root,
        environment,
    )
    if compare_code != 0 or not isinstance(comparison, dict) or comparison.get("comparable") is not True:
        raise RuntimeError(f"local-mock comparison failed: {error or comparison}")
    return {"setup": "development_only", "plans": plans, "runs": reports,
            "comparison": {"comparable": True, "differences": comparison.get("differences", [])}}


def fixture_capture(fixture: Path, agent: str, profile: str) -> dict[str, Any]:
    cassette = json.loads(fixture.read_text(encoding="utf-8"))
    contents = cassette["contents"]
    # The checked-in Gemini cassette is already redacted for documentation,
    # while the recording API expects an unmarked capture and applies its own
    # redaction.  Omit the credential-bearing header in this credential-free
    # development fixture; the strict dialect contract does not require it.
    for interaction in contents.get("interactions", []):
        request = interaction.get("request", {})
        request["headers"] = [
            header for header in request.get("headers", [])
            if header.get("name", "").lower() not in {
                "authorization", "cookie", "proxy-authorization", "set-cookie",
                "x-api-key", "x-goog-api-key",
            }
        ]
    # The fixture is public synthetic material.  Keep its response and request
    # body intact, but bind the envelope to the selected local identity.
    return {
        "schema_version": WORKFLOW_SCHEMA_VERSION,
        "provider_profile_sha256": profile,
        "agent_id": agent,
        "network": "loopback_only",
        "estimated_cost_minor": 0,
        "confirmation": {"record": True, "network": True, "cost": False},
        "contents": contents,
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
            ["replay-offline", entry["cassette_path"], PROFILE, next(
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
            "result_digest": replay.get("result_digest") or replay.get("response_sha256"),
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
    negative_agents = list(agents)
    negative_workloads = list(workloads)
    for candidate in ALL_AGENTS:
        if candidate not in negative_agents:
            negative_agents.append(candidate)
            break
    if len(negative_agents) == len(agents):
        for candidate in ALL_WORKLOADS:
            if candidate not in negative_workloads:
                negative_workloads.append(candidate)
                break
    capture = workspace / "negative-capture.json"
    cassette = workspace / "negative-cassette.json"
    capture.write_bytes(canonical(fixture_capture(fixture, agents[0], PROFILE)))
    manifest = workspace / "negative-campaign.json"
    manifest.write_bytes(canonical({
        "schema_version": WORKFLOW_SCHEMA_VERSION,
        "provider_profile_sha256": PROFILE,
        "agent_ids": negative_agents,
        "workload_ids": negative_workloads,
        "entries": [{"workload_id": negative_workloads[0], "capture_path": str(capture), "cassette_path": str(cassette)}],
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
    parser.add_argument("--tui-head", required=True, help="exact paired asb-tui main revision used for this qualification")
    parser.add_argument("--receipt", type=Path, required=True)
    args = parser.parse_args(argv)
    root = args.root.resolve()
    # The buffered fixture intentionally uses the ``synthetic`` dialect and is
    # rejected by StrictReplayService.  Qualification must use a provider
    # dialect fixture while remaining credential-free and offline.
    fixture = (args.fixture or root / "crates/asb-replay/fixtures/v1/gemini-generate-content.json").resolve()
    if not args.asb.is_absolute() or not args.asb.is_file():
        parser.error("--asb must identify an existing exact binary")
    if not fixture.is_file():
        parser.error("fixture does not exist")
    agents = select(args.agents, ALL_AGENTS, DEFAULT_AGENTS, args.all)
    workloads = select(args.workloads, ALL_WORKLOADS, DEFAULT_WORKLOADS, args.all)
    binary = [str(args.asb.resolve())]
    with tempfile.TemporaryDirectory(prefix="asb-content-addressed-") as directory:
        workspace = Path(directory)
        journey = plan_and_run_journey(root, binary, workspace)
        campaign = run_campaign(root, binary, fixture, agents, workloads, workspace)
        negative = negative_incomplete(root, binary, fixture, agents, workloads, workspace)
    receipt = {
        "schema_version": SCHEMA_VERSION,
        "status": "qualified",
        "network": "denied",
        "credentials": "none",
        "source_revision": git_revision(root),
        "paired_heads": {"asb": git_revision(root), "asb_tui": args.tui_head},
        "runner_sha256": digest_bytes(Path(__file__).read_bytes()),
        "fixture_sha256": digest_bytes(fixture.read_bytes()),
        "provider_profile_sha256": PROFILE,
        "selection": {"agents": agents, "workloads": workloads},
        "plan_run_journey": journey,
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
