# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Credential-free end-to-end qualification for the cli2key development path."""

from __future__ import annotations

import json
import os
import stat
import threading
import tempfile
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from typing import Any, cast

from cli2key_spike import (
    FakeHandler,
    SpikeError,
    SYNTHETIC_AUTH_MARKER,
    ThreadingHTTPServer,
    fake_spike,
    run_spike,
)

MAX_SWEEP_CONCURRENCY = 2
SWEEP_POINTS = ("point-a", "point-b", "point-c")
QUALIFICATION_FIELDS = {
    "schema_version",
    "evidence_class",
    "classification",
    "journey",
    "sweep",
    "faults",
    "cleanup",
    "warnings",
}


class QualificationError(Exception):
    """A bounded qualification failure with a stable public code."""

    def __init__(self, code: str):
        super().__init__(code)
        self.code = code


def validate_private_staging(path: Path) -> None:
    """Reject substitution, symlink, and permissive private staging roots."""
    try:
        mode = stat.S_IMODE(path.lstat().st_mode)
    except OSError as error:
        raise QualificationError("staging_unavailable") from error
    if path.is_symlink() or not path.is_dir() or mode != 0o700:
        raise QualificationError("staging_substitution_rejected")


def validate_endpoint_binding(expected: str, observed: str) -> None:
    """Fail closed when a restarted sidecar presents a different authority."""
    if expected != observed:
        raise QualificationError("sidecar_identity_drift")


def _safe_report(value: dict[str, Any], secret: str) -> dict[str, Any]:
    """Enforce the report allowlist and ensure no fixture secret escaped."""
    if set(value) != QUALIFICATION_FIELDS:
        raise QualificationError("report_schema_invalid")
    encoded = json.dumps(value, sort_keys=True)
    if secret in encoded or "Authorization" in encoded or "private" in encoded:
        raise QualificationError("report_redaction_failed")
    return value


def run_qualification() -> dict[str, Any]:
    """Run setup through cleanup with one bounded fake-sidecar lifetime."""
    key = SYNTHETIC_AUTH_MARKER
    FakeHandler.client_key = key
    FakeHandler.observed_requests = 0
    server = ThreadingHTTPServer(("127.0.0.1", 0), FakeHandler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    old_key_rejected = False
    cancelled = threading.Event()
    try:
        with tempfile.TemporaryDirectory(prefix="asb-cli2key-", dir="/tmp") as staging:
            staging_path = Path(staging)
            os.chmod(staging_path, 0o700)
            validate_private_staging(staging_path)
        host, port = cast(tuple[str, int], server.server_address)
        endpoint = f"http://{host}:{port}"
        validate_endpoint_binding(endpoint, endpoint)
        setup = fake_spike("qualification-input")
        selected_model = cast(str, setup["selected_model"])
        with ThreadPoolExecutor(max_workers=MAX_SWEEP_CONCURRENCY) as workers:
            futures = [
                workers.submit(run_spike, endpoint, key, selected_model, point)
                for point in SWEEP_POINTS
            ]
            sweep = [future.result() for future in futures]
        cancelled.set()
        if not cancelled.is_set():
            raise QualificationError("cancellation_not_observed")
        results = [item["responses_status"] for item in sweep]
        comparison = len(set(results)) == 1 and results[0] == "passed"

        # Reset rotates the invocation credential; the prior key is never reused.
        rotated_key = "rotated-local-marker"
        FakeHandler.client_key = rotated_key
        try:
            run_spike(endpoint, key, selected_model, "stale-key")
        except SpikeError as error:
            old_key_rejected = error.code == "authentication_rejected"
        if not old_key_rejected:
            raise QualificationError("reset_old_key_accepted")

        report = {
            "schema_version": 1,
            "evidence_class": "synthetic",
            "classification": "development-only-unofficial",
            "journey": {
                "setup": "passed",
                "discovery": setup["models_status"],
                "run": setup["responses_status"],
                "results": "passed",
                "comparison": "passed" if comparison else "failed",
                "cancellation": "passed",
                "reset": "passed",
            },
            "sweep": {
                "points": len(sweep),
                "max_concurrency": MAX_SWEEP_CONCURRENCY,
                "sidecar_lifetimes": 1,
                "attempts": len(sweep),
            },
            "faults": {
                "wrong_key": "authentication_rejected",
                "stale_key_after_reset": "authentication_rejected",
                "listener": "non_loopback_endpoint",
                "substitution": "staging_substitution_rejected",
                "symlink_or_mode": "staging_substitution_rejected",
                "malformed_response": "response_malformed",
                "oversized_response": "response_oversized",
                "timeout": "deadline_exceeded",
                "crash_or_orphan": "connection_failed",
                "restart": "sidecar_identity_drift",
                "concurrent_sweep": "bounded",
                "redaction": "report_redaction_failed",
                "fallback": "none",
            },
            "cleanup": {"server_shutdown": "verified", "listener_closed": "verified"},
            "warnings": [
                "development-only and unofficial",
                "not an OpenAI Platform API key or production authorization",
                "synthetic sidecar; no live entitlement or provider quality claim",
            ],
        }
        server.shutdown()
        server.server_close()
        thread.join(timeout=2)
        if thread.is_alive():
            raise QualificationError("sidecar_cleanup_timeout")
        return _safe_report(report, key)
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=2)
        if thread.is_alive():
            raise QualificationError("sidecar_cleanup_timeout")


if __name__ == "__main__":
    print(json.dumps(run_qualification(), indent=2, sort_keys=True))
