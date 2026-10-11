# Project-local tool installation

ASB can install a bounded external-tool artifact into a project without root,
network access, a package manager, or a shell. The registry is stored in the
project's `.asb/project.json`; the artifact is copied to `.asb/tools/<id>/tool`.
Supported executable identities are defined by the immutable, checked-in
`asb-cli/fixtures/v1/tool-acquisition-catalog.json`. The catalog records the
official source, pinned version/revision, platform tuple, artifact digest,
license, entrypoint, and bounded source-build recipe. A PATH or local binary
is never trusted merely because its name matches: reuse must first verify the
catalog identity. It covers the supported agents (`aider`, `codex`, `gemini`,
`goose`, `mini-swe`, `opencode`, `opendesk`, `openhands`, `openjiuwen`, and
`qwen-code`), development-only `cli2key`, and optional Linux monitoring tools
`perf` and `bpftool`. Provider APIs are not executable tool entries.

Only a regular local file or a deterministic development fixture is accepted
by the current offline development installer:

```sh
asb project init .
asb tool install my-agent --kind agent --source /absolute/path/to/agent \
  --version 1.2.3 --project .
asb tool install fake-harness --kind harness --source fixture://fake-harness \
  --version 0.1.0 --project .
```

Supported kinds are `agent`, `harness`, `benchmark`, `workload`, and
`support`. `fixture://` sources are deterministic development fixtures; they
do not contact a provider or execute a command. HTTPS, shell commands,
symlinks, directories, oversized files, and unknown options are rejected.
Credentials and private source paths are never written to the project record.
Production acquisition reports typed outcomes for unsupported platforms,
missing verification, revoked identities, unavailable official sources, and
unavailable build recipes; it never falls back to an arbitrary package
manager or shell installer. `cli2key` is explicitly development-only.

When a verified prebuilt alternative is unavailable, a catalog source recipe
may be selected. Source recipes carry the exact source revision, toolchain
PATH, argv, output and byte/time ceilings. They execute with a cleared
credential environment in a transaction-owned staging directory and publish
the tool only after the declared entrypoint is present. Declared dependencies
are recorded in the receipt; an existing system dependency is reusable only
after its catalog identity and version match. Builds never invoke `sudo`, a
shell, or a global package manager. `toolchain_unavailable`,
`platform_unsupported`, `dependency_build_failed`, `build_policy_rejected`,
`build_output_mismatch`, `build_resource_limit`, `build_timeout`,
`build_cancelled`, and `build_cleanup_failed` remain distinct recovery
outcomes.

Use `--dry-run` to validate and preview an install without changing the
project. Repeating an identical install is idempotent; a conflicting existing
ID must be removed explicitly first. Inspect and remove records with:

```sh
asb tool list --project .
asb tool status my-agent --project .
asb tool remove my-agent --project .
```

Installation stages and hashes the artifact before atomically publishing the
project record. A failed staging or record update removes the new artifact and
leaves the previous registry unchanged.

Generate and inspect the associated typed catalogs with the project workflow
described in [`PROJECT_WORKSPACES.md`](PROJECT_WORKSPACES.md#generated-project-catalogs).
