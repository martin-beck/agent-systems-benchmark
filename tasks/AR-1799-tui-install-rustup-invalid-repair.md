---
{
  "branch": "repair/ar-1799-tui-install-rustup-invalid",
  "checkpoint_commit": "",
  "claim_expires": "",
  "depends_on": [
    "AR-1738",
    "AR-1636"
  ],
  "id": "AR-1799",
  "next_action": "PR #613 (63ba8e7973545f65bd8dda56f3aa24d9d339bb16) is open; focused rustup tests, fmt, clippy, and independent review pass. Await remaining exact-head hosted checks, then merge and run post-merge installed-entrypoint qualification.",
  "observed_branch": "repair/ar-1799-tui-install-rustup-invalid",
  "observed_dirty": 0,
  "observed_head": "e058ea2b5881e047c2a02a5bc5bd4f23103c28c0",
  "owner": "",
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
  "status": "open",
  "summary": "Repair the remaining current-main asb tui install trusted_tool_invalid failure for development rustup/rustc layouts and qualify the real installed entrypoint.",
  "task_revision": 7,
  "title": "Repair asb tui install rustup/rustc trusted-tool rejection",
  "updated_at": "2026-10-12T17:10:58+00:00",
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
