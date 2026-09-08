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
require_installation
require_registration_state
require_service_identity_idle
test "$(cat "$ASB_RUNNER_ROOT/runner/.env")" = 'ACTIONS_RUNNER_REQUIRE_JOB_CONTAINER=true' || die 'job-container enforcement is unavailable'
install -d -m 0700 -o "$ASB_SERVICE_UID" -g "$ASB_SERVICE_GID" \
    "$ASB_RUNNER_ROOT/tmp/home" "$ASB_RUNNER_ROOT/tmp/xdg-config" \
    "$ASB_RUNNER_ROOT/tmp/xdg-cache" "$ASB_RUNNER_ROOT/tmp/runtime"
unit=asb-runner-${ASB_RUNNER_NAME#asb-runner-}
systemd-run --quiet --wait --collect --service-type=exec \
    --unit "$unit" --uid "$ASB_SERVICE_UID" --gid "$ASB_SERVICE_GID" \
    --property NoNewPrivileges=yes --property PrivateTmp=yes \
    --property ProtectSystem=strict --property ProtectHome=yes \
    --property RestrictSUIDSGID=yes --property LockPersonality=yes \
    --property KillMode=control-group \
    --property "InaccessiblePaths=-$ASB_SERVICE_HOME" \
    --property "ReadWritePaths=$ASB_RUNNER_ROOT/runner/_work" \
    --property "ReadWritePaths=$ASB_RUNNER_ROOT/runner/_diag" \
    --property "ReadWritePaths=$ASB_RUNNER_ROOT/cache" \
    --property "ReadWritePaths=$ASB_RUNNER_ROOT/artifacts" \
    --property "ReadWritePaths=$ASB_RUNNER_ROOT/tmp" \
    --property RuntimeMaxSec=900 --property TimeoutStopSec=30 \
    --setenv "HOME=$ASB_RUNNER_ROOT/tmp/home" \
    --setenv "XDG_CONFIG_HOME=$ASB_RUNNER_ROOT/tmp/xdg-config" \
    --setenv "XDG_CACHE_HOME=$ASB_RUNNER_ROOT/tmp/xdg-cache" \
    --setenv "XDG_RUNTIME_DIR=$ASB_RUNNER_ROOT/tmp/runtime" \
    --setenv "TMPDIR=$ASB_RUNNER_ROOT/tmp" \
    --working-directory "$ASB_RUNNER_ROOT/runner" \
    "$ASB_RUNNER_ROOT/runner/bin/Runner.Listener" run --once
