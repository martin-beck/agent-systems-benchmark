---
{
  "branch": "repair/ar-1823-ar1820-receipt",
  "checkpoint_commit": "",
  "claim_expires": "",
  "depends_on": [],
  "id": "AR-1823",
  "next_action": "Publish the already-verified AR-1820 receipt and done transition from the durable current-main state; do not change product or historical PR evidence.",
  "owner": "",
  "plan": "../plans/AR-1823-ar1820-receipt-publication.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_ref": "specs/AR-1823.json",
  "spec_revision": 1,
  "status": "planned",
  "summary": "Publish the already-verified AR-1820 provenance receipt and durable done state.",
  "task_revision": 1,
  "title": "AR-1820 receipt publication cleanup",
  "updated_at": "2026-10-12T15:00:00+00:00",
  "worktree_key": ""
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
