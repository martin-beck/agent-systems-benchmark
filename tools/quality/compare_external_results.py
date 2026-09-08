#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Compare validated external results only within a shared evaluator contract."""

import argparse
import json
from pathlib import Path


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("results", type=Path, nargs="+")
    args = parser.parse_args()
    rows = [json.loads(path.read_text(encoding="utf-8")) for path in args.results]
    if not rows or any(set(row) != {"workload", "oracle", "evaluator", "score"} for row in rows):
        raise SystemExit("comparison input schema is not exact")
    identities = {(row["oracle"], row["evaluator"]) for row in rows}
    if len(identities) != 1:
        print(json.dumps({"status": "non-comparable", "reason": "evaluator identity differs"}, sort_keys=True))
        return 2
    ranking = sorted(({"workload": row["workload"], "score": row["score"]} for row in rows), key=lambda row: (-row["score"], row["workload"]))
    print(json.dumps({"status": "comparable", "oracle": rows[0]["oracle"], "evaluator": rows[0]["evaluator"], "ranking": ranking}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
