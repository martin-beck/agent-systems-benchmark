# External workload registry v1

`external-workloads.json` is a provenance-only registry for opt-in suites whose
datasets are never checked into ASB. A runner must acquire the exact source
revision explicitly, verify its digest and license metadata, and record the
evaluator and container image digest before execution. `planned` is not evidence
of support: only a public, reproducible oracle run can promote a platform cell.

The registry deliberately distinguishes source, dataset, evaluator, and image
identities. A missing image digest keeps a suite unqualified and prevents a
benchmark result from being presented as comparable. The Aider Polyglot source
has no repository SPDX declaration; its per-exercise licenses and attribution
are therefore mandatory. Unsupported arm64 cells must remain explicit until
native oracle-parity evidence exists.

The Terminal-Bench v4 record deliberately pins the source release, dataset
manifest, and Harbor harness as different identities. The manifest points to
published Harbor package digests; a tag checkout is not a substitute for those
package bytes. The record remains `planned` because selected task images, SBOMs,
network policy, reset behavior, and repeated native oracle results have not all
been qualified. In particular, the resolved Harbor public-network default is
not silently treated as an ASB network-isolated workload.
