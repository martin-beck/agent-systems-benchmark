#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Qualify one bounded, disposable native x86_64 development-host cell."""

from __future__ import annotations

import argparse
import fcntl
import hashlib
import json
import os
import platform
import re
import selectors
import shutil
import signal
import stat
import subprocess
import time
from collections.abc import Callable, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Any

MAX_TEXT_BYTES = 1 << 20
MAX_OUTPUT_BYTES = 16 << 20
MAX_FILE_BYTES = 256 << 20
RUN_ID = re.compile(r"^[a-z0-9][a-z0-9-]{0,63}$")
COMMIT = re.compile(r"^[0-9a-f]{40}$")
SHA256 = re.compile(r"^sha256:[0-9a-f]{64}$")
EXPECTED_RELEASE = {
    "ID": "ubuntu",
    "VERSION_ID": "24.04",
    "VERSION": "24.04.4 LTS (Noble Numbat)",
}
EXPECTED_PACKAGES = {
    "/usr/bin/bwrap": ("bubblewrap", "0.9.0-1ubuntu0.1"),
    "/usr/bin/systemd-run": ("systemd", "255.4-1ubuntu8.17"),
    "/usr/bin/taskset": ("util-linux", "2.39.3-9ubuntu6.6"),
}
UBUNTU_PACKAGE_SOURCE = "https://archive.ubuntu.com/ubuntu/dists/noble-updates/"
EXPECTED_KERNEL = "7.0.0-28-generic"
EXPECTED_KERNEL_PACKAGE = "linux-image-7.0.0-28-generic"
EXPECTED_KERNEL_PACKAGE_VERSION = "7.0.0-28.28~24.04.1"
CHECKS = {
    "metrics": ("-p", "asb-metrics", "--test", "native_linux"),
    "process": ("-p", "asb-runtime", "--test", "process_boundary"),
    "sandbox": ("-p", "asb-runtime", "--test", "sandbox_boundary"),
    "workloads": ("-p", "asb-workloads", "--test", "public_api"),
}


class CapacityError(RuntimeError):
    """A native capacity precondition or qualification check failed."""


