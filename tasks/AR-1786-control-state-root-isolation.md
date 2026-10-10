---
{
  "branch": "codex/ar-1786-control-state",
  "checkpoint_commit": "",
  "claim_expires": "2026-10-10T22:38:36+00:00",
  "depends_on": [
    "AR-1711",
    "AR-1733"
  ],
  "id": "AR-1786",
  "next_action": "Promote after AR-1785's blocked review is reconciled; reproduce the ordinary workspace state-root collision in a dedicated worktree, repair isolation, and run repeated full gates.",
  "observed_branch": "codex/ar-1786-control-state",
  "observed_dirty": 0,
  "observed_head": "0ce437b966099c265c5c256e1f6ec47706922e09",
  "owner": "codex-ar1786-control-race",
  "plan": "../plans/AR-1786-control-state-root-isolation.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_ref": "specs/AR-1786.json",
  "spec_revision": 1,
  "status": "in_progress",
  "summary": "Eliminate control state-root ownership collisions in ordinary parallel workspace tests.",
  "task_revision": 13,
  "title": "Repair control state-root isolation race",
  "updated_at": "2026-10-10T20:38:36+00:00",
  "worktree_key": "agent-systems-benchmark-ar1786-control"
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

- 2026-10-10T20:37:03+00:00: Heartbeat by codex-ar1786-control-race.

- 2026-10-10T20:37:06+00:00: Recorded command exit 0; command argv SHA-256
  599225c1489a39ad2dc00bec49906045b80e7ec1d6d2e85a7979fcc17839ea18.

- 2026-10-10T20:37:34+00:00: Heartbeat by codex-ar1786-control-race.

- 2026-10-10T20:37:44+00:00: Recorded command exit 0; command argv SHA-256
  38bea93da73d24de012ae726701344927ad83be2b5f706d5a230e5ad5115ca3e.

- 2026-10-10T20:37:51+00:00: Recorded command exit 128; command argv SHA-256
  395df38e2a83dc2b6b853dd3add8fe4b6f5d7fca1de922c0a2c28c95a319196f.

- 2026-10-10T20:38:00+00:00: Recorded command exit 0; command argv SHA-256
  bd14aa25f0c6c589a618820a4804aca4bd35a25ac085f282cc64454a9a19b5d0.

- 2026-10-10T20:38:07+00:00: Recorded command exit 0; command argv SHA-256
  baa0f415515a673757c523a19f778c74680658da9724e8f0ba1a415a4f4e58fc.

- 2026-10-10T20:38:16+00:00: Recorded command exit 0; command argv SHA-256
  190aac4dfe591738746fa13f4f66c4344a9eccc776d7256b78108bbb8817487a.

- 2026-10-10T20:38:23+00:00: Recorded command exit 0; command argv SHA-256
  89901aa84043e8dc7e1eb76ddfef2f9c56172ab8c80f666ef8217add6a12066d.

- 2026-10-10T20:38:36+00:00: Heartbeat by codex-ar1786-control-race.
