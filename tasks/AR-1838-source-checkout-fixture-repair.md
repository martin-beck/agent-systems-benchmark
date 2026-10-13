---
{
  "branch": "",
  "checkpoint_commit": "",
  "claim_expires": "",
  "depends_on": [],
  "id": "AR-1838",
  "next_action": "Reproduce the repeated source_checkout fixture Unavailable failure, repair the owning test/fixture contract without weakening it, then run full gates and publish a reviewed repair.",
  "observed_branch": "",
  "observed_dirty": 0,
  "observed_head": "",
  "owner": "",
  "plan": "../plans/AR-1838-source-checkout-fixture-repair.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_acceptance": {"evidence_class": "contract-test", "evidence_digest": "", "evidence_ref": "", "spec_ref": "specs/AR-1838.json", "spec_revision": 1, "status": "pending"},
  "spec_ref": "specs/AR-1838.json",
  "spec_revision": 1,
  "status": "planned",
  "summary": "Repair the reproducible source_checkout exact-fixture test failure blocking protected-main post-merge quality.",
  "task_revision": 1,
  "title": "Repair reproducible source checkout fixture failure",
  "updated_at": "2026-10-13T06:00:00+00:00",
  "worktree_key": "agent-systems-benchmark-ar-1838-source-checkout-fixture-repair"
}
---

The exact post-merge `Credential-free benchmark path` and `Repository quality`
jobs both fail at
`source_checkout::tests::exact_fixture_checkout_is_returned_and_cleanup_is_explicit`
(`crates/asb-cli/src/source_checkout.rs:913`) with `Err(Unavailable)` while
456 other unit tests pass. This is a reproducible contract/fixture failure,
not permission to skip coverage or weaken source acquisition validation.

Reproduce it on clean exact main, identify the owning race/fixture assumption,
add positive and negative coverage, preserve bounded cleanup and fail-closed
behavior, then complete independent review, exact-head CI, signed-DCO merge,
and exact-main verification. Do not alter unrelated ARs or provider/network
boundaries.
