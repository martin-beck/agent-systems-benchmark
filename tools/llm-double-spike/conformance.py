# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Credential-free OpenAI/Anthropic deterministic-double conformance spike."""

from __future__ import annotations

import argparse
import hashlib
import http.client
import json
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any, cast
from urllib.parse import urlsplit

ROOT = Path(__file__).parent
PRIVATE = "asb-private-spike-sentinel"


def _json(value: object) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()


def _sse(event: str, value: object) -> bytes:
    return f"event: {event}\ndata: {json.dumps(value, sort_keys=True)}\n\n".encode()


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, _format: str, *_args: object) -> None:
        return

    def do_POST(self) -> None:
        try:
            body = json.loads(self.rfile.read(int(self.headers.get("content-length", "0"))))
        except (ValueError, json.JSONDecodeError):
            self._reply(400, {"error": {"type": "invalid_json"}})
            return
        path = urlsplit(self.path).path
        if path == "/v1/chat/completions":
            self._reply(200, {"id": "chat-fixture", "choices": [{"finish_reason": "stop", "message": {"role": "assistant", "content": "fixture"}}], "usage": {"prompt_tokens": 3, "completion_tokens": 2}})
        elif path == "/v1/responses":
            self._stream([{"type": "response.created", "id": "response-fixture"}, {"type": "response.output_text.delta", "delta": "fixture"}, {"type": "response.completed", "usage": {"input_tokens": 3, "output_tokens": 2}}])
        elif path == "/v1/messages":
            if body.get("stream"):
                self._stream([{"type": "message_start", "message": {"id": "msg-fixture"}}, {"type": "content_block_delta", "delta": {"type": "text_delta", "text": "fixture"}}, {"type": "message_stop"}])
            else:
                self._reply(200, {"id": "msg-fixture", "type": "message", "stop_reason": "end_turn", "content": [{"type": "text", "text": "fixture"}], "usage": {"input_tokens": 3, "output_tokens": 2}})
        elif path == "/v1/fault/rate-limit":
            self._reply(429, {"error": {"type": "rate_limit", "retryable": True}})
        elif path == "/v1/fault/truncated":
            self._stream([{"type": "partial"}], complete=False)
        else:
            self._reply(404, {"error": {"type": "unmatched_fixture_route"}})

    def _reply(self, status: int, value: object) -> None:
        payload = _json(value)
        self.send_response(status)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(payload)))
        self.send_header("connection", "close")
        self.end_headers()
        self.wfile.write(payload)
        self.close_connection = True

    def _stream(self, events: list[dict[str, Any]], complete: bool = True) -> None:
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.send_header("connection", "close")
        self.end_headers()
        for event in events:
            self.wfile.write(_sse(str(event["type"]), event))
            self.wfile.flush()
            time.sleep(0.005)
        if complete:
            self.close_connection = True


def post(address: tuple[str, int], path: str, body: object) -> tuple[int, bytes, str]:
    connection = http.client.HTTPConnection(*address, timeout=3)
    connection.request("POST", path, _json(body), {"content-type": "application/json", "x-asb-session": "fixture"})
    response = connection.getresponse()
    value = response.read()
    kind = response.getheader("content-type", "")
    connection.close()
    return response.status, value, kind


def suite() -> dict[str, Any]:
    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        address = cast(tuple[str, int], server.server_address)
        cases: dict[str, str] = {}
        for name, path, body in (
            ("openai_chat_buffered", "/v1/chat/completions", {"model": "fixture", "messages": []}),
            ("openai_responses_sse", "/v1/responses", {"model": "fixture", "stream": True, "tools": []}),
            ("anthropic_buffered", "/v1/messages", {"model": "fixture", "messages": []}),
            ("anthropic_sse", "/v1/messages", {"model": "fixture", "messages": [], "stream": True}),
        ):
            status, payload, kind = post(address, path, body)
            if status != 200 or "application/json" not in kind and "text/event-stream" not in kind:
                raise AssertionError(f"{name} failed")
            if PRIVATE.encode() in payload:
                raise AssertionError(f"{name} leaked private sentinel")
            repeated = post(address, path, body)[1]
            if payload != repeated:
                raise AssertionError(f"{name} is not byte deterministic")
            cases[name] = hashlib.sha256(payload).hexdigest()
        status, _, _ = post(address, "/v1/fault/rate-limit", {})
        cases["rate_limit"] = "pass" if status == 429 else "fail"
        status, payload, _ = post(address, "/v1/fault/truncated", {})
        cases["truncated_stream"] = "pass" if status == 200 and b"response.completed" not in payload else "fail"
        cases["unmatched_route"] = "pass" if post(address, "/v1/not-recorded", {})[0] == 404 else "fail"
    finally:
        server.shutdown()
        server.server_close()
        thread.join()
    manifest = json.loads((ROOT / "manifest.json").read_text(encoding="utf-8"))
    return {"schema_version": 1, "evidence_class": "synthetic", "network": "loopback-only", "credentials": "none", "cases": cases, "candidates": [{**candidate, "status": "untested", "reason": "no pinned executable artifact is supplied by ASB"} for candidate in manifest["candidates"]]}


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    report = json.dumps(suite(), indent=2, sort_keys=True) + "\n"
    if args.output:
        args.output.write_text(report, encoding="utf-8")
    else:
        print(report, end="")


if __name__ == "__main__":
    main()
