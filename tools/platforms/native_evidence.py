#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Collect privacy-safe, fail-closed native Linux qualification evidence."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import re
import selectors
import signal
import subprocess
import time
from collections.abc import Callable, Sequence
from pathlib import Path
from typing import Any

MAX_SOURCE_BYTES = 1 << 20
MAX_OUTPUT_BYTES = 16 << 20
RUN_ID = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$")
COMMIT = re.compile(r"^[0-9a-f]{40}$")
ARCHES = {"x86_64", "aarch64"}
PLATFORMS = {
    "ubuntu-24.04": {
        "id": "ubuntu", "version_id": "24.04", "release_path": None,
        "release_value": "24.04.4 LTS", "release_mode": "contains",
        "manifest_release": "24.04.4 LTS",
    },
    "debian-13": {
        "id": "debian", "version_id": "13", "release_path": "etc/debian_version",
        "release_value": "13.6", "release_mode": "exact",
        "manifest_release": "13.6 (trixie)",
    },
    "openeuler-24.03-lts-sp2": {
        "id": "openEuler", "version_id": "24.03", "release_path": "etc/openEuler-release",
        "release_value": "LTS-SP2", "release_mode": "contains",
        "manifest_release": "24.03 LTS-SP2",
    },
}
SANDBOX_TOOL_PROFILES = {
    "ubuntu-24.04": (
        ("/usr/bin/bwrap", "bubblewrap 0.9.0", "bubblewrap=0.9.0-1ubuntu0.1",
         "https://packages.ubuntu.com/noble-updates/bubblewrap"),
        ("/usr/bin/systemd-run", "systemd 255 (255.4-1ubuntu8.17)",
         "systemd=255.4-1ubuntu8.17", "https://packages.ubuntu.com/noble-updates/systemd"),
        ("/usr/bin/systemctl", "systemd 255 (255.4-1ubuntu8.17)",
         "systemd=255.4-1ubuntu8.17", "https://packages.ubuntu.com/noble-updates/systemd"),
        ("/usr/bin/taskset", "taskset from util-linux 2.39.3", "util-linux=2.39.3-9ubuntu6.6",
         "https://packages.ubuntu.com/noble-updates/util-linux"),
    ),
    "debian-13": (
        ("/usr/bin/bwrap", "bubblewrap 0.12.0", "bubblewrap=0.12.0-1~deb13u1",
         "https://packages.debian.org/trixie/bubblewrap"),
        ("/usr/bin/systemd-run", "systemd 257 (257.13-1~deb13u1)",
         "systemd=257.13-1~deb13u1", "https://packages.debian.org/trixie/systemd"),
        ("/usr/bin/systemctl", "systemd 257 (257.13-1~deb13u1)",
         "systemd=257.13-1~deb13u1", "https://packages.debian.org/trixie/systemd"),
        ("/usr/bin/taskset", "taskset from util-linux 2.41.5", "util-linux=2.41.5-0+deb13u1",
         "https://packages.debian.org/trixie/util-linux"),
    ),
    "openeuler-24.03-lts-sp2": (
        ("/usr/bin/bwrap", "bubblewrap 0.8.0", "bubblewrap-0.8.0-2.oe2403sp2",
         "https://repo.openeuler.org/openEuler-24.03-LTS-SP2/source/Packages/"),
        ("/usr/bin/systemd-run", "systemd 255 (255-43.oe2403sp2)",
         "systemd-255-43.oe2403sp2",
         "https://repo.openeuler.org/openEuler-24.03-LTS-SP2/source/Packages/"),
        ("/usr/bin/systemctl", "systemd 255 (255-43.oe2403sp2)",
         "systemd-255-43.oe2403sp2",
         "https://repo.openeuler.org/openEuler-24.03-LTS-SP2/source/Packages/"),
        ("/usr/bin/taskset", "taskset from util-linux 2.39.1",
         "util-linux-2.39.1-22.oe2403sp2",
         "https://repo.openeuler.org/openEuler-24.03-LTS-SP2/source/Packages/"),
    ),
}


class EvidenceError(RuntimeError):
    """A qualification precondition or native check failed."""


def read_bounded(path: Path, limit: int = MAX_SOURCE_BYTES) -> str:
    """Read a regular file without following an oversized input."""
    with path.open("rb") as stream:
        data = stream.read(limit + 1)
    if len(data) > limit:
        raise EvidenceError("source exceeds byte limit")
    try:
        return data.decode("utf-8")
    except UnicodeDecodeError as error:
        raise EvidenceError("source is not UTF-8") from error


