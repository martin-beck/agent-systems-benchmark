---
{
  "branch": "feature/ar-1784-command-dependency-registry-and-preflight",
  "checkpoint_commit": "",
  "claim_expires": "2026-10-11T10:10:16+00:00",
  "depends_on": [
    "AR-1776"
  ],
  "id": "AR-1784",
  "next_action": "Hold AR-1784 publication pending dependent AR-1793; focused diagnostic_contract now passes after preserving lib.rs callsite lines and cataloguing preflight legacy identity; rerun unchanged full asb-cli suite, then review and publish.",
  "observed_branch": "feature/ar-1784-command-dependency-registry-and-preflight",
  "observed_dirty": 3,
  "observed_head": "cd8473f925cb3e785f0d675d3c5f83b42acdf48c",
  "owner": "ar1784_dependency_registry_luna56",
  "plan": "../plans/AR-1784-command-dependency-registry-and-preflight.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_acceptance": {
    "evidence_class": "contract-test",
    "evidence_digest": "",
    "evidence_ref": "",
    "spec_ref": "specs/AR-1784.json",
    "spec_revision": 2,
    "status": "pending"
  },
  "spec_ref": "specs/AR-1784.json",
  "spec_revision": 2,
  "status": "in_progress",
  "summary": "Make every ASB command dependency declared, installable or explicitly host-only, and fail-fast before a missing-tool runtime failure.",
  "task_revision": 92,
  "title": "Command dependency registry and preflight",
  "updated_at": "2026-10-11T08:51:39+00:00",
  "worktree_key": "agent-systems-benchmark-ar-1784-command-dependency-registry-and-preflight"
}
---

Create a generated, typed registry that maps every public ASB command, adapter,
monitor, runtime path, and source-build recipe to the exact executable tools,
workload bundles, libraries, and host capabilities it needs. A dependency is
either an `asb tool install` supported ID, an `asb workload install` supported
ID, a catalog-pinned project build dependency, or an explicitly host-only
capability with a concrete platform remediation. No command may invoke or assume
an undeclared executable.

The initial required executable IDs are: `aider`, `codex`, `gemini`, `goose`,
`mini-swe`, `opencode`, `opendesk`, `openhands`, `openjiuwen`, `qwen-code`,
development-only `cli2key`, optional `perf`, and optional `bpftool`. Provider
APIs are not installable tools. Core procfs/cgroup collection needs no external
binary. Linux/cgroup-v2/user-systemd/Bubblewrap and kernel permission/BTF/PMU
conditions are host capabilities, not pretend downloads; preflight must report
them before execution with the owner/action needed to repair them.

Expose an `asb` preflight/status view that reports readiness, missing supported
IDs with exact install commands, missing project-local dependencies, and
uninstallable host-only conditions before a run starts. Generate coverage from
authoritative dispatch/adapter/build/workload declarations, so adding future
tools or workloads requires catalog, command-dependency, preflight, selection,
and end-to-end development-run tests in the same change.

- 2026-10-11T08:10:08+00:00: AR-1776 is complete and its tool catalog is available; promote
  dependency-ready AR-1784 for implementation.

- 2026-10-11T08:10:11+00:00: Claimed by ar1784_dependency_registry_luna56.

- 2026-10-11T08:10:16+00:00: Heartbeat by ar1784_dependency_registry_luna56.
- 2026-10-11: Root cause repaired within AR-1784: baseline origin/main 4245b4a1 passed the human diagnostic assertion; AR-1784 initially failed because preflight additions shifted line-addressed legacy callsites. Preserved lib.rs callsite lines and catalogued preflight identity at line 1010. Added AR-1793 as a narrowly scoped dependent repair record for the legacy output-parent wording contract; state files committed and pushed. Focused diagnostic_contract 9/9, diagnostic_journey 1/1, human_cli 15/15, workflow_transcript 3/3, and unchanged cargo test --locked -p asb-cli (349 unit plus all integration/doc tests) are green at signed product head cd8473f925cb3e785f0d675d3c5f83b42acdf48c. cargo fmt --check reports rustfmt expansion of the dispatch arm, which would shift the repository legacy line-address contract; publication remains pending parent review of this gate constraint.

