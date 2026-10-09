# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Bounded, secret-safe cli2key contract spike."""

from __future__ import annotations

import argparse
import contextlib
import http.client
import json
import os
import re
import socket
import threading
import time
from dataclasses import dataclass
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any, cast
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parents[2]
CONTRACT_PATH = ROOT / "config" / "cli2key-bridge-v1.json"
CLIENT_KEY_ENV = "ASB_CLI2KEY_CLIENT_KEY"
INPUT_ENV = "ASB_CLI2KEY_SPIKE_INPUT"
SYNTHETIC_AUTH_MARKER = "not-a-secret-cli2key-fixture-auth"
MAX_MODELS = 128
MAX_MODELS_BYTES = 256 * 1024
MAX_RESPONSE_BYTES = 1024 * 1024
TIMEOUT_SECONDS = 60
UPSTREAM_AUTHENTICATION = (
    "user-approved Codex OAuth remains in the user-selected CODEX_HOME and is read-only; "
    "only the separate local client key is stored under ~/.cb"
)
REPORT_FIELDS = {
    "schema_version",
    "evidence_class",
    "classification",
    "contract_id",
    "bridge_revision",
    "bridge_tree",
    "bridge_archive_sha256",
    "models_status",
    "models_count",
    "selected_model",
    "responses_status",
    "response_shape_valid",
    "warnings",
}
MODEL_ID = re.compile(r"[A-Za-z0-9][A-Za-z0-9._:-]{0,127}\Z")


class SpikeError(Exception):
    """A public typed failure with no provider or operating-system detail."""

    def __init__(self, code: str):
        super().__init__(code)
        self.code = code


@dataclass(frozen=True)
class Endpoint:
    """Validated exact loopback HTTP endpoint."""

    host: str
    port: int


def load_contract() -> dict[str, Any]:
    """Load and fail closed on the immutable bridge contract."""
    try:
        value = json.loads(CONTRACT_PATH.read_text(encoding="utf-8"))
        bridge = value["bridge"]
        protocol = value["protocol"]
        evidence = value["evidence"]
    except (OSError, KeyError, TypeError, json.JSONDecodeError) as error:
        raise SpikeError("contract_invalid") from error
    validate_contract(value, bridge, protocol, evidence)
    return cast(dict[str, Any], value)


def validate_contract(
    value: dict[str, Any],
    bridge: dict[str, Any],
    protocol: dict[str, Any],
    evidence: dict[str, Any],
) -> None:
    """Validate the immutable identities and public bounds used by the spike."""
    exact = {
        "schema_version": 1,
        "contract_id": "asb.cli2key.development.v1",
        "classification": "development-only-unofficial",
    }
    expected_bridge = {
        "version": "v0.1.10",
        "revision": "da5d271db08cca1f4666c1b035dbfb6c6f9b8f47",
        "tree": "682881f6d7fea1fa117bf0899ae0754291bba3c4",
        "archive_sha256": ("db256f8b8b9392835fb1277cadcff6ded3da99a6831f3ff902d5f7721cc7d5bb"),
    }
    expected_protocol = {
        "upstream_authentication": UPSTREAM_AUTHENTICATION,
        "maximum_models": MAX_MODELS,
        "maximum_models_bytes": MAX_MODELS_BYTES,
        "maximum_response_bytes": MAX_RESPONSE_BYTES,
        "request_timeout_seconds": TIMEOUT_SECONDS,
    }
    invalid = (
        any(value.get(name) != expected for name, expected in exact.items())
        or any(bridge.get(name) != expected for name, expected in expected_bridge.items())
        or any(protocol.get(name) != expected for name, expected in expected_protocol.items())
        or set(evidence.get("allowed_fields", [])) != REPORT_FIELDS
    )
    if invalid:
        raise SpikeError("contract_invalid")


def parse_endpoint(raw: str) -> Endpoint:
    """Accept only explicit numeric loopback HTTP authorities."""
    try:
        parsed = urlsplit(raw)
        port = parsed.port
    except ValueError as error:
        raise SpikeError("non_loopback_endpoint") from error
    if (
        parsed.scheme != "http"
        or parsed.hostname not in {"127.0.0.1", "::1"}
        or port is None
        or parsed.username is not None
        or parsed.password is not None
        or parsed.path not in {"", "/"}
        or parsed.query
        or parsed.fragment
    ):
        raise SpikeError("non_loopback_endpoint")
    return Endpoint(parsed.hostname, port)


def read_bounded(response: http.client.HTTPResponse, maximum: int, oversized: str) -> bytes:
    """Read at most one byte beyond a public protocol bound."""
    declared = response.getheader("content-length")
    if declared is not None:
        try:
            if int(declared) > maximum:
                raise SpikeError(oversized)
        except ValueError as error:
            raise SpikeError("response_malformed") from error
    body = response.read(maximum + 1)
    if len(body) > maximum:
        raise SpikeError(oversized)
    return body


