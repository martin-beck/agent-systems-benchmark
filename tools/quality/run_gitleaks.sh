#!/usr/bin/env bash
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
set -euo pipefail

usage() {
  echo "usage: $0 --gitleaks PATH --config PATH --head SHA [--base SHA] [--repo PATH]" >&2
  exit 2
}

gitleaks=
config=
base=
head=
repo=.
while (($#)); do
  case "$1" in
    --gitleaks) (($# >= 2)) || usage; gitleaks=$2; shift 2 ;;
    --config) (($# >= 2)) || usage; config=$2; shift 2 ;;
    --base) (($# >= 2)) || usage; base=$2; shift 2 ;;
    --head) (($# >= 2)) || usage; head=$2; shift 2 ;;
    --repo) (($# >= 2)) || usage; repo=$2; shift 2 ;;
    *) usage ;;
  esac
done

[[ -n "$gitleaks" && -x "$gitleaks" && -n "$config" && -n "$head" ]] || usage
[[ -d "$repo/.git" || -f "$repo/.git" ]] || { echo "gitleaks scan requires a Git checkout" >&2; exit 2; }
config_abs=$(realpath -e -- "$config") || { echo "gitleaks config is unavailable" >&2; exit 2; }
repo_abs=$(realpath -e -- "$repo") || { echo "gitleaks repository is unavailable" >&2; exit 2; }
[[ "$config_abs" == "$repo_abs/"* ]] || { echo "gitleaks config must be below the repository" >&2; exit 2; }
[[ ! -L "$config" && -f "$config" ]] || { echo "gitleaks config must be a regular non-symlink file" >&2; exit 2; }

git -C "$repo" rev-parse --verify "$head^{commit}" >/dev/null || { echo "gitleaks head is not a commit" >&2; exit 2; }
if [[ -n "$base" ]]; then
  git -C "$repo" rev-parse --verify "$base^{commit}" >/dev/null || { echo "gitleaks base is not a commit" >&2; exit 2; }
  git -C "$repo" merge-base --is-ancestor "$base" "$head" || { echo "gitleaks base is not an ancestor of head" >&2; exit 2; }
  log_opts="$base..$head"
else
  log_opts="$head"
fi

exec "$gitleaks" git --redact --no-banner --config "$config_abs" --log-opts="$log_opts" "$repo_abs"
