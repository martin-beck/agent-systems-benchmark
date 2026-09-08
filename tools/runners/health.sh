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
require_installation
require_registration_state
printf 'healthy version=%s labels=%s lease=active\n' "$ASB_RUNNER_VERSION" "$ASB_RUNNER_LABELS"
