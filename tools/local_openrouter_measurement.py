#!/usr/bin/env python3
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Run a bounded, operator-only OpenRouter measurement.

This helper is deliberately not imported by ASB or invoked by CI. It resolves
one bounded prompt from a checked-in ASB workload fixture, sends it to the
catalog-selected free model, and emits digest-only typed evidence. Prompt and
response bytes are never written, logged, or included in the result.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import sys
import time
import urllib.error
import urllib.request

ENDPOINT = "https://openrouter.ai/api/v1/chat/completions"
MAX_PROMPT_BYTES = 16 * 1024
MAX_RESPONSE_BYTES = 256 * 1024
MAX_TRIALS = 3


def _sha256(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def _key() -> str:
    value = os.environ.get("OPENROUTER_API_KEY")
    if value is None:
        path = pathlib.Path.home() / ".api_key_openrouter"
        try:
            mode = path.stat().st_mode & 0o777
        except OSError as exc:
            raise SystemExit("OpenRouter key is unavailable; set OPENROUTER_API_KEY or create ~/.api_key_openrouter") from exc
        if mode != 0o600:
            raise SystemExit("OpenRouter key file must have mode 0600")
        value = path.read_text(encoding="utf-8").strip()
    if not value or len(value) > 4096 or any(char.isspace() for char in value):
        raise SystemExit("OpenRouter key is empty or malformed")
    return value


def _request(key: str, model: str, prompt: bytes) -> tuple[int, int, int | None, int | None]:
    body = json.dumps(
        {
            "model": model,
            "messages": [{"role": "user", "content": prompt.decode("utf-8")}],
            "stream": False,
            "max_tokens": 128,
            "temperature": 0,
        },
        separators=(",", ":"),
    ).encode("utf-8")
    request = urllib.request.Request(
        ENDPOINT,
        data=body,
        headers={"Authorization": f"Bearer {key}", "Content-Type": "application/json"},
        method="POST",
    )
    started = time.monotonic_ns()
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            status = response.status
            payload = response.read(MAX_RESPONSE_BYTES + 1)
    except urllib.error.HTTPError as error:
        status = error.code
        payload = error.read(MAX_RESPONSE_BYTES + 1)
    elapsed_ms = (time.monotonic_ns() - started) // 1_000_000
    if len(payload) > MAX_RESPONSE_BYTES:
        return status, elapsed_ms, None, None
    try:
        decoded = json.loads(payload)
        usage = decoded.get("usage", {}) if isinstance(decoded, dict) else {}
        prompt_tokens = usage.get("prompt_tokens")
        completion_tokens = usage.get("completion_tokens")
        if not isinstance(prompt_tokens, int) or prompt_tokens < 0:
            prompt_tokens = None
        if not isinstance(completion_tokens, int) or completion_tokens < 0:
            completion_tokens = None
    except (UnicodeDecodeError, json.JSONDecodeError):
        prompt_tokens = completion_tokens = None
    return status, elapsed_ms, prompt_tokens, completion_tokens


def _selection(
    catalog_path: pathlib.Path, provider: str, model: str | None, agents: list[str]
) -> tuple[str, list[str], str, str, str]:
    try:
        catalog = json.loads(catalog_path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise SystemExit("provider catalog is unreadable or malformed") from exc
    if not isinstance(catalog, dict) or catalog.get("ok") is not True:
        raise SystemExit("provider catalog is not an authoritative successful catalog")
    catalog_sha256 = catalog.get("catalog_sha256")
    if not isinstance(catalog_sha256, str) or len(catalog_sha256) != 64:
        raise SystemExit("provider catalog has no valid digest")
    advertised_agents = catalog.get("agents")
    profiles = catalog.get("profiles")
    if not isinstance(advertised_agents, list) or not all(isinstance(v, str) for v in advertised_agents):
        raise SystemExit("provider catalog has no valid agent inventory")
    if not agents:
        agents = ["codex"]
    if len(set(agents)) != len(agents) or any(agent not in advertised_agents for agent in agents):
        raise SystemExit("selected agent set is stale, duplicated, or not advertised by the catalog")
    if not isinstance(profiles, list):
        raise SystemExit("provider catalog has no valid profile inventory")
    profile = next((p for p in profiles if isinstance(p, dict) and p.get("id") == provider), None)
    if not isinstance(profile, dict) or profile.get("selectable") is not True:
        raise SystemExit("selected provider profile is unavailable or not selectable in the catalog")
    catalog_model = profile.get("model")
    if not isinstance(catalog_model, str) or not catalog_model:
        raise SystemExit("selected provider profile has no model identity")
    if model is not None and model != catalog_model:
        raise SystemExit("selected model is stale or does not match the catalog; refresh the catalog")
    if provider != "openrouter":
        raise SystemExit("this operator measurement supports only the OpenRouter endpoint")
    snapshot = f"{catalog_model}@{time.strftime('%Y-%m-%d', time.gmtime())}"
    selection = json.dumps(
        {"catalog_sha256": catalog_sha256, "provider": provider, "model": catalog_model, "agents": agents},
        separators=(",", ":"),
        sort_keys=True,
    ).encode("utf-8")
    return catalog_model, agents, snapshot, catalog_sha256, _sha256(selection)


def _workload_prompt(root: pathlib.Path, workload_id: str) -> bytes:
    manifests = list(root.glob("*/manifest.json"))
    for manifest_path in manifests:
        try:
            manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        except (OSError, UnicodeDecodeError, json.JSONDecodeError):
            continue
        if isinstance(manifest, dict) and manifest.get("workload_id") == workload_id:
            prompt_path = manifest_path.parent / "prompt.md"
            try:
                prompt = prompt_path.read_bytes()
            except OSError as exc:
                raise SystemExit("selected workload prompt is unavailable") from exc
            if len(prompt) > MAX_PROMPT_BYTES:
                raise SystemExit("selected workload prompt exceeds the 16 KiB bound")
            return prompt
    raise SystemExit("selected workload is not present in the ASB fixture catalog")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--catalog", type=pathlib.Path, required=True, help="fresh `asb provider-catalog` JSON")
    parser.add_argument("--workload-root", type=pathlib.Path, default=pathlib.Path("crates/asb-workloads/fixtures/v1"))
    parser.add_argument("--workload-id", required=True, help="public ASB workload identity")
    parser.add_argument("--provider", default="openrouter")
    parser.add_argument("--model", help="must match the selected profile in the fresh catalog")
    parser.add_argument("--agent", action="append", default=[], help="catalog-advertised agent; repeat to select a set")
    parser.add_argument("--trials", type=int, default=1)
    args = parser.parse_args()
    if not args.workload_id or len(args.workload_id) > 128 or any(c.isspace() for c in args.workload_id):
        parser.error("--workload-id must be a bounded non-empty identity")
    if not 1 <= args.trials <= MAX_TRIALS:
        parser.error(f"--trials must be between 1 and {MAX_TRIALS}")
    model, agents, snapshot, catalog_sha256, selection_sha256 = _selection(
        args.catalog, args.provider, args.model, args.agent
    )
    prompt = _workload_prompt(args.workload_root, args.workload_id)
    key = _key()
    trials = []
    for agent in agents:
        for _ in range(args.trials):
            status, elapsed_ms, prompt_tokens, completion_tokens = _request(key, model, prompt)
            trials.append(
                {
                    "agent": agent,
                    "status": "success" if 200 <= status < 300 else "provider_error",
                    "http_status": status,
                    "elapsed_ms": elapsed_ms,
                    "prompt_tokens": prompt_tokens,
                    "completion_tokens": completion_tokens,
                }
            )
    print(
        json.dumps(
            {
                "evidence_class": "remote_live",
                "provider": args.provider,
                "model": model,
                "model_snapshot": snapshot,
                "catalog_sha256": catalog_sha256,
                "selection_sha256": selection_sha256,
                "selected_agents": agents,
                "selection_class": "catalog_bound_transport_measurement",
                "workload_id": args.workload_id,
                "prompt_sha256": _sha256(prompt),
                "trials": trials,
            },
            sort_keys=True,
        )
    )
    return 0 if all(trial["status"] == "success" for trial in trials) else 2


if __name__ == "__main__":
    raise SystemExit(main())