def remaining_timeout(deadline: float) -> float:
    """Return the remaining wall-clock budget or fail at the absolute deadline."""
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        raise SpikeError("deadline_exceeded")
    return remaining


def set_socket_timeout(connection: http.client.HTTPConnection, deadline: float) -> None:
    """Apply only the remaining absolute budget to the connected socket."""
    remaining = remaining_timeout(deadline)
    connection.timeout = remaining
    if connection.sock is not None:
        connection.sock.settimeout(remaining)


def request(
    endpoint: Endpoint,
    method: str,
    path: str,
    client_key: str,
    body: dict[str, Any] | None,
    maximum: int,
    oversized: str,
) -> tuple[int, bytes]:
    """Perform one request under an absolute deadline without logging content."""
    deadline = time.monotonic() + TIMEOUT_SECONDS
    connection = http.client.HTTPConnection(
        endpoint.host,
        endpoint.port,
        timeout=remaining_timeout(deadline),
    )
    deadline_reached = threading.Event()

    def abort_at_deadline() -> None:
        deadline_reached.set()
        connected_socket = connection.sock
        if connected_socket is not None:
            with contextlib.suppress(OSError):
                connected_socket.shutdown(socket.SHUT_RDWR)
        connection.close()

    headers = {"authorization": f"Bearer {client_key}", "accept": "application/json"}
    encoded = None
    if body is not None:
        encoded = json.dumps(body, separators=(",", ":")).encode()
        headers["content-type"] = "application/json"
    deadline_timer = threading.Timer(remaining_timeout(deadline), abort_at_deadline)
    deadline_timer.daemon = True
    deadline_timer.start()
    try:
        set_socket_timeout(connection, deadline)
        connection.request(method, path, body=encoded, headers=headers)
        set_socket_timeout(connection, deadline)
        response = connection.getresponse()
        set_socket_timeout(connection, deadline)
        response_body = read_bounded(response, maximum, oversized)
        remaining_timeout(deadline)
        return response.status, response_body
    except TimeoutError as error:
        raise SpikeError("deadline_exceeded") from error
    except (OSError, ValueError, http.client.HTTPException) as error:
        if deadline_reached.is_set() or time.monotonic() >= deadline:
            raise SpikeError("deadline_exceeded") from error
        raise SpikeError("connection_failed") from error
    finally:
        deadline_timer.cancel()
        connection.close()
        deadline_timer.join()


def discover_models(endpoint: Endpoint, client_key: str) -> list[str]:
    """Discover a bounded list of unique model identities."""
    status, body = request(
        endpoint,
        "GET",
        "/v1/models",
        client_key,
        None,
        MAX_MODELS_BYTES,
        "models_oversized",
    )
    if status in {401, 403}:
        raise SpikeError("authentication_rejected")
    if status != 200:
        raise SpikeError("models_rejected")
    try:
        value = json.loads(body)
        data = value["data"]
        if not isinstance(data, list):
            raise TypeError
        if len(data) > MAX_MODELS:
            raise SpikeError("model_limit_exceeded")
        models = [item["id"] for item in data]
        if not models or any(
            not isinstance(item, str) or MODEL_ID.fullmatch(item) is None for item in models
        ):
            raise TypeError
        if len(set(models)) != len(models):
            raise TypeError
    except SpikeError:
        raise
    except (KeyError, TypeError, json.JSONDecodeError) as error:
        raise SpikeError("models_malformed") from error
    return cast(list[str], models)


def run_spike(
    endpoint_raw: str, client_key: str, model: str | None, input_text: str
) -> dict[str, Any]:
    """Run discovery and exactly one non-streaming Responses request."""
    contract = load_contract()
    endpoint = parse_endpoint(endpoint_raw)
    models = discover_models(endpoint, client_key)
    selected = model or models[0]
    if selected not in models:
        raise SpikeError("model_unavailable")
    status, body = request(
        endpoint,
        "POST",
        "/v1/responses",
        client_key,
        {"model": selected, "input": input_text, "stream": False},
        MAX_RESPONSE_BYTES,
        "response_oversized",
    )
    if status in {401, 403}:
        raise SpikeError("authentication_rejected")
    if status != 200:
        raise SpikeError("responses_rejected")
    try:
        response = json.loads(body)
        valid_shape = isinstance(response, dict) and response.get("object") == "response"
    except json.JSONDecodeError as error:
        raise SpikeError("response_malformed") from error
    if not valid_shape:
        raise SpikeError("response_malformed")
    bridge = contract["bridge"]
    report = {
        "schema_version": 1,
        "evidence_class": "development-live",
        "classification": contract["classification"],
        "contract_id": contract["contract_id"],
        "bridge_revision": bridge["revision"],
        "bridge_tree": bridge["tree"],
        "bridge_archive_sha256": bridge["archive_sha256"],
        "models_status": "passed",
        "models_count": len(models),
        "selected_model": selected,
        "responses_status": "passed",
        "response_shape_valid": True,
        "warnings": [
            "development-only and unofficial",
            "not an OpenAI Platform API key or production authorization",
            "provider bodies and prompts intentionally omitted",
        ],
    }
    if set(report) != REPORT_FIELDS:
        raise SpikeError("contract_invalid")
    serialized = json.dumps(report, sort_keys=True)
    if client_key in serialized:
        raise SpikeError("contract_invalid")
    return report


