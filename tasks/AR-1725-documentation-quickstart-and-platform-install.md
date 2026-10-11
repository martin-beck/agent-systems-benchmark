---
{
  "branch": "docs/ar-1725-quickstart-platform-install",
  "checkpoint_commit": "",
  "claim_expires": "2026-10-11T08:49:01+00:00",
  "depends_on": [
    "AR-1336",
    "AR-1338",
    "AR-1641",
    "AR-1642",
    "AR-1723",
    "AR-1724"
  ],
  "id": "AR-1725",
  "next_action": "Focused tutorial tests, guide_examples, link audit, and diff review pass. Full workspace cargo test has one unrelated mini_swe cancellation failure because handoffctl runs from state root and the temp product path overlaps the detected repository; rerun from product-bound wrapper or record as environment blocker. Platform validator independently fails baseline stale native report ancestry/tree.",
  "observed_branch": "docs/ar-1725-quickstart-platform-install",
  "observed_dirty": 0,
  "observed_head": "d77230ae46ce2a4a4b35a47b7f2ee9f7bc9b677b",
  "owner": "ar1725-docs-luna56",
  "plan": "../plans/AR-1725-documentation-quickstart-and-platform-install.md",
  "priority": "P1",
  "schema_version": 1,
  "spec_ref": "specs/AR-1725.json",
  "spec_revision": 1,
  "status": "in_progress",
  "summary": "Publish a linked, executable tutorial for two-agent OpenRouter benchmarking, ASB TUI workflows, dependencies, and per-distribution installation.",
  "task_revision": 20,
  "title": "Document two-agent OpenRouter and TUI quickstarts",
  "updated_at": "2026-10-11T06:59:39+00:00",
  "worktree_key": "agent-systems-benchmark-ar-1725-documentation-quickstart-and-platform-install"
}
---

Create one short, linked tutorial that starts with installation and setup, selects
two compatible agents and a catalog-advertised OpenRouter free model, runs a
bounded workload against the remote provider, reports results, and explains the
explicit `--local-mock` and offline replay alternatives. Add the equivalent ASB
TUI journey (install, setup, select agents/provider/model, launch, monitor,
report, compare, and recover) with links to the asb-tui documentation boundary.

Add a dependency/install matrix for supported distributions, including pinned
Rust/toolchain requirements, required sandbox utilities, package-manager
commands, architecture notes, and actionable missing-dependency diagnostics.
Link the tutorial from README, QUICKSTART, OPERATOR_QUICKSTART, and the workflow
index. Prefer repository-owned screenshots or terminal transcript images made by
the bounded tutorial runner; never include credentials, private paths, or raw
provider content. Keep live calls opt-in where the current command contract
requires it, and label development/live, local-mock, replay, and qualification
evidence accurately.

Required evidence: tutorial-contract validation, executable command checks,
broken-link and schema checks, per-distribution dependency review, accessibility
text alternatives for every image, focused documentation gates, full applicable
gates, independent review, and exact-head hosted CI.

- 2026-10-11T06:48:55+00:00: Dependencies AR-1336, AR-1338, AR-1641, AR-1642, AR-1723, and AR-1724
  are done with checkpoints; promote documentation work.

- 2026-10-11T06:49:01+00:00: Claimed by ar1725-docs-luna56.

- 2026-10-11T06:49:31+00:00: Recorded command exit 0; command argv SHA-256
  d37193f8b9ad44178b3bff0cac7209222030e84fb25b93f00b19c585f8613a80.

- 2026-10-11T06:50:38+00:00: Recorded command exit 0; command argv SHA-256
  3bbe6c566d9770a7f1e041418b0ef9d95bc000e60deb378c762e2bf9ad167037.

- 2026-10-11T06:51:17+00:00: Recorded command exit 1; command argv SHA-256
  e0a0bb647f155e5f5e9fbfdffe12d5b8c3fc184b58478142d806a3d3ba2ca235.

- 2026-10-11T06:51:31+00:00: Recorded command exit 1; command argv SHA-256
  d99fd3918f7a0ac25cf855ebbf388c5391b566e5bfb0167fd628c89c84c3703b.

- 2026-10-11T06:51:44+00:00: Recorded command exit 1; command argv SHA-256
  bec4f6ab4d3e6d5132eaa43c8346d38f44bdffd66910652f8d66aa0482376e34.

- 2026-10-11T06:51:58+00:00: Recorded command exit 1; command argv SHA-256
  f278f9f9e4a993604fd5adc660ce4f0c5c2f181fc5e0488f47e535c30ba52ccc.

- 2026-10-11T06:52:31+00:00: Recorded command exit 0; command argv SHA-256
  a940268f1a16b759cfb4943e16441addda9e02aa82cc87db8e87ca86c73fed0e.

- 2026-10-11T06:52:43+00:00: Reconciled branch/worktree metadata through declared docs branch;
  verified baseline platform validator failure on untouched origin/main; tutorial freshness and
  guide_examples pass.

- 2026-10-11T06:53:07+00:00: Recorded command exit 101; command argv SHA-256
  50ae99437335f68b1a359bca86cdd88eb1a713a77ce847602ccf0fc639fc13b7.

- 2026-10-11T06:53:26+00:00: Full workspace gate attempted through handoffctl: 221 passed, 1
  ignored, 1 pre-existing environment failure in
  mini_swe::cancellation_leaves_no_runnable_owned_descendant; no product files changed.

- 2026-10-11T06:55:10+00:00: Recorded command exit 101; command argv SHA-256
  534f8613e1c316db8f0f46dd989e44e14a8ca74304a94791b4e4a1c59b2fbb1d.

- 2026-10-11T06:55:20+00:00: Recorded command exit 0; command argv SHA-256
  291fabd83bf08a767418181c052af16df89b1ad4479b78758e72c89c9c1f0ad8.

- 2026-10-11T06:55:32+00:00: Recorded command exit 0; command argv SHA-256
  172e6767dea5102755e50e3776e15187fde7c7a3abccc9a1a2d94cc4bb7cbc8f.

- 2026-10-11T06:55:47+00:00: Recorded command exit 0; command argv SHA-256
  8ad8454be93e66e9620734215054c97b284dbe7368e5072633ee0df0b3db51aa.

- 2026-10-11T06:56:03+00:00: Recorded command exit 8; command argv SHA-256
  4d6bb5ff2d1fb0ee1a20a2ba91c20c542ca5f3e1e9630001fef585e75ac76810.

- 2026-10-11T06:59:39+00:00: Recorded command exit 0; command argv SHA-256
  63a11cb1501eb157019bfa1b8d99416730b8f8052b5a1311148b8a3b737b3ac1.
