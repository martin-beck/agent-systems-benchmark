---
{
  "branch": "repair/ar-1822-role-removal-expiry",
  "checkpoint_commit": "",
  "claim_expires": "",
  "depends_on": [],
  "id": "AR-1822",
  "next_action": "Merged exact reviewed PR #140 at 4cea485091; stale assignment ar1774-pr612-review-ad7f1166 removed at role revision 22 to 23; verify roles list, doctor, and post-merge checks before acceptance and release.",
  "owner": "",
  "plan": "../plans/AR-1822-role-removal-expiry.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_acceptance": {
    "evidence_class": "contract-test",
    "evidence_digest": "sha256:5876e3111c49c1ed178529b602eaa6ae6d5b4ef5616be4cc6de10ad97758ec08",
    "evidence_ref": "quality/AR-1822-role-removal-expiry-closeout.json",
    "spec_ref": "specs/AR-1822.json",
    "spec_revision": 1,
    "status": "pass"
  },
  "spec_ref": "specs/AR-1822.json",
  "spec_revision": 1,
  "status": "done",
  "summary": "Allow exact-revision roles.remove cleanup of explicitly named expired assignments while preserving fail-closed reads and admission.",
  "task_revision": 9,
  "title": "Expired role assignment cleanup semantics",
  "updated_at": "2026-10-12T15:25:32+00:00",
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

- 2026-10-12T15:21:24+00:00: Claimed by ar1822_role_cleanup.

- 2026-10-12T15:21:37+00:00: Post-merge cleanup executed through repaired roles.remove; no runtime
  role file was edited directly.

- 2026-10-12T15:25:11+00:00: Exact reviewed PR #140 merged, all required checks green, repaired
  roles.remove removed the actual stored stale assignment at revision 22 to 23, roles list and
  doctor passed.

- 2026-10-12T15:25:32+00:00: AR-1822 complete: PR #140 merged as 4cea485091; post-merge checks and
  doctor passed; stale assignment cleanup was exact-revision 22 to 23 with actual stored ID
  ar1774-pr612-review-ad7f1166.
