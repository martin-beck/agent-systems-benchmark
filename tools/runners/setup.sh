#!/bin/sh
# SPDX-License-Identifier: MIT
set -eu
. "$(dirname "$0")/common.sh"
require_root
require_identity
if test -e "$ASB_RUNNER_ROOT/runner"; then
    test -d "$ASB_RUNNER_ROOT/runner" && test ! -L "$ASB_RUNNER_ROOT/runner" || die 'existing runner is not a regular directory'
    require_installation
    printf 'prepared runner %s (unchanged)\n' "$ASB_RUNNER_VERSION"
    exit 0
fi
test -f "${ASB_RUNNER_ARCHIVE:-}" && test ! -L "$ASB_RUNNER_ARCHIVE" || die 'regular runner archive is required'
observed=$(sha256sum "$ASB_RUNNER_ARCHIVE" | cut -d' ' -f1)
test "$observed" = "$ASB_RUNNER_LINUX_X64_SHA256" || die 'runner archive digest mismatch'
umask 077
mkdir -p "$ASB_RUNNER_ROOT/control" "$ASB_RUNNER_ROOT/cache" "$ASB_RUNNER_ROOT/artifacts" "$ASB_RUNNER_ROOT/tmp"
require_root
stage=$ASB_RUNNER_ROOT/.stage-$$
mkdir "$stage"
trap 'rm -rf "$stage"' EXIT HUP INT TERM
tar -xzf "$ASB_RUNNER_ARCHIVE" --no-same-owner --no-same-permissions -C "$stage"
test -x "$stage/config.sh" && test -x "$stage/run.sh" || die 'runner archive lacks required entry points'
mv "$stage" "$ASB_RUNNER_ROOT/runner"
trap - EXIT HUP INT TERM
manifest=$ASB_RUNNER_ROOT/control/manifest
config_hash=$(sha256sum "$ASB_RUNNER_ROOT/runner/config.sh" | cut -d' ' -f1)
run_hash=$(sha256sum "$ASB_RUNNER_ROOT/runner/run.sh" | cut -d' ' -f1)
listener_hash=$(sha256sum "$ASB_RUNNER_ROOT/runner/bin/Runner.Listener" | cut -d' ' -f1)
printf '%s %s %s %s %s %s %s\n' "$ASB_RUNNER_VERSION" "$ASB_RUNNER_NAME" "$ASB_RUNNER_LABELS" "$ASB_RUNNER_LINUX_X64_SHA256" "$config_hash" "$run_hash" "$listener_hash" > "$manifest"
chmod 600 "$manifest"
require_installation
printf 'prepared runner %s\n' "$ASB_RUNNER_VERSION"
