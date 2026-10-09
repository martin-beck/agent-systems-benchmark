# Human diagnostic completeness

Every public ASB failure and warning must preserve a machine-compatible error
envelope while providing a specific, privacy-safe human explanation.  The
typed catalogue in `crates/asb-cli/src/diagnostic.rs` is the public authority
for cause, safe subject, state change, and recovery.

When adding a public diagnostic:

1. Add a stable producer identity and a non-fallback `Cause` mapping to
   `CATALOGUED_CODES` and `classify`.
2. Give it a concrete safe subject, operation, state-change result, and
   remediation.  Do not pass a host path, operating-system error, credential,
   prompt, provider payload, or arbitrary subprocess text into the catalogue.
3. Add a focused positive test and a negative fixture where applicable.  A
   routed TUI `RouterError::policy` or `RouterError::operation` producer is
   mechanically checked against the catalogue by
   `crates/asb-cli/tests/diagnostic_contract.rs`.
4. For a warning, add a consequence to `warning_text` in `human.rs`; a warning
   without an operator-visible limitation is not sufficient.
5. Add an invalid, side-effect-free case to `diagnostic_journey.rs` for a new
   top-level command. Run `cargo test --locked -p asb-cli --test
   diagnostic_contract` and `cargo test --locked -p asb-cli --test
   diagnostic_journey`; also run `cargo test --locked -p asb-cli
   every_public_command_has_a_human_safe_invalid_argument_journey`. Repository
   quality runs all required gates on pull requests and protected main.

Unknown producer identities are intentionally rendered as unclassified.  They
are safe fallback behavior for compatibility, not permission to add a new
public producer without the reviewed catalogue entry and tests.
