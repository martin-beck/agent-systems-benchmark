---
{
  "branch": "repair/ar-1837-pr627-protected-main-identity",
  "checkpoint_commit": "",
  "claim_expires": "2026-10-13T06:30:22+00:00",
  "depends_on": [],
  "id": "AR-1837",
  "next_action": "Create a clean signed-DCO recovery topic from the current main tip, obtain independent exact-head review and green CI, then publish only with merge_pr.py.",
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
  "task_revision": 8,
  "title": "PR #627 protected-main identity recovery",
  "updated_at": "2026-10-13T05:31:53+00:00",
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
