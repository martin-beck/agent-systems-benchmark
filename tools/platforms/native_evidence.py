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
import stat
import subprocess
import time
from collections.abc import Callable, Sequence
from pathlib import Path
from typing import Any

MAX_SOURCE_BYTES = 1 << 20
MAX_OUTPUT_BYTES = 16 << 20
MAX_BINARY_BYTES = 256 << 20
RUN_ID = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$")
COMMIT = re.compile(r"^[0-9a-f]{40}$")
ARCHES = {"x86_64", "aarch64"}
PLATFORMS = {
    "ubuntu-24.04": {
        "id": "ubuntu", "version_id": "24.04", "release_path": None,
        "version": "24.04.4 LTS (Noble Numbat)",
        "release_value": "24.04.4 LTS (Noble Numbat)",
        "manifest_release": "24.04.4 LTS",
        "package_manager": "dpkg",
        "kernel_source": "https://packages.ubuntu.com/noble-updates/kernel/",
    },
    "debian-13": {
        "id": "debian", "version_id": "13", "release_path": "etc/debian_version",
        "version": "13 (trixie)", "release_value": "13.6",
        "manifest_release": "13.6 (trixie)",
        "package_manager": "dpkg",
        "kernel_source": "https://packages.debian.org/trixie/kernel/",
    },
    "openeuler-24.03-lts-sp2": {
        "id": "openEuler", "version_id": "24.03", "release_path": "etc/openEuler-release",
        "version": "24.03 (LTS-SP2)",
        "release_value": "openEuler release 24.03 (LTS-SP2)",
        "manifest_release": "24.03 LTS-SP2",
        "package_manager": "rpm",
        "kernel_source": "https://repo.openeuler.org/openEuler-24.03-LTS-SP2/source/Packages/",
    },
}
SANDBOX_TOOL_PROFILES = {
    "ubuntu-24.04": (
        ("/usr/bin/bwrap", "bubblewrap 0.9.0", "bubblewrap", "0.9.0-1ubuntu0.1",
         "https://packages.ubuntu.com/noble-updates/bubblewrap"),
        ("/usr/bin/systemd-run", "systemd 255 (255.4-1ubuntu8.17)", "systemd",
         "255.4-1ubuntu8.17", "https://packages.ubuntu.com/noble-updates/systemd"),
        ("/usr/bin/systemctl", "systemd 255 (255.4-1ubuntu8.17)", "systemd",
         "255.4-1ubuntu8.17", "https://packages.ubuntu.com/noble-updates/systemd"),
        ("/usr/bin/taskset", "taskset from util-linux 2.39.3", "util-linux",
         "2.39.3-9ubuntu6.6",
         "https://packages.ubuntu.com/noble-updates/util-linux"),
    ),
    "debian-13": (
        ("/usr/bin/bwrap", "bubblewrap 0.12.0", "bubblewrap", "0.12.0-1~deb13u1",
         "https://packages.debian.org/trixie/bubblewrap"),
        ("/usr/bin/systemd-run", "systemd 257 (257.13-1~deb13u1)", "systemd",
         "257.13-1~deb13u1", "https://packages.debian.org/trixie/systemd"),
        ("/usr/bin/systemctl", "systemd 257 (257.13-1~deb13u1)", "systemd",
         "257.13-1~deb13u1", "https://packages.debian.org/trixie/systemd"),
        ("/usr/bin/taskset", "taskset from util-linux 2.41.5", "util-linux",
         "2.41.5-0+deb13u1",
         "https://packages.debian.org/trixie/util-linux"),
    ),
    "openeuler-24.03-lts-sp2": (
        ("/usr/bin/bwrap", "bubblewrap 0.8.0", "bubblewrap", "0.8.0-2.oe2403sp2",
         "https://repo.openeuler.org/openEuler-24.03-LTS-SP2/source/Packages/"),
        ("/usr/bin/systemd-run", "systemd 255 (255-43.oe2403sp2)", "systemd",
         "255-43.oe2403sp2",
         "https://repo.openeuler.org/openEuler-24.03-LTS-SP2/source/Packages/"),
        ("/usr/bin/systemctl", "systemd 255 (255-43.oe2403sp2)", "systemd",
         "255-43.oe2403sp2",
         "https://repo.openeuler.org/openEuler-24.03-LTS-SP2/source/Packages/"),
        ("/usr/bin/taskset", "taskset from util-linux 2.39.1", "util-linux",
         "2.39.1-22.oe2403sp2",
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
    if release.get("VERSION") != profile["version"]:
        raise EvidenceError("observed distribution does not match exact pinned release")
    release_evidence = (
        release.get("VERSION", "")
        if profile["release_path"] is None
        else read_bounded(root / profile["release_path"]).strip()
    )
    if release_evidence != profile["release_value"]:
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


def digest_file(path: Path, limit: int = MAX_BINARY_BYTES) -> tuple[str, str]:
    """Hash a bounded regular file without following its final component."""
    try:
        descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
        metadata = os.fstat(descriptor)
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_size > limit:
            raise EvidenceError("integrity input is not a bounded regular file")
        sha256 = hashlib.sha256()
        md5 = hashlib.md5(usedforsecurity=False)
        size = 0
        while True:
            chunk = os.read(descriptor, 65536)
            if not chunk:
                break
            size += len(chunk)
            if size > limit:
                raise EvidenceError("integrity input exceeds byte limit")
            sha256.update(chunk)
            md5.update(chunk)
        return sha256.hexdigest(), md5.hexdigest()
    except OSError as error:
        raise EvidenceError("integrity input could not be read") from error
    finally:
        try:
            os.close(descriptor)
        except (NameError, OSError):
            pass


def dpkg_metadata(
    executable: str, package: str, expected_version: str, architecture: str,
    root: Path, cwd: Path, probe: Callable[[Sequence[str], Path], str],
) -> dict[str, str]:
    """Bind a Debian-family executable to installed package metadata and md5sums."""
    owner = probe(["/usr/bin/dpkg-query", "-S", executable], cwd)
    if owner.rsplit(": ", 1) != [package, executable]:
        raise EvidenceError("sandbox executable package ownership differs")
    metadata = probe(
        ["/usr/bin/dpkg-query", "-W", "-f=${binary:Package}\\t${Version}\\t${Architecture}\\t${Status}", package],
        cwd,
    ).split("\t")
    expected_arch = {"x86_64": "amd64", "aarch64": "arm64"}[architecture]
    if metadata != [package, expected_version, expected_arch, "install ok installed"]:
        raise EvidenceError("sandbox executable package metadata differs")
    relative = executable.removeprefix("/")
    manifest = root / f"var/lib/dpkg/info/{package}.md5sums"
    text = read_bounded(manifest)
    entries = [line.split("  ", 1) for line in text.splitlines() if "  " in line]
    matches = [digest for digest, name in entries if name == relative]
    if len(matches) != 1 or not re.fullmatch(r"[0-9a-f]{32}", matches[0]):
        raise EvidenceError("package manifest does not bind sandbox executable")
    binary_sha256, binary_md5 = digest_file(root / relative)
    if binary_md5 != matches[0]:
        raise EvidenceError("sandbox executable differs from package manifest")
    return {
        "package": package,
        "package_version": expected_version,
        "package_architecture": expected_arch,
        "binary_sha256": "sha256:" + binary_sha256,
        "package_manifest_sha256": "sha256:" + hashlib.sha256(text.encode()).hexdigest(),
        "package_integrity": "verified",
    }


def rpm_metadata(
    executable: str, package: str, expected_version: str, architecture: str,
    root: Path, cwd: Path, probe: Callable[[Sequence[str], Path], str],
) -> dict[str, str]:
    """Bind an RPM-family executable to its owner and rpm verification result."""
    metadata = probe(
        ["/usr/bin/rpm", "-qf", "--qf", "%{NAME}\\t%{EVR}\\t%{ARCH}", executable], cwd
    ).split("\t")
    expected_arch = architecture
    if metadata != [package, expected_version, expected_arch]:
        raise EvidenceError("sandbox executable RPM metadata differs")
    verification = probe(["/usr/bin/rpm", "-Vf", executable], cwd)
    if verification:
        raise EvidenceError("sandbox executable failed RPM integrity verification")
    binary_sha256, _ = digest_file(root / executable.removeprefix("/"))
    package_record = "\t".join(metadata).encode()
    return {
        "package": package,
        "package_version": expected_version,
        "package_architecture": expected_arch,
        "binary_sha256": "sha256:" + binary_sha256,
        "package_manifest_sha256": "sha256:" + hashlib.sha256(package_record).hexdigest(),
        "package_integrity": "verified",
    }


def sandbox_tool_evidence(
    platform_id: str, architecture: str, root: Path, cwd: Path,
    probe: Callable[[Sequence[str], Path], str] = command_output,
) -> list[dict[str, str]]:
    """Observe exact versions, ownership and binary/package integrity."""
    manager = PLATFORMS[platform_id]["package_manager"]
    evidence = []
    for executable, expected, package, package_version, source in SANDBOX_TOOL_PROFILES[platform_id]:
        version_output = probe([executable, "--version"], cwd)
        first_line = version_output.splitlines()[0] if version_output else ""
        if first_line != expected:
            raise EvidenceError("sandbox executable version differs")
        observe = dpkg_metadata if manager == "dpkg" else rpm_metadata
        package_evidence = observe(
            executable, package, package_version, architecture, root, cwd, probe
        )
        evidence.append({
            "executable": executable,
            "version": first_line,
            "version_output_sha256": "sha256:" + hashlib.sha256(version_output.encode()).hexdigest(),
            "source": source,
            **package_evidence,
        })
    return evidence


def sandbox_capable(platform_id: str, architecture: str, root: Path, cwd: Path) -> bool:
    """Mirror the production constrained-scope and namespace capability probe."""
    try:
        sandbox_tool_evidence(platform_id, architecture, root, cwd)
    except EvidenceError:
        return False
    for executable, expected, _, _, _ in SANDBOX_TOOL_PROFILES[platform_id]:
        try:
            result = subprocess.run(
                [executable, "--version"], cwd=cwd, stdin=subprocess.DEVNULL,
                capture_output=True, text=True, timeout=15, check=False,
            )
        except (OSError, subprocess.TimeoutExpired):
            return False
        lines = result.stdout.splitlines()
        if result.returncode != 0 or not lines or lines[0] != expected:
            return False
    unit = f"asb-native-probe-{os.getpid()}-{time.monotonic_ns():x}"
    delegation = [
        "/usr/bin/systemd-run", "--user", "--scope", "--quiet", "--collect",
        f"--unit={unit}.scope",
        "--property", "MemoryMax=67108864",
        "--property", "MemorySwapMax=0",
        "--property", "TasksMax=16",
        "--property", "CPUQuota=100%",
        "--property", "RuntimeMaxSec=10000ms",
        "/usr/bin/bwrap", "--die-with-parent", "--new-session", "--unshare-all",
        "--unshare-user", "--clearenv", "--disable-userns", "--assert-userns-disabled",
        "--cap-drop", "ALL", "--ro-bind", "/usr", "/usr",
    ]
    for runtime_path in ("/bin", "/lib", "/lib64", "/etc/ld.so.cache"):
        delegation.extend(("--ro-bind-try", runtime_path, runtime_path))
    delegation.extend(("--", "/usr/bin/true"))
    try:
        scope = subprocess.run(
            delegation,
            cwd=cwd, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL, timeout=30, check=False,
        )
    except (OSError, subprocess.TimeoutExpired):
        return False
    return scope.returncode == 0


def kernel_provenance(
    platform_id: str, architecture: str, kernel: str, root: Path, cwd: Path,
    probe: Callable[[Sequence[str], Path], str] = command_output,
) -> dict[str, str]:
    """Bind the running kernel to installed package metadata and live kernel notes."""
    manager = PLATFORMS[platform_id]["package_manager"]
    image = f"/boot/vmlinuz-{kernel}"
    expected_arch = {"x86_64": "amd64", "aarch64": "arm64"}[architecture]
    if manager == "dpkg":
        owner = probe(["/usr/bin/dpkg-query", "-S", image], cwd)
        if not owner.endswith(": " + image):
            raise EvidenceError("booted kernel package ownership is unavailable")
        package = owner.rsplit(": ", 1)[0]
        fields = probe(
            ["/usr/bin/dpkg-query", "-W", "-f=${binary:Package}\\t${Version}\\t${Architecture}\\t${Status}", package],
            cwd,
        ).split("\t")
        if fields[:1] != [package] or fields[2:] != [expected_arch, "install ok installed"]:
            raise EvidenceError("booted kernel package metadata differs")
        package_version = fields[1]
        manifest = read_bounded(root / f"var/lib/dpkg/info/{package}.md5sums")
        image_name = image.removeprefix("/")
        image_digests = [
            line.split("  ", 1)[0] for line in manifest.splitlines()
            if line.endswith("  " + image_name)
        ]
        if len(image_digests) != 1 or not re.fullmatch(r"[0-9a-f]{32}", image_digests[0]):
            raise EvidenceError("booted kernel image is absent from package manifest")
        integrity_digest = hashlib.sha256(manifest.encode()).hexdigest()
        package_integrity = "package-manifest-bound"
    else:
        fields = probe(
            ["/usr/bin/rpm", "-qf", "--qf", "%{NAME}\\t%{EVR}\\t%{ARCH}", image], cwd
        ).split("\t")
        if len(fields) != 3 or fields[2] != architecture:
            raise EvidenceError("booted kernel RPM metadata differs")
        package, package_version, _ = fields
        verification = probe(["/usr/bin/rpm", "-Vf", image], cwd)
        if verification:
            raise EvidenceError("booted kernel failed RPM integrity verification")
        image_digests = ["rpm-verified"]
        integrity_digest = hashlib.sha256("\t".join(fields).encode()).hexdigest()
        package_integrity = "rpm-verified"
    notes_sha256, _ = digest_file(root / "sys/kernel/notes", MAX_SOURCE_BYTES)
    version = read_bounded(root / "proc/version")
    signature_path = root / "proc/version_signature"
    try:
        signature = read_bounded(signature_path).strip()
    except OSError:
        signature = "unavailable"
    if signature != "unavailable" and (
        not signature or len(signature.encode()) > 512
        or any(ord(character) < 32 or ord(character) == 127 for character in signature)
    ):
        raise EvidenceError("booted kernel version signature is unsafe")
    return {
        "package": package,
        "package_version": package_version,
        "package_architecture": fields[2],
        "package_source": PLATFORMS[platform_id]["kernel_source"],
        "package_integrity": package_integrity,
        "package_manifest_sha256": "sha256:" + integrity_digest,
        "image_package_digest": image_digests[0],
        "running_notes_sha256": "sha256:" + notes_sha256,
        "running_version_sha256": "sha256:" + hashlib.sha256(version.encode()).hexdigest(),
        "version_signature": signature,
    }


def source_identity(
    source: Path, base_commit: str,
    probe: Callable[[Sequence[str], Path], str] = command_output,
) -> tuple[str, str]:
    """Require a clean immutable source descendant and return commit/tree identity."""
    if not COMMIT.fullmatch(base_commit):
        raise EvidenceError("source base commit is not immutable")
    commit = probe(["git", "rev-parse", "HEAD"], source)
    tree = probe(["git", "rev-parse", "HEAD^{tree}"], source)
    if not COMMIT.fullmatch(commit) or not COMMIT.fullmatch(tree):
        raise EvidenceError("source commit/tree ancestry is not immutable")
    merge_base = probe(["git", "merge-base", base_commit, commit], source)
    if merge_base != base_commit:
        raise EvidenceError("source commit/tree ancestry is not immutable")
    if probe(["git", "status", "--porcelain"], source):
        raise EvidenceError("source tree is dirty")
    return commit, tree


def security_module_state(root: Path) -> dict[str, str]:
    """Retain bounded active AppArmor and SELinux state, or explicit unavailability."""
    try:
        active = {
            item.strip()
            for item in read_bounded(root / "sys/kernel/security/lsm").split(",")
        }
    except (EvidenceError, OSError):
        return {"apparmor": "unavailable", "selinux": "unavailable"}
    if (
        not active
        or "" in active
        or "capability" not in active
        or any(not re.fullmatch(r"[a-z][a-z0-9_-]{0,63}", item) for item in active)
    ):
        return {"apparmor": "unavailable", "selinux": "unavailable"}
    apparmor = "registered-unproven" if "apparmor" in active else "not-registered"
    if "selinux" not in active:
        selinux = "not-registered"
    else:
        try:
            enforcing = read_bounded(root / "sys/fs/selinux/enforce").strip()
        except (EvidenceError, OSError):
            selinux = "unavailable"
        else:
            selinux = {"0": "permissive-observed", "1": "enforcing-observed"}.get(
                enforcing, "unavailable"
            )
    return {"apparmor": apparmor, "selinux": selinux}


def collect(
    platform_id: str,
    expected_arch: str,
    run_id: str,
    source: Path,
    base_commit: str,
    checks: list[tuple[str, list[str]]],
    optional_checks: list[tuple[str, list[str]]] | None = None,
    root: Path = Path("/"),
    probe: Callable[[Sequence[str], Path], str] = command_output,
    sandbox_probe: Callable[[str, str, Path, Path], bool] | None = None,
    tool_evidence_probe: Callable[..., list[dict[str, str]]] | None = None,
    kernel_evidence_probe: Callable[..., dict[str, str]] | None = None,
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
    if virtualization.lower() not in {
        "none", "kvm", "vmware", "microsoft", "oracle", "xen", "zvm",
        "parallels", "amazon", "google",
    }:
        raise EvidenceError("emulated execution cannot qualify a native cell")
    commit, tree = source_identity(source, base_commit, probe)
    optional_checks = optional_checks or []
    all_checks = checks + optional_checks
    if (
        {name for name, _ in checks} != {"process", "metrics"}
        or [name for name, _ in optional_checks] != ["sandbox"]
        or len({name for name, _ in all_checks}) != len(all_checks)
    ):
        raise EvidenceError("checks must be exactly process, metrics, and optional sandbox")
    tool_evidence_probe = tool_evidence_probe or sandbox_tool_evidence
    tools = tool_evidence_probe(platform_id, arch, root, source, probe)
    kernel_evidence_probe = kernel_evidence_probe or kernel_provenance
    kernel_evidence = kernel_evidence_probe(platform_id, arch, kernel, root, source, probe)
    def execute(name: str, argv: list[str]) -> dict[str, Any]:
        try:
            return run_check(argv, source)
        except EvidenceError as error:
            raise EvidenceError(f"{name} check did not pass ({error})") from error

    results = {name: execute(name, argv) for name, argv in checks}
    sandbox_probe = sandbox_probe or sandbox_capable
    for name, argv in optional_checks:
        if sandbox_probe(platform_id, arch, root, source):
            results[name] = execute(name, argv)
        else:
            results[name] = {"status": "unavailable"}
    final_commit, final_tree = source_identity(source, base_commit, probe)
    if (final_commit, final_tree) != (commit, tree):
        raise EvidenceError("source identity changed while checks ran")
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
        "kernel_provenance": kernel_evidence,
        "virtualization": virtualization,
        "run_id": run_id,
        "source_commit": commit,
        "source_tree": tree,
        "source_base_commit": base_commit,
        "capabilities": {
            "cgroup_v2": "available",
            "psi": "available",
            "sandbox": results.get("sandbox", {"status": "not-run"})["status"],
            "security_modules": security_modules,
        },
        "sandbox_tools": tools,
        "checks": results,
    }


def write_atomic(path: Path, report: dict[str, Any], trusted_root: Path) -> None:
    """Write canonical JSON via component-wise no-follow traversal below a trusted root."""
    path = path.absolute()
    trusted_root = trusted_root.absolute()
    if ".." in path.parts or ".." in trusted_root.parts:
        raise EvidenceError("output path must be normalized")
    try:
        relative = path.relative_to(trusted_root)
    except ValueError as error:
        raise EvidenceError("output path escapes trusted root") from error
    if not relative.parts or relative.name in {"", ".", ".."}:
        raise EvidenceError("output path is invalid")
    data = (json.dumps(report, indent=2, sort_keys=True) + "\n").encode()
    flags = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW
    try:
        parent_fd = os.open("/", flags)
        for component in trusted_root.parts[1:]:
            if component in {"", ".", ".."}:
                raise EvidenceError("trusted output root is invalid")
            child_fd = os.open(component, flags, dir_fd=parent_fd)
            os.close(parent_fd)
            parent_fd = child_fd
        for component in relative.parent.parts:
            if component in {"", ".", ".."}:
                raise EvidenceError("output parent is invalid")
            try:
                child_fd = os.open(component, flags, dir_fd=parent_fd)
            except FileNotFoundError:
                os.mkdir(component, mode=0o700, dir_fd=parent_fd)
                child_fd = os.open(component, flags, dir_fd=parent_fd)
            os.close(parent_fd)
            parent_fd = child_fd
    except OSError as error:
        try:
            os.close(parent_fd)
        except (NameError, OSError):
            pass
        raise EvidenceError("output parent must not contain symlinks") from error
    try:
        try:
            os.stat(path.name, dir_fd=parent_fd, follow_symlinks=False)
        except FileNotFoundError:
            pass
        else:
            os.close(parent_fd)
            raise EvidenceError("output path must not already exist")
        temporary = path.name + ".tmp-" + str(os.getpid())
        descriptor = os.open(
            temporary,
            os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
            0o600,
            dir_fd=parent_fd,
        )
    except OSError as error:
        os.close(parent_fd)
        raise EvidenceError("output path could not be created safely") from error
    try:
        with os.fdopen(descriptor, "wb") as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        os.link(
            temporary, path.name, src_dir_fd=parent_fd, dst_dir_fd=parent_fd,
            follow_symlinks=False,
        )
        os.unlink(temporary, dir_fd=parent_fd)
        os.fsync(parent_fd)
    except OSError as error:
        raise EvidenceError("output path could not be committed safely") from error
    finally:
        try:
            os.unlink(temporary, dir_fd=parent_fd)
        except FileNotFoundError:
            pass
        os.close(parent_fd)


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
    parser.add_argument("--base-commit", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--output-root", type=Path, required=True)
    parser.add_argument("--check", action="append", type=parse_check, required=True)
    parser.add_argument("--optional-check", action="append", type=parse_check, default=[])
    args = parser.parse_args()
    try:
        report = collect(
            args.platform_id, args.architecture, args.run_id,
            args.source.resolve(strict=True), args.base_commit, args.check, args.optional_check,
        )
        write_atomic(args.output, report, args.output_root.resolve(strict=True))
    except EvidenceError as error:
        print(f"ERROR: {error}")
        return 1
    print(f"wrote native evidence for {report['platform_id']}/{report['architecture']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
