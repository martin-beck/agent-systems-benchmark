---
{
  "branch": "feature/ar-1800-development-warning-only-trust-contract",
  "checkpoint_commit": "",
  "claim_expires": "",
  "depends_on": [
    "AR-1799"
  ],
  "id": "AR-1800",
  "next_action": "Release done after exact-main 74b391348fc5c1361034a14742f6afef747d72db verification: PR #615 merged, 13/13 exact-main checks green, focused trust tests and full locked workspace tests passed; perform final state reconciliation.",
  "observed_branch": "feature/ar-1800-development-warning-only-trust-contract",
  "observed_dirty": 0,
  "observed_head": "961243b302dcb803f426eac507d0f0ce01e2fba0",
  "owner": "",
  "plan": "../plans/AR-1800-development-warning-only-trust-contract.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_acceptance": {
    "evidence_class": "contract-test",
    "evidence_digest": "sha256:edd2c248679ef58c1bc6a1010bb60357300637f58385d6770d74f7f40363d129",
    "evidence_ref": "github:martin-beck/agent-systems-benchmark/commit/74b391348fc5c1361034a14742f6afef747d72db/exact-main-workflows",
    "spec_ref": "specs/AR-1800.json",
    "spec_revision": 1,
    "status": "pass"
  },
  "spec_ref": "specs/AR-1800.json",
  "spec_revision": 1,
  "status": "done",
  "summary": "Make development-channel trust, digest, and exact-version mismatches warning-only without weakening stable/production policy.",
  "task_revision": 14,
  "title": "Development channel warning-only trust contracts",
  "updated_at": "2026-10-12T21:25:43+00:00",
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

- 2026-10-12T21:20:25+00:00: Recorded command exit 0; command argv SHA-256
  67d86b07798119c042fae09c962eef82b4680b88ea0440aa15b7c8fcd2a1e6e8.

- 2026-10-12T21:22:19+00:00: Recorded command exit 0; command argv SHA-256
  64a45e97742b783161eb70e48934ba1eb120f83ea3b9bf09d7a31e5efcf7c247.

- 2026-10-12T21:23:19+00:00: Recorded command exit 0; command argv SHA-256
  5345cd4b5f66285dc80cddfe59c0a95475263f9ff88d25d2639fce3c300aa4b9.

- 2026-10-12T21:24:22+00:00: Focused trust tests passed 3/3. Full cargo test --locked --workspace
  passed all executed tests and doc tests; wrapper post-reconcile bookkeeping initially failed only
  on generated handoffctl bytecode size guard, with no product failure.

- 2026-10-12T21:25:21+00:00: Acceptance evidence: PR #615 merged reviewed head
  961243b302dcb803f426eac507d0f0ce01e2fba0; protected main 74b391348fc5c1361034a14742f6afef747d72db
  exact-main checks 13/13 green; focused trust tests 3/3 and full locked workspace tests/doc-tests
  passed; dev-only warning scope and fail-closed stable/production/structural boundaries verified.

- 2026-10-12T21:25:43+00:00: Completed on protected main 74b391348fc5c1361034a14742f6afef747d72db.
  PR #615 merged reviewed head 961243b302dcb803f426eac507d0f0ce01e2fba0; exact-main checks 13/13
  green, focused trust tests 3/3, full locked workspace tests/doc-tests passed; dev-only warning
  scope and fail-closed stable/production/structural boundaries verified; AR-1799 installed
  qualification consumed.
