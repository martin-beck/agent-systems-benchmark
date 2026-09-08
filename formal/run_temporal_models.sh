#!/usr/bin/env bash
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
set -euo pipefail
TLA_SHA256=4c7bb1f6b050d56c197ee9ddd6e57fe521eae175f5043c9fb98b169f7b2d5407
ALLOY_SHA256=6b8c1cb5bc93bedfc7c61435c4e1ab6e688a242dc702a394628d9a9801edb78d
TLA_BYTES=4487756
ALLOY_BYTES=21062377
TLA_URL=https://api.github.com/repos/tlaplus/tlaplus/releases/assets/551007111
ALLOY_URL=https://github.com/AlloyTools/org.alloytools.alloy/releases/download/v6.2.0/org.alloytools.alloy.dist.jar
if [[ $# -ne 2 || $1 != /* || $2 != /* ]]; then
  echo "usage: $0 ABSOLUTE_TOOL_DIR ABSOLUTE_NEW_SCRATCH_DIR" >&2
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
fetch "$TLA_URL" "$tool_dir/tla2tools-v1.8.0.jar" "$TLA_SHA256" "$TLA_BYTES"
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
