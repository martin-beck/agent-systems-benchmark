#!/usr/bin/env python3
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Validate and probe the bounded emulated-aarch64 portability lane."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import re
import subprocess
import tempfile
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
DEFAULT_CONFIG = ROOT / "platforms/v1/emulated-aarch64.json"
SHA256 = re.compile(r"^[0-9a-f]{64}$")
SAFE_RELEASE = re.compile(r"^[A-Za-z0-9._+~-]{1,128}$")
IMAGE = re.compile(r"^docker\.io/[a-z0-9_./-]+@sha256:[0-9a-f]{64}$")
SCOPES = {
    "userspace-smoke",
    "protocol",
    "adapter-logic",
    "packaging",
    "replay",
    "failure-paths",
}
FALSE_CLAIMS = {
    "native_hardware",
    "native_kernel",
    "architecture_performance",
    "timing",
    "contention",
    "distribution_boot",
}


def load(path: Path) -> dict[str, Any]:
    """Load a bounded JSON object."""
    if path.stat().st_size > 16_384:
        raise ValueError("configuration exceeds 16384 bytes")
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise TypeError("configuration must be an object")
    return value


def exact_keys(value: object, keys: set[str], label: str, errors: list[str]) -> None:
    """Require a closed object."""
    if not isinstance(value, dict) or set(value) != keys:
        errors.append(f"{label} must contain the exact registered fields")


def validate_config(data: dict[str, Any]) -> list[str]:
    """Return every fail-closed configuration error."""
    errors: list[str] = []
    exact_keys(
        data,
        {
            "format_version",
            "evidence_kind",
            "host_architecture",
            "target_architecture",
            "kernel_scope",
            "emulator",
            "guest_image",
            "toolchain",
            "limits",
            "claims",
            "test_scope",
        },
        "configuration",
        errors,
    )
    if data.get("format_version") != 1:
        errors.append("format_version must equal 1")
    if data.get("evidence_kind") != "emulated-aarch64":
        errors.append("evidence kind must be emulated-aarch64")
    if (
        data.get("host_architecture") != "x86_64"
        or data.get("target_architecture") != "aarch64"
    ):
        errors.append("lane must be x86_64-hosted aarch64")
    if data.get("kernel_scope") != "shared-host-kernel-not-guest":
        errors.append("kernel scope must deny guest-kernel evidence")

    emulator = data.get("emulator")
    exact_keys(
        emulator,
        {
            "kind",
            "version",
            "package",
            "package_sha256",
            "binfmt_package",
            "binfmt_package_sha256",
            "executable",
            "sha256",
        },
        "emulator",
        errors,
    )
    if isinstance(emulator, dict):
        if emulator.get("kind") != "qemu-user" or emulator.get("version") != "8.2.2":
            errors.append("emulator identity is not the reviewed QEMU release")
        if emulator.get("package") != "qemu-user=1:8.2.2+ds-0ubuntu1.18":
            errors.append("emulator package must be exact")
        if emulator.get("binfmt_package") != "qemu-user-binfmt=1:8.2.2+ds-0ubuntu1.18":
            errors.append("binfmt package must be exact")
        if not SHA256.fullmatch(
            str(emulator.get("package_sha256", ""))
        ) or not SHA256.fullmatch(str(emulator.get("binfmt_package_sha256", ""))):
            errors.append("emulator package SHA-256 pins are required")
        if emulator.get(
            "executable"
        ) != "/usr/bin/qemu-aarch64" or not SHA256.fullmatch(
            str(emulator.get("sha256", ""))
        ):
            errors.append("emulator executable and SHA-256 pin are required")

    image = data.get("guest_image")
    exact_keys(
        image,
        {"platform_id", "platform", "reference", "release"},
        "guest_image",
        errors,
    )
    if isinstance(image, dict):
        if (
            image.get("platform_id") != "ubuntu-24.04"
            or image.get("platform") != "linux/arm64"
        ):
            errors.append("guest image identity must be Ubuntu aarch64")
        if not IMAGE.fullmatch(str(image.get("reference", ""))):
            errors.append("guest image must use an immutable sha256 reference")
        if image.get("release") != "24.04.4 LTS":
            errors.append("guest release must match the platform manifest exactly")

    toolchain = data.get("toolchain")
    exact_keys(
        toolchain,
        {
            "rust",
            "rust_commit",
            "target",
            "linker_package",
            "linker",
            "linker_version",
            "linker_sha256",
            "sysroot_package",
        },
        "toolchain",
        errors,
    )
    if isinstance(toolchain, dict):
        expected = {
            "rust": "1.93.0",
            "rust_commit": "254b59607d4417e9dffbc307138ae5c86280fe4c",
            "target": "aarch64-unknown-linux-gnu",
            "linker_package": "gcc-aarch64-linux-gnu=4:13.2.0-7ubuntu1",
            "linker": "/usr/bin/aarch64-linux-gnu-gcc",
            "linker_version": "13.3.0",
            "sysroot_package": "libc6-dev-arm64-cross=2.39-0ubuntu8cross1",
        }
        if any(toolchain.get(key) != value for key, value in expected.items()):
            errors.append("toolchain identity differs from the reviewed pins")
        if not SHA256.fullmatch(str(toolchain.get("linker_sha256", ""))):
            errors.append("linker SHA-256 pin is required")

    limits = data.get("limits")
    exact_keys(
        limits,
        {"command_timeout_seconds", "job_timeout_minutes", "max_report_bytes"},
        "limits",
        errors,
    )
    if limits != {
        "command_timeout_seconds": 120,
        "job_timeout_minutes": 20,
        "max_report_bytes": 4096,
    }:
        errors.append("execution limits must equal the reviewed finite bounds")
    claims = data.get("claims")
    exact_keys(claims, FALSE_CLAIMS, "claims", errors)
    if not isinstance(claims, dict) or any(
        claims.get(key) is not False for key in FALSE_CLAIMS
    ):
        errors.append(
            "every native, kernel, timing and performance claim must be false"
        )
    scope = data.get("test_scope")
    if (
        not isinstance(scope, list)
        or len(scope) != len(set(scope))
        or set(scope) != SCOPES
    ):
        errors.append(
            "test scope must contain every registered portability area exactly once"
        )
    return errors


