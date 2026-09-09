#!/bin/sh
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
set -eu

die() { printf '%s\n' "ERROR: $*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "required tool unavailable: $1"; }
case "${ASB_MANIFEST_URL:-}" in
    https://?*) : ;;
    *) die 'ASB_MANIFEST_URL must be an HTTPS URL' ;;
esac
case "${ASB_MANIFEST_SHA256:-}" in
    [0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]*)
        test "${#ASB_MANIFEST_SHA256}" -eq 64 || die 'ASB_MANIFEST_SHA256 must be lowercase SHA-256' ;;
    *) die 'ASB_MANIFEST_SHA256 is required' ;;
esac
need curl
need sha256sum
need python3
need install
need mv
need mktemp
need ssh-keygen

umask 077
data_root=${ASB_DATA_ROOT:-${XDG_DATA_HOME:-$HOME/.local/share}/asb}
config_root=${ASB_CONFIG_ROOT:-${XDG_CONFIG_HOME:-$HOME/.config}/asb}
runtime_root=${ASB_RUNTIME_ROOT:-${XDG_RUNTIME_DIR:-$HOME/.local/run}/asb}
case "$data_root:$config_root:$runtime_root" in
    /*:*:/*) : ;;
    *) die 'installation roots must be absolute' ;;
esac
install -d -m 0700 "$data_root" "$config_root" "$runtime_root"

tmp=$(mktemp -d "${TMPDIR:-/tmp}/asb-install.XXXXXX")
cleanup() { rm -rf -- "$tmp"; }
trap cleanup EXIT HUP INT TERM
manifest=$tmp/manifest.json
curl --fail --silent --show-error --location --proto '=https' --tlsv1.2 \
    --output "$manifest" "$ASB_MANIFEST_URL"
observed=$(sha256sum "$manifest" | awk '{print $1}')
test "$observed" = "$ASB_MANIFEST_SHA256" || die 'release manifest digest mismatch'

python3 - "$manifest" "$tmp/selection" <<'PY'
import datetime as dt
import json
import platform
import sys
from urllib.parse import urlparse

source, target = sys.argv[1:]
with open(source, encoding="utf-8") as handle:
    value = json.load(handle)
if value.get("schema_version") != 1:
    raise SystemExit("unsupported release manifest")
try:
    expires = dt.datetime.fromisoformat(value["expires_at"].replace("Z", "+00:00"))
except (KeyError, ValueError) as error:
    raise SystemExit("invalid release expiry") from error
if expires <= dt.datetime.now(dt.timezone.utc):
    raise SystemExit("release manifest is expired")
machine = platform.machine().lower()
architecture = {"amd64": "x86_64", "aarch64": "aarch64"}.get(machine, machine)
libc_name, libc_version = platform.libc_ver()
matches = [artifact for artifact in value.get("artifacts", [])
           if artifact.get("target", {}).get("operating_system") == platform.system().lower()
           and artifact.get("target", {}).get("architecture") == architecture
           and artifact.get("target", {}).get("libc") == (libc_name or "unknown")
           and artifact.get("target", {}).get("libc_version") == libc_version]
if len(matches) != 1:
    raise SystemExit("no unique exact native artifact")
artifact = matches[0]
url = artifact.get("url", "")
if (not url.startswith("https://") or urlparse(url).query or urlparse(url).fragment
        or len(artifact.get("sha256", "")) != 64
        or len(artifact.get("signature_sha256", "")) != 64
        or not isinstance(artifact.get("size"), int) or artifact["size"] <= 0):
    raise SystemExit("malformed selected artifact")
with open(target, "w", encoding="ascii") as handle:
    handle.write(json.dumps(artifact, sort_keys=True, separators=(",", ":")) + "\n")
PY

artifact_url=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["url"])' "$tmp/selection")
artifact_id=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["artifact_id"])' "$tmp/selection")
artifact_sha=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["sha256"])' "$tmp/selection")
artifact_size=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["size"])' "$tmp/selection")
signature_sha=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["signature_sha256"])' "$tmp/selection")
case "$artifact_id" in ''|*[!A-Za-z0-9._-]*) die 'artifact id is malformed' ;; esac
archive=$tmp/artifact
curl --fail --silent --show-error --location --proto '=https' --tlsv1.2 \
    --output "$archive" "$artifact_url"
test "$(stat -c '%s' "$archive")" = "$artifact_size" || die 'artifact size mismatch'
test "$(sha256sum "$archive" | awk '{print $1}')" = "$artifact_sha" || die 'artifact digest mismatch'
signature=$tmp/artifact.sig
signature_url=${ASB_ARTIFACT_SIGNATURE_URL:-$artifact_url.sig}
case "$signature_url" in https://?*) : ;; *) die 'artifact signature URL must be HTTPS' ;; esac
curl --fail --silent --show-error --location --proto '=https' --tlsv1.2 \
    --output "$signature" "$signature_url"
test "$(sha256sum "$signature" | awk '{print $1}')" = "$signature_sha" || die 'artifact signature digest mismatch'
test -n "${ASB_ALLOWED_SIGNERS_FILE:-}" || die 'ASB_ALLOWED_SIGNERS_FILE is required'
test -f "$ASB_ALLOWED_SIGNERS_FILE" && test ! -L "$ASB_ALLOWED_SIGNERS_FILE" || die 'allowed-signers file is unavailable'
test -n "${ASB_SIGNATURE_PRINCIPAL:-}" || die 'ASB_SIGNATURE_PRINCIPAL is required'
ssh-keygen -Y verify -f "$ASB_ALLOWED_SIGNERS_FILE" -I "$ASB_SIGNATURE_PRINCIPAL" \
    -n asb-runtime-bundle-v1 -s "$signature" < "$archive" >/dev/null 2>&1 \
    || die 'artifact signature verification failed'

release_root=$data_root/releases/$artifact_id
test ! -e "$release_root" || die 'refusing to overwrite an existing release'
install -d -m 0700 "$data_root/releases"
stage=$data_root/.stage-$artifact_id-$$
mkdir "$stage"
python3 - "$archive" "$stage" <<'PY'
import pathlib
import sys
import tarfile

archive, destination = sys.argv[1:]
root = pathlib.Path(destination).resolve()
with tarfile.open(archive, "r:*") as source:
    members = source.getmembers()
    for member in members:
        path = pathlib.PurePosixPath(member.name)
        if path.is_absolute() or ".." in path.parts or member.issym() or member.islnk():
            raise SystemExit("archive contains unsafe path topology")
        if not (member.isdir() or member.isfile()):
            raise SystemExit("archive contains unsupported entry type")
    source.extractall(root, filter="data")
for item in root.rglob("*"):
    if item.is_symlink() or not item.resolve().is_relative_to(root):
        raise SystemExit("archive extraction escaped its root")
PY
test -f "$stage/asb" && test -x "$stage/asb" || die 'release lacks executable asb entrypoint'
mv "$stage" "$release_root"
current_new=$data_root/current.new-$$
ln -s "releases/$artifact_id" "$current_new"
mv -Tf "$current_new" "$data_root/current"

if test ! -e "$config_root/config.toml"; then
    printf '%s\n' '# ASB user configuration; add explicit provider references.' > "$config_root/config.toml"
    chmod 0600 "$config_root/config.toml"
fi
"$data_root/current/asb" doctor >/dev/null || die 'installed asb doctor failed'

if test "${ASB_SERVICE_MODE:-auto}" != disabled; then
    unit_dir=${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user
    if command -v systemctl >/dev/null 2>&1 && systemctl --user --version >/dev/null 2>&1; then
        install -d -m 0700 "$unit_dir"
        unit=$unit_dir/asb-runner.service
        if test ! -e "$unit"; then
            printf '%s\n' '[Unit]' 'Description=ASB local runner' '[Service]' \
                "ExecStart=$data_root/current/asb serve $config_root/config.toml" \
                'Restart=on-failure' '[Install]' 'WantedBy=default.target' > "$unit"
            chmod 0600 "$unit"
        fi
        systemctl --user daemon-reload
        systemctl --user enable --now asb-runner.service
    else
        printf '%s\n' "No supported user service manager; supervise: $data_root/current/asb serve $config_root/config.toml" >&2
    fi
fi

printf 'installed artifact=%s root=%s\n' "$artifact_id" "$data_root/current"
if test -t 1 && test "${ASB_NO_TUI:-0}" != 1 && test -x "$data_root/current/asb-tui"; then
    exec "$data_root/current/asb-tui"
fi
