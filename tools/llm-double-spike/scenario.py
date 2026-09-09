# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Fail-closed validator for generated synthetic LLM scenarios."""
from __future__ import annotations

import argparse
import hashlib
import json
import re
from pathlib import Path
from typing import Any

MAX_BYTES = 1_048_576
PRIVATE_KEYS = {"api_key", "apikey", "authorization", "credential", "password", "secret", "token"}
PRIVATE_MARKERS = ("/home/", "/private/", "BEGIN OPENSSH", "BEGIN RSA", "ghp_", "sk-")
SESSION = re.compile(r"^[a-z][a-z0-9._-]{0,31}$")
class ScenarioError(ValueError):
    """A scenario is malformed, private, unbounded, or evidence-incompatible."""
def _walk(value: Any, key: str = "") -> None:
    if isinstance(value, dict):
        if len(value) > 32: raise ScenarioError("object has too many fields")
        for name, child in value.items():
            if name.lower() in PRIVATE_KEYS or any(marker in str(child) for marker in PRIVATE_MARKERS): raise ScenarioError("private or credential-like field")
            _walk(child, name)
    elif isinstance(value, list):
        if len(value) > 64: raise ScenarioError("array is unbounded")
        for child in value: _walk(child, key)
    elif isinstance(value, str):
        if len(value.encode()) > 16_384: raise ScenarioError("string exceeds bounded field size")
        if ("http://" in value or "https://" in value) and "127.0.0.1" not in value and "localhost" not in value:
            raise ScenarioError("scenario URL is not loopback")
def validate(value: Any) -> dict[str, Any]:
    if not isinstance(value, dict) or value.get("schema_version") != 1: raise ScenarioError("unsupported scenario schema")
    if value.get("evidence_origin") != "synthetic": raise ScenarioError("evidence origin must remain synthetic")
    if not isinstance(value.get("scenario_id"), str) or not re.fullmatch(r"[a-z][a-z0-9._-]{0,63}", value["scenario_id"]): raise ScenarioError("invalid scenario id")
    if value.get("protocol") not in {"openai-chat", "openai-responses", "anthropic-messages"}: raise ScenarioError("unsupported protocol")
    events = value.get("events")
    if not isinstance(events, list) or not events or len(events) > 64: raise ScenarioError("event sequence is empty or unbounded")
    sequences, sessions = [], set()
    for event in events:
        if not isinstance(event, dict) or set(event) != {"sequence", "session", "kind", "payload"}: raise ScenarioError("event fields are not closed")
        sequence, session, kind, payload = event["sequence"], event["session"], event["kind"], event["payload"]
        if not isinstance(sequence, int) or sequence < 0 or sequence >= 64 or sequence in sequences: raise ScenarioError("event sequence is invalid or duplicated")
        if not isinstance(session, str) or not SESSION.fullmatch(session): raise ScenarioError("invalid session")
        if kind not in {"request", "buffered", "sse", "tool_call", "tool_result", "fault", "cancelled", "completed"}: raise ScenarioError("unsupported event kind")
        if not isinstance(payload, dict): raise ScenarioError("event payload must be an object")
        sequences.append(sequence); sessions.add(session)
    if len(sessions) > 16 or sequences != sorted(sequences): raise ScenarioError("sessions or ordering exceed bounds")
    _walk(value)
    canonical = json.dumps(value, sort_keys=True, separators=(",", ":")).encode()
    if len(canonical) > MAX_BYTES: raise ScenarioError("scenario exceeds 1 MiB")
    return value
def canonical_bytes(value: Any) -> bytes:
    validate(value); return json.dumps(value, sort_keys=True, separators=(",", ":")).encode() + b"\n"
def main() -> None:
    parser = argparse.ArgumentParser(); parser.add_argument("scenario", type=Path); args = parser.parse_args()
    payload = canonical_bytes(json.loads(args.scenario.read_text(encoding="utf-8")))
    print(json.dumps({"schema_version": 1, "evidence_origin": "synthetic", "sha256": hashlib.sha256(payload).hexdigest(), "bytes": len(payload)}, sort_keys=True))
if __name__ == "__main__": main()
