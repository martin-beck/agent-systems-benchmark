# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Positive and deliberate-failure tests for platform manifests."""

from __future__ import annotations

import base64
import copy
import hashlib
import importlib.util
import json
import subprocess
import tempfile
import unittest
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "validate_manifests", ROOT / "tools/platforms/validate_manifests.py"
)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("cannot load manifest validator")
VALIDATOR = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(VALIDATOR)


def apply_mutations(document: dict[str, Any], mutations: list[dict[str, Any]]) -> None:
    """Apply a fixture mutation to an in-memory document."""
    for mutation in mutations:
        target: Any = document
        for component in mutation["path"][:-1]:
            target = target[component]
        target[mutation["path"][-1]] = mutation["value"]


class ManifestTests(unittest.TestCase):
    """Exercise canonical and known-invalid platform documents."""

    @classmethod
    def setUpClass(cls) -> None:
        cls.platforms = VALIDATOR.load(ROOT / "platforms/v1/platforms.json")
        cls.agents = VALIDATOR.load(ROOT / "platforms/v1/agents.json")

    def test_canonical_manifests_pass(self) -> None:
        self.assertEqual(VALIDATOR.validate(self.platforms, self.agents), [])

    def test_native_schema_closes_every_report_field(self) -> None:
        schema = VALIDATOR.load(ROOT / "platforms/v1/native-evidence.schema.json")
        self.assertFalse(schema["additionalProperties"])
        self.assertEqual(set(schema["required"]), VALIDATOR.REPORT_KEYS)
        self.assertEqual(set(schema["properties"]), VALIDATOR.REPORT_KEYS)
        self.assertEqual(VALIDATOR.validate_native_schema(), [])

    def test_npm_and_pypi_integrity_formats_are_accepted(self) -> None:
        sri = "sha512-" + base64.b64encode(b"a" * 64).decode("ascii")
        self.assertTrue(VALIDATOR.valid_integrity(sri))
        self.assertTrue(VALIDATOR.valid_integrity("sha256:" + "a" * 64))

    def test_npm_integrity_rejects_bad_alphabet_padding_and_length(self) -> None:
        sri = "sha512-" + base64.b64encode(b"a" * 64).decode("ascii")
        self.assertFalse(VALIDATOR.valid_integrity("sha512-not*base64="))
        self.assertFalse(VALIDATOR.valid_integrity(sri.rstrip("=")))
        self.assertFalse(VALIDATOR.valid_integrity("sha512-YWJjZA=="))

    def test_failure_fixtures_are_rejected(self) -> None:
        fixtures = sorted((ROOT / "tests/platforms/fixtures").glob("*.json"))
        self.assertGreaterEqual(len(fixtures), 3)
        for path in fixtures:
            with self.subTest(path=path.name):
                fixture = json.loads(path.read_text(encoding="utf-8"))
                candidate = copy.deepcopy(self.platforms)
                apply_mutations(candidate, fixture["mutations"])
                errors = VALIDATOR.validate(candidate, self.agents)
                self.assertTrue(
                    any(fixture["expected_error"] in error for error in errors),
                    errors,
                )

    def test_native_claim_is_bound_to_complete_immutable_report(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            subprocess.run(["git", "init", "-q", str(root)], check=True)
            subprocess.run(["git", "-C", str(root), "config", "user.name", "ASB Test"], check=True)
            subprocess.run(["git", "-C", str(root), "config", "user.email", "asb@example.invalid"], check=True)
            (root / "seed").write_text("seed\n", encoding="utf-8")
            subprocess.run(["git", "-C", str(root), "add", "seed"], check=True)
            subprocess.run(["git", "-C", str(root), "commit", "-q", "-m", "seed"], check=True)
            source_commit = subprocess.run(
                ["git", "-C", str(root), "rev-parse", "HEAD"], check=True,
                capture_output=True, text=True,
            ).stdout.strip()
            source_tree = subprocess.run(
                ["git", "-C", str(root), "rev-parse", "HEAD^{tree}"], check=True,
                capture_output=True, text=True,
            ).stdout.strip()
            path = root / "platforms/v1/native-evidence/ubuntu-x86.json"
            path.parent.mkdir(parents=True)
            check = {
                "status": "passed",
                "argv_sha256": "sha256:" + "b" * 64,
                "output_sha256": "sha256:" + "c" * 64,
                "output_bytes": 1,
            }
            report = {
                "format_version": 1,
                "kind": "native-run",
                "qualification": "native-functional",
                "performance_baseline": False,
                "platform_id": "ubuntu-24.04",
                "architecture": "x86_64",
                "kernel_release": "7.0.0-test",
                "kernel_provenance": {
                    "package": "linux-image-7.0.0-test",
                    "package_version": "7.0.0-1",
                    "package_architecture": "amd64",
                    "package_source": VALIDATOR.NATIVE_KERNEL_SOURCES["ubuntu-24.04"],
                    "package_integrity": "package-manifest-bound",
                    "package_manifest_sha256": "sha256:" + "4" * 64,
                    "image_package_digest": "5" * 32,
                    "running_notes_sha256": "sha256:" + "6" * 64,
                    "running_version_sha256": "sha256:" + "7" * 64,
                    "version_signature": "Ubuntu 7.0.0-1-test 7.0.0",
                },
                "virtualization": "kvm",
                "run_id": "run-1",
                "source_commit": source_commit,
                "source_tree": source_tree,
                "source_base_commit": source_commit,
                "distribution": {
                    "id": "ubuntu",
                    "version_id": "24.04",
                    "version": "24.04.4 LTS (Noble Numbat)",
                    "release_evidence": "24.04.4 LTS (Noble Numbat)",
                    "manifest_release": "24.04.4 LTS",
                },
                "capabilities": {
                    "cgroup_v2": "available",
                    "psi": "available",
                    "sandbox": "passed",
                    "security_modules": {
                        "apparmor": "registered-unproven", "selinux": "not-registered"
                    },
                },
                "sandbox_tools": [
                    {
                        "executable": executable,
                        "version": version,
                        "package": package,
                        "package_version": package_version,
                        "package_architecture": "amd64",
                        "source": source,
                        "version_output_sha256": "sha256:" + "8" * 64,
                        "binary_sha256": "sha256:" + "9" * 64,
                        "package_manifest_sha256": "sha256:" + "a" * 64,
                        "package_integrity": "verified",
                    }
                    for executable, version, package, package_version, source in
                    VALIDATOR.NATIVE_SANDBOX_TOOLS["ubuntu-24.04"]
                ],
                "checks": {name: check for name in ("process", "metrics", "sandbox")},
            }
            data = (json.dumps(report, indent=2, sort_keys=True) + "\n").encode()
            path.write_bytes(data)
            candidate = copy.deepcopy(self.platforms)
            cell = candidate["platforms"][0]["architectures"]["x86_64"]
            cell["native_kernel"] = "native-tested"
            cell["native_evidence"] = {
                "kind": "native-run",
                "platform_id": "ubuntu-24.04",
                "architecture": "x86_64",
                "kernel_release": "7.0.0-test",
                "run_id": "run-1",
                "artifact_path": "platforms/v1/native-evidence/ubuntu-x86.json",
                "artifact_digest": "sha256:" + hashlib.sha256(data).hexdigest(),
            }
            self.assertEqual(VALIDATOR.validate(candidate, self.agents, root), [])
            for mutate, expected in (
                (
                    lambda value: value["distribution"].update(
                        {"release_evidence": "24.04.3 LTS"}
                    ),
                    "exact release evidence differs",
                ),
                (
                    lambda value: value["distribution"].update(
                        {"manifest_release": "24.04 LTS"}
                    ),
                    "distribution binding differs",
                ),
                (
                    lambda value: value["capabilities"]["security_modules"].update(
                        {"apparmor": "unavailable"}
                    ),
                    "AppArmor and SELinux state",
                ),
                (
                    lambda value: value["sandbox_tools"][0].update(
                        {"package_version": "unreviewed"}
                    ),
                    "tool pins or provenance differ",
                ),
                (
                    lambda value: value.update({"virtualization": "qemu"}),
                    "virtualization is not closed",
                ),
                (
                    lambda value: value["checks"]["process"].update(
                        {"output_bytes": (16 << 20) + 1}
                    ),
                    "output bounds differ",
                ),
                (
                    lambda value: value["checks"].update({"extra": copy.deepcopy(check)}),
                    "check set differs",
                ),
                (
                    lambda value: value["kernel_provenance"].update(
                        {"package_version": "secret=private"}
                    ),
                    "privacy or safe-string boundary differs",
                ),
                (
                    lambda value: value.update({"source_tree": "0" * 40}),
                    "source ancestry/tree differs",
                ),
                (
                    lambda value: value["kernel_provenance"].update(
                        {"version_signature": "Ubuntu other"}
                    ),
                    "kernel version signature differs",
                ),
            ):
                with self.subTest(expected=expected):
                    broken_report = copy.deepcopy(report)
                    mutate(broken_report)
                    broken_data = (json.dumps(broken_report, indent=2, sort_keys=True) + "\n").encode()
                    path.write_bytes(broken_data)
                    cell["native_evidence"]["artifact_digest"] = (
                        "sha256:" + hashlib.sha256(broken_data).hexdigest()
                    )
                    self.assertTrue(
                        any(
                            expected in error
                            for error in VALIDATOR.validate(candidate, self.agents, root)
                        )
                    )
            noncanonical = (json.dumps(report, sort_keys=True) + "\n").encode()
            path.write_bytes(noncanonical)
            cell["native_evidence"]["artifact_digest"] = (
                "sha256:" + hashlib.sha256(noncanonical).hexdigest()
            )
            self.assertTrue(any(
                "not canonical JSON" in error
                for error in VALIDATOR.validate(candidate, self.agents, root)
            ))
            path.write_bytes(data)
            cell["native_evidence"]["artifact_digest"] = (
                "sha256:" + hashlib.sha256(data).hexdigest()
            )
            for key, value, expected in (
                ("artifact_digest", "sha256:" + "0" * 64, "digest differs"),
                ("artifact_path", "../outside.json", "path is unsafe"),
            ):
                with self.subTest(key=key):
                    broken = copy.deepcopy(candidate)
                    broken["platforms"][0]["architectures"]["x86_64"]["native_evidence"][key] = value
                    self.assertTrue(
                        any(expected in error for error in VALIDATOR.validate(broken, self.agents, root))
                    )
            report["checks"]["sandbox"] = {"status": "unavailable"}
            bad_data = (json.dumps(report, indent=2, sort_keys=True) + "\n").encode()
            path.write_bytes(bad_data)
            cell["native_evidence"]["artifact_digest"] = "sha256:" + hashlib.sha256(bad_data).hexdigest()
            self.assertTrue(
                any("sandbox did not pass" in error for error in VALIDATOR.validate(candidate, self.agents, root))
            )
            for malformed in ([], {"checks": []}, {"capabilities": []}):
                with self.subTest(malformed=malformed):
                    malformed_data = (json.dumps(malformed) + "\n").encode()
                    path.write_bytes(malformed_data)
                    cell["native_evidence"]["artifact_digest"] = (
                        "sha256:" + hashlib.sha256(malformed_data).hexdigest()
                    )
                    errors = VALIDATOR.validate(candidate, self.agents, root)
                    self.assertTrue(errors)


if __name__ == "__main__":
    unittest.main()
