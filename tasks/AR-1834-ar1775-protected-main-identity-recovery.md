---
{
  "branch": "repair/ar-1834-ar1775-protected-main-identity",
  "checkpoint_commit": "",
  "depends_on": [],
  "id": "AR-1834",
  "next_action": "Claim a clean product worktree, create a signed-DCO metadata-only recovery topic from immutable main 356a669e, and publish only through merge_pr.py after independent review and green exact-head checks",
  "observed_dirty": 0,
  "observed_head": "",
  "owner": "",
  "plan": "../plans/AR-1834-ar1775-protected-main-identity-recovery.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_acceptance": {"evidence_class": "hosted", "evidence_digest": "", "evidence_ref": "", "spec_ref": "specs/AR-1834.json", "spec_revision": 1, "status": "pending"},
  "spec_ref": "specs/AR-1834.json",
  "spec_revision": 1,
  "status": "planned",
  "summary": "Recover AR-1775 protected-main provenance with the established local signed integration identity without rewriting historical merge 356a669e",
  "task_revision": 1,
  "title": "AR-1775 protected-main local identity recovery",
  "updated_at": "2026-10-13T00:30:00+00:00",
  "worktree_key": "agent-systems-benchmark-ar-1834-ar1775-protected-main-identity"
}
---

Historical AR-1775 merge `356a669e0c6ebbb6cd3675a119cba16fea486283` is immutable evidence. Its two-parent topology, reviewed tree, SSH signature and DCO are preserved, but GitHub Web Flow committed the protected merge as `GitHub <noreply@github.com>`. Portable protected-main provenance, Repository quality and Rust DCO therefore correctly reject it against the exact local integration identity.

This successor is metadata-only and forward-only. It must not alter `LOCAL_COMMITTER`, accept arbitrary signers, rewrite main, weaken any gate, change AR-1779 or AR-1783, or touch AR-1826--AR-1832. Use the established AR-1794/AR-1820 recovery path: clean isolated worktree, signed-DCO recovery topic preserving the current tree, independent exact-head review, focused/full gates, and `tools/integration/merge_pr.py` local signed merge.

- 2026-10-13T00:30:00+00:00: Created from current exact-main failures for AR-1775; historical merge and all failed workflow evidence remain immutable inputs.
