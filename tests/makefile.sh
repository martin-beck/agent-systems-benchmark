#!/bin/sh
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)
tmp=$(mktemp -d "${TMPDIR:-/tmp}/asb-makefile-test.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT HUP INT TERM
make -C "$root" help >/dev/null
grep -F -- '.DEFAULT_GOAL := lifecycle' "$root/Makefile" >/dev/null
grep -F -- 'lifecycle: update build test install' "$root/Makefile" >/dev/null
if grep -F -- 'lifecycle: install test build update' "$root/Makefile" >/dev/null; then
    printf '%s\n' 'lifecycle order is reversed' >&2
    exit 1
fi
order=$(make -C "$root" -qp 2>/dev/null | awk '$1 == "lifecycle:" { print $2, $3, $4, $5; exit }')
test "$order" = 'update build test install'
if make -f - lifecycle >/dev/null 2>&1 <<'EOF'
.PHONY: lifecycle update build test install
lifecycle: update build test install
update:
	@exit 1
build test install:
	@touch "$@"
EOF
then
    printf '%s\n' 'failing lifecycle unexpectedly passed' >&2
    exit 1
fi
test ! -e build && test ! -e test && test ! -e install
if make -C "$root" CARGO="$tmp/missing-cargo" check-deps >"$tmp/out" 2>&1; then
    printf '%s\n' 'missing Cargo unexpectedly passed' >&2
    exit 1
fi
grep -F 'missing' "$tmp/out" >/dev/null
grep -F -- 'build --locked --workspace' "$root/Makefile" >/dev/null
grep -F -- 'fmt --all -- --check' "$root/Makefile" >/dev/null
grep -F -- 'metadata --locked --no-deps' "$root/Makefile" >/dev/null
grep -F -- 'refresh-lock' "$root/Makefile" >/dev/null
if grep -A8 '^update:' "$root/Makefile" | grep -F -- 'update --locked' >/dev/null; then
    printf '%s\n' 'update still attempts a lockfile mutation with --locked' >&2
    exit 1
fi
if grep -A8 '^refresh-lock:' "$root/Makefile" | grep -F -- 'update --locked' >/dev/null; then
    printf '%s\n' 'refresh-lock incorrectly passes --locked to Cargo update' >&2
    exit 1
fi
if grep -F 'install --path' "$root/Makefile" | grep -v -- '--locked' >/dev/null; then
    printf '%s\n' 'install target is not locked' >&2
    exit 1
fi

# Exercise the update/refresh split with bounded fake tools: update validates
# locked metadata without mutating the lock, while refresh-lock invokes the
# lock-mutating command without --locked and validates afterwards.
cat >"$tmp/rustup" <<'EOF'
#!/bin/sh
case "$1" in
    toolchain) printf '%s\n' '1.93.0 (default)' ;;
    target) printf '%s\n' 'x86_64-unknown-linux-gnu (installed)' ;;
    *) exit 2 ;;
esac
EOF
cat >"$tmp/git" <<'EOF'
#!/bin/sh
case "$1" in
    diff) exit "${FAKE_GIT_DIRTY:-0}" ;;
    pull) printf '%s\n' 'pull' >>"$FAKE_LOG" ;;
    *) exit 0 ;;
esac
EOF
cat >"$tmp/cargo" <<'EOF'
#!/bin/sh
printf 'cargo %s\n' "$*" >>"$FAKE_LOG"
case "$*" in
    *' update'*) test "${FAKE_CARGO_FAIL:-0}" -eq 0 ;;
    *' metadata --locked --no-deps'*) test "${FAKE_METADATA_FAIL:-0}" -eq 0 ;;
    *) exit 2 ;;
esac
EOF
chmod +x "$tmp/rustup" "$tmp/git" "$tmp/cargo"
: >"$tmp/log"
FAKE_LOG="$tmp/log" CARGO="$tmp/cargo" RUSTUP="$tmp/rustup" GIT="$tmp/git" \
    make -C "$root" update >/dev/null
grep -F 'cargo +1.93.0 metadata --locked --no-deps' "$tmp/log" >/dev/null
if grep -F 'cargo +1.93.0 update' "$tmp/log" >/dev/null; then
    printf '%s\n' 'update unexpectedly refreshed Cargo.lock' >&2
    exit 1
fi
: >"$tmp/log"
FAKE_LOG="$tmp/log" CARGO="$tmp/cargo" RUSTUP="$tmp/rustup" GIT="$tmp/git" \
    make -C "$root" refresh-lock >/dev/null
grep -F 'cargo +1.93.0 update' "$tmp/log" >/dev/null
grep -F 'cargo +1.93.0 metadata --locked --no-deps' "$tmp/log" >/dev/null
if ! awk '/cargo \+1.93.0 update/{ refreshed=1 } /metadata --locked --no-deps/{ validated=1 } END { exit !(refreshed && validated) }' "$tmp/log"; then
    printf '%s\n' 'refresh-lock did not validate after refreshing' >&2
    exit 1
fi
: >"$tmp/log"
if FAKE_LOG="$tmp/log" FAKE_CARGO_FAIL=1 CARGO="$tmp/cargo" RUSTUP="$tmp/rustup" GIT="$tmp/git" \
    make -C "$root" refresh-lock >/dev/null 2>&1; then
    printf '%s\n' 'failed lock refresh unexpectedly passed' >&2
    exit 1
fi
if grep -F 'metadata --locked --no-deps' "$tmp/log" >/dev/null; then
    printf '%s\n' 'refresh-lock validated after failed refresh' >&2
    exit 1
fi
: >"$tmp/log"
if FAKE_LOG="$tmp/log" FAKE_GIT_DIRTY=1 CARGO="$tmp/cargo" RUSTUP="$tmp/rustup" GIT="$tmp/git" \
    make -C "$root" refresh-lock >/dev/null 2>&1; then
    printf '%s\n' 'dirty refresh unexpectedly passed' >&2
    exit 1
fi
test ! -s "$tmp/log"
printf '%s\n' 'repository Makefile checks passed'
