# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Credential-free Codex Responses fixture for the ignored native test."""

import json
import os
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any

port_file, workspace, mode = sys.argv[1:4]
count = 0
base: dict[str, Any] = {
    "id": "resp_fixture",
    "object": "response",
    "created_at": 1,
    "status": "in_progress",
    "error": None,
    "incomplete_details": None,
    "instructions": None,
    "max_output_tokens": None,
    "model": "fixture-model",
    "output": [],
    "parallel_tool_calls": True,
    "previous_response_id": None,
    "reasoning": {"effort": None, "summary": None},
    "store": False,
    "temperature": 1.0,
    "text": {"format": {"type": "text"}},
    "tool_choice": "auto",
    "tools": [],
    "top_p": 1.0,
    "truncation": "disabled",
    "usage": None,
    "metadata": {},
}


def ev(kind: str, **data: Any) -> bytes:
    return (
        "event: "
        + kind
        + "\ndata: "
        + json.dumps({"type": kind, **data}, separators=(",", ":"))
        + "\n\n"
    ).encode()


def send(handler: BaseHTTPRequestHandler, events: list[bytes]) -> None:
    handler.send_response(200)
    handler.send_header("content-type", "text/event-stream")
    handler.send_header("connection", "close")
    handler.end_headers()
    for event in events:
        handler.wfile.write(event)
        handler.wfile.flush()
    handler.close_connection = True


def completed(output: list[dict[str, Any]]) -> dict[str, Any]:
    return {
        **base,
        "status": "completed",
        "output": output,
        "usage": {
            "input_tokens": 7,
            "input_tokens_details": {"cached_tokens": 0},
            "output_tokens": 3,
            "output_tokens_details": {"reasoning_tokens": 0},
            "total_tokens": 10,
        },
    }


class H(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *args: Any) -> None:
        pass

    def do_GET(self) -> None:
        body = json.dumps({"models": []}).encode()
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self) -> None:
        global count
        length = int(self.headers.get("content-length", "0"))
        if length <= 0 or length > 4 * 1024 * 1024:
            os._exit(2)
        request = json.loads(self.rfile.read(length))
        if (
            not self.path.endswith("/responses")
            or request.get("model") != "fixture-model"
        ):
            os._exit(3)
        if self.headers.get("authorization") != "Bearer asb-credential-free-fixture":
            os._exit(4)
        inputs = request.get("input", [])
        has_result = any(
            isinstance(x, dict) and x.get("type") == "function_call_output"
            for x in inputs
        )
        if count == 0 and has_result:
            os._exit(5)
        count += 1
        if not has_result:
            if mode == "journey":
                command = "printf fixture-tool > tool-output.txt"
            else:
                command = "sh -c 'exec -a asb-codex-ar0304-child sleep 60'"
            args = json.dumps(
                {
                    "cmd": command,
                    "workdir": workspace,
                    "yield_time_ms": 1000,
                    "max_output_tokens": 2000,
                }
            )
            call = {
                "id": "fc_fixture",
                "type": "function_call",
                "status": "completed",
                "call_id": "call_fixture",
                "name": "exec_command",
                "arguments": args,
            }
            send(
                self,
                [
                    ev("response.created", response=base),
                    ev("response.output_item.added", output_index=0, item=call),
                    ev(
                        "response.function_call_arguments.done",
                        item_id="fc_fixture",
                        output_index=0,
                        name="exec_command",
                        arguments=args,
                    ),
                    ev("response.output_item.done", output_index=0, item=call),
                    ev("response.completed", response=completed([call])),
                ],
            )
            return
        if mode != "journey":
            os._exit(6)
        item = {
            "id": "msg_fixture",
            "type": "message",
            "status": "completed",
            "role": "assistant",
            "content": [
                {"type": "output_text", "text": "fixture complete", "annotations": []}
            ],
        }
        send(
            self,
            [
                ev("response.created", response=base),
                ev(
                    "response.output_item.added",
                    output_index=0,
                    item={**item, "status": "in_progress", "content": []},
                ),
                ev("response.output_item.done", output_index=0, item=item),
                ev("response.completed", response=completed([item])),
            ],
        )
        threading.Thread(target=self.server.shutdown, daemon=True).start()


server = ThreadingHTTPServer(("127.0.0.1", 0), H)
with open(port_file, "w", encoding="utf-8") as stream:
    stream.write(str(server.server_address[1]))
server.serve_forever()
