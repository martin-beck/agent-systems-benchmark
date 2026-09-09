# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Host-independent tests for the authorized capacity lifecycle contract."""

from __future__ import annotations

import fcntl
import importlib.util
import json
import os
import stat
import sys
import tempfile
import unittest
from collections.abc import Callable
from contextlib import redirect_stderr
from dataclasses import replace
from io import StringIO
from pathlib import Path
from typing import cast
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "capacity_lifecycle", ROOT / "tools/capacity/lifecycle.py"
)
assert SPEC and SPEC.loader
LIFECYCLE = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = LIFECYCLE
SPEC.loader.exec_module(LIFECYCLE)


class CapacityLifecycleTests(unittest.TestCase):
    def setUp(self) -> None:
        self.available = LIFECYCLE.CapacityState.available("dev-x86-012345abcdef")

    def test_acquire_renew_release_is_deterministic(self) -> None:
        first = self.available.acquire(
            expected_revision=0, owner="worker-a", now=1000, ttl_seconds=60
        )
        repeated = self.available.acquire(
            expected_revision=0, owner="worker-a", now=1000, ttl_seconds=60
        )
        self.assertEqual(first.to_json(), repeated.to_json())
        renewed = first.renew(
            expected_revision=1, owner="worker-a", now=1010, ttl_seconds=60
        )
        released = renewed.release(expected_revision=2, owner="worker-a", now=1020)
        self.assertEqual(released.state, "available")
        self.assertEqual(released.revision, 3)
        self.assertIsNone(released.owner)
        self.assertEqual(
            released, LIFECYCLE.CapacityState.from_json(released.to_json())
        )

    def test_conflict_stale_owner_expiry_and_cost_fail_closed(self) -> None:
        leased = self.available.acquire(
            expected_revision=0, owner="worker-a", now=1000, ttl_seconds=60
        )
        cases: tuple[Callable[[], object], ...] = (
            lambda: leased.acquire(
                expected_revision=1, owner="worker-b", now=1001, ttl_seconds=60
            ),
            lambda: leased.renew(
                expected_revision=0, owner="worker-a", now=1001, ttl_seconds=60
            ),
            lambda: leased.release(expected_revision=1, owner="worker-b", now=1001),
            lambda: leased.renew(
                expected_revision=1, owner="worker-a", now=1060, ttl_seconds=60
            ),
            lambda: leased.release(expected_revision=1, owner="worker-a", now=1060),
            lambda: leased.release(expected_revision=1, owner="worker-a", now=999),
            lambda: replace(leased, cost_ceiling_microunits=1).validate(),
        )
        for case in cases:
            with self.subTest(case=case), self.assertRaises(LIFECYCLE.CapacityError):
                case()

    def test_uncertain_effect_requires_exact_positive_reconciliation(self) -> None:
        leased = self.available.acquire(
            expected_revision=0, owner="worker-a", now=1000, ttl_seconds=60
        )
        uncertain = leased.uncertain(expected_revision=1, owner="worker-a")
        self.assertEqual(uncertain.state, "needs_reconciliation")
        for kwargs in (
            {"expected_revision": 1, "owner": "worker-a", "observed_clean": True},
            {"expected_revision": 2, "owner": "worker-b", "observed_clean": True},
            {"expected_revision": 2, "owner": "worker-a", "observed_clean": False},
        ):
            with (
                self.subTest(kwargs=kwargs),
                self.assertRaises(LIFECYCLE.CapacityError),
            ):
                uncertain.reconcile_clean(**kwargs)
        clean = uncertain.reconcile_clean(
            expected_revision=2, owner="worker-a", observed_clean=True
        )
        self.assertEqual((clean.state, clean.revision), ("available", 3))

    def test_closed_canonical_parser_rejects_private_or_unsupported_identity(
        self,
    ) -> None:
        good = self.available.to_json()
        self.assertEqual(self.available, LIFECYCLE.CapacityState.from_json(good))
        mutations: tuple[Callable[[dict[str, object]], None], ...] = (
            lambda value: value.update({"hostname": "private"}),
            lambda value: value.update({"capacity_id": "private-host.example"}),
            lambda value: value.update({"architecture": "aarch64"}),
            lambda value: value.update({"trust_class": "public-pr"}),
            lambda value: value.update({"revision": True}),
        )
        for mutation in mutations:
            value: dict[str, object]
            value = json.loads(good)
            mutation(value)
            payload = json.dumps(value, sort_keys=True, separators=(",", ":")).encode()
            with (
                self.subTest(payload=payload),
                self.assertRaises(LIFECYCLE.CapacityError),
            ):
                LIFECYCLE.CapacityState.from_json(payload)
        with self.assertRaisesRegex(LIFECYCLE.CapacityError, "canonical"):
            LIFECYCLE.CapacityState.from_json(
                json.dumps(json.loads(good), indent=2).encode()
            )

    def test_time_and_identity_bounds(self) -> None:
        for owner, now, ttl in (
            ("UPPER", 1000, 60),
            ("worker-a", -1, 60),
            ("worker-a", 1000, 0),
            ("worker-a", 1000, LIFECYCLE.MAX_LEASE_SECONDS + 1),
            ("worker-a", LIFECYCLE.MAX_CLOCK_SECONDS, 1),
        ):
            with (
                self.subTest(owner=owner, now=now, ttl=ttl),
                self.assertRaises(LIFECYCLE.CapacityError),
            ):
                self.available.acquire(
                    expected_revision=0,
                    owner=owner,
                    now=now,
                    ttl_seconds=ttl,
                )

    def test_private_ledger_persists_one_exact_revision(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            root.chmod(0o700)
            ledger = LIFECYCLE.CapacityLedger(root)
            self.assertEqual(
                ledger.initialize(self.available.capacity_id), self.available
            )
            leased = ledger.transition(
                lambda state: state.acquire(
                    expected_revision=0,
                    owner="worker-a",
                    now=1000,
                    ttl_seconds=60,
                )
            )
            self.assertEqual(ledger.load(), leased)
            self.assertEqual((root / "state.json").stat().st_mode & 0o777, 0o600)
            self.assertFalse((root / LIFECYCLE.TEMP_NAME).exists())

    def test_corrupt_symlink_and_failed_transition_never_replace_state(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            root.chmod(0o700)
            ledger = LIFECYCLE.CapacityLedger(root)
            ledger.initialize(self.available.capacity_id)
            state_path = root / "state.json"
            original = state_path.read_bytes()
            with self.assertRaisesRegex(LIFECYCLE.CapacityError, "stale"):
                ledger.transition(
                    lambda state: state.acquire(
                        expected_revision=99,
                        owner="worker-a",
                        now=1000,
                        ttl_seconds=60,
                    )
                )
            self.assertEqual(state_path.read_bytes(), original)
            state_path.write_bytes(b"not-json")
            with self.assertRaisesRegex(LIFECYCLE.CapacityError, "valid JSON"):
                ledger.load()
            state_path.unlink()
            state_path.symlink_to(root / "missing")
            with self.assertRaisesRegex(LIFECYCLE.CapacityError, "ownership or mode"):
                ledger.load()

    def test_lock_contention_fails_without_waiting_or_mutation(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            root.chmod(0o700)
            ledger = LIFECYCLE.CapacityLedger(root)
            ledger.initialize(self.available.capacity_id)
            original = (root / "state.json").read_bytes()
            lock_descriptor = os.open(root / LIFECYCLE.LOCK_NAME, os.O_RDWR)
            try:
                fcntl.flock(lock_descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
                with self.assertRaisesRegex(LIFECYCLE.CapacityError, "already locked"):
                    ledger.load()
            finally:
                os.close(lock_descriptor)
            self.assertEqual((root / "state.json").read_bytes(), original)

    def test_mid_read_replacement_is_detected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            root.chmod(0o700)
            ledger = LIFECYCLE.CapacityLedger(root)
            ledger.initialize(self.available.capacity_id)
            original_reader = LIFECYCLE._read_bounded_fd

            def replacing_reader(descriptor: int) -> bytes:
                payload = cast(bytes, original_reader(descriptor))
                replacement = root / "replacement"
                replacement.write_bytes(payload)
                replacement.chmod(0o600)
                replacement.replace(root / "state.json")
                return payload

            with (
                mock.patch.object(LIFECYCLE, "_read_bounded_fd", replacing_reader),
                self.assertRaisesRegex(LIFECYCLE.CapacityError, "during read"),
            ):
                ledger.load()

    def test_root_mode_and_crash_residue_fail_closed(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            root.chmod(0o755)
            ledger = LIFECYCLE.CapacityLedger(root)
            with self.assertRaisesRegex(LIFECYCLE.CapacityError, "root ownership"):
                ledger.initialize(self.available.capacity_id)
            root.chmod(0o700)
            (root / LIFECYCLE.TEMP_NAME).write_bytes(b"stale")
            (root / LIFECYCLE.TEMP_NAME).chmod(0o600)
            with self.assertRaisesRegex(LIFECYCLE.CapacityError, "replaced safely"):
                ledger.initialize(self.available.capacity_id)

    def test_temporary_replacement_is_detected_before_commit(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            root.chmod(0o700)
            ledger = LIFECYCLE.CapacityLedger(root)
            ledger.initialize(self.available.capacity_id)
            original = (root / "state.json").read_bytes()
            original_fsync = os.fsync
            replaced = False

            def swapping_fsync(descriptor: int) -> None:
                nonlocal replaced
                original_fsync(descriptor)
                if not replaced and stat.S_ISREG(os.fstat(descriptor).st_mode):
                    intruder = root / "intruder"
                    intruder.write_bytes(original)
                    intruder.chmod(0o600)
                    intruder.replace(root / LIFECYCLE.TEMP_NAME)
                    replaced = True

            with (
                mock.patch.object(LIFECYCLE.os, "fsync", swapping_fsync),
                self.assertRaisesRegex(
                    LIFECYCLE.CapacityError, "temporary state changed"
                ),
            ):
                ledger.transition(
                    lambda state: state.acquire(
                        expected_revision=0,
                        owner="worker-a",
                        now=1000,
                        ttl_seconds=60,
                    )
                )
            self.assertTrue(replaced)
            self.assertEqual((root / "state.json").read_bytes(), original)

    def test_cli_lifecycle_fixture_reaches_clean_terminal_state(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            root.chmod(0o700)
            fixtures = (
                (
                    "initialize",
                    "--root",
                    str(root),
                    "--capacity-id",
                    self.available.capacity_id,
                ),
                (
                    "acquire",
                    "--root",
                    str(root),
                    "--expected-revision",
                    "0",
                    "--owner",
                    "worker-a",
                    "--now",
                    "1000",
                    "--ttl-seconds",
                    "60",
                ),
                (
                    "uncertain",
                    "--root",
                    str(root),
                    "--expected-revision",
                    "1",
                    "--owner",
                    "worker-a",
                ),
                (
                    "reconcile",
                    "--root",
                    str(root),
                    "--expected-revision",
                    "2",
                    "--owner",
                    "worker-a",
                    "--observed-clean",
                ),
            )
            states = [
                LIFECYCLE.execute(LIFECYCLE.parser().parse_args(case))
                for case in fixtures
            ]
            self.assertEqual(
                [(state.revision, state.state) for state in states],
                [
                    (0, "available"),
                    (1, "reserved"),
                    (2, "needs_reconciliation"),
                    (3, "available"),
                ],
            )
            self.assertEqual(LIFECYCLE.CapacityLedger(root).load(), states[-1])

    def test_cli_negative_is_atomic_and_does_not_disclose_root(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            root.chmod(0o700)
            ledger = LIFECYCLE.CapacityLedger(root)
            ledger.initialize(self.available.capacity_id)
            original = (root / "state.json").read_bytes()
            error = StringIO()
            with redirect_stderr(error):
                result = LIFECYCLE.main(
                    (
                        "acquire",
                        "--root",
                        str(root),
                        "--expected-revision",
                        "9",
                        "--owner",
                        "worker-a",
                        "--now",
                        "1000",
                        "--ttl-seconds",
                        "60",
                    )
                )
            self.assertEqual(result, 2)
            self.assertEqual(
                error.getvalue(), "ERROR: capacity lifecycle request rejected\n"
            )
            self.assertNotIn(str(root), error.getvalue())
            self.assertEqual((root / "state.json").read_bytes(), original)
            error.seek(0)
            error.truncate()
            with redirect_stderr(error):
                malformed = LIFECYCLE.main(
                    (
                        "acquire",
                        "--root",
                        str(root),
                        "--expected-revision",
                        "0",
                        "--owner",
                        "worker-a",
                        "--now",
                        "private-sentinel",
                        "--ttl-seconds",
                        "60",
                    )
                )
            self.assertEqual(malformed, 2)
            self.assertEqual(
                error.getvalue(), "ERROR: capacity lifecycle request rejected\n"
            )
            self.assertNotIn("private-sentinel", error.getvalue())
            self.assertEqual((root / "state.json").read_bytes(), original)


if __name__ == "__main__":
    unittest.main()
