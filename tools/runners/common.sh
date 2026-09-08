#!/bin/sh
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
set -eu

PATH=/usr/sbin:/usr/bin:/sbin:/bin
export PATH
case "${ASB_RUNNER_TOOLS_DIR:-}" in /*) ;; *) printf 'ERROR: runner tools directory is unavailable\n' >&2; exit 1 ;; esac

ASB_RUNNER_VERSION=2.337.0
ASB_RUNNER_LINUX_X64_SHA256=70920811a4f8ad4328818682bca5c6469c1c942fab52448868071d0063816613
ASB_RUNNER_LINUX_X64_BYTES=226430031
ASB_RUNNER_LABELS=asb-development-v1-x86_64-ubuntu2404
ASB_MAX_DIAGNOSTIC_FILES=64
ASB_MAX_DIAGNOSTIC_FILE_BYTES=65536
ASB_MAX_DIAGNOSTIC_TOTAL_BYTES=1048576

die() { printf 'ERROR: %s\n' "$1" >&2; exit 1; }

require_numeric_id() {
    case "$2" in ''|*[!0-9]*) die "$1 must be a numeric identity" ;; esac
}

require_split_identities() {
    require_numeric_id ASB_OPERATOR_UID "${ASB_OPERATOR_UID:-}"
    require_numeric_id ASB_OPERATOR_GID "${ASB_OPERATOR_GID:-}"
    require_numeric_id ASB_SERVICE_UID "${ASB_SERVICE_UID:-}"
    require_numeric_id ASB_SERVICE_GID "${ASB_SERVICE_GID:-}"
    test "$ASB_OPERATOR_UID:$ASB_OPERATOR_GID" = 0:0 || die 'operator broker must be root-owned'
    test "$ASB_OPERATOR_UID" != "$ASB_SERVICE_UID" || die 'operator and service identities must differ'
    test "$(id -u)" = "$ASB_OPERATOR_UID" || die 'operator command used by the wrong identity'
    service_record=$(getent passwd "$ASB_SERVICE_UID") || die 'service account is unavailable'
    test "$(printf '%s\n' "$service_record" | wc -l)" -eq 1 || die 'service account is ambiguous'
    IFS=: read -r service_name service_password service_uid service_gid service_gecos ASB_SERVICE_HOME service_shell service_extra <<EOF
$service_record
EOF
    test "$service_uid:$service_gid" = "$ASB_SERVICE_UID:$ASB_SERVICE_GID" || die 'service account identity drifted'
    case "$service_name" in ''|*[!A-Za-z0-9._-]*) die 'service account name is malformed' ;; esac
    case "$ASB_SERVICE_HOME" in /*) ;; *) die 'service account home is malformed' ;; esac
    case "$ASB_SERVICE_HOME" in *[!A-Za-z0-9._/-]*|*/../*|*/..|*/./*|*/.) die 'service account home is malformed' ;; esac
    case "$service_shell" in /usr/sbin/nologin|/sbin/nologin|/bin/false|/usr/bin/false) ;; *) die 'service account must deny interactive login' ;; esac
    test -z "${service_extra:-}" || die 'service account is malformed'
    test "$(id -G "$ASB_SERVICE_UID")" = "$ASB_SERVICE_GID" || die 'service account has unexpected supplementary groups'
}

require_service_identity_idle() {
    if pgrep -u "$ASB_SERVICE_UID" >/dev/null 2>&1; then
        die 'service identity still owns a process'
    fi
}

procfs_has_private_pids() {
    awk '$2 == "/proc" {
        count += 1
        n = split($4, options, ",")
        for (i = 1; i <= n; i++) {
            if (options[i] == "hidepid=2" || options[i] == "hidepid=invisible") private = 1
        }
    }
    END { exit !(count == 1 && private == 1) }' "$1"
}

require_private_procfs() {
    procfs_has_private_pids /proc/mounts || die 'procfs must hide operator processes from the service identity'
}

