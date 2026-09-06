# SPDX-License-Identifier: MIT
"""Synthetic provider used to evaluate response record/replay candidates.

This is research tooling, not an ASB provider implementation. It deliberately
uses only the Python standard library so a candidate can proxy the same wire
traffic without installing an SDK or contacting an external service.
"""

from __future__ import annotations

import argparse
import json
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlsplit


def _event(kind: str, **values: object) -> bytes:
    payload = {"type": kind, **values}
    return f"event: {kind}\ndata: {json.dumps(payload, sort_keys=True)}\n\n".encode()


class ProviderHandler(BaseHTTPRequestHandler):
    """Serve deterministic buffered, SSE, error, and truncated trajectories."""

    protocol_version = "HTTP/1.1"

    def log_message(self, format: str, *args: object) -> None:
        del format, args

    def do_POST(self) -> None:
        length = int(self.headers.get("content-length", "0"))
        try:
            request = json.loads(self.rfile.read(length))
        except (UnicodeDecodeError, json.JSONDecodeError):
            self._buffered(400, {"error": {"type": "invalid_json"}})
            return

        path = urlsplit(self.path).path
        if path == "/v1/buffered":
            self._buffered(
                200,
                {
                    "id": "response-buffered-1",
                    "output": [{"type": "message", "text": "fixture response"}],
                    "request_model": request.get("model"),
                },
            )
        elif path == "/v1/stream":
            self._stream(request)
        elif path == "/v1/error":
            self._buffered(429, {"error": {"type": "rate_limit", "retryable": True}})
        elif path == "/v1/truncated":
            self._truncated()
        else:
            self._buffered(418, {"error": {"type": "unmatched_fixture_route"}})

    def _buffered(self, status: int, payload: object) -> None:
        body = json.dumps(payload, sort_keys=True, separators=(",", ":")).encode()
        self.send_response(status)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.send_header("connection", "close")
        self.end_headers()
        self.wfile.write(body)
        self.close_connection = True

    def _stream(self, request: dict[str, object]) -> None:
        session = self.headers.get("x-asb-session") or str(
            request.get("session", "missing")
        )
        previous = self.headers.get("x-asb-previous-response") or str(
            request.get("previous_response_id", "none")
        )
        events = [
            _event(
                "response.created",
                response_id=f"response-{session}",
                previous_response_id=previous,
            ),
            _event("response.output_text.delta", delta="fixture "),
            _event(
                "response.function_call_arguments.delta",
                call_id=f"call-{session}",
                delta='{"path":',
            ),
            _event(
                "response.function_call_arguments.done",
                call_id=f"call-{session}",
                arguments='{"path":"src/lib.rs"}',
            ),
            _event(
                "response.completed",
                response_id=f"response-{session}",
                usage={"input_tokens": 7, "output_tokens": 5},
            ),
        ]
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.send_header("cache-control", "no-cache")
        self.send_header("connection", "close")
        self.end_headers()
        for event in events:
            try:
                self.wfile.write(event)
                self.wfile.flush()
            except (BrokenPipeError, ConnectionResetError):
                self.close_connection = True
                return
            time.sleep(0.025)
        self.close_connection = True

    def _truncated(self) -> None:
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.send_header("connection", "close")
        self.end_headers()
        self.wfile.write(_event("response.output_text.delta", delta="partial"))
        self.wfile.flush()
        self.close_connection = True


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=0)
    parser.add_argument("--write-address")
    args = parser.parse_args()
    server = ThreadingHTTPServer((args.host, args.port), ProviderHandler)
    address = f"http://{server.server_address[0]}:{server.server_address[1]}"
    if args.write_address:
        with open(args.write_address, "w", encoding="utf-8") as stream:
            stream.write(address + "\n")
    else:
        print(address, flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