class FakeHandler(BaseHTTPRequestHandler):
    """Credential-free synthetic implementation of the two contract routes."""

    protocol_version = "HTTP/1.1"
    client_key = ""
    observed_requests = 0

    def log_message(self, _format: str, *_args: object) -> None:
        return

    def authorized(self) -> bool:
        return self.headers.get("authorization") == f"Bearer {self.client_key}"

    def do_GET(self) -> None:
        if not self.authorized():
            self.reply(401, {"error": {"type": "authentication_error"}})
        elif self.path == "/v1/models":
            self.reply(
                200,
                {
                    "object": "list",
                    "data": [{"id": "fixture-cli2key", "object": "model"}],
                },
            )
        else:
            self.reply(404, {"error": {"type": "not_found"}})

    def do_POST(self) -> None:
        if not self.authorized():
            self.reply(401, {"error": {"type": "authentication_error"}})
            return
        try:
            length = int(self.headers.get("content-length", "0"))
            value = json.loads(self.rfile.read(min(length, MAX_RESPONSE_BYTES + 1)))
        except (ValueError, json.JSONDecodeError):
            self.reply(400, {"error": {"type": "invalid_request"}})
            return
        if self.path != "/v1/responses" or value.get("model") != "fixture-cli2key":
            self.reply(404, {"error": {"type": "not_found"}})
            return
        type(self).observed_requests += 1
        self.reply(200, {"id": "fixture-response", "object": "response", "output": []})

    def reply(self, status: int, value: object) -> None:
        payload = json.dumps(value, separators=(",", ":")).encode()
        self.send_response(status)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(payload)))
        self.send_header("connection", "close")
        self.end_headers()
        self.wfile.write(payload)
        self.close_connection = True


def fake_spike(input_text: str = "synthetic-private-input") -> dict[str, Any]:
    """Run the contract using an explicit non-secret marker and loopback fake."""
    key = SYNTHETIC_AUTH_MARKER
    FakeHandler.client_key = key
    FakeHandler.observed_requests = 0
    server = ThreadingHTTPServer(("127.0.0.1", 0), FakeHandler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        host, port = cast(tuple[str, int], server.server_address)
        report = run_spike(
            endpoint_raw=f"http://{host}:{port}",
            client_key=key,
            model="fixture-cli2key",
            input_text=input_text,
        )
        report["evidence_class"] = "synthetic"
        if FakeHandler.observed_requests != 1:
            raise SpikeError("contract_invalid")
        serialized = json.dumps(report, sort_keys=True)
        if key in serialized:
            raise SpikeError("contract_invalid")
        return report
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


def main() -> int:
    """CLI entrypoint; live mode is explicit and secrets come only from the environment."""
    parser = argparse.ArgumentParser()
    parser.add_argument("--fake", action="store_true")
    parser.add_argument("--confirm-live", action="store_true")
    parser.add_argument("--endpoint")
    parser.add_argument("--model")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    try:
        if args.fake:
            if args.confirm_live or args.endpoint or args.model:
                raise SpikeError("contract_invalid")
            report = fake_spike()
        else:
            if not args.confirm_live or not args.endpoint:
                raise SpikeError("opt_in_required")
            key = os.environ.get(CLIENT_KEY_ENV)
            if not key:
                raise SpikeError("client_key_unavailable")
            input_text = os.environ.get(INPUT_ENV)
            if not input_text:
                raise SpikeError("input_unavailable")
            report = run_spike(args.endpoint, key, args.model, input_text)
        encoded = json.dumps(report, indent=2, sort_keys=True) + "\n"
        if args.output:
            args.output.write_text(encoded, encoding="utf-8")
        else:
            print(encoded, end="")
        return 0
    except (OSError, SpikeError) as error:
        code = error.code if isinstance(error, SpikeError) else "contract_invalid"
        print(f"cli2key spike failed: {code}", file=os.sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
