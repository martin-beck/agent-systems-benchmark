# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Fail-closed, credential-free black-box qualification of MockAgents."""
from __future__ import annotations

import argparse
import hashlib
import http.client
import json
import platform
import re
import shutil
import socket
import subprocess
import tarfile
import tempfile
import time
from pathlib import Path
from typing import Any, cast
from urllib.parse import urlsplit
from urllib.request import Request, build_opener

ROOT = Path(__file__).parent
LOCK = ROOT / "mockagents-v0.5.0.lock.json"
MAX_OUTPUT = 1_048_576
MAX_ARCHIVE = 64 * 1024 * 1024
EXPECTED_MEMBERS = {"LICENSE", "README.md", "mockagents"}
HEX64 = re.compile(r"^[0-9a-f]{64}$")
HEX40 = re.compile(r"^[0-9a-f]{40}$")


class QualificationError(ValueError):
    """An artifact or executable did not satisfy the closed qualification contract."""


def _digest(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def load_lock(path: Path = LOCK) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if set(value) != {"schema_version", "candidate", "version", "source", "license", "artifacts", "checksums"}:
        raise QualificationError("lock file fields are not closed")
    if value["schema_version"] != 1 or value["candidate"] != "mockagents" or value["version"] != "0.5.0":
        raise QualificationError("unsupported candidate lock")
    source = value["source"]
    if set(source) != {"repository", "tag", "tag_object", "commit"} or source["tag"] != "v0.5.0":
        raise QualificationError("source pin is incomplete")
    for item in (source["tag_object"], source["commit"]):
        if not isinstance(item, str) or not re.fullmatch(r"[0-9a-f]{40}", item):
            raise QualificationError("source identity is not a SHA-1")
    license_pin = value["license"]
    if set(license_pin) != {"name", "git_blob"} or license_pin["name"] != "Apache-2.0" or not HEX40.fullmatch(license_pin["git_blob"]):
        raise QualificationError("license pin is incomplete")
    for artifact in value["artifacts"].values():
        if set(artifact) != {"url", "size", "sha256"} or not HEX64.fullmatch(artifact["sha256"]):
            raise QualificationError("artifact pin is incomplete")
        parsed = urlsplit(artifact["url"])
        if parsed.scheme != "https" or parsed.netloc != "github.com" or "/releases/download/v0.5.0/" not in parsed.path:
            raise QualificationError("artifact URL is outside the pinned GitHub release")
        if not isinstance(artifact["size"], int) or artifact["size"] <= 0 or artifact["size"] > MAX_ARCHIVE:
            raise QualificationError("artifact size is invalid")
    return cast(dict[str, Any], value)


class _PinnedRedirect:
    def __init__(self, expected: str) -> None:
        self.expected = expected

    def __call__(self, request: Request, response: Any) -> Any:
        location = response.headers.get("Location")
        if location != self.expected:
            raise QualificationError("release download redirected outside its exact URL")
        return response


def download(url: str, destination: Path, size: int, sha256: str) -> None:
    opener = build_opener()
    opener.addheaders = [("User-Agent", "asb-mockagents-qualification/1")]
    request = Request(url, method="GET")
    with opener.open(request, timeout=30) as response, destination.open("wb") as output:
        count = 0
        digest = hashlib.sha256()
        while block := response.read(1024 * 1024):
            count += len(block)
            if count > size:
                raise QualificationError("artifact exceeds pinned size")
            digest.update(block)
            output.write(block)
    if count != size or digest.hexdigest() != sha256:
        raise QualificationError("artifact size or digest mismatch")


def verify_archive(archive: Path, lock: dict[str, Any], destination: Path) -> Path:
    destination.mkdir(mode=0o700)
    try:
        with tarfile.open(archive, mode="r:gz") as tar:
            members = tar.getmembers()
            names = [member.name for member in members]
            if set(names) != EXPECTED_MEMBERS or len(names) != len(EXPECTED_MEMBERS):
                raise QualificationError("archive member set is not exact")
            for member in members:
                if member.name != Path(member.name).name or not member.isfile() or member.size > MAX_ARCHIVE:
                    raise QualificationError("archive contains unsafe member")
                if member.name == "mockagents" and member.mode & 0o111 == 0:
                    raise QualificationError("executable member is not executable")
                if member.name == "LICENSE":
                    license_path = destination / member.name
                    source = tar.extractfile(member)
                    if source is None:
                        raise QualificationError("license member cannot be read")
                    with source, license_path.open("wb") as output:
                        shutil.copyfileobj(source, output, length=1024 * 1024)
                    content = license_path.read_bytes()
                    blob_header = f"blob {len(content)}\0".encode()
                    if hashlib.sha1(blob_header + content).hexdigest() != lock["license"]["git_blob"]:
                        raise QualificationError("embedded license digest mismatch")
            tar.extractall(destination, filter="data")
    except (tarfile.TarError, OSError) as error:
        raise QualificationError("invalid release archive") from error
    executable = destination / "mockagents"
    if not executable.is_file() or _digest(executable) == "0" * 64:
        raise QualificationError("executable was not extracted")
    return executable


def _port() -> int:
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return int(listener.getsockname()[1])


def _post(port: int, path: str, payload: dict[str, Any], timeout: float = 3) -> tuple[int, bytes, str]:
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=timeout)
    connection.request("POST", path, json.dumps(payload).encode(), {"content-type": "application/json", "x-api-key": "mock", "authorization": "Bearer mock"})
    response = connection.getresponse()
    body = response.read(MAX_OUTPUT + 1)
    content_type = response.getheader("content-type", "")
    connection.close()
    if len(body) > MAX_OUTPUT:
        raise QualificationError("response exceeded output bound")
    return response.status, body, content_type


