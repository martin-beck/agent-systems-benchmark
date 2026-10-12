---
{
  "branch": "repair/ar-1820-ar1772-provenance",
  "checkpoint_commit": "",
  "claim_expires": "",
  "depends_on": [],
  "id": "AR-1820",
  "next_action": "Await independent review of PR #614 exact head 7ebfd47a5ab6279779aa899ef83b27c6c4d86edd and the three remaining hosted checks; merge only through tools/integration/merge_pr.py after all green.",
  "observed_branch": "",
  "observed_dirty": 0,
  "observed_head": "",
  "owner": "",
  "plan": "../plans/AR-1820-ar1772-provenance.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_acceptance": {
    "evidence_class": "hosted",
    "evidence_digest": "sha256:fb4285c78143d1ab188cf6df66de051f2b72e15ba6a29364d48201daaad0c6ca",
    "evidence_ref": "quality/AR-1820-postmerge-receipt.json",
    "spec_ref": "specs/AR-1820.json",
    "spec_revision": 1,
    "status": "pass"
  },
  "spec_ref": "specs/AR-1820.json",
  "spec_revision": 1,
  "status": "done",
  "summary": "Requalify AR-1772 protected-main provenance and DCO without rewriting PR #602 or weakening gates.",
  "task_revision": 32,
  "title": "AR-1772 protected-main provenance and DCO requalification",
  "updated_at": "2026-10-12T14:41:17+00:00",
  "worktree_key": "agent-systems-benchmark-ar-1820-ar1772-provenance"
}
---

AR-1772 PR #602 is immutable historical evidence. Its reviewed topic tree and
two-parent merge tree are preserved, but exact-main runs 38210171658,
38210171401, and 38210171442 rejected the GitHub-generated protected-main
committer instead of the local integration identity. Create a clean,
signed-DCO, local-integration recovery path from current ASB main without
rewriting protected history, changing product behavior, weakening provenance or
DCO gates, or using the GitHub web merge path.

AR-1773 and AR-1775 remain unreleased until the requalification evidence is
durable. A recovery may be metadata-only and must preserve the exact reviewed
tree; any product-policy change requires separate explicit scope and review.


- 2026-10-12T13:56:38+00:00: Dependencies are empty; historical AR-1772 merge/provenance failures
  are preserved as explicit repair inputs.

- 2026-10-12T13:56:39+00:00: Claimed by ar1772-provenance-repair.

- 2026-10-12T13:56:44+00:00: Recorded command exit 0; command argv SHA-256
  b3a7436650549b1f343c833644c1d17e576769fc0ab60daca5a140f30d62e69b.

- 2026-10-12T13:57:02+00:00: Recorded command exit 0; command argv SHA-256
  17e60db663daca66edaef834fd2c7fc6883c9efe79d14a05ebc1c29e98bf2754.

- 2026-10-12T13:57:10+00:00: Recorded command exit 1; command argv SHA-256
  c5374811b3eca396dcc7c54451ae8f8c9f3f2f396fda8719dfbb0643102a1fc8.

- 2026-10-12T13:57:25+00:00: Heartbeat by ar1772-provenance-repair.

- 2026-10-12T13:57:30+00:00: Historical failure reproduced locally: policy rejects merge 4b14797a
  solely because its protected-main committer is GitHub <noreply@github.com>; no
  topology/tree/product failure observed.

- 2026-10-12T13:57:38+00:00: Recorded command exit 0; command argv SHA-256
  da38de82f8856a5dc11ad48985d141de06b1706d8c30007fd24ed2e0fe784078.

- 2026-10-12T13:57:45+00:00: Recorded command exit 0; command argv SHA-256
  ea1e6d2653d43a200bae742f96e125cdcb42bd771b53ef7b75839279b1e229af.

- 2026-10-12T13:57:49+00:00: Recorded command exit 0; command argv SHA-256
  76ea70f63216af01c15975df0408d98def7f1767f7de11945aee5559b691b2f4.

- 2026-10-12T13:58:02+00:00: Recorded command exit 0; command argv SHA-256
  fc19b2481840d562f8366e29c3e05b1b3e91afb33beac56db1c55ca0126b7f1d.

- 2026-10-12T13:58:12+00:00: Recorded command exit 0; command argv SHA-256
  ec57c0f2f929e45a15f8290a81f88d277d841df6dababc08694eaa0e55892e16.

