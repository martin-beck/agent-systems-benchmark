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
