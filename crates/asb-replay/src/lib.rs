// SPDX-License-Identifier: MIT
//! Immutable, privacy-reviewed provider response cassettes.
//!
//! It also provides strict, inbound-only replay matching and immediate HTTP/SSE
//! delivery. Provider-client conformance and paced delivery require later ARs.

#![forbid(unsafe_code)]

mod cassette;
mod migration;
mod redaction;
mod service;

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
pub use service::{
    DialectCapability, MAX_IO_TIMEOUT, ReplayError, ReplayHttpRequest, ReplayHttpResponse,
    ReplayLimits, ReplayRoute, StrictReplayService, dialect_capabilities,
};
