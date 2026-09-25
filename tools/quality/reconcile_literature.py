#!/usr/bin/env python3
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Build the closed, documentation-to-registry literature parity view.

This check is deliberately local.  It reads only the checked-in registry and
the four public documents named by AR-1423; it never acquires a dataset or
contacts a provider.  A display name is allowed to be an alias (for example,
the Lite and Verified splits) but every alias has exactly one canonical
registry identity and an explicit boundary.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

ROOT = Path(__file__).parents[2]
REGISTRY = ROOT / "crates/asb-workloads/registry/v1/external-workloads.json"
OUTPUT = ROOT / "docs/generated/literature-parity-v1.json"
DOCS = ("WORKLOADS.md", "RELATED_WORK.md", "PLAN.md")

# (documentation spelling, canonical registry id, family, classification).
# Keep this table stable: changing an identity is a reviewed contract change.
EXPECTATIONS = (
    ("SWE-bench", "swe-bench", "repository-repair", "workload"),
    ("SWE-bench Lite", "swe-bench", "repository-repair", "workload"),
    ("SWE-bench Verified", "swe-bench", "repository-repair", "workload"),
    ("Terminal-Bench", "terminal-bench", "terminal", "workload"),
    ("Aider Polyglot", "aider-polyglot", "code-generation", "workload"),
    ("Exercism Tracks", "exercism-tracks", "code-generation", "workload"),
    ("SWE-bench Pro", "swe-bench-pro", "repository-repair", "workload"),
    ("BigCodeBench", "bigcodebench", "code-generation", "workload"),
    ("HumanEval+", "evalplus", "code-generation", "workload"),
    ("MBPP+", "evalplus", "code-generation", "workload"),
    ("EvalPlus", "evalplus", "code-generation", "workload"),
    ("LiveCodeBench", "livecodebench", "code-generation", "workload"),
    ("SWE-Lancer", "swe-lancer", "repository-repair", "workload"),
    ("SWE-rebench", "swe-rebench", "repository-repair", "workload"),
    ("SWE-Perf", "swe-perf", "performance", "workload"),
    ("SWE-fficiency", "swe-fficiency", "performance", "workload"),
    ("CORE-Bench", "core-bench", "computational-reproducibility", "workload"),
    ("AgentBench", "agentbench", "interactive-tool-use", "workload"),
    ("tau-bench", "tau-bench", "interactive-tool-use", "workload"),
    ("AgentDojo", "agentdojo", "interactive-tool-use", "workload"),
    ("Harbor", "harbor", "framework", "framework"),
    ("Inspect AI", "inspect-ai", "framework", "framework"),
    ("HAL", "hal", "harness", "framework"),
    ("AgentOps", "agentops", "observability", "framework"),
    ("HELM", "helm", "evaluation-harness", "framework"),
)


def fail(message: str) -> None:
    raise ValueError(f"literature parity: {message}")


def _revision(record: dict[str, Any]) -> str:
    source = record.get("source", {})
    return str(source.get("commit") or source.get("revision") or record.get("version") or "")


def _dataset_revision(record: dict[str, Any]) -> str:
    dataset = record.get("dataset", {})
    return str(dataset.get("revision") or dataset.get("split") or "not-applicable")


def build_parity(registry: dict[str, Any], documents: dict[str, str]) -> dict[str, Any]:
    records = registry.get("workloads")
    if registry.get("schema_version") != 1 or not isinstance(records, list):
        fail("registry must contain schema_version 1 and a workloads list")
    by_id: dict[str, dict[str, Any]] = {}
    for record in records:
        ident = record.get("id")
        if not isinstance(ident, str) or not ident or ident in by_id:
            fail(f"duplicate or invalid registry identity: {ident!r}")
        by_id[ident] = record
    entries: list[dict[str, Any]] = []
    seen_names: set[str] = set()
    for name, ident, family, classification in EXPECTATIONS:
        if name in seen_names:
            fail(f"duplicate documentation name: {name}")
        seen_names.add(name)
        mention = {
            "SWE-bench Verified": "SWE-bench Lite / Verified",
        }.get(name, name)
        if not any(mention in text for text in documents.values()):
            fail(f"documentation mention is missing: {name}")
        record = by_id.get(ident)
        if record is None:
            fail(f"{name}: no registry identity {ident!r}")
        selection = record.get("selection", "executable-candidate")
        kind = record.get("kind", "")
        framework = classification == "framework"
        if framework != (selection == "methodology-only"):
            fail(f"{name}: framework classification disagrees with registry selection")
        evaluator = record.get("evaluator", {})
        provenance = evaluator.get("provenance", {})
        license_value = record.get("source", {}).get("license", "NOASSERTION")
        license_state = record.get("source", {}).get("license_status", "declared")
        entries.append(
            {
                "name": name,
                "registry_id": ident,
                "family": family,
                "kind": classification,
                "source_revision": _revision(record),
                "dataset_revision": _dataset_revision(record),
                "license": license_value,
                "license_state": license_state,
                "evaluator": evaluator.get("entrypoint", "not-applicable"),
                "evaluator_state": provenance.get("status", "not-applicable"),
                "evidence_status": provenance.get("status", "not-applicable"),
                "selectable": not framework,
                "framework_boundary": (
                    "framework-only; requires an independent task protocol and grader"
                    if framework
                    else "independent task protocol and grader required"
                ),
            }
        )
    return {
        "schema_version": 1,
        "source_documents": list(DOCS),
        "entries": entries,
    }


def validate_artifact(document: dict[str, Any]) -> None:
    """Validate the generated instance against the checked-in closed schema."""
    if set(document) != {"schema_version", "source_documents", "entries"}:
        fail("generated artifact has unknown or missing top-level fields")
    if document["schema_version"] != 1 or document["source_documents"] != list(DOCS):
        fail("generated artifact schema version or source documents drifted")
    required = {
        "name", "registry_id", "family", "kind", "source_revision", "dataset_revision",
        "license", "license_state", "evaluator", "evaluator_state", "evidence_status",
        "selectable", "framework_boundary",
    }
    names: set[str] = set()
    for entry in document["entries"]:
        if set(entry) != required:
            fail("generated artifact entry is not closed")
        if not isinstance(entry["name"], str) or entry["name"] in names:
            fail("generated artifact names must be unique")
        names.add(entry["name"])
        if entry["kind"] not in {"workload", "framework"}:
            fail("generated artifact kind is invalid")
        if entry["kind"] == "framework" and entry["selectable"]:
            fail("framework-only entries cannot be selectable")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--registry", type=Path, default=REGISTRY)
    parser.add_argument("--docs-root", type=Path, default=ROOT / "docs")
    parser.add_argument("--output", type=Path, default=OUTPUT)
    parser.add_argument("--write", action="store_true")
    args = parser.parse_args()
    registry = json.loads(args.registry.read_text(encoding="utf-8"))
    documents = {
        name: (args.docs_root / name).read_text(encoding="utf-8") for name in DOCS
    }
    parity = build_parity(registry, documents)
    validate_artifact(parity)
    rendered = json.dumps(parity, indent=2) + "\n"
    if args.write:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(rendered, encoding="utf-8")
    elif args.output.read_text(encoding="utf-8") != rendered:
        fail("generated parity artifact is stale; run with --write")
    print(json.dumps({"entries": len(parity["entries"]), "schema_version": 1}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
