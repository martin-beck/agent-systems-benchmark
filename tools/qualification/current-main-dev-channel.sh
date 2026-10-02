#!/bin/sh
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
#
# Disposable qualification for the development channel.  This intentionally
# builds from clean clones and never writes to a checkout or the stable link.
set -eu

die() { printf '%s\n' "ERROR: $*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "required tool unavailable: $1"; }
need git; need cargo; need mktemp; need mv; need ln; need cp; need install; need sha256sum; need du
need timeout; need python3

asb_url=${ASB_REPOSITORY_URL:-https://github.com/martin-beck/agent-systems-benchmark.git}
tui_url=${ASB_TUI_REPOSITORY_URL:-https://github.com/martin-beck/asb-tui.git}
asb_ref=${ASB_REF:-main}; tui_ref=${ASB_TUI_REF:-main}
install_root=${ASB_DEV_INSTALL_ROOT:-${TMPDIR:-/tmp}/asb-dev-install}
stable_link=${ASB_STABLE_LINK:-$install_root/stable}
timeout_seconds=${ASB_DEV_TIMEOUT_SECONDS:-900}
quota_kib=${ASB_DEV_WORKSPACE_QUOTA_KIB:-2097152}

case "$install_root:$stable_link" in /*:/*) ;; *) die 'installation paths must be absolute' ;; esac
case "$timeout_seconds:$quota_kib" in *[!0-9:]*|:*) die 'timeout and quota must be positive integers' ;; esac
test "$timeout_seconds" -gt 0 && test "$quota_kib" -gt 0 || die 'timeout and quota must be positive integers'

workspace=$(mktemp -d "${TMPDIR:-/tmp}/asb-dev-channel.XXXXXX")
trap 'rm -rf -- "$workspace"' EXIT HUP INT TERM
chmod 700 "$workspace"
log=$workspace/transcript.log
say() { printf '%s\n' "$*" | tee -a "$log"; }
run_bounded() { timeout --signal=TERM --kill-after=15s "$timeout_seconds" "$@" >>"$log" 2>&1; }

say "channel=dev asb_ref=$asb_ref tui_ref=$tui_ref"
run_bounded git clone --filter=blob:none --no-tags --depth=1 --branch "$asb_ref" "$asb_url" "$workspace/asb" \
    || die 'ASB clean clone/build timed out or failed'
run_bounded git clone --filter=blob:none --no-tags --depth=1 --branch "$tui_ref" "$tui_url" "$workspace/asb-tui" \
    || die 'TUI clean clone/build timed out or failed'
asb_head=$(git -C "$workspace/asb" rev-parse HEAD)
tui_head=$(git -C "$workspace/asb-tui" rev-parse HEAD)
say "asb_head=$asb_head tui_head=$tui_head"

run_bounded cargo build --locked --release --manifest-path "$workspace/asb/Cargo.toml" --bin asb \
    || die 'ASB current-main build failed'
run_bounded cargo build --locked --release --manifest-path "$workspace/asb-tui/Cargo.toml" --bin asb-tui \
    || die 'TUI current-main build failed'
asb_bin=$workspace/asb/target/release/asb; tui_bin=$workspace/asb-tui/target/release/asb-tui
test -x "$asb_bin" && test -x "$tui_bin" || die 'build did not produce both entrypoints'

used=$(du -sk "$workspace" | awk '{print $1}')
test "$used" -le "$quota_kib" || die "clean workspace exceeds ${quota_kib} KiB quota (${used} KiB)"
asb_digest=$(sha256sum "$asb_bin" | awk '{print $1}')
tui_digest=$(sha256sum "$tui_bin" | awk '{print $1}')
release_id="dev-${asb_head}-${tui_head}"
stage=$install_root/.stage-$release_id-$$
release=$install_root/releases/$release_id
install -d -m 700 "$install_root/releases"
test ! -e "$release" || die "release already exists: $release"
rm -rf -- "$stage"
mkdir -m 700 "$stage"
cp "$asb_bin" "$stage/asb"; cp "$tui_bin" "$stage/asb-tui"
chmod 700 "$stage/asb" "$stage/asb-tui"
printf '%s\n' "channel=dev" "asb_head=$asb_head" "tui_head=$tui_head" \
    "asb_sha256=$asb_digest" "tui_sha256=$tui_digest" > "$stage/manifest"
chmod 600 "$stage/manifest"
mv "$stage" "$release"

previous=''
if test -L "$install_root/dev"; then previous=$(readlink "$install_root/dev"); fi
ln -s "releases/$release_id" "$install_root/dev.new-$$"
mv -Tf "$install_root/dev.new-$$" "$install_root/dev"
test -x "$install_root/dev/asb" && test -x "$install_root/dev/asb-tui" || die 'atomic dev publication incomplete'
run_bounded "$install_root/dev/asb" --help || die 'dev ASB restart/entrypoint check failed'
tui_probe=$workspace/tui-doctor.json
set +e
timeout --signal=TERM --kill-after=15s "$timeout_seconds" "$install_root/dev/asb-tui" doctor --format json >"$tui_probe" 2>>"$log"
tui_status=$?
set -e
# A source-only TUI may intentionally return 3 while still emitting its
# machine-readable startup/compatibility diagnosis. Any other status is a
# failed restart check.
test "$tui_status" -eq 0 || test "$tui_status" -eq 3 || die 'dev TUI restart/entrypoint check failed'
python3 - "$tui_probe" <<'PY' || die 'dev TUI did not emit a JSON startup diagnosis'
import json, sys
with open(sys.argv[1], encoding="utf-8") as handle:
    value = json.load(handle)
if not isinstance(value, dict) or value.get("protocol") != "asb-cli-capabilities":
    raise SystemExit(1)
PY
test "$(sha256sum "$install_root/dev/asb" | awk '{print $1}')" = "$asb_digest"
test "$(sha256sum "$install_root/dev/asb-tui" | awk '{print $1}')" = "$tui_digest"

if test -n "$previous"; then
    ln -s "$previous" "$install_root/dev.rollback-$$"; mv -Tf "$install_root/dev.rollback-$$" "$install_root/dev"
    test "$(readlink "$install_root/dev")" = "$previous" || die 'rollback did not restore prior dev release'
    ln -s "releases/$release_id" "$install_root/dev.restore-$$"; mv -Tf "$install_root/dev.restore-$$" "$install_root/dev"
else
    test "$(readlink "$install_root/dev")" = "releases/$release_id"
fi

if test -L "$stable_link"; then
    stable_before=$(readlink "$stable_link")
    test "$(readlink "$stable_link")" = "$stable_before" || die 'stable channel changed during dev install'
    test "$stable_before" != "releases/$release_id" || die 'dev release leaked into stable channel'
else
    say 'stable_isolation=unconfigured (no stable link was present)'
fi
say "qualified channel=dev asb_head=$asb_head tui_head=$tui_head asb_sha256=$asb_digest tui_sha256=$tui_digest workspace_kib=$used install=$install_root/dev"
say "transcript=$log"
