# Formal assurance roadmap

Agent Relay currently maintains TLA+ and Alloy workflow/concurrency models plus
a bounded executable interleaving verifier and named crash/recovery regressions.
ASB adopts that structure and strengthens the CI exit criteria by requiring the
actual pinned model-checking tools to run, implementation trace conformance and
mutation tests that prove each gate can fail.

AR-0904 first makes design artifacts mechanically consistent: JSON Schemas,
Rust serialization, protocol examples, public API snapshots, capability output
and generated documentation. AR-0901 covers pure checked invariants and
synchronization models. AR-0905 covers temporal run/recovery/replay models.

## Checked recovery boundary

The checked recovery boundary uses two independent formal languages plus Rust
implementation traces. TLC 1.8.0 explores 3,709 reachable states through depth
17 with two attempts, two replay sessions, epoch bound two, journal bound four
and cursor bound two. Alloy 6.2.0 checks six relational assertions with exactly
two attempts/sessions, four events and 4-bit integers, while requiring one
satisfiable witness. A Rust explorer covers reachable traces through depth
eight, retains six named hostile traces, and binds recovery decisions to the
real durable store.

Checked safety properties are one lease owner, exact epoch fencing, monotonic
journal projection, no duplicate accepted terminal result, terminal uncertainty
that forbids restart, and replay cursor isolation. Deliberately weakened TLA+
and Alloy inputs must expose counterexamples. These are finite safety results,
not liveness, fairness, crash-consistency, kernel, filesystem, timing, native
platform, or unbounded-history claims.

These classifications must appear beside every result:

- **Mechanical invariant:** checked for every reachable state within the stated
  finite model and depth/scope.
- **Bounded proof:** verified by Kani or a solver for the explicit input bounds.
- **Implementation model test:** explored by Loom or generated transition traces.
- **Property/fuzz evidence:** sampled or coverage-guided tests, not proof.
- **Environmental assumption:** enforced or measured outside the model.

Initial temporal invariants include lease uniqueness and fencing, event and
journal ordering, recovery uncertainty, cancellation terminality, artifact
acceptance, replay cursor/session separation and verifier-bound completion.
Pure invariants include units, ranges, capacity/SLO evidence, experiment
comparability and strict request normalization. Native fault tests validate the
model assumptions where possible.