- 2026-10-11T08:10:39+00:00: Recorded command exit 0; command argv SHA-256
  431771e25c6fcb63cee2c212739b3c89638fc0e4eea3b496d3f3b0603f464748.

- 2026-10-11T08:11:05+00:00: Recorded command exit 0; command argv SHA-256
  a80c3db7ed242b537b65e8f3b4a3ddb2554300ce2b1a5e4f8d3ac0651f3d0869.

- 2026-10-11T08:12:21+00:00: Recorded command exit 0; command argv SHA-256
  c7323847342774673319b7ffd857e626b77892358d868ecb570bff056d311898.

- 2026-10-11T08:12:46+00:00: Recorded command exit 0; command argv SHA-256
  ebd2395a6f3af72a541d6b9dd8ffecb9ec9085e6f64d76d108293c9c622d6a56.

- 2026-10-11T08:13:14+00:00: Recorded command exit 101; command argv SHA-256
  0e8b8ec438821ebf3b5b185af8adff8c05c54025dba46010f1057a05ba1d6d2b.

- 2026-10-11T08:13:41+00:00: Recorded command exit 0; command argv SHA-256
  c7323847342774673319b7ffd857e626b77892358d868ecb570bff056d311898.

- 2026-10-11T08:14:06+00:00: Recorded command exit 0; command argv SHA-256
  a4eb05d6d82c911062a7e91d765c0bcdc391fb7aa6781e391d86d09d8ae4a11b.

- 2026-10-11T08:14:33+00:00: Recorded command exit 0; command argv SHA-256
  47bcc84d3b9fdd5b3bb1e5ae460b912d8bf60595e58088926ab03e366e708695.

- 2026-10-11T08:14:49+00:00: Recorded command exit 101; command argv SHA-256
  0e8b8ec438821ebf3b5b185af8adff8c05c54025dba46010f1057a05ba1d6d2b.

- 2026-10-11T08:15:24+00:00: Recorded command exit 0; command argv SHA-256
  c7323847342774673319b7ffd857e626b77892358d868ecb570bff056d311898.

- 2026-10-11T08:15:54+00:00: Recorded command exit 101; command argv SHA-256
  0e8b8ec438821ebf3b5b185af8adff8c05c54025dba46010f1057a05ba1d6d2b.

- 2026-10-11T08:16:37+00:00: Root cause of repeated exit 101: preflight was added as an
  InvocationKind while an accidental stale CommandKind::Preflight arm remained after family mapping
  to Doctor. Remove stale arm and rerun cargo check. Wrapper post-reconcile also repeatedly saw
  disposable handoffctl pycache size guard; preserve and quarantine generated bytecode.

- 2026-10-11T08:16:46+00:00: Recorded command exit 101; command argv SHA-256
  0e8b8ec438821ebf3b5b185af8adff8c05c54025dba46010f1057a05ba1d6d2b.

- 2026-10-11T08:17:18+00:00: Recorded command exit 0; command argv SHA-256
  0e8b8ec438821ebf3b5b185af8adff8c05c54025dba46010f1057a05ba1d6d2b.

- 2026-10-11T08:18:03+00:00: Recorded command exit 0; command argv SHA-256
  82b893c0185fef6d76635523b4de3b7a2baadcdf82314f52f1db386a60d2503d.

- 2026-10-11T08:18:38+00:00: Recorded command exit 0; command argv SHA-256
  eadb1624311d49cd479c401aed6471f8f6a4eeac5e00ffdf4dc1da70c5b0b256.

- 2026-10-11T08:19:21+00:00: Recorded command exit 0; command argv SHA-256
  c7323847342774673319b7ffd857e626b77892358d868ecb570bff056d311898.

- 2026-10-11T08:20:25+00:00: Recorded command exit 0; command argv SHA-256
  2bd70e0c26aa986612d3eee65d9d3a6c101e21a93620f69f0b2353ef488adf00.

