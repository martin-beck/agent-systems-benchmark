#!/usr/bin/env python3
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Validate pinned platform and agent availability manifests."""

from __future__ import annotations

import argparse
import base64
import binascii
import hashlib
import json
import re
from collections import Counter
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
DEFAULT_PLATFORMS = ROOT / "platforms/v1/platforms.json"
DEFAULT_AGENTS = ROOT / "platforms/v1/agents.json"
SHA256 = re.compile(r"^sha256:[0-9a-f]{64}$")
COMMIT = re.compile(r"^[0-9a-f]{40}$")
EVIDENCE_PATH = re.compile(r"^platforms/v1/native-evidence/[A-Za-z0-9._-]+\.json$")
PINNED_IMAGE = re.compile(r"^docker\.io/[a-z0-9_./-]+@sha256:[0-9a-f]{64}$")
TAGGED_IMAGE = re.compile(r"^docker\.io/[a-z0-9_./-]+:[A-Za-z0-9_.-]+$")
ARCHES = {"x86_64": "amd64", "aarch64": "arm64"}
STATUSES = {"planned", "build-only", "simulated", "native-tested", "unsupported"}
AGENTS = {"opencode", "opendesk", "aider", "codex"}
FAMILIES = {
    "ubuntu",
    "debian",
    "fedora",
    "rocky",
    "alma",
    "opensuse-leap",
    "opensuse-tumbleweed",
    "arch",
    "alpine",
    "openeuler",
}
HOST_CAPABILITIES = {
    "cgroup-v2-delegation",
    "systemd",
    "SELinux",
    "AppArmor",
    "perf-permissions",
    "PSI",
    "BTF",
}
NATIVE_DISTRIBUTIONS = {
    "ubuntu-24.04": ("ubuntu", "24.04", "24.04.4 LTS", "contains"),
    "debian-13": ("debian", "13", "13.6", "exact"),
    "openeuler-24.03-lts-sp2": ("openEuler", "24.03", "LTS-SP2", "contains"),
}
NATIVE_SANDBOX_TOOLS = {
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


def load(path: Path) -> dict[str, Any]:
    """Load one JSON document."""
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"{path}: top level must be an object")
    return value


def valid_integrity(value: object) -> bool:
    """Accept an exact SHA-256 or canonical 64-byte npm SHA-512 SRI."""
    if not isinstance(value, str):
        return False
    if SHA256.fullmatch(value):
        return True
    if not value.startswith("sha512-"):
        return False
    encoded = value.removeprefix("sha512-")
    try:
        decoded = base64.b64decode(encoded, validate=True)
    except (binascii.Error, ValueError):
        return False
    return len(decoded) == 64 and base64.b64encode(decoded).decode("ascii") == encoded


def validate_native_report(
    evidence: dict[str, Any], row: dict[str, Any], ident: str, arch: str, root: Path
) -> list[str]:
    """Validate a native claim against one immutable sanitized report."""
    errors: list[str] = []
    relative = evidence.get("artifact_path")
    if not isinstance(relative, str) or not EVIDENCE_PATH.fullmatch(relative):
        return [f"{ident}/{arch}: native evidence path is unsafe or missing"]
    path = root / relative
    try:
        if path.is_symlink() or not path.is_file() or path.stat().st_size > 1 << 20:
            raise ValueError
        data = path.read_bytes()
        report = json.loads(data)
        if not isinstance(report, dict):
            raise ValueError
    except (OSError, ValueError, json.JSONDecodeError):
        return [f"{ident}/{arch}: native evidence artifact is unavailable or invalid"]
    digest = "sha256:" + hashlib.sha256(data).hexdigest()
    if digest != evidence.get("artifact_digest"):
        errors.append(f"{ident}/{arch}: native evidence artifact digest differs")
    expected = {
        "format_version": 1,
        "kind": "native-run",
        "qualification": "native-functional",
        "performance_baseline": False,
        "platform_id": ident,
        "architecture": arch,
        "kernel_release": evidence.get("kernel_release"),
        "run_id": evidence.get("run_id"),
    }
    for key, value in expected.items():
        if report.get(key) != value:
            errors.append(f"{ident}/{arch}: native report {key} binding differs")
    if not COMMIT.fullmatch(report.get("source_commit", "")):
        errors.append(f"{ident}/{arch}: native report source commit is not immutable")
    distribution = report.get("distribution", {})
    expected_id, expected_version, release_value, release_mode = NATIVE_DISTRIBUTIONS.get(
        ident, ("", "", "", "exact")
    )
    if not isinstance(distribution, dict):
        distribution = {}
    if (
        distribution.get("id") != expected_id
        or distribution.get("version_id") != expected_version
        or distribution.get("manifest_release") != row.get("release")
    ):
        errors.append(f"{ident}/{arch}: native report distribution binding differs")
    release_evidence = distribution.get("release_evidence")
    if not isinstance(release_evidence, str) or not (
        release_evidence == release_value
        if release_mode == "exact"
        else release_value in release_evidence
    ):
        errors.append(f"{ident}/{arch}: native report exact release evidence differs")
    capabilities = report.get("capabilities", {})
    if not isinstance(capabilities, dict):
        capabilities = {}
    if capabilities.get("cgroup_v2") != "available" or capabilities.get("psi") != "available":
        errors.append(f"{ident}/{arch}: cgroup v2 and PSI evidence are required")
    if capabilities.get("sandbox") != "passed":
        errors.append(f"{ident}/{arch}: sandbox capability did not pass")
    security = capabilities.get("security_modules", {})
    if (
        not isinstance(security, dict)
        or security.get("apparmor") not in {"enabled", "disabled"}
        or security.get("selinux") not in {"enforcing", "permissive", "disabled"}
    ):
        errors.append(f"{ident}/{arch}: AppArmor and SELinux state must be available")
    expected_tools = [
        {"executable": executable, "version": version, "package": package, "source": source}
        for executable, version, package, source in NATIVE_SANDBOX_TOOLS.get(ident, ())
    ]
    if report.get("sandbox_tools") != expected_tools:
        errors.append(f"{ident}/{arch}: sandbox tool pins or provenance differ")
    checks = report.get("checks", {})
    if not isinstance(checks, dict):
        checks = {}
    for name in ("process", "metrics", "sandbox"):
        result = checks.get(name, {})
        if not isinstance(result, dict):
            result = {}
        if result.get("status") != "passed":
            errors.append(f"{ident}/{arch}: required native check {name} did not pass")
        for field in ("argv_sha256", "output_sha256"):
            if not SHA256.fullmatch(result.get(field, "")):
                errors.append(f"{ident}/{arch}: native check {name} lacks {field}")
    return errors


def validate(
    platforms: dict[str, Any], agents: dict[str, Any], root: Path = ROOT
) -> list[str]:
    """Return all semantic validation failures."""
    errors: list[str] = []
    if platforms.get("format_version") != 1 or agents.get("format_version") != 1:
        errors.append("format_version must equal 1")
    if set(platforms.get("evidence_statuses", [])) != STATUSES:
        errors.append("evidence_statuses must contain the exact supported vocabulary")
    policy = platforms.get("combination_policy", {})
    if policy.get("default_evidence_status") != "planned":
        errors.append("unimplemented combinations must default to planned")
    if policy.get("workload_set_status") != "planned":
        errors.append("workload set must remain planned until workloads exist")

    agent_rows = agents.get("agents", [])
    agent_ids = [row.get("id") for row in agent_rows]
    if set(agent_ids) != AGENTS or len(agent_ids) != len(set(agent_ids)):
        errors.append("agent IDs must be unique and exactly match the planned adapters")
    for row in agent_rows:
        ident = row.get("id", "<unknown>")
        package = row.get("package", {})
        if not package.get("version") or not valid_integrity(package.get("integrity")):
            errors.append(f"{ident}: package version and immutable integrity are required")
        if not str(package.get("source", "")).startswith("https://"):
            errors.append(f"{ident}: package source must use HTTPS")
        variants = row.get("variants", [])
        if not ARCHES.keys() <= {variant.get("architecture") for variant in variants}:
            errors.append(f"{ident}: both architecture availabilities must be tracked")
        for variant in variants:
            if variant.get("architecture") not in ARCHES:
                errors.append(f"{ident}: unknown architecture variant")
            if variant.get("availability") not in {"package-inspected", "metadata-only", "unsupported"}:
                errors.append(f"{ident}: invalid package availability label")
            if not valid_integrity(variant.get("integrity")):
                errors.append(f"{ident}: variant lacks immutable integrity")

    rows = platforms.get("platforms", [])
    ids = [row.get("id") for row in rows]
    family_counts = Counter(row.get("family") for row in rows)
    if len(ids) != len(set(ids)):
        errors.append("platform IDs must be unique")
    if family_counts != Counter({family: 1 for family in FAMILIES}):
        errors.append("platform manifest must contain exactly one row per required family")
    if set(platforms.get("unverified_host_capabilities", [])) != HOST_CAPABILITIES:
        errors.append("host capability unknowns must remain explicit")

    for row in rows:
        ident = row.get("id", "<unknown>")
        image = row.get("image", {})
        if not PINNED_IMAGE.fullmatch(image.get("index", "")):
            errors.append(f"{ident}: image index must be digest-pinned")
        tag = image.get("tag", "")
        index = image.get("index", "")
        if not TAGGED_IMAGE.fullmatch(tag):
            errors.append(f"{ident}: discovery tag must name the registry and tag")
        if isinstance(tag, str) and isinstance(index, str):
            if tag.rsplit(":", 1)[0] != index.split("@", 1)[0]:
                errors.append(f"{ident}: discovery tag and index repositories differ")
        if not str(row.get("source", "")).startswith("https://"):
            errors.append(f"{ident}: platform source must use HTTPS")
        libc = row.get("libc", {})
        if libc.get("family") not in {"glibc", "musl"} or not re.fullmatch(
            r"[0-9]+\.[0-9]+(?:\.[0-9]+)?", libc.get("version", "")
        ):
            errors.append(f"{ident}: exact glibc or musl baseline is required")
        architectures = row.get("architectures", {})
        if set(architectures) != set(ARCHES):
            errors.append(f"{ident}: both canonical architectures must be present")
            continue
        for arch, oci in ARCHES.items():
            cell = architectures[arch]
            if cell.get("oci") != oci:
                errors.append(f"{ident}/{arch}: OCI architecture alias mismatch")
            user_status = cell.get("user_space")
            native_status = cell.get("native_kernel")
            if user_status not in STATUSES or native_status not in STATUSES:
                errors.append(f"{ident}/{arch}: unknown evidence status")
            digest = cell.get("image_digest")
            if digest is None:
                if user_status != "unsupported" or native_status != "unsupported":
                    errors.append(f"{ident}/{arch}: missing image must remain unsupported")
            elif not SHA256.fullmatch(digest):
                errors.append(f"{ident}/{arch}: child image digest is not pinned")
            if native_status in {"build-only", "simulated"}:
                errors.append(f"{ident}/{arch}: native kernel status cannot be cross-build or simulated")
            if "native-tested" in {user_status, native_status}:
                evidence = cell.get("native_evidence", {})
                bound = (
                    evidence.get("kind") == "native-run"
                    and evidence.get("platform_id") == ident
                    and evidence.get("architecture") == arch
                    and isinstance(evidence.get("kernel_release"), str)
                    and bool(evidence.get("kernel_release"))
                    and isinstance(evidence.get("run_id"), str)
                    and bool(evidence.get("run_id"))
                    and SHA256.fullmatch(evidence.get("artifact_digest", ""))
                )
                if not bound:
                    errors.append(f"{ident}/{arch}: native-tested requires bound native-run evidence")
                else:
                    errors.extend(validate_native_report(evidence, row, ident, arch, root))
    return errors


def main() -> int:
    """Validate files selected on the command line."""
    parser = argparse.ArgumentParser()
    parser.add_argument("--platforms", type=Path, default=DEFAULT_PLATFORMS)
    parser.add_argument("--agents", type=Path, default=DEFAULT_AGENTS)
    args = parser.parse_args()
    errors = validate(load(args.platforms), load(args.agents))
    for error in errors:
        print(f"ERROR: {error}")
    if errors:
        return 1
    print(f"validated {args.platforms} and {args.agents}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
