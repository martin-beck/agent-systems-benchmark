# ASB command output contract

The executable CLI is human-first by default. Every human presentation line
starts with one fixed-width, color-independent state marker: `[ OK ]` for a
success, `[ERR ]` for an error or failure, `[WARN]` for a warning or partial
result, and `[WAIT]` for a bounded in-progress operation. Only facts needed to
understand that command follow. When the user can safely make progress, the
final line contains exactly one copyable command:

```text
[ OK ] ASB validated the benchmark plan.
[ OK ] Run: example-run.
[ OK ] Workload: original.bug-fix.
[ OK ] Agent: codex.
[ OK ] Next: asb run /absolute/path/experiment.toml --use-config
```

Default output does not expose envelope fields such as `schema_version`, `ok`,
transport state, internal classifications, development flags, or content
digests unless that fact changes the user's decision. Successes that need no
follow-up do not invent a next step. Warnings and partial results say so
explicitly. Failures distinguish invalid user input, unavailable host or
external dependencies, and ASB product failures. Suggested argv is structured
internally, rejects control characters, is shell-quoted only for display, and
never includes credential values.

Every failure also names its catalogued cause, safe affected subject, state
change (including when durable state is unknown), and the reviewed recovery.
For example, `permission denied`, `not a directory`, and `unsafe symlink`
remain visibly different; provider authentication is not presented as a
transport failure; timeout is not presented as cancellation. A `Next:` command
is printed only where the invocation and diagnostic make that exact command
safe; otherwise the output gives a concrete correction and explicitly avoids a
blind retry.

The routed TUI path also keeps trusted-tool, source-identity, candidate
lifecycle, artifact-transfer, rollback-state, and bounded-quota failures
distinct. These identities are rendered with a concrete consequence and
recovery rather than being reported as provider rejection or a generic
unclassified failure.

Ordinary human results use stdout; human operational messages and diagnostics
use stderr. A project created by `asb project init` persists this normal-level
default in its credential-free `output` configuration. It has the closed levels
`quiet`, `normal`, `verbose`, and `debug`; command-line `--quiet` (`-q`),
`--verbose`/`--details`, and `--debug` override the project level. Quiet
suppresses ASB-owned human presentation while preserving effects, result
schemas, and exit status. It does not suppress required interactive input or
the launched TUI application itself.

Progress and long-running service startup messages belong on stderr. Plain
output contains no terminal escape sequences, so `NO_COLOR`, redirected output,
and narrow terminals remain readable. Prose wraps to the bounded terminal
width; a `Next:` command stays on one line so it can be copied safely.

Before mutating a command-owned destination, human mode may emit one bounded
stderr notice in the form `ASB will create directory PATH for PURPOSE.`
Notices are emitted only for directories that are actually missing; existing
directories are reused silently. JSON mode keeps stdout byte-stable and does
not include local path notices. Unsafe, symlink, non-directory, read-only, and
permission-conflicting destinations fail closed with an actionable error.

## Machine and diagnostic modes

Use the global `--json` option for the complete, versioned machine response:

```sh
asb --json provider-catalog > catalog.json
asb --json provider-plan --use-config > selection.json
asb --json report /absolute/results/runs/RUN_ID > report.json
```

`--format=json` and `--format json` remain exact compatibility aliases. JSON
mode is silent on stderr and emits no human result, diagnostic, status, update,
or progress bytes on either stream; `-q --json` has the same machine-output
behavior. JSON schemas, field meanings, bytes for deterministic commands, exit
codes, redaction, and stdout/stderr boundaries are unchanged. The library
`run()` and runtime-authority entry points also retain their structured
interfaces; only the executable's ordinary terminal mode selects human
presentation.

Use `--details` (or its `--verbose` alias) for bounded operational detail, or
`--debug` for bounded development diagnostics, while keeping the human layout.
Neither can be combined with JSON mode, and `--quiet` cannot be combined with a
different human level. No level reveals secrets or recursively dumps an
unrecognized object. Version, completion scripts, and command-specific TUI
help/version remain explicit text artifacts and retain their established bytes.
Top-level help intentionally changes to document human output, JSON selectors,
and details mode.

| Outcome | Exit behavior | Human guidance |
| --- | --- | --- |
| success | existing command-specific zero status | outcome and relevant facts |
| warning or partial result | existing command-specific status | limitation is named; no false success claim |
| usage error | 2 | argument cause and `asb --help` |
| validation error | 3 | invalid input/configuration and a safe correction route |
| operation/product failure | 4 | plain cause and state uncertainty where relevant |
| failed/inconclusive benchmark | 5/6 | benchmark outcome, retained run count, no fabricated recovery |
| cancelled benchmark | 130 | explicit cancellation outcome |

Credential values, prompts, responses, authorization headers, and private
runtime material are forbidden from both human output and suggested commands.

## First-run human progression

The ordinary executable guides a first run without pretending that user
choices are automatic. `asb setup` ends with `Next: asb provider-catalog`.
The provider catalog shows each profile-to-model route and labels only routes
that setup can persist; it then shows this choice form, not a fabricated Next:

```text
Command form: asb setup --agent AGENT --provider-profile PROVIDER --model MODEL --persist.
```

After the user substitutes displayed values, setup confirms the actual agent,
provider, and model and ends with `Next: asb workload-catalog`. The workload
catalog lists runnable workloads and shows the supported creation form:

```text
asb plan create --workload WORKLOAD --agent AGENT \
  --agent-executable AGENT_EXECUTABLE --output PLAN --use-config
```

The created plan points to `asb plan PLAN --use-config`. Validation chooses
`run` or `sweep` from the validated plan, preserving the selection option.
Execution names retained run IDs and, when retained run directories exist,
ends with an exact `asb report RUN...` command. A report over at least two runs
ends with the corresponding `asb compare RUN...` command. `asb tui` remains a
separate explicit launch; installation failures may offer `asb tui install`
only when that command addresses the reported condition.
