---
{
  "branch": "feature/ar-1825-source-to-executable-adapter-runtime-contract",
  "checkpoint_commit": "acf439195f13df0c06fe53f2a1924d794d600e56",
  "claim_expires": "2026-10-12T20:52:28+00:00",
  "depends_on": [
    "AR-1776",
    "AR-1778"
  ],
  "id": "AR-1825",
  "next_action": "Run exact-head full gates on PR #622 at 4e37d542; obtain independent re-review and green CI before protected merge.",
  "observed_branch": "feature/ar-1825-source-to-executable-adapter-runtime-contract",
  "observed_dirty": 0,
  "observed_head": "19ce887552b0c2921c437df420bf7a8f0563f60a",
  "owner": "ar1825-implementation-luna",
  "plan": "../plans/AR-1825-bounded-source-to-executable-adapter-runtime-contract.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_acceptance": {
    "evidence_class": "contract-test",
    "evidence_digest": "",
    "evidence_ref": "",
    "spec_ref": "specs/AR-1825.json",
    "spec_revision": 1,
    "status": "pending"
  },
  "spec_ref": "specs/AR-1825.json",
  "spec_revision": 1,
  "status": "in_progress",
  "summary": "Implement bounded source adapter contract",
  "task_revision": 111,
  "title": "Bounded source-to-executable adapter/runtime contract",
  "updated_at": "2026-10-12T19:19:37+00:00",
  "worktree_key": "agent-systems-benchmark-ar-1825-source-to-executable-adapter-runtime-contract"
}
---

AR-1779's source fallback correctly refuses to claim success for source or
archive catalog entries that do not identify a verified executable adapter. This
successor defines the missing contract without weakening that fail-closed
boundary. It must turn an immutable catalog source revision into a usable,
project-local executable only through a declared per-tool adapter and bounded
runtime/dependency closure.

The contract must specify, for every supported tool, the immutable source
revision, package/install strategy, bounded project-local dependency and runtime
closure, launcher/entrypoint identity, regular-file or symlink policy and mode
checks, and the exact supported/unsupported outcome. It must include offline
fixtures and content-minimized receipts covering source, adapter, dependencies,
entrypoint, smoke result, rollback and retention. Publication is transactional:
failed materialization or smoke verification leaves no false usable state and
retains the last known-good installation according to an explicit bounded
retention policy.

The implementation must reject unknown strategies, archive outputs without a
verified adapter, missing or substituted entrypoints, runtime mismatches,
unbounded installers, and any runtime download. Unknown tool support is not an
implicit generic source build. The support matrix must make each tool's
strategy, runtime closure, launcher identity, offline fixture and qualification
status explicit, including unsupported rows where no safe adapter exists.

AR-1779 remains pending and must link this successor in its next action until
AR-1825's acceptance evidence is independently reviewed and durable.

- 2026-10-12T18:13:50+00:00: Dependencies AR-1776 and AR-1778 are done; open successor for bounded
  source-to-executable adapter/runtime contract.

- 2026-10-12T18:17:19+00:00: Claimed by ar1825-implementation-luna.

- 2026-10-12T18:17:40+00:00: Recorded command exit 0; command argv SHA-256
  80ee1d92863e14a0a0969fb334f047b353fafe77826c973800ab01aeef639b70.

- 2026-10-12T18:17:52+00:00: Recorded command exit 0; command argv SHA-256
  b11bd0c2660240422a6522d1777832fd1181c5d3a9fe4c3934abba4f1cac4021.

- 2026-10-12T18:18:10+00:00: Heartbeat by ar1825-implementation-luna.

- 2026-10-12T18:24:59+00:00: Recorded command exit 0; command argv SHA-256
  cfddfc1b8766f37e6aede838c174f3c1396c0e28ea1eae79d472cf1b97a74f13.

- 2026-10-12T18:25:51+00:00: Recorded command exit 101; command argv SHA-256
  a6de7611398a4c02deac7a4990906dc038e73008d5cd7908029ebb6a4af3dfb3.

- 2026-10-12T18:26:14+00:00: Heartbeat by ar1825-implementation-luna.

- 2026-10-12T18:26:31+00:00: Recorded command exit 0; command argv SHA-256
  a6de7611398a4c02deac7a4990906dc038e73008d5cd7908029ebb6a4af3dfb3.

- 2026-10-12T18:27:17+00:00: Recorded command exit 0; command argv SHA-256
  a37d35e290b63ddfe8627f9bdb0474ef6d3f2654ade91f05cd9a55c3a713fceb.