acquire_lifecycle_lock() {
    lock=$ASB_RUNNER_ROOT/control/lifecycle.lock
    test ! -L "$lock" || die 'lifecycle lock is a symlink'
    : > "$lock"
    chown "$ASB_OPERATOR_UID:$ASB_OPERATOR_GID" "$lock"
    chmod 0600 "$lock"
    exec 8<> "$lock"
    flock -n 8 || die 'another runner lifecycle operation is active'
}

require_owned_mode() {
    test -e "$1" && test ! -L "$1" || die "$4 is unavailable"
    test "$(stat -c '%u:%g:%a' "$1")" = "$2:$3:$5" || die "$4 ownership or permissions drifted"
}

require_root() {
    require_split_identities
    test -n "${ASB_STORAGE_ROOT:-}" || die 'ASB_STORAGE_ROOT is required'
    test -n "${ASB_RUNNER_ROOT:-}" || die 'ASB_RUNNER_ROOT is required'
    test "${ASB_STORAGE_ROOT#/}" != "$ASB_STORAGE_ROOT" || die 'storage root must be absolute'
    resolved_storage=$(realpath -e "$ASB_STORAGE_ROOT") || die 'storage root cannot be resolved'
    test "$resolved_storage" = "$ASB_STORAGE_ROOT" || die 'storage root must be exact and canonical'
    case "$resolved_storage" in /srv/data/projects/*) ;; *) die 'storage root must be beneath /srv/data/projects' ;; esac
    storage_name=${resolved_storage##*/}
    storage_suffix=${storage_name#asb-runner-storage-}
    test "$storage_suffix" != "$storage_name" || die 'storage root must use the dedicated runner name'
    case "$storage_suffix" in ''|*[!A-Za-z0-9._-]*) die 'storage root name is malformed' ;; esac
    test "$(stat -c '%u:%g:%a' "$resolved_storage")" = "$ASB_OPERATOR_UID:$ASB_SERVICE_GID:750" || die 'storage root ownership or permissions drifted'
    case "$ASB_RUNNER_ROOT" in "$ASB_STORAGE_ROOT"/asb-ci-runners/asb-*) ;; *) die 'runner root is outside the dedicated project storage namespace' ;; esac
    parent=$(dirname "$ASB_RUNNER_ROOT")
    test -d "$parent" || die 'runner parent does not exist'
    resolved_parent=$(realpath -e "$parent") || die 'runner parent cannot be resolved'
    test "$resolved_parent" = "$ASB_STORAGE_ROOT/asb-ci-runners" || die 'runner parent resolves outside project storage'
    test "$(stat -c '%d' "$resolved_parent")" = "$(stat -c '%d' "$resolved_storage")" || die 'runner parent is on another filesystem'
    test "$(stat -c '%u:%g' "$resolved_parent")" = "$ASB_OPERATOR_UID:$ASB_SERVICE_GID" || die 'runner parent ownership drifted'
    test "$(stat -c '%a' "$resolved_parent")" = 750 || die 'runner parent permissions must be 0750'
    child_count=$(find "$resolved_parent" -xdev -mindepth 1 -maxdepth 1 -printf . | wc -c)
    if test -e "$ASB_RUNNER_ROOT"; then
        test "$child_count" -eq 1 || die 'runner parent is not dedicated'
    else
        test "$child_count" -eq 0 || die 'runner parent is not empty'
    fi
    test ! -L "$ASB_RUNNER_ROOT" || die 'runner root is a symlink'
    if test -e "$ASB_RUNNER_ROOT"; then
        test -d "$ASB_RUNNER_ROOT" || die 'runner root is not a directory'
        test "$(stat -c '%d' "$ASB_RUNNER_ROOT")" = "$(stat -c '%d' "$resolved_storage")" || die 'runner root is on another filesystem'
        test "$(stat -c '%u:%g' "$ASB_RUNNER_ROOT")" = "$ASB_OPERATOR_UID:$ASB_SERVICE_GID" || die 'runner root ownership drifted'
        test "$(stat -c '%a' "$ASB_RUNNER_ROOT")" = 750 || die 'runner root permissions must be 0750'
    fi
}

