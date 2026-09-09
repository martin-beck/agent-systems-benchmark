#!/usr/bin/env python3
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Deterministic, provider-neutral development-capacity lease contract."""

from __future__ import annotations

import argparse
import fcntl
import json
import os
import re
import stat
import sys
from collections.abc import Callable, Iterator, Sequence
from contextlib import contextmanager
from dataclasses import asdict, dataclass, replace
from pathlib import Path
from typing import NoReturn

SCHEMA_VERSION = 1
ARCHITECTURE = "x86_64"
TRUST_CLASS = "trusted-development"
MAX_LEASE_SECONDS = 2 * 60 * 60
MAX_CLOCK_SECONDS = 4_102_444_800  # 2100-01-01T00:00:00Z
CAPACITY_ID = re.compile(r"^dev-x86-[0-9a-f]{12}$")
OWNER_ID = re.compile(r"^[a-z][a-z0-9._-]{0,63}$")
STATES = {"available", "reserved", "needs_reconciliation"}
FIELDS = {
    "schema_version",
    "capacity_id",
    "architecture",
    "trust_class",
    "revision",
    "state",
    "owner",
    "issued_at",
    "expires_at",
    "cost_ceiling_microunits",
}
STATE_NAME = "state.json"
LOCK_NAME = "lifecycle.lock"
TEMP_NAME = ".state.json.tmp"


class CapacityError(ValueError):
    """The requested transition is malformed, stale, or unsafe."""


class ClosedParser(argparse.ArgumentParser):
    """Reject malformed CLI input without reflecting private argument values."""

    def error(self, message: str) -> NoReturn:
        del message
        raise CapacityError("capacity command is malformed")


@dataclass(frozen=True)
class CapacityState:
    """Public lease state; capacity IDs are sanitized pseudonyms."""

    schema_version: int
    capacity_id: str
    architecture: str
    trust_class: str
    revision: int
    state: str
    owner: str | None
    issued_at: int | None
    expires_at: int | None
    cost_ceiling_microunits: int

    @classmethod
    def available(cls, capacity_id: str) -> CapacityState:
        """Create one unleased, zero-cost authorized x86_64 cell."""
        result = cls(
            schema_version=SCHEMA_VERSION,
            capacity_id=capacity_id,
            architecture=ARCHITECTURE,
            trust_class=TRUST_CLASS,
            revision=0,
            state="available",
            owner=None,
            issued_at=None,
            expires_at=None,
            cost_ceiling_microunits=0,
        )
        result.validate()
        return result

    @classmethod
    def from_json(cls, payload: bytes) -> CapacityState:
        """Parse one closed, bounded canonical state document."""
        if not payload or len(payload) > 4096:
            raise CapacityError("capacity state exceeds its byte bound")
        try:
            raw = json.loads(payload)
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            raise CapacityError("capacity state is not valid JSON") from error
        if not isinstance(raw, dict) or set(raw) != FIELDS:
            raise CapacityError("capacity state fields are not closed")
        try:
            state = cls(**raw)
        except TypeError as error:
            raise CapacityError("capacity state fields are malformed") from error
        state.validate()
        if state.to_json() != payload:
            raise CapacityError("capacity state is not canonical")
        return state

    def validate(self) -> None:
        """Reject unsupported, identifying, unbounded, or inconsistent state."""
        if self.schema_version != SCHEMA_VERSION:
            raise CapacityError("unsupported capacity schema")
        if not isinstance(self.capacity_id, str) or not CAPACITY_ID.fullmatch(
            self.capacity_id
        ):
            raise CapacityError("capacity identity is not a sanitized pseudonym")
        if self.architecture != ARCHITECTURE or self.trust_class != TRUST_CLASS:
            raise CapacityError("capacity class is unsupported")
        if type(self.revision) is not int or not 0 <= self.revision < 2**63:
            raise CapacityError("capacity revision is invalid")
        if self.state not in STATES:
            raise CapacityError("capacity state is invalid")
        if self.cost_ceiling_microunits != 0:
            raise CapacityError("external capacity cost is not authorized")
        if self.state == "available":
            if (self.owner, self.issued_at, self.expires_at) != (None, None, None):
                raise CapacityError("available capacity retains lease identity")
            return
        if not isinstance(self.owner, str) or not OWNER_ID.fullmatch(self.owner):
            raise CapacityError("lease owner is invalid")
        if (
            type(self.issued_at) is not int
            or type(self.expires_at) is not int
            or not 0 <= self.issued_at <= MAX_CLOCK_SECONDS
            or not self.issued_at < self.expires_at <= MAX_CLOCK_SECONDS
            or self.expires_at - self.issued_at > MAX_LEASE_SECONDS
        ):
            raise CapacityError("lease deadline is invalid")

    def to_json(self) -> bytes:
        """Return deterministic bounded public evidence."""
        return json.dumps(asdict(self), sort_keys=True, separators=(",", ":")).encode()

    def _expect(self, revision: int) -> None:
        if type(revision) is not int or revision != self.revision:
            raise CapacityError("stale lifecycle revision")

    def acquire(
        self, *, expected_revision: int, owner: str, now: int, ttl_seconds: int
    ) -> CapacityState:
        """Acquire only clean available capacity with an exact bounded lease."""
        self.validate()
        self._expect(expected_revision)
        if self.state != "available":
            raise CapacityError("capacity is not available")
        if not OWNER_ID.fullmatch(owner):
            raise CapacityError("lease owner is invalid")
        if (
            type(now) is not int
            or type(ttl_seconds) is not int
            or not 0 <= now <= MAX_CLOCK_SECONDS
            or not 1 <= ttl_seconds <= MAX_LEASE_SECONDS
            or now + ttl_seconds > MAX_CLOCK_SECONDS
        ):
            raise CapacityError("lease deadline is invalid")
        return replace(
            self,
            revision=self.revision + 1,
            state="reserved",
            owner=owner,
            issued_at=now,
            expires_at=now + ttl_seconds,
        )

    def renew(
        self, *, expected_revision: int, owner: str, now: int, ttl_seconds: int
    ) -> CapacityState:
        """Renew an exact live owner lease with a new bounded lease window."""
        self.validate()
        self._expect(expected_revision)
        if self.state != "reserved" or self.owner != owner:
            raise CapacityError("lease ownership differs")
        if (
            self.issued_at is None
            or self.expires_at is None
            or type(now) is not int
            or type(ttl_seconds) is not int
            or now < self.issued_at
            or now >= self.expires_at
            or not 1 <= ttl_seconds <= MAX_LEASE_SECONDS
            or now + ttl_seconds > MAX_CLOCK_SECONDS
        ):
            raise CapacityError("lease deadline is invalid")
        return replace(
            self,
            revision=self.revision + 1,
            issued_at=now,
            expires_at=now + ttl_seconds,
        )

    def release(self, *, expected_revision: int, owner: str, now: int) -> CapacityState:
        """Release a live exact-owner lease; expired work cannot report success."""
        self.validate()
        self._expect(expected_revision)
        if self.state != "reserved" or self.owner != owner:
            raise CapacityError("lease ownership differs")
        if (
            self.issued_at is None
            or self.expires_at is None
            or type(now) is not int
            or now < self.issued_at
            or now >= self.expires_at
        ):
            raise CapacityError("expired lease requires reconciliation")
        return replace(
            self,
            revision=self.revision + 1,
            state="available",
            owner=None,
            issued_at=None,
            expires_at=None,
        )

    def uncertain(self, *, expected_revision: int, owner: str) -> CapacityState:
        """Fence a reserved cell after process loss or uncertain external effects."""
        self.validate()
        self._expect(expected_revision)
        if self.state != "reserved" or self.owner != owner:
            raise CapacityError("lease ownership differs")
        return replace(self, revision=self.revision + 1, state="needs_reconciliation")

    def reconcile_clean(
        self, *, expected_revision: int, owner: str, observed_clean: bool
    ) -> CapacityState:
        """Return fenced capacity only after an operator proves cleanup."""
        self.validate()
        self._expect(expected_revision)
        if self.state != "needs_reconciliation" or self.owner != owner:
            raise CapacityError("reconciliation ownership differs")
        if observed_clean is not True:
            raise CapacityError("capacity cleanup is not proven")
        return replace(
            self,
            revision=self.revision + 1,
            state="available",
            owner=None,
            issued_at=None,
            expires_at=None,
        )


def _entry(metadata: os.stat_result) -> tuple[int, ...]:
    """Return the identity and mutable metadata relevant to one ledger entry."""
    return (
        metadata.st_dev,
        metadata.st_ino,
        metadata.st_mode,
        metadata.st_uid,
        metadata.st_gid,
        metadata.st_nlink,
        metadata.st_size,
        metadata.st_mtime_ns,
        metadata.st_ctime_ns,
    )


def _read_bounded_fd(descriptor: int) -> bytes:
    """Read one already-verified descriptor without accepting growth."""
    chunks: list[bytes] = []
    size = 0
    while chunk := os.read(descriptor, 4097 - size):
        size += len(chunk)
        if size > 4096:
            raise CapacityError("capacity state exceeds its byte bound")
        chunks.append(chunk)
    return b"".join(chunks)


class CapacityLedger:
    """Private, locked and atomically replaced capacity state storage."""

    def __init__(self, root: Path, expected_uid: int | None = None) -> None:
        self.root = root
        self.expected_uid = os.getuid() if expected_uid is None else expected_uid

    def _open_root(self) -> int:
        try:
            before = self.root.lstat()
            descriptor = os.open(
                self.root,
                os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC,
            )
            after = os.fstat(descriptor)
        except OSError as error:
            raise CapacityError("capacity root cannot be opened safely") from error
        if (
            not stat.S_ISDIR(before.st_mode)
            or _entry(before) != _entry(after)
            or after.st_uid != self.expected_uid
            or stat.S_IMODE(after.st_mode) != 0o700
        ):
            os.close(descriptor)
            raise CapacityError("capacity root ownership or mode differs")
        return descriptor

    @contextmanager
    def _locked_root(self) -> Iterator[int]:
        root_descriptor = self._open_root()
        lock_descriptor = -1
        try:
            try:
                lock_descriptor = os.open(
                    LOCK_NAME,
                    os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW | os.O_CLOEXEC,
                    0o600,
                    dir_fd=root_descriptor,
                )
                lock_metadata = os.fstat(lock_descriptor)
            except OSError as error:
                raise CapacityError("capacity lock cannot be opened safely") from error
            if (
                not stat.S_ISREG(lock_metadata.st_mode)
                or lock_metadata.st_uid != self.expected_uid
                or stat.S_IMODE(lock_metadata.st_mode) != 0o600
                or lock_metadata.st_nlink != 1
            ):
                raise CapacityError("capacity lock ownership or mode differs")
            try:
                fcntl.flock(lock_descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError as error:
                raise CapacityError("capacity lifecycle is already locked") from error
            yield root_descriptor
        finally:
            if lock_descriptor >= 0:
                os.close(lock_descriptor)
            os.close(root_descriptor)

    def _path_entry(self, root_descriptor: int) -> tuple[int, ...] | None:
        try:
            metadata = os.stat(
                STATE_NAME, dir_fd=root_descriptor, follow_symlinks=False
            )
        except FileNotFoundError:
            return None
        except OSError as error:
            raise CapacityError("capacity state cannot be inspected safely") from error
        if (
            not stat.S_ISREG(metadata.st_mode)
            or metadata.st_uid != self.expected_uid
            or stat.S_IMODE(metadata.st_mode) != 0o600
            or metadata.st_nlink != 1
            or metadata.st_size > 4096
        ):
            raise CapacityError("capacity state ownership or mode differs")
        return _entry(metadata)

    def _read_locked(
        self, root_descriptor: int
    ) -> tuple[CapacityState, tuple[int, ...]]:
        before = self._path_entry(root_descriptor)
        if before is None:
            raise CapacityError("capacity state is unavailable")
        descriptor = -1
        try:
            descriptor = os.open(
                STATE_NAME,
                os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC,
                dir_fd=root_descriptor,
            )
            opened = _entry(os.fstat(descriptor))
            if opened != before:
                raise CapacityError("capacity state changed before read")
            payload = _read_bounded_fd(descriptor)
            after = _entry(os.fstat(descriptor))
        except OSError as error:
            raise CapacityError("capacity state cannot be read safely") from error
        finally:
            if descriptor >= 0:
                os.close(descriptor)
        if opened != after or self._path_entry(root_descriptor) != before:
            raise CapacityError("capacity state changed during read")
        return CapacityState.from_json(payload), before

    def _write_locked(
        self,
        root_descriptor: int,
        state: CapacityState,
        expected_entry: tuple[int, ...] | None,
    ) -> None:
        state.validate()
        payload = state.to_json()
        descriptor = -1
        created = False
        try:
            descriptor = os.open(
                TEMP_NAME,
                os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC,
                0o600,
                dir_fd=root_descriptor,
            )
            created = True
            initial = os.fstat(descriptor)
            if (
                not stat.S_ISREG(initial.st_mode)
                or initial.st_uid != self.expected_uid
                or stat.S_IMODE(initial.st_mode) != 0o600
                or initial.st_nlink != 1
            ):
                raise CapacityError("temporary state ownership or mode differs")
            view = memoryview(payload)
            while view:
                written = os.write(descriptor, view)
                if written <= 0:
                    raise CapacityError("capacity state write made no progress")
                view = view[written:]
            os.fsync(descriptor)
            temporary_entry = _entry(os.fstat(descriptor))
            try:
                named_temporary = _entry(
                    os.stat(
                        TEMP_NAME,
                        dir_fd=root_descriptor,
                        follow_symlinks=False,
                    )
                )
            except OSError as error:
                raise CapacityError(
                    "temporary state changed before replacement"
                ) from error
            if named_temporary != temporary_entry:
                raise CapacityError("temporary state changed before replacement")
            if self._path_entry(root_descriptor) != expected_entry:
                raise CapacityError("capacity state changed before replacement")
            os.replace(
                TEMP_NAME,
                STATE_NAME,
                src_dir_fd=root_descriptor,
                dst_dir_fd=root_descriptor,
            )
            created = False
            os.fsync(root_descriptor)
        except OSError as error:
            raise CapacityError("capacity state cannot be replaced safely") from error
        finally:
            if descriptor >= 0:
                os.close(descriptor)
            if created:
                try:
                    os.unlink(TEMP_NAME, dir_fd=root_descriptor)
                except FileNotFoundError:
                    pass

    def initialize(self, capacity_id: str) -> CapacityState:
        """Create the first state only in an empty private ledger."""
        state = CapacityState.available(capacity_id)
        with self._locked_root() as root_descriptor:
            if self._path_entry(root_descriptor) is not None:
                raise CapacityError("capacity state already exists")
            self._write_locked(root_descriptor, state, None)
        return state

    def load(self) -> CapacityState:
        """Load one stable canonical state while holding the lifecycle lock."""
        with self._locked_root() as root_descriptor:
            state, _ = self._read_locked(root_descriptor)
        return state

    def transition(
        self, change: Callable[[CapacityState], CapacityState]
    ) -> CapacityState:
        """Persist exactly one validated revision transition atomically."""
        with self._locked_root() as root_descriptor:
            before, entry = self._read_locked(root_descriptor)
            after = change(before)
            if (
                not isinstance(after, CapacityState)
                or after.capacity_id != before.capacity_id
                or after.revision != before.revision + 1
            ):
                raise CapacityError("capacity transition is not one exact revision")
            self._write_locked(root_descriptor, after, entry)
        return after


def _common_transition(parser: argparse.ArgumentParser, *, timed: bool) -> None:
    parser.add_argument("--root", required=True, type=Path)
    parser.add_argument("--expected-revision", required=True, type=int)
    parser.add_argument("--owner", required=True)
    if timed:
        parser.add_argument("--now", required=True, type=int)


def parser() -> ClosedParser:
    """Build the closed command surface without environment-derived defaults."""
    result = ClosedParser(prog="asb-capacity-lifecycle")
    commands = result.add_subparsers(dest="command", required=True)
    initialize = commands.add_parser("initialize")
    initialize.add_argument("--root", required=True, type=Path)
    initialize.add_argument("--capacity-id", required=True)
    show = commands.add_parser("show")
    show.add_argument("--root", required=True, type=Path)
    for name in ("acquire", "renew"):
        transition = commands.add_parser(name)
        _common_transition(transition, timed=True)
        transition.add_argument("--ttl-seconds", required=True, type=int)
    release = commands.add_parser("release")
    _common_transition(release, timed=True)
    uncertain = commands.add_parser("uncertain")
    _common_transition(uncertain, timed=False)
    reconcile = commands.add_parser("reconcile")
    _common_transition(reconcile, timed=False)
    reconcile.add_argument("--observed-clean", action="store_true")
    return result


def execute(args: argparse.Namespace) -> CapacityState:
    """Execute exactly one parsed lifecycle operation."""
    ledger = CapacityLedger(args.root)
    if args.command == "initialize":
        return ledger.initialize(args.capacity_id)
    if args.command == "show":
        return ledger.load()
    if args.command == "acquire":
        return ledger.transition(
            lambda state: state.acquire(
                expected_revision=args.expected_revision,
                owner=args.owner,
                now=args.now,
                ttl_seconds=args.ttl_seconds,
            )
        )
    if args.command == "renew":
        return ledger.transition(
            lambda state: state.renew(
                expected_revision=args.expected_revision,
                owner=args.owner,
                now=args.now,
                ttl_seconds=args.ttl_seconds,
            )
        )
    if args.command == "release":
        return ledger.transition(
            lambda state: state.release(
                expected_revision=args.expected_revision,
                owner=args.owner,
                now=args.now,
            )
        )
    if args.command == "uncertain":
        return ledger.transition(
            lambda state: state.uncertain(
                expected_revision=args.expected_revision, owner=args.owner
            )
        )
    if args.command == "reconcile":
        return ledger.transition(
            lambda state: state.reconcile_clean(
                expected_revision=args.expected_revision,
                owner=args.owner,
                observed_clean=args.observed_clean,
            )
        )
    raise CapacityError("capacity command is unsupported")


def main(argv: Sequence[str] | None = None) -> int:
    """Run one bounded operation and emit only canonical public state."""
    try:
        state = execute(parser().parse_args(argv))
    except CapacityError:
        print("ERROR: capacity lifecycle request rejected", file=sys.stderr)
        return 2
    sys.stdout.buffer.write(state.to_json() + b"\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
