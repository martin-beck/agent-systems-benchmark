#!/bin/sh
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
set -eu
root=$(mktemp -d "${TMPDIR:-/tmp}/asb-dev-toolchain-test.XXXXXX")
trap 'rm -rf -- "$root"' EXIT HUP INT TERM
chmod 0700 "$root"
ASB_DEV_TOOLCHAIN_ROOT="$root/tools" \
ASB_DEV_SOURCE_CARGO="$(command -v cargo)" \
ASB_DEV_SOURCE_GIT="$(command -v git)" \
ASB_DEV_SOURCE_SETSID="$(command -v setsid)" \
    tools/dev-toolchain/setup.sh >/dev/null
test "$(stat -c '%a' "$root/tools")" = 700
test "$(stat -c '%a' "$root/tools/bin/cargo")" = 700
test "$(stat -c '%a' "$root/tools/bin/cargo-wrapper")" = 700
! grep -q '^export HOME=' "$root/tools/bin/cargo-wrapper"
! grep -q '^export CARGO_HOME=' "$root/tools/bin/cargo-wrapper"
! grep -q '^export RUSTUP_HOME=' "$root/tools/bin/cargo-wrapper"
test "$(stat -c '%a' "$root/tools/bin/git")" = 700
test "$(stat -c '%a' "$root/tools/bin/setsid")" = 700
test -z "$(find "$root" -maxdepth 1 -name '.dev-toolchain-stage-*' -print -quit)"
ASB_DEV_TOOLCHAIN_ROOT="$root/tools" tools/dev-toolchain/run.sh git --version >/dev/null
test "$(ASB_DEV_TOOLCHAIN_ROOT="$root/tools" tools/dev-toolchain/run.sh sh -c 'printf %s "$ASB_DEV_GIT"')" = "$root/tools/bin/git"
test "$(ASB_DEV_TOOLCHAIN_ROOT="$root/tools" tools/dev-toolchain/run.sh sh -c 'printf %s "$HOME"')" = "$root/tools"
test "$(ASB_DEV_TOOLCHAIN_ROOT="$root/tools" tools/dev-toolchain/run.sh sh -c 'printf %s "$CARGO_HOME"')" = "$root/tools/cargo-home"
printf '%s' 'sentinel' > "$root/tools/sentinel"
set +e
ASB_DEV_TOOLCHAIN_ROOT="$root/tools" ASB_DEV_SOURCE_CARGO=/missing/cargo \
    tools/dev-toolchain/setup.sh >/dev/null 2>&1
status=$?
set -e
test "$status" -eq 4
test -f "$root/tools/sentinel"
hostile="$root/hostile"
mkdir -m 0777 "$hostile"
set +e
ASB_DEV_TOOLCHAIN_ROOT="$hostile/nested/tools" tools/dev-toolchain/setup.sh >/dev/null 2>&1
status=$?
set -e
test "$status" -eq 3
run_parent="$root/run-parent"
mkdir -m 0700 "$run_parent"
ASB_DEV_TOOLCHAIN_ROOT="$run_parent/tools" tools/dev-toolchain/setup.sh >/dev/null
chmod 0777 "$run_parent"
set +e
ASB_DEV_TOOLCHAIN_ROOT="$run_parent/tools" tools/dev-toolchain/run.sh true >/dev/null 2>&1
status=$?
set -e
test "$status" -eq 4
printf '%s\n' 'private development toolchain runner passed'
set +e
output=$(ASB_DEV_TOOLCHAIN_ROOT="$root/missing" tools/dev-toolchain/run.sh true 2>&1)
status=$?
set -e
test "$status" -eq 4
printf '%s\n' "$output" | grep -Fx 'ERROR: development toolchain unavailable'
printf 'private development toolchain runner passed\n'
