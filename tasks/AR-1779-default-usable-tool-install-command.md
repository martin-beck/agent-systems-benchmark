---
{
  "branch": "feature/ar-1779-default-usable-tool-install-command",
  "checkpoint_commit": "",
  "claim_expires": "2026-10-11T15:54:34+00:00",
  "depends_on": [
    "AR-1777",
    "AR-1778",
    "AR-1784"
  ],
  "id": "AR-1779",
  "next_action": "Make asb tool install supported-id resolve, acquire/build, validate, and register a usable project-local executable tool by default.",
  "observed_branch": "",
  "observed_dirty": 0,
  "observed_head": "",
  "owner": "luna56-ar1779",
  "plan": "../plans/AR-1779-default-usable-tool-install-command.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_acceptance": {
    "evidence_class": "integration-test",
    "evidence_digest": "",
    "evidence_ref": "",
    "spec_ref": "specs/AR-1779.json",
    "spec_revision": 1,
    "status": "pending"
  },
  "spec_ref": "specs/AR-1779.json",
  "spec_revision": 1,
  "status": "in_progress",
  "summary": "Make default asb tool install produce and verify a usable project-local supported executable tool without manual source copying.",
  "task_revision": 5,
  "title": "Default usable tool-install command",
  "updated_at": "2026-10-11T13:57:08+00:00",
  "worktree_key": "agent-systems-benchmark-ar-1779-default-usable-tool-install-command"
}
---

Change the public experience to `asb tool install <supported-id> [--project PATH]`.
It resolves the catalog, reuses a compatible verified local binary when one is
available, otherwise retrieves a verified official prebuilt artifact, otherwise
uses the pinned source-build/dependency fallback. A success is valid only after
an executable smoke check proves the installed entrypoint can be used
by the target ASB adapter. Persist the complete receipt and active usable state
only after that check passes.

Keep explicit local-file or fixture installation as a clearly labelled
development/import mode rather than the default supported-user flow. Offer
`status`, `repair`, `update`, and `remove` semantics that expose current
catalog/source/digest/platform/build state without downloading on read-only
commands. Human output must explain selection, downloads/builds, reused versus
created dependencies, result location, and next action; JSON retains its stable
machine contract and never includes secrets. The command must not silently use a
wrong binary, silently fall back to an unrelated tool, or claim installation when
the adapter smoke check failed.

- 2026-10-11T13:53:29+00:00: Promote dependency-safe AR-1779 now that AR-1777, AR-1778, and AR-1784
  are durably done.

- 2026-10-11T13:54:34+00:00: Claimed by luna56-ar1779.

- 2026-10-11T13:55:27+00:00: Recorded command exit 0; command argv SHA-256
  ff6d1fa7a7185010f75fcfafe4bdc1bdd5b9804d8c2b3e7abdd8a1b96cd81c63.

- 2026-10-11T13:57:08+00:00: Recorded command exit 0; command argv SHA-256
  f7e20666638201d8567703a3d7bc028e04a5e8e4c6c509de13d4976b0c91a669.
