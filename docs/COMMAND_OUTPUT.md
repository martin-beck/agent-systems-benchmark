# ASB command output contract

The executable CLI is human-first by default. Each completed command starts
with one plain sentence that says what succeeded, partially completed, or
failed. Only facts needed to understand that command follow. When the user can
safely make progress, the final line contains exactly one copyable command:

```text
ASB validated the benchmark plan.
  Run: example-run.
  Workload: original.bug-fix.
  Agent: codex.
Next: asb run /absolute/path/experiment.toml --use-config
```

Default output does not expose envelope fields such as `schema_version`, `ok`,
transport state, internal classifications, development flags, or content
digests unless that fact changes the user's decision. Successes that need no
follow-up do not invent a next step. Warnings and partial results say so
explicitly. Failures distinguish invalid user input, unavailable host or
external dependencies, and ASB product failures. Suggested argv is structured
internally, rejects control characters, is shell-quoted only for display, and
never includes credential values.

Progress and long-running service startup messages belong on stderr. Results
belong on stdout. Plain output contains no terminal escape sequences, so
`NO_COLOR`, redirected output, and narrow terminals remain readable. Prose
wraps to the bounded terminal width; a `Next:` command stays on one line so it
can be copied safely.

## Machine and diagnostic modes

Use the global `--json` option for the complete, versioned machine response:

```sh
asb --json provider-catalog > catalog.json
asb --json provider-plan --use-config > selection.json
asb --json report /absolute/results/runs/RUN_ID > report.json
```

`--format=json` and `--format json` remain exact compatibility aliases. JSON
schemas, field meanings, bytes for deterministic commands, exit codes,
redaction, and stdout/stderr boundaries are unchanged. The library `run()` and
runtime-authority entry points also retain their structured interfaces; only
the executable's ordinary terminal mode selects human presentation.

Use `--details` (or its `--verbose` alias) for bounded diagnostic identifiers
while keeping the human layout. It cannot be combined with JSON mode. Details
never reveal secrets or recursively dump an unrecognized object. Help,
version, completion scripts, and command-specific TUI help/version remain
explicit text artifacts and retain their bytes.

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
