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
  "observed_dirty": 0,
  "observed_head": "eff009eeefb1b7c4c95217044ba44de5c4d44853",
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
  "task_revision": 51,
  "title": "Verified prebuilt tool acquisition",
  "updated_at": "2026-10-11T09:19:16+00:00",
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

- 2026-10-11T08:27:20+00:00: Recorded command exit 0; command argv SHA-256
  354d1ffbaab3d1c7bc32de06e4a142bdace02fc6138f7607d3a169eabfddf873.

- 2026-10-11T08:27:55+00:00: Recorded command exit 101; command argv SHA-256
  bbfa4cb9328e16865a97a7cfe74f9cc4cdfc93b196b5430ed320e8cbe21e8c7c.

- 2026-10-11T08:28:10+00:00: Recorded command exit 0; command argv SHA-256
  cfddfc1b8766f37e6aede838c174f3c1396c0e28ea1eae79d472cf1b97a74f13.

- 2026-10-11T08:28:35+00:00: Recorded command exit 101; command argv SHA-256
  bbfa4cb9328e16865a97a7cfe74f9cc4cdfc93b196b5430ed320e8cbe21e8c7c.

- 2026-10-11T08:56:09+00:00: Recorded command exit 0; command argv SHA-256
  b7c438686581cf292b6cd41497ceabb3af42dfed57e04dfd6cc89efed8e2d15b.

- 2026-10-11T08:56:44+00:00: Recorded command exit 1; command argv SHA-256
  79023ab80adf283830116cb1fb28b61eb026e79cd8c629f9d515d856b0f120d1.

- 2026-10-11T08:57:21+00:00: Recorded command exit 0; command argv SHA-256
  5c8c2b57164656b7b9dc47db58a13f4f84aef17ff2dcb9d0e5aa63ce3daa0be8.

- 2026-10-11T08:57:43+00:00: Recorded command exit 0; command argv SHA-256
  a5d8928bac41f8c30d18988880639b39b1c8761f78d5cb8b6b1b434c70fcf507.

- 2026-10-11T08:58:10+00:00: Recorded command exit 101; command argv SHA-256
  c5cbe4ba97c11f490f707d94f43896554adfc4783cb740d4ef00618697d9a2c2.

- 2026-10-11T08:58:48+00:00: Recorded command exit 101; command argv SHA-256
  27012d5c11663bd6aa158639d5d11a59bd02737e13b1862854d5171f63802b34.

- 2026-10-11T08:59:30+00:00: Recorded command exit 0; command argv SHA-256
  27012d5c11663bd6aa158639d5d11a59bd02737e13b1862854d5171f63802b34.

- 2026-10-11T08:59:59+00:00: Recorded command exit 1; command argv SHA-256
  6055db822f0bc18d73a91c94c941ee18a235c84207312d5b1f8f2e8ab75c6143.

- 2026-10-11T09:00:22+00:00: Recorded command exit 0; command argv SHA-256
  ff14524c322d63b7a96bc562f2680b60c5b01f305ff6d91d4c42070100a7ddbb.

- 2026-10-11T09:00:43+00:00: Recorded command exit 0; command argv SHA-256
  6055db822f0bc18d73a91c94c941ee18a235c84207312d5b1f8f2e8ab75c6143.

- 2026-10-11T09:01:20+00:00: Recorded command exit 1; command argv SHA-256
  5e8c41f5b30c843bc6af1148d6ca516863d0cac7f9b94f2ea1e9c299030aeb33.

- 2026-10-11T09:01:43+00:00: Recorded command exit 0; command argv SHA-256
  670566b2aa46cbc9cc5097e391ebc58381a7bfcbc1cc3b0d8735efc1557fe2d6.

- 2026-10-11T09:02:06+00:00: Recorded command exit 0; command argv SHA-256
  cd5574d53ab39bd215e74bf8c6ec8361edc60cd4b17ffa73a80e5d1288b0ef6e.

- 2026-10-11T09:02:40+00:00: Recorded command exit 128; command argv SHA-256
  297962ed21f62e9e39dfd878795cdef98512878e8443159c62be2d6bbdecde4c.

- 2026-10-11T09:03:06+00:00: Recorded command exit 0; command argv SHA-256
  8a5c3ad5fe45397d3ade8103e51c02899d3091e184b71efc744183048f067fc0.

- 2026-10-11T09:03:30+00:00: Recorded command exit 0; command argv SHA-256
  49d904942b2f3bcbbe9f9f973b7b12909fe6acab79f0c2aa55b087265db593f3.

- 2026-10-11T09:10:32+00:00: Recorded command exit 0; command argv SHA-256
  a365e89629a3e9f997658f3f4ed59346f39728961f7d9e21ed7905d0635478ee.

- 2026-10-11T09:10:56+00:00: Recorded command exit 0; command argv SHA-256
  1e8f688336a622ace647fc2a4fcf04bac119cd52cfda898e24f9ec7441021a6d.

- 2026-10-11T09:11:20+00:00: Recorded command exit 0; command argv SHA-256
  6e0a7fe1fefdfac5849cdc8e6a939d8146eb2f643a5835eab6193f70c7d40e3b.

- 2026-10-11T09:11:46+00:00: Recorded command exit 0; command argv SHA-256
  ae36b4dda97b54a6246bc832bd92a038a9b6f3a4d0a015c3982a523d3312c954.

- 2026-10-11T09:12:56+00:00: Recorded command exit 0; command argv SHA-256
  27012d5c11663bd6aa158639d5d11a59bd02737e13b1862854d5171f63802b34.

- 2026-10-11T09:17:20+00:00: Recorded command exit 101; command argv SHA-256
  71c8c61ba6f64d4300d6a82b383271de01e28115385011fc9cb09b660b5f1f0a.

- 2026-10-11T09:17:59+00:00: Recorded command exit 101; command argv SHA-256
  71c8c61ba6f64d4300d6a82b383271de01e28115385011fc9cb09b660b5f1f0a.

- 2026-10-11T09:18:27+00:00: Recorded command exit 0; command argv SHA-256
  71c8c61ba6f64d4300d6a82b383271de01e28115385011fc9cb09b660b5f1f0a.

- 2026-10-11T09:18:56+00:00: Recorded command exit 0; command argv SHA-256
  ff14524c322d63b7a96bc562f2680b60c5b01f305ff6d91d4c42070100a7ddbb.

- 2026-10-11T09:19:16+00:00: Recorded command exit 0; command argv SHA-256
  a26d528be767c9e4c36e318fff7ed3c73e91c6f74bc06f23b853015967372d73.
