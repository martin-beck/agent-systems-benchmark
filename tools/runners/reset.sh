#!/bin/sh
# SPDX-License-Identifier: MIT
set -eu
. "$(dirname "$0")/common.sh"
require_root
require_identity
require_lease
for relative in runner/_work cache artifacts tmp; do
    path=$ASB_RUNNER_ROOT/$relative
    test ! -L "$path" || die 'refusing symlinked reset root'
    if test -e "$path"; then
        find "$path" -xdev -type l -print -quit | grep -q . && die 'refusing reset tree containing symlinks'
        rm -rf -- "$path"
    fi
    mkdir -p "$path"
    chmod 700 "$path"
done
printf 'reset complete\n'
