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
)


def required_ruleset() -> dict[str, object]:
    """Return the exact main ruleset accepted by the offline oracle."""
    return {
        "name": RULESET_NAME,
        "target": "branch",
        "enforcement": "active",
        "bypass_actors": [],
        "conditions": {"ref_name": {"include": ["refs/heads/main"], "exclude": []}},
        "rules": [
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
            {
                "type": "committer_email_pattern",
                "parameters": {
                    "name": "Reject GitHub Web Flow commits",
                    "negate": True,
                    "operator": "regex",
                    "pattern": r"^noreply@github\.com$",
                },
            },
        ],
    }


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


def apply(repository: str) -> tuple[dict[str, object], dict[str, object]]:
    _, existing = fetch(repository)
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
        input_value=required_ruleset(),
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
    return settings, ruleset


def validate(settings: dict[str, object], ruleset: dict[str, object] | None) -> None:
    for key, expected in REQUIRED_SETTINGS.items():
        if settings.get(key) is not expected:
            raise ValueError(
                f"repository setting {key} must be {str(expected).lower()}"
            )
    if ruleset is None:
        raise ValueError("protected-main ruleset is missing")
    expected_ruleset = required_ruleset()
    if {key: ruleset.get(key) for key in expected_ruleset} != expected_ruleset:
        raise ValueError("protected-main ruleset differs from the required policy")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repository", default="martin-beck/agent-systems-benchmark")
    parser.add_argument("--settings-json", help="offline settings fixture")
    parser.add_argument("--ruleset-json", help="offline ruleset fixture")
    parser.add_argument("--apply", action="store_true")
    args = parser.parse_args()
    try:
        fixtures = args.settings_json is not None or args.ruleset_json is not None
        if fixtures and (args.settings_json is None or args.ruleset_json is None):
            raise ValueError("offline audit requires both fixture arguments")
        if fixtures and args.apply:
            raise ValueError("offline fixtures cannot be combined with --apply")
        if fixtures:
            settings = json.loads(args.settings_json)
            ruleset = json.loads(args.ruleset_json)
        elif args.apply:
            settings, ruleset = apply(args.repository)
        else:
            settings, ruleset = fetch(args.repository)
        if not isinstance(settings, dict) or not isinstance(ruleset, dict):
            raise TypeError("protected-publication fixture has unexpected shape")
        validate(settings, ruleset)
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
