# Project-local tool installation

ASB can install a bounded external-tool artifact into a project without root,
network access, a package manager, or a shell. The registry is stored in the
project's `.asb/project.json`; the artifact is copied to `.asb/tools/<id>/tool`.
Only a regular local file or a deterministic development fixture is accepted:

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
