---
{
  "branch": "policy/concurrent-integration-liveness",
  "checkpoint_commit": "",
  "claim_expires": "",
  "depends_on": [],
  "id": "AR-1796",
  "next_action": "Publish the bounded policy repair, then add enforceable integration-queue and evidence-reuse controls without weakening merge-result assurance.",
  "observed_branch": "policy/concurrent-integration-liveness",
  "observed_dirty": 0,
  "observed_head": "",
  "owner": "",
  "plan": "../plans/AR-1796-concurrent-integration-liveness.md",
  "priority": "P0",
  "schema_version": 1,
  "status": "open",
  "summary": "Keep autonomous development live during concurrent protected-main integration.",
  "task_revision": 1,
  "title": "Concurrent integration liveness",
  "updated_at": "2026-10-11T12:00:00+00:00",
  "worktree_key": ""
}
---

Repair the policy that turns ordinary concurrent protected-main progress into
terminal worker blocking. Preserve exact merge-result assurance while ensuring
workers checkpoint, retry, and continue useful work during integration churn.
