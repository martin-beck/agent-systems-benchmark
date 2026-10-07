# Development review identity policy

This policy applies only to development integration. It does not qualify a
release, alter signed merge integrity, or weaken production, provenance, or
verified-channel controls.

Before a development change is merged, the exact candidate must still have
passing applicable CI and privacy checks, SSH-signed/DCO-valid commits, and an
independent technical review-worker result bound to the exact head and tree.
The reviewer-worker boundary is the safety requirement: the reviewer runs in a
separate worker context from the implementer and records its findings.

The GitHub login identity is not an additional safety boundary for development.
A same-account GitHub approval by the topic author is therefore accepted after
the independent reviewer-worker has approved the exact candidate. A second
GitHub account, authorized maintainer, or collaborator approval is not required
for development integration. The GitHub approval cannot replace the
independent technical review, exact-head checks, signatures, DCO, or privacy
checks.

Verified releases remain fail-closed under the existing release policy. Their
independent verifier, signed artifacts, provenance, and post-publication gates
are unchanged; a same-account development approval is never release evidence.
