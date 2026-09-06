# Planned formal assurance

ASB will follow Agent Relay's layered formal-methods pattern: readable TLA+ and
Alloy specifications, a deterministic finite executable model, named hostile
counterexample traces, implementation contract tests and an explicit table of
environmental assumptions. This directory contains the design boundary at
bootstrap; no checked model is claimed yet. AR-0901, AR-0904 and AR-0905 own the
machine-checked artifacts and CI integration.

## Model partitions

| Model | Safety properties |
| --- | --- |
| Experiment identity and units | Required identity fields, compatible comparisons, dimensional correctness, missing evidence never passes |
| Run and process lifecycle | Valid transitions, unique terminal outcome, cancellation fencing, bounded attempts, reconciliation before retry |
| Journal and artifact publication | Monotonic revisions/sequences, atomic visibility, hashes precede acceptance, projections never observe future facts |
| Scheduler and resource leases | Exclusive resource ownership, stale lease fencing, stop-budget enforcement, no accepted duplicate trial |
| Replay matching and streams | Per-session cursor isolation, causal response references, fail-closed mismatch, no outbound fallback, ordered terminal event |
| Scoring and completion | Immutable verifier identity, score revisions refer to durable observations, completion requires every declared contract |

TLA+ models state transitions and temporal safety. Alloy provides a second
relational expression for structural constraints. A small executable verifier
exhaustively explores bounded interleavings and replays retained hostile traces.
Kani proves suitable bounded Rust functions; Loom enumerates synchronization
interleavings in implementations. Property and fuzz tests cover larger input
spaces without being described as proofs.

## CI evidence rules

Pin the exact TLC, Alloy, Kani, Loom and solver/toolchain revisions and verify
download hashes. Pull-request CI runs all bounded safety checks within declared
limits. A scheduled job raises selected scopes to catch state-space sensitivity.
For each result publish model/tool revision, finite bounds, state count, elapsed
budget, invariants and assumptions. A timeout, skipped assertion, reduced bound
or incomplete search fails; it is not a successful proof.

The verifier must parse and execute the specification or independently evaluate
its transition relation. Merely checking that declaration names occur in text
does not establish an invariant. Mutation fixtures must demonstrate that each
critical invariant fails when deliberately broken. Generated counterexample
traces become stable Rust conformance tests.

Machine-readable contracts use a canonical versioned schema. CI round-trips
schema examples through Rust types, checks negative corpora, snapshots the public
Rust API, checks semantic compatibility and regenerates CLI capability/support
documentation with a clean diff. Every completion claim is derived from recorded
contract evidence rather than free-form narrative.

Keep identities opaque in models. Do not include hosts, accounts, paths, prompts,
responses or secrets. Environmental assumptions such as monotonic clocks,
filesystem durability, cgroup/kernel enforcement and atomic rename guarantees
are documented and validated by separate fault/native tests.
