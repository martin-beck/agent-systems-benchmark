---
{
  "branch": "feature/ar-1825-source-to-executable-adapter-runtime-contract",
  "checkpoint_commit": "",
  "claim_expires": "2026-10-12T20:17:19+00:00",
  "depends_on": [
    "AR-1776",
    "AR-1778"
  ],
  "id": "AR-1825",
  "next_action": "Implement the bounded source-to-executable adapter/runtime contract, its per-tool support matrix, offline fixtures, receipts, rollback/retention behavior, and hostile negative gates; keep AR-1779 pending until this AR is accepted.",
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
  "summary": "Define and implement a bounded source-to-executable adapter and runtime contract for supported ASB tools.",
  "task_revision": 4,
  "title": "Bounded source-to-executable adapter/runtime contract",
  "updated_at": "2026-10-12T18:17:40+00:00",
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
