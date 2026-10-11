---
{
  "branch": "",
  "checkpoint_commit": "",
  "claim_expires": "2026-10-11T02:47:38+00:00",
  "depends_on": [
    "AR-1699",
    "AR-1700",
    "AR-1723",
    "AR-1724",
    "AR-1733"
  ],
  "id": "AR-1754",
  "next_action": "Promote after AR-1723, AR-1724, and cli2key qualification AR-1733 are current; inventory every run/sweep/easy/record/campaign/TUI entry point, implement default-online routing with explicit --local-mock and offline replay alternatives, then qualify positive and negative paths.",
  "owner": "codex-ar1754-online-integration",
  "plan": "../plans/AR-1754-default-online-live-workflow.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_ref": "specs/AR-1754.json",
  "spec_revision": 1,
  "status": "in_progress",
  "summary": "Make configured online/live provider execution the default for canonical run, sweep, easy, recording/campaign, and TUI handoff paths while keeping local mock and strict replay explicit alternatives.",
  "task_revision": 17,
  "title": "Make online execution the default across ASB workflows",
  "updated_at": "2026-10-11T02:25:38+00:00",
  "worktree_key": ""
}
---

The user-facing ASB workflow currently requires explicit online/live flags in
several places and makes `asb easy run|sweep` local-mock-only. Define one
consistent development contract: configured online provider execution is the
default, while `--local-mock` is an explicit credential-free alternative and
strict offline replay remains explicit. A live failure must remain a typed
failure; it must never silently fall back to mock or replay.

Cover canonical `run`, `sweep`, `benchmark-live`, `record-live`, recording
campaigns, `asb easy` commands, and the ASB/TUI handoff/configuration contract.
Do not modify the separate asb-tui repository in this AR; publish any required
cross-repository contract and dependency notes in the state plan. Preserve
credential-reference-only configuration, bounded progress, network/cost
warnings, provider/model/agent/workload identity, and all existing fail-closed
production authority checks.

- 2026-10-11T02:17:31+00:00: Dependencies AR-1699, AR-1700, AR-1723, AR-1724, and AR-1733 are
  released/current; promote for canonical online-default integration.

- 2026-10-11T02:17:38+00:00: Claimed by codex-ar1754-online-integration.

- 2026-10-11T02:17:46+00:00: Recorded command exit 0; command argv SHA-256
  904023b98f9c92c714e92042ea44849c3dc5cd79aee9bc2c4f6c2da4145ad191.

- 2026-10-11T02:19:31+00:00: Recorded command exit 101; command argv SHA-256
  56e66b0bd6b13838995dbe88c5b57a45c337007ffea52ed3e4fef3fe9a121b86.

- 2026-10-11T02:20:28+00:00: Recorded command exit 1; command argv SHA-256
  2640793016b9144188de39b32560b8e0bd1213cc844ee2f20b39d5e197ffb7be.

- 2026-10-11T02:20:31+00:00: Recorded command exit 0; command argv SHA-256
  c9b2d9ebecb13115258e662490978a2e2e84bf44fd4af7bd0795c4fe3905f166.

- 2026-10-11T02:21:20+00:00: Recorded command exit 0; command argv SHA-256
  56e66b0bd6b13838995dbe88c5b57a45c337007ffea52ed3e4fef3fe9a121b86.

- 2026-10-11T02:22:17+00:00: Recorded command exit 0; command argv SHA-256
  2b0c9cb4a2554e7e091651501f93c5f0fb1946899b15da96b5bb7ee72971131d.

- 2026-10-11T02:23:10+00:00: Recorded command exit 101; command argv SHA-256
  2640793016b9144188de39b32560b8e0bd1213cc844ee2f20b39d5e197ffb7be.

- 2026-10-11T02:24:29+00:00: Recorded command exit 0; command argv SHA-256
  b5bea41b6c623f7c09f1bf24dcae58ebab3c0cdd90ad966bc43a45b44867e12b.

- 2026-10-11T02:24:50+00:00: Recorded command exit 0; command argv SHA-256
  ce21884ced71789f65656793d4d30c9d0e99bbdb220e0cbe03652c5deb5418a5.

- 2026-10-11T02:24:53+00:00: Recorded command exit 0; command argv SHA-256
  d359c8c2c51a46c67222ccd336cda4aa608863df9cc790abedc47759ca8ae2a6.

- 2026-10-11T02:25:12+00:00: Recorded command exit 0; command argv SHA-256
  c75ed860523a165a754ae3874851f69b8742199e109ef5e19da40af2f02bf4fb.

- 2026-10-11T02:25:15+00:00: Recorded command exit 0; command argv SHA-256
  cddfd45995ac3b485d0e12f389dc008a425c1cb747b5c7dab0edb69284483137.

- 2026-10-11T02:25:34+00:00: Recorded command exit 1; command argv SHA-256
  f7e20666638201d8567703a3d7bc028e04a5e8e4c6c509de13d4976b0c91a669.

- 2026-10-11T02:25:38+00:00: Recorded command exit 1; command argv SHA-256
  71423a8987b2a4357131676a137bd93d92c8f2769116fa49a16efd3e972f5f82.