def digest(path: Path) -> str:
    """Hash a reviewed executable."""
    return hashlib.sha256(path.read_bytes()).hexdigest()


def first_line(command: list[str]) -> str:
    """Run a bounded version query."""
    result = subprocess.run(
        command, check=True, capture_output=True, text=True, timeout=10
    )
    return result.stdout.splitlines()[0]


def probe(
    data: dict[str, Any],
    sysroot: Path,
    userspace_reference: str,
    output: Path,
    emulator_path: Path | None = None,
) -> None:
    """Compile and execute a bounded aarch64 probe and write public evidence."""
    errors = validate_config(data)
    if errors:
        raise ValueError("; ".join(errors))
    if platform.machine() != "x86_64":
        raise RuntimeError("emulation lane requires an x86_64 host")
    emulator, toolchain = data["emulator"], data["toolchain"]
    qemu = emulator_path or Path(emulator["executable"])
    linker = Path(toolchain["linker"])
    if (
        digest(qemu) != emulator["sha256"]
        or digest(linker) != toolchain["linker_sha256"]
    ):
        raise RuntimeError("emulator or linker digest differs from the reviewed pin")
    if f"version {emulator['version']}" not in first_line([str(qemu), "--version"]):
        raise RuntimeError("emulator version differs from the reviewed pin")
    if toolchain["linker_version"] not in first_line([str(linker), "--version"]):
        raise RuntimeError("linker version differs from the reviewed pin")
    if not sysroot.is_dir() or not (sysroot / "lib/ld-linux-aarch64.so.1").is_file():
        raise RuntimeError("aarch64 sysroot lacks its dynamic loader")
    source = '#include <stdio.h>\n#include <sys/utsname.h>\nint main(void){struct utsname u;if(uname(&u))return 3;unsigned int x=1;printf("%s %zu %d\\n",u.machine,sizeof(void*)*8,*(unsigned char*)&x);return 0;}\n'
    base = Path(os.environ.get("ASB_EMULATION_TMP", "/srv/data/projects/.asb-local"))
    base.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="ar0707-", dir=base) as temporary:
        src, binary = Path(temporary) / "probe.c", Path(temporary) / "probe"
        src.write_text(source, encoding="utf-8")
        subprocess.run(
            [str(linker), "-O2", "-o", str(binary), str(src)], check=True, timeout=30
        )
        result = subprocess.run(
            [str(qemu), "-L", str(sysroot), str(binary)],
            check=True,
            capture_output=True,
            text=True,
            timeout=data["limits"]["command_timeout_seconds"],
        )
    if result.stdout.strip() != "aarch64 64 1":
        raise RuntimeError("emulated probe did not report aarch64/64-bit/little-endian")
    kernel = platform.release()
    if not SAFE_RELEASE.fullmatch(kernel):
        raise RuntimeError("host kernel release is empty or unsafe")
    allowed_userspaces = {
        data["guest_image"]["reference"],
        data["toolchain"]["sysroot_package"],
    }
    if userspace_reference not in allowed_userspaces:
        raise RuntimeError("userspace reference differs from every reviewed pin")
    report = {
        "format_version": 1,
        "evidence_kind": "emulated-aarch64",
        "host_architecture": "x86_64",
        "emulated_architecture": "aarch64",
        "kernel_release": kernel,
        "kernel_scope": "shared-host-kernel-not-guest",
        "userspace_reference": userspace_reference,
        "emulator_sha256": emulator["sha256"],
        "toolchain_target": toolchain["target"],
        "native_kernel": False,
        "architecture_performance": False,
    }
    payload = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if len(payload.encode()) > data["limits"]["max_report_bytes"]:
        raise RuntimeError("evidence report exceeds its configured bound")
    output.write_text(payload, encoding="utf-8")


