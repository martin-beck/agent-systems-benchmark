# ASB command output contract

ASB command results are newline-delimited, versioned JSON on stdout. Diagnostic
progress belongs on stderr. The global `--json` selector is accepted in any
option position and is the stable spelling for scripts; it is deliberately
additive because existing releases already emit JSON by default. The setup
`--format=json` spelling remains supported, and `--format json` is accepted as
an equivalent alias.

| Command family | JSON selector | Exit classes | Credential behavior |
| --- | --- | --- | --- |
| `doctor`, `setup`, `provider-catalog`, `workload-catalog` | optional `--json` | 0 success, 2 usage, 3 validation, 4 operation | credential-free |
| `provider-plan`, `plan`, `report`, `compare` | optional `--json` | 0 success, 2 usage, 3 validation, 4 operation | planning/inspection only |
| `run`, `sweep`, `record`, `replay*`, `easy` | optional `--json` | 0 success, 2 usage, 3 validation, 4 operation | local/mock and strict replay remain fail-closed |
| `auth`, `serve`, `tui` | command-specific JSON contract | 0 success, 2 usage, 3 validation, 4 operation | runtime/control-owned |

Every successful machine-readable response contains `schema_version`, `ok`, and
`command` where the command owns a response envelope. Errors use the same
stdout envelope and retain the owning command name. Development-only missing
credentials, signature validation, and key-management services are represented
as warnings in the relevant response and do not block local/mock setup.

Examples:

```sh
asb provider-catalog --json > catalog.json
asb provider-catalog > catalog.json       # compatibility default
asb setup --json
asb setup --format=json                   # compatibility alias
asb setup --format json                   # equivalent alias
```

The catalog output is secret-free: it contains provider/model identities,
credential source labels, selection status, and unavailable reasons, never
credential values or authorization headers.
