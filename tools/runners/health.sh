#!/bin/sh
# SPDX-License-Identifier: MIT
set -eu
. "$(dirname "$0")/common.sh"
require_root
require_identity
require_lease
manifest=$ASB_RUNNER_ROOT/control/manifest
test -f "$manifest" && test ! -L "$manifest" || die 'runner manifest is unavailable'
test "$(stat -c '%a' "$manifest")" = 600 || die 'runner manifest permissions must be 0600'
expected="$ASB_RUNNER_VERSION $ASB_RUNNER_NAME $ASB_RUNNER_LABELS"
test "$(cat "$manifest")" = "$expected" || die 'runner manifest drifted'
test -x "$ASB_RUNNER_ROOT/runner/Runner.Listener" || die 'runner listener is unavailable'
test -d "$ASB_RUNNER_ROOT/runner/_work" || die 'runner work root is unavailable'
printf 'healthy version=%s labels=%s lease=active\n' "$ASB_RUNNER_VERSION" "$ASB_RUNNER_LABELS"
