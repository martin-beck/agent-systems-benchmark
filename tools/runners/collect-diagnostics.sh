#!/bin/sh
# SPDX-License-Identifier: MIT
set -eu
ASB_RUNNER_TOOLS_DIR=$(CDPATH= cd -- "$(dirname "$0")" && pwd)
export ASB_RUNNER_TOOLS_DIR
. "$ASB_RUNNER_TOOLS_DIR/common.sh"
require_root
require_identity
acquire_lifecycle_lock
require_service_identity_idle
source_root=$ASB_RUNNER_ROOT/runner/_diag
output=$ASB_RUNNER_ROOT/control/diagnostics-summary-v1.txt
require_owned_mode "$source_root" "$ASB_SERVICE_UID" "$ASB_SERVICE_GID" diagnostics 700
test ! -L "$output" || die 'diagnostic summary is a symlink'
if find "$source_root" -xdev -mindepth 1 ! -type f -print -quit | grep -q .; then
    die 'diagnostic tree contains an unsupported entry'
fi
count=$(find "$source_root" -xdev -type f -printf '.' | wc -c)
test "$count" -le "$ASB_MAX_DIAGNOSTIC_FILES" || die 'too many diagnostic files'
name_lines=$(find "$source_root" -xdev -type f -printf '%f\n' | wc -l)
test "$name_lines" -eq "$count" || die 'diagnostic filename is unsafe'
if find "$source_root" -xdev -type f -printf '%f\n' | grep -Ev '^[A-Za-z0-9._-]+$' | grep -q .; then
    die 'diagnostic filename is unsafe'
fi
if find "$source_root" -xdev -type f -size +"${ASB_MAX_DIAGNOSTIC_FILE_BYTES}"c -print -quit | grep -q .; then
    die 'diagnostic file is oversized'
fi
total=$(find "$source_root" -xdev -type f -printf '%s\n' | awk '{sum += $1} END {print sum + 0}')
test "$total" -le "$ASB_MAX_DIAGNOSTIC_TOTAL_BYTES" || die 'diagnostic set is oversized'
count=0
total=0
body=$ASB_RUNNER_ROOT/control/diagnostics.$$
trap 'rm -f "$body"' EXIT HUP INT TERM
: > "$body"
find "$source_root" -xdev -type f -print | LC_ALL=C sort | while IFS= read -r file; do
    count=$((count + 1))
    test "$count" -le "$ASB_MAX_DIAGNOSTIC_FILES" || die 'too many diagnostic files'
    size=$(stat -c '%s' "$file")
    test "$size" -le "$ASB_MAX_DIAGNOSTIC_FILE_BYTES" || die 'diagnostic file is oversized'
    total=$((total + size))
    test "$total" -le "$ASB_MAX_DIAGNOSTIC_TOTAL_BYTES" || die 'diagnostic set is oversized'
    digest=$(sha256sum "$file" | cut -d' ' -f1)
    printf 'file=%s bytes=%s sha256=%s\n' "$count" "$size" "$digest" >> "$body"
done
lines=$(wc -l < "$body")
bytes=$(awk -F '[ =]' '{sum += $4} END {print sum + 0}' "$body")
tmp=$ASB_RUNNER_ROOT/control/diagnostics-summary.$$
{
    printf 'schema=asb-runner-diagnostics-summary-v1\nfiles=%s\nbytes=%s\n' "$lines" "$bytes"
    cat "$body"
} > "$tmp"
chmod 0600 "$tmp"
chown "$ASB_OPERATOR_UID:$ASB_OPERATOR_GID" "$tmp"
mv -f "$tmp" "$output"
chown "$ASB_OPERATOR_UID:$ASB_OPERATOR_GID" "$output"
chmod 0600 "$output"
rm -f "$body"
trap - EXIT HUP INT TERM
printf 'diagnostic summary retained without names or content\n'
