#!/bin/sh
# SPDX-License-Identifier: MIT
set -eu
ASB_RUNNER_TOOLS_DIR=$(CDPATH= cd -- "$(dirname "$0")" && pwd)
export ASB_RUNNER_TOOLS_DIR
. "$ASB_RUNNER_TOOLS_DIR/common.sh"
require_root
require_identity
require_service_identity_idle
if test -e "$ASB_RUNNER_ROOT/runner"; then
    test -d "$ASB_RUNNER_ROOT/runner" && test ! -L "$ASB_RUNNER_ROOT/runner" || die 'existing runner is not a regular directory'
    require_installation
    printf 'prepared runner %s (unchanged)\n' "$ASB_RUNNER_VERSION"
    exit 0
fi
test -f "${ASB_RUNNER_ARCHIVE:-}" && test ! -L "$ASB_RUNNER_ARCHIVE" || die 'regular runner archive is required'
test "$(stat -c '%s' "$ASB_RUNNER_ARCHIVE")" = "$ASB_RUNNER_LINUX_X64_BYTES" || die 'runner archive size mismatch'
umask 077
install -d -m 0750 -o "$ASB_OPERATOR_UID" -g "$ASB_SERVICE_GID" "$ASB_RUNNER_ROOT"
install -d -m 0700 -o "$ASB_OPERATOR_UID" -g "$ASB_OPERATOR_GID" "$ASB_RUNNER_ROOT/control"
for mutable in cache artifacts tmp; do
    install -d -m 0700 -o "$ASB_SERVICE_UID" -g "$ASB_SERVICE_GID" "$ASB_RUNNER_ROOT/$mutable"
done
require_root
archive=$ASB_RUNNER_ROOT/control/archive.$$
cp --reflink=never -- "$ASB_RUNNER_ARCHIVE" "$archive"
chown "$ASB_OPERATOR_UID:$ASB_OPERATOR_GID" "$archive"
chmod 0600 "$archive"
stage=
trap 'rm -f "$archive"; test -z "$stage" || rm -rf "$stage"' EXIT HUP INT TERM
test "$(stat -c '%s' "$archive")" = "$ASB_RUNNER_LINUX_X64_BYTES" || die 'runner archive copy size mismatch'
observed=$(sha256sum "$archive" | cut -d' ' -f1)
test "$observed" = "$ASB_RUNNER_LINUX_X64_SHA256" || die 'runner archive digest mismatch'
stage=$ASB_RUNNER_ROOT/.stage-$$
mkdir "$stage"
tar -xzf "$archive" --no-same-owner --no-same-permissions -C "$stage"
rm -f "$archive"
test -x "$stage/config.sh" && test -x "$stage/run.sh" || die 'runner archive lacks required entry points'
chown -R "$ASB_OPERATOR_UID:$ASB_SERVICE_GID" "$stage"
find "$stage" -type d -exec chmod 0750 {} +
find "$stage" -type f -perm /111 -exec chmod 0550 {} +
find "$stage" -type f ! -perm /111 -exec chmod 0440 {} +
printf '%s\n' 'ACTIONS_RUNNER_REQUIRE_JOB_CONTAINER=true' > "$stage/.env"
chown "$ASB_OPERATOR_UID:$ASB_SERVICE_GID" "$stage/.env"
chmod 0640 "$stage/.env"
mv "$stage" "$ASB_RUNNER_ROOT/runner"
trap - EXIT HUP INT TERM
install -d -m 0700 -o "$ASB_SERVICE_UID" -g "$ASB_SERVICE_GID" \
    "$ASB_RUNNER_ROOT/runner/_work" "$ASB_RUNNER_ROOT/runner/_diag"
manifest=$ASB_RUNNER_ROOT/control/manifest
printf '%s %s %s %s\n' "$ASB_RUNNER_VERSION" "$ASB_RUNNER_NAME" "$ASB_RUNNER_LABELS" "$ASB_RUNNER_LINUX_X64_SHA256" > "$manifest"
installation_inventory >> "$manifest"
chmod 600 "$manifest"
require_installation
printf 'prepared runner %s\n' "$ASB_RUNNER_VERSION"
