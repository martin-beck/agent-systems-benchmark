#!/bin/sh
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
set -eu
die() { printf '%s\n' "ERROR: $*" >&2; exit 1; }
data_root=${ASB_DATA_ROOT:-${XDG_DATA_HOME:-$HOME/.local/share}/asb}
config_root=${ASB_CONFIG_ROOT:-${XDG_CONFIG_HOME:-$HOME/.config}/asb}
runtime_root=${ASB_RUNTIME_ROOT:-${XDG_RUNTIME_DIR:-$HOME/.local/run}/asb}
case "$data_root:$config_root:$runtime_root" in /*:*:/*) ;; *) die 'installation roots must be absolute' ;; esac
current=$data_root/current; previous_file=$data_root/.previous-release; backup_root=$data_root/backups
service_unit=${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/asb-runner.service
backup_state() {
    install -d -m 0700 "$backup_root"; stamp=$(date -u +%Y%m%dT%H%M%SZ); destination=$backup_root/$stamp
    (umask 077; mkdir "$destination") || die 'backup destination already exists'
    if test -f "$config_root/config.toml" && test ! -L "$config_root/config.toml"; then install -m 0600 "$config_root/config.toml" "$destination/config.toml"; fi
    for name in runs.json index.json; do if test -f "$data_root/$name" && test ! -L "$data_root/$name"; then install -m 0600 "$data_root/$name" "$destination/$name"; fi; done
    find "$backup_root" -mindepth 1 -maxdepth 1 -type d -printf '%T@ %p\n' | sort -rn | tail -n +4 | cut -d' ' -f2- | while IFS= read -r old; do rm -rf -- "$old"; done
    printf '%s\n' "$destination"
}
status() { test -L "$current" || die 'no active installation'; printf 'current=%s\n' "$(readlink "$current")"; test ! -f "$previous_file" || printf 'previous=%s\n' "$(sed -n '1p' "$previous_file")"; printf 'config=%s\nbackups=%s\n' "$config_root/config.toml" "$backup_root"; }
rollback() {
    test -f "$previous_file" || die 'no rollback release recorded'; target=$(sed -n '1p' "$previous_file")
    case "$target" in releases/*) ;; *) die 'rollback record is malformed' ;; esac
    test -x "$data_root/$target/asb" || die 'rollback release is unavailable'; "$data_root/$target/asb" doctor >/dev/null || die 'rollback health check failed'
    readlink "$current" > "$previous_file.new"; chmod 0600 "$previous_file.new"; mv -f "$previous_file.new" "$previous_file"
    ln -s "$target" "$current.new"; mv -Tf "$current.new" "$current"
}
repair() {
    test -L "$current" || die 'no active installation to repair'; target=$(readlink "$current")
    case "$target" in releases/*) ;; *) die 'active installation link is malformed' ;; esac
    test -x "$data_root/$target/asb" || die 'active release is incomplete'; install -d -m 0700 "$config_root"
    if test ! -f "$config_root/config.toml" || test -L "$config_root/config.toml"; then printf '%s\n' '# ASB user configuration; add explicit provider references.' > "$config_root/config.toml"; fi
    chmod 0600 "$config_root/config.toml"; "$data_root/$target/asb" doctor >/dev/null || die 'active release health check failed'
}
uninstall() { rm -f -- "$service_unit" "$current"; printf '%s\n' "retained-data=$data_root" "retained-config=$config_root" "retained-runtime=$runtime_root" 'binaries and releases retained; purge requires a separate explicit operation.'; }
case "${1:-}" in status) status ;; backup) backup_state ;; rollback) backup_state >/dev/null; rollback ;; repair) repair ;; uninstall) uninstall ;; *) die 'usage: lifecycle.sh {status|backup|rollback|repair|uninstall}' ;; esac
