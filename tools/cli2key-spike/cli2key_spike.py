# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Bounded, secret-safe cli2key contract spike."""

from __future__ import annotations

import argparse
import http.client
import json
import os
import re
import secrets
import threading
from dataclasses import dataclass
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any, cast
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parents[2]
CONTRACT_PATH = ROOT / "config" / "cli2key-bridge-v1.json"
CLIENT_KEY_ENV = "ASB_CLI2KEY_CLIENT_KEY"
INPUT_ENV = "ASB_CLI2KEY_SPIKE_INPUT"
MAX_MODELS = 128
MAX_MODELS_BYTES = 256 * 1024
MAX_RESPONSE_BYTES = 1024 * 1024
TIMEOUT_SECONDS = 60
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


class SpikeFailure(Exception):
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
        raise SpikeFailure("contract_invalid") from error
    exact = {
        "schema_version": 1,
        "contract_id": "asb.cli2key.development.v1",
        "classification": "development-only-unofficial",
    }
    if any(value.get(name) != expected for name, expected in exact.items()):
        raise SpikeFailure("contract_invalid")
    if bridge.get("version") != "v0.1.10":
        raise SpikeFailure("contract_invalid")
    if bridge.get("revision") != "da5d271db08cca1f4666c1b035dbfb6c6f9b8f47":
        raise SpikeFailure("contract_invalid")
    if bridge.get("tree") != "682881f6d7fea1fa117bf0899ae0754291bba3c4":
        raise SpikeFailure("contract_invalid")
    if (
        bridge.get("archive_sha256")
        != "db256f8b8b9392835fb1277cadcff6ded3da99a6831f3ff902d5f7721cc7d5bb"
    ):
        raise SpikeFailure("contract_invalid")
    if protocol.get("maximum_models") != MAX_MODELS:
        raise SpikeFailure("contract_invalid")
    if protocol.get("maximum_models_bytes") != MAX_MODELS_BYTES:
        raise SpikeFailure("contract_invalid")
    if protocol.get("maximum_response_bytes") != MAX_RESPONSE_BYTES:
        raise SpikeFailure("contract_invalid")
    if protocol.get("request_timeout_seconds") != TIMEOUT_SECONDS:
        raise SpikeFailure("contract_invalid")
    if set(evidence.get("allowed_fields", [])) != REPORT_FIELDS:
        raise SpikeFailure("contract_invalid")
    return cast(dict[str, Any], value)


def parse_endpoint(raw: str) -> Endpoint:
    """Accept only explicit numeric loopback HTTP authorities."""
    try:
        parsed = urlsplit(raw)
        port = parsed.port
    except ValueError as error:
        raise SpikeFailure("non_loopback_endpoint") from error
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
        raise SpikeFailure("non_loopback_endpoint")
    return Endpoint(parsed.hostname, port)


def read_bounded(
    response: http.client.HTTPResponse, maximum: int, oversized: str
) -> bytes:
    """Read at most one byte beyond a public protocol bound."""
    declared = response.getheader("content-length")
    if declared is not None:
        try:
            if int(declared) > maximum:
                raise SpikeFailure(oversized)
        except ValueError as error:
            raise SpikeFailure("response_malformed") from error
    body = response.read(maximum + 1)
    if len(body) > maximum:
        raise SpikeFailure(oversized)
    return body


def request(
    endpoint: Endpoint,
    method: str,
    path: str,
    client_key: str,
    body: dict[str, Any] | None,
    maximum: int,
    oversized: str,
) -> tuple[int, bytes]:
    """Perform one bounded request without logging request or response content."""
    connection = http.client.HTTPConnection(
        endpoint.host, endpoint.port, timeout=TIMEOUT_SECONDS
    )
    headers = {"authorization": f"Bearer {client_key}", "accept": "application/json"}
    encoded = None
    if body is not None:
        encoded = json.dumps(body, separators=(",", ":")).encode()
        headers["content-type"] = "application/json"
    try:
        connection.request(method, path, body=encoded, headers=headers)
        response = connection.getresponse()
        return response.status, read_bounded(response, maximum, oversized)
    except TimeoutError as error:
        raise SpikeFailure("deadline_exceeded") from error
    except (OSError, http.client.HTTPException) as error:
        raise SpikeFailure("connection_failed") from error
    finally:
        connection.close()


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
        raise SpikeFailure("authentication_rejected")
    if status != 200:
        raise SpikeFailure("models_rejected")
    try:
        value = json.loads(body)
        data = value["data"]
        if not isinstance(data, list):
            raise TypeError
        if len(data) > MAX_MODELS:
            raise SpikeFailure("model_limit_exceeded")
        models = [item["id"] for item in data]
        if not models or any(
            not isinstance(item, str) or MODEL_ID.fullmatch(item) is None
            for item in models
        ):
            raise TypeError
        if len(set(models)) != len(models):
            raise TypeError
    except SpikeFailure:
        raise
    except (KeyError, TypeError, json.JSONDecodeError) as error:
        raise SpikeFailure("models_malformed") from error
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
        raise SpikeFailure("model_unavailable")
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
        raise SpikeFailure("authentication_rejected")
    if status != 200:
        raise SpikeFailure("responses_rejected")
    try:
        response = json.loads(body)
        valid_shape = (
            isinstance(response, dict) and response.get("object") == "response"
        )
    except json.JSONDecodeError as error:
        raise SpikeFailure("response_malformed") from error
    if not valid_shape:
        raise SpikeFailure("response_malformed")
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
        raise SpikeFailure("contract_invalid")
    serialized = json.dumps(report, sort_keys=True)
    if client_key in serialized:
        raise SpikeFailure("contract_invalid")
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
    """Run the complete contract against a random-key loopback fake."""
    key = secrets.token_urlsafe(32)
    FakeHandler.client_key = key
    FakeHandler.observed_requests = 0
    server = ThreadingHTTPServer(("127.0.0.1", 0), FakeHandler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        host, port = cast(tuple[str, int], server.server_address)
        report = run_spike(f"http://{host}:{port}", key, "fixture-cli2key", input_text)
        report["evidence_class"] = "synthetic"
        if FakeHandler.observed_requests != 1:
            raise SpikeFailure("contract_invalid")
        serialized = json.dumps(report, sort_keys=True)
        if key in serialized:
            raise SpikeFailure("contract_invalid")
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
                raise SpikeFailure("contract_invalid")
            report = fake_spike()
        else:
            if not args.confirm_live or not args.endpoint:
                raise SpikeFailure("opt_in_required")
            key = os.environ.get(CLIENT_KEY_ENV)
            if not key:
                raise SpikeFailure("client_key_unavailable")
            input_text = os.environ.get(INPUT_ENV)
            if not input_text:
                raise SpikeFailure("input_unavailable")
            report = run_spike(args.endpoint, key, args.model, input_text)
        encoded = json.dumps(report, indent=2, sort_keys=True) + "\n"
        if args.output:
            args.output.write_text(encoded, encoding="utf-8")
        else:
            print(encoded, end="")
        return 0
    except (OSError, SpikeFailure) as error:
        code = error.code if isinstance(error, SpikeFailure) else "contract_invalid"
        print(f"cli2key spike failed: {code}", file=os.sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
