# Project and external-tool configuration v1

`ProjectConfigV1` is the single credential-free inventory contract used by the
project initialization, discovery, installation, and catalog-selection ARs.
It lives in `asb-config`; later commands must extend this contract rather than
introduce another configuration authority.

The document contains portable relative roots (`project`, `results`, and
`catalogs`), typed inventories for agents, harnesses, benchmarks, workloads,
and support tools, active selections, and digest-bound generated catalog
references. Records retain public source/version/platform/path/digest,
capabilities, and status. Paths are relative and cannot contain `..`, absolute
roots, symlink intent, or home-directory expansion. Unknown JSON fields are
rejected. Credential values, API keys, tokens, prompts, and private host paths
are not part of this contract.

The machine-readable schema is
[`schema/v1/project-config.schema.json`](schema/v1/project-config.schema.json).
Rust callers use `ProjectConfigV1::validate` and `decode_project_config`; both
apply the same bounds and reference checks before a later AR persists state.
