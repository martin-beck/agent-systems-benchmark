#!/bin/sh
# SPDX-License-Identifier: MIT
set -eu

ASB_RUNNER_VERSION=2.337.0
ASB_RUNNER_LINUX_X64_SHA256=70920811a4f8ad4328818682bca5c6469c1c942fab52448868071d0063816613
ASB_RUNNER_LABELS=asb-development-v1,asb-x86_64-v1,asb-ubuntu-24.04-v1

die() { printf 'ERROR: %s\n' "$1" >&2; exit 1; }

require_root() {
    test -n "${ASB_STORAGE_ROOT:-}" || die 'ASB_STORAGE_ROOT is required'
    test -n "${ASB_RUNNER_ROOT:-}" || die 'ASB_RUNNER_ROOT is required'
    test "${ASB_STORAGE_ROOT#/}" != "$ASB_STORAGE_ROOT" || die 'storage root must be absolute'
    resolved_storage=$(realpath -e "$ASB_STORAGE_ROOT") || die 'storage root cannot be resolved'
    test "$resolved_storage" = "$ASB_STORAGE_ROOT" || die 'storage root must be exact and canonical'
    case "$ASB_RUNNER_ROOT" in "$ASB_STORAGE_ROOT"/asb-ci-runners/asb-*) ;; *) die 'runner root is outside the dedicated project storage namespace' ;; esac
    parent=$(dirname "$ASB_RUNNER_ROOT")
    test -d "$parent" || die 'runner parent does not exist'
    resolved_parent=$(realpath -e "$parent") || die 'runner parent cannot be resolved'
    test "$resolved_parent" = "$ASB_STORAGE_ROOT/asb-ci-runners" || die 'runner parent resolves outside project storage'
    test ! -L "$ASB_RUNNER_ROOT" || die 'runner root is a symlink'
}

require_identity() {
    case "${ASB_RUNNER_NAME:-}" in
        asb-runner-[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]) ;;
        *) die 'runner name must be a pseudonymous 12-hex identifier' ;;
    esac
    test "${ASB_RUNNER_CONFIG_LABELS:-}" = "$ASB_RUNNER_LABELS" || die 'runner labels differ from the exact capability set'
    case ",$ASB_RUNNER_CONFIG_LABELS," in
        *,self-hosted,*|*,linux,*|*,x64,*|*,arm64,*) die 'default or generic labels are forbidden' ;;
    esac
}

require_lease() {
    lease=$ASB_RUNNER_ROOT/control/lease
    test -f "$lease" && test ! -L "$lease" || die 'active regular lease is required'
    IFS=' ' read -r owner expiry extra < "$lease" || die 'lease is unreadable'
    test -n "$owner" && test -n "$expiry" && test -z "${extra:-}" || die 'lease is malformed'
    case "$owner" in *[!A-Za-z0-9._-]*|'') die 'lease owner is malformed' ;; esac
    case "$expiry" in *[!0-9]*|'') die 'lease expiry is malformed' ;; esac
    now=$(date +%s)
    test "$expiry" -gt "$now" || die 'lease is expired'
}
