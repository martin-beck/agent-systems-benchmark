#!/bin/sh
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
set -eu
fail() { echo "capability-unavailable: $1" >&2; exit 78; }
command -v bwrap >/dev/null 2>&1 || fail "bubblewrap is absent"
command -v systemd-run >/dev/null 2>&1 || fail "systemd-run is absent"
[ -e /run/systemd/system ] || fail "systemd manager is not available"
bwrap --ro-bind / / --dev /dev --proc /proc --unshare-net --die-with-parent \
  /bin/true >/dev/null 2>&1 || fail "bubblewrap namespace creation failed"
echo capability-ok
