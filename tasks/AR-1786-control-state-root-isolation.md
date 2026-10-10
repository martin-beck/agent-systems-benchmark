---
{
  "branch": "",
  "checkpoint_commit": "",
  "claim_expires": "2026-10-10T22:36:22+00:00",
  "depends_on": [
    "AR-1711",
    "AR-1733"
  ],
  "id": "AR-1786",
  "next_action": "Promote after AR-1785's blocked review is reconciled; reproduce the ordinary workspace state-root collision in a dedicated worktree, repair isolation, and run repeated full gates.",
  "observed_branch": "",
  "observed_dirty": 0,
  "observed_head": "",
  "owner": "codex-ar1786-control-race",
  "plan": "../plans/AR-1786-control-state-root-isolation.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_ref": "specs/AR-1786.json",
  "spec_revision": 1,
  "status": "in_progress",
  "summary": "Eliminate control state-root ownership collisions in ordinary parallel workspace tests.",
  "task_revision": 3,
  "title": "Repair control state-root isolation race",
  "updated_at": "2026-10-10T20:36:22+00:00",
  "worktree_key": ""
}
---

This development repair is independent of GitHub Reviews API approvals. It requires
independent technical review, exact-head CI, signed DCO integration, and post-merge
verification. It must not weaken control ownership or convert the full workspace
gate into a serial-only or exclusion-based check.

- 2026-10-10T20:35:00+00:00: Created from the AR-1785 independent review blocker. Two
  ordinary `cargo test --locked --workspace` runs at e9ad590e failed in distinct
  `asb-cli::control` tests with `control state root is already owned`, while isolated
  runs passed. No AR-1785 publication or merge is authorized until this race is repaired.

- 2026-10-10T20:34:12+00:00: AR-1711 and AR-1733 are done; promote control state-root isolation
  repair discovered by AR-1785 exact-head review

- 2026-10-10T20:36:22+00:00: Claimed by codex-ar1786-control-race.
