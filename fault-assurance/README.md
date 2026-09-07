# Fault assurance

AR-0902 adds bounded, reproducible assurance campaigns without making fuzz or
mutation tooling a normal ASB build dependency.

- The fuzz directory is an independent cargo-fuzz workspace pinned to
  cargo-fuzz 0.13.2, libfuzzer-sys 0.4.13, and nightly-2025-11-21. Targets cover
  bounded JSON-RPC framing, cassette decoding and chunking, structured SSE replay
  encoding, and artifact-name traversal at a real private store.
- The store fault regression kills a real child after durable running intent
  and injects a storage-full reader error. The pre-existing atomic replacement
  unit test covers partial-write and rename crash boundaries.
- The replay fault regression drops a real loopback peer before a large SSE
  response completes and proves that the interaction is retryable.
- cargo-mutants 27.1.0 is limited to five strict-matcher comparison mutants,
  one SLO minimum-bound comparison, and one analysis-accounting field mutant.
  All seven must be viable and caught by the normal test suite.
  Mutants run serially because sharing one explicit Cargo target across copied
  source trees can otherwise cross-contaminate Cargo fingerprint decisions.

Pull requests run retained fault tests, bounded fuzz executions, and all seven
mutation sentinels. The scheduled job increases fuzz runs but remains bounded.
The checked-in synthetic seeds retain reviewed regressions. A newly discovered
counterexample remains only on its disposable runner; reproduce it locally,
then minimize, privacy review, and commit it before relying on it as evidence.

These campaigns prove Linux process, filesystem, and local loopback behavior on
disposable GitHub-hosted x86_64 and aarch64 runners. They do not prove physical
disk exhaustion, power-loss persistence, hostile same-UID ancestor replacement,
kernel OOM behavior, remote-network security, or non-Linux platforms.
