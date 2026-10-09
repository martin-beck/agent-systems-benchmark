#!/bin/sh
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)
tmp=$(mktemp -d "${TMPDIR:-/tmp}/asb-makefile-test.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT HUP INT TERM
make -C "$root" help >"$tmp/help"
grep -F -- 'make install     Install asb into PREFIX' "$tmp/help" >/dev/null
if grep -F -- 'make install --force' "$tmp/help" >/dev/null; then
    printf '%s\n' 'help advertises a Cargo-only option to Make users' >&2
    exit 1
fi
grep -F -- '.DEFAULT_GOAL := lifecycle' "$root/Makefile" >/dev/null
grep -F -- 'PREFIX ?= $(HOME)/.local' "$root/Makefile" >/dev/null
grep -F -- 'default: $$HOME/.local/bin/asb' "$root/Makefile" >/dev/null
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
grep -F -- 'install --locked --force --path crates/asb-cli' "$root/Makefile" >/dev/null
if grep -F -- 'make install --force' "$root/Makefile" "$root/README.md" >/dev/null; then
    printf '%s\n' 'Cargo overwrite option is incorrectly advertised as a Make option' >&2
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

# Exercise the default user-local install and a safe explicit packaging prefix
# with a bounded fake Cargo. No root privileges or real compilation are used.
cat >"$tmp/install-cargo" <<'EOF'
#!/bin/sh
printf 'cargo %s\n' "$*" >>"$FAKE_LOG"
case "$*" in
    *' install '*)
        root=
        previous=
        force=false
        for arg in "$@"; do
            if test "$previous" = '--root'; then root=$arg; fi
            if test "$arg" = '--force'; then force=true; fi
            previous=$arg
        done
        test -n "$root"
        test -n "${FAKE_INSTALL_CONTENT:-}"
        mkdir -p "$root/bin"
        if test -e "$root/bin/asb" && test "$force" != true; then
            printf '%s\n' 'destination exists; pass Cargo --force' >&2
            exit 1
        fi
        printf '#!/bin/sh\n# %s\n' "$FAKE_INSTALL_CONTENT" >"$root/bin/asb"
        chmod 755 "$root/bin/asb"
        ;;
    *) exit 2 ;;
esac
EOF
chmod +x "$tmp/install-cargo"
install_home="$tmp/home"
mkdir -p "$install_home"
: >"$tmp/log"
HOME="$install_home" FAKE_INSTALL_CONTENT=default-one FAKE_LOG="$tmp/log" CARGO="$tmp/install-cargo" RUSTUP="$tmp/rustup" GIT="$tmp/git" \
    make -C "$root" install >"$tmp/install-out"
test -x "$install_home/.local/bin/asb"
test -f "$install_home/.local/.asb-make-staging"
grep -F -- '# default-one' "$install_home/.local/bin/asb" >/dev/null
grep -F -- "Installed ASB at $install_home/.local/bin/asb" "$tmp/install-out" >/dev/null
grep -F -- "--root $install_home/.local" "$tmp/log" >/dev/null
grep -F -- 'install --locked --force --path crates/asb-cli' "$tmp/log" >/dev/null
printf '%s\n' 'keep-default' >"$install_home/.local/unrelated"
HOME="$install_home" FAKE_INSTALL_CONTENT=default-two FAKE_LOG="$tmp/log" CARGO="$tmp/install-cargo" RUSTUP="$tmp/rustup" GIT="$tmp/git" \
    make -C "$root" install >/dev/null
grep -F -- '# default-two' "$install_home/.local/bin/asb" >/dev/null
grep -F -- 'keep-default' "$install_home/.local/unrelated" >/dev/null

explicit="$tmp/package-root"
: >"$tmp/log"
FAKE_INSTALL_CONTENT=explicit-one FAKE_LOG="$tmp/log" CARGO="$tmp/install-cargo" RUSTUP="$tmp/rustup" GIT="$tmp/git" PREFIX="$explicit" \
    make -C "$root" install >/dev/null
test -x "$explicit/bin/asb"
grep -F -- '# explicit-one' "$explicit/bin/asb" >/dev/null
printf '%s\n' 'keep-explicit' >"$explicit/unrelated"
FAKE_INSTALL_CONTENT=explicit-two FAKE_LOG="$tmp/log" CARGO="$tmp/install-cargo" RUSTUP="$tmp/rustup" GIT="$tmp/git" PREFIX="$explicit" \
    make -C "$root" install >/dev/null
grep -F -- '# explicit-two' "$explicit/bin/asb" >/dev/null
grep -F -- 'keep-explicit' "$explicit/unrelated" >/dev/null

