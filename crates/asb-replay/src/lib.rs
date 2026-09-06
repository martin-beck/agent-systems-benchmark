// SPDX-License-Identifier: MIT
//! Immutable, privacy-reviewed provider response cassettes.
//!
//! This crate defines the storage boundary only. Network serving, matching,
//! playback cursors, provider compatibility, and pacing belong to later ARs.

#![forbid(unsafe_code)]

mod cassette;
mod migration;
mod redaction;

pub use cassette::{
    CASSETTE_SCHEMA_VERSION, Cassette, CassetteContents, CassetteError, CassetteEvent,
    CassetteIntegrity, CassetteLimits, Header, Interaction, PolicyVersion, ProviderDialect,
    RecordedRequest, RecordedResponse, RedactedCassetteContents, RedactionDescriptor,
    RedactionSelectors, ResponseBody, TerminalEvent, canonical_contents_bytes,
    canonical_json_bytes, decode_cassette, decode_cassette_chunks, seal_cassette,
};
pub use migration::{MigrationError, ReferenceGraph, verify_migration_references};
pub use redaction::{
    DEFAULT_REDACTION_POLICY_VERSION, RedactionError, RedactionPolicy, RedactionReport, Redactor,
};
