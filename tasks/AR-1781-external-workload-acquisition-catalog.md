---
{
  "branch": "feature/ar-1781-external-workload-acquisition-catalog",
  "checkpoint_commit": "3827523be8ee914c66368789e2c0dc841743d2de",
  "claim_expires": "2026-10-11T11:46:40+00:00",
  "depends_on": [
    "AR-1760"
  ],
  "id": "AR-1781",
  "next_action": "Continue monitoring exact-head checks for 72abf364; merge only after Rust, AArch64, and policy/supply-chain checks pass, then post-merge verify and accept/release.",
  "observed_branch": "feature/ar-1781-external-workload-acquisition-catalog",
  "observed_dirty": 0,
  "observed_head": "0db5831b7e6d87bf0993fcfd38889abc19595b6f",
  "owner": "ar1781-luna56",
  "plan": "../plans/AR-1781-external-workload-acquisition-catalog.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_acceptance": {
    "evidence_class": "contract-test",
    "evidence_digest": "",
    "evidence_ref": "",
    "spec_ref": "specs/AR-1781.json",
    "spec_revision": 2,
    "status": "pending"
  },
  "spec_ref": "specs/AR-1781.json",
  "spec_revision": 2,
  "status": "in_progress",
  "summary": "Make every documented external AI-agent workload a pinned, development-supported ASB workload, including SWE-mini.",
  "task_revision": 17,
  "title": "External workload acquisition catalog",
  "updated_at": "2026-10-11T09:46:40+00:00",
  "worktree_key": "agent-systems-benchmark-ar-1781-external-workload-acquisition-catalog"
}
---

Create the dedicated authoritative catalog behind `asb workload`. The initial
development-supported set is `agentbench`, `agentdojo`, `agentops`,
`ai-agents-that-matter`, `aider-polyglot`, `bigcodebench`, `core-bench`,
`evalplus`, `exercism-tracks`, `hal`, `harbor`, `helm`, `inspect-ai`,
`livecodebench`, `swe-bench`, `swe-bench-pro`, `swe-fficiency`, `swe-lancer`,
`swe-mini`, `swe-perf`, `swe-rebench`, `tau-bench`, and `terminal-bench`.
Inventory any subsequently documented external suite in the same way and define
its official primary source, immutable dataset/release
identity, digest/signature/provenance checks, license/terms, supported platform,
size/resource requirements, required preparation/runtime dependencies, bounded
normalization recipe, workload adapter entrypoint, and compatibility limits.

An external workload is a data/bundle lifecycle, not a tool executable. Do not
reuse `asb tool` records, accept arbitrary URLs, scrape moving upstream state, or
download datasets merely because they appear in a catalog. In development,
absence of a production release authorization, signature, or hosted verification
must not demote a documented suite to methodology-only or block its catalog entry,
installation, selection, or bounded execution. Preserve pinned source identity
and explicit legal/resource constraints; report genuine license, compatibility,
or source failures distinctly. Production qualification may remain separately
labelled.

- 2026-10-11T09:12:57+00:00: Dependency AR-1760 is done; promote AR-1781 for implementation.

- 2026-10-11T09:13:08+00:00: Claimed by ar1781-luna56.

- 2026-10-11T09:17:53+00:00: Recorded command exit 0; command argv SHA-256
  379d22b10394143332e165d35aef5072d33bcfe42a074b67c37a24320167a0ea.

- 2026-10-11T09:26:53+00:00: Heartbeat by ar1781-luna56.

- 2026-10-11T09:26:59+00:00: Checkpointed source commit fd5868cf4c33c67db01cca18eefa4b8aacc024f1.

- 2026-10-11T09:27:06+00:00: PR #572 exact head 66b0df47; refreshed stale workflow provenance digest
  after Rust CI failure. Focused tests and local gates pass; exact-head CI rerun has
  platform/provenance/header/AWQ/fault/fuzz/credential/Kani/Loom green, remaining required jobs
  pending.

- 2026-10-11T09:31:02+00:00: Heartbeat by ar1781-luna56.

- 2026-10-11T09:31:09+00:00: Checkpointed source commit 3827523be8ee914c66368789e2c0dc841743d2de.

- 2026-10-11T09:31:12+00:00: Rust CI root cause was generated catalog capability-family drift:
  generator omitted agentops, helm, AI Agents That Matter, and swe-mini mappings. Added mappings and
  regenerated catalog; focused parity and transcript tests pass. New signed head 72abf364 pushed to
  PR #572.

- 2026-10-11T09:34:52+00:00: Heartbeat by ar1781-luna56.

- 2026-10-11T09:34:57+00:00: Repair 72abf364 is pushed and exact-head CI has no failures. Green:
  AWQ, fuzz, credential-free, headers, Kani, Loom, TLC/Alloy, retained faults, platform, provenance,
  matcher/SLO, TUI. Pending: Rust, emulated AArch64, policy/coverage/supply-chain.

- 2026-10-11T09:45:10+00:00: Heartbeat by ar1781-luna56.

- 2026-10-11T09:46:40+00:00: Heartbeat by ar1781-luna56.
