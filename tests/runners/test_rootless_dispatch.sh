#!/bin/sh
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
set -eu

ROOT=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)
fake_path=$(mktemp -d "$ROOT/.asb-rootless-path-XXXXXXXXXXXX")
cleanup() {
    rm -rf -- "$fake_path"
}
trap cleanup EXIT HUP INT TERM
ln -s /usr/bin/id "$fake_path/id"
ln -s /usr/bin/mktemp "$fake_path/mktemp"
set +e
output=$(PATH="$fake_path" "$ROOT/tests/runners/test_runner_scripts.sh" 2>&1)
status=$?
set -e
test "$status" -eq 1
printf '%s\n' "$output" | grep -Fqx 'ERROR: hardened rootless runner lacks sudo and fakeroot; install fakeroot or provide an approved rootless fixture runtime'
printf 'rootless dispatch negative fixture passed\n'
