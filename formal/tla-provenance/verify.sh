#!/usr/bin/env bash
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
set -euo pipefail

case ${1:-} in
    --manifest-only)
        [[ $# == 2 ]] || exit 64
        mode=manifest; manifest=$2; source_tree=; output= ;;
    --inventory|--pruned)
        [[ $# == 3 ]] || exit 64
        mode=${1#--}; manifest=$2; source_tree=$3; output= ;;
    *)
        [[ $# == 3 ]] || { echo "usage: verify.sh [--manifest-only MANIFEST | --inventory MANIFEST SOURCE_TREE | --pruned MANIFEST SOURCE_TREE | MANIFEST PRUNED_SOURCE_TREE OUTPUT]" >&2; exit 64; }
        mode=full; manifest=$1; source_tree=$2; output=$3 ;;
esac
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
exec python3 "$script_dir/verify.py" "$mode" "$manifest" "$source_tree" "$output" "$script_dir"