def parse_os_release(text: str) -> dict[str, str]:
    """Parse the bounded fields needed to bind a distribution release."""
    result: dict[str, str] = {}
    for raw in text.splitlines():
        if not raw or raw.startswith("#"):
            continue
        if "=" not in raw:
            raise EvidenceError("malformed os-release line")
        key, value = raw.split("=", 1)
        if not re.fullmatch(r"[A-Z][A-Z0-9_]*", key) or key in result:
            raise EvidenceError("invalid or duplicate os-release key")
        if value.startswith("\"") and value.endswith("\""):
            value = value[1:-1]
        if not value or any(ord(char) < 32 or ord(char) == 127 for char in value):
            raise EvidenceError("invalid os-release value")
        result[key] = value
    if "ID" not in result or "VERSION_ID" not in result:
        raise EvidenceError("os-release lacks ID or VERSION_ID")
    return result


def canonical_arch(value: str) -> str:
    """Return an ASB architecture and reject aliases or unknown machines."""
    if value not in ARCHES:
        raise EvidenceError("architecture is not canonical x86_64 or aarch64")
    return value


def validate_platform(platform_id: str, release: dict[str, str], root: Path) -> str:
    """Require the exact declared distribution and point-release evidence."""
    if platform_id not in PLATFORMS:
        raise EvidenceError("platform is not eligible for native qualification")
    profile = PLATFORMS[platform_id]
    if release["ID"] != profile["id"] or release["VERSION_ID"] != profile["version_id"]:
        raise EvidenceError("observed distribution does not match platform ID")
    release_evidence = (
        release.get("VERSION", "")
        if profile["release_path"] is None
        else read_bounded(root / profile["release_path"]).strip()
    )
    matches = (
        release_evidence == profile["release_value"]
        if profile["release_mode"] == "exact"
        else profile["release_value"] in release_evidence
    )
    if not matches:
        raise EvidenceError("observed distribution does not match exact pinned release")
    return release_evidence


def run_check(argv: Sequence[str], cwd: Path, timeout: int = 900) -> dict[str, Any]:
    """Run one argv-only check and retain only bounded digests, never raw output."""
    if not argv or any(not isinstance(item, str) or not item for item in argv):
        raise EvidenceError("check argv must be a nonempty string array")
    try:
        process = subprocess.Popen(
            list(argv), cwd=cwd, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT, close_fds=True, start_new_session=True,
        )
    except OSError as error:
        raise EvidenceError("native check could not start") from error
    assert process.stdout is not None
    digest = hashlib.sha256()
    size = 0
    deadline = time.monotonic() + timeout
    selector = selectors.DefaultSelector()
    selector.register(process.stdout, selectors.EVENT_READ)
    failure = ""
    try:
        while selector.get_map():
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                failure = "native check timed out"
                break
            events = selector.select(min(remaining, 1.0))
            if not events and process.poll() is not None:
                events = [(selector.get_key(process.stdout), selectors.EVENT_READ)]
            for key, _ in events:
                chunk = os.read(key.fd, 65536)
                if not chunk:
                    selector.unregister(key.fileobj)
                    continue
                size += len(chunk)
                if size > MAX_OUTPUT_BYTES:
                    failure = "native check output exceeded byte limit"
                    break
                digest.update(chunk)
            if failure:
                break
        if failure:
            os.killpg(process.pid, signal.SIGKILL)
        returncode = process.wait(timeout=10)
    except (OSError, subprocess.TimeoutExpired) as error:
        try:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=10)
        except (OSError, subprocess.TimeoutExpired):
            pass
        raise EvidenceError("native check could not complete") from error
    finally:
        selector.close()
        process.stdout.close()
    if failure:
        raise EvidenceError(failure)
    if returncode != 0:
        raise EvidenceError("native check failed")
    argv_digest = hashlib.sha256(
        json.dumps(list(argv), separators=(",", ":"), ensure_ascii=True).encode()
    ).hexdigest()
    return {
        "status": "passed",
        "argv_sha256": "sha256:" + argv_digest,
        "output_sha256": "sha256:" + digest.hexdigest(),
        "output_bytes": size,
    }


