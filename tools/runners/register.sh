#!/bin/sh
# SPDX-License-Identifier: MIT
set -eu
. "$(dirname "$0")/common.sh"
require_root
require_identity
require_installation
test "${ASB_REPOSITORY:-}" = martin-beck/agent-systems-benchmark || die 'repository is not the approved public ASB repository'
test ! -e "$ASB_RUNNER_ROOT/runner/.runner" || die 'runner is already registered'
IFS= read -r token || die 'short-lived registration token is required on stdin'
test -n "$token" || die 'short-lived registration token is empty'
set +x
"$ASB_RUNNER_ROOT/runner/config.sh" --url "https://github.com/$ASB_REPOSITORY" --token "$token" --name "$ASB_RUNNER_NAME" --labels "$ASB_RUNNER_LABELS" --work _work --unattended --ephemeral --disableupdate --no-default-labels
token=
test -f "$ASB_RUNNER_ROOT/runner/.runner" && test ! -L "$ASB_RUNNER_ROOT/runner/.runner" || die 'registration did not create runner state'
chmod 600 "$ASB_RUNNER_ROOT/runner/.runner"
printf 'registered ephemeral runner with exact capability label\n'
