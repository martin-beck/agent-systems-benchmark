#!/bin/sh
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
set -eu
ASB_RUNNER_TOOLS_DIR=$(CDPATH= cd -- "$(dirname "$0")" && pwd)
export ASB_RUNNER_TOOLS_DIR
. "$ASB_RUNNER_TOOLS_DIR/common.sh"
require_root
require_identity
acquire_lifecycle_lock
require_installation
require_service_identity_idle
require_private_procfs
test "${ASB_REPOSITORY:-}" = martin-beck/agent-systems-benchmark || die 'repository is not the approved public ASB repository'
for name in .runner .credentials .credentials_rsaparams; do
    test ! -e "$ASB_RUNNER_ROOT/runner/$name" || die 'runner registration state already exists'
done
registration_complete=0
cleanup_registration() {
    token=
    if test "$registration_complete" -ne 1; then
        rm -f -- "$ASB_RUNNER_ROOT/runner/.runner" \
            "$ASB_RUNNER_ROOT/runner/.credentials" \
            "$ASB_RUNNER_ROOT/runner/.credentials_rsaparams"
    fi
}
trap cleanup_registration EXIT HUP INT TERM
IFS= read -r token || die 'short-lived registration token is required on stdin'
test -n "$token" || die 'short-lived registration token is empty'
test "${#token}" -le 1024 || die 'short-lived registration token is oversized'
case "$token" in *[!A-Za-z0-9._-]*) die 'short-lived registration token is malformed' ;; esac
if IFS= read -r extra; then
    die 'registration input has trailing data'
fi
set +x
export RUNNER_ALLOW_RUNASROOT=1
"$ASB_RUNNER_ROOT/runner/config.sh" --url "https://github.com/$ASB_REPOSITORY" --token "$token" --name "$ASB_RUNNER_NAME" --labels "$ASB_RUNNER_LABELS" --work _work --unattended --ephemeral --disableupdate --no-default-labels
token=
for name in .runner .credentials .credentials_rsaparams; do
    state=$ASB_RUNNER_ROOT/runner/$name
    test -f "$state" && test ! -L "$state" || die 'registration did not create complete runner state'
    chown "$ASB_OPERATOR_UID:$ASB_SERVICE_GID" "$state"
    chmod 0440 "$state"
done
require_registration_state
require_installation
registration_complete=1
trap - EXIT HUP INT TERM
printf 'registered ephemeral runner with exact capability label\n'