- 2026-10-12T18:28:27+00:00: Recorded command exit 0; command argv SHA-256
  cfddfc1b8766f37e6aede838c174f3c1396c0e28ea1eae79d472cf1b97a74f13.

- 2026-10-12T18:29:02+00:00: Recorded command exit 0; command argv SHA-256
  a6de7611398a4c02deac7a4990906dc038e73008d5cd7908029ebb6a4af3dfb3.

- 2026-10-12T18:29:25+00:00: Heartbeat by ar1825-implementation-luna.

- 2026-10-12T18:29:35+00:00: Recorded command exit 0; command argv SHA-256
  cfddfc1b8766f37e6aede838c174f3c1396c0e28ea1eae79d472cf1b97a74f13.

- 2026-10-12T18:29:58+00:00: Heartbeat by ar1825-implementation-luna.

- 2026-10-12T18:30:05+00:00: Recorded command exit 0; command argv SHA-256
  a6de7611398a4c02deac7a4990906dc038e73008d5cd7908029ebb6a4af3dfb3.

- 2026-10-12T18:30:29+00:00: Heartbeat by ar1825-implementation-luna.

- 2026-10-12T18:30:57+00:00: Recorded command exit 0; command argv SHA-256
  3d4cc54827fa0ae5c402321266e4a8e9a84a69a17f7ff27b7a0f0c858d454784.

- 2026-10-12T18:31:21+00:00: Heartbeat by ar1825-implementation-luna.

- 2026-10-12T18:31:45+00:00: Recorded command exit 0; command argv SHA-256
  cfddfc1b8766f37e6aede838c174f3c1396c0e28ea1eae79d472cf1b97a74f13.

- 2026-10-12T18:32:08+00:00: Heartbeat by ar1825-implementation-luna.

- 2026-10-12T18:32:14+00:00: Recorded command exit 0; command argv SHA-256
  a6de7611398a4c02deac7a4990906dc038e73008d5cd7908029ebb6a4af3dfb3.

- 2026-10-12T18:34:00+00:00: Recorded command exit 0; command argv SHA-256
  cfddfc1b8766f37e6aede838c174f3c1396c0e28ea1eae79d472cf1b97a74f13.

- 2026-10-12T18:34:24+00:00: Heartbeat by ar1825-implementation-luna.

- 2026-10-12T18:34:35+00:00: Recorded command exit 0; command argv SHA-256
  a6de7611398a4c02deac7a4990906dc038e73008d5cd7908029ebb6a4af3dfb3.

- 2026-10-12T18:35:00+00:00: Heartbeat by ar1825-implementation-luna.

- 2026-10-12T18:35:24+00:00: Recorded command exit 0; command argv SHA-256
  3d4cc54827fa0ae5c402321266e4a8e9a84a69a17f7ff27b7a0f0c858d454784.

- 2026-10-12T18:35:55+00:00: Recorded command exit 0; command argv SHA-256
  cfddfc1b8766f37e6aede838c174f3c1396c0e28ea1eae79d472cf1b97a74f13.

- 2026-10-12T18:36:18+00:00: Heartbeat by ar1825-implementation-luna.

- 2026-10-12T18:36:26+00:00: Recorded command exit 0; command argv SHA-256
  a6de7611398a4c02deac7a4990906dc038e73008d5cd7908029ebb6a4af3dfb3.

- 2026-10-12T18:36:50+00:00: Heartbeat by ar1825-implementation-luna.

- 2026-10-12T18:37:14+00:00: Recorded command exit 0; command argv SHA-256
  4d940a900f8db64b3054d3aaf1a69c486034a93395c2a14b81c9e5a1b07c0198.

- 2026-10-12T18:37:34+00:00: Recorded command exit 0; command argv SHA-256
  266833fff66de91dcd4fa12caaa637b13ea66034974a3dbedf6d3ad08e096982.

- 2026-10-12T18:37:49+00:00: Recorded command exit 0; command argv SHA-256
  321c534968b9c456a64caf5e6d09b63780c507f97548057d2bfe31edcfd73ff4.

- 2026-10-12T18:38:16+00:00: Recorded command exit 1; command argv SHA-256
  b7c438686581cf292b6cd41497ceabb3af42dfed57e04dfd6cc89efed8e2d15b.

- 2026-10-12T18:38:38+00:00: Heartbeat by ar1825-implementation-luna.

- 2026-10-12T18:39:34+00:00: Heartbeat by ar1825-implementation-luna.

