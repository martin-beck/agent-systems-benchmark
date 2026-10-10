# ASB project workspaces

`asb project init [PATH]` creates the bounded, portable workspace used by the
project and external-tool workflow. With no path, the current directory is
initialized:

```sh
asb project init .
```

The command creates these entries without writing credentials, host paths, or
provider responses:

```text
.asb/project.json   # ProjectConfigV1, the single project configuration
results/             # Bounded benchmark results and run evidence
catalogs/            # Generated agent/tool/workload catalogs
```

ASB prepares missing command-owned output directories automatically, one
validated component at a time. In human mode it announces each creation on
stderr, for example `ASB will create directory ... for benchmark results.`;
stdout remains reserved for the command result, including unchanged JSON
schemas. Input directories are never created implicitly: a missing or unsafe
project supplied as `--project` fails closed and tells the user to run
`asb project init` first. Dry-run commands do not create directories.

Initialization is safe to repeat. A valid existing `.asb/project.json` is
validated and preserved; missing layout directories are recreated. A partial
initialization can therefore be recovered by rerunning the same command. An
invalid configuration, symlink, or unrelated file in a required directory is
rejected rather than overwritten. The command supports `--json` for scripts;
the JSON output contains only the relative layout and next commands.

After initialization, inspect the available provider and tool contracts before
creating a plan:

```sh
asb provider-catalog
asb project init . --json
```

The project configuration contract is documented in
[`asb-config/PROJECT_CONFIG.md`](../crates/asb-config/PROJECT_CONFIG.md), with
its checked schema at
[`project-config.schema.json`](../crates/asb-config/schema/v1/project-config.schema.json).
Project initialization does not install tools or contact a provider; those
operations belong to later project-tool and catalog workflows.

The project-local tool workflow is documented in
[`PROJECT_TOOLS.md`](PROJECT_TOOLS.md). It provides rootless, bounded
installation and inventory commands without invoking arbitrary installers.

## Generated project catalogs

After installing or discovering project tools, generate the deterministic,
content-addressed catalogs that later project commands select:

```sh
asb catalog generate --project .
asb catalog list --project .
asb catalog show project-agent-catalog-v1 --project .
asb catalog select project-agent-catalog-v1 --kind agent \
  --digest-sha256 DIGEST_FROM_LIST --project .
```

Generation writes only public inventory metadata to `catalogs/` and records the
typed schema, source, digest, compatibility labels, generation time, and active
selection in `.asb/project.json`. The artifact filename includes its SHA-256.
`show` and `select` recompute that digest and reject stale, altered,
incompatible, or unavailable catalogs without changing the active selection.
Use `--json` on each command for the versioned machine result; the normal route
has human-readable output. Catalog artifacts never include paths, credentials,
API keys, prompts, or provider responses.
