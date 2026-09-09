# Formal models and bounded proofs

This directory is a standalone Rust workspace so formal dependencies do not
enter the ASB runtime or root lockfile. Kani 0.67.0 runs the proof harnesses;
Loom 0.7.2 explores synchronization. TLC 1.8.0 and Alloy 6.2.0 independently
check the finite recovery relation. All are exact pins.
Machine-readable release, archive, compiler, action, crate and source pins are
recorded in [toolchains.toml](toolchains.toml). Kani currently runs only on the
x86_64 disposable CI worker; Loom and the executable state models run natively
on both disposable x86_64 and aarch64 workers.

| Tool | Official source pin | Archive SHA-256 | License evidence |
| --- | --- | --- | --- |
| TLC 1.8.0 | TLA+ commit b123b22654942bd7f8b1bcadcc47da4ee2cf4c0e; GitHub release asset 551717837 | f3a6ba408f84e155d23c75aecc4c89322cfb99822b56d4525833419230cab8bb | upstream repository MIT file inspected |
| Alloy 6.2.0 | Alloy commit 59ba2033993449d483d54acad0e11a7bbf20354f | 6b8c1cb5bc93bedfc7c61435c4e1ab6e688a242dc702a394628d9a9801edb78d | upstream 6.2.0 LICENSE declares current code MIT |
| Temurin JRE | 17.0.20+8 through setup-java commit dded0888837ed1f317902acf8a20df0ad188d165 | action-managed distribution | Eclipse Temurin binary license boundary |

The upstream v1.8.0 TLA+ release asset was recreated again on 2026-09-09. The current official
`tla2tools.jar` is pinned through immutable GitHub asset ID 551717837, its release URL,
API-reported size and digest, and embedded source revision b123b226. The release URL is used
for runner portability; the byte-size and SHA-256 checks remain mandatory.

## Evidence classification and bounds

| Check | Classification | Complete within | Excluded |
| --- | --- | --- | --- |
| concurrency range | bounded proof | every pair of u32 bounds | allocation and scheduler behavior |
| aggregate SLO decision | bounded proof | three required criteria, all 27 decision vectors | statistical estimator correctness |
| missing latency SLO | bounded proof | every u64 latency bound for one missing successful sample | larger sampled histories |
| microseconds to nanoseconds | bounded proof | every u32 input plus the u64 overflow endpoint | larger valid u64 counters and kernel counter validity |
| replay cursor separation | bounded proof | every pair of u8 cursors, one session advance | HTTP parser details and persistence |
| request selector scope | bounded proof | every pair of u8 interaction IDs and both method classes | JSON pointer parsing and HTTP transport |
| cancellation ownership | implementation model test | every Loom interleaving of two cancellers and two finishers | signals, PID reuse and the Linux kernel |
| attempt lifecycle | exhaustive state model | every three-event trace through depth six | crash/restart and persisted journals |
| replay session separation | exhaustive state model | every two-session trace through depth four | provider dialect normalization |
| request selector negatives | exhaustive state model | three interaction IDs and both method classes | unbounded real cassette cardinality |
| temporal recovery | mechanical invariant | TLC: 3,709 reachable states, depth 17; two attempts/sessions, epoch 2, journal 4, cursor 2 | liveness, time, OS effects and larger scopes |
| recovery relations | bounded proof | Alloy: two attempts/sessions, four events, 4-bit integers; one witness and six assertions | transition order and temporal liveness |
| hostile recovery traces | implementation model test | six named traces and exhaustive Rust exploration through depth eight | unbounded histories |
| durable recovery trace | implementation model test | real AtomicStore running/terminal/stale-attempt decisions | power loss, filesystem/kernel durability |

Kani and the finite state explorer are mechanical only within the stated bounds.
Loom checks the atomic ownership algorithm, not the OS process implementation.
Kani reports unsupported caller-location and foreign-function constructs in the
compiled dependency graph; the six successful harnesses do not reach or verify
those constructs.
The TLA+ and Alloy checks are independent encodings, not equivalent proofs.
The deliberately stale TLA+ state and six weakened Alloy facts all produce
counterexamples, proving the receipt gate observes failures. Alloy's upstream
6.2.0 archive describes its current code as MIT while retaining draft Apache
text; this records that wording rather than making a stronger license claim.

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
formal/run_temporal_models.sh /absolute/tool-cache /absolute/new-scratch
~~~

The Kani command is a deliberate negative. The temporal runner verifies pinned
archive hashes, rejects pre-existing scratch directories, checks the positive
models, and requires every deliberate model mutation to produce a counterexample.
Set ASB_FORMAL_OFFLINE=1 only after both exact archives are present. No proof
result establishes native platform support, timing, fairness or liveness.
