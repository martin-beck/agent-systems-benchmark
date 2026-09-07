#!/bin/sh
# SPDX-License-Identifier: MIT
set -eu
export ASB_STORAGE_ROOT=/srv/data/projects
mkdir -p "$ASB_STORAGE_ROOT/asb-ci-runners"
root=$(mktemp -d "$ASB_STORAGE_ROOT/asb-ci-runners/asb-test-XXXXXXXXXXXX")
trap 'rm -rf "$root"' EXIT HUP INT TERM
export ASB_RUNNER_ROOT=$root
export ASB_RUNNER_NAME=asb-runner-0123456789ab
export ASB_RUNNER_CONFIG_LABELS=asb-development-v1,asb-x86_64-v1,asb-ubuntu-24.04-v1
mkdir -p "$root/control" "$root/runner/_work" "$root/cache" "$root/artifacts" "$root/tmp"
printf '2.337.0 %s %s\n' "$ASB_RUNNER_NAME" "$ASB_RUNNER_CONFIG_LABELS" > "$root/control/manifest"
printf 'runner\n' > "$root/runner/Runner.Listener"
chmod 700 "$root/runner/Runner.Listener"
printf 'tester %s\n' "$(( $(date +%s) + 60 ))" > "$root/control/lease"
tools/runners/health.sh | grep -q '^healthy version=2.337.0 '
touch "$root/runner/_work/residue" "$root/cache/residue" "$root/artifacts/residue" "$root/tmp/residue"
tools/runners/reset.sh | grep -q '^reset complete$'
test ! -e "$root/runner/_work/residue" && test ! -e "$root/cache/residue"
ln -s /srv/data/projects "$root/cache/escape"
if tools/runners/reset.sh >/dev/null 2>&1; then
    echo 'symlink reset unexpectedly passed' >&2
    exit 1
fi
rm "$root/cache/escape"
printf 'tester 1\n' > "$root/control/lease"
if tools/runners/health.sh >/dev/null 2>&1; then
    echo 'expired lease unexpectedly passed' >&2
    exit 1
fi
export ASB_RUNNER_CONFIG_LABELS=self-hosted,linux,x64
if tools/runners/health.sh >/dev/null 2>&1; then
    echo 'generic labels unexpectedly passed' >&2
    exit 1
fi