def _stable_hash(body: bytes) -> str:
    """Hash protocol semantics while excluding server-generated request identity."""
    try:
        value = json.loads(body)
    except json.JSONDecodeError:
        return hashlib.sha256(body).hexdigest()
    if isinstance(value, dict):
        value.pop("id", None)
        value.pop("created", None)
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def qualify(executable: Path) -> dict[str, Any]:
    with tempfile.TemporaryDirectory(prefix="asb-mockagents-") as state:
        root = Path(state)
        agents = root / "agents"
        agents.mkdir(mode=0o700)
        (agents / "openai.yaml").write_text(
            "apiVersion: mockagents/v1\nkind: Agent\nmetadata:\n  name: openai-fixture\nspec:\n  protocol: openai-chat-completions\n  model: openai-fixture\n  behavior:\n    scenarios:\n      - name: default\n        response:\n          content: READY\n",
            encoding="utf-8",
        )
        (agents / "anthropic.yaml").write_text(
            "apiVersion: mockagents/v1\nkind: Agent\nmetadata:\n  name: anthropic-fixture\nspec:\n  protocol: anthropic-messages\n  model: anthropic-fixture\n  behavior:\n    scenarios:\n      - name: default\n        response:\n          content: READY\n",
            encoding="utf-8",
        )
        port = _port()
        environment = {"PATH": str(executable.parent), "HOME": str(root), "LANG": "C", "NO_PROXY": "*", "no_proxy": "*"}
        process = subprocess.Popen(
            [str(executable), "start", "--host", "127.0.0.1", "--port", str(port), "--agents-dir", str(agents), "--no-color"],
            cwd=root, env=environment, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        )
        cases: dict[str, str] = {}
        try:
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline:
                if process.poll() is not None:
                    raise QualificationError("server exited before readiness")
                try:
                    status, body, _ = _post(port, "/v1/chat/completions", {"model": "openai-fixture", "messages": [{"role": "user", "content": "ready"}]})
                    if status == 200:
                        cases["openai_chat_buffered"] = _stable_hash(body)
                        if cases["openai_chat_buffered"] != _stable_hash(_post(port, "/v1/chat/completions", {"model": "openai-fixture", "messages": [{"role": "user", "content": "ready"}]})[1]):
                            raise QualificationError("response is not deterministic")
                        break
                except (ConnectionError, TimeoutError, OSError):
                    time.sleep(0.05)
            else:
                raise QualificationError("server readiness timeout")
            status, body, kind = _post(port, "/v1/messages", {"model": "anthropic-fixture", "messages": [{"role": "user", "content": "ready"}]})
            if status != 200 or "application/json" not in kind:
                raise QualificationError("anthropic buffered protocol failed")
            cases["anthropic_buffered"] = _stable_hash(body)
            status, body, kind = _post(port, "/v1/messages", {"model": "anthropic-fixture", "messages": [{"role": "user", "content": "ready"}], "stream": True})
            if status != 200 or "text/event-stream" not in kind or b"message_stop" not in body:
                raise QualificationError("anthropic SSE protocol failed")
            cases["anthropic_sse"] = hashlib.sha256(body).hexdigest()
            status, body, kind = _post(port, "/v1/responses", {"model": "openai-fixture", "input": "ready"})
            if status != 200 or "application/json" not in kind:
                raise QualificationError("OpenAI Responses protocol failed")
            cases["openai_responses"] = _stable_hash(body)
            status, body, _ = _post(port, "/v1/not-recorded", {})
            cases["unmatched_route"] = "pass" if status == 404 else "fail"
            if cases["unmatched_route"] != "pass":
                raise QualificationError("unmatched route was accepted")
        finally:
            process.terminate()
            try:
                process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=3)
            if process.poll() is None:
                raise QualificationError("server process did not terminate")
        return {"schema_version": 1, "evidence_class": "synthetic", "candidate": "mockagents", "version": "0.5.0", "platform": platform.machine(), "cases": cases, "network": "loopback-only", "credentials": "none"}


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--artifact", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    lock = load_lock()
    artifact_name = "linux-arm64" if platform.machine() in {"aarch64", "arm64"} else "linux-amd64"
    pin = lock["artifacts"][artifact_name]
    with tempfile.TemporaryDirectory(prefix="asb-mockagents-download-") as cache:
        archive = args.artifact or Path(cache) / "release.tar.gz"
        if args.artifact is None:
            download(pin["url"], archive, pin["size"], pin["sha256"])
        elif archive.stat().st_size != pin["size"] or _digest(archive) != pin["sha256"]:
            raise QualificationError("supplied artifact does not match lock")
        executable = verify_archive(archive, lock, Path(cache) / "unpacked")
        report = qualify(executable)
    payload = json.dumps(report, sort_keys=True, indent=2) + "\n"
    if args.output:
        args.output.write_text(payload, encoding="utf-8")
    else:
        print(payload, end="")


if __name__ == "__main__":
    main()
