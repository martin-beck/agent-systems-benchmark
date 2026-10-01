#!/bin/sh
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
set -eu

root=${ASB_DEV_TOOLCHAIN_ROOT:-${HOME:?}/.cache/asb/dev-toolchain-v1}
case "$root" in /*) ;; *) printf '%s\n' 'ERROR: toolchain root must be absolute' >&2; exit 3 ;; esac
case "$root" in *[!A-Za-z0-9._/-]*|*/../*|*/..|*/./*|*/.) printf '%s\n' 'ERROR: toolchain root is malformed' >&2; exit 3 ;; esac
parent=$(dirname "$root")
mkdir -p "$parent"
chmod 0700 "$parent"
test "$(stat -c '%a' "$parent")" = 700 || { printf '%s\n' 'ERROR: toolchain parent is not private' >&2; exit 3; }
test ! -L "$root" || { printf '%s\n' 'ERROR: toolchain root is a symlink' >&2; exit 3; }

source_cargo=${ASB_DEV_SOURCE_CARGO:-$(command -v cargo 2>/dev/null || true)}
source_git=${ASB_DEV_SOURCE_GIT:-$(command -v git 2>/dev/null || true)}
source_setsid=${ASB_DEV_SOURCE_SETSID:-$(command -v setsid 2>/dev/null || true)}
for tool in cargo git setsid; do
    eval "source=\${source_$tool:-}"
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

stage="$parent/.dev-toolchain-stage-$$"
test ! -e "$stage" || { printf '%s\n' 'ERROR: staging collision' >&2; exit 4; }
trap 'rm -rf -- "$stage"' EXIT HUP INT TERM
mkdir -m 0700 "$stage"
mkdir -m 0700 "$stage/bin"
install -m 0700 "$source_git" "$stage/bin/git"
install -m 0700 "$source_setsid" "$stage/bin/setsid"

cat > "$stage/bin/cargo" <<EOF
#!/bin/sh
set -eu
export HOME=$(printf '%s' "${HOME}")
export CARGO_HOME=$(printf '%s' "${HOME}/.cargo")
export RUSTUP_HOME=$(printf '%s' "${HOME}/.rustup")
export PATH=$(printf '%s' "${HOME}/.cargo/bin:/usr/local/bin:/usr/bin:/bin")
exec $(printf '%s' "$source_cargo") "\$@"
EOF
chmod 0700 "$stage/bin/cargo"
printf 'schema=asb-development-toolchain-v1\nroot=%s\ncargo=%s\ngit=%s\nsetsid=%s\n' \
    "$root" "$source_cargo" "$source_git" "$source_setsid" > "$stage/manifest"
chmod 0600 "$stage/manifest"
if test -e "$root"; then
    test -d "$root" && test ! -L "$root" || { printf '%s\n' 'ERROR: existing toolchain root is unsafe' >&2; exit 4; }
    test "$(stat -c '%a' "$root")" = 700 || { printf '%s\n' 'ERROR: existing toolchain root is not private' >&2; exit 4; }
    rm -rf -- "$root/bin" "$root/manifest"
else
    mkdir -m 0700 "$root"
fi
mv "$stage/bin" "$root/bin"
mv "$stage/manifest" "$root/manifest"
trap - EXIT HUP INT TERM
printf 'development toolchain ready: %s\n' "$root"
