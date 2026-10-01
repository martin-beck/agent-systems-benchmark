#!/bin/sh
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
set -eu
root=${ASB_DEV_TOOLCHAIN_ROOT:-${HOME:?}/.cache/asb/dev-toolchain-v1}
case "$root" in /*) ;; *) printf '%s\n' 'ERROR: toolchain root must be absolute' >&2; exit 3 ;; esac
test -d "$root" && test ! -L "$root" || { printf '%s\n' 'ERROR: development toolchain unavailable' >&2; exit 4; }
test "$(stat -c '%u:%a' "$root")" = "$(id -u):700" || { printf '%s\n' 'ERROR: development toolchain root is unsafe' >&2; exit 4; }
uid=$(id -u)
ancestor=$(dirname "$root")
while :; do
    test -d "$ancestor" && test ! -L "$ancestor" || { printf '%s\n' 'ERROR: development toolchain ancestor is unsafe' >&2; exit 4; }
    owner=$(stat -c '%u' "$ancestor")
    test "$owner" = "$uid" || test "$owner" = 0 || { printf '%s\n' 'ERROR: development toolchain ancestor owner is unsafe' >&2; exit 4; }
    if find "$ancestor" -prune -perm /022 -print -quit | grep -q .; then
        mode=$(stat -c '%a' "$ancestor")
        test "$owner:$mode" = '0:1777' || { printf '%s\n' 'ERROR: development toolchain ancestor is writable' >&2; exit 4; }
    fi
    test "$ancestor" = / && break
    next=$(dirname "$ancestor")
    test "$next" != "$ancestor" || break
    ancestor="$next"
done
for tool in cargo git setsid; do
    path="$root/bin/$tool"
    test "$tool" = cargo && path="$root/bin/cargo-wrapper"
    test -f "$path" && test ! -L "$path" && test -x "$path" || { printf 'ERROR: development tool %s unavailable\n' "$tool" >&2; exit 4; }
    variable=$(printf '%s' "$tool" | tr '[:lower:]' '[:upper:]')
    export "ASB_DEV_${variable}=$path"
done
test "$#" -gt 0 || { printf '%s\n' 'ERROR: command is required' >&2; exit 2; }
export HOME="$root"
case "${ASB_DEV_CARGO_HOME:-$root/cargo-home}" in /*) ;; *) printf '%s\n' 'ERROR: cargo home must be absolute' >&2; exit 3 ;; esac
if test -n "${ASB_DEV_CARGO_HOME:-}"; then
    export CARGO_HOME="$ASB_DEV_CARGO_HOME"
    test -d "$CARGO_HOME" && test ! -L "$CARGO_HOME" || { printf '%s\n' 'ERROR: configured cargo home is unavailable' >&2; exit 4; }
    test "$(stat -c '%u:%a' "$CARGO_HOME")" = "$(id -u):700" || { printf '%s\n' 'ERROR: configured cargo home is unsafe' >&2; exit 4; }
    ancestor=$(dirname "$CARGO_HOME")
    while :; do
        test -d "$ancestor" && test ! -L "$ancestor" || { printf '%s\n' 'ERROR: configured cargo home ancestor is unsafe' >&2; exit 4; }
        test "$ancestor" = / && break
        next=$(dirname "$ancestor")
        test "$next" != "$ancestor" || break
        ancestor="$next"
    done
else
    export CARGO_HOME="$root/cargo-home"
    mkdir -p -m 0700 "$CARGO_HOME"
fi
case "$1" in
    cargo|git|setsid)
        tool="$1"
        shift
        command_path="$root/bin/$tool"
        test "$tool" = cargo && command_path="$root/bin/cargo-wrapper"
        set -- "$command_path" "$@"
        ;;
esac
case "$1" in
    cargo|git|setsid) : ;;
    /*)
        command_path=$(realpath -e "$1" 2>/dev/null || true)
        test -f "$command_path" && test -x "$command_path" || { printf '%s\n' 'ERROR: command is not a private executable' >&2; exit 4; }
        test -z "$(find "$command_path" -prune -perm /022 -print -quit)" || { printf '%s\n' 'ERROR: command is writable by group or other' >&2; exit 4; }
        ;;
    *) printf '%s\n' 'ERROR: command must be staged or absolute' >&2; exit 3 ;;
esac
exec "$@"
