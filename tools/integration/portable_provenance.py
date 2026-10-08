#!/usr/bin/env python3
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Verify portable pull-request and protected-main publication provenance."""

from __future__ import annotations

import argparse
import re
import sys
import unicodedata
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tools.integration import repository_settings
from tools.integration.merge_pr import bounded_command
from tools.quality import repository_policy

FULL_OID = re.compile(r"^[0-9a-f]{40}$")
REPOSITORY = re.compile(r"^[A-Za-z0-9_.-]{1,100}/[A-Za-z0-9_.-]{1,100}$")
MAX_COMMITS = 128


def fail(message: str) -> None:
    raise ValueError(message)


def git(root: Path, *args: str) -> str:
    result = bounded_command(root, "git", *args)
    if result.returncode:
        fail("bounded Git provenance operation failed")
    return result.stdout.strip()


def parents(root: Path, revision: str) -> list[str]:
    values = git(root, "show", "-s", "--format=%P", revision).split()
    if any(not FULL_OID.fullmatch(value) for value in values):
        fail("commit parent metadata is malformed")
    return values


def revisions(root: Path, base: str, head: str) -> list[str]:
    values = git(root, "rev-list", "--reverse", f"{base}..{head}").splitlines()
    if not values:
        fail("portable provenance range is empty")
    if len(values) > MAX_COMMITS:
        fail("portable provenance range exceeds the bounded commit count")
    if any(not FULL_OID.fullmatch(value) for value in values):
        fail("portable provenance range is malformed")
    return values


def validate_identity(root: Path, revision: str) -> None:
    raw = git(root, "show", "-s", "--format=%an%x00%ae%x00%cn%x00%ce", revision)
    fields = raw.split("\0")
    if (
        len(raw.encode()) > 512
        or len(fields) != 4
        or any(
            not value
            or any(
                ord(character) == 127 or unicodedata.category(character).startswith("C")
                for character in value
            )
            for value in fields
        )
    ):
        fail("commit identity is malformed or oversized")
    emails = (fields[1].casefold(), fields[3].casefold())
    if any(
        email == "noreply@github.com" or email.endswith("@users.noreply.github.com")
        for email in emails
    ):
        fail("GitHub Web Flow or noreply commit identity is forbidden")


def validate_pull_topology(root: Path, base: str, head: str) -> None:
    introduced = revisions(root, base, head)
    if introduced != repository_policy.topic_first_parent_spine(root, base, head):
        fail("pull-request range contains commits outside its first-parent spine")
    merges = [revision for revision in introduced if len(parents(root, revision)) != 1]
    if merges:
        if merges != [head]:
            fail("pull-request synchronization merge must be the topic tip")
        sync = parents(root, head)
        if len(sync) != 2 or sync[1] != base:
            fail("pull-request synchronization merge must merge the exact base")
        if repository_policy.is_ancestor(root, base, sync[0]):
            fail("pull-request synchronization merge is redundant")


def api_commit(repository: str, revision: str) -> dict[str, object]:
    value = repository_settings.api(
        f"repos/{repository}/commits/{revision}",
        operation="read",
        subject="commit-provenance",
    )
    if not isinstance(value, dict) or value.get("sha") != revision:
        fail("GitHub commit provenance identity is malformed or stale")
    commit = value.get("commit")
    verification = commit.get("verification") if isinstance(commit, dict) else None
    if (
        not isinstance(verification, dict)
        or verification.get("verified") is not True
        or verification.get("reason") != "valid"
    ):
        fail("GitHub commit verification is not valid")
    api_parents = value.get("parents")
    if not isinstance(api_parents, list) or any(
        not isinstance(item, dict) or not FULL_OID.fullmatch(str(item.get("sha", "")))
        for item in api_parents
    ):
        fail("GitHub commit parent metadata is malformed")
    return value


def validate_pull_association(
    root: Path, repository: str, base: str, head: str
) -> None:
    merge_parents = parents(root, head)
    value = repository_settings.api(
        "-H",
        "Accept: application/vnd.github+json",
        f"repos/{repository}/commits/{head}/pulls?per_page=2",
        operation="read",
        subject="merge-pull-association",
    )
    if not isinstance(value, list) or len(value) != 1 or not isinstance(value[0], dict):
        fail("protected-main merge pull-request association is missing or ambiguous")
    pull = value[0]
    pull_base, pull_head = pull.get("base"), pull.get("head")
    if (
        len(merge_parents) != 2
        or merge_parents[0] != base
        or not isinstance(pull.get("number"), int)
        or pull.get("state") != "closed"
        or not pull.get("merged_at")
        or pull.get("merge_commit_sha") != head
        or not isinstance(pull_base, dict)
        or pull_base.get("sha") != base
        or pull_base.get("ref") != "main"
        or not isinstance(pull_head, dict)
        or pull_head.get("sha") != merge_parents[1]
    ):
        fail("protected-main merge pull-request association is stale or mismatched")


def validate(
    root: Path,
    repository: str,
    event: str,
    ref: str,
    base: str,
    head: str,
    event_head: str,
) -> None:
    if not REPOSITORY.fullmatch(repository):
        fail("repository identity is malformed")
    if any(not FULL_OID.fullmatch(value) for value in (base, head, event_head)):
        fail("provenance identities must be full lowercase SHA-1 values")
    if head != event_head:
        fail("checked head differs from the immutable event head")
    introduced = revisions(root, base, head)
    if event == "pull_request":
        if ref != "refs/pull":
            fail("pull-request provenance requires the canonical event ref class")
        validate_pull_topology(root, base, head)
        repository_policy.validate_commits(
            base, head, mode="ssh-only", event=event, ref=ref, root=root
        )
    elif event == "push":
        if ref != "refs/heads/main":
            fail("push provenance requires protected main")
        repository_policy.validate_commits(
            base, head, mode="protected-main", event=event, ref=ref, root=root
        )
        validate_pull_association(root, repository, base, head)
    else:
        fail("portable provenance event is unsupported")
    for revision in introduced:
        validate_identity(root, revision)
        value = api_commit(repository, revision)
        observed = [str(item["sha"]) for item in value["parents"]]
        if observed != parents(root, revision):
            fail("GitHub commit parents differ from local immutable history")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument("--repository", required=True)
    parser.add_argument("--event", choices=("pull_request", "push"), required=True)
    parser.add_argument("--ref", required=True)
    parser.add_argument("--base", required=True)
    parser.add_argument("--head", required=True)
    parser.add_argument("--event-head", required=True)
    args = parser.parse_args()
    try:
        validate(
            args.root.resolve(),
            args.repository,
            args.event,
            args.ref,
            args.base,
            args.head,
            args.event_head,
        )
    except (OSError, TypeError, ValueError) as error:
        print(f"portable provenance: {error}", file=sys.stderr)
        return 1
    print("portable provenance: exact signed-DCO publication accepted")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
