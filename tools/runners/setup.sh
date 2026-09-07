#!/bin/sh
# SPDX-License-Identifier: MIT
set -eu
. "$(dirname "$0")/common.sh"
require_root
require_identity
test -f "${ASB_RUNNER_ARCHIVE:-}" && test ! -L "$ASB_RUNNER_ARCHIVE" || die 'regular runner archive is required'
observed=$(sha256sum "$ASB_RUNNER_ARCHIVE" | cut -d' ' -f1)
test "$observed" = "$ASB_RUNNER_LINUX_X64_SHA256" || die 'runner archive digest mismatch'
umask 077
mkdir -p "$ASB_RUNNER_ROOT/control" "$ASB_RUNNER_ROOT/cache" "$ASB_RUNNER_ROOT/artifacts" "$ASB_RUNNER_ROOT/tmp"
test ! -e "$ASB_RUNNER_ROOT/runner" || die 'runner installation already exists'
stage=$ASB_RUNNER_ROOT/.stage-$$
mkdir "$stage"
trap 'rm -rf "$stage"' EXIT HUP INT TERM
tar -xzf "$ASB_RUNNER_ARCHIVE" --no-same-owner --no-same-permissions -C "$stage"
test -x "$stage/config.sh" && test -x "$stage/run.sh" || die 'runner archive lacks required entry points'
mv "$stage" "$ASB_RUNNER_ROOT/runner"
trap - EXIT HUP INT TERM
printf '%s %s %s\n' "$ASB_RUNNER_VERSION" "$ASB_RUNNER_NAME" "$ASB_RUNNER_LABELS" > "$ASB_RUNNER_ROOT/control/manifest"
chmod 600 "$ASB_RUNNER_ROOT/control/manifest"
printf 'prepared runner %s\n' "$ASB_RUNNER_VERSION"
