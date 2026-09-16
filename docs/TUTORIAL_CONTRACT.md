# Offline tutorial contract

ASB tutorials use the versioned JSON Schema in `tools/tutorials/schema/v1` and the matching
command metadata in `tools/tutorials`. The validator checks
argument-array syntax, option ordering, bounded repository-relative references, expected output
shapes, and stable step identifiers. It does not execute ASB, start a provider, open a network
connection, read credentials, or invoke an LLM.

Validate a tutorial offline with:

```sh
python3 tools/tutorials/validate.py tools/tutorials/example-v1.json
```

Each step has an `id`, an argument-array `command`, and an `expect` object. Steps must declare
`network: "denied"` and `credentials: "none"`; omitted values use those safe defaults. References
are repository-relative and bounded. Unknown fields, commands, options, reordered options,
secret-shaped values, shell syntax, absolute/private paths, and malformed contracts fail closed.

The checked-in command metadata is versioned with the contract. When the CLI grammar changes, the
metadata and its validator tests must change in the same review. The validator is syntax-only: a
successful result does not claim that a tutorial command or provider actually ran.
