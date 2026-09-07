#!/bin/sh
# SPDX-License-Identifier: MIT
set -eu
. "$(dirname "$0")/common.sh"
require_root
require_identity
require_lease
require_installation
test -d "$ASB_RUNNER_ROOT/runner/_work" || die 'runner work root is unavailable'
printf 'healthy version=%s labels=%s lease=active\n' "$ASB_RUNNER_VERSION" "$ASB_RUNNER_LABELS"
