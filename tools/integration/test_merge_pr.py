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
from unittest import mock

from tools.integration import merge_pr, portable_provenance, repository_settings
from tools.quality import repository_policy

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
            "if args and args[0] == 'fetch' and mode == 'fetch-target-race':\n"
            "    subprocess.run([real, '--git-dir', os.environ['ASB_REMOTE'],\n"
            "    'update-ref', 'refs/heads/main', os.environ['ASB_DRIFT_OID']], check=True)\n"
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
    def test_preview_binds_reviewed_tree_to_exact_parent_pair(self) -> None:
        with tempfile.TemporaryDirectory(prefix="asb-merge-preview-") as raw:
            fixture = Fixture(Path(raw))
            merge_pr.verify_merge_preview(
                fixture.repository, fixture.base, fixture.head, fixture.tree
            )
            with self.assertRaisesRegex(ValueError, "merge preview"):
                merge_pr.verify_merge_preview(
                    fixture.repository, fixture.base, fixture.head, "0" * 40
                )

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

    def test_rejects_remote_target_advanced_after_review(self) -> None:
        """A reviewed base must still be the protected target at admission."""
        with tempfile.TemporaryDirectory(prefix="asb-merge-stale-target-") as raw:
            fixture = Fixture(Path(raw))
            command(
                fixture.repository,
                "git",
                "checkout",
                "-qb",
                "advanced-target",
                fixture.base,
            )
            (fixture.repository / "target-advance").write_text(
                "protected target advanced\n", encoding="utf-8"
            )
            fixture.commit("advance protected target")
            command(
                fixture.repository,
                "git",
                "push",
                "-q",
                "origin",
                "HEAD:refs/heads/main",
            )
            command(fixture.repository, "git", "checkout", "-q", "main")

            result = command(fixture.repository, *fixture.merge_command(), check=False)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("fresh exact-main qualification required", result.stderr)

    def test_requalifies_target_after_fetch_before_constructing_merge(self) -> None:
        """Main advancing during object refresh must invalidate the qualification."""
        with tempfile.TemporaryDirectory(prefix="asb-merge-fetch-race-") as raw:
            fixture = Fixture(Path(raw))
            result = command(
                fixture.repository,
                *fixture.merge_command(),
                check=False,
                env=fixture.fake_git_environment("fetch-target-race", fixture.head),
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("fresh exact-main qualification required", result.stderr)

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
        for mode, drift in (
            ("pr-drift", None),
            ("target-race", "head"),
        ):
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

    def test_main_post_merge_workflows_queue_exact_pushes(self) -> None:
        """A later main push must not cancel evidence for an earlier merge."""
        workflows = (
            "quality.yml",
            "verify.yml",
            "formal.yml",
            "fault-assurance.yml",
            "emulated-aarch64.yml",
            "license-headers.yml",
            "native-platforms.yml",
        )
        required = "  cancel-in-progress: ${{ github.event_name != 'push' || github.ref != 'refs/heads/main' }}"
        for workflow in workflows:
            with self.subTest(workflow=workflow):
                text = (ROOT / ".github/workflows" / workflow).read_text(
                    encoding="utf-8"
                )
                self.assertIn(required, text)

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

    def test_settings_oracle_requires_merge_only_and_exact_ruleset(self) -> None:
        good_settings = dict(repository_settings.REQUIRED_SETTINGS) | {
            "id": 17,
            "visibility": "public",
            "default_branch": "main",
            "owner": {"login": "owner", "type": "User"},
        }
        good_owner = {"login": "owner", "type": "User", "plan": {"name": "free"}}
        good_ruleset = repository_settings.required_ruleset()

        def audit(
            settings: object, ruleset: object
        ) -> subprocess.CompletedProcess[str]:
            return command(
                ROOT,
                "python3",
                str(SETTINGS),
                "--settings-json",
                json.dumps(settings),
                "--ruleset-json",
                json.dumps(ruleset),
                "--owner-json",
                json.dumps(good_owner),
                "--provenance-status",
                "success",
                "--repository",
                "owner/repository",
                check=False,
            )

        result = audit(good_settings, good_ruleset)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("protected publication is enforced", result.stdout)

        for field, value in repository_settings.REQUIRED_SETTINGS.items():
            mutated = dict(good_settings)
            mutated[field] = not value
            self.assertNotEqual(audit(mutated, good_ruleset).returncode, 0)
            missing = dict(good_settings)
            del missing[field]
            self.assertNotEqual(audit(missing, good_ruleset).returncode, 0)

        all_disabled = dict(good_settings)
        all_disabled["allow_merge_commit"] = False
        self.assertNotEqual(audit(all_disabled, good_ruleset).returncode, 0)

        mutations = []
        for mutate in (
            lambda value: value.update(enforcement="evaluate"),
            lambda value: value["conditions"]["ref_name"]["include"].clear(),
            lambda value: value["rules"][3]["parameters"].update(
                allowed_merge_methods=["squash"]
            ),
            lambda value: value["rules"][4]["parameters"].update(
                strict_required_status_checks_policy=False
            ),
            lambda value: value["rules"][4]["parameters"][
                "required_status_checks"
            ].pop(),
            lambda value: value["rules"][4]["parameters"]["required_status_checks"].pop(
                0
            ),
        ):
            candidate = json.loads(json.dumps(good_ruleset))
            mutate(candidate)
            mutations.append(candidate)
        for candidate in mutations:
            self.assertNotEqual(audit(good_settings, candidate).returncode, 0)

        self.assertNotEqual(audit(good_settings, None).returncode, 0)

    def test_settings_api_failures_are_typed_bounded_and_private(self) -> None:
        cases = (
            (401, "Bad credentials", "authentication", "rejected"),
            (
                403,
                "Resource not accessible by personal access token",
                "authorization",
                "rejected",
            ),
            (404, "Not Found", "not-found", "rejected"),
            (409, "Conflict", "conflict", "rejected"),
            (422, "Validation Failed", "validation", "rejected"),
            (500, "Internal Server Error", "server", "ambiguous"),
        )
        sentinel = "private-path-token-query-secret"
        for status, message, category, effect in cases:
            with self.subTest(status=status):
                response = merge_pr.CommandResult(
                    1,
                    json.dumps(
                        {
                            "message": message,
                            "status": str(status),
                            "errors": [{"code": "invalid", "private": sentinel}],
                            "documentation_url": f"https://example.invalid/?{sentinel}",
                        }
                    ),
                    f"gh: {message} (HTTP {status}) {sentinel}",
                )
                with (
                    mock.patch.object(
                        repository_settings, "bounded_command", return_value=response
                    ),
                    self.assertRaises(ValueError) as raised,
                ):
                    repository_settings.api(
                        f"repos/owner/name?token={sentinel}",
                        operation="update",
                        subject="repository-settings",
                    )
                diagnostic = str(raised.exception)
                self.assertIn(f"category={category}", diagnostic)
                self.assertIn(f"status={status}", diagnostic)
                self.assertIn(f"effect={effect}", diagnostic)
                self.assertIn(f"message={message}", diagnostic)
                self.assertIn("code=invalid", diagnostic)
                self.assertNotIn(sentinel, diagnostic)
                self.assertLess(len(diagnostic), 256)

        network = merge_pr.CommandResult(1, "", f"network failed {sentinel}")
        with (
            mock.patch.object(
                repository_settings, "bounded_command", return_value=network
            ),
            self.assertRaises(ValueError) as raised,
        ):
            repository_settings.api(
                "repos/owner/name",
                operation="read",
                subject="ruleset-index",
            )
        diagnostic = str(raised.exception)
        self.assertIn("category=transport", diagnostic)
        self.assertIn("status=unknown", diagnostic)
        self.assertIn("effect=ambiguous", diagnostic)
        self.assertNotIn(sentinel, diagnostic)

    def test_settings_api_redacts_unapproved_response_fields(self) -> None:
        sentinel = "credential-private-host-path"
        response = merge_pr.CommandResult(
            1,
            json.dumps(
                {
                    "message": sentinel,
                    "code": sentinel,
                    "status": "422",
                    "errors": [{"code": sentinel}],
                }
            ),
            f"gh: {sentinel} (HTTP 422)",
        )
        with (
            mock.patch.object(
                repository_settings, "bounded_command", return_value=response
            ),
            self.assertRaises(ValueError) as raised,
        ):
            repository_settings.api(
                "repos/private/value",
                operation="create",
                subject="protected-main-ruleset",
            )
        diagnostic = str(raised.exception)
        self.assertEqual(
            diagnostic,
            "GitHub API create protected-main-ruleset failed; "
            "category=validation; status=422; effect=rejected; exit=1",
        )
        self.assertNotIn(sentinel, diagnostic)

    def test_settings_apply_reports_partial_effect(self) -> None:
        ruleset = repository_settings.required_ruleset()
        ruleset["id"] = 17
        settings = dict(repository_settings.REQUIRED_SETTINGS) | {
            "id": 17,
            "visibility": "public",
            "default_branch": "main",
            "owner": {"login": "owner", "type": "User"},
        }
        failure = ValueError(
            "GitHub API update repository-settings failed; "
            "category=validation; status=422; effect=rejected"
        )
        with (
            mock.patch.object(
                repository_settings,
                "fetch_admission",
                return_value=(settings, None, "core", "a" * 40),
            ),
            mock.patch.object(
                repository_settings, "api", side_effect=(ruleset, failure)
            ) as mocked_api,
            self.assertRaises(ValueError) as raised,
        ):
            repository_settings.apply("owner/repository")
        diagnostic = str(raised.exception)
        self.assertIn("phase=repository-settings", diagnostic)
        self.assertIn("prior-ruleset-effect=applied", diagnostic)
        self.assertEqual(mocked_api.call_count, 2)

    def test_settings_reapply_updates_only_named_ruleset(self) -> None:
        settings = dict(repository_settings.REQUIRED_SETTINGS) | {
            "id": 17,
            "visibility": "public",
            "default_branch": "main",
            "owner": {"login": "owner", "type": "User"},
        }
        ruleset = repository_settings.required_ruleset()
        ruleset["id"] = 17
        with (
            mock.patch.object(
                repository_settings,
                "fetch_admission",
                side_effect=(
                    (settings, ruleset, "core", "a" * 40),
                    (settings, ruleset, "core", "a" * 40),
                ),
            ),
            mock.patch.object(
                repository_settings, "api", side_effect=(ruleset, settings)
            ) as mocked_api,
        ):
            applied_settings, applied_ruleset = repository_settings.apply(
                "owner/repository"
            )
        self.assertEqual(applied_settings, settings)
        self.assertEqual(applied_ruleset, ruleset)
        update_call = mocked_api.call_args_list[0]
        self.assertIn("PUT", update_call.args)
        self.assertIn("repos/owner/repository/rulesets/17", update_call.args)
        self.assertNotIn("repos/owner/repository/rulesets/99", update_call.args)

        duplicates = [
            {"id": 17, "name": repository_settings.RULESET_NAME},
            {"id": 18, "name": repository_settings.RULESET_NAME},
        ]
        with (
            mock.patch.object(
                repository_settings, "api", side_effect=(settings, duplicates)
            ),
            self.assertRaisesRegex(ValueError, "duplicated"),
        ):
            repository_settings.fetch("owner/repository")

    def test_settings_capability_matrix_and_safe_validation_detail(self) -> None:
        settings = {
            "id": 17,
            "visibility": "public",
            "default_branch": "main",
            "owner": {"login": "owner", "type": "User"},
        }
        self.assertEqual(
            repository_settings.capability(
                "owner/repository",
                settings,
                {"login": "owner", "type": "User", "plan": {"name": "free"}},
            ),
            "core",
        )
        organization = dict(settings) | {
            "owner": {"login": "owner", "type": "Organization"}
        }
        self.assertEqual(
            repository_settings.capability(
                "owner/repository",
                organization,
                {
                    "login": "owner",
                    "type": "Organization",
                    "plan": {"name": "enterprise"},
                },
            ),
            "metadata",
        )
        with self.assertRaisesRegex(ValueError, "unknown|missing"):
            repository_settings.capability(
                "owner/repository",
                organization,
                {"login": "owner", "type": "Organization", "plan": {"name": "mystery"}},
            )
        response = merge_pr.CommandResult(
            1,
            json.dumps(
                {
                    "message": "Validation Failed",
                    "status": "422",
                    "errors": [
                        {
                            "resource": "RepositoryRule",
                            "field": "type",
                            "code": "invalid",
                            "private": "secret-value",
                        }
                    ],
                }
            ),
            "gh: Validation Failed (HTTP 422)",
        )
        with (
            mock.patch.object(
                repository_settings, "bounded_command", return_value=response
            ),
            self.assertRaises(ValueError) as raised,
        ):
            repository_settings.api(
                "ignored", operation="create", subject="protected-main-ruleset"
            )
        self.assertIn("detail=RepositoryRule.type.invalid", str(raised.exception))
        self.assertNotIn("secret-value", str(raised.exception))

    def test_settings_requires_successful_exact_main_before_mutation(self) -> None:
        failure = ValueError("portable provenance check is missing or ambiguous")
        with (
            mock.patch.object(
                repository_settings, "fetch_admission", side_effect=failure
            ),
            mock.patch.object(repository_settings, "api") as mutation,
            self.assertRaisesRegex(ValueError, "missing or ambiguous"),
        ):
            repository_settings.apply("owner/repository")
        mutation.assert_not_called()

    def test_settings_provenance_status_rejects_missing_stale_and_truncated(
        self,
    ) -> None:
        settings = {"default_branch": "main"}
        head = "a" * 40
        commit = {"sha": head}
        success = {
            "total_count": 1,
            "check_runs": [
                {
                    "name": repository_settings.PROVENANCE_CONTEXT,
                    "head_sha": head,
                    "status": "completed",
                    "conclusion": "success",
                }
            ],
        }
        with mock.patch.object(
            repository_settings, "api", side_effect=(commit, success)
        ):
            self.assertEqual(
                repository_settings.provenance_status("owner/repository", settings),
                head,
            )
        for checks, diagnostic in (
            ({"total_count": 0, "check_runs": []}, "missing or ambiguous"),
            (
                {
                    "total_count": 2,
                    "check_runs": success["check_runs"] * 2,
                },
                "missing or ambiguous",
            ),
            (
                {
                    "total_count": 1,
                    "check_runs": [success["check_runs"][0] | {"head_sha": "b" * 40}],
                },
                "stale, skipped, or unsuccessful",
            ),
            (
                {
                    "total_count": 1,
                    "check_runs": [
                        success["check_runs"][0]
                        | {"status": "completed", "conclusion": "skipped"}
                    ],
                },
                "stale, skipped, or unsuccessful",
            ),
            (
                {
                    "total_count": 1,
                    "check_runs": [
                        success["check_runs"][0]
                        | {"status": "completed", "conclusion": "failure"}
                    ],
                },
                "stale, skipped, or unsuccessful",
            ),
            ({"total_count": 101, "check_runs": []}, "truncated or malformed"),
        ):
            with (
                mock.patch.object(
                    repository_settings, "api", side_effect=(commit, checks)
                ),
                self.assertRaisesRegex(ValueError, diagnostic),
            ):
                repository_settings.provenance_status("owner/repository", settings)

    def test_portable_provenance_rejects_hostile_identity_and_api_data(self) -> None:
        for identity, diagnostic in (
            (
                "Trusted Human\0noreply@github.com\0Integrator\0safe@example.invalid",
                "Web Flow",
            ),
            (
                (
                    "Contributor\0safe@example.invalid\0Trusted Human\0"
                    "12345+owner@users.noreply.github.com"
                ),
                "Web Flow",
            ),
            (
                "Contributor\u202e\0safe@example.invalid\0Integrator\0safe@example.invalid",
                "malformed or oversized",
            ),
            (
                "Contributor\0safe@example.invalid\0Integrator\nInjected\0safe@example.invalid",
                "malformed or oversized",
            ),
        ):
            with (
                mock.patch.object(portable_provenance, "git", return_value=identity),
                self.assertRaisesRegex(ValueError, diagnostic),
            ):
                portable_provenance.validate_identity(ROOT, "a" * 40)
        with (
            mock.patch.object(
                repository_settings,
                "api",
                return_value={
                    "sha": "a" * 40,
                    "commit": {
                        "verification": {"verified": False, "reason": "bad_signature"}
                    },
                    "parents": [],
                    "message": "private-token-host-path",
                },
            ),
            self.assertRaisesRegex(ValueError, "verification is not valid") as raised,
        ):
            portable_provenance.api_commit("owner/repository", "a" * 40)
        self.assertNotIn("private-token-host-path", str(raised.exception))

    def test_settings_post_read_failure_reports_both_applied_effects(self) -> None:
        settings = dict(repository_settings.REQUIRED_SETTINGS) | {
            "id": 17,
            "visibility": "public",
            "default_branch": "main",
            "owner": {"login": "owner", "type": "User"},
        }
        ruleset = repository_settings.required_ruleset() | {"id": 17}
        with (
            mock.patch.object(
                repository_settings,
                "fetch_admission",
                side_effect=(
                    (settings, None, "core", "a" * 40),
                    ValueError("post-read mismatch"),
                ),
            ),
            mock.patch.object(
                repository_settings, "api", side_effect=(ruleset, settings)
            ),
            self.assertRaises(ValueError) as raised,
        ):
            repository_settings.apply("owner/repository")
        diagnostic = str(raised.exception)
        self.assertIn("phase=post-read", diagnostic)
        self.assertIn("prior-ruleset-effect=applied", diagnostic)
        self.assertIn("prior-settings-effect=applied", diagnostic)
        self.assertIn("effect=ambiguous", diagnostic)

    def test_portable_provenance_accepts_signed_topic_and_rejects_event_drift(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory(prefix="asb-portable-pr-") as raw:
            fixture = Fixture(Path(raw))

            def api(*args: str, **_kwargs: object) -> object:
                revision = args[-1].rsplit("/", 1)[-1]
                return {
                    "sha": revision,
                    "commit": {"verification": {"verified": True, "reason": "valid"}},
                    "parents": [
                        {"sha": value}
                        for value in portable_provenance.parents(
                            fixture.repository, revision
                        )
                    ],
                }

            with (
                mock.patch.object(repository_settings, "api", side_effect=api),
                mock.patch.object(
                    repository_policy,
                    "LOCAL_COMMITTER",
                    "Fixture <fixture@example.invalid>",
                ),
            ):
                portable_provenance.validate(
                    fixture.repository,
                    "owner/repository",
                    "pull_request",
                    "refs/pull",
                    fixture.base,
                    fixture.head,
                    fixture.head,
                )
                with self.assertRaisesRegex(ValueError, "event head"):
                    portable_provenance.validate(
                        fixture.repository,
                        "owner/repository",
                        "pull_request",
                        "refs/pull",
                        fixture.base,
                        fixture.head,
                        "0" * 40,
                    )

    def test_portable_provenance_accepts_exact_signed_merge_association(self) -> None:
        with tempfile.TemporaryDirectory(prefix="asb-portable-push-") as raw:
            fixture = Fixture(Path(raw))
            result = command(fixture.repository, *fixture.merge_command())
            merge = dict(line.split("=", 1) for line in result.stdout.splitlines())[
                "MERGE"
            ]

            def api(*args: str, **_kwargs: object) -> object:
                endpoint = args[-1]
                if endpoint.endswith("pulls?per_page=2"):
                    return [
                        {
                            "number": 7,
                            "state": "closed",
                            "merged_at": "2026-10-08T00:00:00Z",
                            "merge_commit_sha": merge,
                            "base": {"sha": fixture.base, "ref": "main"},
                            "head": {"sha": fixture.head},
                        }
                    ]
                revision = endpoint.rsplit("/", 1)[-1]
                return {
                    "sha": revision,
                    "commit": {"verification": {"verified": True, "reason": "valid"}},
                    "parents": [
                        {"sha": value}
                        for value in portable_provenance.parents(
                            fixture.repository, revision
                        )
                    ],
                }

            with (
                mock.patch.object(repository_settings, "api", side_effect=api),
                mock.patch.object(
                    repository_policy,
                    "LOCAL_COMMITTER",
                    "Fixture <fixture@example.invalid>",
                ),
            ):
                portable_provenance.validate(
                    fixture.repository,
                    "owner/repository",
                    "push",
                    "refs/heads/main",
                    fixture.base,
                    merge,
                    merge,
                )
            with (
                mock.patch.object(repository_settings, "api", return_value=[]),
                self.assertRaisesRegex(ValueError, "missing or ambiguous"),
            ):
                portable_provenance.validate_pull_association(
                    fixture.repository, "owner/repository", fixture.base, merge
                )


if __name__ == "__main__":
    unittest.main()
