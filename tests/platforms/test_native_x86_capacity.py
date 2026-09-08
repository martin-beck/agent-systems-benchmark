# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
from __future__ import annotations

import fcntl
import importlib.util
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import jsonschema

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "native_x86_capacity", ROOT / "tools/platforms/native_x86_capacity.py"
)
assert SPEC and SPEC.loader
CAPACITY = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = CAPACITY
SPEC.loader.exec_module(CAPACITY)


def valid_report() -> dict[str, object]:
    digest = "sha256:" + "a" * 64
    commit = "b" * 40
    check = {
        "argv_sha256": digest,
        "output_bytes": 1,
        "output_sha256": digest,
        "status": "passed",
    }
    return {
        "format_version": 1,
        "kind": "native-x86-capacity",
        "qualification": "native-functional",
        "performance_baseline": False,
        "run_id": "native-x86-test",
        "source": {"base_commit": commit, "commit": commit, "tree": commit},
        "host": {
            "architecture": "x86_64",
            "distribution": {
                "id": "ubuntu",
                "version_id": "24.04",
                "version": "24.04.4 LTS (Noble Numbat)",
            },
            "kernel": {
                "package": "linux-image-7.0.0-28-generic",
                "package_version": "7.0.0-28.28~24.04.1",
                "image_package_md5": "c" * 32,
                "license_document_sha256": digest,
                "package_manifest_sha256": digest,
                "package_source": CAPACITY.UBUNTU_PACKAGE_SOURCE,
                "release": "7.0.0-28-generic",
                "running_notes_sha256": digest,
                "running_version_sha256": digest,
            },
            "virtualization": "none",
            "cgroup_v2": "available",
            "psi": {"cpu": "available", "io": "available", "memory": "available"},
            "apparmor": "registered-unproven",
            "selinux": "unavailable",
            "tools": [
                {
                    "binary_sha256": digest,
                    "executable": executable,
                    "license_document_sha256": digest,
                    "package": package,
                    "package_architecture": "amd64",
                    "package_integrity": "verified",
                    "package_manifest_sha256": digest,
                    "package_source": CAPACITY.UBUNTU_PACKAGE_SOURCE,
                    "package_version": version,
                }
                for executable, (package, version) in CAPACITY.EXPECTED_PACKAGES.items()
            ],
        },
        "limits": {
            "cpu_quota_percent": 400,
            "memory_mib": 8192,
            "storage_mib": 16384,
            "tasks_max": 256,
            "timeout_seconds": 1200,
        },
        "isolation": {
            "asb_exclusive_lease": "passed",
            "credential_injection": "none",
            "network": "private-loopback-only",
            "persistent_runner_dispatch": "not-used",
            "transient_user_service": "passed",
        },
        "checks": {name: check.copy() for name in CAPACITY.CHECKS},
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


class NativeX86CapacityTests(unittest.TestCase):
    def test_committed_native_evidence_is_schema_valid_and_sanitized(self) -> None:
        schema = json.loads(
            (ROOT / "platforms/v1/native-x86-capacity.schema.json").read_text()
        )
        evidence = sorted((ROOT / "platforms/v1/native-x86-evidence").glob("*.json"))
        self.assertEqual(
            [path.name for path in evidence],
            ["ubuntu-24-04-x86-64-native-functional.json"],
        )
        report = json.loads(evidence[0].read_text())
        jsonschema.Draft202012Validator(schema).validate(report)
        serialized = json.dumps(report, sort_keys=True).lower()
        for forbidden in (
            "hostname",
            "/home/",
            "/srv/data/projects",
            "api_key",
            "bearer ",
            "authorization:",
        ):
            self.assertNotIn(forbidden, serialized)

    def test_exact_release_and_resource_bounds_fail_closed(self) -> None:
        release = (
            b'ID=ubuntu\nVERSION_ID="24.04"\nVERSION="24.04.4 LTS (Noble Numbat)"\n'
        )
        self.assertEqual(CAPACITY.parse_os_release(release)["ID"], "ubuntu")
        with self.assertRaisesRegex(CAPACITY.CapacityError, "exact release"):
            CAPACITY.parse_os_release(release.replace(b"24.04.4", b"24.04.3"))
        limits = CAPACITY.Limits(400, 8192, 16384, 1200, 256)
        limits.validate(32, 128000, 140000)
        for invalid in (
            CAPACITY.Limits(900, 8192, 16384, 1200, 256),
            CAPACITY.Limits(400, 70000, 16384, 1200, 256),
            CAPACITY.Limits(400, 8192, 90000, 1200, 256),
            CAPACITY.Limits(400, 8192, 16384, 0, 256),
            CAPACITY.Limits(400, 8192, 16384, 1200, 999),
        ):
            with self.assertRaises(CAPACITY.CapacityError):
                invalid.validate(32, 128000, 140000)

    def test_release_symlink_requires_exact_root_owned_contract(self) -> None:
        with tempfile.TemporaryDirectory(dir=ROOT) as temporary:
            root = Path(temporary)
            target = root / "usr/lib/os-release"
            target.parent.mkdir(parents=True)
            target.write_text("ID=ubuntu\n")
            target.chmod(0o644)
            link = root / "etc/os-release"
            link.parent.mkdir()
            link.symlink_to(Path("../usr/lib/os-release"))
            self.assertEqual(
                CAPACITY.read_os_release(link, target, os.getuid()), b"ID=ubuntu\n"
            )
            link.unlink()
            link.symlink_to(target)
            with self.assertRaisesRegex(CAPACITY.CapacityError, "differs"):
                CAPACITY.read_os_release(link, target, os.getuid())

    def test_scoped_command_is_shell_free_private_and_bounded(self) -> None:
        argv = CAPACITY.scoped_argv(
            "native-x86-test",
            Path("/cell/source"),
            Path("/cell"),
            Path("/cargo"),
            Path("/cargo-home"),
            Path("/rustup"),
            CAPACITY.Limits(400, 8192, 16384, 1200, 256),
            "metrics",
        )
        joined = "\n".join(argv)
        self.assertEqual(argv[0], "/usr/bin/systemd-run")
        self.assertIn("--property=PrivateNetwork=yes", argv)
        self.assertIn("--property=MemoryMax=8589934592", argv)
        self.assertIn("--property=RuntimeMaxSec=1200s", argv)
        self.assertIn(
            "--property=TemporaryFileSystem=/cell/target:rw,size=16384M,mode=0700", argv
        )
        self.assertIn("--property=ReadWritePaths=/cell", argv)
        self.assertIn("/cargo", argv)
        self.assertNotIn("sh", argv)
        self.assertNotIn("bash", argv)
        self.assertNotIn("TOKEN", joined)
        with self.assertRaisesRegex(CAPACITY.CapacityError, "unknown"):
            CAPACITY.scoped_argv(
                "native-x86-test",
                Path("/cell/source"),
                Path("/cell"),
                Path("/cargo"),
                Path("/cargo-home"),
                Path("/rustup"),
                CAPACITY.Limits(400, 8192, 16384, 1200, 256),
                "other",
            )
        with self.assertRaisesRegex(CAPACITY.CapacityError, "escapes"):
            CAPACITY.scoped_argv(
                "native-x86-test",
                Path("/source"),
                Path("/cell"),
                Path("/cargo"),
                Path("/cargo-home"),
                Path("/rustup"),
                CAPACITY.Limits(400, 8192, 16384, 1200, 256),
                "metrics",
            )

    def test_bounded_runner_rejects_failure_timeout_and_output_amplification(
        self,
    ) -> None:
        passed = CAPACITY.run_bounded(["/usr/bin/printf", "ok"], ROOT, 5)
        self.assertEqual(passed["status"], "passed")
        with self.assertRaisesRegex(CAPACITY.CapacityError, "failed"):
            CAPACITY.run_bounded(["/usr/bin/false"], ROOT, 5)
        with self.assertRaisesRegex(CAPACITY.CapacityError, "timed out"):
            CAPACITY.run_bounded(["/usr/bin/sleep", "2"], ROOT, 1)
        old = CAPACITY.MAX_OUTPUT_BYTES
        try:
            CAPACITY.MAX_OUTPUT_BYTES = 3
            with self.assertRaisesRegex(CAPACITY.CapacityError, "byte limit"):
                CAPACITY.run_bounded(["/usr/bin/printf", "overflow"], ROOT, 5)
        finally:
            CAPACITY.MAX_OUTPUT_BYTES = old

    def test_private_root_rejects_escape_mode_and_symlink(self) -> None:
        with tempfile.TemporaryDirectory(dir=ROOT) as temporary:
            trusted = Path(temporary)
            trusted.chmod(0o700)
            good = trusted / "good"
            good.mkdir(mode=0o700)
            CAPACITY.validate_root(good, trusted)
            good.chmod(0o755)
            with self.assertRaisesRegex(CAPACITY.CapacityError, "private"):
                CAPACITY.validate_root(good, trusted)
            outside = Path(temporary).parent
            with self.assertRaisesRegex(CAPACITY.CapacityError, "escapes"):
                CAPACITY.validate_root(outside, trusted)
            target = trusted / "target"
            target.mkdir(mode=0o700)
            link = trusted / "link"
            link.symlink_to(target, target_is_directory=True)
            with self.assertRaisesRegex(CAPACITY.CapacityError, "not a directory"):
                CAPACITY.validate_root(link, trusted)

    def test_lease_contention_and_cleanup_are_fail_closed(self) -> None:
        with tempfile.TemporaryDirectory(dir=ROOT) as temporary:
            root = Path(temporary)
            root.chmod(0o700)
            lock = os.open(
                root / ".native-x86-capacity.lock", os.O_RDWR | os.O_CREAT, 0o600
            )
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            with (
                mock.patch.object(CAPACITY, "validate_root"),
                mock.patch.object(
                    CAPACITY, "system_resources", return_value=(32, 128000, 140000)
                ),
                mock.patch.object(
                    CAPACITY, "source_identity", return_value=("a" * 40, "b" * 40)
                ),
                self.assertRaisesRegex(CAPACITY.CapacityError, "already leased"),
            ):
                CAPACITY.qualify(
                    "native-x86-test",
                    ROOT,
                    "a" * 40,
                    root,
                    Path("/usr/bin/true"),
                    root,
                    root,
                    CAPACITY.Limits(400, 8192, 16384, 1200, 256),
                )
            fcntl.flock(lock, fcntl.LOCK_UN)
            os.close(lock)

    def test_success_removes_cell_and_never_dispatches_persistent_runner(self) -> None:
        with tempfile.TemporaryDirectory(dir=ROOT) as temporary:
            root = Path(temporary)
            root.chmod(0o700)
            seen: list[list[str]] = []

            def runner(argv: object, _cwd: Path, _timeout: int) -> dict[str, object]:
                cast = list(argv)  # type: ignore[arg-type]
                seen.append(cast)
                return {
                    "argv_sha256": "sha256:" + "a" * 64,
                    "output_bytes": 0,
                    "output_sha256": "sha256:" + "b" * 64,
                    "status": "passed",
                }

            with (
                mock.patch.object(CAPACITY, "validate_root"),
                mock.patch.object(
                    CAPACITY, "system_resources", return_value=(32, 128000, 140000)
                ),
                mock.patch.object(
                    CAPACITY, "source_identity", return_value=("a" * 40, "b" * 40)
                ),
                mock.patch.object(
                    CAPACITY, "host_evidence", return_value=valid_report()["host"]
                ),
                mock.patch.object(
                    CAPACITY,
                    "materialize_source",
                    side_effect=lambda _source, destination, *_identity: (
                        destination.mkdir(mode=0o700) or destination
                    ),
                ),
            ):
                report = CAPACITY.qualify(
                    "native-x86-test",
                    ROOT,
                    "a" * 40,
                    root,
                    Path("/usr/bin/true"),
                    root,
                    root,
                    CAPACITY.Limits(400, 8192, 16384, 1200, 256),
                    runner,
                    lambda _argv, _cwd: True,
                )
            self.assertEqual(len(seen), 4)
            self.assertFalse((root / "cell-native-x86-test").exists())
            self.assertEqual(report["cleanup"]["cell_root"], "removed")  # type: ignore[index]
            self.assertEqual(
                report["isolation"]["persistent_runner_dispatch"], "not-used"
            )  # type: ignore[index]

    def test_missing_transient_unit_cleanup_fails_before_evidence(self) -> None:
        with tempfile.TemporaryDirectory(dir=ROOT) as temporary:
            root = Path(temporary)
            root.chmod(0o700)
            passed = {
                "argv_sha256": "sha256:" + "a" * 64,
                "output_bytes": 0,
                "output_sha256": "sha256:" + "b" * 64,
                "status": "passed",
            }
            with (
                mock.patch.object(CAPACITY, "validate_root"),
                mock.patch.object(
                    CAPACITY, "system_resources", return_value=(32, 128000, 140000)
                ),
                mock.patch.object(
                    CAPACITY, "source_identity", return_value=("a" * 40, "b" * 40)
                ),
                mock.patch.object(
                    CAPACITY, "host_evidence", return_value=valid_report()["host"]
                ),
                mock.patch.object(
                    CAPACITY,
                    "materialize_source",
                    side_effect=lambda _source, destination, *_identity: (
                        destination.mkdir(mode=0o700) or destination
                    ),
                ),
                self.assertRaisesRegex(CAPACITY.CapacityError, "not collected"),
            ):
                CAPACITY.qualify(
                    "native-x86-test",
                    ROOT,
                    "a" * 40,
                    root,
                    Path("/usr/bin/true"),
                    root,
                    root,
                    CAPACITY.Limits(400, 8192, 16384, 1200, 256),
                    lambda _argv, _cwd, _timeout: passed,
                    lambda _argv, _cwd: False,
                )
            self.assertFalse((root / "cell-native-x86-test").exists())

    def test_schema_rejects_forged_arch_cost_cleanup_and_extra_data(self) -> None:
        schema = json.loads(
            (ROOT / "platforms/v1/native-x86-capacity.schema.json").read_text()
        )
        jsonschema.Draft202012Validator(schema).validate(valid_report())
        mutations = [
            ("host", "architecture", "aarch64"),
            ("qualification", None, "native-performance"),
            ("performance_baseline", None, True),
            ("availability", None, "guaranteed"),
            ("cleanup", "cell_root", "unknown"),
            ("cost", "existing_host_cost", "0"),
        ]
        for outer, inner, value in mutations:
            report = valid_report()
            if inner is None:
                report[outer] = value
            else:
                report[outer][inner] = value  # type: ignore[index]
            with self.assertRaises(jsonschema.ValidationError):
                jsonschema.Draft202012Validator(schema).validate(report)
        report = valid_report()
        report["hostname"] = "private"
        with self.assertRaises(jsonschema.ValidationError):
            jsonschema.Draft202012Validator(schema).validate(report)

    def test_atomic_output_rejects_overwrite_and_symlinked_root(self) -> None:
        with tempfile.TemporaryDirectory(dir=ROOT) as temporary:
            output_root = Path(temporary)
            output_root.chmod(0o755)
            output = output_root / "native-x86-test.json"
            CAPACITY.write_atomic(output, valid_report(), output_root)
            self.assertTrue(output.is_file())
            with self.assertRaisesRegex(CAPACITY.CapacityError, "atomically"):
                CAPACITY.write_atomic(output, valid_report(), output_root)
            mismatched = output_root / "another-run.json"
            with self.assertRaisesRegex(CAPACITY.CapacityError, "safe run ID"):
                CAPACITY.write_atomic(mismatched, valid_report(), output_root)
            self.assertFalse(mismatched.exists())
            target = output_root / "target"
            target.mkdir()
            link = output_root / "link"
            link.symlink_to(target, target_is_directory=True)
            with self.assertRaisesRegex(CAPACITY.CapacityError, "ancestor"):
                CAPACITY.write_atomic(
                    link / "native-x86-test.json", valid_report(), link
                )


if __name__ == "__main__":
    unittest.main()
