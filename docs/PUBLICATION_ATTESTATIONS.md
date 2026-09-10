# Publication attestations

Publication attestations preserve immutable evidence when a protected-history
merge has a policy defect that cannot be rewritten. They do not retroactively
make the defective commit compliant.

The [PR #131 measurement catalog merge attestation](attestations/measurement-catalog-pr131-merge.json)
records the reviewed base, head and tree, all twelve successful exact-head
checks, the tree-equivalent GitHub-verified merge, and its missing matching DCO
trailer. The merge commit remains non-DCO. The corrective attestation commit is
signed and DCO-compliant; its eventual publication must use a real multiline
GitHub merge message whose final trailer is:

```text
Signed-off-by: Martin Beck <martin.beck2@gmx.de>
```

The attestation is bounded and validated by the `asb-protocol` integration
suite. Validation binds it to the checked-in catalog fixture and rejects
unknown fields, malformed identities, false signature or DCO claims, incomplete
check evidence, and reviewed/published tree mismatch.