require_fixed_roots() {
    require_owned_mode "$ASB_RUNNER_ROOT/control" "$ASB_OPERATOR_UID" "$ASB_OPERATOR_GID" control 700
    require_owned_mode "$ASB_RUNNER_ROOT/runner" "$ASB_OPERATOR_UID" "$ASB_SERVICE_GID" runner 750
}

require_mutable_roots() {
    for relative in runner/_work runner/_diag cache artifacts tmp; do
        require_owned_mode "$ASB_RUNNER_ROOT/$relative" "$ASB_SERVICE_UID" "$ASB_SERVICE_GID" "$relative" 700
    done
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
    test "$(stat -c '%u' "$lease")" = "$ASB_OPERATOR_UID" || die 'lease is not operator-owned'
    IFS=' ' read -r owner expiry extra < "$lease" || die 'lease is unreadable'
    test -n "$owner" && test -n "$expiry" && test -z "${extra:-}" || die 'lease is malformed'
    case "$owner" in *[!A-Za-z0-9._-]*|'') die 'lease owner is malformed' ;; esac
    case "$expiry" in *[!0-9]*|'') die 'lease expiry is malformed' ;; esac
    now=$(date +%s)
    test "$expiry" -gt "$now" || die 'lease is expired'
}

installation_inventory() {
    inventory_root=$ASB_RUNNER_ROOT/runner
    /usr/bin/python3 "$ASB_RUNNER_TOOLS_DIR/inventory.py" \
        "$inventory_root" "$ASB_OPERATOR_UID" "$ASB_SERVICE_GID"
}

require_registration_state() {
    for name in .runner .credentials .credentials_rsaparams; do
        credential=$ASB_RUNNER_ROOT/runner/$name
        test -f "$credential" && test ! -L "$credential" || die 'runner credential state is unavailable'
        test "$(stat -c '%u:%g:%a' "$credential")" = "$ASB_OPERATOR_UID:$ASB_SERVICE_GID:440" || die 'runner credential state permissions drifted'
    done
}

require_immutable_installation() {
    require_fixed_roots
    manifest=$ASB_RUNNER_ROOT/control/manifest
    test -f "$manifest" && test ! -L "$manifest" || die 'runner manifest is unavailable'
    test "$(stat -c '%a' "$manifest")" = 600 || die 'runner manifest permissions must be 0600'
    test "$(stat -c '%u' "$manifest")" = "$ASB_OPERATOR_UID" || die 'runner manifest is not operator-owned'
    for entry in config.sh run.sh bin/Runner.Listener; do
        test -f "$ASB_RUNNER_ROOT/runner/$entry" && test ! -L "$ASB_RUNNER_ROOT/runner/$entry" && test -x "$ASB_RUNNER_ROOT/runner/$entry" || die 'runner entry point is unavailable'
    done
    expected_header="$ASB_RUNNER_VERSION $ASB_RUNNER_NAME $ASB_RUNNER_LABELS $ASB_RUNNER_LINUX_X64_SHA256"
    test "$(sed -n '1p' "$manifest")" = "$expected_header" || die 'runner installation identity drifted'
    actual=$ASB_RUNNER_ROOT/control/manifest.actual.$$
    trap 'rm -f "$actual"' EXIT HUP INT TERM
    installation_inventory > "$actual"
    tail -n +2 "$manifest" | cmp -s - "$actual" || die 'runner installation drifted'
    rm -f "$actual"
    trap - EXIT HUP INT TERM
}

require_installation() {
    require_immutable_installation
    require_mutable_roots
}
