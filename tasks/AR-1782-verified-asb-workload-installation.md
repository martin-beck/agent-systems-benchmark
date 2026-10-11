---
{
  "branch": "feature/ar-1782-verified-asb-workload-installation",
  "checkpoint_commit": "",
  "claim_expires": "2026-10-11T16:40:52+00:00",
  "depends_on": [
    "AR-1781"
  ],
  "id": "AR-1782",
  "next_action": "PR #584 updated to signed+DCO head 16c9bbd9; diagnostic identity and 379 CLI lib tests pass locally. Monitor fresh exact-head CI, independently review, then merge only after all required checks pass.",
  "observed_branch": "feature/ar-1782-verified-asb-workload-installation",
  "observed_dirty": 1,
  "observed_head": "a1090297dede4412bcf1487d2d7bc00315c69dc5",
  "owner": "ar1782_workload_install_luna56",
  "plan": "../plans/AR-1782-verified-asb-workload-installation.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_acceptance": {
    "evidence_class": "contract-test",
    "evidence_digest": "",
    "evidence_ref": "",
    "spec_ref": "specs/AR-1782.json",
    "spec_revision": 2,
    "status": "pending"
  },
  "spec_ref": "specs/AR-1782.json",
  "spec_revision": 2,
  "status": "in_progress",
  "summary": "Implement dedicated verified remote external workload installation and lifecycle through asb workload.",
  "task_revision": 41,
  "title": "Verified asb workload installation",
  "updated_at": "2026-10-11T14:42:02+00:00",
  "worktree_key": "agent-systems-benchmark-ar-1782-verified-asb-workload-installation"
}
---

Add `asb workload install <supported-id> [--project PATH]` and `list`, `status`,
`select`, `repair`, `update`, and `remove` lifecycle commands. Install resolves only the
AR-1781 catalog, downloads the official primary-source workload release into a
project-local workload root, verifies immutable identity/proof/license/size and
archive boundaries, performs only declared bounded preparation, validates the
adapter-ready structure, and atomically records a workload receipt.

The command must support all AR-1781 development workload IDs, including
SWE-mini, not merely list them. It must preserve a good prior workload on failed update,
never execute downloaded installer scripts, require root, or mutate global package
manager state. Read-only operations are network-free. Download, integrity,
license, preparation, storage, incompatibility, stale/revoked, and offline cases
must remain separate actionable outcomes. Development operation must not be
blocked by production authorization or hosted-release verification requirements;
the pinned primary-source identity, declared constraints, and bounded preparation
remain mandatory.

- 2026-10-11T13:53:25+00:00: Promote dependency-safe AR-1782 now that AR-1781 is durably done with
  exact-main evidence.

- 2026-10-11T13:53:59+00:00: Claimed by ar1782_workload_install_luna56.

- 2026-10-11T13:54:05+00:00: Recorded command exit 0; command argv SHA-256
  ff6d1fa7a7185010f75fcfafe4bdc1bdd5b9804d8c2b3e7abdd8a1b96cd81c63.

- 2026-10-11T14:00:48+00:00: Heartbeat by ar1782_workload_install_luna56.

- 2026-10-11T14:04:41+00:00: Recorded command exit 0; command argv SHA-256
  e274074a17e0682b5edf79b5caca577a41520ed9ca8daf9342f0433b678582a1.

- 2026-10-11T14:05:07+00:00: Recorded command exit 0; command argv SHA-256
  acc9cf8127a84779f740f6350aa3a1616ba38c8dc1c9797006e2886289fc16e0.

- 2026-10-11T14:05:20+00:00: Recorded command exit 0; command argv SHA-256
  e6e449e9d1f470db7ec6f673f670b5e31f0066cbbd41fd7b2de98c8bdbed15c6.

- 2026-10-11T14:05:31+00:00: Recorded command exit 0; command argv SHA-256
  afb0c01c2d49638af443d2502727e1b5ac6bfd221160d69ddb93f495dd751142.

- 2026-10-11T14:05:54+00:00: Recorded command exit 8; command argv SHA-256
  78f325bde3962aea10f3844ad0018af88b2cf3e001f1e02858b67193c031b6ae.

- 2026-10-11T14:06:26+00:00: PR #584 is open at signed+DCO product head a1090297. Exact-head CI is
  running; header/provenance/AWQ shadow checks pass, remaining required workflows pending.

- 2026-10-11T14:06:30+00:00: Heartbeat by ar1782_workload_install_luna56.

- 2026-10-11T14:06:58+00:00: Recorded command exit 8; command argv SHA-256
  78f325bde3962aea10f3844ad0018af88b2cf3e001f1e02858b67193c031b6ae.

- 2026-10-11T14:07:43+00:00: Recorded command exit 8; command argv SHA-256
  78f325bde3962aea10f3844ad0018af88b2cf3e001f1e02858b67193c031b6ae.

