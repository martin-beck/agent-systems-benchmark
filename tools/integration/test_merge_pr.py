#!/usr/bin/env python3
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Adversarial tests for the exact signed merge boundary."""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MERGE = ROOT / "tools/integration/merge_pr.py"
SETTINGS = ROOT / "tools/integration/repository_settings.py"


def command(
    cwd: Path,
    *args: str,
    check: bool = True,
    env: dict[str, str] | None = None,
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        args, cwd=cwd, text=True, capture_output=True, check=check, env=env
    )


class Fixture:
    def __init__(self, root: Path) -> None:
        self.root = root
        self.remote = root / "remote.git"
        self.repository = root / "work"
        self.key = root / "signing-key"
        command(
            root, "ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", str(self.key)
        )
        command(root, "git", "init", "-q", "--bare", str(self.remote))
        command(root, "git", "init", "-q", "-b", "main", str(self.repository))
        command(self.repository, "git", "config", "user.name", "Fixture")
        command(
            self.repository, "git", "config", "user.email", "fixture@example.invalid"
        )
        command(self.repository, "git", "config", "gpg.format", "ssh")
        command(self.repository, "git", "config", "user.signingkey", str(self.key))
        command(self.repository, "git", "config", "commit.gpgsign", "true")
        command(self.repository, "git", "remote", "add", "origin", str(self.remote))
        (self.repository / "config").mkdir()
        public_key = self.key.with_suffix(".pub").read_text(encoding="utf-8").strip()
        (self.repository / "config/allowed_signers").write_text(
            f'fixture@example.invalid namespaces="git" {public_key}\n', encoding="utf-8"
        )
        quality = self.repository / "tools/quality"
        quality.mkdir(parents=True)
        shutil.copy2(ROOT / "tools/quality/check_dco.py", quality / "check_dco.py")
        (self.repository / "base").write_text("base\n", encoding="utf-8")
        self.commit("base")
        self.base = self.oid("HEAD")
        command(self.repository, "git", "push", "-q", "origin", "main")
        command(self.repository, "git", "checkout", "-qb", "feature")
        (self.repository / "feature").write_text("feature\n", encoding="utf-8")
        self.commit("feature")
        self.head = self.oid("HEAD")
        self.tree = self.oid("HEAD^{tree}")
        command(self.repository, "git", "push", "-q", "origin", "HEAD:refs/pull/7/head")
        command(self.repository, "git", "checkout", "-q", "main")

    def commit(self, subject: str, *, signed: bool = True, dco: bool = True) -> None:
        command(self.repository, "git", "add", ".")
        args = ["git"]
        if not signed:
            args.extend(["-c", "commit.gpgsign=false"])
        args.extend(["commit", "-qm", subject])
        if signed:
            args.insert(-2, "-S")
        if dco:
            args.insert(-2, "-s")
        command(self.repository, *args)

    def oid(self, revision: str) -> str:
        return command(self.repository, "git", "rev-parse", revision).stdout.strip()

    def merge_command(self, *extra: str) -> list[str]:
        return [
            "python3",
            str(MERGE),
            "--root",
            str(self.repository),
            "--pr-ref",
            "refs/pull/7/head",
            "--expected-base",
            self.base,
            "--expected-head",
            self.head,
            "--expected-tree",
            self.tree,
            "--subject",
            "Merge reviewed PR #7",
            *extra,
        ]

    def fake_git_environment(
        self, mode: str, drift_oid: str | None = None
    ) -> dict[str, str]:
        fake_bin = self.root / f"fake-git-{mode}"
        fake_bin.mkdir()
        script = fake_bin / "git"
        script.write_text(
            "#!/usr/bin/env python3\n"
            "import os, subprocess, sys\n"
            "real = os.environ['ASB_REAL_GIT']\n"
            "args = sys.argv[1:]\n"
            "mode = os.environ['ASB_FAKE_GIT_MODE']\n"
            "if args and args[0] == 'ls-remote' and mode == 'private-diagnostic':\n"
            "    sys.stderr.write(os.environ['ASB_PRIVATE_SENTINEL'] * 70000)\n"
            "    raise SystemExit(1)\n"
            "if args and args[0] == 'push':\n"
            "    if mode == 'target-race':\n"
            "        subprocess.run([real, '--git-dir', os.environ['ASB_REMOTE'], "
            "'update-ref', 'refs/heads/main', os.environ['ASB_DRIFT_OID']], check=True)\n"
            "    result = subprocess.run([real, *args])\n"
            "    if mode == 'pr-drift':\n"
            "        subprocess.run([real, '--git-dir', os.environ['ASB_REMOTE'], "
            "'update-ref', 'refs/pull/7/head', os.environ['ASB_DRIFT_OID']], check=True)\n"
            "    if mode == 'accepted-error':\n"
            "        raise SystemExit(1)\n"
            "    raise SystemExit(result.returncode)\n"
            "os.execv(real, [real, *args])\n",
            encoding="utf-8",
        )
        script.chmod(0o755)
        environment = os.environ.copy()
        environment.update(
            {
                "PATH": f"{fake_bin}:{environment['PATH']}",
                "ASB_REAL_GIT": shutil.which("git", path=os.environ["PATH"]) or "git",
                "ASB_FAKE_GIT_MODE": mode,
                "ASB_REMOTE": str(self.remote),
                "ASB_DRIFT_OID": drift_oid or self.base,
                "ASB_PRIVATE_SENTINEL": "private-user-machine-path-secret-token",
            }
        )
        return environment


class MergeIntegrityTests(unittest.TestCase):
    def test_constructs_and_publishes_exact_signed_dco_merge(self) -> None:
        with tempfile.TemporaryDirectory(prefix="asb-merge-positive-") as raw:
            fixture = Fixture(Path(raw))
            result = command(fixture.repository, *fixture.merge_command("--push"))
            values = dict(line.split("=", 1) for line in result.stdout.splitlines())
            merge = values["MERGE"]
            self.assertEqual(values["PUBLISHED"], "1")
            self.assertEqual(
                command(
                    fixture.repository, "git", "show", "-s", "--format=%P", merge
                ).stdout.strip(),
                f"{fixture.base} {fixture.head}",
            )
            self.assertEqual(fixture.oid(f"{merge}^{{tree}}"), fixture.tree)
            self.assertIn(
                "Signed-off-by: Fixture <fixture@example.invalid>",
                command(
                    fixture.repository, "git", "show", "-s", "--format=%B", merge
                ).stdout,
            )
            command(
                fixture.repository,
                "git",
                "-c",
                "gpg.format=ssh",
                "-c",
                f"gpg.ssh.allowedSignersFile={fixture.repository / 'config/allowed_signers'}",
                "verify-commit",
                merge,
            )

    def test_rejects_stale_base_wrong_head_and_wrong_tree(self) -> None:
        with tempfile.TemporaryDirectory(prefix="asb-merge-identities-") as raw:
            fixture = Fixture(Path(raw))
            cases = (
                ("--expected-base", "0" * 40),
                ("--expected-head", "1" * 40),
                ("--expected-tree", "2" * 40),
            )
            for option, value in cases:
                args = fixture.merge_command()
                args[args.index(option) + 1] = value
                self.assertNotEqual(
                    command(fixture.repository, *args, check=False).returncode, 0
                )

    def test_rejects_unsigned_or_non_dco_feature_commit(self) -> None:
        for signed, dco in ((False, True), (True, False)):
            with (
                self.subTest(signed=signed, dco=dco),
                tempfile.TemporaryDirectory(prefix="asb-merge-policy-") as raw,
            ):
                fixture = Fixture(Path(raw))
                command(
                    fixture.repository,
                    "git",
                    "checkout",
                    "-qb",
                    "bad-candidate",
                    fixture.base,
                )
                (fixture.repository / "bad").write_text("bad\n", encoding="utf-8")
                fixture.commit("bad", signed=signed, dco=dco)
                fixture.head = fixture.oid("HEAD")
                fixture.tree = fixture.oid("HEAD^{tree}")
                command(
                    fixture.repository,
                    "git",
                    "push",
                    "-q",
                    "--force",
                    "origin",
                    "HEAD:refs/pull/7/head",
                )
                command(fixture.repository, "git", "checkout", "-q", "main")
                self.assertNotEqual(
                    command(
                        fixture.repository, *fixture.merge_command(), check=False
                    ).returncode,
                    0,
                )

    def test_rejects_signer_identity_spoof(self) -> None:
        with tempfile.TemporaryDirectory(prefix="asb-merge-identity-") as raw:
            fixture = Fixture(Path(raw))
            command(fixture.repository, "git", "checkout", "-qb", "spoof", fixture.base)
            (fixture.repository / "spoof").write_text("spoof\n", encoding="utf-8")
            command(fixture.repository, "git", "add", ".")
            command(
                fixture.repository,
                "git",
                "-c",
                "user.name=Victim",
                "-c",
                "user.email=victim@example.invalid",
                "commit",
                "-S",
                "-s",
                "-qm",
                "spoofed identity",
            )
            fixture.head = fixture.oid("HEAD")
            fixture.tree = fixture.oid("HEAD^{tree}")
            command(
                fixture.repository,
                "git",
                "push",
                "-q",
                "--force",
                "origin",
                "HEAD:refs/pull/7/head",
            )
            command(fixture.repository, "git", "checkout", "-q", "main")
            result = command(fixture.repository, *fixture.merge_command(), check=False)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("signer principal", result.stderr)
            self.assertNotIn("Victim", result.stderr)

    def test_rejects_github_generated_merge_head(self) -> None:
        with tempfile.TemporaryDirectory(prefix="asb-merge-github-") as raw:
            fixture = Fixture(Path(raw))
            github_merge = command(
                fixture.repository,
                "git",
                "-c",
                "commit.gpgsign=false",
                "commit-tree",
                "-p",
                fixture.base,
                "-p",
                fixture.head,
                fixture.tree,
                check=True,
            ).stdout.strip()
            command(
                fixture.repository,
                "git",
                "push",
                "-q",
                "--force",
                "origin",
                f"{github_merge}:refs/pull/7/head",
            )
            fixture.head = github_merge
            result = command(fixture.repository, *fixture.merge_command(), check=False)
            self.assertNotEqual(result.returncode, 0)

    def test_reconciles_accepted_push_error(self) -> None:
        with tempfile.TemporaryDirectory(prefix="asb-merge-accepted-error-") as raw:
            fixture = Fixture(Path(raw))
            result = command(
                fixture.repository,
                *fixture.merge_command("--push"),
                env=fixture.fake_git_environment("accepted-error"),
            )
            values = dict(line.split("=", 1) for line in result.stdout.splitlines())
            self.assertEqual(values["PUBLISHED"], "1")
            self.assertEqual(values["PUBLICATION"], "accepted-after-error")
            self.assertEqual(
                command(
                    fixture.repository,
                    "git",
                    "--git-dir",
                    str(fixture.remote),
                    "rev-parse",
                    "refs/heads/main",
                ).stdout.strip(),
                values["MERGE"],
            )

    def test_fails_closed_on_pr_or_target_drift_during_push(self) -> None:
        for mode, drift in (("pr-drift", None), ("target-race", "head")):
            with (
                self.subTest(mode=mode),
                tempfile.TemporaryDirectory(prefix="asb-merge-atomic-") as raw,
            ):
                fixture = Fixture(Path(raw))
                drift_oid = fixture.head if drift == "head" else fixture.base
                result = command(
                    fixture.repository,
                    *fixture.merge_command("--push"),
                    check=False,
                    env=fixture.fake_git_environment(mode, drift_oid),
                )
                self.assertNotEqual(result.returncode, 0)
                self.assertLess(len(result.stderr), 256)

    def test_diagnostics_are_bounded_generic_and_private(self) -> None:
        with tempfile.TemporaryDirectory(prefix="asb-merge-private-") as raw:
            fixture = Fixture(Path(raw))
            sentinel = "private-user-machine-path-secret-token"
            result = command(
                fixture.repository,
                *fixture.merge_command(),
                check=False,
                env=fixture.fake_git_environment("private-diagnostic"),
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertNotIn(sentinel, result.stderr)
            self.assertNotIn(str(fixture.remote), result.stderr)
            self.assertLess(len(result.stderr), 256)

    def test_settings_oracle_rejects_every_web_merge_mode(self) -> None:
        good = {
            "allow_merge_commit": False,
            "allow_squash_merge": False,
            "allow_rebase_merge": False,
            "allow_auto_merge": False,
            "web_commit_signoff_required": True,
        }
        result = command(
            ROOT, "python3", str(SETTINGS), "--settings-json", json.dumps(good)
        )
        self.assertIn("incompatible web merge modes are disabled", result.stdout)
        for field, value in good.items():
            mutated = dict(good)
            mutated[field] = not value
            self.assertNotEqual(
                command(
                    ROOT,
                    "python3",
                    str(SETTINGS),
                    "--settings-json",
                    json.dumps(mutated),
                    check=False,
                ).returncode,
                0,
            )


if __name__ == "__main__":
    unittest.main()
