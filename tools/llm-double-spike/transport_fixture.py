# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Bounded credential-free transport lifecycle fixture for MockAgents evidence."""
from __future__ import annotations
import argparse, http.client, json, os, signal, socket, subprocess, sys, threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

MAX_BODY = 64 * 1024

class FixtureHandler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    events = [{"kind": "tool_call", "sequence": 0}, {"kind": "tool_result", "sequence": 1}, {"kind": "completed", "sequence": 2}]
    def log_message(self, *_args: object) -> None: return
    def do_POST(self) -> None:
        length = int(self.headers.get("content-length", "0"))
        if length > MAX_BODY:
            self.send_error(413)
            return
        self.rfile.read(length)
        payload = json.dumps(self.events, separators=(",", ":")).encode()
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(payload)))
        self.send_header("x-asb-session", "fixture")
        self.end_headers()
        self.wfile.write(payload)
        self.wfile.flush()

def loopback_ordering() -> bytes:
    server = ThreadingHTTPServer(("127.0.0.1", 0), FixtureHandler)
    thread = threading.Thread(target=server.serve_forever, daemon=True); thread.start()
    try:
        connection = http.client.HTTPConnection("127.0.0.1", server.server_port, timeout=3)
        connection.request("POST", "/v1/fixture", b"{}", {"content-type": "application/json"})
        response = connection.getresponse(); events = json.loads(response.read(MAX_BODY + 1)); connection.close()
        if response.status != 200 or [event["kind"] for event in events] != ["tool_call", "tool_result", "completed"]:
            raise ValueError("tool-result ordering failed")
        return json.dumps(events, separators=(",", ":")).encode()
    finally:
        server.shutdown(); server.server_close(); thread.join(timeout=3)

def cancellation_cleanup() -> bool:
    process = subprocess.Popen([sys.executable, "-c", "import subprocess,time; subprocess.Popen([\"sleep\", \"30\"]); time.sleep(30)"], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
    os.killpg(process.pid, signal.SIGTERM)
    try: process.wait(timeout=3)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGKILL); process.wait(timeout=3)
    return process.poll() is not None

def invoke_runner(runner: Path, artifact: Path, digest: str) -> bool:
    result = subprocess.run([sys.executable, str(runner), "--image", "python@sha256:ed86c82274b3c69b52fb5820f358f0bd7df0b603332063cb5c6e32bd220c3e6e", "--artifact", str(artifact), "--artifact-sha256", digest, "--verify-artifact-version"], stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, timeout=120, text=True, check=False)
    return result.returncode == 0 and "artifact-version-verified" in result.stdout

def outbound_attempt() -> str:
    try:
        with socket.create_connection(("192.0.2.1", 9), timeout=1): return "unexpectedly-connected"
    except (OSError, TimeoutError): return "unavailable-outside-isolation"

def main() -> int:
    parser = argparse.ArgumentParser(); parser.add_argument("--executable", type=Path); parser.add_argument("--runner", type=Path); parser.add_argument("--artifact", type=Path); parser.add_argument("--artifact-sha256"); args = parser.parse_args()
    first = loopback_ordering()
    if first != loopback_ordering() or not cancellation_cleanup():
        print(json.dumps({"status": "failed"}, sort_keys=True)); return 1
    report = {"status": "passed", "tool_result_order": "verified", "repeat_clean_state": "verified", "cancellation_cleanup": "verified", "outbound": outbound_attempt()}
    if args.executable is not None and (not args.executable.is_file() or not args.executable.stat().st_mode & 0o111):
        print(json.dumps({"status": "failed"}, sort_keys=True)); return 1
    if args.runner is not None:
        if args.artifact is None or args.artifact_sha256 is None or not invoke_runner(args.runner, args.artifact, args.artifact_sha256):
            print(json.dumps({"status": "runner-invocation-failed"}, sort_keys=True)); return 1
        report["mockagents_runner"] = "pinned-version-verified"
    print(json.dumps(report, sort_keys=True)); return 0

if __name__ == "__main__": raise SystemExit(main())
