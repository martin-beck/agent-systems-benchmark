#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Validate a bounded external-oracle result without trusting raw diagnostics."""

import argparse
import json
from pathlib import Path


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("result", type=Path)
    parser.add_argument("--max-score", type=float, default=1.0)
    args = parser.parse_args()
    try:
        result = json.loads(args.result.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise SystemExit(f"oracle result is not valid JSON: {exc}") from exc
    if not isinstance(result, dict) or set(result) != {"oracle", "score"}:
        raise SystemExit("oracle result schema is not exact")
    if not isinstance(result["oracle"], str) or not result["oracle"]:
        raise SystemExit("oracle identity is missing")
    if not isinstance(result["score"], (int, float)) or isinstance(result["score"], bool) or not 0 <= result["score"] <= args.max_score:
        raise SystemExit("oracle score is outside its declared bounds")
    print(json.dumps({"oracle": result["oracle"], "score": result["score"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
