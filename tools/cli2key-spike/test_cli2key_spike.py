# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Contract, privacy, and hostile-boundary tests for cli2key."""

from __future__ import annotations

import json
import threading
import time
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from unittest import mock

from cli2key_spike import (
    REPORT_FIELDS,
    SYNTHETIC_AUTH_MARKER,
    Endpoint,
    SpikeError,
    discover_models,
    fake_spike,
    load_contract,
    parse_endpoint,
    request,
)


class Cli2KeySpikeTests(unittest.TestCase):
    def test_contract_pins_source_license_protocol_and_key_distinction(self) -> None:
        contract = load_contract()
        bridge = contract["bridge"]
        self.assertEqual("MIT", bridge["license"])
        self.assertEqual("v0.1.10", bridge["version"])
        self.assertEqual(40, len(bridge["revision"]))
        self.assertEqual(64, len(bridge["archive_sha256"]))
        self.assertEqual("GET /v1/models", contract["protocol"]["models"])
        self.assertEqual("POST /v1/responses", contract["protocol"]["responses"])
        self.assertFalse(contract["credential_workflow"]["local_client_key_is_platform_api_key"])
        self.assertFalse(contract["credential_workflow"]["asb_reads_codex_auth_files"])
        self.assertFalse(contract["credential_workflow"]["bridge_private_oauth_store"])
        self.assertTrue(contract["credential_workflow"]["bridge_private_client_key_store"])
        self.assertTrue(contract["runtime_requirements"]["fresh_client_key_per_invocation"])
        upstream = contract["protocol"]["upstream_authentication"]
        self.assertIn("user-selected CODEX_HOME", upstream)
        self.assertIn("read-only", upstream)
        self.assertIn("separate local client key", upstream)
        self.assertIn("~/.cb", upstream)
        self.assertNotIn("OAuth stored by the bridge", upstream)

    def test_fake_proves_two_routes_and_omits_sensitive_content(self) -> None:
        private_input = "input-must-not-enter-evidence"
        report = fake_spike(private_input)
        encoded = json.dumps(report, sort_keys=True)
        self.assertEqual(REPORT_FIELDS, set(report))
        self.assertEqual("synthetic", report["evidence_class"])
        self.assertEqual("passed", report["models_status"])
        self.assertEqual(1, report["models_count"])
        self.assertEqual("passed", report["responses_status"])
        self.assertNotIn(private_input, encoded)
        self.assertNotIn("Authorization", encoded)
        self.assertNotIn("output", encoded)

    def test_nested_agent_and_authority_claims_are_prohibited(self) -> None:
        prohibitions = " ".join(load_contract()["prohibitions"])
        self.assertIn("app-server", prohibitions)
        self.assertIn("nested-agent", prohibitions)
        self.assertIn("OpenAI Platform API key", prohibitions)
        self.assertIn("production readiness", prohibitions)

    def test_non_loopback_and_ambiguous_endpoints_fail_closed(self) -> None:
        for endpoint in (
            "https://127.0.0.1:8317",
            "http://localhost:8317",
            "http://0.0.0.0:8317",
            "http://127.0.0.1:8317/v1",
            "http://user@127.0.0.1:8317",
            "http://127.0.0.1:8317?key=value",
        ):
            with (
                self.subTest(endpoint=endpoint),
                self.assertRaisesRegex(SpikeError, "non_loopback_endpoint"),
            ):
                parse_endpoint(endpoint)
        self.assertEqual("127.0.0.1", parse_endpoint("http://127.0.0.1:8317").host)
        self.assertEqual("::1", parse_endpoint("http://[::1]:8317").host)

    def test_model_identity_that_could_smuggle_private_content_is_rejected(
        self,
    ) -> None:
        from cli2key_spike import Endpoint, FakeHandler, ThreadingHTTPServer, threading

        original = FakeHandler.do_GET

        def hostile(handler: FakeHandler) -> None:
            handler.reply(200, {"data": [{"id": "/private/path"}]})

        FakeHandler.do_GET = hostile
        FakeHandler.client_key = SYNTHETIC_AUTH_MARKER
        server = ThreadingHTTPServer(("127.0.0.1", 0), FakeHandler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            host, port = server.server_address
            with self.assertRaisesRegex(SpikeError, "models_malformed"):
                discover_models(Endpoint(host, port), SYNTHETIC_AUTH_MARKER)
        finally:
            server.shutdown()
            server.server_close()
            thread.join()
            FakeHandler.do_GET = original

    def test_taxonomy_covers_bounded_auth_discovery_and_response_failures(self) -> None:
        failures = set(load_contract()["failure_taxonomy"])
        self.assertTrue(
            {
                "opt_in_required",
                "authentication_rejected",
                "models_oversized",
                "model_limit_exceeded",
                "response_oversized",
                "deadline_exceeded",
            }.issubset(failures)
        )

    def test_slow_drip_is_aborted_at_absolute_deadline(self) -> None:
        class SlowDripHandler(BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def log_message(self, _format: str, *_args: object) -> None:
                return

            def do_GET(self) -> None:
                self.send_response(200)
                self.send_header("content-length", "64")
                self.end_headers()
                try:
                    for _ in range(64):
                        self.wfile.write(b"x")
                        self.wfile.flush()
                        time.sleep(0.02)
                except OSError:
                    pass

        server = ThreadingHTTPServer(("127.0.0.1", 0), SlowDripHandler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        started = time.monotonic()
        try:
            host, port = server.server_address
            with (
                mock.patch("cli2key_spike.TIMEOUT_SECONDS", 0.15),
                self.assertRaisesRegex(SpikeError, "deadline_exceeded"),
            ):
                request(
                    Endpoint(host, port),
                    "GET",
                    "/slow",
                    SYNTHETIC_AUTH_MARKER,
                    None,
                    128,
                    "response_oversized",
                )
            self.assertLess(time.monotonic() - started, 1.0)
        finally:
            server.shutdown()
            server.server_close()
            thread.join()


if __name__ == "__main__":
    unittest.main()
