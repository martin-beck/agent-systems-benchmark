#!/bin/sh
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
set -eu
root=${ASB_DEV_TOOLCHAIN_ROOT:-${HOME:?}/.cache/asb/dev-toolchain-v1}
case "$root" in /*) ;; *) printf '%s\n' 'ERROR: toolchain root must be absolute' >&2; exit 3 ;; esac
test -d "$root" && test ! -L "$root" || { printf '%s\n' 'ERROR: development toolchain unavailable' >&2; exit 4; }
test "$(stat -c '%u:%a' "$root")" = "$(id -u):700" || { printf '%s\n' 'ERROR: development toolchain root is unsafe' >&2; exit 4; }
for tool in cargo git setsid; do
    path="$root/bin/$tool"
    test -f "$path" && test ! -L "$path" && test -x "$path" || { printf 'ERROR: development tool %s unavailable\n' "$tool" >&2; exit 4; }
    variable=$(printf '%s' "$tool" | tr '[:lower:]' '[:upper:]')
    export "ASB_DEV_${variable}=$path"
done
test "$#" -gt 0 || { printf '%s\n' 'ERROR: command is required' >&2; exit 2; }
exec "$@"
