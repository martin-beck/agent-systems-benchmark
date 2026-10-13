---
{
  "branch": "",
  "checkpoint_commit": "",
  "claim_expires": "2026-10-13T12:33:59+00:00",
  "depends_on": [
    "AR-1795"
  ],
  "id": "AR-1841",
  "next_action": "Promote and claim; synchronize the immutable Coordinator v0.5.3 snapshot containing upstream AR-0088, then run complete state gates and publish the reviewed adoption.",
  "owner": "ar1841-v053-adoption-luna",
  "plan": "../plans/AR-1841-coordinator-v053-adoption.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_acceptance": {
    "evidence_class": "mechanical",
    "evidence_digest": "",
    "evidence_ref": "",
    "spec_ref": "specs/AR-1841.json",
    "spec_revision": 1,
    "status": "pending"
  },
  "spec_ref": "specs/AR-1841.json",
  "spec_revision": 1,
  "status": "in_progress",
  "summary": "Adopt Coordinator v0.5.3 with partial legacy worktree binding.",
  "task_revision": 3,
  "title": "Coordinator v0.5.3 adoption",
  "updated_at": "2026-10-13T09:33:59+00:00",
  "worktree_key": ""
}
---

This downstream adoption consumes upstream AR-0088; it does not modify the
Coordinator implementation. It exists to replace the stale v0.5.1 state vendor
with the verified v0.5.3 snapshot needed to unblock AR-1826.

- 2026-10-13T09:31:12+00:00: v0.5.3 adoption is dependency-ready; upstream AR-0088 is complete and
  exact release tag verified locally

- 2026-10-13T09:33:59+00:00: Claimed by ar1841-v053-adoption-luna.
