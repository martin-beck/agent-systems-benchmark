---
{
  "branch": "",
  "checkpoint_commit": "",
  "claim_expires": "",
  "depends_on": ["AR-1776", "AR-1778"],
  "id": "AR-1798",
  "next_action": "Implement the bounded immutable source-checkout provider required by AR-1778 and consume it from AR-1779.",
  "observed_branch": "",
  "observed_dirty": 0,
  "observed_head": "",
  "owner": "",
  "plan": "../plans/AR-1798-pinned-source-checkout-acquisition.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_acceptance": {"evidence_class": "contract-test", "evidence_digest": "", "evidence_ref": "", "spec_ref": "specs/AR-1798.json", "spec_revision": 1, "status": "pending"},
  "spec_ref": "specs/AR-1798.json",
  "spec_revision": 1,
  "status": "planned",
  "summary": "Provide a bounded immutable source checkout so supported tool installation can invoke the pinned source builder without fabricating provenance.",
  "task_revision": 1,
  "title": "Pinned source-checkout acquisition",
  "updated_at": "2026-10-11T14:12:00Z",
  "worktree_key": "agent-systems-benchmark-ar-1798-pinned-source-checkout-acquisition"
}
---

AR-1798 closes the integration gap discovered while reviewing AR-1779. The
AR-1778 source builder intentionally accepts only a caller-owned checkout whose
HEAD exactly matches the catalog revision, but AR-1779 currently has no safe
provider for that checkout.

Implement a project-local, bounded provider that resolves only the AR-1776
catalog repository and immutable revision, fetches a checkout without shell
hooks or inherited credentials, verifies repository identity and exact HEAD,
enforces resource and path limits, and cleans transaction-owned staging data.
Expose a typed result that AR-1779 can pass to the existing AR-1778 builder.
Offline, unreachable, wrong-revision, repository-identity, resource-limit,
cancellation, and cleanup failures must remain distinct and actionable. Never
silently substitute an archive, moving branch, unrelated local checkout, or
prebuilt artifact for a requested source-only recipe.

The implementation must add deterministic local fixtures and positive/negative
tests, preserve zero runtime dependencies, and leave no credentials or
unbounded command output in receipts. It must be integrated into AR-1779 and
qualified through the normal independent review, signed-DCO PR, exact-head CI,
protected merge, and post-merge evidence path.
