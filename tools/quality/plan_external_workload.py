#!/usr/bin/env python3
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Create an external-workload execution plan without performing external effects."""

import argparse
import json
from pathlib import Path

from validate_external_qualification import validate_document

REGISTRY = Path(__file__).parents[2] / "crates/asb-workloads/registry/v1/external-workloads.json"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("workload_id")
    parser.add_argument("--registry", type=Path, default=REGISTRY)
    parser.add_argument(
        "--qualification",
        type=Path,
        help="optional reviewed native-qualification evidence; never acquired automatically",
    )
    args = parser.parse_args()
    document = json.loads(args.registry.read_text(encoding="utf-8"))
    records = [item for item in document["workloads"] if item.get("id") == args.workload_id]
    if len(records) != 1:
        raise SystemExit("unknown or duplicate external workload")
    record = records[0]
    evaluator = record["evaluator"]
    provenance = evaluator["provenance"]
    if provenance.get("status") != "qualified":
        print(json.dumps({"workload": args.workload_id, "status": "unqualified", "reason": "evaluator provenance is not qualified"}, sort_keys=True))
        return 2
    if args.qualification is None:
        print(
            json.dumps(
                {
                    "workload": args.workload_id,
                    "status": "unqualified",
                    "reason": "native qualification evidence was not supplied",
                },
                sort_keys=True,
            )
        )
        return 2
    try:
        qualification = validate_document(
            json.loads(args.qualification.read_text(encoding="utf-8"))
        )
    except (OSError, json.JSONDecodeError, ValueError) as exc:
        raise SystemExit(str(exc)) from exc
    if qualification["workload"] != args.workload_id:
        raise SystemExit("qualification workload does not match requested workload")
    required = (evaluator.get("image_digest"), provenance.get("sbom_sha256"), provenance.get("evidence"))
    if not all(required):
        raise SystemExit("qualified evaluator is missing immutable provenance")
    print(json.dumps({"workload": args.workload_id, "status": "planned", "source": record["source"], "evaluator": evaluator}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
