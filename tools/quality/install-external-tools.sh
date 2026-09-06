#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
set -Eeuo pipefail
IFS=$'\n\t'

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
manifest="$repo_root/config/quality-tools.json"
destination="${1:-${RUNNER_TEMP:-$repo_root/target}/asb-quality-tools}"

case "$(uname -m)" in
  x86_64) platform=linux_x86_64 ;;
  aarch64 | arm64) platform=linux_aarch64 ;;
  *)
    printf 'unsupported quality-tool architecture: %s\n' "$(uname -m)" >&2
    exit 1
    ;;
esac

mkdir -p "$destination/bin" "$destination/cache"
destination="$(realpath -e "$destination")"

for tool in actionlint zizmor gitleaks; do
  version="$(jq -er --arg tool "$tool" '.external[$tool].version' "$manifest")"
  repository="$(jq -er --arg tool "$tool" '.external[$tool].repository' "$manifest")"
  asset="$(jq -er --arg tool "$tool" --arg platform "$platform" '.external[$tool][$platform].asset' "$manifest")"
  digest="$(jq -er --arg tool "$tool" --arg platform "$platform" '.external[$tool][$platform].sha256' "$manifest")"
  archive="$destination/cache/$asset"
  if [[ ! -f "$archive" ]]; then
    url="https://github.com/$repository/releases/download/v$version/$asset"
    temporary="$archive.partial.$$"
    trap 'rm -f -- "$temporary"' EXIT
    curl --fail --location --retry 3 --proto '=https' --output "$temporary" "$url"
    mv -- "$temporary" "$archive"
    trap - EXIT
  fi
  printf '%s  %s\n' "$digest" "$archive" | sha256sum --check --strict >/dev/null
  extract="$(mktemp -d "$destination/.extract.XXXXXX")"
  trap 'rm -rf -- "$extract"' EXIT
  tar --extract --gzip --file "$archive" --directory "$extract"
  binary="$(find "$extract" -type f -name "$tool" -perm -u+x -print -quit)"
  [[ -n "$binary" ]] || {
    printf '%s archive does not contain its executable\n' "$tool" >&2
    exit 1
  }
  install -m 0755 "$binary" "$destination/bin/$tool"
  rm -rf -- "$extract"
  trap - EXIT
done

"$destination/bin/actionlint" -version | grep -F "1.7.12" >&2
"$destination/bin/zizmor" --version | grep -F "1.30.0" >&2
"$destination/bin/gitleaks" version | grep -F "8.30.1" >&2
printf '%s\n' "$destination/bin"
