---
{
  "branch": "",
  "checkpoint_commit": "",
  "claim_expires": "",
  "depends_on": ["AR-1711", "AR-1733"],
  "id": "AR-1790",
  "next_action": "Promote and claim; repair the effective metrics WouldBlock retry guard and add retry-budget regression coverage.",
  "observed_branch": "",
  "observed_dirty": 0,
  "observed_head": "",
  "owner": "",
  "plan": "../plans/AR-1790-metrics-retry-budget.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_ref": "specs/AR-1790.json",
  "spec_revision": 1,
  "status": "planned",
  "summary": "Repair the ineffective bounded metrics spawn retry guard found in AR-1789 review.",
  "task_revision": 1,
  "title": "Make metrics spawn retry budget effective",
  "worktree_key": "agent-systems-benchmark-ar1790-metrics-retry-budget",
  "updated_at": "2026-10-10T21:43:00+00:00"
}
---

This development repair does not require a GitHub Reviews API approval object.
Independent technical review, exact-head CI, signed DCO integration, and
post-merge verification remain mandatory.

- 2026-10-10T21:43:00Z: Created from the AR-1789 independent review. The
  candidate declares eight spawn attempts but still retries only while
  `attempt < 2`; no publication or merge is authorized until the guard and
  regression evidence are repaired.
