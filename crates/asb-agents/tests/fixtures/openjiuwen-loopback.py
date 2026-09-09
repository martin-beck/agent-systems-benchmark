#!/usr/bin/env python3
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Credential-free OpenAI-compatible loopback fixture for pinned OpenJiuwen."""
from __future__ import annotations

import argparse, json, pathlib, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
MAX_BODY = 1024 * 1024
PUBLIC_SENTINEL = "asb-loopback-public-sentinel"
PROMPT_SENTINEL = "ASB_OPENJIUWEN_PROMPT"
def event(payload): return ("data: " + json.dumps(payload, separators=(",", ":")) + "\n\n").encode()
def chunk(delta, finish=None, usage=None):
    value = {"id":"asb-openjiuwen-fixture","object":"chat.completion.chunk","created":1,"model":"asb-loopback","choices":[{"index":0,"delta":delta,"finish_reason":finish}]}
    if usage is not None: value["usage"] = usage
    return value
def main():
    parser = argparse.ArgumentParser()
    for item in ("port-file", "receipt", "target", "events"): parser.add_argument("--" + item, required=True)
    parser.add_argument("--mode", choices=("success", "usage", "delay", "malformed", "tool_corrupt", "http_error", "trickle", "child_escape"), required=True)
    args = parser.parse_args()
    events = [json.loads(line) for line in pathlib.Path(args.events).read_text().splitlines()]
    requests = [item["request"] for item in events if "request" in item]
    scenarios = {item["scenario"] for item in events if "scenario" in item}
    if requests != [1, 2] or scenarios != {"tool_corrupt", "http_error", "trickle", "usage", "child_escape"}: raise SystemExit("fixture scenario inventory is not closed")
    if events[1]["prompt_tokens"] <= 0 or events[1]["completion_tokens"] <= 0 or events[1]["prompt_tokens"] + events[1]["completion_tokens"] != events[1]["total_tokens"]: raise SystemExit("fixture usage is not positive and internally consistent")
    state = {"requests": 0, "tool_result_seen": False}
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, _format, *_args): return
        def do_POST(self):
            if self.client_address[0] not in ("127.0.0.1", "::1"): self.send_error(403); return
            length = int(self.headers.get("content-length", "0"))
            if length <= 0 or length > MAX_BODY: self.send_error(413); return
            try: body = json.loads(self.rfile.read(length))
            except Exception: self.send_error(400); return
            state["requests"] += 1
            state["tool_result_seen"] = any(item.get("role") == "tool" for item in body.get("messages", []) if isinstance(item, dict))
            valid = {"path":self.path == "/v1/chat/completions","authorization":self.headers.get("authorization") == "Bearer " + PUBLIC_SENTINEL,"model":body.get("model") == "asb-loopback","stream":body.get("stream") is True,"prompt":PROMPT_SENTINEL in json.dumps(body.get("messages", []))}
            pathlib.Path(args.receipt).write_text(json.dumps({"requests":state["requests"],"tool_result_seen":state["tool_result_seen"],"loopback":True,**valid}, sort_keys=True) + "\n")
            if not all(valid.values()) or state["requests"] > 8: self.send_error(400); return
            if args.mode == "delay": time.sleep(30); return
            if args.mode == "http_error": self.send_error(503); return
            self.send_response(200); self.send_header("content-type", "text/event-stream"); self.end_headers()
            if args.mode == "malformed": self.wfile.write(b"data: {not-json}\n\n"); self.wfile.flush(); return
            if args.mode == "trickle": self.wfile.write(b"data: "); self.wfile.flush(); time.sleep(30); return
            if args.mode == "usage":
                self.wfile.write(event(chunk({"role":"assistant","content":events[1]["content"]})))
                self.wfile.write(event(chunk({}, "stop", {"prompt_tokens":events[1]["prompt_tokens"],"completion_tokens":events[1]["completion_tokens"],"total_tokens":events[1]["total_tokens"]})))
                receipt = json.loads(pathlib.Path(args.receipt).read_text()); receipt["positive_usage_sent"] = True
                pathlib.Path(args.receipt).write_text(json.dumps(receipt, sort_keys=True) + "\n")
            elif not state["tool_result_seen"]:
                names = [t.get("function", {}).get("name", "") for t in body.get("tools", [])]
                bash = next((name for name in names if name == "bash" or name.startswith("bash_")), "")
                if not bash: self.send_error(400); return
                if args.mode == "tool_corrupt": arguments = "{"
                elif args.mode == "child_escape": arguments = json.dumps({"command":"setsid /bin/sh -c 'sleep 2; printf escaped > " + args.target + ".escape' >/dev/null 2>&1 & printf %s $! > " + args.target + ".pid; sleep 30"})
                else: arguments = json.dumps({"command":"printf 'ASB_OPENJIUWEN_EDIT\\n' > " + args.target})
                self.wfile.write(event(chunk({"role":"assistant","tool_calls":[{"index":0,"id":"asb_call_1","type":"function","function":{"name":bash,"arguments":arguments}}]})))
                self.wfile.write(event(chunk({}, "tool_calls")))
            else:
                if not state["tool_result_seen"]: self.send_error(400); return
                self.wfile.write(event(chunk({"role":"assistant","content":events[1]["content"]})))
                self.wfile.write(event(chunk({}, "stop", {"prompt_tokens":events[1]["prompt_tokens"],"completion_tokens":events[1]["completion_tokens"],"total_tokens":events[1]["total_tokens"]})))
                receipt = json.loads(pathlib.Path(args.receipt).read_text())
                receipt["positive_usage_sent"] = True
                pathlib.Path(args.receipt).write_text(json.dumps(receipt, sort_keys=True) + "\n")
            self.wfile.write(b"data: [DONE]\n\n"); self.wfile.flush()
    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    pathlib.Path(args.port_file).write_text(str(server.server_port) + "\n")
    server.serve_forever()
if __name__ == "__main__": main()
