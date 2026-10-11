---
{
  "branch": "feature/ar-1777-verified-prebuilt-tool-and-workload-acquisition",
  "checkpoint_commit": "",
  "claim_expires": "2026-10-11T10:19:17+00:00",
  "depends_on": [
    "AR-1776"
  ],
  "id": "AR-1777",
  "next_action": "Implement catalog-driven verified compatible-binary reuse and official prebuilt executable-tool acquisition into project-local storage.",
  "observed_branch": "feature/ar-1777-verified-prebuilt-tool-and-workload-acquisition",
  "observed_dirty": 2,
  "observed_head": "3031428aab47be95943b75cd753b0b5e3aeb4f6b",
  "owner": "codex-ar1777-verified-prebuilt-tool-and-workload-acquisition-luna56",
  "plan": "../plans/AR-1777-verified-prebuilt-tool-and-workload-acquisition.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_acceptance": {
    "evidence_class": "contract-test",
    "evidence_digest": "",
    "evidence_ref": "",
    "spec_ref": "specs/AR-1777.json",
    "spec_revision": 1,
    "status": "pending"
  },
  "spec_ref": "specs/AR-1777.json",
  "spec_revision": 1,
  "status": "in_progress",
  "summary": "Acquire, verify, and atomically install official compatible prebuilt executable tools into a project-local root.",
  "task_revision": 17,
  "title": "Verified prebuilt tool acquisition",
  "updated_at": "2026-10-11T08:27:06+00:00",
  "worktree_key": "agent-systems-benchmark-ar-1777-verified-prebuilt-tool-and-workload-acquisition"
}
---

Implement catalog-driven acquisition for a compatible existing system binary and
for official prebuilt executable tools. The resolver selects only an entry
matching the current OS, architecture, libc/ABI and catalog identity; downloads
only from the catalog's primary source; verifies TLS transfer policy, immutable
digest, signature/provenance when declared, archive boundaries, executable
entrypoint, and license metadata before atomically publishing under the
selected project. It records the exact catalog revision, source identity,
artifact digest, verifier outcome, platform, and usable entrypoint.

No partial output becomes active. Failed transfer, checksum/signature mismatch,
wrong platform, archive traversal, unsafe executable, revoked artifact, and
offline/unavailable source must be separately diagnosed and leave the previous
usable installation intact. Do not execute downloaded installer scripts, require
root, persist credentials, or use a package manager. Use the shared human status
reporting contract when available while keeping machine-mode output safe.

- 2026-10-11T08:19:13+00:00: AR-1776 dependency is accepted on protected main; open AR-1777 for
  implementation.

- 2026-10-11T08:19:17+00:00: Claimed by
  codex-ar1777-verified-prebuilt-tool-and-workload-acquisition-luna56.

- 2026-10-11T08:20:50+00:00: Recorded command exit 0; command argv SHA-256
  cfddfc1b8766f37e6aede838c174f3c1396c0e28ea1eae79d472cf1b97a74f13.

- 2026-10-11T08:21:48+00:00: Recorded command exit 101; command argv SHA-256
  09de3547b72d103e84c99b9d48ae326ffdc04a44851acde9c6a3f2bacafe2f00.

- 2026-10-11T08:22:09+00:00: Recorded command exit 0; command argv SHA-256
  cfddfc1b8766f37e6aede838c174f3c1396c0e28ea1eae79d472cf1b97a74f13.

- 2026-10-11T08:22:40+00:00: Recorded command exit 0; command argv SHA-256
  09de3547b72d103e84c99b9d48ae326ffdc04a44851acde9c6a3f2bacafe2f00.

- 2026-10-11T08:24:01+00:00: Recorded command exit 0; command argv SHA-256
  cfddfc1b8766f37e6aede838c174f3c1396c0e28ea1eae79d472cf1b97a74f13.

- 2026-10-11T08:24:23+00:00: Recorded command exit 101; command argv SHA-256
  bf5fcb6fc92c4de38ff2147178dd3a153eae3976f2b397ae65308300d2d8bafb.

- 2026-10-11T08:24:52+00:00: Recorded command exit 0; command argv SHA-256
  bf5fcb6fc92c4de38ff2147178dd3a153eae3976f2b397ae65308300d2d8bafb.

- 2026-10-11T08:25:14+00:00: Recorded command exit 0; command argv SHA-256
  5049a9f7f59f1c6a2a9a01db4a5e2d1744178900d10126367f823600583fe2ff.

- 2026-10-11T08:25:33+00:00: Recorded command exit 0; command argv SHA-256
  38f4dbdf79a077b55761808a83ccaa00eb1ab4128b256d42e633598470885d2f.

- 2026-10-11T08:25:58+00:00: Recorded command exit 0; command argv SHA-256
  8f9b6c1dfb961b5cdd4b75169caa06c61f3be5335277e2672faf1eae2004e617.

- 2026-10-11T08:26:44+00:00: Recorded command exit 0; command argv SHA-256
  cfddfc1b8766f37e6aede838c174f3c1396c0e28ea1eae79d472cf1b97a74f13.

- 2026-10-11T08:27:06+00:00: Recorded command exit 0; command argv SHA-256
  e274074a17e0682b5edf79b5caca577a41520ed9ca8daf9342f0433b678582a1.
