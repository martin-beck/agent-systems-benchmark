#!/usr/bin/env python3
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Audit or apply GitHub-compatible protected-main publication controls."""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

if __package__:
    from .merge_pr import bounded_command
else:
    from merge_pr import bounded_command

RULESET_NAME = "ASB protected main publication"
PROVENANCE_CONTEXT = "Portable protected-main provenance"
ROOT = Path(__file__).resolve().parents[2]
OPENAPI_PROJECTION = ROOT / "config/github-ruleset-openapi.json"
HTTP_STATUS = re.compile(r"\(HTTP ([1-5][0-9]{2})\)")
SAFE_GITHUB_MESSAGES = frozenset(
    {
        "Bad credentials",
        "Branch not protected",
        "Conflict",
        "Gateway Timeout",
        "Internal Server Error",
        "Not Found",
        "Requires authentication",
        "Resource not accessible by personal access token",
        "Server Error",
        "Service Unavailable",
        "Validation Failed",
    }
)
SAFE_GITHUB_CODES = frozenset(
    {
        "already_exists",
        "custom",
        "invalid",
        "missing",
        "missing_field",
        "not_found",
        "protected_branch",
        "unprocessable",
        "unprocessable_entity",
    }
)
SAFE_GITHUB_RESOURCES = frozenset(
    {"Repository", "RepositoryRule", "RepositoryRuleset", "Ruleset"}
)
SAFE_GITHUB_FIELDS = frozenset({"enforcement", "parameters", "rules", "target", "type"})
REQUIRED_SETTINGS = {
    # GitHub requires one pull-request merge method. Keep only the two-parent
    # method used by merge_pr.py; the ruleset rejects GitHub Web Flow commits.
    "allow_merge_commit": True,
    "allow_squash_merge": False,
    "allow_rebase_merge": False,
    "allow_auto_merge": False,
    "web_commit_signoff_required": True,
}
REQUIRED_CHECKS = (
    "Policy, coverage, and supply chain",
    "Rust checks (ubuntu-24.04)",
    "Emulated aarch64 (x86_64 host)",
    "Retained faults (ubuntu-24.04)",
    "Matcher and SLO mutation sentinels",
    "Exact TUI inherited-fd and PTY journey",
    "TLC and Alloy recovery models",
    "Platform evidence (ubuntu-24.04)",
    "Exact Huawei 2026 and SPDX MIT headers",
    "Credential-free benchmark path",
    "AWQ v0.1.0 shadow evidence",
    "Bounded fuzz regressions",
    "Kani bounded proofs",
    "Loom and state models (ubuntu-24.04)",
    PROVENANCE_CONTEXT,
)


def required_ruleset(metadata_capable: bool = False) -> dict[str, object]:
    """Return the exact main ruleset accepted by the offline oracle."""
    rules: list[dict[str, object]] = [
        {"type": "deletion"},
        {"type": "non_fast_forward"},
        {"type": "required_signatures"},
        {
            "type": "pull_request",
            "parameters": {
                "required_approving_review_count": 1,
                "dismiss_stale_reviews_on_push": True,
                "require_code_owner_review": False,
                "require_last_push_approval": True,
                "required_review_thread_resolution": True,
                "allowed_merge_methods": ["merge"],
            },
        },
        {
            "type": "required_status_checks",
            "parameters": {
                "required_status_checks": [
                    {"context": context} for context in REQUIRED_CHECKS
                ],
                "strict_required_status_checks_policy": True,
                "do_not_enforce_on_create": False,
            },
        },
    ]
    if metadata_capable:
        rules.append(
            {
                "type": "committer_email_pattern",
                "parameters": {
                    "name": "Reject GitHub Web Flow commits",
                    "negate": True,
                    "operator": "regex",
                    "pattern": r"^noreply@github\.com$",
                },
            }
        )
    return {
        "name": RULESET_NAME,
        "target": "branch",
        "enforcement": "active",
        "bypass_actors": [],
        "conditions": {"ref_name": {"include": ["refs/heads/main"], "exclude": []}},
        "rules": rules,
    }


def validate_openapi_projection(ruleset: dict[str, object]) -> None:
    value = json.loads(OPENAPI_PROJECTION.read_text(encoding="utf-8"))
    if (
        value.get("schema_version") != 1
        or value.get("api_version") != "2022-11-28"
        or value.get("source_commit") != "7dee0622aeecf9df3c5060ca28c7a57ee5007804"
        or value.get("source_blob") != "63926c4dc629284e7d29ecc9de9938bf4ac0fd82"
        or value.get("source_sha256")
        != "0d2fa885d4a15f57a6fe06811cbd13e7e0f580d292530199db1a59cfc2b7d14e"
    ):
        raise ValueError("official GitHub ruleset schema projection differs")
    allowed = value.get("repository_rule_types")
    rules = ruleset.get("rules")
    if (
        not isinstance(allowed, list)
        or not isinstance(rules, list)
        or any(
            not isinstance(rule, dict) or rule.get("type") not in allowed
            for rule in rules
        )
    ):
        raise ValueError("ruleset payload is outside the pinned structural schema")


def _github_failure(
    operation: str, subject: str, returncode: int, stdout: str, stderr: str
) -> ValueError:
    """Return a bounded diagnostic without echoing an endpoint or raw response."""
    body: object = None
    try:
        body = json.loads(stdout)
    except (json.JSONDecodeError, UnicodeError):
        pass

    status: int | None = None
    match = HTTP_STATUS.search(stderr)
    if match is not None:
        status = int(match.group(1))
    if status is None and isinstance(body, dict):
        candidate = body.get("status")
        if isinstance(candidate, int) and 100 <= candidate <= 599:
            status = candidate
        elif (
            isinstance(candidate, str)
            and candidate.isascii()
            and candidate.isdigit()
            and 100 <= int(candidate) <= 599
        ):
            status = int(candidate)

    categories = {
        401: "authentication",
        403: "authorization",
        404: "not-found",
        409: "conflict",
        422: "validation",
    }
    category = categories.get(
        status, "server" if status and status >= 500 else "transport"
    )
    effect = "ambiguous" if status is None or status >= 500 else "rejected"
    parts = [
        f"GitHub API {operation} {subject} failed",
        f"category={category}",
        f"status={status if status is not None else 'unknown'}",
        f"effect={effect}",
        f"exit={returncode if 0 <= returncode <= 255 else 'unknown'}",
    ]
    if isinstance(body, dict):
        message = body.get("message")
        if isinstance(message, str) and message in SAFE_GITHUB_MESSAGES:
            parts.append(f"message={message}")
        code: object = body.get("code")
        errors = body.get("errors")
        if code is None and isinstance(errors, list):
            for item in errors[:8]:
                if isinstance(item, dict) and item.get("code") in SAFE_GITHUB_CODES:
                    code = item["code"]
                    break
        if isinstance(code, str) and code in SAFE_GITHUB_CODES:
            parts.append(f"code={code}")
        if isinstance(errors, list):
            for item in errors[:2]:
                if not isinstance(item, dict):
                    continue
                resource, field, detail_code = (
                    item.get("resource"),
                    item.get("field"),
                    item.get("code"),
                )
                if (
                    resource in SAFE_GITHUB_RESOURCES
                    and field in SAFE_GITHUB_FIELDS
                    and detail_code in SAFE_GITHUB_CODES
                ):
                    parts.append(f"detail={resource}.{field}.{detail_code}")
    return ValueError("; ".join(parts))


def api(
    *args: str,
    operation: str,
    subject: str,
    input_value: object | None = None,
) -> object:
    try:
        result = bounded_command(
            Path.cwd(),
            "gh",
            "api",
            *args,
            input_text=json.dumps(input_value) if input_value is not None else None,
        )
    except (OSError, ValueError) as error:
        raise ValueError(
            f"GitHub API {operation} {subject} failed; "
            "category=local-boundary; status=unknown; effect=ambiguous"
        ) from error
    if result.returncode:
        raise _github_failure(
            operation, subject, result.returncode, result.stdout, result.stderr
        )
    try:
        return json.loads(result.stdout)
    except json.JSONDecodeError as error:
        raise ValueError(
            f"GitHub API {operation} {subject} failed; "
            "category=response-invalid; status=success; effect=ambiguous"
        ) from error


def fetch(repository: str) -> tuple[dict[str, object], dict[str, object] | None]:
    settings = api(
        f"repos/{repository}", operation="read", subject="repository-settings"
    )
    summaries = api(
        f"repos/{repository}/rulesets",
        operation="read",
        subject="ruleset-index",
    )
    if not isinstance(settings, dict) or not isinstance(summaries, list):
        raise TypeError("GitHub settings response has unexpected shape")
    if any(not isinstance(item, dict) for item in summaries):
        raise TypeError("GitHub ruleset index has unexpected shape")
    matches = [item for item in summaries if item.get("name") == RULESET_NAME]
    if len(matches) > 1:
        raise ValueError("protected-main ruleset is duplicated")
    ruleset = None
    if matches:
        identifier = matches[0].get("id")
        if not isinstance(identifier, int):
            raise TypeError("protected-main ruleset ID is malformed")
        value = api(
            f"repos/{repository}/rulesets/{identifier}",
            operation="read",
            subject="protected-main-ruleset",
        )
        if not isinstance(value, dict):
            raise TypeError("protected-main ruleset response is not an object")
        ruleset = value
    return settings, ruleset


def capability(
    repository: str, settings: dict[str, object], owner: dict[str, object]
) -> str:
    requested_owner = repository.split("/", 1)[0]
    repository_owner = settings.get("owner")
    if (
        not isinstance(settings.get("id"), int)
        or settings["id"] <= 0
        or settings.get("visibility") not in {"public", "private", "internal"}
        or not isinstance(repository_owner, dict)
        or repository_owner.get("login") != requested_owner
        or repository_owner.get("type") not in {"User", "Organization"}
        or owner.get("login") != requested_owner
        or owner.get("type") != repository_owner.get("type")
    ):
        raise ValueError(
            "repository owner or identity capability evidence is malformed"
        )
    plan = owner.get("plan")
    if not isinstance(plan, dict) or not isinstance(plan.get("name"), str):
        raise TypeError("repository owner plan capability evidence is missing")
    plan_name = plan["name"].casefold()
    if repository_owner["type"] == "User" and plan_name in {"free", "pro"}:
        return "core"
    if repository_owner["type"] == "Organization" and plan_name == "enterprise":
        return "metadata"
    if repository_owner["type"] == "Organization" and plan_name in {"free", "team"}:
        return "core"
    raise ValueError("repository owner plan capability is unknown")


def provenance_status(repository: str, settings: dict[str, object]) -> str:
    if settings.get("default_branch") != "main":
        raise ValueError("repository default branch is not protected main")
    commit = api(
        f"repos/{repository}/commits/main",
        operation="read",
        subject="protected-main-head",
    )
    if not isinstance(commit, dict) or not re.fullmatch(
        r"[0-9a-f]{40}", str(commit.get("sha", ""))
    ):
        raise ValueError("protected-main head identity is malformed")
    head = str(commit["sha"])
    checks = api(
        f"repos/{repository}/commits/{head}/check-runs?filter=latest&per_page=100",
        operation="read",
        subject="portable-provenance-check",
    )
    if not isinstance(checks, dict) or not isinstance(checks.get("check_runs"), list):
        raise TypeError("portable provenance check response is malformed")
    runs = checks["check_runs"]
    if (
        checks.get("total_count") != len(runs)
        or len(runs) > 100
        or any(not isinstance(run, dict) for run in runs)
    ):
        raise ValueError("portable provenance check response is truncated or malformed")
    matching = [run for run in runs if run.get("name") == PROVENANCE_CONTEXT]
    if len(matching) != 1:
        raise ValueError("portable provenance check is missing or ambiguous")
    run = matching[0]
    if (
        run.get("head_sha") != head
        or run.get("status") != "completed"
        or run.get("conclusion") != "success"
    ):
        raise ValueError("portable provenance check is stale, skipped, or unsuccessful")
    return head


def fetch_admission(
    repository: str,
) -> tuple[dict[str, object], dict[str, object] | None, str, str]:
    settings, ruleset = fetch(repository)
    repository_owner = settings.get("owner")
    if not isinstance(repository_owner, dict):
        raise TypeError("repository owner capability evidence is malformed")
    login, owner_type = repository_owner.get("login"), repository_owner.get("type")
    if not isinstance(login, str) or owner_type not in {"User", "Organization"}:
        raise ValueError("repository owner capability evidence is malformed")
    owner = api(
        f"{'users' if owner_type == 'User' else 'orgs'}/{login}",
        operation="read",
        subject="owner-capability",
    )
    if not isinstance(owner, dict):
        raise TypeError("repository owner capability response is malformed")
    return (
        settings,
        ruleset,
        capability(repository, settings, owner),
        provenance_status(repository, settings),
    )


def apply(repository: str) -> tuple[dict[str, object], dict[str, object]]:
    before, existing, mode, head = fetch_admission(repository)
    expected = required_ruleset(mode == "metadata")
    validate_openapi_projection(expected)
    endpoint = f"repos/{repository}/rulesets"
    method = "POST"
    if existing is not None:
        endpoint += f"/{existing['id']}"
        method = "PUT"
    ruleset = api(
        "--method",
        method,
        endpoint,
        "--input",
        "-",
        operation="create" if method == "POST" else "update",
        subject="protected-main-ruleset",
        input_value=expected,
    )
    if not isinstance(ruleset, dict):
        raise TypeError(
            "GitHub ruleset update response has unexpected shape; effect=ambiguous"
        )
    command = ["--method", "PATCH", f"repos/{repository}"]
    for key, value in REQUIRED_SETTINGS.items():
        command.extend(["-F", f"{key}={str(value).lower()}"])
    try:
        settings = api(*command, operation="update", subject="repository-settings")
        if not isinstance(settings, dict):
            raise TypeError("GitHub settings update response has unexpected shape")
    except (TypeError, ValueError) as error:
        raise ValueError(
            f"{error}; phase=repository-settings; prior-ruleset-effect=applied"
        ) from error
    try:
        after, observed, after_mode, after_head = fetch_admission(repository)
        if (
            after.get("id") != before.get("id")
            or after_mode != mode
            or after_head != head
        ):
            raise ValueError("post-apply repository identity or capability changed")
        validate(after, observed, after_mode, True)
    except (TypeError, ValueError) as error:
        raise ValueError(
            f"{error}; phase=post-read; prior-ruleset-effect=applied; "
            "prior-settings-effect=applied; effect=ambiguous"
        ) from error
    return after, observed


def validate(
    settings: dict[str, object],
    ruleset: dict[str, object] | None,
    mode: str,
    provenance_ready: bool,
) -> None:
    for key, expected in REQUIRED_SETTINGS.items():
        if settings.get(key) is not expected:
            raise ValueError(
                f"repository setting {key} must be {str(expected).lower()}"
            )
    if ruleset is None:
        raise ValueError("protected-main ruleset is missing")
    if mode not in {"core", "metadata"}:
        raise ValueError("repository capability mode is unknown")
    if provenance_ready is not True:
        raise ValueError("portable provenance check is not successful on exact main")
    expected_ruleset = required_ruleset(mode == "metadata")
    validate_openapi_projection(expected_ruleset)
    if {key: ruleset.get(key) for key in expected_ruleset} != expected_ruleset:
        raise ValueError("protected-main ruleset differs from the required policy")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repository", default="martin-beck/agent-systems-benchmark")
    parser.add_argument("--settings-json", help="offline settings fixture")
    parser.add_argument("--ruleset-json", help="offline ruleset fixture")
    parser.add_argument("--owner-json", help="offline owner capability fixture")
    parser.add_argument(
        "--provenance-status",
        choices=("success", "missing", "stale", "skipped", "failure"),
    )
    parser.add_argument("--apply", action="store_true")
    args = parser.parse_args()
    try:
        fixtures = any(
            value is not None
            for value in (
                args.settings_json,
                args.ruleset_json,
                args.owner_json,
                args.provenance_status,
            )
        )
        if fixtures and any(
            value is None
            for value in (
                args.settings_json,
                args.ruleset_json,
                args.owner_json,
                args.provenance_status,
            )
        ):
            raise ValueError("offline audit requires every fixture argument")
        if fixtures and args.apply:
            raise ValueError("offline fixtures cannot be combined with --apply")
        if fixtures:
            settings = json.loads(args.settings_json)
            ruleset = json.loads(args.ruleset_json)
            owner = json.loads(args.owner_json)
            if not isinstance(settings, dict) or not isinstance(owner, dict):
                raise TypeError("offline capability fixture has unexpected shape")
            mode = capability(args.repository, settings, owner)
            provenance_ready = args.provenance_status == "success"
        elif args.apply:
            settings, ruleset = apply(args.repository)
            mode = (
                "metadata"
                if any(
                    rule.get("type") == "committer_email_pattern"
                    for rule in ruleset["rules"]
                )
                else "core"
            )
            provenance_ready = True
        else:
            settings, ruleset, mode, _ = fetch_admission(args.repository)
            provenance_ready = True
        if not isinstance(settings, dict) or not isinstance(ruleset, dict):
            raise TypeError("protected-publication fixture has unexpected shape")
        validate(settings, ruleset, mode, provenance_ready)
    except OSError:
        print("merge settings: bounded local operation failed", file=sys.stderr)
        return 1
    except (TypeError, ValueError, json.JSONDecodeError) as error:
        print(f"merge settings: {error}", file=sys.stderr)
        return 1
    print("merge settings: GitHub-compatible protected publication is enforced")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
