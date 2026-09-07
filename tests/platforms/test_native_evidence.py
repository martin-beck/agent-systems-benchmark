# SPDX-License-Identifier: MIT
"""Positive and adversarial tests for native platform evidence."""

from __future__ import annotations

import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "native_evidence", ROOT / "tools/platforms/native_evidence.py"
)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("cannot load native evidence collector")
EVIDENCE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(EVIDENCE)


class NativeEvidenceTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(dir=os.environ.get("TMPDIR"))
        self.root = Path(self.temporary.name)
        (self.root / "etc").mkdir()
        (self.root / "proc/self").mkdir(parents=True)
        (self.root / "proc/pressure").mkdir(parents=True)
        (self.root / "sys/kernel/security").mkdir(parents=True)
        (self.root / "etc/os-release").write_text(
            "ID=ubuntu\nVERSION_ID=24.04\nVERSION=24.04.4 LTS (Noble Numbat)\n",
            encoding="utf-8",
        )
        (self.root / "proc/self/cgroup").write_text("0::/asb\n", encoding="utf-8")
        (self.root / "sys/kernel/security/lsm").write_text(
            "capability,landlock,apparmor\n", encoding="utf-8"
        )
        for name in ("cpu", "memory", "io"):
            (self.root / f"proc/pressure/{name}").write_text(
                "some avg10=0.00 total=0\n", encoding="utf-8"
            )

    def tearDown(self) -> None:
        self.temporary.cleanup()

    @staticmethod
    def probe(argv: list[str], _cwd: Path) -> str:
        values = {
            ("systemd-detect-virt", "--container"): "none",
            ("systemd-detect-virt", "--vm"): "kvm",
            ("git", "rev-parse", "HEAD"): "a" * 40,
            ("git", "status", "--porcelain"): "",
        }
        return values[tuple(argv)]

    def collect(self, probe=None, platform_id="ubuntu-24.04"):
        result = {
            "status": "passed",
            "argv_sha256": "sha256:" + "b" * 64,
            "output_sha256": "sha256:" + "c" * 64,
            "output_bytes": 3,
        }
        with mock.patch.object(EVIDENCE.platform, "machine", return_value="x86_64"), mock.patch.object(
            EVIDENCE.platform, "release", return_value="7.0.0-test"
        ), mock.patch.object(EVIDENCE, "run_check", return_value=result):
            return EVIDENCE.collect(
                platform_id, "x86_64", "run-1", ROOT,
                [("process", ["cargo", "test"])], root=self.root,
                probe=probe or self.probe,
            )

    def test_report_is_bound_sanitized_and_not_a_performance_baseline(self) -> None:
        report = self.collect()
        self.assertEqual(report["source_commit"], "a" * 40)
        self.assertEqual(report["virtualization"], "kvm")
        self.assertFalse(report["performance_baseline"])
        self.assertEqual(
            report["capabilities"]["security_modules"],
            {"apparmor": "enabled", "selinux": "disabled"},
        )
        self.assertEqual(len(report["sandbox_tools"]), 4)
        self.assertTrue(
            all(tool["source"].startswith("https://") for tool in report["sandbox_tools"])
        )
        encoded = json.dumps(report)
        self.assertNotIn(str(ROOT), encoded)
        self.assertNotIn("cargo test", encoded)

    def test_distribution_architecture_container_and_emulation_fail_closed(self) -> None:
        (self.root / "etc/os-release").write_text(
            "ID=debian\nVERSION_ID=13\nVERSION=13.6\n", encoding="utf-8"
        )
        with self.assertRaisesRegex(EVIDENCE.EvidenceError, "distribution"):
            self.collect()
        (self.root / "etc/os-release").write_text(
            "ID=ubuntu\nVERSION_ID=24.04\nVERSION=24.04.4 LTS\n", encoding="utf-8"
        )
        with mock.patch.object(
            EVIDENCE.platform, "machine", return_value="armv8l"
        ), self.assertRaisesRegex(EVIDENCE.EvidenceError, "canonical"):
            EVIDENCE.collect(
                "ubuntu-24.04", "aarch64", "run-1", ROOT,
                [("process", ["true"])], root=self.root, probe=self.probe,
            )
        container = lambda argv, cwd: "docker" if argv[-1] == "--container" else self.probe(argv, cwd)
        with self.assertRaisesRegex(EVIDENCE.EvidenceError, "container"):
            self.collect(container)
        emulated = lambda argv, cwd: "qemu" if argv[-1] == "--vm" else self.probe(argv, cwd)
        with self.assertRaisesRegex(EVIDENCE.EvidenceError, "emulated"):
            self.collect(emulated)
        (self.root / "etc/os-release").write_text(
            "ID=ubuntu\nVERSION_ID=24.04\nVERSION=24.04.3 LTS\n", encoding="utf-8"
        )
        with self.assertRaisesRegex(EVIDENCE.EvidenceError, "exact pinned release"):
            self.collect()
        (self.root / "etc/os-release").write_text(
            "ID=debian\nVERSION_ID=13\nVERSION=13 (trixie)\n", encoding="utf-8"
        )
        (self.root / "etc/debian_version").write_text("13.5\n", encoding="utf-8")
        with self.assertRaisesRegex(EVIDENCE.EvidenceError, "exact pinned release"):
            self.collect(platform_id="debian-13")
        (self.root / "etc/debian_version").write_text("13.6\n", encoding="utf-8")
        report = self.collect(platform_id="debian-13")
        self.assertEqual(report["distribution"]["release_evidence"], "13.6")

    def test_dirty_source_missing_cgroup_and_psi_fail_closed(self) -> None:
        dirty = lambda argv, cwd: " M secret" if argv[:2] == ["git", "status"] else self.probe(argv, cwd)
        with self.assertRaisesRegex(EVIDENCE.EvidenceError, "dirty"):
            self.collect(dirty)
        (self.root / "proc/self/cgroup").write_text("2:cpu:/legacy\n", encoding="utf-8")
        with self.assertRaisesRegex(EVIDENCE.EvidenceError, "cgroup"):
            self.collect()
        (self.root / "proc/self/cgroup").write_text("0::/asb\n", encoding="utf-8")
        (self.root / "proc/pressure/io").write_text("full avg10=0.0\n", encoding="utf-8")
        with self.assertRaisesRegex(EVIDENCE.EvidenceError, "PSI"):
            self.collect()

    def test_malformed_duplicate_and_oversized_inputs_fail_closed(self) -> None:
        for content in (
            "ID=ubuntu\nID=debian\nVERSION_ID=24.04\n",
            "ID ubuntu\nVERSION_ID=24.04\n",
            "ID=ubuntu\nVERSION_ID=24.04\x01\n",
        ):
            with self.subTest(content=content):
                (self.root / "etc/os-release").write_text(content, encoding="utf-8")
                with self.assertRaises(EVIDENCE.EvidenceError):
                    self.collect()
        (self.root / "etc/os-release").write_bytes(b"A" * (EVIDENCE.MAX_SOURCE_BYTES + 1))
        with self.assertRaisesRegex(EVIDENCE.EvidenceError, "byte limit"):
            self.collect()
        invalid = self.root / "invalid"
        invalid.write_bytes(b"\xff")
        with self.assertRaisesRegex(EVIDENCE.EvidenceError, "UTF-8"):
            EVIDENCE.read_bounded(invalid)
        with self.assertRaisesRegex(EVIDENCE.EvidenceError, "lacks ID"):
            EVIDENCE.parse_os_release("# comment\nNAME=test\n")
        self.assertEqual(
            EVIDENCE.parse_os_release('ID="ubuntu"\nVERSION_ID="24.04"\n')["ID"],
            "ubuntu",
        )

    def test_unsafe_run_checks_and_output_paths_fail_closed(self) -> None:
        with mock.patch.object(
            EVIDENCE.platform, "machine", return_value="x86_64"
        ), mock.patch.object(
            EVIDENCE.platform, "release", return_value="7.0.0-test"
        ), self.assertRaisesRegex(EVIDENCE.EvidenceError, "run ID"):
            EVIDENCE.collect(
                "ubuntu-24.04", "x86_64", "x" * 129, ROOT,
                [("process", ["true"])], root=self.root, probe=self.probe,
            )
        with mock.patch.object(
            EVIDENCE.platform, "machine", return_value="x86_64"
        ), mock.patch.object(
            EVIDENCE.platform, "release", return_value="7.0.0-test"
        ), self.assertRaisesRegex(EVIDENCE.EvidenceError, "nonempty"):
            EVIDENCE.collect(
                "ubuntu-24.04", "x86_64", "run-1", ROOT, [],
                root=self.root, probe=self.probe,
            )
        target = self.root / "report.json"
        outside = self.root / "outside"
        outside.write_text("sentinel", encoding="utf-8")
        target.symlink_to(outside)
        with self.assertRaisesRegex(EVIDENCE.EvidenceError, "already exist"):
            EVIDENCE.write_atomic(target, {"safe": True})
        self.assertEqual(outside.read_text(encoding="utf-8"), "sentinel")
        target.unlink()
        EVIDENCE.write_atomic(target, {"safe": True})
        self.assertEqual(json.loads(target.read_text(encoding="utf-8")), {"safe": True})
        with self.assertRaisesRegex(EVIDENCE.EvidenceError, "already exist"):
            EVIDENCE.write_atomic(target, {"safe": False})
        real_parent = self.root / "real-parent"
        real_parent.mkdir()
        alias_parent = self.root / "alias-parent"
        alias_parent.symlink_to(real_parent, target_is_directory=True)
        with self.assertRaisesRegex(EVIDENCE.EvidenceError, "parent"):
            EVIDENCE.write_atomic(alias_parent / "report.json", {"safe": True})
        with self.assertRaisesRegex(EVIDENCE.EvidenceError, "parent"):
            EVIDENCE.write_atomic(alias_parent / "missing/report.json", {"safe": True})
        self.assertFalse((real_parent / "missing").exists())

    def test_real_command_records_digests_and_failure_without_raw_output(self) -> None:
        result = EVIDENCE.run_check(["/bin/sh", "-c", "printf private"], ROOT)
        self.assertEqual(result["output_bytes"], 7)
        self.assertNotIn("private", json.dumps(result))
        with self.assertRaisesRegex(EVIDENCE.EvidenceError, "failed"):
            EVIDENCE.run_check(["/bin/false"], ROOT)
        with mock.patch.object(
            EVIDENCE, "MAX_OUTPUT_BYTES", 100
        ), self.assertRaisesRegex(EVIDENCE.EvidenceError, "byte limit"):
            EVIDENCE.run_check(
                ["/usr/bin/python3", "-c", "import sys; sys.stdout.write(chr(120) * 1000)"],
                ROOT, timeout=5,
            )

    def test_optional_unavailable_check_is_partial_not_false_green(self) -> None:
        passed = {
            "status": "passed",
            "argv_sha256": "sha256:" + "b" * 64,
            "output_sha256": "sha256:" + "c" * 64,
            "output_bytes": 3,
        }
        with mock.patch.object(EVIDENCE.platform, "machine", return_value="x86_64"), mock.patch.object(
            EVIDENCE.platform, "release", return_value="7.0.0-test"
        ), mock.patch.object(
            EVIDENCE, "run_check", side_effect=[passed, EVIDENCE.EvidenceError("unavailable")]
        ):
            report = EVIDENCE.collect(
                "ubuntu-24.04", "x86_64", "run-1", ROOT,
                [("process", ["true"])], [("sandbox", ["false"])],
                root=self.root, probe=self.probe, sandbox_probe=lambda _platform, _cwd: False,
            )
        self.assertEqual(report["qualification"], "native-functional-partial")
        self.assertEqual(report["checks"]["sandbox"], {"status": "unavailable"})
        self.assertEqual(report["capabilities"]["sandbox"], "unavailable")

    def test_capable_optional_sandbox_failure_is_fatal(self) -> None:
        passed = {
            "status": "passed",
            "argv_sha256": "sha256:" + "b" * 64,
            "output_sha256": "sha256:" + "c" * 64,
            "output_bytes": 3,
        }
        with mock.patch.object(EVIDENCE.platform, "machine", return_value="x86_64"), mock.patch.object(
            EVIDENCE.platform, "release", return_value="7.0.0-test"
        ), mock.patch.object(
            EVIDENCE, "run_check", side_effect=[passed, EVIDENCE.EvidenceError("regression")]
        ), self.assertRaisesRegex(EVIDENCE.EvidenceError, "regression"):
            EVIDENCE.collect(
                "ubuntu-24.04", "x86_64", "run-1", ROOT,
                [("process", ["true"])], [("sandbox", ["false"])],
                root=self.root, probe=self.probe, sandbox_probe=lambda _platform, _cwd: True,
            )

    def test_only_sandbox_may_be_optional(self) -> None:
        passed = {
            "status": "passed",
            "argv_sha256": "sha256:" + "b" * 64,
            "output_sha256": "sha256:" + "c" * 64,
            "output_bytes": 3,
        }
        with mock.patch.object(EVIDENCE.platform, "machine", return_value="x86_64"), mock.patch.object(
            EVIDENCE.platform, "release", return_value="7.0.0-test"
        ), mock.patch.object(EVIDENCE, "run_check", return_value=passed), self.assertRaisesRegex(
            EVIDENCE.EvidenceError, "only the native sandbox"
        ):
            EVIDENCE.collect(
                "ubuntu-24.04", "x86_64", "run-1", ROOT,
                [("process", ["true"])], [("metrics", ["true"])],
                root=self.root, probe=self.probe,
            )

    def test_check_parser_rejects_shellish_and_malformed_forms(self) -> None:
        self.assertEqual(EVIDENCE.parse_check('process=["/bin/true"]'), ("process", ["/bin/true"]))
        for value in ("missing", "UPPER=[]", "name=bad", "name=[]", 'name=[""]'):
            with self.subTest(value=value), self.assertRaises(EVIDENCE.argparse.ArgumentTypeError):
                EVIDENCE.parse_check(value)
        for argv in ([], [""], [1]):
            with self.subTest(argv=argv), self.assertRaisesRegex(EVIDENCE.EvidenceError, "argv"):
                EVIDENCE.run_check(argv, ROOT)

    def test_probe_and_timeout_failures_are_bounded(self) -> None:
        self.assertEqual(
            EVIDENCE.command_output(["/bin/sh", "-c", "printf none; exit 1"], ROOT),
            "none",
        )
        with self.assertRaisesRegex(EVIDENCE.EvidenceError, "probe failed"):
            EVIDENCE.command_output(["/bin/false"], ROOT)
        with self.assertRaisesRegex(EVIDENCE.EvidenceError, "could not complete"):
            EVIDENCE.command_output(["/definitely/absent"], ROOT)
        with self.assertRaisesRegex(EVIDENCE.EvidenceError, "timed out"):
            EVIDENCE.run_check(["/bin/sleep", "1"], ROOT, timeout=0)
        with self.assertRaisesRegex(EVIDENCE.EvidenceError, "could not start"):
            EVIDENCE.run_check(["/definitely/absent"], ROOT)

    def test_sandbox_preflight_checks_every_pin_and_scope(self) -> None:
        for platform_id, tools in EVIDENCE.SANDBOX_TOOL_PROFILES.items():
            good = subprocess.CompletedProcess(
                [], 0, " ".join(version for _, version, _, _ in tools), ""
            )
            with self.subTest(platform_id=platform_id), mock.patch.object(
                EVIDENCE.subprocess, "run", return_value=good
            ):
                self.assertTrue(EVIDENCE.sandbox_capable(platform_id, ROOT))
                evidence = EVIDENCE.sandbox_tool_evidence(platform_id)
                self.assertEqual(
                    [item["package"] for item in evidence],
                    [package for _, _, package, _ in tools],
                )
        bad = subprocess.CompletedProcess([], 1, "", "")
        with mock.patch.object(EVIDENCE.subprocess, "run", return_value=bad):
            self.assertFalse(EVIDENCE.sandbox_capable("ubuntu-24.04", ROOT))
        with mock.patch.object(EVIDENCE.subprocess, "run", side_effect=OSError):
            self.assertFalse(EVIDENCE.sandbox_capable("ubuntu-24.04", ROOT))

    def test_security_modules_are_explicit_and_missing_privilege_is_partial(self) -> None:
        self.assertEqual(
            EVIDENCE.security_module_state(self.root),
            {"apparmor": "enabled", "selinux": "disabled"},
        )
        (self.root / "sys/kernel/security/lsm").write_text(
            "capability,selinux\n", encoding="utf-8"
        )
        (self.root / "sys/fs/selinux").mkdir(parents=True)
        (self.root / "sys/fs/selinux/enforce").write_text("1\n", encoding="utf-8")
        self.assertEqual(
            EVIDENCE.security_module_state(self.root),
            {"apparmor": "disabled", "selinux": "enforcing"},
        )
        (self.root / "sys/kernel/security/lsm").unlink()
        self.assertEqual(
            EVIDENCE.security_module_state(self.root),
            {"apparmor": "unavailable", "selinux": "unavailable"},
        )
        report = self.collect()
        self.assertEqual(report["qualification"], "native-functional-partial")

    def test_additional_collection_bindings_fail_closed(self) -> None:
        with self.assertRaisesRegex(EVIDENCE.EvidenceError, "eligible"):
            EVIDENCE.validate_platform(
                "unknown",
                {"ID": "ubuntu", "VERSION_ID": "24.04", "VERSION": "24.04.4 LTS"},
                self.root,
            )
        invalid_probe = lambda argv, cwd: "short" if argv[:2] == ["git", "rev-parse"] else self.probe(argv, cwd)
        with self.assertRaisesRegex(EVIDENCE.EvidenceError, "immutable"):
            self.collect(invalid_probe)
        with mock.patch.object(EVIDENCE.platform, "machine", return_value="x86_64"), mock.patch.object(
            EVIDENCE.platform, "release", return_value=""
        ), self.assertRaisesRegex(EVIDENCE.EvidenceError, "kernel"):
            EVIDENCE.collect(
                "ubuntu-24.04", "x86_64", "run-1", ROOT,
                [("process", ["true"])], root=self.root, probe=self.probe,
            )
        with mock.patch.object(
            EVIDENCE.platform, "machine", return_value="x86_64"
        ), self.assertRaisesRegex(EVIDENCE.EvidenceError, "differs"):
            EVIDENCE.collect(
                "ubuntu-24.04", "aarch64", "run-1", ROOT,
                [("process", ["true"])], root=self.root, probe=self.probe,
            )

    def test_main_writes_success_and_reports_collector_failure(self) -> None:
        output = self.root / "main.json"
        report = {"platform_id": "ubuntu-24.04", "architecture": "x86_64"}
        argv = [
            "native_evidence.py", "--platform-id", "ubuntu-24.04",
            "--architecture", "x86_64", "--run-id", "run-1",
            "--source", str(ROOT), "--output", str(output),
            "--check", 'process=["/bin/true"]',
        ]
        with mock.patch.object(sys, "argv", argv), mock.patch.object(
            EVIDENCE, "collect", return_value=report
        ):
            self.assertEqual(EVIDENCE.main(), 0)
        self.assertTrue(output.is_file())
        output.unlink()
        with mock.patch.object(sys, "argv", argv), mock.patch.object(
            EVIDENCE, "collect", side_effect=EVIDENCE.EvidenceError("expected")
        ):
            self.assertEqual(EVIDENCE.main(), 1)


if __name__ == "__main__":
    unittest.main()
