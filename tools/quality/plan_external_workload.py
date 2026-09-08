#!/usr/bin/env python3
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Create an external-workload execution plan without performing external effects."""

import argparse
import json
from pathlib import Path


REGISTRY = Path(__file__).parents[2] / "crates/asb-workloads/registry/v1/external-workloads.json"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("workload_id")
    args = parser.parse_args()
    document = json.loads(REGISTRY.read_text(encoding="utf-8"))
    records = [item for item in document["workloads"] if item.get("id") == args.workload_id]
    if len(records) != 1:
        raise SystemExit("unknown or duplicate external workload")
    record = records[0]
    evaluator = record["evaluator"]
    provenance = evaluator["provenance"]
    if provenance.get("status") != "qualified":
        print(json.dumps({"workload": args.workload_id, "status": "unqualified", "reason": "evaluator provenance is not qualified"}, sort_keys=True))
        return 2
    required = (evaluator.get("image_digest"), provenance.get("sbom_sha256"), provenance.get("evidence"))
    if not all(required):
        raise SystemExit("qualified evaluator is missing immutable provenance")
    print(json.dumps({"workload": args.workload_id, "status": "planned", "source": record["source"], "evaluator": evaluator}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
