---
{
  "branch": "repair/ar-1799-tui-install-rustup-invalid",
  "checkpoint_commit": "",
  "claim_expires": "2026-10-12T19:32:11+00:00",
  "depends_on": [
    "AR-1738",
    "AR-1636"
  ],
  "id": "AR-1799",
  "next_action": "PR #613 (63ba8e7973545f65bd8dda56f3aa24d9d339bb16) is open; focused rustup tests, fmt, clippy, and independent review pass. Await remaining exact-head hosted checks, then merge and run post-merge installed-entrypoint qualification.",
  "observed_branch": "repair/ar-1799-tui-install-rustup-invalid",
  "observed_dirty": 0,
  "observed_head": "e058ea2b5881e047c2a02a5bc5bd4f23103c28c0",
  "owner": "codex-ar1799-tui-install-rustup-invalid-20261012",
  "plan": "../plans/AR-1799-tui-install-rustup-invalid-repair.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_acceptance": {
    "evidence_class": "environmental",
    "evidence_digest": "",
    "evidence_ref": "",
    "spec_ref": "specs/AR-1799.json",
    "spec_revision": 1,
    "status": "pending"
  },
  "spec_ref": "specs/AR-1799.json",
  "spec_revision": 1,
  "status": "in_progress",
  "summary": "Repair the remaining current-main asb tui install trusted_tool_invalid failure for development rustup/rustc layouts and qualify the real installed entrypoint.",
  "task_revision": 22,
  "title": "Repair asb tui install rustup/rustc trusted-tool rejection",
  "updated_at": "2026-10-12T17:35:56+00:00",
  "worktree_key": "agent-systems-benchmark-ar-1799-tui-install-rustup-invalid"
}
---

The merged permission compatibility repairs are not sufficient if the actual
`asb tui install` entrypoint still returns `trusted_tool_invalid`. Reproduce
that exact failure against current protected ASB main using a disposable HOME,
the conventional rustup shim/toolchain layout, and the affected permission or
ownership variants. Trace the typed error to its real validator, repair the
development-only path without weakening stable/production trust boundaries, and
make permission/ownership findings warning-only as already specified for
development. Preserve hard failures for path traversal, symlink substitution,
malformed settings, missing tools, and toolchain pairing races.

The acceptance target is the installed-entrypoint journey (`asb tui install`,
then status/launch or the documented no-provider qualification), not only an
isolated preflight helper. No API key, signature, or key-management artifact
may block development qualification or enter durable state.

- 2026-10-12T13:23:02+00:00: Open P0 repair for the reproduced user-visible asb tui install
  trusted_tool_invalid failure.

- 2026-10-12T13:23:22+00:00: Claimed by codex-ar1799-tui-install-rustup-invalid-20261012.

- 2026-10-12T13:37:36+00:00: Reproduced the real bound-rustc permission gap, implemented PR #613,
  and recorded focused/local review evidence.

- 2026-10-12T16:51:28+00:00: Released expired stale claim after verified owner process absence;
  preserved PR #613 evidence and exact next action. recover-expired was unavailable because no
  exact-current session snapshot existed.

- 2026-10-12T17:29:19+00:00: Claimed by codex-ar1799-tui-install-rustup-invalid-20261012.

- 2026-10-12T17:31:31+00:00: Recorded command exit 0; command argv SHA-256
  b5bea41b6c623f7c09f1bf24dcae58ebab3c0cdd90ad966bc43a45b44867e12b.

- 2026-10-12T17:31:50+00:00: Recorded command exit 101; command argv SHA-256
  882e1b66b36d6a284e4aab312dcbb48e6e97ff7055bfd2b8931406634fc6cc87.

- 2026-10-12T17:32:11+00:00: Heartbeat by codex-ar1799-tui-install-rustup-invalid-20261012.

- 2026-10-12T17:32:57+00:00: Recorded command exit 0; command argv SHA-256
  42614cc7215ab8b366935e3cc2aaf36f8cd263440b0f802ebab443e48d28ae7f.

- 2026-10-12T17:33:15+00:00: Recorded command exit 0; command argv SHA-256
  449faa451c07a0d8de43d97c05952c952a06cfb360968a06f6a7ccda2de5d28d.

- 2026-10-12T17:33:42+00:00: Recorded command exit 3; command argv SHA-256
  3a6b3c590a56b2869a913cab3cbb8992681eccd7a43b864d7df2984845e849dd.

- 2026-10-12T17:33:53+00:00: Recorded command exit 0; command argv SHA-256
  0c4c606f4152c51780fa55e87463aaa155dc4e6fd7f1e8a0473663dc86dd4d6c.

- 2026-10-12T17:34:04+00:00: Recorded command exit 0; command argv SHA-256
  213361578c6609141b7774a901311fffd62c120885225d33e772f699b2729f4c.

- 2026-10-12T17:34:13+00:00: Recorded command exit 0; command argv SHA-256
  c71d32cf82c4ea9e346db9a51de2ba5ab935ea341a5e8d3413c05eea568c0835.

- 2026-10-12T17:34:57+00:00: Recorded command exit 0; command argv SHA-256
  f5df66b5df9f492ab64ac242010828f9ff9b17b750812aad8c289110008c899f.

- 2026-10-12T17:35:22+00:00: Recorded command exit 1; command argv SHA-256
  e5288923b2aca470d15a690cdb61f1829e152d29dbabeb12233e704d835b86a0.

- 2026-10-12T17:35:33+00:00: Recorded command exit 0; command argv SHA-256
  972f0a26b7847e5f3631fbc7ba30ed7686ffd2f74071393ccbd3c055c1493c37.

- 2026-10-12T17:35:48+00:00: Recorded command exit 0; command argv SHA-256
  60c265741830d60dd5c62f3f59cd1873e77352003eaa1de35b94853022cb612c.

- 2026-10-12T17:35:56+00:00: Recorded command exit 0; command argv SHA-256
  a08ffcd926a32fc769588e9c189850e7ef064e58edb0c6ede17b1af2e23d21df.
