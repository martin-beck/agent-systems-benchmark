---
{
  "branch": "feature/ar-1800-development-warning-only-trust-contract",
  "checkpoint_commit": "",
  "claim_expires": "",
  "depends_on": [
    "AR-1799"
  ],
  "id": "AR-1800",
  "next_action": "Promote and claim AR-1800 after coordinator state is reconciled; inventory every scoped trust validator and define one channel-aware policy before implementation.",
  "owner": "",
  "plan": "../plans/AR-1800-development-warning-only-trust-contract.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_acceptance": {
    "evidence_class": "contract-test",
    "evidence_digest": "",
    "evidence_ref": "",
    "spec_ref": "specs/AR-1800.json",
    "spec_revision": 1,
    "status": "pending"
  },
  "spec_ref": "specs/AR-1800.json",
  "spec_revision": 1,
  "status": "planned",
  "summary": "Make development-channel trust, digest, and exact-version mismatches warning-only without weakening stable/production policy.",
  "task_revision": 1,
  "title": "Development channel warning-only trust contracts",
  "updated_at": "2026-10-12T00:00:00+00:00",
  "worktree_key": "agent-systems-benchmark-ar-1800-development-warning-only-trust-contract"
}
---

ASB currently blocks development work when a trusted-tool ownership or mode check,
SHA-256 comparison, exact tool/version match, or `asb tui install` development
trust-contract check disagrees with the observed machine. This P0 AR introduces a
single channel-aware policy for those findings. In the `dev` channel, each scoped
mismatch must be emitted as a typed warning at warning log level (and above when
the selected log level permits it), retained in human output and `--json`
diagnostics, and must not fail the command solely because the mismatch occurred.
The command and TUI install path must continue with best-effort execution while
clearly stating that trust or identity was not verified.

The change is deliberately limited to development-channel trust/identity
findings. Stable and production channels retain their strict fail-closed
contracts. Malformed CLI input, unsafe command construction or path traversal,
secret exposure, impossible filesystem/process operations, and other structural
safety failures remain non-bypassable; this AR must not turn those into trusted
execution. No credential, API key, raw token, or sensitive host path may appear
in warnings, JSON, logs, or evidence.
