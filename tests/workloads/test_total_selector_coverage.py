# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Machine-check the documented workload inventory against the catalog."""

import json
from pathlib import Path

ROOT = Path(__file__).parents[2]
REGISTRY = ROOT / "crates/asb-workloads/registry/v1/external-workloads.json"
CATALOG = ROOT / "docs/generated/workload-catalog-v1.json"
DOCS = "\n".join(
    (ROOT / name).read_text(encoding="utf-8")
    for name in ("docs/WORKLOADS.md", "docs/RELATED_WORK.md")
)

# These labels are the public names used by the two literature inventories.
# Their stable IDs are the only values allowed into selection and reports.
DOCUMENTED_IDS = {
    "AgentBench": "agentbench",
    "AgentDojo": "agentdojo",
    "AgentOps": "agentops",
    "AI Agents That Matter": "ai-agents-that-matter",
    "Aider Polyglot": "aider-polyglot",
    "BigCodeBench": "bigcodebench",
    "CORE-Bench": "core-bench",
    "EvalPlus": "evalplus",
    "Exercism Tracks": "exercism-tracks",
    "HAL": "hal",
    "Harbor": "harbor",
    "HELM": "helm",
    "HumanEval+": "humaneval-plus",
    "Inspect AI": "inspect-ai",
    "LiveCodeBench": "livecodebench",
    "MBPP+": "mbpp-plus",
    "SWE-bench": "swe-bench",
    "SWE-bench Lite / Verified": "swe-bench-lite",
    "SWE-bench Pro": "swe-bench-pro",
    "SWE-fficiency": "swe-fficiency",
    "SWE-Lancer": "swe-lancer",
    "SWE-Perf": "swe-perf",
    "SWE-rebench": "swe-rebench",
    "Terminal-Bench": "terminal-bench",
    "tau-bench": "tau-bench",
}


def test_every_documented_and_registered_id_has_one_catalog_entry():
    registry = json.loads(REGISTRY.read_text(encoding="utf-8"))
    catalog = json.loads(CATALOG.read_text(encoding="utf-8"))
    registry_ids = {item["id"] for item in registry["workloads"]}
    entries = catalog["entries"]
    by_id = {item["id"]: item for item in entries}

    assert len(entries) == len(by_id)
    assert registry_ids <= set(by_id)
    for label, workload_id in DOCUMENTED_IDS.items():
        assert label in DOCS, f"documented workload label missing: {label}"
        assert workload_id in by_id, f"documented workload absent from catalog: {workload_id}"


def test_catalog_evidence_keeps_unqualified_records_unavailable():
    registry = json.loads(REGISTRY.read_text(encoding="utf-8"))
    catalog = json.loads(CATALOG.read_text(encoding="utf-8"))
    by_id = {item["id"]: item for item in catalog["entries"]}
    for record in registry["workloads"]:
        entry = by_id[record["id"]]
        if record.get("selection") == "methodology-only":
            assert entry["kind"] == "methodology"
            assert entry["availability"] == "unavailable"
        else:
            assert entry["kind"] == "literature"
            assert entry["availability"] == "fixture_only"
            assert entry["evidence"] in {"planned", "unqualified"}