- 2026-10-11T08:21:15+00:00: Recorded command exit 0; command argv SHA-256
  c50160c33f34ec7955196138b6f40b52c9c3006f8928e00c37e1ca66932c1fee.

- 2026-10-11T08:21:51+00:00: Recorded command exit 101; command argv SHA-256
  770ef66e05618edaf585abd6d89f8f4b9cea6dae022116745e9147ef57d01a1d.

- 2026-10-11T08:23:11+00:00: Complete asb-cli suite exposed three concrete failures: optional
  perf/bpftool host-tool remediation was incorrectly asserted as ASB-installable (test fixed to
  require install syntax only for mandatory tools); public command inventory lacked its preflight
  golden fixture (fixture row added); one existing human error-presentation assertion was
  environment/baseline-sensitive and showed output-parent wording, to be rechecked independently.

- 2026-10-11T08:23:33+00:00: Recorded command exit 1; command argv SHA-256
  6e451b47efa59de3b300c2fe3ff0278cab5c6d099c43967c15fb3b77515626c2.

- 2026-10-11T08:23:58+00:00: Recorded command exit 0; command argv SHA-256
  82b893c0185fef6d76635523b4de3b7a2baadcdf82314f52f1db386a60d2503d.

- 2026-10-11T08:24:47+00:00: Recorded command exit 101; command argv SHA-256
  634ca9b663d7d070b704cdbbf6ef7bfaccf7ae3bff834259c9289b273eaac204.

- 2026-10-11T08:25:36+00:00: Recorded command exit 0; command argv SHA-256
  634ca9b663d7d070b704cdbbf6ef7bfaccf7ae3bff834259c9289b273eaac204.

- 2026-10-11T08:26:21+00:00: Recorded command exit 0; command argv SHA-256
  71438883e0e7f3c41f793a3d738713da6206360a4d1273aa78f4b4aaa64205e4.

- 2026-10-11T08:27:10+00:00: Recorded command exit 0; command argv SHA-256
  fb41037a684c4eacabd62eb60a0ae25a18a6247d6c01ca1a8338a3de1e54b7d4.

- 2026-10-11T08:27:39+00:00: Recorded command exit 101; command argv SHA-256
  770ef66e05618edaf585abd6d89f8f4b9cea6dae022116745e9147ef57d01a1d.

- 2026-10-11T08:27:57+00:00: Full asb-cli rerun is deterministic for AR-1784 changes: 348/349
  passed; only existing
  human::tests::error_presentations_name_cause_state_and_recovery_without_private_text fails on
  legacy output-parent-unavailable wording (expected state-change phrase absent; output says
  requested input/configuration needs correction and no successful state change). New
  dependency_registry tests all pass, and preflight public-command golden/inventory test passes. Do
  not mask or weaken unrelated diagnostic gate.

- 2026-10-11T08:29:01+00:00: Recorded command exit 0; command argv SHA-256
  c7323847342774673319b7ffd857e626b77892358d868ecb570bff056d311898.

- 2026-10-11T08:29:50+00:00: Recorded command exit 0; command argv SHA-256
  18e0772891505bdf727e478f07b0d434d7a3304b45e32bf6a42bfb70db96970c.

- 2026-10-11T08:30:51+00:00: Recorded command exit 0; command argv SHA-256
  e758278b0f11127364562bc4f94c69d11d6e2eaf9ef0b1a63673a68b98526712.

- 2026-10-11T08:31:36+00:00: Recorded command exit 101; command argv SHA-256
  062915ecc2fffd497e5e68f1bb83aa0b573c6fcfcd601ed460f67b18512cebd3.

- 2026-10-11T08:33:01+00:00: Recorded command exit 0; command argv SHA-256
  062915ecc2fffd497e5e68f1bb83aa0b573c6fcfcd601ed460f67b18512cebd3.

- 2026-10-11T08:33:44+00:00: Recorded command exit 101; command argv SHA-256
  770ef66e05618edaf585abd6d89f8f4b9cea6dae022116745e9147ef57d01a1d.

