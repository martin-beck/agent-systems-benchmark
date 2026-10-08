#!/bin/sh
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)
tmp=$(mktemp -d "${TMPDIR:-/tmp}/asb-makefile-test.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT HUP INT TERM
make -C "$root" help >/dev/null
if make -C "$root" CARGO="$tmp/missing-cargo" check-deps >"$tmp/out" 2>&1; then
    printf '%s\n' 'missing Cargo unexpectedly passed' >&2
    exit 1
fi
grep -F 'missing' "$tmp/out" >/dev/null
grep -F -- 'build --locked --workspace' "$root/Makefile" >/dev/null
grep -F -- 'fmt --all -- --check' "$root/Makefile" >/dev/null
if grep -F 'install --path' "$root/Makefile" | grep -v -- '--locked' >/dev/null; then
    printf '%s\n' 'install target is not locked' >&2
    exit 1
fi
printf '%s\n' 'repository Makefile checks passed'
