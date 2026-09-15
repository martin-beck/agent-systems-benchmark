# Agent-catalog content digest (v1.4)

`AgentCatalog.catalog_sha256` is the content identity used by independent
frontends, including `asb-tui`. SHA-256 provides content integrity: it detects
changes to the snapshot but does not authenticate its origin or authorize a
package. It is not a signature and does not replace verification of the
detached package-signature digests in each entry; those signatures provide the
package authenticity check.

## Exact algorithm

1. Decode the response into the typed `AgentCatalog` v1.4 value and run its
   fail-closed validation. Do not sort, trim, case-fold, repair, or otherwise
   normalize an invalid response.
2. Serialize the snapshot object formed by all `AgentCatalog` members except
   `catalog_sha256`: `runner_instance_id`, `generation`, `target`, `agents`,
   and `refreshed`.
3. Recursively sort every JSON object member by its UTF-8 bytewise key order.
   This makes the order of Rust/JSON object members irrelevant. Arrays are not
   sorted: `agents` must already be strictly increasing by `agent_id`, and each
   `capabilities` array must already be strictly increasing by UTF-8 bytewise
   order. Duplicate IDs or
   capabilities are rejected by validation rather than deduplicated.
4. Encode strings as UTF-8 JSON, use the typed integer JSON representation,
   emit no insignificant whitespace, and use the JSON escaping produced by the
   ASB serde implementation. v1.4 has no floating-point values, so there is no
   floating-point normalization rule.
5. Hash those exact bytes with SHA-256 and lowercase the 32-byte digest as 64
   ASCII hexadecimal characters. The result must equal `catalog_sha256`.

The canonical bytes are therefore the RFC-8259 JSON object with recursively
byte-sorted keys and compact separators. The digest excludes only the digest
member itself; changing any other member, array order, value, target, runner
identity, generation, or refresh bit changes the identity. The runner must
compute the digest after constructing the complete snapshot and before
publication; a frontend must recompute it before using the catalog.

## Vectors

For a one-entry snapshot with runner `runner-1`, generation `1`, Linux
`x86_64`/glibc `2.35`, package `agent-package` version `1.2.3`, capabilities
`["chat","tools"]`, and the `a`/`b`/`c`/`d` placeholder digests from the Rust
unit fixture, the canonical bytes are:

```text
{"agents":[{"agent_id":"agent-a","availability":{"status":"available"},"capabilities":["chat","tools"],"package":{"package_id":"agent-package","sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","signature_sha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","version":"1.2.3"},"provenance":{"manifest_sha256":"dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd","source_revision":"cccccccccccccccccccccccccccccccccccccccc"},"target":{"architecture":"x86_64","libc":"glibc","libc_version":"2.35","operating_system":"linux"}}],"generation":1,"refreshed":false,"runner_instance_id":"runner-1","target":{"architecture":"x86_64","libc":"glibc","libc_version":"2.35","operating_system":"linux"}}
```

Its digest is
`cba97a13d8123b0d24c381174cd26a35fcfb64d24d34cfaa8549bec8ea578520`.

The checked-in v1.4 response fixture uses generation `3` and has digest
`e24007b09d9f6b112269689a535ba00968300c9653d71e349f37835d0d8c5e92`.
The executable `canonical_agent_catalog_bytes` and tests are the normative
implementation and regression vectors for both repositories.

## Rejection behavior

Unknown fields, malformed JSON, invalid identities/digests, mismatched entry
targets, duplicate or unsorted agent IDs, duplicate capabilities, and a
`catalog_sha256` that does not equal the recomputed digest are invalid. A
consumer must not attempt to hash a partially decoded object or silently use
an unverified catalog. The outer JSON-RPC/request identity remains separately
validated and is not included in this catalog content digest.
