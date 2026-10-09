# Fine-grained human diagnostic contract

The public CLI boundary resolves every `CliError` through
`crates/asb-cli/src/diagnostic.rs`. The resolver is a closed vocabulary: a
known producer receives a typed cause, subject, operation phase, state-change
result, and bounded remediation. A newly introduced code or message is
classified as `UnknownCause` until it is explicitly reviewed and added to the
catalog. This prevents an accidental fallback from hiding a known cause in a
generic `operation`, `validation`, or `unavailable` label.

## Severity and state semantics

`Error` means the request was rejected before work could begin, `Failure` means
an attempted operation did not complete, and `Warning` is reserved for a
completed or continuing operation with a material limitation. These values are
diagnostic metadata only: the existing exit codes and machine JSON envelope
remain unchanged. `StateChange` distinguishes not-started, unchanged,
completed, partial, rollback, and unknown outcomes; an unknown state is used
when reconciliation is required before retrying.

The minimum cause set is intentionally explicit: missing parent/input,
already-existing target, target type errors, permission and read-only storage,
unsafe topology, invalid/malformed/incompatible input, stale identity,
unavailable capability, missing tool, provider authentication/rejection,
transport failure, timeout, cancellation, partial completion, reconciliation,
unexpected product failure, and unknown cause.

## Producer inventory

The catalog is attached at the shared `CliError` constructor boundary, which
covers the following public producers on the implementation baseline:

| Surface | Producers covered | Exit/JSON boundary |
| --- | --- | --- |
| CLI parsing and setup/config/auth | usage, validation, filesystem/configuration, credential-reference and control-service paths | `CliError::{usage,validation,operation}`; existing envelope and exits |
| project/tool/catalog | project initialization, registry mutation, install/remove, discovery, provider and workload catalog reads | same shared constructors |
| plan/run/sweep | plan decoding, preparation, runtime admission, execution, cancellation, partial points and store/reconciliation paths | same shared constructors; result schemas unchanged |
| report/compare | stored-run loading, comparison incompatibility and report read paths | same shared constructors |
| record/replay | capture validation/sealing, cassette compatibility, strict replay and offline boundaries | same shared constructors |
| provider/network | credential, catalog, transport, provider status, response bounds and authentication paths | `operation_code`/`validation_code` mappings |
| `easy` | delegated setup, lifecycle, provider, plan, run, sweep, report, compare and recording paths | delegated shared constructors |
| routed TUI | trusted-tool, channel, artifact, transfer, lifecycle, rollback and development workspace codes | `RouterError::diagnostic`; stable response fields remain unchanged |

The routed TUI response retains its existing `classification` and
`remediation` fields for machine compatibility. Its internal error is resolved
through the same catalog before the response is rendered.

## Privacy boundary

Catalog context contains only static safe labels. It never stores or emits
host paths, raw operating-system errors, credentials, provider payloads,
prompts, transcripts, or private runtime locations. User-supplied destinations
remain governed by the existing human renderer and are not added to machine
JSON. `--details` may show the typed cause, subject, phase, and state result,
but does not bypass redaction or change the public envelope.

Focused tests cover cause preservation, explicit unknown-cause behavior,
context privacy, and routed TUI compatibility. Adding a public diagnostic
producer requires a catalog mapping and a corresponding positive and negative
test.
