#!/usr/bin/env bash
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
set -euo pipefail
TLA_SHA256=8c200a88d151c6c183c8dbc57a6b633d135e7a2b18242a3afbf243a9e4b68d3e
ALLOY_SHA256=6b8c1cb5bc93bedfc7c61435c4e1ab6e688a242dc702a394628d9a9801edb78d
TLA_BYTES=4512486
ALLOY_BYTES=21062377
TLA_SOURCE_URL=https://codeload.github.com/tlaplus/tlaplus/tar.gz/b123b22654942bd7f8b1bcadcc47da4ee2cf4c0e
TLA_SOURCE_SHA256=1f96ee7ef950e456794d13b7e4d8123c345a91528257a3a41b7cc1b506d1b58f
TLA_SOURCE_BYTES=82989507
ANT_URL=https://archive.apache.org/dist/ant/binaries/apache-ant-1.10.15-bin.tar.gz
ANT_SHA256=71334d7e5d98cfe53d6c429a648a5021137a967378667306c5f613dff5180506
ANT_BYTES=6925830
TLA_BUILD_IMAGE=eclipse-temurin@sha256:c0d1549d1e0f5fa5b83622ec0033b00456107e0b1d0cfcce4c1d831532ce621e
ALLOY_URL=https://github.com/AlloyTools/org.alloytools.alloy/releases/download/v6.2.0/org.alloytools.alloy.dist.jar
if [[ $# -ne 2 || $1 != /* || $2 != /* ]]; then
  echo "usage: $0 ABSOLUTE_TOOL_DIR ABSOLUTE_NEW_SCRATCH_DIR" >&2
  exit 2
fi
if [[ ${ASB_FORMAL_OFFLINE:-0} != 0 && ${ASB_FORMAL_OFFLINE:-0} != 1 ]] \
   || [[ ${ASB_FORMAL_ACQUIRE_ONLY:-0} != 0 && ${ASB_FORMAL_ACQUIRE_ONLY:-0} != 1 ]]; then
  echo "formal mode flag is invalid" >&2
  exit 2
fi
tool_dir=$1
scratch_dir=$2
case "$scratch_dir" in
  /|"$tool_dir"|"$tool_dir"/*) echo "unsafe scratch directory" >&2; exit 2 ;;
esac
if [[ -e "$scratch_dir" ]]; then
  echo "scratch directory must not already exist" >&2
  exit 2
fi
mkdir -p "$tool_dir" "$scratch_dir/tlc/positive" "$scratch_dir/tlc/negative" \
  "$scratch_dir/alloy-positive/tmp" \
  "$scratch_dir/alloy-negative/tmp"
if [[ -L "$tool_dir" || -L "$scratch_dir" \
   || $(realpath -e "$tool_dir") != "$tool_dir" \
   || $(realpath -e "$scratch_dir") != "$scratch_dir" ]]; then
  echo "tool and scratch directories must be canonical non-symlink directories" >&2
  exit 2
fi
if [[ $(stat -c '%u' "$tool_dir") != $(id -u) \
   || $((8#$(stat -c '%a' "$tool_dir") & 8#022)) -ne 0 ]]; then
  echo "tool directory must be owner-controlled" >&2
  exit 2
fi
verify_file() {
  local path=$1 expected=$2 expected_bytes=$3
  [[ -f "$path" && ! -L "$path" \
     && $(stat -c '%u' "$path") == $(id -u) \
     && $(stat -c '%h' "$path") == 1 \
     && $((8#$(stat -c '%a' "$path") & 8#022)) -eq 0 \
     && $(stat -c '%s' "$path") == "$expected_bytes" ]] \
    && printf '%s  %s\n' "$expected" "$path" | sha256sum --check --status
}
verify_dir() {
  local path=$1
  [[ -d "$path" && ! -L "$path" \
     && $(stat -c '%u' "$path") == $(id -u) \
     && $((8#$(stat -c '%a' "$path") & 8#022)) -eq 0 ]]
}
acquire_input() (
  local url=$1 path=$2 expected=$3 expected_bytes=$4 approved_host=$5
  local lock="$path.lock" partial="$path.partial" effective acquired=0 created=0
  for _ in $(seq 1 100); do
    if mkdir -m 700 "$lock" 2>/dev/null; then acquired=1; break; fi
    sleep 0.1
  done
  [[ $acquired == 1 ]] || { echo "tool cache lock unavailable" >&2; return 2; }
  trap 'if [[ $created == 1 ]]; then rm -f "$partial"; fi; rmdir "$lock" 2>/dev/null || true' EXIT
  if [[ -e "$path" || -L "$path" ]]; then
    verify_file "$path" "$expected" "$expected_bytes" || {
      echo "cached build input failed integrity checks" >&2; return 2;
    }
    return
  fi
  [[ ${ASB_FORMAL_OFFLINE:-0} == 0 ]] || {
    echo "offline build input missing" >&2; return 2;
  }
  (set -o noclobber; : >"$partial") 2>/dev/null || {
    echo "partial build input already exists" >&2; return 2;
  }
  created=1
  chmod 600 "$partial"
  effective=$(curl --fail --silent --show-error --location --max-redirs 3 \
    --proto '=https' --proto-redir '=https' --tlsv1.2 --max-time 180 \
    --max-filesize "$expected_bytes" --retry 2 --retry-delay 1 --retry-max-time 180 \
    --output "$partial" --write-out '%{url_effective}' "$url")
  python3 - "$effective" "$approved_host" <<'PY'
import sys, urllib.parse
url = urllib.parse.urlsplit(sys.argv[1])
good = (url.scheme == "https" and url.hostname == sys.argv[2]
        and not url.username and not url.password and not url.fragment)
raise SystemExit(0 if good else 1)
PY
  verify_file "$partial" "$expected" "$expected_bytes" || {
    echo "downloaded build input failed integrity checks" >&2; return 2;
  }
  chmod 400 "$partial"
  ln -- "$partial" "$path" || {
    echo "build input destination appeared during acquisition" >&2; return 2;
  }
  rm -- "$partial"
  created=0
  verify_file "$path" "$expected" "$expected_bytes" || {
    echo "promoted build input failed integrity checks" >&2; return 2;
  }
)

prepare_tla() (
  local output=$1 cache="$tool_dir/tla-source-cache" acquired=0
  local lock="$output.acquire.lock"
  local build_cache built
  local docker_cmd=(docker)
  if [[ -e "$output" || -L "$output" ]]; then
    verify_file "$output" "$TLA_SHA256" "$TLA_BYTES" || {
      echo "cached TLA tool failed integrity checks" >&2; return 2;
    }
    return
  fi
  [[ ${ASB_FORMAL_OFFLINE:-0} == 0 ]] || { echo "offline TLA tool missing" >&2; return 2; }
  for _ in $(seq 1 1500); do
    if mkdir -m 700 "$lock" 2>/dev/null; then acquired=1; break; fi
    if verify_file "$output" "$TLA_SHA256" "$TLA_BYTES"; then return; fi
    sleep 0.2
  done
  [[ $acquired == 1 ]] || { echo "TLA build lock unavailable" >&2; return 2; }
  trap '[[ -z ${build_cache:-} ]] || rm -rf -- "$build_cache"; [[ -z ${built:-} ]] || rm -f -- "$built"; rmdir "$lock" 2>/dev/null || true' EXIT
  if verify_file "$output" "$TLA_SHA256" "$TLA_BYTES"; then return; fi
  mkdir -m 700 "$cache" 2>/dev/null || verify_dir "$cache" || {
    echo "source cache directory must be owner-controlled" >&2; return 2;
  }
  verify_dir "$cache" || {
    echo "source cache directory must be owner-controlled" >&2; return 2;
  }
  acquire_input "$TLA_SOURCE_URL" "$cache/tlaplus-b123b226.tar.gz" \
    "$TLA_SOURCE_SHA256" "$TLA_SOURCE_BYTES" codeload.github.com
  acquire_input "$ANT_URL" "$cache/apache-ant-1.10.15-bin.tar.gz" \
    "$ANT_SHA256" "$ANT_BYTES" archive.apache.org
  build_cache=$(mktemp -d "$tool_dir/.tla-build-inputs.XXXXXXXX")
  chmod 700 "$build_cache"
  cp --reflink=never --no-preserve=mode,ownership,timestamps \
    "$cache/tlaplus-b123b226.tar.gz" "$build_cache/tlaplus-b123b226.tar.gz"
  cp --reflink=never --no-preserve=mode,ownership,timestamps \
    "$cache/apache-ant-1.10.15-bin.tar.gz" "$build_cache/apache-ant-1.10.15-bin.tar.gz"
  chmod 400 "$build_cache"/*.tar.gz
  if ! verify_file "$build_cache/tlaplus-b123b226.tar.gz" \
      "$TLA_SOURCE_SHA256" "$TLA_SOURCE_BYTES" \
     || ! verify_file "$build_cache/apache-ant-1.10.15-bin.tar.gz" \
      "$ANT_SHA256" "$ANT_BYTES"; then
    echo "private build input snapshot failed integrity checks" >&2
    return 2
  fi
  if ! docker info >/dev/null 2>&1; then
    docker_cmd=(sudo -n docker)
  fi
  if ! "${docker_cmd[@]}" image inspect "$TLA_BUILD_IMAGE" >/dev/null 2>&1; then
    timeout --signal=TERM 180 "${docker_cmd[@]}" pull "$TLA_BUILD_IMAGE" >/dev/null
  fi
  [[ $("${docker_cmd[@]}" image inspect --format '{{.Id}} {{.Os}}/{{.Architecture}}' "$TLA_BUILD_IMAGE") \
     == "sha256:c0d1549d1e0f5fa5b83622ec0033b00456107e0b1d0cfcce4c1d831532ce621e linux/amd64" ]] || {
    echo "TLA build image identity differs" >&2; return 2;
  }
  built="$build_cache/tla2tools.jar"
  "$(dirname "$0")/tla-provenance/build.sh" "$build_cache" "$built"
  chmod 400 "$built"
  verify_file "$built" "$TLA_SHA256" "$TLA_BYTES" || {
    echo "source-built TLA tool failed integrity checks" >&2; return 2;
  }
  ln -- "$built" "$output" || {
    echo "TLA output destination appeared during build" >&2; return 2;
  }
  rm -- "$built"
  built=
  verify_file "$output" "$TLA_SHA256" "$TLA_BYTES" || return 2
)
fetch() {
  local url=$1 path=$2 expected=$3 expected_bytes=$4
  if [[ -e "$path" || -L "$path" ]]; then
    if [[ ! -f "$path" || -L "$path" ]]; then
      echo "tool path must be a regular non-symlink file: $path" >&2
      exit 2
    fi
  else
    if [[ ${ASB_FORMAL_OFFLINE:-0} == 1 ]]; then
      echo "offline tool missing: $path" >&2
      exit 2
    fi
    if [[ -e "$path.partial" || -L "$path.partial" ]]; then
      echo "partial tool path already exists" >&2
      exit 2
    fi
    curl --fail --location --proto '=https' --tlsv1.2 --max-time 120 \
      --max-filesize "$expected_bytes" --header 'Accept: application/octet-stream' \
      --header 'X-GitHub-Api-Version: 2022-11-28' --output "$path.partial" "$url"
    [[ $(stat -c '%s' "$path.partial") == "$expected_bytes" ]]
    printf '%s  %s\n' "$expected" "$path.partial" | sha256sum --check --status
    mv "$path.partial" "$path"
  fi
  [[ -f "$path" && ! -L "$path" ]]
  [[ $(stat -c '%s' "$path") == "$expected_bytes" ]]
  printf '%s  %s\n' "$expected" "$path" | sha256sum --check --status
}
prepare_tla "$tool_dir/tla2tools-v1.8.0.jar"
if [[ ${ASB_FORMAL_ACQUIRE_ONLY:-0} == 1 ]]; then
  echo "TLA artifact acquired"
  exit 0
fi
fetch "$ALLOY_URL" "$tool_dir/alloy-v6.2.0.jar" "$ALLOY_SHA256" "$ALLOY_BYTES"
model_dir=$(cd "$(dirname "$0")/models" && pwd -P)
(
  cd "$model_dir"
  java -XX:+UseParallelGC -Xmx1g -Djava.io.tmpdir="$scratch_dir/tlc" \
    -cp "$tool_dir/tla2tools-v1.8.0.jar" tlc2.TLC -workers 1 -deadlock \
    -metadir "$scratch_dir/tlc/positive" Recovery.tla
)
(
  cd "$scratch_dir/tlc/negative"
  if java -XX:+UseParallelGC -Xmx1g -Djava.io.tmpdir="$scratch_dir/tlc" \
    -cp "$tool_dir/tla2tools-v1.8.0.jar" tlc2.TLC -workers 1 -deadlock \
    -noGenerateSpecTE \
    -metadir "$scratch_dir/tlc/negative" \
    -config "$model_dir/RecoveryStaleMutation.cfg" \
    "$model_dir/RecoveryStaleMutation.tla" \
    >"$scratch_dir/tlc/stale-mutation.log" 2>&1; then
    echo "TLC accepted the deliberate stale-epoch mutation" >&2
    exit 1
  fi
  grep -q "Invariant StaleEventsFenced is violated" "$scratch_dir/tlc/stale-mutation.log"
)
alloy() {
  local model=$1 root=$2
  java -XX:+UseSerialGC -Xmx1g -Djava.io.tmpdir="$root/tmp" \
    -jar "$tool_dir/alloy-v6.2.0.jar" exec --command '*' --type json \
    --output "$root/out" "$model_dir/$model"
}
alloy Recovery.als "$scratch_dir/alloy-positive"
alloy RecoveryMutants.als "$scratch_dir/alloy-negative"
python3 "$(dirname "$0")/verify_alloy_receipt.py" \
  "$scratch_dir/alloy-positive/out/receipt.json" \
  "$scratch_dir/alloy-negative/out/receipt.json"
echo "temporal models passed"
