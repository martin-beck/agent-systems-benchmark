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
Git trailer block, plus an SSH
signature accepted by [allowed_signers](../config/allowed_signers). GitHub's
branch `required_signatures` rule is not the source of truth because GitHub
rejected a locally valid SSH-signed GMX-identity commit. The CI gate uses Git's
allowed-signers verification directly. Contributors add their public signing
identity through review before their commits can pass; private keys are never
stored in this repository.

The negative suite invokes the production gate commands against controlled
defects. It proves rejection of mutable Actions, missing DCO/signatures,
malformed and unsafe workflows, a synthetic credential, a denied dependency, a
known-vulnerable lockfile and an impossible coverage floor. A missing tool,
unexpectedly permissive new release, or stale command syntax therefore fails CI
instead of silently skipping a gate.

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
```

Online advisory refreshes and external downloads make cargo-audit and tool
installation network-dependent. The compiler, tests, coverage, Cargo Deny,
repository policy and already-installed analyzer executions are deterministic
against the checked-out tree. Coverage establishes exercised lines, not
correctness; formal, concurrency, mutation and fuzz evidence remain owned by
their later ARs.
