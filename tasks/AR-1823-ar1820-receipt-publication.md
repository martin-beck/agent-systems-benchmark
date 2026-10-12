---
{
  "branch": "repair/ar-1823-ar1820-receipt",
  "checkpoint_commit": "",
  "claim_expires": "2026-10-12T17:01:26+00:00",
  "depends_on": [],
  "id": "AR-1823",
  "next_action": "Publish the already-verified AR-1820 receipt and done transition from the durable current-main state; do not change product or historical PR evidence.",
  "owner": "ar1823_receipt_cleanup",
  "plan": "../plans/AR-1823-ar1820-receipt-publication.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_acceptance": {
    "evidence_class": "hosted",
    "evidence_digest": "sha256:fb4285c78143d1ab188cf6df66de051f2b72e15ba6a29364d48201daaad0c6ca",
    "evidence_ref": "quality/AR-1820-postmerge-receipt.json",
    "spec_ref": "specs/AR-1823.json",
    "spec_revision": 1,
    "status": "pass"
  },
  "spec_ref": "specs/AR-1823.json",
  "spec_revision": 1,
  "status": "in_progress",
  "summary": "Publish the already-verified AR-1820 provenance receipt and durable done state.",
  "task_revision": 5,
  "title": "AR-1820 receipt publication cleanup",
  "updated_at": "2026-10-12T15:09:01+00:00",
  "worktree_key": "agent-systems-benchmark-state-ar-1823-receipt"
}
---

# AR-1823 — AR-1820 receipt publication cleanup

The verified AR-1820 state exists in a detached local coordinator checkout but
is absent from durable `origin/main`. Publish only that already-recorded task
closure and receipt. Preserve PR #614, its exact reviewed merge, all hosted
evidence, and historical failures; do not change product/runtime behavior or
reclassify any other AR.

- 2026-10-12T15:00:00+00:00: Created after reconciliation found the verified
  AR-1820 closure only in detached local state, not current durable main.

- 2026-10-12T15:01:20+00:00: Dependencies verified: publish only the existing verified AR-1820
  receipt and done state.

- 2026-10-12T15:01:26+00:00: Claimed by ar1823_receipt_cleanup.

- 2026-10-12T15:01:53+00:00: Bound worktree agent-systems-benchmark-state-ar-1823-receipt on branch
  repair/ar-1823-ar1820-receipt.

- 2026-10-12T15:09:01+00:00: AR-1820 task, plan, spec, session history, and exact receipt are now
  durable on current main; Coordination verification 38267827300 and live doctor pass.
