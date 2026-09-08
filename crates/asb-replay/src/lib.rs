// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Immutable, privacy-reviewed provider response cassettes.
//!
//! It also provides strict, inbound-only replay matching and immediate HTTP/SSE
//! delivery plus explicit, measured response pacing. Provider-client conformance
//! requires a later AR.

#![forbid(unsafe_code)]

mod cassette;
mod migration;
mod pacing;
mod redaction;
mod selection;
mod service;

pub use cassette::{
    CASSETTE_SCHEMA_VERSION, Cassette, CassetteContents, CassetteError, CassetteEvent,
    CassetteIntegrity, CassetteLimits, Header, Interaction, MAX_EVENTS, PolicyVersion,
    ProviderDialect, RecordedRequest, RecordedResponse, RedactedCassetteContents,
    RedactionDescriptor, RedactionSelectors, RequestBodyRedactionRule, ResponseBody, TerminalEvent,
    canonical_contents_bytes, canonical_json_bytes, decode_cassette, decode_cassette_chunks,
    seal_cassette,
};
pub use migration::{MigrationError, ReferenceGraph, verify_migration_references};
pub use pacing::{
    CalibrationLimits, CalibrationSample, CancellationToken, HeadroomAssessment, HeadroomVerdict,
    PacingClassification, PacingConfig, PacingError, PacingMode, PacingReport, SegmentTiming,
    SystemMonotonicClock, assess_replay_headroom, write_paced_segments,
};
pub use redaction::{
    DEFAULT_REDACTION_POLICY_VERSION, INTERACTION_REDACTION_POLICY_VERSION, RedactionError,
    RedactionPolicy, RedactionReport, Redactor,
};
pub use selection::{
    ExecutionSource, RecordingDescriptor, RecordingIndex, RecordingOffer, SourceChoice,
    SourceSelectionError,
};
pub use service::{
    DialectCapability, MAX_IO_TIMEOUT, ReplayDeliveryReport, ReplayError, ReplayHttpRequest,
    ReplayHttpResponse, ReplayLimits, ReplayRoute, StrictReplayService, dialect_capabilities,
};
