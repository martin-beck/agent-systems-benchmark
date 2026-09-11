#!/usr/bin/env python3
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Adversarial tests for the offline commit-signature trust boundary."""

from __future__ import annotations

import hashlib
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

from tools.quality import repository_policy as policy

A01 = "a01f7f21f5be07dda7f18185724122c56dff1bb7"
CAD9 = "cad9fa9777aaca45b9ee62801d89168c5f3e8c32"
MERGE_0C = "0c65159d70ee728e21c7936663a90bea49ab0366"
MERGE_0C_BASE = "58d0da27736d6c22ca7c43f76ade497165b29919"


def run(*args: str, cwd: Path, env: dict[str, str] | None = None) -> str:
    result = subprocess.run(
        args,
        cwd=cwd,
        env=env,
        text=True,
        capture_output=True,
        check=True,
    )
    return result.stdout.strip()


class SignaturePolicyTests(unittest.TestCase):
    def test_current_protected_merge_and_historical_dco_failures(self) -> None:
        for revision in (MERGE_0C, A01, CAD9):
            policy.verify_github_web_flow(
                policy.ROOT,
                policy.GITHUB_WEB_FLOW_KEY,
                policy.GITHUB_WEB_FLOW_KEY_SHA256,
                policy.GITHUB_WEB_FLOW_FINGERPRINT,
                revision,
            )
        policy.validate_commits(
            A01,
            CAD9,
            mode="protected-main",
            event="push",
            ref="refs/heads/main",
        )
        for base, head in ((MERGE_0C_BASE, MERGE_0C), (MERGE_0C, A01)):
            with self.assertRaisesRegex(ValueError, "matching Signed-off-by"):
                policy.validate_commits(
                    base,
                    head,
                    mode="protected-main",
                    event="push",
                    ref="refs/heads/main",
                )

    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="asb-signature-policy-")
        self.root = Path(self.temporary.name)
        run("git", "init", "-q", "-b", "main", cwd=self.root)
        self.ssh_key = self.root / "fixture-ssh"
        run(
            "ssh-keygen",
            "-q",
            "-t",
            "ed25519",
            "-N",
            "",
            "-f",
            str(self.ssh_key),
            cwd=self.root,
        )
        public = self.ssh_key.with_suffix(".pub").read_text(encoding="utf-8").split()
        self.allowed = self.root / "allowed_signers"
        self.allowed.write_text(
            f"fixture@example.invalid {public[0]} {public[1]}\n", encoding="utf-8"
        )
        run("git", "config", "user.name", "Fixture", cwd=self.root)
        run("git", "config", "user.email", "fixture@example.invalid", cwd=self.root)
        run("git", "config", "gpg.format", "ssh", cwd=self.root)
        run("git", "config", "user.signingkey", str(self.ssh_key), cwd=self.root)
        run("git", "config", "commit.gpgsign", "true", cwd=self.root)
        (self.root / "base").write_text("base\n", encoding="utf-8")
        run("git", "add", "base", cwd=self.root)
        self.ssh_commit("base")
        self.base = run("git", "rev-parse", "HEAD", cwd=self.root)
        run("git", "checkout", "-q", "-b", "topic", cwd=self.root)
        (self.root / "topic").write_text("topic\n", encoding="utf-8")
        run("git", "add", "topic", cwd=self.root)
        self.ssh_commit("topic")
        self.topic = run("git", "rev-parse", "HEAD", cwd=self.root)
        run("git", "checkout", "-q", "main", cwd=self.root)
        self.gpg_home = self.root / "gnupg"
        self.gpg_home.mkdir(mode=0o700)
        self.gpg_env = os.environ.copy()
        self.gpg_env["GNUPGHOME"] = str(self.gpg_home)
        self.fingerprint = self.make_gpg_key("Fixture Web Flow <noreply@github.com>")
        self.web_key = self.root / "web-flow.gpg"
        exported = subprocess.check_output(
            ["gpg", "--batch", "--armor", "--export", self.fingerprint],
            env=self.gpg_env,
        )
        self.web_key.write_bytes(exported)
        self.web_key_sha256 = hashlib.sha256(exported).hexdigest()
        self.merge = self.gpg_merge(
            "GitHub", "noreply@github.com", "Web Author", "web@example.invalid"
        )

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def make_gpg_key(self, identity: str) -> str:
        run(
            "gpg",
            "--batch",
            "--passphrase",
            "",
            "--quick-generate-key",
            identity,
            "rsa2048",
            "sign",
            "0",
            cwd=self.root,
            env=self.gpg_env,
        )
        listing = run(
            "gpg",
            "--batch",
            "--with-colons",
            "--fingerprint",
            identity,
            cwd=self.root,
            env=self.gpg_env,
        )
        return next(
            line.split(":")[9]
            for line in listing.splitlines()
            if line.startswith("fpr:")
        )

    def ssh_commit(self, subject: str) -> None:
        run(
            "git",
            "commit",
            "-q",
            "-m",
            f"{subject}\n\nSigned-off-by: Fixture <fixture@example.invalid>",
            cwd=self.root,
        )

    def gpg_merge(
        self,
        committer_name: str,
        committer_email: str,
        author_name: str,
        author_email: str,
        trailer_name: str | None = None,
    ) -> str:
        environment = self.gpg_env | {
            "GIT_AUTHOR_NAME": author_name,
            "GIT_AUTHOR_EMAIL": author_email,
            "GIT_COMMITTER_NAME": committer_name,
            "GIT_COMMITTER_EMAIL": committer_email,
        }
        signer = trailer_name or author_name
        run(
            "git",
            "-c",
            "gpg.format=openpgp",
            "-c",
            f"user.signingkey={self.fingerprint}",
            "merge",
            "--no-ff",
            "-S",
            "topic",
            "-m",
            f"merge fixture\n\nSigned-off-by: {signer} <{author_email}>",
            cwd=self.root,
            env=environment,
        )
        return run("git", "rev-parse", "HEAD", cwd=self.root)

    def validate(
        self, *, base: str | None = None, head: str | None = None, **changes: object
    ) -> None:
        arguments: dict[str, object] = {
            "mode": "protected-main",
            "event": "push",
            "ref": "refs/heads/main",
            "root": self.root,
            "allowed": self.allowed,
            "web_flow_key": self.web_key,
            "web_flow_key_sha256": self.web_key_sha256,
            "web_flow_fingerprint": self.fingerprint,
        }
        arguments.update(changes)
        policy.validate_commits(base or self.base, head or self.merge, **arguments)

    def test_valid_fixture_and_default_mode_is_ssh_only(self) -> None:
        self.validate()
        policy.validate_commits(
            self.base,
            self.topic,
            mode="ssh-only",
            event="pull_request",
            ref="refs/pull/1/merge",
            root=self.root,
            allowed=self.allowed,
        )
        with self.assertRaisesRegex(ValueError, "allowed SSH signature"):
            policy.validate_commits(
                self.base,
                self.merge,
                root=self.root,
                allowed=self.allowed,
            )

    def test_context_key_topology_and_parent_fail_closed(self) -> None:
        for changes in (
            {"event": "pull_request"},
            {"event": "workflow_dispatch"},
            {"ref": "refs/heads/topic"},
        ):
            with self.assertRaisesRegex(
                ValueError, "canonical push event and main ref"
            ):
                self.validate(**changes)
        with self.assertRaisesRegex(ValueError, "key digest differs"):
            self.validate(web_flow_key_sha256="0" * 64)
        with self.assertRaisesRegex(ValueError, "topology or first parent differs"):
            self.validate(base=self.topic)
        with self.assertRaisesRegex(ValueError, "final two-parent merge"):
            self.validate(head=self.topic)
        with self.assertRaisesRegex(ValueError, "immutable range base"):
            policy.validate_commits(
                None,
                self.merge,
                mode="protected-main",
                event="push",
                ref="refs/heads/main",
                root=self.root,
                allowed=self.allowed,
                web_flow_key=self.web_key,
                web_flow_key_sha256=self.web_key_sha256,
                web_flow_fingerprint=self.fingerprint,
            )
        with self.assertRaisesRegex(ValueError, "full lowercase SHA-1"):
            policy.validate_commits(
                self.base[:12],
                self.merge,
                mode="protected-main",
                event="push",
                ref="refs/heads/main",
                root=self.root,
                allowed=self.allowed,
                web_flow_key=self.web_key,
                web_flow_key_sha256=self.web_key_sha256,
                web_flow_fingerprint=self.fingerprint,
            )

    def test_wrong_committer_and_dco_fail_closed(self) -> None:
        run("git", "reset", "--hard", "-q", self.base, cwd=self.root)
        wrong_committer = self.gpg_merge(
            "Not GitHub", "noreply@github.com", "Web Author", "web@example.invalid"
        )
        with self.assertRaisesRegex(ValueError, "committer"):
            self.validate(head=wrong_committer)
        run("git", "reset", "--hard", "-q", self.base, cwd=self.root)
        wrong_dco = self.gpg_merge(
            "GitHub",
            "noreply@github.com",
            "Web Author",
            "web@example.invalid",
            trailer_name="Different Author",
        )
        with self.assertRaisesRegex(ValueError, "matching Signed-off-by"):
            self.validate(head=wrong_dco)

    def test_wrong_signer_and_mixed_range_fail_closed(self) -> None:
        other_fingerprint = self.make_gpg_key("Substitute <substitute@example.invalid>")
        other_key = self.root / "other.gpg"
        other_bytes = subprocess.check_output(
            ["gpg", "--batch", "--armor", "--export", other_fingerprint],
            env=self.gpg_env,
        )
        other_key.write_bytes(other_bytes)
        with self.assertRaisesRegex(ValueError, "pinned GitHub Web Flow signature"):
            self.validate(
                web_flow_key=other_key,
                web_flow_key_sha256=hashlib.sha256(other_bytes).hexdigest(),
            )
        run("git", "checkout", "-q", "-b", "after-merge", self.merge, cwd=self.root)
        (self.root / "after").write_text("after\n", encoding="utf-8")
        run("git", "add", "after", cwd=self.root)
        self.ssh_commit("after")
        mixed_head = run("git", "rev-parse", "HEAD", cwd=self.root)
        with self.assertRaisesRegex(ValueError, "final two-parent merge"):
            self.validate(head=mixed_head)

    def test_unsigned_protected_merge_fails_closed(self) -> None:
        run("git", "reset", "--hard", "-q", self.base, cwd=self.root)
        environment = os.environ.copy() | {
            "GIT_AUTHOR_NAME": "Web Author",
            "GIT_AUTHOR_EMAIL": "web@example.invalid",
            "GIT_COMMITTER_NAME": "GitHub",
            "GIT_COMMITTER_EMAIL": "noreply@github.com",
        }
        run(
            "git",
            "-c",
            "commit.gpgsign=false",
            "merge",
            "--no-ff",
            "topic",
            "-m",
            "unsigned fixture\n\nSigned-off-by: Web Author <web@example.invalid>",
            cwd=self.root,
            env=environment,
        )
        unsigned = run("git", "rev-parse", "HEAD", cwd=self.root)
        with self.assertRaisesRegex(ValueError, "pinned GitHub Web Flow signature"):
            self.validate(head=unsigned)

    def test_octopus_merge_fails_closed(self) -> None:
        run("git", "checkout", "-q", "-b", "extra", self.base, cwd=self.root)
        (self.root / "extra").write_text("extra\n", encoding="utf-8")
        run("git", "add", "extra", cwd=self.root)
        self.ssh_commit("extra")
        run("git", "checkout", "-q", "main", cwd=self.root)
        run("git", "reset", "--hard", "-q", self.base, cwd=self.root)
        environment = self.gpg_env | {
            "GIT_AUTHOR_NAME": "Web Author",
            "GIT_AUTHOR_EMAIL": "web@example.invalid",
            "GIT_COMMITTER_NAME": "GitHub",
            "GIT_COMMITTER_EMAIL": "noreply@github.com",
        }
        run(
            "git",
            "-c",
            "gpg.format=openpgp",
            "-c",
            f"user.signingkey={self.fingerprint}",
            "merge",
            "--no-ff",
            "-S",
            "topic",
            "extra",
            "-m",
            "octopus fixture\n\nSigned-off-by: Web Author <web@example.invalid>",
            cwd=self.root,
            env=environment,
        )
        octopus = run("git", "rev-parse", "HEAD", cwd=self.root)
        self.assertEqual(len(policy.commit_parents(self.root, octopus)), 3)
        with self.assertRaisesRegex(ValueError, "merge topology"):
            self.validate(head=octopus)


if __name__ == "__main__":
    unittest.main()
