# AR-1800 plan: development warning-only trust contracts

## Outcome

In the development channel, ASB remains usable on machines whose tool ownership,
permissions, digest, or exact-version metadata does not match a published trust
contract. Users see an actionable warning, never a false claim of verification,
and the requested command or `asb tui install` continues. Strict release-channel
and structural-safety behavior remains unchanged.

## Implementation

1. Audit current main for every trusted-tool validator and call site: executable
   ownership/mode/path checks, SHA-256 and bundle comparisons, exact toolchain or
   version checks, provider/model/catalog materialization, workload/benchmark
   acquisition, and `asb tui install`. Record which result types and exit paths
   currently hard-fail and how human and JSON diagnostics are rendered.
2. Define one channel-aware policy/helper and typed diagnostic for a scoped trust
   mismatch. In `dev`, downgrade only those findings to warning-level diagnostics,
   preserve the expected degraded-trust facts, and return the normal best-effort
   continuation path. Do not duplicate channel decisions at individual call
   sites. Make warning visibility obey the existing log-level contract and expose
   equivalent redacted warning facts in `--json`.
3. Route TUI installation and all audited acquisition/materialization paths through
   the helper. A development trust-contract mismatch must not stop installation or
   command execution, but output must say what was not verified and that the
   result is development-only. Keep parser, argument, path-boundary, secret,
   process-launch, and unrecoverable I/O safety failures hard.
4. Add a complete matrix of fixtures and contract tests: each trust/mode/path,
   SHA-256, and exact-version mismatch continues in `dev`; `asb tui install`
   continues with the warning; warnings appear at warning and above and in JSON;
   repeated warnings remain bounded and redacted; stable/production negative
   fixtures still fail; malformed and unsafe inputs still fail; successful and
   partially successful outputs remain truthful.
5. Independently review the exact diff, run focused and full locked checks, verify
   exact-head and protected-main CI, qualify an installed `asb tui install` and
   representative live command, then record post-merge evidence and reconcile
   the coordinator state.

## Dependencies and boundaries

AR-1799 owns the current `asb tui install` rustup/trusted-tool repair. This AR
consumes that repaired path and owns the later policy transition; it must not
silently broaden stable/production trust exceptions or alter ASB-TUI's separate
repository without a coordinated downstream AR.