@dataclass(frozen=True)
class Limits:
    """Reviewed resource limits for one local qualification cell."""

    cpu_quota_percent: int
    memory_mib: int
    storage_mib: int
    timeout_seconds: int
    tasks_max: int

    def validate(
        self, online_cpus: int, memory_mib: int, free_storage_mib: int
    ) -> None:
        """Fail closed unless the request is bounded and leaves host headroom."""
        if not 100 <= self.cpu_quota_percent <= min(800, online_cpus * 100):
            raise CapacityError("CPU quota is outside the reviewed bound")
        if not 1024 <= self.memory_mib <= min(16384, memory_mib // 2):
            raise CapacityError("memory limit is outside the reviewed bound")
        if not 2048 <= self.storage_mib <= min(32768, free_storage_mib // 2):
            raise CapacityError("storage limit is outside the reviewed bound")
        if not 60 <= self.timeout_seconds <= 1800:
            raise CapacityError("runtime limit is outside the reviewed bound")
        if not 32 <= self.tasks_max <= 512:
            raise CapacityError("task limit is outside the reviewed bound")


def read_regular(path: Path, limit: int = MAX_TEXT_BYTES) -> bytes:
    """Read one bounded regular file without following its final component."""
    descriptor = -1
    try:
        descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
        metadata = os.fstat(descriptor)
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_size > limit:
            raise CapacityError("input is not a bounded regular file")
        chunks = []
        size = 0
        while True:
            chunk = os.read(descriptor, 65536)
            if not chunk:
                break
            size += len(chunk)
            if size > limit:
                raise CapacityError("input exceeds byte limit")
            chunks.append(chunk)
        return b"".join(chunks)
    except OSError as error:
        raise CapacityError("input cannot be read safely") from error
    finally:
        if descriptor >= 0:
            os.close(descriptor)


def safe_text(data: bytes) -> str:
    """Decode bounded UTF-8 text and reject control characters."""
    try:
        value = data.decode("utf-8").strip()
    except UnicodeDecodeError as error:
        raise CapacityError("input is not UTF-8") from error
    if not value or any(
        (ord(char) < 32 and char not in {"\n", "\t"}) or ord(char) == 127
        for char in value
    ):
        raise CapacityError("input contains unsafe text")
    return value


def parse_os_release(data: bytes) -> dict[str, str]:
    """Parse the small exact release identity needed for qualification."""
    result: dict[str, str] = {}
    for raw in safe_text(data).splitlines():
        if raw.startswith("#"):
            continue
        if "=" not in raw:
            raise CapacityError("os-release is malformed")
        key, value = raw.split("=", 1)
        if not re.fullmatch(r"[A-Z][A-Z0-9_]*", key) or key in result:
            raise CapacityError("os-release key is invalid or duplicated")
        if len(value) >= 2 and value[0] == value[-1] == '"':
            value = value[1:-1]
        if not value or any(ord(char) < 32 or ord(char) == 127 for char in value):
            raise CapacityError("os-release value is unsafe")
        result[key] = value
    if any(result.get(key) != value for key, value in EXPECTED_RELEASE.items()):
        raise CapacityError("host distribution is not the reviewed exact release")
    return result


def read_os_release(
    link: Path = Path("/etc/os-release"),
    target: Path = Path("/usr/lib/os-release"),
    expected_uid: int = 0,
) -> bytes:
    """Follow only the reviewed distribution identity symlink to a root-owned file."""
    try:
        link_metadata = link.lstat()
        target_metadata = target.lstat()
        expected = os.path.relpath(target, link.parent)
        if (
            not stat.S_ISLNK(link_metadata.st_mode)
            or os.readlink(link) != expected
            or not stat.S_ISREG(target_metadata.st_mode)
            or target_metadata.st_uid != expected_uid
            or stat.S_IMODE(target_metadata.st_mode) != 0o644
        ):
            raise CapacityError("os-release link contract differs")
    except OSError as error:
        raise CapacityError("os-release link contract cannot be inspected") from error
    return read_regular(target)


def command_output(argv: Sequence[str], cwd: Path, limit: int = MAX_TEXT_BYTES) -> str:
    """Return one bounded probe result without inheriting stdin."""
    try:
        result = subprocess.run(
            list(argv),
            cwd=cwd,
            stdin=subprocess.DEVNULL,
            capture_output=True,
            timeout=15,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise CapacityError("native probe could not complete") from error
    if result.returncode != 0 or len(result.stdout) > limit or result.stderr:
        raise CapacityError("native probe failed")
    return safe_text(result.stdout)


def file_digests(path: Path, limit: int = MAX_FILE_BYTES) -> tuple[str, str]:
    """Hash a bounded regular file without following its final component."""
    data = read_regular(path, limit)
    return (
        "sha256:" + hashlib.sha256(data).hexdigest(),
        hashlib.md5(data, usedforsecurity=False).hexdigest(),
    )


def sha256_file(path: Path, limit: int = MAX_FILE_BYTES) -> str:
    """Return the SHA-256 member of the bounded file digest set."""
    return file_digests(path, limit)[0]


def source_identity(source: Path, base_commit: str) -> tuple[str, str]:
    """Require a clean immutable source descendant and return commit/tree."""
    if not COMMIT.fullmatch(base_commit):
        raise CapacityError("base commit is not immutable")
    commit = command_output(["git", "rev-parse", "HEAD"], source)
    tree = command_output(["git", "rev-parse", "HEAD^{tree}"], source)
    merge_base = command_output(["git", "merge-base", base_commit, commit], source)
    status = subprocess.run(
        ["git", "status", "--porcelain"],
        cwd=source,
        stdin=subprocess.DEVNULL,
        capture_output=True,
        timeout=15,
        check=False,
    )
    if (
        not COMMIT.fullmatch(commit)
        or not COMMIT.fullmatch(tree)
        or merge_base != base_commit
        or status.returncode != 0
        or status.stdout
        or status.stderr
    ):
        raise CapacityError(
            "source identity is dirty or not based on the reviewed commit"
        )
    return commit, tree


def package_evidence(
    executable: str, package: str, version: str, cwd: Path
) -> dict[str, str]:
    """Bind an executable to exact installed dpkg metadata and its binary hash."""
    owner = command_output(["/usr/bin/dpkg-query", "-S", executable], cwd)
    if owner != f"{package}: {executable}":
        raise CapacityError("tool package ownership differs")
    fields = command_output(
        [
            "/usr/bin/dpkg-query",
            "-W",
            "-f=${binary:Package}\\t${Version}\\t${Architecture}\\t${Status}",
            package,
        ],
        cwd,
    ).split("\t")
    if fields != [package, version, "amd64", "install ok installed"]:
        raise CapacityError("tool package metadata differs")
    manifest = Path(f"/var/lib/dpkg/info/{package}.md5sums")
    manifest_data = read_regular(manifest)
    relative = executable.removeprefix("/")
    matches = [
        line.split("  ", 1)[0]
        for line in safe_text(manifest_data).splitlines()
        if line.endswith("  " + relative)
    ]
    binary_sha256, binary_md5 = file_digests(Path(executable))
    if len(matches) != 1 or matches[0] != binary_md5:
        raise CapacityError("tool binary differs from its package manifest")
    return {
        "binary_sha256": binary_sha256,
        "executable": executable,
        "license_document_sha256": sha256_file(
            Path(f"/usr/share/doc/{package}/copyright")
        ),
        "package": package,
        "package_architecture": "amd64",
        "package_integrity": "verified",
        "package_manifest_sha256": "sha256:"
        + hashlib.sha256(manifest_data).hexdigest(),
        "package_source": UBUNTU_PACKAGE_SOURCE,
        "package_version": version,
    }


def host_evidence(source: Path) -> dict[str, Any]:
    """Collect exact native identity without hostname, address, or account data."""
    if platform.machine() != "x86_64":
        raise CapacityError("host is not native x86_64")
    release = parse_os_release(read_os_release())
    kernel = platform.release()
    if kernel != EXPECTED_KERNEL:
        raise CapacityError("booted kernel differs from the reviewed release")
    container = subprocess.run(
        ["/usr/bin/systemd-detect-virt", "--container"],
        cwd=source,
        stdin=subprocess.DEVNULL,
        capture_output=True,
        timeout=15,
        check=False,
    )
    if container.returncode not in {0, 1} or container.stderr:
        raise CapacityError("container boundary could not be determined")
    container_kind = safe_text(container.stdout) if container.stdout.strip() else "none"
    if container_kind != "none":
        raise CapacityError("container execution is not native host evidence")
    vm = subprocess.run(
        ["/usr/bin/systemd-detect-virt", "--vm"],
        cwd=source,
        stdin=subprocess.DEVNULL,
        capture_output=True,
        timeout=15,
        check=False,
    )
    if vm.returncode not in {0, 1} or vm.stderr:
        raise CapacityError("virtualization boundary could not be determined")
    vm_kind = safe_text(vm.stdout) if vm.stdout.strip() else "none"
    if vm_kind != "none":
        raise CapacityError(
            "this qualification is bound to the reviewed bare-metal host class"
        )
    cgroup = safe_text(read_regular(Path("/proc/self/cgroup")))
    if not any(line.startswith("0::") for line in cgroup.splitlines()):
        raise CapacityError("cgroup v2 unified membership is unavailable")
    for name in ("cpu", "memory", "io"):
        if "some " not in safe_text(read_regular(Path(f"/proc/pressure/{name}"))):
            raise CapacityError("required PSI evidence is unavailable")
    tools = [
        package_evidence(executable, package, version, source)
        for executable, (package, version) in EXPECTED_PACKAGES.items()
    ]
    kernel_fields = command_output(
        [
            "/usr/bin/dpkg-query",
            "-W",
            "-f=${binary:Package}\\t${Version}\\t${Architecture}\\t${Status}",
            EXPECTED_KERNEL_PACKAGE,
        ],
        source,
    ).split("\t")
    expected_kernel_fields = [
        EXPECTED_KERNEL_PACKAGE,
        EXPECTED_KERNEL_PACKAGE_VERSION,
        "amd64",
        "install ok installed",
    ]
    if kernel_fields != expected_kernel_fields:
        raise CapacityError("booted kernel package metadata differs")
    lsm = safe_text(read_regular(Path("/sys/kernel/security/lsm"))).split(",")
    apparmor = safe_text(read_regular(Path("/sys/module/apparmor/parameters/enabled")))
    if apparmor != "Y" or "apparmor" not in lsm:
        raise CapacityError("AppArmor registration is unavailable")
    if Path("/sys/fs/selinux/enforce").exists():
        raise CapacityError("SELinux state differs from the reviewed host")
    kernel_manifest = read_regular(
        Path(f"/var/lib/dpkg/info/{EXPECTED_KERNEL_PACKAGE}.md5sums")
    )
    kernel_image = f"boot/vmlinuz-{kernel}"
    image_entries = [
        line.split("  ", 1)[0]
        for line in safe_text(kernel_manifest).splitlines()
        if line.endswith("  " + kernel_image)
    ]
    if len(image_entries) != 1 or not re.fullmatch(r"[0-9a-f]{32}", image_entries[0]):
        raise CapacityError("booted kernel image is absent from its package manifest")
    return {
        "architecture": "x86_64",
        "distribution": {key.lower(): release[key] for key in EXPECTED_RELEASE},
        "kernel": {
            "package": EXPECTED_KERNEL_PACKAGE,
            "package_version": EXPECTED_KERNEL_PACKAGE_VERSION,
            "image_package_md5": image_entries[0],
            "license_document_sha256": sha256_file(
                Path(f"/usr/share/doc/{EXPECTED_KERNEL_PACKAGE}/copyright")
            ),
            "package_manifest_sha256": "sha256:"
            + hashlib.sha256(kernel_manifest).hexdigest(),
            "package_source": UBUNTU_PACKAGE_SOURCE,
            "release": kernel,
            "running_notes_sha256": sha256_file(
                Path("/sys/kernel/notes"), MAX_TEXT_BYTES
            ),
            "running_version_sha256": sha256_file(
                Path("/proc/version"), MAX_TEXT_BYTES
            ),
        },
        "virtualization": "none",
        "cgroup_v2": "available",
        "psi": {"cpu": "available", "io": "available", "memory": "available"},
        "apparmor": "registered-unproven",
        "selinux": "unavailable",
        "tools": tools,
    }


def system_resources(storage_root: Path) -> tuple[int, int, int]:
    """Return online CPU, memory MiB, and free storage MiB."""
    online = os.cpu_count() or 0
    memory_kib = 0
    for line in safe_text(read_regular(Path("/proc/meminfo"))).splitlines():
        if line.startswith("MemTotal:"):
            fields = line.split()
            if len(fields) == 3 and fields[2] == "kB" and fields[1].isdigit():
                memory_kib = int(fields[1])
            break
    usage = shutil.disk_usage(storage_root)
    if online <= 0 or memory_kib <= 0:
        raise CapacityError("host resource inventory is unavailable")
    return online, memory_kib // 1024, usage.free // (1 << 20)


def validate_root(root: Path, trusted: Path) -> None:
    """Require an existing private non-symlink root below the trusted storage tree."""
    root = root.absolute()
    trusted = trusted.absolute()
    try:
        root.relative_to(trusted)
    except ValueError as error:
        raise CapacityError("cell root escapes trusted storage") from error
    current = Path("/")
    for component in root.parts[1:]:
        current /= component
        metadata = current.lstat()
        if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISDIR(metadata.st_mode):
            raise CapacityError("cell root ancestor is not a directory")
    metadata = root.stat()
    if metadata.st_uid != os.getuid() or stat.S_IMODE(metadata.st_mode) != 0o700:
        raise CapacityError(
            "cell root must be private and owned by the invoking identity"
        )
    if root.stat().st_dev != trusted.stat().st_dev:
        raise CapacityError("cell root must use the trusted storage filesystem")


def validate_output_root(root: Path) -> None:
    """Require an owned, non-writable-by-others, symlink-free public output directory."""
    root = root.absolute()
    current = Path("/")
    for component in root.parts[1:]:
        current /= component
        metadata = current.lstat()
        if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISDIR(metadata.st_mode):
            raise CapacityError("output root ancestor is not a directory")
    metadata = root.stat()
    if metadata.st_uid != os.getuid() or stat.S_IMODE(metadata.st_mode) & 0o022:
        raise CapacityError("output root ownership or permissions are unsafe")


def scoped_argv(
    run_id: str,
    source: Path,
    cell: Path,
    cargo: Path,
    cargo_home: Path,
    rustup_home: Path,
    limits: Limits,
    check: str,
) -> list[str]:
    """Build the shell-free transient service invocation for one fixed check."""
    if check not in CHECKS:
        raise CapacityError("unknown qualification check")
    unit_suffix = hashlib.sha256(f"{run_id}-{check}".encode()).hexdigest()[:16]
    return [
        "/usr/bin/systemd-run",
        "--user",
        "--quiet",
        "--collect",
        "--wait",
        "--pipe",
        f"--unit=asb-native-x86-{unit_suffix}",
        "--property=PrivateNetwork=yes",
        "--property=PrivateTmp=yes",
        "--property=NoNewPrivileges=yes",
        "--property=ProtectSystem=strict",
        "--property=ProtectHome=yes",
        "--property=KillMode=control-group",
        "--property=TimeoutStopSec=10s",
        "--property=RestrictAddressFamilies=AF_UNIX",
        "--property=SystemCallArchitectures=native",
        f"--property=CPUQuota={limits.cpu_quota_percent}%",
        f"--property=MemoryMax={limits.memory_mib * (1 << 20)}",
        "--property=MemorySwapMax=0",
        f"--property=TasksMax={limits.tasks_max}",
        f"--property=RuntimeMaxSec={limits.timeout_seconds}s",
        f"--property=ReadOnlyPaths={source}",
        f"--property=ReadOnlyPaths={cargo_home}",
        f"--property=ReadOnlyPaths={rustup_home}",
        f"--property=ReadWritePaths={cell}",
        f"--property=TemporaryFileSystem={cell / 'target'}:rw,size={limits.storage_mib}M,mode=0700",
        f"--working-directory={source}",
        "/usr/bin/env",
        "-i",
        f"HOME={cell / 'home'}",
        f"TMPDIR={cell / 'tmp'}",
        f"CARGO_HOME={cargo_home}",
        f"RUSTUP_HOME={rustup_home}",
        f"CARGO_TARGET_DIR={cell / 'target'}",
        f"PATH={cargo.parent}:{rustup_home / 'toolchains/1.93.0-x86_64-unknown-linux-gnu/bin'}:/usr/bin:/bin",
        str(cargo),
        "test",
        "--locked",
        "--offline",
        *CHECKS[check],
    ]


def run_bounded(argv: Sequence[str], cwd: Path, timeout: int) -> dict[str, Any]:
    """Run one isolated check and retain only bounded output digests."""
    try:
        process = subprocess.Popen(
            list(argv),
            cwd=cwd,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            close_fds=True,
            start_new_session=True,
        )
    except OSError as error:
        raise CapacityError("qualification check could not start") from error
    assert process.stdout is not None
    selector = selectors.DefaultSelector()
    selector.register(process.stdout, selectors.EVENT_READ)
    digest = hashlib.sha256()
    size = 0
    deadline = time.monotonic() + timeout
    failure = ""
    try:
        while selector.get_map():
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                failure = "qualification check timed out"
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
                    failure = "qualification output exceeded byte limit"
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
        raise CapacityError("qualification check could not be reaped") from error
    finally:
        selector.close()
        process.stdout.close()
    if failure:
        raise CapacityError(failure)
    if returncode != 0:
        raise CapacityError("qualification check failed")
    return {
        "argv_sha256": "sha256:"
        + hashlib.sha256(
            json.dumps(list(argv), separators=(",", ":")).encode()
        ).hexdigest(),
        "output_bytes": size,
        "output_sha256": "sha256:" + digest.hexdigest(),
        "status": "passed",
    }


def unit_is_collected(argv: Sequence[str], cwd: Path) -> bool:
    """Require the exact transient service to disappear after its bounded check."""
    units = [
        item.removeprefix("--unit=") for item in argv if item.startswith("--unit=")
    ]
    if len(units) != 1 or not re.fullmatch(r"asb-native-x86-[0-9a-f]{16}", units[0]):
        return False
    unit = units[0] + ".service"
    for _ in range(20):
        try:
            result = subprocess.run(
                [
                    "/usr/bin/systemctl",
                    "--user",
                    "list-units",
                    "--all",
                    "--plain",
                    "--no-legend",
                    unit,
                ],
                cwd=cwd,
                stdin=subprocess.DEVNULL,
                capture_output=True,
                timeout=5,
                check=False,
            )
        except (OSError, subprocess.TimeoutExpired):
            return False
        if result.returncode != 0 or result.stderr:
            return False
        if not result.stdout:
            return True
        time.sleep(0.05)
    return False


def write_atomic(path: Path, report: dict[str, Any], output_root: Path) -> None:
    """Commit canonical evidence beneath an existing trusted output root."""
    path = path.absolute()
    output_root = output_root.absolute()
    try:
        relative = path.relative_to(output_root)
    except ValueError as error:
        raise CapacityError("output path escapes its trusted root") from error
    if (
        len(relative.parts) != 1
        or not RUN_ID.fullmatch(path.stem)
        or report.get("run_id") != path.stem
    ):
        raise CapacityError("output filename is not a safe run ID")
    validate_output_root(output_root)
    data = (json.dumps(report, indent=2, sort_keys=True) + "\n").encode()
    directory = os.open(output_root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    temporary = f".{path.name}.tmp-{os.getpid()}"
    try:
        descriptor = os.open(
            temporary,
            os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
            0o600,
            dir_fd=directory,
        )
        with os.fdopen(descriptor, "wb") as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        os.link(temporary, path.name, src_dir_fd=directory, dst_dir_fd=directory)
        os.unlink(temporary, dir_fd=directory)
        os.fsync(directory)
    except OSError as error:
        raise CapacityError("evidence could not be committed atomically") from error
    finally:
        try:
            os.unlink(temporary, dir_fd=directory)
        except FileNotFoundError:
            pass
        os.close(directory)


def qualify(
    run_id: str,
    source: Path,
    base_commit: str,
    storage_root: Path,
    cargo: Path,
    cargo_home: Path,
    rustup_home: Path,
    limits: Limits,
    runner: Callable[[Sequence[str], Path, int], dict[str, Any]] = run_bounded,
    cleanup_probe: Callable[[Sequence[str], Path], bool] = unit_is_collected,
) -> dict[str, Any]:
    """Acquire, exercise, and remove one fail-closed native x86_64 cell."""
    if not RUN_ID.fullmatch(run_id):
        raise CapacityError("run ID is empty, unsafe, or oversized")
    validate_root(storage_root, Path("/srv/data/projects"))
    online, memory, free_storage = system_resources(storage_root)
    limits.validate(online, memory, free_storage)
    if not cargo.is_file() or cargo.is_symlink():
        raise CapacityError("cargo executable is unavailable or mutable by link")
    commit, tree = source_identity(source, base_commit)
    lock_path = storage_root / ".native-x86-capacity.lock"
    lock_descriptor = os.open(lock_path, os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600)
    lock_metadata = os.fstat(lock_descriptor)
    if (
        not stat.S_ISREG(lock_metadata.st_mode)
        or lock_metadata.st_uid != os.getuid()
        or stat.S_IMODE(lock_metadata.st_mode) != 0o600
    ):
        os.close(lock_descriptor)
        raise CapacityError("capacity lock ownership or permissions are unsafe")
    cell = storage_root / f"cell-{run_id}"
    try:
        fcntl.flock(lock_descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError as error:
        os.close(lock_descriptor)
        raise CapacityError("native x86 capacity is already leased") from error
    checks: dict[str, dict[str, Any]] = {}
    try:
        if cell.exists() or cell.is_symlink():
            raise CapacityError("cell root already exists")
        cell.mkdir(mode=0o700)
        for name in ("home", "tmp", "target"):
            (cell / name).mkdir(mode=0o700)
        lease = {
            "cpu_quota_percent": limits.cpu_quota_percent,
            "memory_mib": limits.memory_mib,
            "run_id": run_id,
            "storage_mib": limits.storage_mib,
            "tasks_max": limits.tasks_max,
            "timeout_seconds": limits.timeout_seconds,
        }
        lease_path = cell / "lease.json"
        descriptor = os.open(lease_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(descriptor, "wb") as stream:
            stream.write((json.dumps(lease, sort_keys=True) + "\n").encode())
            stream.flush()
            os.fsync(stream.fileno())
        host = host_evidence(source)
        for name in sorted(CHECKS):
            argv = scoped_argv(
                run_id, source, cell, cargo, cargo_home, rustup_home, limits, name
            )
            checks[name] = runner(argv, source, limits.timeout_seconds + 30)
            if not cleanup_probe(argv, source):
                raise CapacityError("transient service was not collected")
        final_commit, final_tree = source_identity(source, base_commit)
        if (commit, tree) != (final_commit, final_tree):
            raise CapacityError("source identity changed while qualification ran")
    finally:
        if cell.exists() and not cell.is_symlink():
            if not shutil.rmtree.avoids_symlink_attacks:
                raise CapacityError("safe recursive cleanup is unavailable")
            shutil.rmtree(cell)
        fcntl.flock(lock_descriptor, fcntl.LOCK_UN)
        os.close(lock_descriptor)
    if cell.exists() or cell.is_symlink():
        raise CapacityError("cell cleanup left storage residue")
    return {
        "format_version": 1,
        "kind": "native-x86-capacity",
        "qualification": "native-functional",
        "performance_baseline": False,
        "run_id": run_id,
        "source": {"base_commit": base_commit, "commit": commit, "tree": tree},
        "host": host,
        "limits": {
            "cpu_quota_percent": limits.cpu_quota_percent,
            "memory_mib": limits.memory_mib,
            "storage_mib": limits.storage_mib,
            "tasks_max": limits.tasks_max,
            "timeout_seconds": limits.timeout_seconds,
        },
        "isolation": {
            "asb_exclusive_lease": "passed",
            "credential_injection": "none",
            "network": "private-loopback-only",
            "persistent_runner_dispatch": "not-used",
            "transient_user_service": "passed",
        },
        "checks": checks,
        "cleanup": {"cell_root": "removed", "transient_units": "collected"},
        "cost": {
            "external_provider_spend": "none-authorized-or-incurred",
            "existing_host_cost": "unmeasured",
        },
        "availability": "single-local-cell-no-sla",
        "limitations": [
            "native-aarch64-unavailable",
            "not-a-performance-baseline",
            "not-for-public-pull-request-code",
            "apparmor-enforcement-unproven",
        ],
    }


def main() -> int:
    """CLI entry point."""
    parser = argparse.ArgumentParser()
    parser.add_argument("--run-id", required=True)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--base-commit", required=True)
    parser.add_argument("--storage-root", type=Path, required=True)
    parser.add_argument("--cargo", type=Path, required=True)
    parser.add_argument("--cargo-home", type=Path, required=True)
    parser.add_argument("--rustup-home", type=Path, required=True)
    parser.add_argument("--output-root", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--cpu-quota-percent", type=int, default=400)
    parser.add_argument("--memory-mib", type=int, default=8192)
    parser.add_argument("--storage-mib", type=int, default=16384)
    parser.add_argument("--timeout-seconds", type=int, default=1200)
    parser.add_argument("--tasks-max", type=int, default=256)
    args = parser.parse_args()
    limits = Limits(
        args.cpu_quota_percent,
        args.memory_mib,
        args.storage_mib,
        args.timeout_seconds,
        args.tasks_max,
    )
    try:
        report = qualify(
            args.run_id,
            args.source.resolve(strict=True),
            args.base_commit,
            args.storage_root.absolute(),
            args.cargo.absolute(),
            args.cargo_home.absolute(),
            args.rustup_home.absolute(),
            limits,
        )
        write_atomic(args.output, report, args.output_root.absolute())
    except (CapacityError, OSError) as error:
        print(f"ERROR: {error}")
        return 1
    print("native x86_64 capacity qualification passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
