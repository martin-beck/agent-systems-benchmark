---
{
  "branch": "repair/ar-1837-pr627-protected-main-identity",
  "checkpoint_commit": "",
  "claim_expires": "2026-10-13T06:46:53+00:00",
  "depends_on": [],
  "id": "AR-1837",
  "next_action": "Await terminal-success exact-head checks on PR #628 and independent review; then publish only with tools/integration/merge_pr.py and verify exact-main workflows.",
  "observed_branch": "repair/ar-1837-pr627-protected-main-identity",
  "observed_dirty": 0,
  "observed_head": "5e78a03bcd0988da32b681ad7c0bff9f2d17c68f",
  "owner": "ar1837-protected-main-repair-luna",
  "plan": "../plans/AR-1837-pr627-protected-main-identity-recovery.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_acceptance": {
    "evidence_class": "hosted",
    "evidence_digest": "",
    "evidence_ref": "",
    "spec_ref": "specs/AR-1837.json",
    "spec_revision": 1,
    "status": "pending"
  },
  "spec_ref": "specs/AR-1837.json",
  "spec_revision": 1,
  "status": "in_progress",
  "summary": "Recover current protected-main provenance after PR #627 was web-merged with a noreply committer, without rewriting history or weakening gates.",
  "task_revision": 16,
  "title": "PR #627 protected-main identity recovery",
  "updated_at": "2026-10-13T05:51:20+00:00",
  "worktree_key": "agent-systems-benchmark-ar-1837-pr627-protected-main-identity"
}
---

PR #627 was correctly reviewed and all exact-head checks passed, but its GitHub
merge commit `6ea3648e483f25179c3112b54ed7c348e141cad4` was created by
`GitHub <noreply@github.com>`. The protected-main provenance gate therefore
failed, as designed. Preserve that immutable historical merge and its failure
receipt. This AR owns only a forward recovery from the current main tip.

- Do not rewrite or force-update main, reuse the old-base PR, weaken the
  provenance policy, accept arbitrary signers, or alter unrelated ARs.
- Create the smallest signed-DCO recovery topic preserving the current product
  tree, review it independently, run all focused/full gates and exact-head CI,
  and publish a local signed two-parent merge only through
  `tools/integration/merge_pr.py`.
- Verify the repaired exact-main merge, every required post-merge workflow, and
  durable state before acceptance.

- 2026-10-13T05:30:19+00:00: Promoted as an independent forward recovery; it preserves PR #627
  history and requires local signed integration.

- 2026-10-13T05:30:22+00:00: Claimed by ar1837-protected-main-repair-luna.

- 2026-10-13T05:31:20+00:00: Recorded command exit 0; command argv SHA-256
  819507c80908e7d0f961bda582bed4551953e58ffbff136a489cbb200f46f81e.

- 2026-10-13T05:31:41+00:00: Recorded command exit 0; command argv SHA-256
  11b2dffef7908e9c3962538d06308618fb66a17e2faa9796b7050fbf6f5854a6.

- 2026-10-13T05:31:53+00:00: Recorded command exit 0; command argv SHA-256
  0abe2a88b3c0de762bc6784f9db549737556dd139bc87ce4a9f12e8c1efd2d58.

- 2026-10-13T05:32:08+00:00: Recorded command exit 0; command argv SHA-256
  12a7a6f9c64082cafd9bf0cc38aa8025e343cce8caa73825e85976580fdb3a98.

- 2026-10-13T05:33:58+00:00: Independent review at exact head
  5e78a03bcd0988da32b681ad7c0bff9f2d17c68f: metadata-only tree
  bf0cab4849363586724b29e649992319c642a6a7 equals current main parent tree; parent is immutable web
  merge 6ea3648e483f25179c3112b54ed7c348e141cad4; HEAD SSH signature and matching Martin Beck DCO
  verified; repository policy, current-topic-base, portable provenance, and 57 focused
  integration/policy tests passed. Hosted PR #628: provenance and 7 other checks passed; 6 required
  checks still pending at review time. No merge authorized until all exact-head required checks
  terminal-success.

- 2026-10-13T05:36:48+00:00: Created exact-tree signed-DCO recovery topic 5e78a03b from immutable
  main 6ea3648e; PR #628 exact-head provenance and most checks pass, with Rust, repository quality,
  and emulated-aarch64 still pending. Local repository policy and signature tests pass.

- 2026-10-13T05:39:46+00:00: Recorded command exit 0; command argv SHA-256
  88ca7864cb9316c6bd317236ea560f8f4d162e9c033542ebb6339f45412060de.

- 2026-10-13T05:40:22+00:00: Recorded command exit 0; command argv SHA-256
  acb9e76f0ab803d47bc3bb755b59054859f060c84e60cbce9137696dd1019947.

- 2026-10-13T05:40:30+00:00: Recorded command exit 1; command argv SHA-256
  e275ff2c2ee1c6c3c8f5c971b03b46fcf8d2f60f2961c35f7b0428f88534012d.

- 2026-10-13T05:46:53+00:00: Heartbeat by ar1837-protected-main-repair-luna.

- 2026-10-13T05:51:20+00:00: Recorded command exit 0; command argv SHA-256
  0bac8af44b521ce07e90307bdb641df182dc2efbf38166cc9e52da33de0c83a1.
