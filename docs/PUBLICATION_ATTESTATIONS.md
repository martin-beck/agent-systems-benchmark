# Publication attestations

Publication attestations preserve immutable evidence when a protected-history
merge has a policy defect that cannot be rewritten. They do not retroactively
make the defective commit compliant.

The [measurement catalog merge attestation](attestations/measurement-catalog-pr131-merge.json)
records both immutable recovery boundaries. PR #131 had twelve successful
exact-head checks and a tree-equivalent, GitHub-verified merge, but that merge
had no matching DCO trailer. PR #133 then reviewed and published the bounded
attestation with twelve successful exact-head checks and exact tree equality.
Its GitHub merge is validly signed, but its author is
`martin-beck <martin.beck2@gmx.de>` while the raw trailer names `Martin Beck`.
It therefore remains non-DCO; the second attestation does not rewrite or repair
either historical commit.

The [PR #132 capability coverage attestation](attestations/capability-coverage-pr132-merge.json)
records a separate validly signed, DCO-valid and tree-equivalent publication.
Its exact-head checks all passed, but the first post-merge Repository quality run
rejected the topic's synchronization topology. AR-1043 preserves that immutable
failure while adding only the bounded first-parent-spine form documented in the
quality gates.

The [PR #136 Gemini readiness attestation](attestations/gemini-readiness-pr136-merge.json)
records a validly signed, tree-equivalent merge whose GitHub author was
`martin-beck <martin.beck2@gmx.de>` but whose raw message contained no
`Signed-off-by` trailer. It binds the reviewed Gemini source bytes, all twelve
successful exact-head checks, and the precise protected-main policy failure.
The historical merge remains non-DCO and visible; the attestation supplies no
retroactive compliance claim.

The [PR #140 measurement catalog attestation](attestations/measurement-catalog-pr140-merge.json)
records the independently reviewed v1.2 catalog publication. The reviewed head
had twelve successful exact-head checks, and the GitHub-verified merge preserved
its exact tree. All seven protected-main workflow outcomes are retained,
including successful formal lockfile qualification and the Repository quality
and Rust verification failures. The merge author is
`martin-beck <martin.beck2@gmx.de>`, but its raw trailer names `Martin Beck`, so
the author-sensitive DCO boundary correctly rejected it. This attestation does
not alter the catalog, rewrite the merge, or make that historical commit
compliant.

For a GitHub-created merge with that author identity, the real multiline merge
message must end with the exact, case-sensitive trailer:

```text
Signed-off-by: martin-beck <martin.beck2@gmx.de>
```

This merge recipe does not change local implementation identity: locally
authored implementation commits remain signed by and carry the matching trailer
for `Martin Beck <martin.beck2@gmx.de>`.

Publication uses `gh pr merge <pr> --merge --subject <subject> --body
"$merge_body"`, where `merge_body` contains actual newline bytes and ends with
the exact lowercase trailer above. Default merge messages, rebase, squash,
escaped-literal newlines, and a title-cased `Martin Beck` trailer do not satisfy
the protected publication boundary.

The attestation is bounded and validated by the `asb-protocol` integration
suite. Validation binds it to the checked-in catalog fixture and rejects
unknown fields, malformed identities, false signature or DCO claims, incomplete
check evidence, reviewed/published tree mismatch, or widening either recovery
boundary.