- 2026-10-12T18:39:44+00:00: Recorded command exit 0; command argv SHA-256
  a6de7611398a4c02deac7a4990906dc038e73008d5cd7908029ebb6a4af3dfb3.

- 2026-10-12T18:40:10+00:00: Heartbeat by ar1825-implementation-luna.

- 2026-10-12T18:40:36+00:00: Recorded command exit 0; command argv SHA-256
  3d4cc54827fa0ae5c402321266e4a8e9a84a69a17f7ff27b7a0f0c858d454784.

- 2026-10-12T18:41:19+00:00: Heartbeat by ar1825-implementation-luna.

- 2026-10-12T18:41:26+00:00: Recorded command exit 0; command argv SHA-256
  f7e20666638201d8567703a3d7bc028e04a5e8e4c6c509de13d4976b0c91a669.

- 2026-10-12T18:42:03+00:00: Recorded command exit 101; command argv SHA-256
  1024fc1e6cefa31b078f0b5d2ed85c7ea4586897b388bf1e308f62db58574461.

- 2026-10-12T18:42:32+00:00: Recorded command exit 0; command argv SHA-256
  cfddfc1b8766f37e6aede838c174f3c1396c0e28ea1eae79d472cf1b97a74f13.

- 2026-10-12T18:42:55+00:00: Recorded command exit 0; command argv SHA-256
  e274074a17e0682b5edf79b5caca577a41520ed9ca8daf9342f0433b678582a1.

- 2026-10-12T18:43:20+00:00: Recorded command exit 0; command argv SHA-256
  ae229ef9248bd926ee0e5c6ac3a95523d7b1f514ff01301a70dd92a4300e0127.

- 2026-10-12T18:43:48+00:00: Recorded command exit 0; command argv SHA-256
  1024fc1e6cefa31b078f0b5d2ed85c7ea4586897b388bf1e308f62db58574461.

- 2026-10-12T18:45:01+00:00: Recorded command exit 101; command argv SHA-256
  534f8613e1c316db8f0f46dd989e44e14a8ca74304a94791b4e4a1c59b2fbb1d.

- 2026-10-12T18:45:59+00:00: Heartbeat by ar1825-implementation-luna.

- 2026-10-12T18:46:11+00:00: Recorded command exit 0; command argv SHA-256
  cfddfc1b8766f37e6aede838c174f3c1396c0e28ea1eae79d472cf1b97a74f13.

- 2026-10-12T18:46:35+00:00: Recorded command exit 0; command argv SHA-256
  3df6ea4248e788ba05ee59cda07137e7f77d1db86c2da3ca139b267dc2f130ab.

- 2026-10-12T18:46:57+00:00: Recorded command exit 0; command argv SHA-256
  f9256b7f879ad474b4061a1739716a82204bceecb138b2b939d0c2c7f36b239a.

- 2026-10-12T18:47:34+00:00: Recorded command exit 0; command argv SHA-256
  d32df0a8f7802e39b9a6743613fff16ac35e4dddba3905aa783a204f77a7e43f.

- 2026-10-12T18:48:37+00:00: Recorded command exit 101; command argv SHA-256
  534f8613e1c316db8f0f46dd989e44e14a8ca74304a94791b4e4a1c59b2fbb1d.

- 2026-10-12T18:48:54+00:00: Recorded command exit 0; command argv SHA-256
  a5e91d8a6e356ba88fd4835c765ebd2411c75eacaa60f98e203295ec257bd3a5.

- 2026-10-12T18:49:15+00:00: Recorded command exit 0; command argv SHA-256
  110a36e04acb1ef13bcee00422062d2e5cccd7c1447b608337b4f76f676ae949.

- 2026-10-12T18:49:50+00:00: Recorded command exit 0; command argv SHA-256
  ef19043205064ea87bec66574bfe246191c9311bc8c404cd8e347d08fbdc44f0.

- 2026-10-12T18:50:33+00:00: Recorded command exit 0; command argv SHA-256
  8f020a8266d9eda8e0ed704996e99736ad1cf29c0294cefaaa456dc2d9fb4d92.

- 2026-10-12T18:51:36+00:00: Recorded command exit 0; command argv SHA-256
  b49a90ee600c1023cee9ee9f9e8a4f9a06c53eef8c48d3932d88e482126c7c01.

- 2026-10-12T18:52:02+00:00: Recorded command exit 0; command argv SHA-256
  b7c438686581cf292b6cd41497ceabb3af42dfed57e04dfd6cc89efed8e2d15b.

