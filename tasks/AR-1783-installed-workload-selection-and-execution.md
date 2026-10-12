---
{
  "branch": "feature/ar-1783-installed-workload-selection-and-execution",
  "checkpoint_commit": "b8fc46907afd281781a49dbd04ccb0aa6337a850",
  "claim_expires": "2026-10-12T22:48:24+00:00",
  "depends_on": [
    "AR-1763",
    "AR-1764",
    "AR-1782"
  ],
  "id": "AR-1783",
  "next_action": "Integrate every development-supported installed workload bundle into selection and prove full run/sweep/report/compare consumes its exact recorded identity.",
  "observed_branch": "feature/ar-1783-installed-workload-selection-and-execution",
  "observed_dirty": 0,
  "observed_head": "ed93fd438730bdf2d68ac1f0f0056807f215a664",
  "owner": "ar1783-workload-selection-luna",
  "plan": "../plans/AR-1783-installed-workload-selection-and-execution.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_acceptance": {
    "evidence_class": "environmental",
    "evidence_digest": "",
    "evidence_ref": "",
    "spec_ref": "specs/AR-1783.json",
    "spec_revision": 2,
    "status": "pending"
  },
  "spec_ref": "specs/AR-1783.json",
  "spec_revision": 2,
  "status": "in_progress",
  "summary": "Select every development external workload and prove ASB uses its exact project-local identity end to end.",
  "task_revision": 26,
  "title": "Installed workload selection and execution",
  "updated_at": "2026-10-12T20:49:03+00:00",
  "worktree_key": "agent-systems-benchmark-ar-1783-installed-workload-selection-and-execution"
}
---

Make generated project catalogs and run/sweep consume the dedicated `asb workload`
records. Selection validates the installed workload's exact catalog revision,
identity, digest, license/terms compatibility, preparation state, adapter
version, and resource/platform requirements. Execution binds this identity into
the plan/run/record/report/compare evidence and rejects ambient, stale, revoked,
missing, incompatible, or manually substituted workload directories before a run
starts. Cover every AR-1781 workload ID—including SWE-mini—through a fresh
project's install, selection, bounded run and sweep, report, and comparison
journey. Development runs may use a controlled local/development backend where
needed; lack of production authorization or hosted qualification must not block
the suite from this end-to-end path.

- 2026-10-12T20:41:51+00:00: Dependencies AR-1763, AR-1764, and AR-1782 are durably done with
  acceptance evidence; promote for implementation.

- 2026-10-12T20:41:59+00:00: Claimed by ar1783-workload-selection-luna.

- 2026-10-12T20:42:47+00:00: Recorded command exit 0; command argv SHA-256
  91c481d4f1a058d3265e9102b0531d1eefe09cb45e5c1807ecda5e8c8b6b67d5.

- 2026-10-12T20:43:05+00:00: Checkpointed source commit b8fc46907afd281781a49dbd04ccb0aa6337a850.

- 2026-10-12T20:45:12+00:00: Recorded command exit 0; command argv SHA-256
  15bdc813ad2750ae688262967a32a5f5e34493aa64ac096b88091db382829347.

- 2026-10-12T20:45:31+00:00: Recorded command exit 1; command argv SHA-256
  96bc6a20aef6507ede6669f72a95e8e5cd3421bc4c3c176c86eb6c223a216973.

- 2026-10-12T20:45:47+00:00: Recorded command exit 0; command argv SHA-256
  3420e48a388c9ec83c9b28479ca01cf94a2337d61d26e396b712930ea1461a88.

- 2026-10-12T20:45:58+00:00: Recorded command exit 1; command argv SHA-256
  5cffee7c0bfd57b65d9794c05064ec0d1631c677dbb8952d45330ba4d7cf4250.

- 2026-10-12T20:46:23+00:00: Recorded command exit 0; command argv SHA-256
  d6a4561800f8e79160f1f30891a3f07ef4351a4749925c2eb7c421e26b160fb0.

- 2026-10-12T20:46:32+00:00: Recorded command exit 0; command argv SHA-256
  3420e48a388c9ec83c9b28479ca01cf94a2337d61d26e396b712930ea1461a88.

- 2026-10-12T20:47:02+00:00: Recorded command exit 0; command argv SHA-256
  d6a4561800f8e79160f1f30891a3f07ef4351a4749925c2eb7c421e26b160fb0.

- 2026-10-12T20:47:27+00:00: Recorded command exit 0; command argv SHA-256
  fa8b5b71edf3200d321ee3c09e348eccd55c38732142f7ba47f3a35c7d1b1356.

- 2026-10-12T20:48:06+00:00: Recorded command exit 0; command argv SHA-256
  d89c43b28f2a6ac6fb83707b9e20e8397e336513c903c1ccec8e4ccc3ec5d7dc.

- 2026-10-12T20:48:24+00:00: Heartbeat by ar1783-workload-selection-luna.

- 2026-10-12T20:48:27+00:00: Recorded command exit 0; command argv SHA-256
  c8aae51f164559c1c2734b310b508fd66c2e7b4e955a4544d735ff59c4a48832.

- 2026-10-12T20:49:03+00:00: Recorded command exit 0; command argv SHA-256
  3d4cc54827fa0ae5c402321266e4a8e9a84a69a17f7ff27b7a0f0c858d454784.
