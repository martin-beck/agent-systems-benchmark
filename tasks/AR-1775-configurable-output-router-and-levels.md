---
{
  "branch": "feature/ar-1775-configurable-output-router-and-levels",
  "checkpoint_commit": "d7ab95883d3913fae7b1b2646266e114f3de297d",
  "claim_expires": "2026-10-12T23:02:41+00:00",
  "depends_on": [
    "AR-1771"
  ],
  "id": "AR-1775",
  "next_action": "Implement the sole configurable output router, generated project output defaults, and four-level selection after the output contract is accepted.",
  "observed_branch": "feature/ar-1775-configurable-output-router-and-levels",
  "observed_dirty": 0,
  "observed_head": "067baebb23bdac32307d50bff63077aa1cd5afac",
  "owner": "ar1775-cli2key-runtime-luna",
  "plan": "../plans/AR-1775-configurable-output-router-and-levels.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_acceptance": {
    "evidence_class": "contract-test",
    "evidence_digest": "",
    "evidence_ref": "",
    "spec_ref": "specs/AR-1775.json",
    "spec_revision": 1,
    "status": "pending"
  },
  "spec_ref": "specs/AR-1775.json",
  "spec_revision": 1,
  "status": "in_progress",
  "summary": "Implement the mandatory configurable output router and project-configured quiet, normal, verbose, and debug levels.",
  "task_revision": 36,
  "title": "Configurable output router and levels",
  "updated_at": "2026-10-12T21:04:52+00:00",
  "worktree_key": "agent-systems-benchmark-ar-1775-configurable-output-router-and-levels"
}
---

Implement the AR-1771 output contract before progress reporting. Add one typed,
injectable output router which is the mandatory path for all ASB-owned
user-visible output. It must route ordinary human result output and operational
diagnostic/status output to independently configurable safe writers, with stdout
and stderr respectively as the default. Project initialization generates and
persists the `output` configuration with `level = "normal"`; repeated project
initialization preserves an explicit user selection.

Implement the closed levels `quiet`, `normal`, `verbose`, and `debug`, their
documented project configuration and CLI override precedence, and `-q` as a
quiet-level override. Every writer implementation must apply the same level
filter and privacy rules. `debug` is development detail, never credential or
private-host disclosure. Error handling for an unavailable/unwritable configured
destination must be specific, use the AR-1766 diagnostic catalog, and preserve
the original command outcome where safe to do so.

Route all pre-existing ASB output through this router before adding new progress
behavior. JSON is not a configurable human writer: `--json` retains its exact
stdout envelope and silences all human/operational output on both streams.

- 2026-10-12T20:45:06+00:00: AR-1771 dependency is done with accepted contract evidence; promote
  configurable output router implementation.

- 2026-10-12T20:45:28+00:00: Claimed by ar1775-cli2key-runtime-luna.

- 2026-10-12T20:47:24+00:00: Checkpointed source commit d7ab95883d3913fae7b1b2646266e114f3de297d.

- 2026-10-12T20:47:32+00:00: Heartbeat by ar1775-cli2key-runtime-luna.

- 2026-10-12T20:48:18+00:00: Heartbeat by ar1775-cli2key-runtime-luna.

- 2026-10-12T20:49:13+00:00: Recorded command exit 0; command argv SHA-256
  05423f546638fa0266e03f759296fdc2689e698f112d4b7b172bb75e95f5dca1.

- 2026-10-12T20:49:36+00:00: Heartbeat by ar1775-cli2key-runtime-luna.

- 2026-10-12T20:49:45+00:00: Recorded command exit 0; command argv SHA-256
  05faae2b09a181ad08a6c6862f1c232e689dee071f41e897288fcb13981a49cf.

- 2026-10-12T20:50:50+00:00: Recorded command exit 0; command argv SHA-256
  8acca2e00c96fbcd7f4fd47063e1d51d6bd90ee8831fa0bc8312cb3323ac2812.

- 2026-10-12T20:51:29+00:00: Recorded command exit 0; command argv SHA-256
  f7e20666638201d8567703a3d7bc028e04a5e8e4c6c509de13d4976b0c91a669.

