# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Independent negative and concurrency checks for the replay spike fixture."""

from __future__ import annotations

import concurrent.futures
import http.client
import json
import threading
import time
import unittest
from collections.abc import Iterator
from contextlib import contextmanager
from http.server import ThreadingHTTPServer

from fixture import ProviderHandler


@contextmanager
def provider() -> Iterator[tuple[str, int]]:
    server = ThreadingHTTPServer(("127.0.0.1", 0), ProviderHandler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield server.server_address
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


def post(
    address: tuple[str, int],
    path: str,
    payload: object,
    headers: dict[str, str] | None = None,
) -> tuple[int, bytes, str]:
    connection = http.client.HTTPConnection(*address, timeout=3)
    body = json.dumps(payload).encode()
    request_headers = {"content-type": "application/json", **(headers or {})}
    connection.request("POST", path, body, request_headers)
    response = connection.getresponse()
    content_type = response.getheader("content-type", "")
    data = response.read()
    connection.close()
    return response.status, data, content_type


class FixtureTests(unittest.TestCase):
    def test_buffered_and_error_trajectories(self) -> None:
        with provider() as address:
            status, body, kind = post(
                address,
                "/v1/buffered?access=SYNTHETIC_QUERY_MARKER",
                {"model": "fixture-model"},
            )
            self.assertEqual(200, status)
            self.assertEqual("application/json", kind)
            self.assertEqual("fixture-model", json.loads(body)["request_model"])
            status, body, _ = post(address, "/v1/error", {})
            self.assertEqual(429, status)
            self.assertTrue(json.loads(body)["error"]["retryable"])

    def test_stream_preserves_tool_and_causal_identifiers(self) -> None:
        with provider() as address:
            status, body, kind = post(
                address,
                "/v1/stream",
                {"session": "alpha", "previous_response_id": "response-parent"},
            )
        self.assertEqual(200, status)
        self.assertEqual("text/event-stream", kind)
        text = body.decode()
        self.assertIn('"call_id": "call-alpha"', text)
        self.assertIn('"previous_response_id": "response-parent"', text)
        self.assertLess(
            text.index("response.created"), text.index("response.completed")
        )

    def test_parallel_identical_requests_remain_session_scoped(self) -> None:
        with (
            provider() as address,
            concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool,
        ):
            futures = [
                pool.submit(
                    post,
                    address,
                    "/v1/stream",
                    {"model": "same"},
                    {"x-asb-session": session},
                )
                for session in ("left", "right")
            ]
            bodies = [future.result()[1].decode() for future in futures]
        self.assertIn('"response_id": "response-left"', bodies[0])
        self.assertNotIn("response-right", bodies[0])
        self.assertIn('"response_id": "response-right"', bodies[1])
        self.assertNotIn("response-left", bodies[1])

    def test_invalid_and_unmatched_requests_fail_without_success(self) -> None:
        with provider() as address:
            connection = http.client.HTTPConnection(*address, timeout=3)
            connection.request("POST", "/v1/buffered", b"not-json")
            self.assertEqual(400, connection.getresponse().status)
            connection.close()
            status, _, _ = post(address, "/not-recorded", {})
            self.assertEqual(418, status)

    def test_truncated_stream_has_no_completion_marker(self) -> None:
        with provider() as address:
            status, body, kind = post(address, "/v1/truncated", {})
        self.assertEqual(200, status)
        self.assertEqual("text/event-stream", kind)
        self.assertIn(b"partial", body)
        self.assertNotIn(b"response.completed", body)

    def test_client_can_cancel_before_stream_completion(self) -> None:
        with provider() as address:
            connection = http.client.HTTPConnection(*address, timeout=3)
            body = json.dumps({"session": "cancelled"}).encode()
            connection.request(
                "POST", "/v1/stream", body, {"content-type": "application/json"}
            )
            response = connection.getresponse()
            started = time.monotonic()
            prefix = response.read(64)
            connection.close()
            elapsed = time.monotonic() - started
        self.assertEqual(200, response.status)
        self.assertIn(b"response.created", prefix)
        self.assertLess(elapsed, 0.1)


if __name__ == "__main__":
    unittest.main()