- 2026-10-11T14:08:05+00:00: Recorded command exit 8; command argv SHA-256
  78f325bde3962aea10f3844ad0018af88b2cf3e001f1e02858b67193c031b6ae.

- 2026-10-11T14:08:46+00:00: Recorded command exit 8; command argv SHA-256
  78f325bde3962aea10f3844ad0018af88b2cf3e001f1e02858b67193c031b6ae.

- 2026-10-11T14:08:49+00:00: Recorded command exit 8; command argv SHA-256
  78f325bde3962aea10f3844ad0018af88b2cf3e001f1e02858b67193c031b6ae.

- 2026-10-11T14:09:50+00:00: Recorded command exit 8; command argv SHA-256
  78f325bde3962aea10f3844ad0018af88b2cf3e001f1e02858b67193c031b6ae.

- 2026-10-11T14:10:45+00:00: Recorded command exit 1; command argv SHA-256
  78f325bde3962aea10f3844ad0018af88b2cf3e001f1e02858b67193c031b6ae.

- 2026-10-11T14:11:09+00:00: Recorded command exit 0; command argv SHA-256
  e8fa5ae4c6687147b213085b0419c35165daa77f6f62165ff84de0a0ca2e56d4.

- 2026-10-11T14:12:41+00:00: Rust checks failed only at diagnostic_contract ordinary producer
  identity; local generated catalog repair passes focused test. Do not push overlapping repair
  before AR-1793 merge.

- 2026-10-11T14:12:45+00:00: Heartbeat by ar1782_workload_install_luna56.

- 2026-10-11T14:31:22+00:00: Recorded command exit 0; command argv SHA-256
  a19f9863e49c2e0fe53777cca6eb65ba80003ffaa6ac29ced611d1a7d816da3e.

- 2026-10-11T14:31:39+00:00: Recorded command exit 0; command argv SHA-256
  b7c438686581cf292b6cd41497ceabb3af42dfed57e04dfd6cc89efed8e2d15b.

- 2026-10-11T14:31:50+00:00: Recorded command exit 0; command argv SHA-256
  6b0340b78e00b5e8d62e9973b7e72f6c6de22448e8636894d4001797a91460ec.

- 2026-10-11T14:32:09+00:00: Recorded command exit 0; command argv SHA-256
  e7bd0253a3301d42ef2d00a6221164345479a524c2349ff67e6eca5bfbd24951.

- 2026-10-11T14:32:38+00:00: Recorded command exit 0; command argv SHA-256
  04c7d6439ff8aae8c18a38395591b0d548ab41ade5cfeaa27267e6ac20bbb736.

- 2026-10-11T14:33:06+00:00: Recorded command exit 0; command argv SHA-256
  ac3e505f30fd620a7c294d45f78bcf244146c7ac1be401dd8c78fa01c13f8abb.

- 2026-10-11T14:33:35+00:00: Recorded command exit 8; command argv SHA-256
  78f325bde3962aea10f3844ad0018af88b2cf3e001f1e02858b67193c031b6ae.

- 2026-10-11T14:34:05+00:00: Rebased after AR-1793 merge and pushed generated diagnostic identity
  repair 0c31a205. Fresh PR checks are running; headers/provenance/AWQ shadow pass, remaining checks
  pending.

- 2026-10-11T14:34:47+00:00: Recorded command exit 8; command argv SHA-256
  78f325bde3962aea10f3844ad0018af88b2cf3e001f1e02858b67193c031b6ae.

- 2026-10-11T14:39:20+00:00: Recorded command exit 0; command argv SHA-256
  e7bd0253a3301d42ef2d00a6221164345479a524c2349ff67e6eca5bfbd24951.

- 2026-10-11T14:39:45+00:00: Recorded command exit 0; command argv SHA-256
  2c9d442f017d15b4fd94d96190844d436ce52d408beaa15376675a764087bb7e.

- 2026-10-11T14:40:13+00:00: Recorded command exit 0; command argv SHA-256
  ac3e505f30fd620a7c294d45f78bcf244146c7ac1be401dd8c78fa01c13f8abb.

- 2026-10-11T14:40:45+00:00: Corrected legacy_cli_ar1782_4239 collision: tool_kind callsite is
  reviewed as legacy_cli_ar1777_0028 with MalformedInput; removed duplicate workload identity.
  Focused diagnostic tests and all 379 asb-cli lib tests pass.

- 2026-10-11T14:40:52+00:00: Heartbeat by ar1782_workload_install_luna56.

- 2026-10-11T14:41:00+00:00: Recorded command exit 8; command argv SHA-256
  78f325bde3962aea10f3844ad0018af88b2cf3e001f1e02858b67193c031b6ae.

- 2026-10-11T14:42:02+00:00: Recorded command exit 8; command argv SHA-256
  78f325bde3962aea10f3844ad0018af88b2cf3e001f1e02858b67193c031b6ae.
