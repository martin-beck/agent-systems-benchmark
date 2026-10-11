---
{
  "branch": "feature/ar-1798-pinned-source-checkout-acquisition",
  "checkpoint_commit": "",
  "claim_expires": "2026-10-11T16:14:35+00:00",
  "depends_on": [
    "AR-1776",
    "AR-1778"
  ],
  "id": "AR-1798",
  "next_action": "Implement the bounded immutable source-checkout provider required by AR-1778 and consume it from AR-1779.",
  "observed_branch": "feature/ar-1798-pinned-source-checkout-acquisition",
  "observed_dirty": 0,
  "observed_head": "b305ff826c1f06da84205e2f32899291b976ebc7",
  "owner": "ar1798-source-checkout-luna56",
  "plan": "../plans/AR-1798-pinned-source-checkout-acquisition.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_acceptance": {
    "evidence_class": "contract-test",
    "evidence_digest": "",
    "evidence_ref": "",
    "spec_ref": "specs/AR-1798.json",
    "spec_revision": 1,
    "status": "pending"
  },
  "spec_ref": "specs/AR-1798.json",
  "spec_revision": 1,
  "status": "in_progress",
  "summary": "Provide a bounded immutable source checkout so supported tool installation can invoke the pinned source builder without fabricating provenance.",
  "task_revision": 28,
  "title": "Pinned source-checkout acquisition",
  "updated_at": "2026-10-11T14:21:17+00:00",
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

- 2026-10-11T14:12:04+00:00: Promote source-checkout repair after AR-1776 and AR-1778 exact-main
  completion; it unblocks AR-1779 source-only fallback.

- 2026-10-11T14:13:59+00:00: Claimed by ar1798-source-checkout-luna56.

- 2026-10-11T14:14:35+00:00: Heartbeat by ar1798-source-checkout-luna56.

- 2026-10-11T14:16:11+00:00: Bound worktree
  agent-systems-benchmark-ar-1798-pinned-source-checkout-acquisition on branch
  feature/ar-1798-pinned-source-checkout-acquisition.

- 2026-10-11T14:16:26+00:00: Recorded command exit 0; command argv SHA-256
  2e3a457932064a4b1b3300359d2f06a5d99c1e54742aeef15adf6a1ab9c29bca.

- 2026-10-11T14:17:29+00:00: Recorded command exit 1; command argv SHA-256
  14a3093c47c7b991fee51d1747ba3390e9867621c57ada30bff9d5104335fbc1.

- 2026-10-11T14:17:39+00:00: Recorded command exit 1; command argv SHA-256
  d9e72bd4efa8fd1203c9290dcc0b84f3e4f4c4c864c210b61451da1bf74e91a0.

- 2026-10-11T14:17:50+00:00: Recorded command exit 1; command argv SHA-256
  2ed4a0a1873eff6440923853f2f950b5add081a3120bcf879594f20308aa807f.

- 2026-10-11T14:18:01+00:00: Recorded command exit 0; command argv SHA-256
  933daeb04abce554c13570ddc1d50edc120bd9b05f9236121612f127b5adb148.

- 2026-10-11T14:18:14+00:00: Recorded command exit 101; command argv SHA-256
  c7373b115e91c7c61d6e5ef48d16694e29de6f7403d5191531f4365e26d937c4.

- 2026-10-11T14:18:45+00:00: Recorded command exit 101; command argv SHA-256
  ac0cc5f1b0d87f32989315938703ca696ceab56aeda4ebec9e4bbf76c7db2ca9.

- 2026-10-11T14:18:56+00:00: Recorded command exit 0; command argv SHA-256
  933daeb04abce554c13570ddc1d50edc120bd9b05f9236121612f127b5adb148.

- 2026-10-11T14:19:09+00:00: Recorded command exit 101; command argv SHA-256
  ac0cc5f1b0d87f32989315938703ca696ceab56aeda4ebec9e4bbf76c7db2ca9.

- 2026-10-11T14:19:20+00:00: Recorded command exit 0; command argv SHA-256
  933daeb04abce554c13570ddc1d50edc120bd9b05f9236121612f127b5adb148.

- 2026-10-11T14:19:37+00:00: Recorded command exit 0; command argv SHA-256
  ac0cc5f1b0d87f32989315938703ca696ceab56aeda4ebec9e4bbf76c7db2ca9.

- 2026-10-11T14:19:50+00:00: Recorded command exit 0; command argv SHA-256
  933daeb04abce554c13570ddc1d50edc120bd9b05f9236121612f127b5adb148.

- 2026-10-11T14:20:02+00:00: Recorded command exit 101; command argv SHA-256
  ac0cc5f1b0d87f32989315938703ca696ceab56aeda4ebec9e4bbf76c7db2ca9.

- 2026-10-11T14:20:11+00:00: Recorded command exit 0; command argv SHA-256
  933daeb04abce554c13570ddc1d50edc120bd9b05f9236121612f127b5adb148.

- 2026-10-11T14:20:21+00:00: Recorded command exit 0; command argv SHA-256
  ac0cc5f1b0d87f32989315938703ca696ceab56aeda4ebec9e4bbf76c7db2ca9.

- 2026-10-11T14:20:29+00:00: Recorded command exit 0; command argv SHA-256
  2b6237a715a1af319b57958b12a0d6b1fd465487d462262366c43be4e2397e2a.

- 2026-10-11T14:20:36+00:00: Recorded command exit 0; command argv SHA-256
  fba9ea62fb7c1db9f35b352580f114579e964cacb92265446f1be600256d179c.

- 2026-10-11T14:21:06+00:00: Recorded command exit 0; command argv SHA-256
  003edfc2e5ab0c4138da9f9fa3454f0282d4210e2c9557b5f2732fbcf1ccb1b1.

- 2026-10-11T14:21:17+00:00: Recorded command exit 0; command argv SHA-256
  b3c9964fa12b483fa537d08c62ff6d9cb83d9ccd928e7a4a3fcafd1ab3ce4074.
