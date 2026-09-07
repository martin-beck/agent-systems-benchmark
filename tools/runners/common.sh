#!/bin/sh
# SPDX-License-Identifier: MIT
set -eu

ASB_RUNNER_VERSION=2.337.0
ASB_RUNNER_LINUX_X64_SHA256=70920811a4f8ad4328818682bca5c6469c1c942fab52448868071d0063816613
ASB_RUNNER_LABELS=asb-development-v1-x86_64-ubuntu2404

die() { printf 'ERROR: %s\n' "$1" >&2; exit 1; }

require_root() {
    test -n "${ASB_STORAGE_ROOT:-}" || die 'ASB_STORAGE_ROOT is required'
    test -n "${ASB_RUNNER_ROOT:-}" || die 'ASB_RUNNER_ROOT is required'
    test "${ASB_STORAGE_ROOT#/}" != "$ASB_STORAGE_ROOT" || die 'storage root must be absolute'
    resolved_storage=$(realpath -e "$ASB_STORAGE_ROOT") || die 'storage root cannot be resolved'
    test "$resolved_storage" = "$ASB_STORAGE_ROOT" || die 'storage root must be exact and canonical'
    case "$resolved_storage" in /srv/data/projects|/srv/data/projects/*) ;; *) die 'storage root must resolve beneath /srv/data/projects' ;; esac
    case "$ASB_RUNNER_ROOT" in "$ASB_STORAGE_ROOT"/asb-ci-runners/asb-*) ;; *) die 'runner root is outside the dedicated project storage namespace' ;; esac
    parent=$(dirname "$ASB_RUNNER_ROOT")
    test -d "$parent" || die 'runner parent does not exist'
    resolved_parent=$(realpath -e "$parent") || die 'runner parent cannot be resolved'
    test "$resolved_parent" = "$ASB_STORAGE_ROOT/asb-ci-runners" || die 'runner parent resolves outside project storage'
    test "$(stat -c '%d' "$resolved_parent")" = "$(stat -c '%d' "$resolved_storage")" || die 'runner parent is on another filesystem'
    test "$(stat -c '%u' "$resolved_parent")" = "$(id -u)" || die 'runner parent is not owned by the service identity'
    test "$(stat -c '%a' "$resolved_parent")" = 700 || die 'runner parent permissions must be 0700'
    test ! -L "$ASB_RUNNER_ROOT" || die 'runner root is a symlink'
    if test -e "$ASB_RUNNER_ROOT"; then
        test -d "$ASB_RUNNER_ROOT" || die 'runner root is not a directory'
        test "$(stat -c '%d' "$ASB_RUNNER_ROOT")" = "$(stat -c '%d' "$resolved_storage")" || die 'runner root is on another filesystem'
        test "$(stat -c '%u' "$ASB_RUNNER_ROOT")" = "$(id -u)" || die 'runner root is not owned by the service identity'
        test "$(stat -c '%a' "$ASB_RUNNER_ROOT")" = 700 || die 'runner root permissions must be 0700'
    fi
}

require_identity() {
    case "${ASB_RUNNER_NAME:-}" in
        asb-runner-[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]) ;;
        *) die 'runner name must be a pseudonymous 12-hex identifier' ;;
    esac
    test "${ASB_RUNNER_CONFIG_LABELS:-}" = "$ASB_RUNNER_LABELS" || die 'runner labels differ from the exact capability set'
    case "$ASB_RUNNER_CONFIG_LABELS" in *,*) die 'capability label must be indivisible' ;; esac
}

require_lease() {
    lease=$ASB_RUNNER_ROOT/control/lease
    test -f "$lease" && test ! -L "$lease" || die 'active regular lease is required'
    test "$(stat -c '%a' "$lease")" = 600 || die 'lease permissions must be 0600'
    IFS=' ' read -r owner expiry extra < "$lease" || die 'lease is unreadable'
    test -n "$owner" && test -n "$expiry" && test -z "${extra:-}" || die 'lease is malformed'
    case "$owner" in *[!A-Za-z0-9._-]*|'') die 'lease owner is malformed' ;; esac
    case "$expiry" in *[!0-9]*|'') die 'lease expiry is malformed' ;; esac
    now=$(date +%s)
    test "$expiry" -gt "$now" || die 'lease is expired'
}
