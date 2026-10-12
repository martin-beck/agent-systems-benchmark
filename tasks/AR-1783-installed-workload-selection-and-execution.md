---
{
  "branch": "feature/ar-1783-installed-workload-selection-and-execution",
  "checkpoint_commit": "e0474221c5a95e7e091265738b4ba05938767a47",
  "claim_expires": "2026-10-12T23:15:30+00:00",
  "depends_on": [
    "AR-1763",
    "AR-1764",
    "AR-1782"
  ],
  "id": "AR-1783",
  "next_action": "Integrate every development-supported installed workload bundle into selection and prove full run/sweep/report/compare consumes its exact recorded identity.",
  "observed_branch": "feature/ar-1783-installed-workload-selection-and-execution",
  "observed_dirty": 0,
  "observed_head": "e0474221c5a95e7e091265738b4ba05938767a47",
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
  "task_revision": 59,
  "title": "Installed workload selection and execution",
  "updated_at": "2026-10-12T21:15:30+00:00",
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

- 2026-10-12T20:49:19+00:00: Recorded command exit 0; command argv SHA-256
  f7e20666638201d8567703a3d7bc028e04a5e8e4c6c509de13d4976b0c91a669.

- 2026-10-12T20:49:40+00:00: Heartbeat by ar1783-workload-selection-luna.

- 2026-10-12T20:50:02+00:00: Recorded command exit 0; command argv SHA-256
  1024fc1e6cefa31b078f0b5d2ed85c7ea4586897b388bf1e308f62db58574461.

- 2026-10-12T20:50:13+00:00: Heartbeat by ar1783-workload-selection-luna.

- 2026-10-12T20:51:15+00:00: Recorded command exit 101; command argv SHA-256
  534f8613e1c316db8f0f46dd989e44e14a8ca74304a94791b4e4a1c59b2fbb1d.

- 2026-10-12T20:51:25+00:00: Recorded command exit 0; command argv SHA-256
  a5e91d8a6e356ba88fd4835c765ebd2411c75eacaa60f98e203295ec257bd3a5.

- 2026-10-12T20:51:39+00:00: Recorded command exit 0; command argv SHA-256
  ce7d45fffe378f17b6453e4e851b9576b579444d490ac31ccb139f845c58245e.

- 2026-10-12T20:51:54+00:00: Recorded command exit 0; command argv SHA-256
  ef19043205064ea87bec66574bfe246191c9311bc8c404cd8e347d08fbdc44f0.

- 2026-10-12T20:52:53+00:00: Recorded command exit 0; command argv SHA-256
  534f8613e1c316db8f0f46dd989e44e14a8ca74304a94791b4e4a1c59b2fbb1d.

- 2026-10-12T20:53:14+00:00: Recorded command exit 0; command argv SHA-256
  b36d15881eb65027724bc584f1b04db05666cd79896b100806e528307c04b65c.

- 2026-10-12T20:54:03+00:00: Recorded command exit 0; command argv SHA-256
  b49a90ee600c1023cee9ee9f9e8a4f9a06c53eef8c48d3932d88e482126c7c01.

- 2026-10-12T20:54:13+00:00: Recorded command exit 0; command argv SHA-256
  fcd17480931a7c27121bea1a4b4cdf0263b22a8542f304a02b7fe3a5e06895df.

- 2026-10-12T20:54:21+00:00: Recorded command exit 0; command argv SHA-256
  77082438719f647bf8d69589a3ecd2c061d48b8f41f8951d6ce69934a4f9550f.

- 2026-10-12T20:54:36+00:00: Checkpointed source commit 60ec53268f51284b6e0d52d4d08080de2c4a70f9.

- 2026-10-12T20:55:34+00:00: Independent read-only review of PR #623 (AR-1686): exact head
  93d75e28a7580ffbd7dbf6e543f5989c6dbe2688 against b8fc46907afd281781a49dbd04ccb0aa6337a850; diff is
  2 files/62 insertions, git diff --check clean; commit SSH signature and DCO pass; receipt runner
  digest matches source, paired ASB/TUI heads are recorded, and no forbidden secret/private-path
  material was found. Verdict: no implementation defect found in the reviewed diff, but not
  merge-ready yet because required exact-head CI remains in progress and AR-1686 is still
  in_progress; recheck all required checks before merge. Receipt lacks a final newline (style-only
  observation).

- 2026-10-12T21:02:59+00:00: Heartbeat by ar1783-workload-selection-luna.

- 2026-10-12T21:03:24+00:00: Recorded command exit 101; command argv SHA-256
  c8aae51f164559c1c2734b310b508fd66c2e7b4e955a4544d735ff59c4a48832.

- 2026-10-12T21:03:48+00:00: Recorded command exit 101; command argv SHA-256
  c8aae51f164559c1c2734b310b508fd66c2e7b4e955a4544d735ff59c4a48832.

- 2026-10-12T21:03:59+00:00: Recorded command exit 0; command argv SHA-256
  c8aae51f164559c1c2734b310b508fd66c2e7b4e955a4544d735ff59c4a48832.

- 2026-10-12T21:04:10+00:00: Recorded command exit 0; command argv SHA-256
  0ca500d01969eddb76c78415cf9bb0d0161afa9dee001b1e1e5192b0e2bac82f.

- 2026-10-12T21:04:18+00:00: Recorded command exit 0; command argv SHA-256
  2111be5240fe4e6cf926f478089ec83fc93a4f4d2c583611780516356ef070c0.

- 2026-10-12T21:07:27+00:00: Recorded command exit 0; command argv SHA-256
  428457ea9cc6a21bd46c5be6651ba2faccb5b0b07ddc2fd6f96eca730dea5ab7.

- 2026-10-12T21:07:49+00:00: Recorded command exit 0; command argv SHA-256
  b7c438686581cf292b6cd41497ceabb3af42dfed57e04dfd6cc89efed8e2d15b.

- 2026-10-12T21:07:54+00:00: Recorded command exit 0; command argv SHA-256
  c2e61a8cf32748be7d24820458d179b7640e90362a58b99bf690cdf7f6613ed1.

- 2026-10-12T21:08:37+00:00: Recorded command exit 0; command argv SHA-256
  f7e20666638201d8567703a3d7bc028e04a5e8e4c6c509de13d4976b0c91a669.

- 2026-10-12T21:08:55+00:00: Recorded command exit 0; command argv SHA-256
  1024fc1e6cefa31b078f0b5d2ed85c7ea4586897b388bf1e308f62db58574461.

- 2026-10-12T21:09:45+00:00: Recorded command exit 0; command argv SHA-256
  534f8613e1c316db8f0f46dd989e44e14a8ca74304a94791b4e4a1c59b2fbb1d.

- 2026-10-12T21:10:00+00:00: Checkpointed source commit e0474221c5a95e7e091265738b4ba05938767a47.

- 2026-10-12T21:15:30+00:00: Heartbeat by ar1783-workload-selection-luna.