# Prove the fake models the original failure: an existing binary is rejected
# when Cargo's overwrite option is omitted. The production Make invocation above
# must therefore include --force for both repeat-install assertions to pass.
if FAKE_INSTALL_CONTENT=without-force FAKE_LOG="$tmp/log" \
    "$tmp/install-cargo" +1.93.0 install --locked --path crates/asb-cli --root "$explicit" \
    >/dev/null 2>&1; then
    printf '%s\n' 'fake Cargo accepted an existing binary without --force' >&2
    exit 1
fi

space_home="$tmp/home with space"
mkdir -p "$space_home"
HOME="$space_home" FAKE_INSTALL_CONTENT=space FAKE_LOG="$tmp/log" CARGO="$tmp/install-cargo" RUSTUP="$tmp/rustup" GIT="$tmp/git" \
    make -C "$root" install >/dev/null
test -x "$space_home/.local/bin/asb"

symlink_prefix="$tmp/symlink-prefix"
outside="$tmp/outside"
mkdir -p "$symlink_prefix" "$outside"
ln -s "$outside" "$symlink_prefix/bin"
if PREFIX="$symlink_prefix" CARGO="$tmp/install-cargo" RUSTUP="$tmp/rustup" GIT="$tmp/git" \
    make -C "$root" install >/dev/null 2>&1; then
    printf '%s\n' 'symlinked PREFIX/bin unexpectedly passed' >&2
    exit 1
fi
test ! -e "$outside/asb"

clean_prefix="$tmp/clean-symlink-prefix"
mkdir -p "$clean_prefix" "$outside"
ln -s "$outside" "$clean_prefix/bin"
printf 'protected\n' >"$outside/asb"
touch "$clean_prefix/.asb-make-staging"
if PREFIX="$clean_prefix" CARGO="$tmp/install-cargo" RUSTUP="$tmp/rustup" GIT="$tmp/git" \
    make -C "$root" clean >/dev/null 2>&1; then
    printf '%s\n' 'symlinked clean PREFIX/bin unexpectedly passed' >&2
    exit 1
fi
test -f "$outside/asb"

if PREFIX=relative-prefix CARGO="$tmp/install-cargo" RUSTUP="$tmp/rustup" GIT="$tmp/git" \
    make -C "$root" install >/dev/null 2>&1; then
    printf '%s\n' 'relative PREFIX unexpectedly passed' >&2
    exit 1
fi
if PREFIX= CARGO="$tmp/install-cargo" RUSTUP="$tmp/rustup" GIT="$tmp/git" \
    make -C "$root" install >/dev/null 2>&1; then
    printf '%s\n' 'empty PREFIX unexpectedly passed' >&2
    exit 1
fi
if PREFIX=/ CARGO="$tmp/install-cargo" RUSTUP="$tmp/rustup" GIT="$tmp/git" \
    make -C "$root" install >/dev/null 2>&1; then
    printf '%s\n' 'root PREFIX unexpectedly passed' >&2
    exit 1
fi
if PREFIX="$root" CARGO="$tmp/install-cargo" RUSTUP="$tmp/rustup" GIT="$tmp/git" \
    make -C "$root" install >/dev/null 2>&1; then
    printf '%s\n' 'repository-root PREFIX unexpectedly passed' >&2
    exit 1
fi
if PREFIX="$tmp/safe/../escape" CARGO="$tmp/install-cargo" RUSTUP="$tmp/rustup" GIT="$tmp/git" \
    make -C "$root" install >/dev/null 2>&1; then
    printf '%s\n' 'traversing PREFIX unexpectedly passed' >&2
    exit 1
fi
not_directory="$tmp/not-directory"
printf '%s\n' 'not a directory' >"$not_directory"
if PREFIX="$not_directory/child" CARGO="$tmp/install-cargo" RUSTUP="$tmp/rustup" GIT="$tmp/git" \
    make -C "$root" install >/dev/null 2>&1; then
    printf '%s\n' 'non-directory PREFIX component unexpectedly passed' >&2
    exit 1
fi
destination_prefix="$tmp/destination-symlink-prefix"
mkdir -p "$destination_prefix/bin"
ln -s "$outside/asb" "$destination_prefix/bin/asb"
if PREFIX="$destination_prefix" CARGO="$tmp/install-cargo" RUSTUP="$tmp/rustup" GIT="$tmp/git" \
    make -C "$root" install >/dev/null 2>&1; then
    printf '%s\n' 'symlinked PREFIX/bin/asb unexpectedly passed' >&2
    exit 1
fi
grep -F -- 'protected' "$outside/asb" >/dev/null
printf '%s\n' 'repository Makefile checks passed'