- 2026-10-12T18:52:28+00:00: Heartbeat by ar1825-implementation-luna.

- 2026-10-12T18:52:41+00:00: Recorded command exit 0; command argv SHA-256
  560d920083f813f28bcb66d2c40257f9c447fd8e17f1b9941c46f4f65a79d5d2.

- 2026-10-12T18:53:58+00:00: Recorded command exit 0; command argv SHA-256
  534f8613e1c316db8f0f46dd989e44e14a8ca74304a94791b4e4a1c59b2fbb1d.

- 2026-10-12T18:54:28+00:00: Recorded command exit 0; command argv SHA-256
  fe03cedc860fa02bc8d48bc5f4295100a675cfd7b0d6370053eac69a861635f7.

- 2026-10-12T18:54:54+00:00: Recorded command exit 0; command argv SHA-256
  179fad915600cc17cbef4ae8e3652c54244075bf049ba5607d1c343b096b8d72.

- 2026-10-12T18:55:21+00:00: Recorded command exit 8; command argv SHA-256
  7db1d034ff356e5d652eb0517359ac6a7bdf36e5eb39faad512d73620f2fad76.

- 2026-10-12T18:57:16+00:00: AR-1825 implementation is ready for independent review: immutable
  matrix, closed package/runtime strategies, launcher and symlink checks, offline receipts,
  rollback/retention, hostile negatives, and source output race validation. Product gates passed:
  fmt, clippy, full workspace tests, docs, release build; all four commits SSH-signed with DCO.
  Runtime gpt-5.6-luna medium.

- 2026-10-12T18:57:20+00:00: Checkpointed source commit 1e3d15f6bdd568f7e3ef1838194e8f84f8ec500c.

- 2026-10-12T18:58:22+00:00: Recorded command exit 8; command argv SHA-256
  7db1d034ff356e5d652eb0517359ac6a7bdf36e5eb39faad512d73620f2fad76.

- 2026-10-12T18:59:17+00:00: Recorded command exit 8; command argv SHA-256
  7db1d034ff356e5d652eb0517359ac6a7bdf36e5eb39faad512d73620f2fad76.

- 2026-10-12T19:00:17+00:00: Independent review found four concrete safety defects before merge:
  rename-before-directory-fsync can orphan executable without receipt/config; pathname
  validation/smoke replacement race; smoke output capture is unbounded before max check;
  source-build staging leaks on inspection/read failure. No merge authorized.

- 2026-10-12T19:01:57+00:00: Recorded command exit 1; command argv SHA-256
  cfddfc1b8766f37e6aede838c174f3c1396c0e28ea1eae79d472cf1b97a74f13.

- 2026-10-12T19:02:36+00:00: Recorded command exit 0; command argv SHA-256
  cfddfc1b8766f37e6aede838c174f3c1396c0e28ea1eae79d472cf1b97a74f13.

- 2026-10-12T19:03:12+00:00: Recorded command exit 0; command argv SHA-256
  b20912e10e2ccb08a481591e01c8449fc3fa1d1feaa70c8206890f1e66ba03df.

- 2026-10-12T19:03:57+00:00: Recorded command exit 0; command argv SHA-256
  cfddfc1b8766f37e6aede838c174f3c1396c0e28ea1eae79d472cf1b97a74f13.

- 2026-10-12T19:04:26+00:00: Recorded command exit 0; command argv SHA-256
  b20912e10e2ccb08a481591e01c8449fc3fa1d1feaa70c8206890f1e66ba03df.

- 2026-10-12T19:05:01+00:00: Recorded command exit 0; command argv SHA-256
  79a52eb639bef57ef0d1a0cd4ac85312480e7a740d0cf05c4d2b86bca808968e.

- 2026-10-12T19:05:24+00:00: Recorded command exit 0; command argv SHA-256
  d01e719eac3192482e93a11bb38a52a19ed0284f09c2d38583e37124c670f6df.

- 2026-10-12T19:05:51+00:00: Recorded command exit 0; command argv SHA-256
  2fc85ff4ba0c957546a447a3ad48379351192367d6b2b9bcbba0abdd886e3b86.

- 2026-10-12T19:06:21+00:00: Recorded command exit 0; command argv SHA-256
  fbd9aad59a2ed167098b7c94406854ea6029259e337fabcc732e8b0d3d63843a.

- 2026-10-12T19:07:18+00:00: Recorded command exit 0; command argv SHA-256
  cfddfc1b8766f37e6aede838c174f3c1396c0e28ea1eae79d472cf1b97a74f13.

