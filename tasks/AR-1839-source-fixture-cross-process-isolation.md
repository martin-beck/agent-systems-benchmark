---
{
  "branch": "",
  "checkpoint_commit": "",
  "claim_expires": "",
  "depends_on": [],
  "id": "AR-1839",
  "next_action": "Reproduce source_checkout fixture collisions across concurrent cargo processes, add process-safe isolation, then run focused/full gates and publish a reviewed repair.",
  "observed_branch": "",
  "observed_dirty": 0,
  "observed_head": "",
  "owner": "",
  "plan": "../plans/AR-1839-source-fixture-cross-process-isolation.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_acceptance": {
    "evidence_class": "contract-test",
    "evidence_digest": "",
    "evidence_ref": "",
    "spec_ref": "specs/AR-1839.json",
    "spec_revision": 1,
    "status": "pending"
  },
  "spec_ref": "specs/AR-1839.json",
  "spec_revision": 1,
  "status": "open",
  "summary": "Make source checkout test fixtures safe when independent cargo processes run concurrently.",
  "task_revision": 2,
  "title": "Repair cross-process source fixture isolation",
  "updated_at": "2026-10-13T08:31:04+00:00",
  "worktree_key": "agent-systems-benchmark-ar-1839-source-fixture-cross-process-isolation"
}
---

Independent review of PR #630 found the in-process semaphore/Mutex repair
passes uncontended runs but still permits `Err(Unavailable)` when separate
cargo/test processes execute the fixture concurrently. Reproduce that case,
identify the shared fixture/staging collision, and add process-safe isolation
or unique bounded fixture roots without weakening fail-closed cleanup or
source identity checks. Add positive and negative regression coverage, then
complete focused/full gates, independent review, signed-DCO merge, and exact
post-merge verification.

- 2026-10-13T08:31:04+00:00: Independent review reproduced cross-process source fixture collisions;
  dedicated isolation repair required.
