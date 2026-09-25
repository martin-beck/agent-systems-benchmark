#!/usr/bin/env python3
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Dependency-free hostile/positive checks for orchestration schema vectors."""

import json
from pathlib import Path

import jsonschema


ROOT = Path(__file__).parents[2]
SCHEMA = json.loads((ROOT / "docs/orchestration-schema-v1.json").read_text())
VALID = json.loads((ROOT / "docs/orchestration-schema-v1.valid.json").read_text())
UNKNOWN = json.loads((ROOT / "docs/orchestration-schema-v1.unknown-field.json").read_text())


def validate_request(value: dict) -> None:
    allowed = {
        "schema_version", "kind", "idempotency_key", "agent_id", "provider_id",
        "model_id", "workload_id", "mode", "catalog_digest", "workload_revision", "scorer_revision",
        "cassette_digest", "credential_ref_digest", "limits",
    }
    required = allowed - {"cassette_digest", "credential_ref_digest"}
    assert set(value) <= required | {"cassette_digest", "credential_ref_digest"}
    assert required <= set(value)
    assert value["schema_version"] == 1 and value["kind"] == "run_request"
    assert value["mode"] == "local-mock"
    assert "cassette_digest" not in value and "credential_ref_digest" not in value
    limits = value["limits"]
    assert 0 < limits["max_events"] <= 1024
    assert 0 < limits["max_artifacts"] <= 1024
    assert 0 < limits["max_artifact_bytes"] <= 67108864
    assert 0 < limits["max_artifact_total_bytes"] <= 268435456


validator = jsonschema.Draft202012Validator(SCHEMA)
validator.check_schema(SCHEMA)
validator.validate(VALID)
for operation in (
    {"schema_version": 1, "kind": "status_request", "run_handle": {"id": "run-1", "generation": 1, "fence": "a" * 64}},
    {"schema_version": 1, "kind": "cancel_request", "run_handle": {"id": "run-1", "generation": 1, "fence": "a" * 64}, "attempt_handle": {"id": "attempt-1", "generation": 1, "fence": "b" * 64}},
    {"schema_version": 1, "kind": "retry_request", "run_handle": {"id": "run-1", "generation": 1, "fence": "a" * 64}, "retry_token": "c" * 64},
    {"schema_version": 1, "kind": "result_page_request", "run_handle": {"id": "run-1", "generation": 1, "fence": "a" * 64}, "page_size": 16},
):
    validator.validate(operation)
live = dict(VALID)
live.update({"mode": "live", "credential_ref_digest": "d" * 64})
validator.validate(live)
replay = dict(VALID)
replay.update({"mode": "strict-replay", "cassette_digest": "e" * 64})
validator.validate(replay)
for invalid in (
    UNKNOWN,
    {**VALID, "mode": "live"},
    {**VALID, "mode": "strict-replay", "cassette_digest": "e" * 64, "credential_ref_digest": "d" * 64},
):
    try:
        validator.validate(invalid)
    except jsonschema.ValidationError:
        pass
    else:
        raise AssertionError("invalid orchestration instance was accepted")


assert SCHEMA["$defs"]["RunRequest"]["additionalProperties"] is False
assert SCHEMA["$defs"]["RunRequest"]["properties"]["model_id"]["$ref"] == "#/$defs/Id"
assert "scorer_revision" in SCHEMA["$defs"]["RunRequest"]["required"]
assert SCHEMA["$defs"]["Limits"]["properties"]["max_artifact_total_bytes"]["maximum"] == 268435456
for operation in ("StatusRequest", "CancelRequest", "RetryRequest", "ResultPageRequest"):
    assert SCHEMA["$defs"][operation]["additionalProperties"] is False
validate_request(VALID)
