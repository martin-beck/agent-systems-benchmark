# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
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
        (self.root / "sys/kernel/notes").write_bytes(b"kernel-notes")
        (self.root / "proc/version").write_text("Linux version test\n", encoding="utf-8")
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
            ("git", "rev-parse", "HEAD^{tree}"): "b" * 40,
            ("git", "merge-base", "d" * 40, "a" * 40): "d" * 40,
            ("git", "status", "--porcelain"): "",
        }
        return values[tuple(argv)]

    @staticmethod
    def tool_evidence(platform_id, architecture, _root, _cwd, _probe):
        package_arch = "amd64" if architecture == "x86_64" else "arm64"
        if platform_id.startswith("openeuler"):
            package_arch = architecture
        return [
            {
                "executable": executable,
                "version": version,
                "version_output_sha256": "sha256:" + "1" * 64,
                "package": package,
                "package_version": package_version,
                "package_architecture": package_arch,
                "source": source,
                "binary_sha256": "sha256:" + "2" * 64,
                "package_manifest_sha256": "sha256:" + "3" * 64,
                "package_integrity": "verified",
            }
            for executable, version, package, package_version, source
            in EVIDENCE.SANDBOX_TOOL_PROFILES[platform_id]
        ]

    @staticmethod
    def kernel_evidence(platform_id, architecture, kernel, _root, _cwd, _probe):
        package_arch = "amd64" if architecture == "x86_64" else "arm64"
        package = "linux-image-" + kernel
        integrity = "package-manifest-bound"
        image_digest = "4" * 32
        if platform_id.startswith("openeuler"):
            package_arch = architecture
            package = "kernel-core"
            integrity = image_digest = "rpm-verified"
        return {
            "package": package,
            "package_version": "1.2.3-1",
            "package_architecture": package_arch,
            "package_source": EVIDENCE.PLATFORMS[platform_id]["kernel_source"],
            "package_integrity": integrity,
            "package_manifest_sha256": "sha256:" + "5" * 64,
            "image_package_digest": image_digest,
            "running_notes_sha256": "sha256:" + "6" * 64,
            "running_version_sha256": "sha256:" + "7" * 64,
            "version_signature": "Ubuntu 1.2.3-test" if platform_id.startswith("ubuntu") else "unavailable",
        }

    def collect(self, probe=None, platform_id="ubuntu-24.04", run_check=None):
        result = {
            "status": "passed",
            "argv_sha256": "sha256:" + "b" * 64,
            "output_sha256": "sha256:" + "c" * 64,
            "output_bytes": 3,
        }
        with mock.patch.object(EVIDENCE.platform, "machine", return_value="x86_64"), mock.patch.object(
            EVIDENCE.platform, "release", return_value="7.0.0-test"
        ), mock.patch.object(EVIDENCE, "run_check", side_effect=run_check, return_value=result):
            return EVIDENCE.collect(
                platform_id, "x86_64", "run-1", ROOT, "d" * 40,
                [("process", ["cargo", "test"]), ("metrics", ["cargo", "test"])],
                [("sandbox", ["cargo", "test"])], root=self.root,
                probe=probe or self.probe,
                sandbox_probe=lambda *_: True,
                tool_evidence_probe=self.tool_evidence,
                kernel_evidence_probe=self.kernel_evidence,
            )

    def test_report_is_bound_sanitized_and_not_a_performance_baseline(self) -> None:
        report = self.collect()
        self.assertEqual(report["source_commit"], "a" * 40)
        self.assertEqual(report["virtualization"], "kvm")
        self.assertFalse(report["performance_baseline"])
        self.assertEqual(
            report["capabilities"]["security_modules"],
            {"apparmor": "registered-unproven", "selinux": "not-registered"},
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
                "ubuntu-24.04", "aarch64", "run-1", ROOT, "d" * 40,
                [("process", ["true"]), ("metrics", ["true"])],
                [("sandbox", ["true"])], root=self.root, probe=self.probe,
            )
        (self.root / "etc/os-release").write_text(
            "ID=ubuntu\nVERSION_ID=24.04\nVERSION=24.04.4 LTS (Noble Numbat)\n",
            encoding="utf-8",
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

    def test_mutating_success_and_source_identity_races_fail_closed(self) -> None:
        changed = False
        passed = {
            "status": "passed", "argv_sha256": "sha256:" + "b" * 64,
            "output_sha256": "sha256:" + "c" * 64, "output_bytes": 0,
        }

        def probe(argv, cwd):
            if argv[:2] == ["git", "status"] and changed:
                return " M tools/platforms/native_evidence.py"
            return self.probe(argv, cwd)

        def mutate(_argv, _cwd):
            nonlocal changed
            changed = True
            return passed

        with self.assertRaisesRegex(EVIDENCE.EvidenceError, "dirty"):
            self.collect(probe=probe, run_check=mutate)

        tree_reads = 0

        def changed_tree(argv, cwd):
            nonlocal tree_reads
            if tuple(argv) == ("git", "rev-parse", "HEAD^{tree}"):
                tree_reads += 1
                return ("b" if tree_reads == 1 else "c") * 40
            return self.probe(argv, cwd)

        with self.assertRaisesRegex(EVIDENCE.EvidenceError, "identity changed"):
            self.collect(probe=changed_tree)
        with self.assertRaisesRegex(EVIDENCE.EvidenceError, "base commit"):
            EVIDENCE.source_identity(ROOT, "not-a-commit", self.probe)

    def test_booted_kernel_is_bound_to_package_database_and_live_notes(self) -> None:
        kernel = "7.0.0-28-generic"
        package = "linux-image-" + kernel
        (self.root / "var/lib/dpkg/info").mkdir(parents=True)
        (self.root / f"var/lib/dpkg/info/{package}.md5sums").write_text(
            "4" * 32 + f"  boot/vmlinuz-{kernel}\n", encoding="utf-8"
        )
        (self.root / "proc/version_signature").write_text(
            "Ubuntu 7.0.0-28.28~24.04.1-generic 7.0.12\n", encoding="utf-8"
        )
        values = {
            ("/usr/bin/dpkg-query", "-S", f"/boot/vmlinuz-{kernel}"):
                f"{package}: /boot/vmlinuz-{kernel}",
            (
                "/usr/bin/dpkg-query", "-W",
                "-f=${binary:Package}\\t${Version}\\t${Architecture}\\t${Status}", package,
            ): f"{package}\t7.0.0-28.28~24.04.1\tamd64\tinstall ok installed",
        }
        evidence = EVIDENCE.kernel_provenance(
            "ubuntu-24.04", "x86_64", kernel, self.root, ROOT,
            lambda argv, _cwd: values[tuple(argv)],
        )
        self.assertEqual(evidence["package"], package)
        self.assertEqual(evidence["package_integrity"], "package-manifest-bound")
        self.assertRegex(evidence["running_notes_sha256"], r"^sha256:[0-9a-f]{64}$")
        (self.root / f"var/lib/dpkg/info/{package}.md5sums").write_text(
            "4" * 32 + "  boot/other\n", encoding="utf-8"
        )
        with self.assertRaisesRegex(EVIDENCE.EvidenceError, "absent from package manifest"):
            EVIDENCE.kernel_provenance(
                "ubuntu-24.04", "x86_64", kernel, self.root, ROOT,
                lambda argv, _cwd: values[tuple(argv)],
            )

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
                "ubuntu-24.04", "x86_64", "x" * 129, ROOT, "d" * 40,
                [("process", ["true"]), ("metrics", ["true"])],
                [("sandbox", ["true"])], root=self.root, probe=self.probe,
            )
        with mock.patch.object(
            EVIDENCE.platform, "machine", return_value="x86_64"
        ), mock.patch.object(
            EVIDENCE.platform, "release", return_value="7.0.0-test"
        ), self.assertRaisesRegex(EVIDENCE.EvidenceError, "exactly"):
            EVIDENCE.collect(
                "ubuntu-24.04", "x86_64", "run-1", ROOT, "d" * 40, [],
                root=self.root, probe=self.probe,
            )
        target = self.root / "report.json"
        outside = self.root / "outside"
        outside.write_text("sentinel", encoding="utf-8")
        target.symlink_to(outside)
        with self.assertRaisesRegex(EVIDENCE.EvidenceError, "already exist"):
            EVIDENCE.write_atomic(target, {"safe": True}, self.root)
        self.assertEqual(outside.read_text(encoding="utf-8"), "sentinel")
        target.unlink()
        EVIDENCE.write_atomic(target, {"safe": True}, self.root)
        self.assertEqual(json.loads(target.read_text(encoding="utf-8")), {"safe": True})
        with self.assertRaisesRegex(EVIDENCE.EvidenceError, "already exist"):
            EVIDENCE.write_atomic(target, {"safe": False}, self.root)
        real_parent = self.root / "real-parent"
        real_parent.mkdir()
        alias_parent = self.root / "alias-parent"
        alias_parent.symlink_to(real_parent, target_is_directory=True)
        with self.assertRaisesRegex(EVIDENCE.EvidenceError, "parent"):
            EVIDENCE.write_atomic(alias_parent / "report.json", {"safe": True}, self.root)
        with self.assertRaisesRegex(EVIDENCE.EvidenceError, "parent"):
            EVIDENCE.write_atomic(alias_parent / "missing/report.json", {"safe": True}, self.root)
        self.assertFalse((real_parent / "missing").exists())
        nested = self.root / "nested/deep/report.json"
        EVIDENCE.write_atomic(nested, {"safe": True}, self.root)
        self.assertEqual(json.loads(nested.read_text(encoding="utf-8")), {"safe": True})
        blocker = self.root / "not-a-directory"
        blocker.write_text("sentinel", encoding="utf-8")
        with self.assertRaisesRegex(EVIDENCE.EvidenceError, "parent"):
            EVIDENCE.write_atomic(blocker / "report.json", {"safe": True}, self.root)
        real_open = EVIDENCE.os.open

        def fail_temporary(path, *args, **kwargs):
            if isinstance(path, str) and ".tmp-" in path:
                raise OSError("expected")
            return real_open(path, *args, **kwargs)

        with mock.patch.object(EVIDENCE.os, "open", side_effect=fail_temporary), self.assertRaisesRegex(
            EVIDENCE.EvidenceError, "created safely"
        ):
            EVIDENCE.write_atomic(self.root / "open-failure.json", {"safe": True}, self.root)

        race_parent = self.root / "race"
        race_parent.mkdir()
        outside_directory = self.root / "outside-directory"
        outside_directory.mkdir()
        real_open = EVIDENCE.os.open
        raced = False

        def race_intermediate(path, *args, **kwargs):
            nonlocal raced
            child = race_parent / "child"
            if path == "child" and child.exists() and not raced:
                child.rename(race_parent / "owned-child")
                child.symlink_to(outside_directory, target_is_directory=True)
                raced = True
            return real_open(path, *args, **kwargs)

        with mock.patch.object(EVIDENCE.os, "open", side_effect=race_intermediate), self.assertRaisesRegex(
            EVIDENCE.EvidenceError, "symlinks"
        ):
            EVIDENCE.write_atomic(race_parent / "child/report.json", {"safe": True}, self.root)
        self.assertTrue(raced)
        self.assertFalse((outside_directory / "report.json").exists())

        destination = self.root / "raced-destination.json"
        real_link = EVIDENCE.os.link

        def race_destination(source, target_name, *args, **kwargs):
            descriptor = os.open(
                target_name, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600,
                dir_fd=kwargs["dst_dir_fd"],
            )
            os.write(descriptor, b"racer")
            os.close(descriptor)
            return real_link(source, target_name, *args, **kwargs)

        with mock.patch.object(EVIDENCE.os, "link", side_effect=race_destination), self.assertRaisesRegex(
            EVIDENCE.EvidenceError, "committed safely"
        ):
            EVIDENCE.write_atomic(destination, {"safe": True}, self.root)
        self.assertEqual(destination.read_bytes(), b"racer")

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
            EVIDENCE, "run_check", side_effect=[passed, passed]
        ):
            report = EVIDENCE.collect(
                "ubuntu-24.04", "x86_64", "run-1", ROOT, "d" * 40,
                [("process", ["true"]), ("metrics", ["true"])],
                [("sandbox", ["false"])], root=self.root, probe=self.probe,
                sandbox_probe=lambda *_: False, tool_evidence_probe=self.tool_evidence,
                kernel_evidence_probe=self.kernel_evidence,
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
            EVIDENCE, "run_check", side_effect=[passed, passed, EVIDENCE.EvidenceError("regression")]
        ), self.assertRaisesRegex(EVIDENCE.EvidenceError, "regression"):
            EVIDENCE.collect(
                "ubuntu-24.04", "x86_64", "run-1", ROOT, "d" * 40,
                [("process", ["true"]), ("metrics", ["true"])],
                [("sandbox", ["false"])], root=self.root, probe=self.probe,
                sandbox_probe=lambda *_: True, tool_evidence_probe=self.tool_evidence,
                kernel_evidence_probe=self.kernel_evidence,
            )

        with mock.patch.object(EVIDENCE.platform, "machine", return_value="x86_64"), mock.patch.object(
            EVIDENCE.platform, "release", return_value="7.0.0-test"
        ), mock.patch.object(
            EVIDENCE, "run_check", side_effect=EVIDENCE.EvidenceError("native check failed")
        ), self.assertRaisesRegex(EVIDENCE.EvidenceError, "process check did not pass"):
            EVIDENCE.collect(
                "ubuntu-24.04", "x86_64", "run-1", ROOT, "d" * 40,
                [("process", ["true"]), ("metrics", ["true"])],
                [("sandbox", ["true"])], root=self.root, probe=self.probe,
                sandbox_probe=lambda *_: True, tool_evidence_probe=self.tool_evidence,
                kernel_evidence_probe=self.kernel_evidence,
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
            EVIDENCE.EvidenceError, "exactly"
        ):
            EVIDENCE.collect(
                "ubuntu-24.04", "x86_64", "run-1", ROOT, "d" * 40,
                [("process", ["true"]), ("metrics", ["true"])],
                [("metrics", ["true"])],
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
        tools = EVIDENCE.SANDBOX_TOOL_PROFILES["ubuntu-24.04"]
        (self.root / "usr/bin").mkdir(parents=True)
        manifests: dict[str, list[str]] = {}
        values: dict[tuple[str, ...], str] = {}
        for index, (executable, version, package, package_version, _) in enumerate(tools):
            binary = self.root / executable.removeprefix("/")
            binary.write_bytes(f"binary-{index}".encode())
            digest = EVIDENCE.hashlib.md5(binary.read_bytes(), usedforsecurity=False).hexdigest()
            manifests.setdefault(package, []).append(f"{digest}  {executable.removeprefix('/')}")
            values[(executable, "--version")] = version
            values[("/usr/bin/dpkg-query", "-S", executable)] = f"{package}: {executable}"
            values[(
                "/usr/bin/dpkg-query", "-W",
                "-f=${binary:Package}\\t${Version}\\t${Architecture}\\t${Status}", package,
            )] = f"{package}\t{package_version}\tamd64\tinstall ok installed"
        (self.root / "var/lib/dpkg/info").mkdir(parents=True)
        for package, lines in manifests.items():
            (self.root / f"var/lib/dpkg/info/{package}.md5sums").write_text(
                "\n".join(lines) + "\n", encoding="utf-8"
            )
        observed = EVIDENCE.sandbox_tool_evidence(
            "ubuntu-24.04", "x86_64", self.root, ROOT,
            lambda argv, _cwd: values[tuple(argv)],
        )
        self.assertEqual([item["package"] for item in observed], [item[2] for item in tools])
        self.assertTrue(all(item["package_integrity"] == "verified" for item in observed))
        (self.root / "usr/bin/bwrap").write_bytes(b"replacement")
        with self.assertRaisesRegex(EVIDENCE.EvidenceError, "package manifest"):
            EVIDENCE.sandbox_tool_evidence(
                "ubuntu-24.04", "x86_64", self.root, ROOT,
                lambda argv, _cwd: values[tuple(argv)],
            )

        delegation = []

        def version_or_scope(argv, **_kwargs):
            if argv[0] == "/usr/bin/systemd-run" and "--user" in argv:
                delegation.extend(argv)
                return subprocess.CompletedProcess(argv, 0, "", "")
            version = next(item[1] for item in tools if item[0] == argv[0])
            return subprocess.CompletedProcess(argv, 0, version + "\n", "")

        with mock.patch.object(EVIDENCE, "sandbox_tool_evidence", return_value=observed), mock.patch.object(
            EVIDENCE.subprocess, "run", side_effect=version_or_scope
        ):
            self.assertTrue(EVIDENCE.sandbox_capable("ubuntu-24.04", "x86_64", self.root, ROOT))
        self.assertIn("MemoryMax=67108864", delegation)
        self.assertIn("--disable-userns", delegation)
        self.assertEqual(delegation[-2:], ["--", "/usr/bin/true"])
        with mock.patch.object(EVIDENCE, "sandbox_tool_evidence", side_effect=EVIDENCE.EvidenceError("drift")):
            self.assertFalse(EVIDENCE.sandbox_capable("ubuntu-24.04", "x86_64", self.root, ROOT))

    def test_package_integrity_metadata_failures_and_rpm_path(self) -> None:
        binary = self.root / "usr/bin/tool"
        binary.parent.mkdir(parents=True)
        binary.write_bytes(b"tool")
        digest = EVIDENCE.hashlib.md5(b"tool", usedforsecurity=False).hexdigest()
        info = self.root / "var/lib/dpkg/info"
        info.mkdir(parents=True)
        (info / "tool.md5sums").write_text(
            f"{digest}  usr/bin/tool\n", encoding="utf-8"
        )

        def dpkg(argv, _cwd):
            if argv[1] == "-S":
                return "tool: /usr/bin/tool"
            return "tool\t1.0\tamd64\tinstall ok installed"

        observed = EVIDENCE.dpkg_metadata(
            "/usr/bin/tool", "tool", "1.0", "x86_64", self.root, ROOT, dpkg
        )
        self.assertEqual(observed["package_integrity"], "verified")
        for replacement, message in (
            (lambda argv, cwd: "other: /usr/bin/tool" if argv[1] == "-S" else dpkg(argv, cwd), "ownership"),
            (lambda argv, cwd: "tool\t2.0\tamd64\tinstall ok installed" if argv[1] == "-W" else dpkg(argv, cwd), "metadata"),
        ):
            with self.subTest(message=message), self.assertRaisesRegex(
                EVIDENCE.EvidenceError, message
            ):
                EVIDENCE.dpkg_metadata(
                    "/usr/bin/tool", "tool", "1.0", "x86_64",
                    self.root, ROOT, replacement,
                )
        (info / "tool.md5sums").write_text("bad  usr/bin/other\n", encoding="utf-8")
        with self.assertRaisesRegex(EVIDENCE.EvidenceError, "does not bind"):
            EVIDENCE.dpkg_metadata(
                "/usr/bin/tool", "tool", "1.0", "x86_64", self.root, ROOT, dpkg
            )
        with self.assertRaisesRegex(EVIDENCE.EvidenceError, "bounded regular"):
            EVIDENCE.digest_file(binary.parent)
        with self.assertRaisesRegex(EVIDENCE.EvidenceError, "could not be read"):
            EVIDENCE.digest_file(self.root / "absent")

        def rpm(argv, _cwd):
            return "tool\t1.0\tx86_64" if argv[1] == "-qf" else ""

        observed = EVIDENCE.rpm_metadata(
            "/usr/bin/tool", "tool", "1.0", "x86_64", self.root, ROOT, rpm
        )
        self.assertEqual(observed["package_architecture"], "x86_64")
        with self.assertRaisesRegex(EVIDENCE.EvidenceError, "metadata"):
            EVIDENCE.rpm_metadata(
                "/usr/bin/tool", "tool", "2.0", "x86_64", self.root, ROOT, rpm
            )
        with self.assertRaisesRegex(EVIDENCE.EvidenceError, "integrity"):
            EVIDENCE.rpm_metadata(
                "/usr/bin/tool", "tool", "1.0", "x86_64", self.root, ROOT,
                lambda argv, cwd: "changed" if argv[1] == "-Vf" else rpm(argv, cwd),
            )

    def test_rpm_kernel_and_kernel_metadata_fail_closed(self) -> None:
        kernel = "6.6.0-test"

        def rpm(argv, _cwd):
            return "kernel-core\t6.6.0-1\tx86_64" if argv[1] == "-qf" else ""

        observed = EVIDENCE.kernel_provenance(
            "openeuler-24.03-lts-sp2", "x86_64", kernel,
            self.root, ROOT, rpm,
        )
        self.assertEqual(observed["package_integrity"], "rpm-verified")
        self.assertEqual(observed["version_signature"], "unavailable")
        with self.assertRaisesRegex(EVIDENCE.EvidenceError, "RPM metadata"):
            EVIDENCE.kernel_provenance(
                "openeuler-24.03-lts-sp2", "x86_64", kernel, self.root, ROOT,
                lambda argv, cwd: "kernel-core\t6.6.0-1\taarch64" if argv[1] == "-qf" else rpm(argv, cwd),
            )
        with self.assertRaisesRegex(EVIDENCE.EvidenceError, "integrity"):
            EVIDENCE.kernel_provenance(
                "openeuler-24.03-lts-sp2", "x86_64", kernel, self.root, ROOT,
                lambda argv, cwd: "changed" if argv[1] == "-Vf" else rpm(argv, cwd),
            )

        package = "linux-image-" + kernel
        info = self.root / "var/lib/dpkg/info"
        info.mkdir(parents=True, exist_ok=True)
        (info / f"{package}.md5sums").write_text(
            "4" * 32 + f"  boot/vmlinuz-{kernel}\n", encoding="utf-8"
        )

        def dpkg(argv, _cwd):
            if argv[1] == "-S":
                return f"{package}: /boot/vmlinuz-{kernel}"
            return f"{package}\t6.6.0-1\tamd64\tinstall ok installed"

        with self.assertRaisesRegex(EVIDENCE.EvidenceError, "ownership"):
            EVIDENCE.kernel_provenance(
                "ubuntu-24.04", "x86_64", kernel, self.root, ROOT,
                lambda argv, cwd: "unowned" if argv[1] == "-S" else dpkg(argv, cwd),
            )
        with self.assertRaisesRegex(EVIDENCE.EvidenceError, "metadata"):
            EVIDENCE.kernel_provenance(
                "ubuntu-24.04", "x86_64", kernel, self.root, ROOT,
                lambda argv, cwd: f"{package}\t6.6.0-1\tarm64\tinstall ok installed"
                if argv[1] == "-W" else dpkg(argv, cwd),
            )
        (self.root / "proc/version_signature").write_bytes(b"bad\x00signature")
        with self.assertRaisesRegex(EVIDENCE.EvidenceError, "unsafe"):
            EVIDENCE.kernel_provenance(
                "ubuntu-24.04", "x86_64", kernel, self.root, ROOT, dpkg
            )

    def test_sandbox_runtime_and_source_ancestry_fail_closed(self) -> None:
        observed = self.tool_evidence("ubuntu-24.04", "x86_64", self.root, ROOT, self.probe)
        with mock.patch.object(EVIDENCE, "sandbox_tool_evidence", return_value=observed), mock.patch.object(
            EVIDENCE.subprocess, "run", side_effect=OSError("expected")
        ):
            self.assertFalse(EVIDENCE.sandbox_capable("ubuntu-24.04", "x86_64", self.root, ROOT))

        calls = 0

        def versions_then_scope(argv, **_kwargs):
            nonlocal calls
            calls += 1
            if "--user" in argv:
                return subprocess.CompletedProcess(argv, 1, "", "")
            expected = next(item[1] for item in EVIDENCE.SANDBOX_TOOL_PROFILES["ubuntu-24.04"] if item[0] == argv[0])
            return subprocess.CompletedProcess(argv, 0, expected + "\n", "")

        with mock.patch.object(EVIDENCE, "sandbox_tool_evidence", return_value=observed), mock.patch.object(
            EVIDENCE.subprocess, "run", side_effect=versions_then_scope
        ):
            self.assertFalse(EVIDENCE.sandbox_capable("ubuntu-24.04", "x86_64", self.root, ROOT))
        self.assertEqual(calls, 5)
        with self.assertRaisesRegex(EVIDENCE.EvidenceError, "ancestry"):
            EVIDENCE.source_identity(
                ROOT, "d" * 40,
                lambda argv, cwd: "e" * 40 if argv[:2] == ["git", "merge-base"] else self.probe(argv, cwd),
            )

    def test_atomic_output_rejects_untrusted_roots_and_commit_failure(self) -> None:
        with self.assertRaisesRegex(EVIDENCE.EvidenceError, "escapes"):
            EVIDENCE.write_atomic(self.root.parent / "escape.json", {}, self.root)
        with self.assertRaisesRegex(EVIDENCE.EvidenceError, "invalid"):
            EVIDENCE.write_atomic(self.root, {}, self.root)
        trusted_link = self.root / "trusted-link"
        trusted_link.symlink_to(self.root, target_is_directory=True)
        with self.assertRaisesRegex(EVIDENCE.EvidenceError, "symlinks"):
            EVIDENCE.write_atomic(trusted_link / "report.json", {}, trusted_link)

        target = self.root / "commit-failure.json"
        with mock.patch.object(EVIDENCE.os, "link", side_effect=OSError("expected")), self.assertRaisesRegex(
            EVIDENCE.EvidenceError, "committed safely"
        ):
            EVIDENCE.write_atomic(target, {}, self.root)
        self.assertFalse(target.exists())

    def test_security_modules_are_explicit_and_missing_privilege_is_partial(self) -> None:
        self.assertEqual(
            EVIDENCE.security_module_state(self.root),
            {"apparmor": "registered-unproven", "selinux": "not-registered"},
        )
        (self.root / "sys/kernel/security/lsm").write_text(
            "capability,selinux\n", encoding="utf-8"
        )
        (self.root / "sys/fs/selinux").mkdir(parents=True)
        (self.root / "sys/fs/selinux/enforce").write_text("1\n", encoding="utf-8")
        self.assertEqual(
            EVIDENCE.security_module_state(self.root),
            {"apparmor": "not-registered", "selinux": "enforcing-observed"},
        )
        (self.root / "sys/fs/selinux/enforce").unlink()
        self.assertEqual(
            EVIDENCE.security_module_state(self.root),
            {"apparmor": "not-registered", "selinux": "unavailable"},
        )
        (self.root / "sys/kernel/security/lsm").write_text(
            "capability,BAD\n", encoding="utf-8"
        )
        self.assertEqual(
            EVIDENCE.security_module_state(self.root),
            {"apparmor": "unavailable", "selinux": "unavailable"},
        )
        (self.root / "sys/kernel/security/lsm").unlink()
        self.assertEqual(
            EVIDENCE.security_module_state(self.root),
            {"apparmor": "unavailable", "selinux": "unavailable"},
        )
        (self.root / "sys/kernel/security/lsm").write_text("", encoding="utf-8")
        self.assertEqual(
            EVIDENCE.security_module_state(self.root),
            {"apparmor": "unavailable", "selinux": "unavailable"},
        )
        (self.root / "sys/kernel/security/lsm").unlink()
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
                "ubuntu-24.04", "x86_64", "run-1", ROOT, "d" * 40,
                [("process", ["true"]), ("metrics", ["true"])],
                [("sandbox", ["true"])], root=self.root, probe=self.probe,
            )
        with mock.patch.object(
            EVIDENCE.platform, "machine", return_value="x86_64"
        ), self.assertRaisesRegex(EVIDENCE.EvidenceError, "differs"):
            EVIDENCE.collect(
                "ubuntu-24.04", "aarch64", "run-1", ROOT, "d" * 40,
                [("process", ["true"]), ("metrics", ["true"])],
                [("sandbox", ["true"])], root=self.root, probe=self.probe,
            )

    def test_main_writes_success_and_reports_collector_failure(self) -> None:
        output = self.root / "main.json"
        report = {"platform_id": "ubuntu-24.04", "architecture": "x86_64"}
        argv = [
            "native_evidence.py", "--platform-id", "ubuntu-24.04",
            "--architecture", "x86_64", "--run-id", "run-1",
            "--source", str(ROOT), "--output", str(output),
            "--base-commit", "d" * 40, "--output-root", str(self.root),
            "--check", 'process=["/bin/true"]', "--check", 'metrics=["/bin/true"]',
            "--optional-check", 'sandbox=["/bin/true"]',
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