- 2026-10-12T13:58:25+00:00: Recorded command exit 0; command argv SHA-256
  c3f8c81e3b9f903306c1ce46fce082e90fc9dc4e65f93165abd4e607c8c3158c.

- 2026-10-12T13:59:10+00:00: Recorded command exit 101; command argv SHA-256
  0a80e5fef7a7fd1f9619dda5154f8e236f7046b2d753e8434f5dc84d2c1342d8.

- 2026-10-12T14:01:23+00:00: Recorded command exit 0; command argv SHA-256
  d156e76f3656e3e6eea78977a9abfca92c5c85f64a98ad73bb2c528e9081480d.

- 2026-10-12T14:01:33+00:00: Recorded command exit 0; command argv SHA-256
  972f1a49ea5913f1ffcfc7a13411d5eedb146a435dd76e576c00be73eb0fd194.

- 2026-10-12T14:01:52+00:00: Recorded command exit 0; command argv SHA-256
  aabf3a5420ac888201c7f852a404a59517d13f6c70ba2a2b97bb8b3e74759dc4.

- 2026-10-12T14:01:57+00:00: Recorded command exit 0; command argv SHA-256
  3085ef57c940cfcc0d3c2008c158279cd763983abafdab55291a07fa5d0abf4b.

- 2026-10-12T14:04:56+00:00: PR #614 published from clean current-main topic. Local repository
  policy, SSH signature/DCO, fmt, locked workspace Clippy, and clean rerun of full locked workspace
  tests pass; 11 hosted checks are green including Portable protected-main provenance. First full
  test attempt exited 101 without a test failure; clean rerun passed and both outcomes are retained.

- 2026-10-12T14:13:27+00:00: Recorded command exit 1; command argv SHA-256
  1f1eb00f15d22cbde7d92ffe61d281990312cd5fc646a5cc83532693b57737b5.

- 2026-10-12T14:13:43+00:00: Recorded command exit 0; command argv SHA-256
  91e8605f9e787df9fafbe1113f7b1b963e79a580ed439feafb35322dc1aa21e4.

- 2026-10-12T14:13:55+00:00: Recorded command exit 0; command argv SHA-256
  ce1b374bbd45431315394336264884237bd53948518cec37aad55433494a3162.

- 2026-10-12T14:20:47+00:00: Heartbeat by ar1772-provenance-repair.

- 2026-10-12T14:39:52+00:00: Recorded command exit 1; command argv SHA-256
  9e7cc7b1eec9a291155952be797eea4a8b8110a615dcf946e6e181b8e0d46a9e.

- 2026-10-12T14:40:01+00:00: Recorded command exit 0; command argv SHA-256
  f4a7cbfd3601af9c4c65372d7cab222f39d954c3f0284efdccc2081939b36041.

- 2026-10-12T14:40:07+00:00: Recorded command exit 0; command argv SHA-256
  b9efa10aafdf26a3d816faeee4b03b131b6e217bebc8920622580c88eb5eef0b.

- 2026-10-12T14:40:47+00:00: Heartbeat by ar1772-provenance-repair.

- 2026-10-12T14:40:50+00:00: Recorded command exit 1; command argv SHA-256
  17cb78fa3e69410c36e36e98782e5432199c66963381054326e281e116d81e82.

- 2026-10-12T14:41:06+00:00: Recorded command exit 0; command argv SHA-256
  05d9cb614a6e27295cc591d474ece410ac68d92b0aae5e6d990d3540d3f8aac4.

- 2026-10-12T14:41:15+00:00: Accepted exact PR #614 merge 65b40d4e4ad2b7f9da84d4037cf683b88ce517b5,
  local protected-main policy pass, signed DCO metadata, and all 10 exact-head post-merge workflows
  terminal-success; historical PR #602 failures remain preserved.

- 2026-10-12T14:41:17+00:00: AR-1820 complete: PR #614 merged as
  65b40d4e4ad2b7f9da84d4037cf683b88ce517b5 with exact reviewed tree; signed DCO and local
  protected-main policy pass; 10 exact-head post-merge workflows terminal-success. Receipt
  quality/AR-1820-postmerge-receipt.json
  sha256:fb4285c78143d1ab188cf6df66de051f2b72e15ba6a29364d48201daaad0c6ca. Historical PR #602 and
  its failed runs remain immutable evidence. AR-1772/1773/1775 remain unreleased/planned pending
  their own dependencies.
