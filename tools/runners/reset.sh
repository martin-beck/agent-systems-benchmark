#!/bin/sh
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
set -eu
ASB_RUNNER_TOOLS_DIR=$(CDPATH= cd -- "$(dirname "$0")" && pwd)
export ASB_RUNNER_TOOLS_DIR
. "$ASB_RUNNER_TOOLS_DIR/common.sh"
require_root
require_identity
require_lease
acquire_lifecycle_lock
require_immutable_installation
require_registration_state
require_service_identity_idle
require_owned_mode "$ASB_RUNNER_ROOT/control/diagnostics-summary-v1.txt" \
    "$ASB_OPERATOR_UID" "$ASB_OPERATOR_GID" 'diagnostic summary' 600
lock=$ASB_RUNNER_ROOT/control/reset.lock
test ! -L "$lock" || die 'reset lock is a symlink'
: > "$lock"
chmod 600 "$lock"
exec 9<> "$lock"
flock -n 9 || die 'another reset is active'
quarantine_root=$ASB_RUNNER_ROOT/.reset-quarantine
test ! -L "$quarantine_root" || die 'reset quarantine is a symlink'
mkdir -p "$quarantine_root"
chmod 700 "$quarantine_root"
stale_count=0
for stale in "$quarantine_root"/*; do
    test -e "$stale" || continue
    stale_count=$((stale_count + 1))
    test "$stale_count" -le 5 || die 'too many stale reset quarantines'
    case "${stale##*/}" in reset-[0-9]*-[1-5]) ;; *) die 'unexpected stale reset quarantine' ;; esac
    test -d "$stale" && test ! -L "$stale" || die 'stale reset quarantine is unsafe'
    rm -rf --one-file-system -- "$stale"
done
sequence=0
for relative in runner/_work runner/_diag cache artifacts tmp; do
    path=$ASB_RUNNER_ROOT/$relative
    test ! -L "$path" || die 'refusing symlinked reset root'
    if test -e "$path"; then
        sequence=$((sequence + 1))
        quarantine=$quarantine_root/reset-$$-$sequence
        test ! -e "$quarantine" || die 'reset quarantine collision'
        mv -- "$path" "$quarantine"
        test -d "$quarantine" && test ! -L "$quarantine" || die 'reset source changed during quarantine'
    fi
    install -d -m 0700 -o "$ASB_SERVICE_UID" -g "$ASB_SERVICE_GID" "$path"
    if test -n "${quarantine:-}" && test -d "$quarantine"; then
        rm -rf --one-file-system -- "$quarantine"
    fi
    quarantine=
done
rmdir "$quarantine_root" 2>/dev/null || die 'reset quarantine is not empty'
for name in .runner .credentials .credentials_rsaparams; do
    rm -f -- "$ASB_RUNNER_ROOT/runner/$name"
    test ! -e "$ASB_RUNNER_ROOT/runner/$name" || die 'registration state cleanup failed'
done
printf 'reset complete\n'
