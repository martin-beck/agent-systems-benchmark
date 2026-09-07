#!/bin/sh
# SPDX-License-Identifier: MIT
set -eu
export ASB_STORAGE_ROOT=/srv/data/projects
mkdir -p "$ASB_STORAGE_ROOT/asb-ci-runners"
root=$(mktemp -d "$ASB_STORAGE_ROOT/asb-ci-runners/asb-test-XXXXXXXXXXXX")
trap 'rm -rf "$root"' EXIT HUP INT TERM
export ASB_RUNNER_ROOT=$root
export ASB_RUNNER_NAME=asb-runner-0123456789ab
export ASB_RUNNER_CONFIG_LABELS=asb-development-v1-x86_64-ubuntu2404
mkdir -p "$root/control" "$root/runner/_work" "$root/cache" "$root/artifacts" "$root/tmp"
printf '2.337.0 %s %s\n' "$ASB_RUNNER_NAME" "$ASB_RUNNER_CONFIG_LABELS" > "$root/control/manifest"
printf 'runner\n' > "$root/runner/Runner.Listener"
printf '#!/bin/sh\nexit 0\n' > "$root/runner/config.sh"
printf '#!/bin/sh\nexit 0\n' > "$root/runner/run.sh"
chmod 700 "$root/runner/Runner.Listener" "$root/runner/config.sh" "$root/runner/run.sh"
printf 'tester %s\n' "$(( $(date +%s) + 60 ))" > "$root/control/lease"
chmod 600 "$root/control/lease" "$root/control/manifest"
tools/runners/health.sh | grep -q '^healthy version=2.337.0 '
tools/runners/setup.sh | grep -q ' (unchanged)$'
touch "$root/runner/_work/residue" "$root/cache/residue" "$root/artifacts/residue" "$root/tmp/residue"
tools/runners/reset.sh | grep -q '^reset complete$'
test ! -e "$root/runner/_work/residue" && test ! -e "$root/cache/residue"
ln -s /srv/data/projects "$root/cache/escape"
if tools/runners/reset.sh >/dev/null 2>&1; then
    echo 'symlink reset unexpectedly passed' >&2
    exit 1
fi
rm "$root/cache/escape"
printf 'tester extra 9 value\n' > "$root/control/lease"
chmod 600 "$root/control/lease"
if tools/runners/health.sh >/dev/null 2>&1; then
    echo 'malformed lease unexpectedly passed' >&2
    exit 1
fi
printf 'tester 1\n' > "$root/control/lease"
chmod 600 "$root/control/lease"
if tools/runners/health.sh >/dev/null 2>&1; then
    echo 'expired lease unexpectedly passed' >&2
    exit 1
fi
printf 'tester %s\n' "$(( $(date +%s) + 60 ))" > "$root/control/lease"
chmod 644 "$root/control/lease"
if tools/runners/health.sh >/dev/null 2>&1; then
    echo 'permissive lease unexpectedly passed' >&2
    exit 1
fi
chmod 600 "$root/control/lease"
exec 8> "$root/control/reset.lock"
flock -n 8
if tools/runners/reset.sh >/dev/null 2>&1; then
    echo 'concurrent reset unexpectedly passed' >&2
    exit 1
fi
flock -u 8
export ASB_RUNNER_CONFIG_LABELS=self-hosted,linux,x64
if tools/runners/health.sh >/dev/null 2>&1; then
    echo 'generic labels unexpectedly passed' >&2
    exit 1
fi
export ASB_RUNNER_CONFIG_LABELS=asb-development-v1-x86_64-ubuntu2404
export ASB_STORAGE_ROOT=/srv/data
export ASB_RUNNER_ROOT=/srv/data/asb-ci-runners/asb-test-0123456789ab
if tools/runners/health.sh >/dev/null 2>&1; then
    echo 'outside storage root unexpectedly passed' >&2
    exit 1
fi
