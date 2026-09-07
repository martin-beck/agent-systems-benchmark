# SPDX-License-Identifier: MIT
"""Positive and adversarial tests for native platform evidence."""

from __future__ import annotations

import importlib.util
import json
import os
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
        (self.root / "etc/os-release").write_text(
            "ID=ubuntu\nVERSION_ID=24.04\n", encoding="utf-8"
        )
        (self.root / "proc/self/cgroup").write_text("0::/asb\n", encoding="utf-8")
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

    def collect(self, probe=None):
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
                "ubuntu-24.04", "x86_64", "run-1", ROOT,
                [("process", ["cargo", "test"])], root=self.root,
                probe=probe or self.probe,
            )

    def test_report_is_bound_sanitized_and_not_a_performance_baseline(self) -> None:
        report = self.collect()
        self.assertEqual(report["source_commit"], "a" * 40)
        self.assertEqual(report["virtualization"], "kvm")
        self.assertFalse(report["performance_baseline"])
        encoded = json.dumps(report)
        self.assertNotIn(str(ROOT), encoded)
        self.assertNotIn("cargo test", encoded)

    def test_distribution_architecture_container_and_emulation_fail_closed(self) -> None:
        (self.root / "etc/os-release").write_text(
            "ID=debian\nVERSION_ID=13\n", encoding="utf-8"
        )
        with self.assertRaisesRegex(EVIDENCE.EvidenceError, "distribution"):
            self.collect()
        (self.root / "etc/os-release").write_text(
            "ID=ubuntu\nVERSION_ID=24.04\n", encoding="utf-8"
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
        with self.assertRaisesRegex(EVIDENCE.EvidenceError, "symlink"):
            EVIDENCE.write_atomic(target, {"safe": True})
        self.assertEqual(outside.read_text(encoding="utf-8"), "sentinel")

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
                root=self.root, probe=self.probe,
            )
        self.assertEqual(report["qualification"], "native-functional-partial")
        self.assertEqual(report["checks"]["sandbox"], {"status": "unavailable"})
        self.assertEqual(report["capabilities"]["sandbox"], "unavailable")


if __name__ == "__main__":
    unittest.main()