def validate_report(data: dict[str, Any], report: dict[str, Any]) -> list[str]:
    """Ensure evidence cannot claim native support."""
    errors: list[str] = []
    exact_keys(
        report,
        {
            "format_version",
            "evidence_kind",
            "host_architecture",
            "emulated_architecture",
            "kernel_release",
            "kernel_scope",
            "userspace_reference",
            "emulator_sha256",
            "toolchain_target",
            "native_kernel",
            "architecture_performance",
        },
        "report",
        errors,
    )
    expected = {
        "format_version": 1,
        "evidence_kind": "emulated-aarch64",
        "host_architecture": "x86_64",
        "emulated_architecture": "aarch64",
        "kernel_scope": "shared-host-kernel-not-guest",
        "emulator_sha256": data.get("emulator", {}).get("sha256"),
        "toolchain_target": data.get("toolchain", {}).get("target"),
        "native_kernel": False,
        "architecture_performance": False,
    }
    if any(report.get(key) != value for key, value in expected.items()):
        errors.append("report is not exactly bound to the configured emulated lane")
    allowed_userspaces = {
        data.get("guest_image", {}).get("reference"),
        data.get("toolchain", {}).get("sysroot_package"),
    }
    if report.get("userspace_reference") not in allowed_userspaces:
        errors.append("report userspace is not bound to a reviewed pin")
    if not SAFE_RELEASE.fullmatch(str(report.get("kernel_release", ""))):
        errors.append("report lacks a bounded host-kernel release")
    return errors


def main() -> int:
    """CLI entry point."""
    parser = argparse.ArgumentParser()
    parser.add_argument("--config", type=Path, default=DEFAULT_CONFIG)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("validate-config")
    probe_parser = commands.add_parser("probe")
    probe_parser.add_argument("--sysroot", type=Path, required=True)
    probe_parser.add_argument("--userspace-reference", required=True)
    probe_parser.add_argument("--output", type=Path, required=True)
    probe_parser.add_argument("--emulator-path", type=Path)
    report_parser = commands.add_parser("validate-report")
    report_parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    data = load(args.config)
    errors = validate_config(data)
    if args.command == "probe" and not errors:
        probe(
            data,
            args.sysroot,
            args.userspace_reference,
            args.output,
            args.emulator_path,
        )
    elif args.command == "validate-report" and not errors:
        if args.report.stat().st_size > data["limits"]["max_report_bytes"]:
            errors.append("report exceeds configured byte bound")
        else:
            errors.extend(validate_report(data, load(args.report)))
    for error in errors:
        print(f"ERROR: {error}")
    if errors:
        return 1
    print(f"validated {args.command} for emulated-aarch64")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