- 2026-10-12T20:52:01+00:00: Recorded command exit 101; command argv SHA-256
  1024fc1e6cefa31b078f0b5d2ed85c7ea4586897b388bf1e308f62db58574461.

- 2026-10-12T20:52:56+00:00: Recorded command exit 0; command argv SHA-256
  53bb3b1941ff1516c3e43d286551fda4447292c217edea1208a3b18886a9611a.

- 2026-10-12T20:53:18+00:00: Recorded command exit 1; command argv SHA-256
  f7e20666638201d8567703a3d7bc028e04a5e8e4c6c509de13d4976b0c91a669.

- 2026-10-12T20:53:31+00:00: Recorded command exit 0; command argv SHA-256
  cfddfc1b8766f37e6aede838c174f3c1396c0e28ea1eae79d472cf1b97a74f13.

- 2026-10-12T20:53:49+00:00: Recorded command exit 0; command argv SHA-256
  1024fc1e6cefa31b078f0b5d2ed85c7ea4586897b388bf1e308f62db58574461.

- 2026-10-12T20:54:06+00:00: Recorded command exit 0; command argv SHA-256
  3955b7351faa1e5ec9868945b5420c568a9c8e105709a1c56a1ba5dfa83d8198.

- 2026-10-12T20:54:17+00:00: Recorded command exit 0; command argv SHA-256
  3f7cc283f35c28c8d016c16d5ffa7a9439baae2dbc8be66b9656386976944650.

- 2026-10-12T20:54:33+00:00: Recorded command exit 0; command argv SHA-256
  ec0999af0b252aa6317b083ed85b52326d6e8e33e8d1a2ebae3792b41cfd25cd.

- 2026-10-12T20:54:57+00:00: Recorded command exit 0; command argv SHA-256
  a8ce9fe217a3aa8fa0c8fd115006d5c46dc7d0062273f4856411b57cb7a7ae51.

- 2026-10-12T20:57:17+00:00: Recorded command exit 101; command argv SHA-256
  534f8613e1c316db8f0f46dd989e44e14a8ca74304a94791b4e4a1c59b2fbb1d.

- 2026-10-12T20:58:24+00:00: Recorded command exit 101; command argv SHA-256
  534f8613e1c316db8f0f46dd989e44e14a8ca74304a94791b4e4a1c59b2fbb1d.

- 2026-10-12T21:00:36+00:00: Recorded command exit 101; command argv SHA-256
  1e8ee4124b4a9cdca24d0e3ecea8cfa7b14b469f5b51bde063aa2c711bd83c35.

- 2026-10-12T21:00:43+00:00: Recorded command exit 101; command argv SHA-256
  b530f6a2c213002ab74502d3d622639491c482d2d15c63d1fb8f25b8c2ed2929.

- 2026-10-12T21:02:41+00:00: Heartbeat by ar1775-cli2key-runtime-luna.

- 2026-10-12T21:02:50+00:00: Recorded command exit 0; command argv SHA-256
  282b80b577d709e12a310c5278a71054282ce0eac13b3aec043ae79e80023351.

- 2026-10-12T21:03:05+00:00: Recorded command exit 101; command argv SHA-256
  fb8290500d0f284337dcc8907a46c8a8226920c27494fc4cab64ea58ca61ef06.

- 2026-10-12T21:03:29+00:00: Recorded command exit 0; command argv SHA-256
  95a0ed864f78b0e08c10bca09dc9cad9c101c39e8d586a4c318c71f76f16b44a.

- 2026-10-12T21:03:41+00:00: Recorded command exit 0; command argv SHA-256
  381a1ecf52daeb21a6b9db8071a84856b7735def95f3326241a485a3459c2edb.

- 2026-10-12T21:03:57+00:00: Recorded command exit 0; command argv SHA-256
  11af4a8f6d0f7b4f7281f83587a05d267b88877ffc809e3ea073b3c2b742a3b3.

- 2026-10-12T21:04:52+00:00: Recorded command exit 101; command argv SHA-256
  e5b7d350b51ebbe317a9da035759522c6386d45005d666719f826e116b60c844.
