# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Run a bounded qualification command in the approved offline container."""
from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import time
from pathlib import Path

IMAGE = "ubuntu@sha256:33ceb71981b602c1a7443a53469e4dba065f7503eab3078a2d7a57a2ab987517"
PROJECT_ROOT = Path("/srv/data/projects")
FORBIDDEN = {"sh", "bash", "dash", "zsh", "-c", "--privileged", "--network=host"}


def build_command(artifact: Path, command: list[str]) -> list[str]:
    if artifact.is_symlink():
        raise ValueError("artifact must not be a symlink")
    resolved = artifact.resolve()
    if not resolved.is_relative_to(PROJECT_ROOT) or not resolved.is_file():
        raise ValueError("artifact must be an existing non-symlink file under /srv/data/projects")
    if not command or any(Path(part).name in FORBIDDEN or part in FORBIDDEN for part in command):
        raise ValueError("a direct executable argument vector is required; shell commands are rejected")
    return [
        "sudo", "-n", "docker", "run", "--rm", "--network", "none", "--read-only",
        "--cap-drop", "ALL", "--security-opt", "no-new-privileges:true", "--pids-limit", "128",
        "--memory", "2g", "--cpus", "2", "--ipc", "private",
        "--tmpfs", "/tmp:rw,noexec,nosuid,nodev,size=256m", "--tmpfs", "/run:rw,noexec,nosuid,nodev,size=64m",
        "--mount", f"type=bind,src={resolved},dst=/input/artifact,readonly", IMAGE, *command,
    ]


def verify_image() -> None:
    try:
        result = subprocess.run(
            ["sudo", "-n", "docker", "image", "inspect", IMAGE, "--format", "{{index .RepoDigests 0}}"],
            stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            timeout=10, check=True, text=True,
        )
    except (OSError, subprocess.SubprocessError) as error:
        raise ValueError("approved Docker image could not be verified") from error
    if result.stdout.strip() != IMAGE:
        raise ValueError("Docker image digest does not match the approved identity")


def verify_artifact(path: Path, expected: str) -> None:
    if len(expected) != 64 or any(char not in "0123456789abcdef" for char in expected):
        raise ValueError("artifact SHA-256 is not a lowercase hexadecimal digest")
    digest = hashlib.sha256(path.read_bytes()).hexdigest()
    if digest != expected:
        raise ValueError("artifact does not match its pinned SHA-256")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--artifact", required=True, type=Path)
    parser.add_argument("--artifact-sha256", required=True)
    parser.add_argument("--timeout", type=int, default=120)
    parser.add_argument("--verify-network-none", action="store_true")
    parser.add_argument("--verify-artifact-version", action="store_true")
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.timeout <= 0 or args.timeout > 600:
        parser.error("timeout must be between 1 and 600 seconds")
    try:
        verify_image()
        verify_artifact(args.artifact, args.artifact_sha256)
        requested = args.command[1:] if args.command[:1] == ["--"] else args.command
        if args.verify_network_none:
            if requested:
                parser.error("network verification does not accept an additional command")
            requested = ["/bin/cat", "/proc/net/route"]
        if args.verify_artifact_version:
            if requested or args.verify_network_none:
                parser.error("artifact version verification does not accept another mode or command")
            requested = ["/input/artifact", "--version"]
        command = build_command(args.artifact, requested)
    except ValueError as error:
        parser.error(str(error))
    started = time.monotonic()
    try:
        result = subprocess.run(command, stdin=subprocess.DEVNULL,
                                stdout=subprocess.PIPE if (args.verify_network_none or args.verify_artifact_version) else subprocess.DEVNULL,
                                stderr=subprocess.DEVNULL, timeout=args.timeout, check=False)
    except subprocess.TimeoutExpired:
        print(json.dumps({"status": "timeout"}, sort_keys=True))
        return 124
    if args.verify_network_none:
        lines = [line for line in result.stdout.splitlines() if line.strip()]
        if result.returncode != 0 or len(lines) != 1 or not lines[0].startswith(b"Iface"):
            print(json.dumps({"status": "network-denial-failed"}, sort_keys=True))
            return 1
        print(json.dumps({"status": "network-none-verified"}, sort_keys=True))
        return 0
    if args.verify_artifact_version:
        if result.returncode != 0 or b"0.5.0" not in result.stdout:
            print(json.dumps({"status": "artifact-identity-failed"}, sort_keys=True))
            return 1
        print(json.dumps({"status": "artifact-version-verified"}, sort_keys=True))
        return 0
    elapsed_ms = int((time.monotonic() - started) * 1000)
    print(json.dumps({"status": "completed", "exit_code": result.returncode, "elapsed_ms": elapsed_ms}, sort_keys=True))
    return result.returncode


if __name__ == "__main__":
    raise SystemExit(main())
