---
{
  "branch": "",
  "checkpoint_commit": "",
  "claim_expires": "2026-10-11T08:25:27+00:00",
  "depends_on": [
    "AR-1733"
  ],
  "id": "AR-1736",
  "next_action": "Promote after AR-1733; inventory the canonical backend registry and implement catalog, selection, run, and sweep parity with exhaustive fixtures.",
  "owner": "codex-asb-ar1736-catalog-luna56-20261011",
  "plan": "../plans/AR-1736-backend-model-catalog-execution.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_ref": "specs/AR-1736.json",
  "spec_revision": 1,
  "status": "in_progress",
  "summary": "Make every supported model-bearing backend enumerate selectable models and carry the exact selection through complete run and sweep execution.",
  "task_revision": 5,
  "title": "Require model catalogs and execution parity for every backend",
  "updated_at": "2026-10-11T06:26:37+00:00",
  "worktree_key": ""
}
---

Establish one mechanically complete contract for every backend ASB advertises.
Each model-bearing backend must enumerate a bounded, generation/digest-bound
model catalog, allow only catalog-backed model selection, and preserve the exact
backend/model identity through plan, normal run, and bounded sweep results.
Backends that are intentionally model-less must declare that fact explicitly
and remain covered by the same registry completeness gate; they may not be
silently omitted. Development fixtures are authoritative for CI. Optional live
checks may add evidence but missing credentials, authentication, signatures, or
key services must not block this development AR.

- 2026-10-11T06:25:24+00:00: Dependency AR-1733 is durably done; promote for backend model catalog
  and execution parity implementation.

- 2026-10-11T06:25:27+00:00: Claimed by codex-asb-ar1736-catalog-luna56-20261011.

- 2026-10-11T06:26:12+00:00: Recorded command exit 0; command argv SHA-256
  5345cd4b5f66285dc80cddfe59c0a95475263f9ff88d25d2639fce3c300aa4b9.

- 2026-10-11T06:26:37+00:00: Recorded command exit 0; command argv SHA-256
  eb3211a6e4a80eba2fbdf225d485899c73ab40da4c28324f6a333b592dbf6fbe.
