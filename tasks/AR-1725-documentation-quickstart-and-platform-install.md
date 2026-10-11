---
{
  "branch": "docs/ar-1725-quickstart-platform-install",
  "checkpoint_commit": "",
  "claim_expires": "",
  "depends_on": [
    "AR-1336",
    "AR-1338",
    "AR-1641",
    "AR-1642",
    "AR-1723",
    "AR-1724"
  ],
  "id": "AR-1725",
  "next_action": "Promote after coordinator integrity repair and dependency verification; audit existing docs, add the CLI/TUI tutorial and platform install matrix, then qualify links, commands, and visual evidence.",
  "observed_branch": "",
  "observed_dirty": 0,
  "observed_head": "",
  "owner": "",
  "plan": "../plans/AR-1725-documentation-quickstart-and-platform-install.md",
  "priority": "P1",
  "schema_version": 1,
  "spec_ref": "specs/AR-1725.json",
  "spec_revision": 1,
  "status": "open",
  "summary": "Publish a linked, executable tutorial for two-agent OpenRouter benchmarking, ASB TUI workflows, dependencies, and per-distribution installation.",
  "task_revision": 2,
  "title": "Document two-agent OpenRouter and TUI quickstarts",
  "updated_at": "2026-10-11T06:48:55+00:00",
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
