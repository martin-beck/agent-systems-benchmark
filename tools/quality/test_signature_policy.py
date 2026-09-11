#!/usr/bin/env python3
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Adversarial tests for the offline commit-signature trust boundary."""

from __future__ import annotations

import hashlib
import json
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
PR132_BASE = "6155d63bec04a5c76c4323843c26649b0c084f6e"
PR132_TOPIC = "297895dbdee6acfc2a7425c5a9ab254c6d2cce96"
PR132_MERGE = "44eb1b48cb79b789252eff1cc798980d868c908c"
PR132_TREE = "1cde5c478dd35aaa7b68e56e4b54254d4f3a401d"
PR132_ATTESTATION = (
    policy.ROOT / "docs/attestations/capability-coverage-pr132-merge.json"
)


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

    def test_pr132_exact_topic_sync_recovery(self) -> None:
        attestation = json.loads(PR132_ATTESTATION.read_text(encoding="utf-8"))
        self.assertEqual(
            attestation,
            {
                "schema_version": 1,
                "pull_request": 132,
                "reviewed_base_commit": PR132_BASE,
                "reviewed_head_commit": PR132_TOPIC,
                "reviewed_head_tree": PR132_TREE,
                "reviewed_head_signature": "valid_allowed_ssh",
                "reviewed_head_dco": True,
                "exact_head_checks": {"expected": 12, "successful": 12},
                "merge_commit": PR132_MERGE,
                "merge_parents": [PR132_BASE, PR132_TOPIC],
                "merge_tree": PR132_TREE,
                "merge_signature": {
                    "verified": True,
                    "reason": "valid",
                    "signer": "github_web_flow",
                    "verified_at": "2026-09-11T00:54:41Z",
                },
                "merge_author": "martin-beck <martin.beck2@gmx.de>",
                "raw_signed_off_by": (
                    "Signed-off-by: martin-beck <martin.beck2@gmx.de>"
                ),
                "merge_commit_dco": True,
                "publication_method": "merge",
                "post_merge_repository_quality": {
                    "run": 34548482976,
                    "conclusion": "failure",
                    "error": (
                        "protected-main range must contain one final two-parent merge"
                    ),
                },
                "classification": (
                    "valid_publication_rejected_by_pre_recovery_topic_topology_policy"
                ),
                "recovery_task": "AR-1043",
            },
        )
        self.assertEqual(
            policy.commit_parents(policy.ROOT, PR132_MERGE),
            [PR132_BASE, PR132_TOPIC],
        )
        self.assertEqual(policy.commit_parents(policy.ROOT, PR132_TOPIC)[1], PR132_BASE)
        self.assertEqual(policy.commit_tree(policy.ROOT, PR132_TOPIC), PR132_TREE)
        self.assertEqual(policy.commit_tree(policy.ROOT, PR132_MERGE), PR132_TREE)
        policy.validate_commits(
            PR132_BASE,
            PR132_MERGE,
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
        topic_ref: str = "topic",
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
            topic_ref,
            "-m",
            f"merge fixture\n\nSigned-off-by: {signer} <{author_email}>",
            cwd=self.root,
            env=environment,
        )
        return run("git", "rev-parse", "HEAD", cwd=self.root)

    def gpg_commit_tree(self, tree: str, parents: list[str]) -> str:
        environment = self.gpg_env | {
            "GIT_AUTHOR_NAME": "Web Author",
            "GIT_AUTHOR_EMAIL": "web@example.invalid",
            "GIT_COMMITTER_NAME": "GitHub",
            "GIT_COMMITTER_EMAIL": "noreply@github.com",
        }
        command = [
            "git",
            "-c",
            "gpg.format=openpgp",
            "-c",
            f"user.signingkey={self.fingerprint}",
            "commit-tree",
            "-S",
            tree,
        ]
        for parent in parents:
            command.extend(["-p", parent])
        completed = subprocess.run(
            command,
            cwd=self.root,
            env=environment,
            input=(
                "merge fixture\n\nSigned-off-by: Web Author <web@example.invalid>\n"
            ),
            text=True,
            capture_output=True,
            check=True,
        )
        return completed.stdout.strip()

    def ssh_commit_tree(
        self, tree: str, parents: list[str], subject: str, *, dco: bool = True
    ) -> str:
        message = subject
        if dco:
            message += "\n\nSigned-off-by: Fixture <fixture@example.invalid>"
        command = [
            "git",
            "-c",
            "gpg.format=ssh",
            "-c",
            f"user.signingkey={self.ssh_key}",
            "commit-tree",
            "-S",
            tree,
        ]
        for parent in parents:
            command.extend(["-p", parent])
        completed = subprocess.run(
            command,
            cwd=self.root,
            input=f"{message}\n",
            text=True,
            capture_output=True,
            check=True,
        )
        return completed.stdout.strip()

    def ssh_merge(
        self,
        revision: str,
        subject: str,
        *,
        signed: bool = True,
        dco: bool = True,
    ) -> str:
        message = subject
        if dco:
            message += "\n\nSigned-off-by: Fixture <fixture@example.invalid>"
        arguments = ["git"]
        if not signed:
            arguments.extend(["-c", "commit.gpgsign=false"])
        arguments.extend(["merge", "--no-ff"])
        if signed:
            arguments.append("-S")
        arguments.extend([revision, "-m", message])
        run(*arguments, cwd=self.root)
        return run("git", "rev-parse", "HEAD", cwd=self.root)

    def sync_fixture(
        self,
        *,
        signed: bool = True,
        dco: bool = True,
        sync_ref: str | None = None,
        source_topic: str | None = None,
    ) -> tuple[str, str, str]:
        run("git", "checkout", "-q", "-B", "advanced-main", self.base, cwd=self.root)
        (self.root / "advanced").write_text("advanced\n", encoding="utf-8")
        run("git", "add", "advanced", cwd=self.root)
        self.ssh_commit("advance main")
        advanced_base = run("git", "rev-parse", "HEAD", cwd=self.root)
        run(
            "git",
            "checkout",
            "-q",
            "-B",
            "sync-topic",
            source_topic or self.topic,
            cwd=self.root,
        )
        sync_tip = self.ssh_merge(
            sync_ref or advanced_base,
            "synchronize protected main",
            signed=signed,
            dco=dco,
        )
        run("git", "checkout", "-q", "advanced-main", cwd=self.root)
        final_merge = self.gpg_merge(
            "GitHub",
            "noreply@github.com",
            "Web Author",
            "web@example.invalid",
            topic_ref="sync-topic",
        )
        return advanced_base, sync_tip, final_merge

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

    def test_valid_topic_tip_sync_fixture(self) -> None:
        advanced_base, sync_tip, final_merge = self.sync_fixture()
        self.assertEqual(policy.commit_parents(self.root, sync_tip)[1], advanced_base)
        self.assertEqual(
            policy.commit_tree(self.root, sync_tip),
            policy.commit_tree(self.root, final_merge),
        )
        self.validate(base=advanced_base, head=final_merge)

    def test_valid_topic_tip_sync_with_one_historical_checkpoint(self) -> None:
        run("git", "checkout", "-q", "-B", "checkpoint-main", self.base, cwd=self.root)
        (self.root / "checkpoint").write_text("checkpoint\n", encoding="utf-8")
        run("git", "add", "checkpoint", cwd=self.root)
        self.ssh_commit("checkpoint main")
        checkpoint = run("git", "rev-parse", "HEAD", cwd=self.root)
        run(
            "git", "checkout", "-q", "-B", "historical-topic", self.topic, cwd=self.root
        )
        self.ssh_merge(checkpoint, "historical sync")
        historical_tip = run("git", "rev-parse", "HEAD", cwd=self.root)
        run("git", "checkout", "-q", "-B", "advanced-main", checkpoint, cwd=self.root)
        (self.root / "advanced").write_text("advanced\n", encoding="utf-8")
        run("git", "add", "advanced", cwd=self.root)
        self.ssh_commit("advance main")
        advanced_base = run("git", "rev-parse", "HEAD", cwd=self.root)
        run("git", "checkout", "-q", "-B", "sync-topic", historical_tip, cwd=self.root)
        self.ssh_merge(advanced_base, "topic-tip sync")
        run("git", "checkout", "-q", "advanced-main", cwd=self.root)
        final_merge = self.gpg_merge(
            "GitHub",
            "noreply@github.com",
            "Web Author",
            "web@example.invalid",
            topic_ref="sync-topic",
        )
        self.validate(base=advanced_base, head=final_merge)

    def test_topic_sync_rejects_an_off_tip_sync(self) -> None:
        advanced_base, _, _ = self.sync_fixture()
        run("git", "checkout", "-q", "sync-topic", cwd=self.root)
        (self.root / "after-sync").write_text("after\n", encoding="utf-8")
        run("git", "add", "after-sync", cwd=self.root)
        self.ssh_commit("commit after sync")
        run("git", "checkout", "-q", "advanced-main", cwd=self.root)
        run("git", "reset", "--hard", "-q", advanced_base, cwd=self.root)
        off_tip = self.gpg_merge(
            "GitHub",
            "noreply@github.com",
            "Web Author",
            "web@example.invalid",
            topic_ref="sync-topic",
        )
        with self.assertRaisesRegex(ValueError, "must be at the tip"):
            self.validate(base=advanced_base, head=off_tip)

    def test_topic_sync_rejects_nested_multiple_and_octopus_merges(self) -> None:
        run("git", "checkout", "-q", "-B", "side", self.base, cwd=self.root)
        (self.root / "side").write_text("side\n", encoding="utf-8")
        run("git", "add", "side", cwd=self.root)
        self.ssh_commit("side")
        side = run("git", "rev-parse", "HEAD", cwd=self.root)
        run("git", "checkout", "-q", "topic", cwd=self.root)
        self.ssh_merge(side, "nested topic merge")
        nested_topic = run("git", "rev-parse", "HEAD", cwd=self.root)
        advanced_base, _, multiple_final = self.sync_fixture(source_topic=nested_topic)
        with self.assertRaisesRegex(ValueError, "outside its first-parent spine"):
            self.validate(base=advanced_base, head=multiple_final)

        run("git", "checkout", "-q", "-B", "advanced-main", self.base, cwd=self.root)
        (self.root / "advanced").write_text("advanced\n", encoding="utf-8")
        run("git", "add", "advanced", cwd=self.root)
        self.ssh_commit("advance main")
        advanced_base = run("git", "rev-parse", "HEAD", cwd=self.root)
        run("git", "checkout", "-q", "-B", "octopus-topic", self.topic, cwd=self.root)
        run(
            "git",
            "merge",
            "--no-ff",
            "-S",
            advanced_base,
            side,
            "-m",
            "octopus sync\n\nSigned-off-by: Fixture <fixture@example.invalid>",
            cwd=self.root,
        )
        octopus_tip = run("git", "rev-parse", "HEAD", cwd=self.root)
        self.assertGreaterEqual(len(policy.commit_parents(self.root, octopus_tip)), 3)
        run("git", "checkout", "-q", "advanced-main", cwd=self.root)
        octopus_final = self.gpg_merge(
            "GitHub",
            "noreply@github.com",
            "Web Author",
            "web@example.invalid",
            topic_ref="octopus-topic",
        )
        with self.assertRaisesRegex(ValueError, "outside its first-parent spine"):
            self.validate(base=advanced_base, head=octopus_final)

    def test_topic_sync_rejects_two_historical_checkpoints(self) -> None:
        run("git", "checkout", "-q", "-B", "checkpoint-main", self.base, cwd=self.root)
        (self.root / "checkpoint-one").write_text("one\n", encoding="utf-8")
        run("git", "add", "checkpoint-one", cwd=self.root)
        self.ssh_commit("first main checkpoint")
        checkpoint_one = run("git", "rev-parse", "HEAD", cwd=self.root)
        run("git", "checkout", "-q", "-B", "many-syncs", self.topic, cwd=self.root)
        self.ssh_merge(checkpoint_one, "first historical sync")
        run("git", "checkout", "-q", "checkpoint-main", cwd=self.root)
        (self.root / "checkpoint-two").write_text("two\n", encoding="utf-8")
        run("git", "add", "checkpoint-two", cwd=self.root)
        self.ssh_commit("second main checkpoint")
        checkpoint_two = run("git", "rev-parse", "HEAD", cwd=self.root)
        run("git", "checkout", "-q", "many-syncs", cwd=self.root)
        self.ssh_merge(checkpoint_two, "second historical sync")
        run("git", "checkout", "-q", "checkpoint-main", cwd=self.root)
        (self.root / "advanced").write_text("advanced\n", encoding="utf-8")
        run("git", "add", "advanced", cwd=self.root)
        self.ssh_commit("advance main")
        advanced_base = run("git", "rev-parse", "HEAD", cwd=self.root)
        run("git", "checkout", "-q", "many-syncs", cwd=self.root)
        self.ssh_merge(advanced_base, "topic-tip sync")
        run("git", "checkout", "-q", "checkpoint-main", cwd=self.root)
        final_merge = self.gpg_merge(
            "GitHub",
            "noreply@github.com",
            "Web Author",
            "web@example.invalid",
            topic_ref="many-syncs",
        )
        with self.assertRaisesRegex(ValueError, "more than one historical"):
            self.validate(base=advanced_base, head=final_merge)

    def test_topic_sync_rejects_a_second_exact_base_sync(self) -> None:
        advanced_base, _, _ = self.sync_fixture()
        run("git", "checkout", "-q", "sync-topic", cwd=self.root)
        (self.root / "between-syncs").write_text("between\n", encoding="utf-8")
        run("git", "add", "between-syncs", cwd=self.root)
        self.ssh_commit("between syncs")
        first_parent = run("git", "rev-parse", "HEAD", cwd=self.root)
        repeated_sync = self.ssh_commit_tree(
            policy.commit_tree(self.root, first_parent),
            [first_parent, advanced_base],
            "second exact-base sync",
        )
        run("git", "reset", "--hard", "-q", repeated_sync, cwd=self.root)
        run("git", "checkout", "-q", "advanced-main", cwd=self.root)
        run("git", "reset", "--hard", "-q", advanced_base, cwd=self.root)
        final_merge = self.gpg_merge(
            "GitHub",
            "noreply@github.com",
            "Web Author",
            "web@example.invalid",
            topic_ref="sync-topic",
        )
        with self.assertRaisesRegex(ValueError, "redundantly merges"):
            self.validate(base=advanced_base, head=final_merge)

    def test_topic_sync_rejects_arbitrary_base_unsigned_and_non_dco(self) -> None:
        run("git", "checkout", "-q", "-B", "unrelated", self.base, cwd=self.root)
        (self.root / "unrelated").write_text("unrelated\n", encoding="utf-8")
        run("git", "add", "unrelated", cwd=self.root)
        self.ssh_commit("unrelated base")
        unrelated = run("git", "rev-parse", "HEAD", cwd=self.root)
        advanced_base, _, arbitrary_final = self.sync_fixture(sync_ref=unrelated)
        with self.assertRaisesRegex(ValueError, "outside its first-parent spine"):
            self.validate(base=advanced_base, head=arbitrary_final)

        for signed, dco, message in (
            (False, True, "allowed SSH signature"),
            (True, False, "matching Signed-off-by"),
        ):
            with self.subTest(signed=signed, dco=dco):
                advanced_base, _, final_merge = self.sync_fixture(
                    signed=signed, dco=dco
                )
                with self.assertRaisesRegex(ValueError, message):
                    self.validate(base=advanced_base, head=final_merge)

    def test_topic_sync_rejects_redundant_and_tree_changing_merges(self) -> None:
        run("git", "checkout", "-q", "-B", "advanced-main", self.base, cwd=self.root)
        (self.root / "advanced").write_text("advanced\n", encoding="utf-8")
        run("git", "add", "advanced", cwd=self.root)
        self.ssh_commit("advance main")
        advanced_base = run("git", "rev-parse", "HEAD", cwd=self.root)
        run(
            "git",
            "checkout",
            "-q",
            "-B",
            "redundant-topic",
            advanced_base,
            cwd=self.root,
        )
        (self.root / "redundant").write_text("redundant\n", encoding="utf-8")
        run("git", "add", "redundant", cwd=self.root)
        self.ssh_commit("redundant topic")
        redundant_parent = run("git", "rev-parse", "HEAD", cwd=self.root)
        redundant_tree = policy.commit_tree(self.root, redundant_parent)
        redundant_tip = self.ssh_commit_tree(
            redundant_tree,
            [redundant_parent, advanced_base],
            "redundant sync",
        )
        run("git", "reset", "--hard", "-q", redundant_tip, cwd=self.root)
        run("git", "checkout", "-q", "advanced-main", cwd=self.root)
        redundant_final = self.gpg_merge(
            "GitHub",
            "noreply@github.com",
            "Web Author",
            "web@example.invalid",
            topic_ref="redundant-topic",
        )
        with self.assertRaisesRegex(ValueError, "redundantly merges"):
            self.validate(base=advanced_base, head=redundant_final)

        advanced_base, _, _ = self.sync_fixture()
        run("git", "checkout", "-q", "advanced-main", cwd=self.root)
        (self.root / "final-only").write_text(
            "changed by final merge\n", encoding="utf-8"
        )
        run("git", "add", "final-only", cwd=self.root)
        self.ssh_commit("temporary final tree source")
        changed_tree = run("git", "show", "-s", "--format=%T", "HEAD", cwd=self.root)
        run("git", "reset", "--hard", "-q", advanced_base, cwd=self.root)
        changed_final = self.gpg_commit_tree(
            changed_tree,
            [advanced_base, run("git", "rev-parse", "sync-topic", cwd=self.root)],
        )
        with self.assertRaisesRegex(ValueError, "reviewed topic tree"):
            self.validate(base=advanced_base, head=changed_final)

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
        with self.assertRaisesRegex(ValueError, "topology or first parent differs"):
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
        with self.assertRaisesRegex(ValueError, "topology or first parent differs"):
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