- 2026-10-11T08:34:46+00:00: Recorded command exit 101; command argv SHA-256
  03608c6f5404b9e2bddb6f52c4961ebff2485abad3f6111cf94ddaacdd8eb253.

- 2026-10-11T08:35:29+00:00: Recorded command exit 101; command argv SHA-256
  03608c6f5404b9e2bddb6f52c4961ebff2485abad3f6111cf94ddaacdd8eb253.

- 2026-10-11T08:36:38+00:00: Recorded command exit 0; command argv SHA-256
  834ef9f25c98cb1a5dae5cea38df6c6b1e9d3058f2527317b78cf43dab77285b.

- 2026-10-11T08:37:23+00:00: Recorded command exit 101; command argv SHA-256
  d32df0a8f7802e39b9a6743613fff16ac35e4dddba3905aa783a204f77a7e43f.

- 2026-10-11T08:38:05+00:00: Recorded command exit 1; command argv SHA-256
  d660b0fb64fef62393e7a84cb53775db6717456c306f17274d5515dcefb374c7.

- 2026-10-11T08:38:32+00:00: Recorded command exit 1; command argv SHA-256
  c02539b5dfd525ddfd6143b1d8d4805ec7fc3e0bddfce82fab51205a2cf5ebd2.

- 2026-10-11T08:39:43+00:00: Recorded command exit 1; command argv SHA-256
  5b149f47d61602eeb2018f8c0e8aefc6fa01d74931a9208c736824fbd94fc380.

- 2026-10-11T08:40:30+00:00: Recorded command exit 0; command argv SHA-256
  7010be0736176d7c12cbd14500a5bf7b25c93fde74effaf7b3b9e6b48424ef2d.

- 2026-10-11T08:41:22+00:00: Recorded command exit 101; command argv SHA-256
  d32df0a8f7802e39b9a6743613fff16ac35e4dddba3905aa783a204f77a7e43f.

- 2026-10-11T08:41:50+00:00: Recorded command exit 0; command argv SHA-256
  f0051149a3885290815715551b3a5bad0d64bfe66de4df261620e6cb11f3313b.

- 2026-10-11T08:42:14+00:00: Recorded command exit 0; command argv SHA-256
  d32df0a8f7802e39b9a6743613fff16ac35e4dddba3905aa783a204f77a7e43f.

- 2026-10-11T08:43:02+00:00: Recorded command exit 0; command argv SHA-256
  99c52dc6d47b48d6e6c78e35a1869d9659404f2ee8896b143f564c59bc7fa63c.

- 2026-10-11T08:43:16+00:00: Recorded command exit 0; command argv SHA-256
  4b9e2292b4d393fe889c2a658ad974dc27afb1a1132e5229c3a17c9ac8211eaf.

- 2026-10-11T08:43:27+00:00: Recorded command exit 0; command argv SHA-256
  a8d82491b2ea53fb650374f73d7bf4d46e007dd7c278e9b7ad2240a244a03d77.

- 2026-10-11T08:43:39+00:00: Recorded command exit 0; command argv SHA-256
  b26915d9a59ecd48b63a5b2ead4566d46def4559b2ec965d6146fbc0392fd8c3.

- 2026-10-11T08:43:52+00:00: Root cause recorded: origin/main baseline 4245b4a1 passed
  human::tests::error_presentations_name_cause_state_and_recovery_without_private_text; AR-1784
  initially failed because added preflight dispatch/help/module lines shifted line-addressed legacy
  diagnostic callsites, causing output-parent handling to emit No successful state change was
  reported. Repair was within AR-1784: preserve lib.rs line positions and add reviewed catalog
  identity at lib.rs:1010. AR-1793 is created and committed/pushed as narrowly scoped dependent
  repair for the legacy wording contract, with AR-1784 dependency.

- 2026-10-11T08:43:58+00:00: Recorded command exit 0; command argv SHA-256
  b26915d9a59ecd48b63a5b2ead4566d46def4559b2ec965d6146fbc0392fd8c3.

