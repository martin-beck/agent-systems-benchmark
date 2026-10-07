#!/usr/bin/env python3
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Audit or apply GitHub-compatible protected-main publication controls."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

if __package__:
    from .merge_pr import bounded_command
else:
    from merge_pr import bounded_command

RULESET_NAME = "ASB protected main publication"
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


def api(*args: str, input_value: object | None = None) -> object:
    result = bounded_command(
        Path.cwd(),
        "gh",
        "api",
        *args,
        input_text=json.dumps(input_value) if input_value is not None else None,
    )
    if result.returncode:
        raise ValueError("GitHub protected-publication query or update failed")
    return json.loads(result.stdout)


def fetch(repository: str) -> tuple[dict[str, object], dict[str, object] | None]:
    settings = api(f"repos/{repository}")
    summaries = api(f"repos/{repository}/rulesets")
    if not isinstance(settings, dict) or not isinstance(summaries, list):
        raise TypeError("GitHub settings response has unexpected shape")
    matches = [item for item in summaries if item.get("name") == RULESET_NAME]
    if len(matches) > 1:
        raise ValueError("protected-main ruleset is duplicated")
    ruleset = None
    if matches:
        identifier = matches[0].get("id")
        if not isinstance(identifier, int):
            raise TypeError("protected-main ruleset ID is malformed")
        value = api(f"repos/{repository}/rulesets/{identifier}")
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
        "--method", method, endpoint, "--input", "-", input_value=required_ruleset()
    )
    command = ["--method", "PATCH", f"repos/{repository}"]
    for key, value in REQUIRED_SETTINGS.items():
        command.extend(["-F", f"{key}={str(value).lower()}"])
    settings = api(*command)
    if not isinstance(settings, dict) or not isinstance(ruleset, dict):
        raise TypeError("GitHub update response has unexpected shape")
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
