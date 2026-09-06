# Formal models and bounded proofs

This directory is a standalone Rust workspace so formal dependencies do not
enter the ASB runtime or root lockfile. Kani 0.67.0 runs the proof harnesses;
Loom 0.7.2 explores the synchronization model. Both are exact pins.
Machine-readable release, archive, compiler, action, crate and source pins are
recorded in [toolchains.toml](toolchains.toml). Kani currently runs only on the
x86_64 disposable CI worker; Loom and the executable state models run natively
on both disposable x86_64 and aarch64 workers.

## Evidence classification and bounds

| Check | Classification | Complete within | Excluded |
| --- | --- | --- | --- |
| concurrency range | bounded proof | every pair of u32 bounds | allocation and scheduler behavior |
| aggregate SLO decision | bounded proof | three required criteria, all 27 decision vectors | statistical estimator correctness |
| missing latency SLO | bounded proof | every u64 latency bound for one missing successful sample | larger sampled histories |
| microseconds to nanoseconds | bounded proof | every u32 input plus the u64 overflow endpoint | larger valid u64 counters and kernel counter validity |
| replay cursor separation | bounded proof | every pair of u8 cursors, one session advance | HTTP parser details and persistence |
| cancellation ownership | implementation model test | every Loom interleaving of two cancellers and two finishers | signals, PID reuse and the Linux kernel |
| attempt lifecycle | exhaustive state model | every three-event trace through depth six | crash/restart and persisted journals |
| replay session separation | exhaustive state model | every two-session trace through depth four | provider dialect normalization |

Kani and the finite state explorer are mechanical only within the stated bounds.
Loom checks the atomic ownership algorithm, not the OS process implementation.
Kani reports unsupported caller-location and foreign-function constructs in the
compiled dependency graph; the five successful harnesses do not reach or verify
those constructs.
Native process cancellation and replay integration tests remain the evidence for
environmental assumptions. The production trace test executes the real Linux
runtime cancellation boundary, real cgroup collector, and real strict replay
service. It compares their outcomes with the corresponding finite models,
including two identical independently routed replay sessions and mismatch
rollback. AR-0905 owns temporal crash/recovery models.

## Reproduction

~~~sh
cargo test --locked --manifest-path formal/Cargo.toml
cargo kani --manifest-path formal/Cargo.toml
kani formal/fixtures/kani_false_assertion.rs  # must fail
~~~

The last command is a deliberate negative proving the Kani gate observes a real
counterexample. No proof result establishes native platform support or timing.
