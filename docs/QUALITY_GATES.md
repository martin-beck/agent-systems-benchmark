# Repository quality gates

AR-0003 makes the roadmap's repository checks mandatory on every pull request.
The machine-readable source of tool versions, Action commits and downloaded
binary SHA-256 digests is [quality-tools.json](../config/quality-tools.json).

CI installs the exact Rust 1.93.0 toolchain and exact cargo-audit, cargo-deny and
cargo-llvm-cov releases. It downloads actionlint, zizmor and Gitleaks only over
HTTPS, verifies a platform-specific digest before extraction, and executes them
from disposable runner storage. Every GitHub Action reference is a full commit
ID. Public workflows use only GitHub-hosted disposable runners and do not use
`pull_request_target`.

Every change must meet 90% line coverage across the workspace and 95% for each
critical crate. The current critical set is `asb-core`; `asb-protocol` and
`asb-replay` become independently subject to the 95% floor when introduced.
Cargo Deny rejects disallowed licenses, duplicate versions, wildcard
dependencies and unknown sources. The exact `syn` 3.0.5 duplicate is narrowly
excepted because schemars/ref-cast requires it while serde and thiserror derive
still require `syn` 2; a version change reopens review. Cargo Audit rejects
RustSec advisories and yanked dependencies. The repository policy checks source
SPDX headers, local Markdown links and workflow immutability without network
access.

The commit gate checks every introduced commit. Each requires a
`Signed-off-by: Name <address>` entry exactly matching its author in the final
Git trailer block. Default, pull-request, workflow-dispatch and ordinary topic
commits require an SSH signature accepted by
[allowed_signers](../config/allowed_signers).

Canonical `push` validation on `refs/heads/main` has one additional offline
publication rule. A final two-parent merge may use GitHub Web Flow's PGP
signature only when its first parent is the immutable range base, its committer
is exactly `GitHub <noreply@github.com>`, its author matches the DCO trailer, and
every topic commit still has an allowed SSH signature. The armored key is pinned
byte-for-byte in [github_web_flow.gpg](../config/github_web_flow.gpg) with
SHA-256 `6e8af687f60cf3f403151c8fb1b26e95e6f9e424ca60cc8f3787bd4466a3ef84`;
only full fingerprint `968479A1AFF927E37D1A566BB5690EEEBB952194` is accepted.
GitHub documents `https://github.com/web-flow.gpg` as the public key for local
verification of web-interface commits. Verification performs no key lookup or
network request. GitHub's branch `required_signatures` result remains supporting
evidence rather than the offline source of truth. Contributors add their public
SSH signing identity through review; private keys are never stored here.

The negative suite invokes the production gate commands against controlled
defects. It proves rejection of mutable Actions, missing DCO/signatures,
malformed and unsafe workflows, a synthetic credential, a denied dependency, a
known-vulnerable lockfile and an impossible coverage floor. A missing tool,
unexpectedly permissive new release, or stale command syntax therefore fails CI
instead of silently skipping a gate.

After every required repository-quality step succeeds, CI prepares one bounded,
content-free JSON evidence record and attempts to retain it for one day. Artifact
publication is explicitly optional: a provider upload failure, including exhausted
storage quota, is reported as structured `unavailable` evidence without changing
the already-established required-check result. Preparation, malformed inputs,
cancellation, and every required artifact remain fail-closed. GitHub does not
provide a typed upload failure output, so ASB does not claim that an arbitrary
provider failure was specifically quota exhaustion. It performs no automatic
retry because account storage recalculation is delayed; a later workflow attempt
is a new, bounded operator-visible observation. AR-0846 separately owns
authorized retention cleanup and remains dependency-gated.

Run the same checks locally on Linux:

```sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --locked --workspace --no-deps
cargo deny --locked check
cargo audit --deny warnings
python3 tools/quality/check_coverage.py
bin_dir="$(tools/quality/install-external-tools.sh)"
"$bin_dir/actionlint" -config-file .github/actionlint.yaml
"$bin_dir/zizmor" --pedantic .
"$bin_dir/gitleaks" git --redact --no-banner .
python3 tools/quality/repository_policy.py --base origin/main --head HEAD
python3 tools/quality/test_failure_paths.py --bin-dir "$bin_dir"
python3 -m unittest tools.quality.test_signature_policy
```

Online advisory refreshes and external downloads make cargo-audit and tool
installation network-dependent. The compiler, tests, coverage, Cargo Deny,
repository policy and already-installed analyzer executions are deterministic
against the checked-out tree. Coverage establishes exercised lines, not
correctness; formal, concurrency, mutation and fuzz evidence remain owned by
their later ARs.
