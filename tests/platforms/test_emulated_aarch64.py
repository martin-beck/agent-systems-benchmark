# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Fail-closed tests for the emulated aarch64 evidence contract."""

from __future__ import annotations

import copy
import importlib.util
import json
import re
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "emulated_aarch64", ROOT / "tools/platforms/emulated_aarch64.py"
)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("cannot load emulated-aarch64 validator")
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)

AGENT_LIBRARY_INVOCATION = """          cargo +1.93.0 test --offline --locked \\
            --target aarch64-unknown-linux-gnu -p asb-agents --lib -- \\
            --test-threads=1 \\
            --skip mini_swe::tests::trajectory_file_and_spawn_failures_are_bounded_and_cleaned
"""


class EmulatedAarch64Tests(unittest.TestCase):
    """Exercise the closed configuration and evidence schema."""

    @classmethod
    def setUpClass(cls) -> None:
        cls.config = MODULE.load(ROOT / "platforms/v1/emulated-aarch64.json")

    def report(self) -> dict[str, object]:
        """Return synthetic public evidence."""
        return {
            "format_version": 1,
            "evidence_kind": "emulated-aarch64",
            "host_architecture": "x86_64",
            "emulated_architecture": "aarch64",
            "kernel_release": "6.8.0-79-generic",
            "kernel_scope": "shared-host-kernel-not-guest",
            "userspace_reference": self.config["guest_image"]["reference"],
            "emulator_sha256": self.config["emulator"]["sha256"],
            "toolchain_target": "aarch64-unknown-linux-gnu",
            "native_kernel": False,
            "architecture_performance": False,
        }

    def test_canonical_configuration_and_report_pass(self) -> None:
        self.assertEqual(MODULE.validate_config(self.config), [])
        self.assertEqual(MODULE.validate_report(self.config, self.report()), [])

    def test_native_or_performance_claims_fail_closed(self) -> None:
        for field in ("native_kernel", "architecture_performance"):
            with self.subTest(field=field):
                report = self.report()
                report[field] = True
                self.assertTrue(MODULE.validate_report(self.config, report))
        config = copy.deepcopy(self.config)
        config["claims"]["native_hardware"] = True
        self.assertTrue(MODULE.validate_config(config))

    def test_host_target_kernel_and_guest_binding_are_exact(self) -> None:
        for field, value in (
            ("host_architecture", "aarch64"),
            ("target_architecture", "x86_64"),
            ("kernel_scope", "native-guest-kernel"),
        ):
            with self.subTest(field=field):
                candidate = copy.deepcopy(self.config)
                candidate[field] = value
                self.assertTrue(MODULE.validate_config(candidate))
        candidate = copy.deepcopy(self.config)
        candidate["guest_image"]["reference"] = "docker.io/library/ubuntu:24.04"
        self.assertTrue(MODULE.validate_config(candidate))

    def test_unknown_missing_and_unbounded_values_are_rejected(self) -> None:
        candidate = copy.deepcopy(self.config)
        candidate["unexpected"] = True
        self.assertTrue(MODULE.validate_config(candidate))
        candidate = copy.deepcopy(self.config)
        candidate["test_scope"].remove("failure-paths")
        self.assertTrue(MODULE.validate_config(candidate))
        candidate = copy.deepcopy(self.config)
        candidate["limits"]["command_timeout_seconds"] = 0
        self.assertTrue(MODULE.validate_config(candidate))

    def test_report_rejects_private_field_bad_kernel_and_wrong_digest(self) -> None:
        report = self.report()
        report["hostname"] = "private-runner"
        self.assertTrue(MODULE.validate_report(self.config, report))
        report = self.report()
        report["kernel_release"] = "bad release with spaces"
        self.assertTrue(MODULE.validate_report(self.config, report))
        report = self.report()
        report["emulator_sha256"] = "0" * 64
        self.assertTrue(MODULE.validate_report(self.config, report))
        report = self.report()
        report["userspace_reference"] = "ambient-rootfs"
        self.assertTrue(MODULE.validate_report(self.config, report))

    def test_loader_bounds_configuration_size(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "oversized.json"
            path.write_text(json.dumps({"padding": "x" * 16_384}), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "exceeds"):
                MODULE.load(path)

    def test_workflow_is_x86_hosted_bounded_and_cleans_every_exit(self) -> None:
        workflow = (ROOT / ".github/workflows/emulated-aarch64.yml").read_text(
            encoding="utf-8"
        )
        self.assertIn("runs-on: ubuntu-24.04", workflow)
        self.assertIn("pull_request:", workflow)
        self.assertRegex(workflow, r"push:\n    branches: \[main\]")
        self.assertNotIn("runs-on: ubuntu-24.04-arm", workflow)
        self.assertIn("timeout-minutes: 20", workflow)
        self.assertIn("trap cleanup EXIT", workflow)
        self.assertIn("QEMU_LD_PREFIX:", workflow)
        self.assertIn(self.config["guest_image"]["reference"], workflow)
        self.assertNotIn("native-tested", workflow)

    def test_agent_library_inventory_runs_once_and_serially(self) -> None:
        workflow = (ROOT / ".github/workflows/emulated-aarch64.yml").read_text(
            encoding="utf-8"
        )
        self.assert_agent_library_invocation(workflow)

    def assert_agent_library_invocation(self, workflow: str) -> None:
        marker = "      - name: Materialize immutable aarch64 guest userspace\n"
        following = "      - name: Clean source tree\n"
        self.assertEqual(workflow.count(marker), 1)
        self.assertEqual(workflow.count(following), 1)
        materialization = workflow.split(marker, 1)[1].split(following, 1)[0]
        self.assertEqual(materialization.count(AGENT_LIBRARY_INVOCATION), 1)
        self.assertEqual(materialization.count("-p asb-agents"), 1)
        self.assertEqual(materialization.count("--test-threads=1"), 1)
        self.assertEqual(
            materialization.count(
                "--skip "
                "mini_swe::tests::trajectory_file_and_spawn_failures_are_bounded_and_cleaned"
            ),
            1,
        )
        self.assertIsNone(
            re.search(r"(?m)^\s*(?:for|while|until|select|retry)\b", materialization)
        )

    def test_agent_library_serialization_contract_rejects_widening(self) -> None:
        workflow = (ROOT / ".github/workflows/emulated-aarch64.yml").read_text(
            encoding="utf-8"
        )
        mutations = {
            "serialization removed": workflow.replace(
                "            --test-threads=1 \\\n", ""
            ),
            "duplicate invocation": workflow.replace(
                AGENT_LIBRARY_INVOCATION, AGENT_LIBRARY_INVOCATION * 2
            ),
            "differently spelled split invocation": workflow.replace(
                AGENT_LIBRARY_INVOCATION,
                AGENT_LIBRARY_INVOCATION
                + "          cargo +1.93.0 test --offline --locked \\\n"
                + "            --target aarch64-unknown-linux-gnu \\\n"
                + "            --lib -p asb-agents\n",
            ),
            "test filter added": workflow.replace(
                "-p asb-agents --lib -- \\\n",
                "-p asb-agents --lib -- gemini::tests:: \\\n",
            ),
            "retry wrapper added": workflow.replace(
                AGENT_LIBRARY_INVOCATION,
                AGENT_LIBRARY_INVOCATION.replace("cargo", "retry 2 cargo", 1),
            ),
            "unchanged invocation looped": workflow.replace(
                AGENT_LIBRARY_INVOCATION,
                "          for attempt in 1 2; do\n"
                + AGENT_LIBRARY_INVOCATION
                + "          done\n",
            ),
        }
        for name, mutation in mutations.items():
            with self.subTest(name=name), self.assertRaises(AssertionError):
                self.assert_agent_library_invocation(mutation)

    def test_guest_pin_matches_canonical_platform_manifest(self) -> None:
        platforms = MODULE.load(ROOT / "platforms/v1/platforms.json")["platforms"]
        ubuntu = next(row for row in platforms if row["id"] == "ubuntu-24.04")
        guest = self.config["guest_image"]
        self.assertEqual(guest["release"], ubuntu["release"])
        self.assertEqual(
            guest["reference"],
            "docker.io/library/ubuntu@"
            + ubuntu["architectures"]["aarch64"]["image_digest"],
        )


if __name__ == "__main__":
    unittest.main()
