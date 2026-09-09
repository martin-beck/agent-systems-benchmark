# ASB original engineering workloads

This crate provides seven small, original, redistributable MIT fixtures for the
workload extension v1 lifecycle: bug fix, feature addition, refactoring, test
generation, dependency migration, build repair, and repository navigation.
Preparation copies only the public prompt and initial project into a new attempt
root. Reference patches, counterexamples, and grader logic remain outside the
agent-writable workspace. Evaluation is offline and returns named checks without
echoing submitted file content.

Every manifest pins the prompt plus initial files with SHA-256 over this byte
sequence, in lexicographic relative-path order: the ASCII domain
`asb-workload-content-v1`, then for each item an eight-byte big-endian path
length, path bytes, eight-byte big-endian content length, and content bytes.
The prompt is the synthetic path `PROMPT.md`. Manifest fixture size counts only
initial workspace bytes. The scorer version identifies protected Rust logic and
is independent from workload content versions.

The lifecycle fails closed on unknown IDs, existing destinations, lexically
unnormalized roots, observed symlink ancestors or workspace objects, unexpected
files, missing ownership markers, excessive file counts or bytes, and invalid
manifests. Cleanup and reset act only on an exact root created for the selected
fixture. A failed ownership check leaves the tree untouched.

## Evidence boundary

The graders never accept lexical token presence. The test-generation oracle
evaluates submitted cases against the intended function and seeded mutants. The
other six fixtures compare the complete bounded submission against one reviewed
canonical source/build or answer structure, including files that the task must not
change. Comment-only, string-only, no-op build, source-tampering, and behavior-changing
submissions therefore fail. This intentionally accepts only the canonical solution,
not every semantically equivalent implementation.

The graders do not execute agent-written code. This avoids treating untrusted
native execution as isolated, but it also means these small fixtures do not prove
general language semantics or resistance to an agent that has read the public
grader source. They are deterministic infrastructure controls, not substitutes
for established external benchmarks.

Evaluation also binds each constructor-controlled report to a domain-separated
verifier-contract digest, the exact workload digest, workload ID, and independent
scoring version before reading the submitted tree. A caller cannot substitute a
different built-in grader, construct a passing report, or make an agent-written
grader script or deceptive exit status authoritative. The contract digest is an
integrity identity, not executable provenance or authentication; production
scoring separately content-addresses and verifies its staged verifier artifact.

Reference patches must pass; checked counterexample patches must fail. Tests also
prove clean reset and cleanup, traversal/symlink rejection, byte/file bounds, and
manifest pin consistency. Patch fixtures use exact-tree zero-context hunks and are
applied with `git apply --unidiff-zero`. Native x86_64 and pinned QEMU-emulated
AArch64 CI exercise the same offline Rust implementation. Native ARM64 remains
optional future qualification. No live API, network destination, compiler for
fixture languages, or external dataset is required.

Controlled runs set `ASB_TEST_SCRATCH` to an absolute directory on the configured
development volume. If it is absent, tests use an absolute `CARGO_TARGET_DIR`
subdirectory when configured, or the operating system's temporary directory for
portable external builds. Relative configured scratch or target paths fail closed;
each attempt remains uniquely named and successful cleanup removes its attempt root.

## Validity and portability registry

`registry/v1/original-workloads.json` is the closed, bounded v1 validity registry.
Its generated JSON Schema is checked structurally against the Rust types, while
runtime validation enforces relationships JSON Schema cannot express. Each exact
workload revision records source, license, content and split digests, public-versus-
holdout policy, scorer/reference/counterexample evidence, direct pins, limitations,
and explicit platform/architecture cells.

Platform states are deliberately not inferred. `native-tested` requires a public run,
artifact digest, date, and exact booted kernel; simulated evidence cannot carry a guest
native claim. Any rebuilt, translated, or filtered workload requires both adaptation
and semantic-parity digests. Performance thresholds require a host-class digest,
paired sample count, uncertainty, and evidence artifact for a declared platform cell.
The initial public fixtures remain `planned` on both Ubuntu architectures: native CI
of the Rust canonical grader does not execute each fixture language and therefore is
not native workload qualification. Their empty performance-calibration lists mean no
speedup or host-performance claim. Public tasks make no holdout or contamination-
resistance claim; imported suites must add their own pinned licenses, dependency
artifacts, exposure policy, adaptations, and qualification evidence.

## External result workflow

External suites are opt-in and remain unqualified until source archives, evaluator
image, SBOM, and oracle evidence are independently pinned. The checked-in helpers
under `tools/quality/` form a fail-closed workflow:

1. verify an acquired artifact with `verify_external_artifact.py`;
2. materialize it with `materialize_external_source.py` (no execution during extraction);
3. create a bounded plan with `plan_external_workload.py`;
4. run only a qualified evaluator with `run_external_workload.py`;
5. validate its exact bounded result using `validate_oracle_result.py`; and
6. compare and render results with `compare_external_results.py` and
   `render_external_report.py`.

Comparisons with different oracle or evaluator identities are reported as
`non-comparable`; they must not be combined into an unqualified global ranking.
