#!/bin/sh
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
set -eu

root=${ASB_DEV_TOOLCHAIN_ROOT:-${HOME:?}/.cache/asb/dev-toolchain-v1}
case "$root" in /*) ;; *) printf '%s\n' 'ERROR: toolchain root must be absolute' >&2; exit 3 ;; esac
case "$root" in *[!A-Za-z0-9._/-]*|*/../*|*/..|*/./*|*/.) printf '%s\n' 'ERROR: toolchain root is malformed' >&2; exit 3 ;; esac
parent=$(dirname "$root")
uid=$(id -u)
validate_ancestors() {
ancestor="$1"
while :; do
    test -d "$ancestor" && test ! -L "$ancestor" || { printf '%s\n' 'ERROR: toolchain ancestor is unsafe' >&2; exit 3; }
    owner=$(stat -c '%u' "$ancestor")
    test "$owner" = "$uid" || test "$owner" = 0 || { printf '%s\n' 'ERROR: toolchain ancestor has unexpected owner' >&2; exit 3; }
    if find "$ancestor" -prune -perm /022 -print -quit | grep -q .; then
        mode=$(stat -c '%a' "$ancestor")
        test "$owner:$mode" = '0:1777' || { printf '%s\n' 'ERROR: toolchain ancestor is writable' >&2; exit 3; }
    fi
    test "$ancestor" = / && break
    next=$(dirname "$ancestor")
    test "$next" != "$ancestor" || break
    ancestor="$next"
done
}
existing="$parent"
while test ! -e "$existing"; do
    next=$(dirname "$existing")
    test "$next" != "$existing" || break
    existing="$next"
done
validate_ancestors "$existing"
mkdir -p "$parent"
validate_ancestors "$parent"
test ! -L "$root" || { printf '%s\n' 'ERROR: toolchain root is a symlink' >&2; exit 3; }

source_cargo=${ASB_DEV_SOURCE_CARGO:-$(command -v cargo 2>/dev/null || true)}
source_git=${ASB_DEV_SOURCE_GIT:-$(command -v git 2>/dev/null || true)}
source_setsid=${ASB_DEV_SOURCE_SETSID:-$(command -v setsid 2>/dev/null || true)}
for tool in cargo git setsid; do
    case "$tool" in
        cargo) source=${source_cargo:-} ;;
        git) source=${source_git:-} ;;
        setsid) source=${source_setsid:-} ;;
    esac
    test -n "$source" && source=$(realpath -e "$source") && test -f "$source" && test ! -L "$source" && test -x "$source" || {
        printf 'ERROR: source %s is unavailable\n' "$tool" >&2
        exit 4
    }
    case "$source" in /*) ;; *) printf 'ERROR: source %s is not absolute\n' "$tool" >&2; exit 4 ;; esac
    case "$source" in *[!A-Za-z0-9._/-]*) printf 'ERROR: source %s path is malformed\n' "$tool" >&2; exit 4 ;; esac
    if find "$source" -prune -perm /022 -print -quit | grep -q .; then
        printf 'ERROR: source %s is writable by group or other\n' "$tool" >&2
        exit 4
    fi
done

cargo_binary="$source_cargo"
if test "$(basename "$source_cargo")" = rustup; then
    cargo_binary=$("$source_cargo" which cargo 2>/dev/null || true)
    cargo_binary=$(realpath -e "$cargo_binary" 2>/dev/null || true)
fi
test -f "$cargo_binary" && test -x "$cargo_binary" || {
    printf '%s\n' 'ERROR: source cargo binary is unavailable' >&2
    exit 4
}
cargo_bin_dir=$(dirname "$cargo_binary")

stage="$parent/.dev-toolchain-stage-$$"
test ! -e "$stage" || { printf '%s\n' 'ERROR: staging collision' >&2; exit 4; }
trap 'rm -rf -- "$stage"' EXIT HUP INT TERM
mkdir -m 0700 "$stage"
mkdir -m 0700 "$stage/bin"
install -m 0700 "$source_git" "$stage/bin/git"
install -m 0700 "$source_setsid" "$stage/bin/setsid"
install -m 0700 "$cargo_binary" "$stage/bin/cargo"

cat > "$stage/bin/cargo-wrapper" <<EOF
#!/bin/sh
set -eu
export PATH=$(printf '%s' "${cargo_bin_dir}:$root/bin")
exec $(printf '%s' "$root/bin/cargo") "\$@"
EOF
chmod 0700 "$stage/bin/cargo-wrapper"
printf 'schema=asb-development-toolchain-v1\nroot=%s\ncargo=%s\ngit=%s\nsetsid=%s\n' \
    "$root" "$source_cargo" "$source_git" "$source_setsid" > "$stage/manifest"
chmod 0600 "$stage/manifest"
if test -e "$root"; then
    test -d "$root" && test ! -L "$root" || { printf '%s\n' 'ERROR: existing toolchain root is unsafe' >&2; exit 4; }
    test "$(stat -c '%a' "$root")" = 700 || { printf '%s\n' 'ERROR: existing toolchain root is not private' >&2; exit 4; }
    backup="$parent/.dev-toolchain-backup-$$"
    test ! -e "$backup" || { printf '%s\n' 'ERROR: backup collision' >&2; exit 4; }
    mv "$root" "$backup"
    if ! mv "$stage" "$root"; then
        mv "$backup" "$root"
        printf '%s\n' 'ERROR: toolchain replacement failed; prior installation restored' >&2
        exit 4
    fi
    rm -rf -- "$backup"
else
    mv "$stage" "$root"
fi
trap - EXIT HUP INT TERM
printf 'development toolchain ready: %s\n' "$root"
