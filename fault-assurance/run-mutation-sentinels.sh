#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
set -euo pipefail

output_root=${1:?usage: run-mutation-sentinels.sh OUTPUT_ROOT}
cargo mutants --no-config --workspace \
  --file crates/asb-replay/src/service.rs \
  --file crates/asb-analysis/src/lib.rs \
  --re 'replace != with == in request_matches|replace >= with < in minimum_decision' \
  --jobs 1 --timeout 180 --no-times --output "$output_root"

python3 - "$output_root/mutants.out/outcomes.json" <<'PY'
import json
import sys

outcomes = json.load(open(sys.argv[1], encoding="utf-8"))
expected = {"total_mutants": 6, "caught": 6, "missed": 0, "timeout": 0, "unviable": 0}
actual = {name: outcomes[name] for name in expected}
if actual != expected:
    raise SystemExit(f"mutation sentinel mismatch: expected {expected}, observed {actual}")
PY