- 2026-10-11T08:44:24+00:00: Recorded command exit 101; command argv SHA-256
  22fc19d872be75a975891ff0abd53f1c305974a409b50c49c5df5d842d033e81.

- 2026-10-11T08:44:48+00:00: Recorded command exit 0; command argv SHA-256
  3307cf1ec5d15347b8ed30462a6d176708a4383f7030eb915206e600392f51b5.

- 2026-10-11T08:44:59+00:00: Recorded command exit 0; command argv SHA-256
  902ca06d6e8795ad99e118114b4b60d5ea3e7c80b78f387a808ba2842c648db7.

- 2026-10-11T08:45:20+00:00: Recorded command exit 101; command argv SHA-256
  22fc19d872be75a975891ff0abd53f1c305974a409b50c49c5df5d842d033e81.

- 2026-10-11T08:45:42+00:00: Recorded command exit 0; command argv SHA-256
  58e3df6a4c9d75cbdbf86aa6a80426aed2162cebc25cbe9189f3421f618cd553.

- 2026-10-11T08:45:53+00:00: Recorded command exit 0; command argv SHA-256
  b6e6e6746812ef4c262370c299c1a905f916a939d0f5fcf04c08e219f6e3a2d0.

- 2026-10-11T08:46:48+00:00: Recorded command exit 0; command argv SHA-256
  4cd658c0d8a353a8f491dc52b351822978f8283a88a9f6b7a57440bbd15e86f8.

- 2026-10-11T08:47:01+00:00: Recorded command exit 0; command argv SHA-256
  ef19043205064ea87bec66574bfe246191c9311bc8c404cd8e347d08fbdc44f0.

- 2026-10-11T08:47:32+00:00: Recorded command exit 0; command argv SHA-256
  22fc19d872be75a975891ff0abd53f1c305974a409b50c49c5df5d842d033e81.

- 2026-10-11T08:47:46+00:00: Recorded command exit 0; command argv SHA-256
  5eecc0425b7abdff8930cdbbf8cf07eb2a238195ff4c5add03be4f2de84e678b.

- 2026-10-11T08:47:59+00:00: Recorded command exit 1; command argv SHA-256
  f7e20666638201d8567703a3d7bc028e04a5e8e4c6c509de13d4976b0c91a669.

- 2026-10-11T08:48:24+00:00: Recorded command exit 0; command argv SHA-256
  8f69a98bff5dc433ce76528a435169940f784109f3c631d40b08c25d78c6cc5e.

- 2026-10-11T08:48:36+00:00: Recorded command exit 0; command argv SHA-256
  b26915d9a59ecd48b63a5b2ead4566d46def4559b2ec965d6146fbc0392fd8c3.

- 2026-10-11T08:49:09+00:00: Recorded command exit 0; command argv SHA-256
  cfddfc1b8766f37e6aede838c174f3c1396c0e28ea1eae79d472cf1b97a74f13.

- 2026-10-11T08:49:23+00:00: Recorded command exit 1; command argv SHA-256
  d68fef343d6f110ed0d90227fec66d3ac5b06f4b8a5c446d59f632450c651061.

- 2026-10-11T08:49:50+00:00: Recorded command exit 1; command argv SHA-256
  32333167a669fc242187110573f967f956f36a4dc16c705eab60e57c93809e54.

- 2026-10-11T08:50:14+00:00: Recorded command exit 0; command argv SHA-256
  22e0a1a8ddd12fb624cd0426534404f5a8ac6b56b6af322d5d4c575a6007b2a9.

- 2026-10-11T08:50:25+00:00: Recorded command exit 0; command argv SHA-256
  f7e20666638201d8567703a3d7bc028e04a5e8e4c6c509de13d4976b0c91a669.

- 2026-10-11T08:50:38+00:00: Recorded command exit 101; command argv SHA-256
  d32df0a8f7802e39b9a6743613fff16ac35e4dddba3905aa783a204f77a7e43f.

- 2026-10-11T08:51:39+00:00: Recorded command exit 0; command argv SHA-256
  9a4a5ff9e6e90890a02666a7bdf5082232d9cd63be47aa35646d577da5cec150.
