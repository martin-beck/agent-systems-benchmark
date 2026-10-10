---
{
  "branch": "",
  "checkpoint_commit": "",
  "claim_expires": "",
  "depends_on": ["AR-1711", "AR-1733"],
  "id": "AR-1787",
  "next_action": "Promote and claim after dependency verification; reproduce the AR-1786 same-process lock-release flaw, repair descriptor lifetime and catalog identities, then run repeated full gates.",
  "observed_branch": "",
  "observed_dirty": 0,
  "observed_head": "",
  "owner": "",
  "plan": "../plans/AR-1787-control-lock-lifetime-repair.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_ref": "specs/AR-1787.json",
  "spec_revision": 1,
  "status": "planned",
  "summary": "Correct control lock descriptor lifetime and diagnostic identities after the AR-1786 review blocker.",
  "task_revision": 1,
  "title": "Repair control lock lifetime semantics",
  "updated_at": "2026-10-10T21:02:00+00:00",
  "worktree_key": ""
}
---

This development repair does not require a GitHub Reviews API approval object.
Independent technical review, exact-head CI, signed DCO integration, and
post-merge verification remain mandatory. It supersedes no existing AR and must
not weaken control ownership or the ordinary workspace gate.

- 2026-10-10T21:02:00+00:00: Created from the AR-1786 independent review. Candidate
  7be2fcb0 had three ordinary workspace failures from stale diagnostic identities,
  and a P0 same-process POSIX record-lock lifetime flaw where a rejected contender's
  descriptor close could release the active owner lock. No publication or merge is authorized.
