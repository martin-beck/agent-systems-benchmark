#!/bin/sh
# SPDX-License-Identifier: MIT
set -eu
. "$(dirname "$0")/common.sh"
require_root
require_identity
require_lease
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
sequence=0
for relative in runner/_work cache artifacts tmp; do
    path=$ASB_RUNNER_ROOT/$relative
    test ! -L "$path" || die 'refusing symlinked reset root'
    if test -e "$path"; then
        find "$path" -xdev -type l -print -quit | grep -q . && die 'refusing reset tree containing symlinks'
        sequence=$((sequence + 1))
        quarantine=$quarantine_root/reset-$$-$sequence
        test ! -e "$quarantine" || die 'reset quarantine collision'
        mv -- "$path" "$quarantine"
        test -d "$quarantine" && test ! -L "$quarantine" || die 'reset source changed during quarantine'
        find "$quarantine" -xdev -type l -print -quit | grep -q . && die 'reset tree changed during quarantine'
    fi
    mkdir -p "$path"
    chmod 700 "$path"
    if test -n "${quarantine:-}" && test -d "$quarantine"; then
        rm -rf --one-file-system -- "$quarantine"
    fi
    quarantine=
done
rmdir "$quarantine_root" 2>/dev/null || die 'reset quarantine is not empty'
printf 'reset complete\n'
