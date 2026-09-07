# SPDX-License-Identifier: MIT
"""Credential-free OpenAI stream fixture for the pinned Qwen Code test."""

from __future__ import annotations

import json
import os
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    requests = 0

    def log_message(self, _format: str, *_args: object) -> None:
        return

    def do_POST(self) -> None:
        if self.path != "/v1/chat/completions":
            self.send_error(404)
            return
        length = int(self.headers.get("content-length", "0"))
        if length <= 0 or length > 4 * 1024 * 1024:
            self.send_error(400)
            return
        request = json.loads(self.rfile.read(length))
        if request.get("stream") is not True:
            self.send_error(400)
            return
        Handler.requests += 1
        if os.environ.get("ASB_QWEN_MODE") == "hang":
            with open(os.environ["ASB_QWEN_READY"], "x", encoding="utf-8") as ready:
                ready.write("ready\n")
            time.sleep(60)
            return
        if os.environ.get("ASB_QWEN_MODE") == "error":
            body = b'{"error":{"message":"private provider diagnostic"}}'
            self.send_response(400)
            self.send_header("content-type", "application/json")
            self.send_header("content-length", str(len(body)))
            self.send_header("connection", "close")
            self.end_headers()
            self.wfile.write(body)
            self.wfile.flush()
            self.close_connection = True
            return
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.send_header("cache-control", "no-cache")
        self.send_header("connection", "close")
        self.end_headers()
        if os.environ.get("ASB_QWEN_MODE") == "edit" and Handler.requests == 1:
            target = os.environ["ASB_QWEN_TARGET"]
            self.event(
                {
                    "id": "chatcmpl-tool",
                    "object": "chat.completion.chunk",
                    "created": 1,
                    "model": "fixture-model",
                    "choices": [
                        {
                            "index": 0,
                            "delta": {
                                "role": "assistant",
                                "tool_calls": [
                                    {
                                        "index": 0,
                                        "id": "call-asb",
                                        "type": "function",
                                        "function": {
                                            "name": "write_file",
                                            "arguments": json.dumps(
                                                {
                                                    "file_path": target,
                                                    "content": "new\n",
                                                }
                                            ),
                                        },
                                    }
                                ],
                            },
                            "finish_reason": None,
                        }
                    ],
                }
            )
            self.event(
                {
                    "id": "chatcmpl-tool",
                    "object": "chat.completion.chunk",
                    "created": 1,
                    "model": "fixture-model",
                    "choices": [
                        {"index": 0, "delta": {}, "finish_reason": "tool_calls"}
                    ],
                }
            )
        else:
            self.event(
                {
                    "id": "chatcmpl-final",
                    "object": "chat.completion.chunk",
                    "created": 1,
                    "model": "fixture-model",
                    "choices": [
                        {
                            "index": 0,
                            "delta": {"role": "assistant", "content": "done"},
                            "finish_reason": None,
                        }
                    ],
                }
            )
            self.event(
                {
                    "id": "chatcmpl-final",
                    "object": "chat.completion.chunk",
                    "created": 1,
                    "model": "fixture-model",
                    "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
                }
            )
        self.event(
            {
                "id": "chatcmpl-usage",
                "object": "chat.completion.chunk",
                "created": 1,
                "model": "fixture-model",
                "choices": [],
                "usage": {
                    "prompt_tokens": 9,
                    "completion_tokens": 3,
                    "total_tokens": 12,
                },
            }
        )
        self.wfile.write(b"data: [DONE]\n\n")
        self.wfile.flush()
        self.close_connection = True

    def event(self, value: object) -> None:
        self.wfile.write(b"data: " + json.dumps(value).encode() + b"\n\n")


def main() -> None:
    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    print(server.server_address[1], flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