def command_output(argv: Sequence[str], cwd: Path) -> str:
    """Read a short probe result without inheriting stdin."""
    try:
        result = subprocess.run(
            list(argv), cwd=cwd, stdin=subprocess.DEVNULL, capture_output=True,
            text=True, timeout=15, check=False,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise EvidenceError("native probe could not complete") from error
    expected_none = result.returncode == 1 and result.stdout.strip() == "none"
    if (result.returncode != 0 and not expected_none) or len(result.stdout.encode()) > MAX_SOURCE_BYTES:
        raise EvidenceError("native probe failed")
    return result.stdout.strip()


def sandbox_capable(platform_id: str, cwd: Path) -> bool:
    """Prove the exact pinned tools and disposable user scope are available."""
    for executable, expected, _, _ in SANDBOX_TOOL_PROFILES[platform_id]:
        try:
            result = subprocess.run(
                [executable, "--version"], cwd=cwd, stdin=subprocess.DEVNULL,
                capture_output=True, text=True, timeout=15, check=False,
            )
        except (OSError, subprocess.TimeoutExpired):
            return False
        if result.returncode != 0 or expected not in result.stdout:
            return False
    try:
        scope = subprocess.run(
            ["/usr/bin/systemd-run", "--user", "--wait", "--quiet", "/usr/bin/true"],
            cwd=cwd, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL, timeout=30, check=False,
        )
    except (OSError, subprocess.TimeoutExpired):
        return False
    return scope.returncode == 0


def sandbox_tool_evidence(platform_id: str) -> list[dict[str, str]]:
    """Return reviewed package pins and official provenance for the platform."""
    return [
        {"executable": executable, "version": version, "package": package, "source": source}
        for executable, version, package, source in SANDBOX_TOOL_PROFILES[platform_id]
    ]


def security_module_state(root: Path) -> dict[str, str]:
    """Retain bounded active AppArmor and SELinux state, or explicit unavailability."""
    try:
        active = {
            item.strip()
            for item in read_bounded(root / "sys/kernel/security/lsm").split(",")
        }
    except (EvidenceError, OSError):
        return {"apparmor": "unavailable", "selinux": "unavailable"}
    apparmor = "enabled" if "apparmor" in active else "disabled"
    if "selinux" not in active:
        selinux = "disabled"
    else:
        try:
            enforcing = read_bounded(root / "sys/fs/selinux/enforce").strip()
        except (EvidenceError, OSError):
            selinux = "unavailable"
        else:
            selinux = {"0": "permissive", "1": "enforcing"}.get(
                enforcing, "unavailable"
            )
    return {"apparmor": apparmor, "selinux": selinux}


def collect(
    platform_id: str,
    expected_arch: str,
    run_id: str,
    source: Path,
    checks: list[tuple[str, list[str]]],
    optional_checks: list[tuple[str, list[str]]] | None = None,
    root: Path = Path("/"),
    probe: Callable[[Sequence[str], Path], str] = command_output,
    sandbox_probe: Callable[[str, Path], bool] | None = None,
) -> dict[str, Any]:
    """Collect an evidence document, failing before output on any mismatch."""
    if not RUN_ID.fullmatch(run_id):
        raise EvidenceError("run ID is empty, unsafe, or oversized")
    arch = canonical_arch(platform.machine())
    if arch != canonical_arch(expected_arch):
        raise EvidenceError("observed architecture differs from requested architecture")
    release = parse_os_release(read_bounded(root / "etc/os-release"))
    release_evidence = validate_platform(platform_id, release, root)
    kernel = platform.release()
    if not kernel or len(kernel.encode()) > 255 or any(ord(char) < 33 for char in kernel):
        raise EvidenceError("kernel release is invalid")
    cgroup = read_bounded(root / "proc/self/cgroup")
    if not any(line.startswith("0::") for line in cgroup.splitlines()):
        raise EvidenceError("cgroup v2 unified membership is unavailable")
    for name in ("cpu", "memory", "io"):
        pressure = read_bounded(root / f"proc/pressure/{name}")
        if "some " not in pressure:
            raise EvidenceError("required PSI evidence is unavailable")
    container_kind = probe(["systemd-detect-virt", "--container"], source)
    if container_kind and container_kind != "none":
        raise EvidenceError("container execution cannot qualify a native cell")
    virtualization = probe(["systemd-detect-virt", "--vm"], source) or "none"
    if virtualization.lower() in {"qemu", "uml", "bochs"}:
        raise EvidenceError("emulated execution cannot qualify a native cell")
    commit = probe(["git", "rev-parse", "HEAD"], source)
    if not COMMIT.fullmatch(commit):
        raise EvidenceError("source commit is not immutable")
    if probe(["git", "status", "--porcelain"], source):
        raise EvidenceError("source tree is dirty")
    optional_checks = optional_checks or []
    all_checks = checks + optional_checks
    if not checks or len({name for name, _ in all_checks}) != len(all_checks):
        raise EvidenceError("checks must be nonempty and uniquely named")
    results = {name: run_check(argv, source) for name, argv in checks}
    sandbox_probe = sandbox_probe or sandbox_capable
    for name, argv in optional_checks:
        if name != "sandbox":
            raise EvidenceError("only the native sandbox check may be optional")
        if sandbox_probe(platform_id, source):
            results[name] = run_check(argv, source)
        else:
            results[name] = {"status": "unavailable"}
    security_modules = security_module_state(root)
    complete = (
        all(result["status"] == "passed" for result in results.values())
        and "unavailable" not in security_modules.values()
    )
    return {
        "format_version": 1,
        "kind": "native-run",
        "qualification": "native-functional" if complete else "native-functional-partial",
        "performance_baseline": False,
        "platform_id": platform_id,
        "architecture": arch,
        "distribution": {
            "id": release["ID"],
            "version_id": release["VERSION_ID"],
            "version": release.get("VERSION", ""),
            "release_evidence": release_evidence,
            "manifest_release": PLATFORMS[platform_id]["manifest_release"],
        },
        "kernel_release": kernel,
        "virtualization": virtualization,
        "run_id": run_id,
        "source_commit": commit,
        "capabilities": {
            "cgroup_v2": "available",
            "psi": "available",
            "sandbox": results.get("sandbox", {"status": "not-run"})["status"],
            "security_modules": security_modules,
        },
        "sandbox_tools": sandbox_tool_evidence(platform_id),
        "checks": results,
    }


def write_atomic(path: Path, report: dict[str, Any]) -> None:
    """Write canonical JSON without following an existing symlink."""
    path = path.absolute()
    ancestor = path.parent
    missing: list[Path] = []
    while not ancestor.exists():
        missing.append(ancestor)
        if ancestor.parent == ancestor:
            raise EvidenceError("output parent has no existing ancestor")
        ancestor = ancestor.parent
    if ancestor.is_symlink() or ancestor.resolve(strict=True) != ancestor:
        raise EvidenceError("output parent must not contain symlinks")
    for directory in reversed(missing):
        directory.mkdir()
        if directory.is_symlink() or directory.resolve(strict=True) != directory:
            raise EvidenceError("output parent must not contain symlinks")
    if path.is_symlink() or path.exists():
        raise EvidenceError("output path must not already exist")
    data = (json.dumps(report, indent=2, sort_keys=True) + "\n").encode()
    temporary = path.with_name(path.name + ".tmp-" + str(os.getpid()))
    descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        with os.fdopen(descriptor, "wb") as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        try:
            temporary.unlink()
        except FileNotFoundError:
            pass


def parse_check(value: str) -> tuple[str, list[str]]:
    """Parse NAME=JSON_ARRAY without shell evaluation."""
    if "=" not in value:
        raise argparse.ArgumentTypeError("check must be NAME=JSON_ARRAY")
    name, encoded = value.split("=", 1)
    if not re.fullmatch(r"[a-z][a-z0-9_-]{0,63}", name):
        raise argparse.ArgumentTypeError("check name is invalid")
    try:
        argv = json.loads(encoded)
    except json.JSONDecodeError as error:
        raise argparse.ArgumentTypeError("check command is not JSON") from error
    if not isinstance(argv, list) or not argv or not all(isinstance(x, str) and x for x in argv):
        raise argparse.ArgumentTypeError("check command must be a string array")
    return name, argv


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--platform-id", required=True)
    parser.add_argument("--architecture", required=True)
    parser.add_argument("--run-id", required=True)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--check", action="append", type=parse_check, required=True)
    parser.add_argument("--optional-check", action="append", type=parse_check, default=[])
    args = parser.parse_args()
    try:
        report = collect(
            args.platform_id, args.architecture, args.run_id,
            args.source.resolve(strict=True), args.check, args.optional_check,
        )
        write_atomic(args.output, report)
    except EvidenceError as error:
        print(f"ERROR: {error}")
        return 1
    print(f"wrote native evidence for {report['platform_id']}/{report['architecture']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
