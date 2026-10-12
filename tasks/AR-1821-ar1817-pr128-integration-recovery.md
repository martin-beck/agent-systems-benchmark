---
{
  "branch": "repair/ar-1821-ar1817-integration",
  "checkpoint_commit": "",
  "claim_expires": "",
  "depends_on": [],
  "id": "AR-1821",
  "next_action": "Claim this recovery task, verify accepted PR #128 exact head against current main, integrate through the signed local merge path, run exact-main post-merge checks, and close with durable evidence.",
  "owner": "",
  "plan": "../plans/AR-1821-ar1817-pr128-integration-recovery.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_ref": "specs/AR-1821.json",
  "spec_revision": 1,
  "status": "open",
  "summary": "Recover the durable task record needed to integrate accepted AR-1817 PR #128 after its original task was removed from current generated state.",
  "task_revision": 3,
  "title": "AR-1817 PR #128 integration recovery",
  "updated_at": "2026-10-12T14:15:17+00:00",
  "worktree_key": ""
}
---

# AR-1821 — AR-1817 PR #128 integration recovery

This narrow recovery task exists only because the original AR-1817 task record
was removed from the current generated state after PR #128 was accepted. It
preserves the accepted schema change evidence without changing PR #128 or
reclassifying the work under AR-1820.

- 2026-10-12T14:13:56+00:00: Claimed by ar1821_ar1817_integration.

- 2026-10-12T14:15:17+00:00: Transfer recovery ownership to the existing coordinator role for signed
  integration.
