---
{
  "branch": "feature/ar-1798-pinned-source-checkout-acquisition",
  "checkpoint_commit": "8eccc3843db2f2ec4dec2de7ca63f42c1b5c31d2",
  "claim_expires": "2026-10-11T16:57:09+00:00",
  "depends_on": [
    "AR-1776",
    "AR-1778"
  ],
  "id": "AR-1798",
  "next_action": "Monitor fresh exact-head CI for 23c7d6a7 and obtain independent re-review; do not merge.",
  "observed_branch": "feature/ar-1798-pinned-source-checkout-acquisition",
  "observed_dirty": 0,
  "observed_head": "ce16703c2ad73d227f6f507303dd7623302a7b74",
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
  "summary": "Coverage repair pushed at 23c7d6a7 with focused source-checkout failure-branch tests.",
  "task_revision": 66,
  "title": "Pinned source-checkout acquisition",
  "updated_at": "2026-10-11T15:07:32+00:00",
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

- 2026-10-11T14:21:30+00:00: Recorded command exit 0; command argv SHA-256
  d5dccbc6c6651ef4274c705414c03300959d6acc689f791d6a745d2b02977fbd.

- 2026-10-11T14:21:43+00:00: Recorded command exit 1; command argv SHA-256
  f70f16a14fd379aae4c812831c9ca931019ec3b1b6f6ec7018c000cf2ab0510c.

- 2026-10-11T14:22:43+00:00: Heartbeat by ar1798-source-checkout-luna56.

- 2026-10-11T14:23:50+00:00: Signed+DCO implementation published as PR #585; clean product tree and
  focused tests verified.

- 2026-10-11T14:25:22+00:00: Recorded command exit 0; command argv SHA-256
  f63fbc97f1a84d51d15d8f2b010e23a9d7105c7005d30a3d4a18033caaa99e1f.

- 2026-10-11T14:26:06+00:00: Recorded command exit 0; command argv SHA-256
  933daeb04abce554c13570ddc1d50edc120bd9b05f9236121612f127b5adb148.

- 2026-10-11T14:26:21+00:00: Recorded command exit 101; command argv SHA-256
  46d2070e30e9508f0e8b81418684ac93d3140b28e5b3f7c766f1cfe287618dec.

- 2026-10-11T14:26:53+00:00: Recorded command exit 0; command argv SHA-256
  933daeb04abce554c13570ddc1d50edc120bd9b05f9236121612f127b5adb148.

- 2026-10-11T14:27:15+00:00: Recorded command exit 0; command argv SHA-256
  efc35262fdcf932faf3533a24021f23efb18a2555e5e8db48831104e70029244.

- 2026-10-11T14:27:26+00:00: Recorded command exit 0; command argv SHA-256
  73e3cddd71a29698502efdd74801a0dee59da040f74dda7f6ed6f2b41ea0665c.

- 2026-10-11T14:27:36+00:00: Recorded command exit 0; command argv SHA-256
  8e7ad2e1157441140c544316fb3631277cea7d1b12d36633f955101fe28350c6.

- 2026-10-11T14:27:57+00:00: Recorded command exit 0; command argv SHA-256
  4ed105cec6782e66fb48bc67ddf0cb72bebe7cdaae4df9c73236fcce287b63b6.

- 2026-10-11T14:28:07+00:00: Recorded command exit 0; command argv SHA-256
  8e7ad2e1157441140c544316fb3631277cea7d1b12d36633f955101fe28350c6.

- 2026-10-11T14:28:17+00:00: Heartbeat by ar1798-source-checkout-luna56.

- 2026-10-11T14:28:22+00:00: Repair commits b305ff82, 56f78e0c, ba981448 add catalog-bound checkout,
  bounded transfer checks, and AR-1779 source-recipe routing; exact-head CI pending.

- 2026-10-11T14:28:32+00:00: Checkpointed source commit 8eccc3843db2f2ec4dec2de7ca63f42c1b5c31d2.

- 2026-10-11T14:31:12+00:00: Recorded command exit 0; command argv SHA-256
  71e8eed8c323dc1cf1ab137891922d8e89140465565d5ce0daaf889654a6f671.

- 2026-10-11T14:31:25+00:00: Recorded command exit 0; command argv SHA-256
  8e7ad2e1157441140c544316fb3631277cea7d1b12d36633f955101fe28350c6.

- 2026-10-11T14:31:34+00:00: Heartbeat by ar1798-source-checkout-luna56.

- 2026-10-11T14:31:37+00:00: One acquisition deadline and live clone/fetch bounds added; five
  deterministic checkout tests pass.

- 2026-10-11T14:31:56+00:00: Recorded command exit 0; command argv SHA-256
  84c31ee18c42da63e48682d806111fb81be7b63d630f9959d58698c319b5a876.

- 2026-10-11T14:34:18+00:00: Recorded command exit 0; command argv SHA-256
  1c0aef773eff0673ebaab13d1a46d344c82e37e5c708072e8a9846f54ade0ed5.

- 2026-10-11T14:34:49+00:00: Recorded command exit 0; command argv SHA-256
  8e7ad2e1157441140c544316fb3631277cea7d1b12d36633f955101fe28350c6.

- 2026-10-11T14:35:07+00:00: Heartbeat by ar1798-source-checkout-luna56.

- 2026-10-11T14:50:07+00:00: Heartbeat by ar1798-source-checkout-luna56.

- 2026-10-11T14:51:21+00:00: Recorded command exit 1; command argv SHA-256
  13feac61902929353f82e778f4e95bb4a70bcc913d699e9cf960e9aaa310820f.

- 2026-10-11T14:51:49+00:00: Exact product head e25983f6; clean worktree. Focused source-checkout
  tests 7 passed; diagnostic contract 9 passed; cargo clippy asb-cli all targets offline with
  warnings denied passed; git diff --check passed. Fresh exact-head CI pending.

- 2026-10-11T14:52:07+00:00: Recorded command exit 1; command argv SHA-256
  2776d7d2255edbb9290f03d5e56b59789cc7e21d74c0c1bb498390d1e64400c4.

- 2026-10-11T14:52:35+00:00: Heartbeat by ar1798-source-checkout-luna56.

- 2026-10-11T14:52:37+00:00: Recorded command exit 0; command argv SHA-256
  2776d7d2255edbb9290f03d5e56b59789cc7e21d74c0c1bb498390d1e64400c4.

- 2026-10-11T14:56:38+00:00: Recorded command exit 1; command argv SHA-256
  564b86f907ac793e229b4c30b3622c7121c37e54a992170eefcf1b88a948d95e.

- 2026-10-11T14:56:51+00:00: Exact product head ce16703c2ad73d227f6f507303dd7623302a7b74; clean
  worktree. workflow_transcript 3 passed; diagnostic_contract 9 passed; clippy all asb-cli targets
  offline passed; diff-check passed. Updated cli_source_sha256 to
  ce392396a0dde999dfbe58437039eb31413d5016562a7bb0b5c27c38a8aaca03. Fresh exact-head CI pending.

- 2026-10-11T14:56:54+00:00: Recorded command exit 0; command argv SHA-256
  d0a5a8f396e21b13fc4b5097d35443a2da50cf0b7abf7b95e2a4a40a7181d61f.

- 2026-10-11T14:57:09+00:00: Heartbeat by ar1798-source-checkout-luna56.

- 2026-10-11T14:57:13+00:00: Recorded command exit 0; command argv SHA-256
  58063010f5fa48ea54966f2175222b4cb499a1c842b889445ce839e9269ade1d.

- 2026-10-11T15:07:29+00:00: Exact product head 23c7d6a7; clean worktree. Source-checkout focused
  tests 9 passed; source_checkout coverage 92.66% lines / 95.13% line execution; cargo clippy
  asb-cli all targets offline passed; git diff --check passed. Locked workspace coverage measured
  88.68% (baseline workspace deficit remains below 90% floor); no threshold weakening.

- 2026-10-11T15:07:32+00:00: Recorded command exit 0; command argv SHA-256
  c1819ac8c7e93322f21e89bd4690764345a79402deed079640e01ca15d81da55.