- 2026-10-12T19:07:44+00:00: Recorded command exit 101; command argv SHA-256
  b20912e10e2ccb08a481591e01c8449fc3fa1d1feaa70c8206890f1e66ba03df.

- 2026-10-12T19:08:13+00:00: Recorded command exit 0; command argv SHA-256
  cfddfc1b8766f37e6aede838c174f3c1396c0e28ea1eae79d472cf1b97a74f13.

- 2026-10-12T19:08:42+00:00: Recorded command exit 0; command argv SHA-256
  b20912e10e2ccb08a481591e01c8449fc3fa1d1feaa70c8206890f1e66ba03df.

- 2026-10-12T19:09:09+00:00: Recorded command exit 0; command argv SHA-256
  1024fc1e6cefa31b078f0b5d2ed85c7ea4586897b388bf1e308f62db58574461.

- 2026-10-12T19:12:22+00:00: Recorded command exit 0; command argv SHA-256
  79a52eb639bef57ef0d1a0cd4ac85312480e7a740d0cf05c4d2b86bca808968e.

- 2026-10-12T19:12:46+00:00: Recorded command exit 0; command argv SHA-256
  377f86b49d6e903786e19935c87c98d14040fb7c98391d6f6cc9f22fbed679e9.

- 2026-10-12T19:13:11+00:00: Recorded command exit 0; command argv SHA-256
  2fc85ff4ba0c957546a447a3ad48379351192367d6b2b9bcbba0abdd886e3b86.

- 2026-10-12T19:13:38+00:00: Recorded command exit 0; command argv SHA-256
  1c5969415b805ba066018e832e4962f79079121f724c2f790da39123b56d708a.

- 2026-10-12T19:14:06+00:00: Recorded command exit 101; command argv SHA-256
  560d920083f813f28bcb66d2c40257f9c447fd8e17f1b9941c46f4f65a79d5d2.

- 2026-10-12T19:14:48+00:00: Recorded command exit 0; command argv SHA-256
  cfddfc1b8766f37e6aede838c174f3c1396c0e28ea1eae79d472cf1b97a74f13.

- 2026-10-12T19:15:18+00:00: Recorded command exit 101; command argv SHA-256
  560d920083f813f28bcb66d2c40257f9c447fd8e17f1b9941c46f4f65a79d5d2.

- 2026-10-12T19:16:01+00:00: Recorded command exit 0; command argv SHA-256
  cfddfc1b8766f37e6aede838c174f3c1396c0e28ea1eae79d472cf1b97a74f13.

- 2026-10-12T19:16:30+00:00: Recorded command exit 0; command argv SHA-256
  b20912e10e2ccb08a481591e01c8449fc3fa1d1feaa70c8206890f1e66ba03df.

- 2026-10-12T19:17:00+00:00: Recorded command exit 101; command argv SHA-256
  560d920083f813f28bcb66d2c40257f9c447fd8e17f1b9941c46f4f65a79d5d2.

- 2026-10-12T19:17:30+00:00: Recorded command exit 0; command argv SHA-256
  ef19043205064ea87bec66574bfe246191c9311bc8c404cd8e347d08fbdc44f0.

- 2026-10-12T19:17:54+00:00: Recorded command exit 0; command argv SHA-256
  bf4fee231e93625094e31fc41221e320fd32ab1c8a6ba8bb226124d2cc499454.

- 2026-10-12T19:18:16+00:00: Recorded command exit 0; command argv SHA-256
  34c89054461d25740ddbd2b805c793c12c7c4335b046c0a0534a81fe36ffd086.

- 2026-10-12T19:18:41+00:00: Recorded command exit 0; command argv SHA-256
  2fc85ff4ba0c957546a447a3ad48379351192367d6b2b9bcbba0abdd886e3b86.

- 2026-10-12T19:19:07+00:00: Recorded command exit 0; command argv SHA-256
  6e04ec0e67f67fa78d2538bc86bdb92ae9d9b57b60aab05c3c660a555592e189.

- 2026-10-12T19:19:33+00:00: Review repairs published in signed+DCO head 4e37d542: private staged
  smoke before activation, process-group termination on every terminal path, bounded concurrent
  output drain, rollback/fsync cleanup, source-build cleanup, and focused descendant/race/size
  negatives. Focused adapter (8), diagnostic (9), and workflow (3) tests pass.

- 2026-10-12T19:19:37+00:00: Checkpointed source commit acf439195f13df0c06fe53f2a1924d794d600e56.
