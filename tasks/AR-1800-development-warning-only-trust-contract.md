---
{
  "branch": "feature/ar-1800-development-warning-only-trust-contract",
  "checkpoint_commit": "",
  "claim_expires": "2026-10-12T23:17:04+00:00",
  "depends_on": [
    "AR-1799"
  ],
  "id": "AR-1800",
  "next_action": "Promote and claim AR-1800 after coordinator state is reconciled; inventory every scoped trust validator and define one channel-aware policy before implementation.",
  "observed_branch": "feature/ar-1800-development-warning-only-trust-contract",
  "observed_dirty": 0,
  "observed_head": "961243b302dcb803f426eac507d0f0ce01e2fba0",
  "owner": "ar1800-dev-warning-luna56",
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
  "status": "in_progress",
  "summary": "Make development-channel trust, digest, and exact-version mismatches warning-only without weakening stable/production policy.",
  "task_revision": 8,
  "title": "Development channel warning-only trust contracts",
  "updated_at": "2026-10-12T21:19:35+00:00",
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

- 2026-10-12T21:16:57+00:00: Dependency AR-1799 is durably done at revision 24 with merged PR #613,
  protected-main provenance, installed-entrypoint qualification, and 10/10 exact-head workflows
  green; reconciled implementer role assignment.

- 2026-10-12T21:17:01+00:00: Claimed by ar1800-dev-warning-luna56.

- 2026-10-12T21:17:04+00:00: Heartbeat by ar1800-dev-warning-luna56.

- 2026-10-12T21:17:55+00:00: Recorded command exit 255; command argv SHA-256
  65ee7987d961cc6752211276ff0975933066ca846574d4232ce63ad7c487271d.

- 2026-10-12T21:19:02+00:00: Recorded command exit 0; command argv SHA-256
  5e300cad686da9675076850c3e714464f5f16e34f08f4151abbcd1681fba265e.

- 2026-10-12T21:19:35+00:00: Recorded command exit 101; command argv SHA-256
  7923bbdbe70707e70a2937c3472a129e15ec8876a7299c4ac47a8cbba789053d.
