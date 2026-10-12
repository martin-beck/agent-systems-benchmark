---
{
  "branch": "repair/ar-1822-role-removal-expiry",
  "checkpoint_commit": "",
  "claim_expires": "",
  "depends_on": [],
  "id": "AR-1822",
  "next_action": "Publish v7 from exact base 06d3a008b624; wait for fresh exact-head review and CI before merge.",
  "owner": "",
  "plan": "../plans/AR-1822-role-removal-expiry.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_acceptance": {
    "evidence_class": "contract-test",
    "evidence_digest": "",
    "evidence_ref": "",
    "spec_ref": "specs/AR-1822.json",
    "spec_revision": 1,
    "status": "pending"
  },
  "spec_ref": "specs/AR-1822.json",
  "spec_revision": 1,
  "status": "open",
  "summary": "Allow exact-revision roles.remove cleanup of explicitly named expired assignments while preserving fail-closed reads and admission.",
  "task_revision": 5,
  "title": "Expired role assignment cleanup semantics",
  "updated_at": "2026-10-12T15:14:45+00:00",
  "worktree_key": "agent-systems-benchmark-state-ar-1822-role-removal-expiry"
}
---

AR-1820 cannot safely clear stale role assignments because the current removal
path validates every assignment with admission-time expiry semantics. This
repair separates explicit, exact-revision administrative cleanup from reads and
admission: removal may delete only the named structurally valid assignment,
including an expired one, while list/check/doctor/admission remain fail-closed.
No runtime authority is broadened and no role state is edited outside the
transactional command.

- 2026-10-12T15:14:27+00:00: Dependencies verified; v7 successor from exact stable main
  06d3a008b624.

- 2026-10-12T15:14:30+00:00: Claimed by ar1817_schema_contract.

- 2026-10-12T15:14:39+00:00: Focused role, schema, vendor, Ruff and mypy gates pass; full bounded
  suite produced no terminal result in the worker session, so CI remains required.

- 2026-10-12T15:14:45+00:00: Publish ownerless v7 current-main successor; preserve fresh review/CI
  gates.
