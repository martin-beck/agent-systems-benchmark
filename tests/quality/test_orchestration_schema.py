#!/usr/bin/env python3
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Dependency-free hostile/positive checks for orchestration schema vectors."""

import json
from pathlib import Path


ROOT = Path(__file__).parents[2]
VALID = json.loads((ROOT / "docs/orchestration-schema-v1.valid.json").read_text())
UNKNOWN = json.loads((ROOT / "docs/orchestration-schema-v1.unknown-field.json").read_text())


def validate_request(value: dict) -> None:
    allowed = {
        "schema_version", "kind", "idempotency_key", "agent_id", "provider_id",
        "model_id", "workload_id", "mode", "catalog_digest", "workload_revision",
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


validate_request(VALID)
try:
    validate_request(UNKNOWN)
except AssertionError:
    pass
else:
    raise AssertionError("unknown orchestration field was accepted")
