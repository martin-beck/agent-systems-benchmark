#!/bin/sh
# SPDX-License-Identifier: MIT
set -eu
. "$(dirname "$0")/common.sh"
require_root
require_identity
require_installation
test "${ASB_REPOSITORY:-}" = martin-beck/agent-systems-benchmark || die 'repository is not the approved public ASB repository'
test ! -e "$ASB_RUNNER_ROOT/runner/.runner" || die 'runner is already registered'
command -v gh >/dev/null 2>&1 || die 'GitHub CLI is required'
token=$(gh api --method POST "repos/$ASB_REPOSITORY/actions/runners/registration-token" --jq .token)
test -n "$token" || die 'GitHub did not return a registration token'
set +x
"$ASB_RUNNER_ROOT/runner/config.sh" --url "https://github.com/$ASB_REPOSITORY" --token "$token" --name "$ASB_RUNNER_NAME" --labels "$ASB_RUNNER_LABELS" --work _work --unattended --ephemeral --disableupdate --no-default-labels
token=
test -f "$ASB_RUNNER_ROOT/runner/.runner" && test ! -L "$ASB_RUNNER_ROOT/runner/.runner" || die 'registration did not create runner state'
chmod 600 "$ASB_RUNNER_ROOT/runner/.runner"
printf 'registered ephemeral runner with exact capability label\n'
