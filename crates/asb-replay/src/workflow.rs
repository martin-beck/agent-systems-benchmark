// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Explicit, privacy-safe recording and replay workflow contracts.

use crate::{
    Cassette, CassetteContents, CassetteLimits, RecordingIndex, RedactionPolicy, RedactionReport,
    Redactor, SourceChoice, SourceSelectionError, seal_cassette,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Version of the user-facing recording workflow envelope.
pub const RECORDING_WORKFLOW_SCHEMA_VERSION: u16 = 1;
/// Maximum public agent identity length accepted by the workflow.
pub const MAX_WORKFLOW_AGENT_BYTES: usize = 128;

/// Network consequence disclosed before a recording is started.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkConsequence {
    /// The capture will contact the configured provider.
    Provider,
    /// The capture is confined to a credential-free loopback fixture.
    LoopbackOnly,
    /// No provider connection is permitted.
    Denied,
}

/// Explicit acknowledgements required before recording can have effects.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecordingConfirmation {
    /// The user opted into creating a persistent recording.
    pub record: bool,
    /// The user acknowledged the disclosed network consequence.
    pub network: bool,
    /// The user acknowledged the estimated provider cost.
    pub cost: bool,
}

/// A bounded raw capture handed to the redaction boundary.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecordingCapture {
    /// Workflow schema version.
    pub schema_version: u16,
    /// Credential-free provider profile identity.
    pub provider_profile_sha256: String,
    /// Selected agent identity.
    pub agent_id: String,
    /// Provider or fixture consequence shown before recording.
    pub network: NetworkConsequence,
    /// Upper-bound cost estimate in minor currency units.
    pub estimated_cost_minor: u64,
    /// Explicit user acknowledgement.
    pub confirmation: RecordingConfirmation,
    /// Raw in-memory capture. It is never returned in public workflow metadata.
    pub contents: CassetteContents,
}

/// Safe catalog metadata emitted after a successful recording.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecordingMetadata {
    /// Workflow schema version.
    pub schema_version: u16,
    /// Stable cassette identity.
    pub cassette_id: String,
    /// Authenticated cassette root digest.
    pub cassette_sha256: String,
    /// Provider profile identity.
    pub provider_profile_sha256: String,
    /// Selected agent identity.
    pub agent_id: String,
    /// Number of captured interactions.
    pub interactions: u32,
    /// Redaction policy result.
    pub redaction: RedactionReport,
    /// Network consequence that was acknowledged.
    pub network: NetworkConsequence,
    /// Whether the artifact is a replayable complete recording.
    pub complete: bool,
    /// Explicit source label for result presentation.
    pub source: &'static str,
}

/// A sealed recording and its non-sensitive catalog metadata.
#[derive(Clone, Debug, PartialEq)]
pub struct RecordedArtifact {
    /// Authenticated cassette used by strict replay.
    pub cassette: Cassette,
    /// Public metadata safe for catalogs and result labels.
    pub metadata: RecordingMetadata,
}

/// Record, redact, authenticate, and catalog one complete capture.
pub fn seal_recording(
    capture: RecordingCapture,
    policy: RedactionPolicy,
    limits: CassetteLimits,
) -> Result<RecordedArtifact, RecordingWorkflowError> {
    validate_capture(&capture)?;
    let agent_id = capture.agent_id.clone();
    let profile = capture.provider_profile_sha256.clone();
    let network = capture.network;
    let interactions = u32::try_from(capture.contents.interactions.len())
        .map_err(|_| RecordingWorkflowError::TooManyInteractions)?;
    let cassette_id = capture.contents.cassette_id.clone();
    let (redacted, report) = Redactor::new(policy)
        .map_err(RecordingWorkflowError::Redaction)?
        .redact_contents(capture.contents)
        .map_err(RecordingWorkflowError::Redaction)?;
    let encoded = seal_cassette(redacted, limits).map_err(RecordingWorkflowError::Cassette)?;
    let cassette =
        crate::decode_cassette(&encoded, limits).map_err(RecordingWorkflowError::Cassette)?;
    let metadata = RecordingMetadata {
        schema_version: RECORDING_WORKFLOW_SCHEMA_VERSION,
        cassette_id,
        cassette_sha256: cassette.integrity.digest.clone(),
        provider_profile_sha256: profile,
        agent_id,
        interactions,
        redaction: report,
        network,
        complete: true,
        source: "live_recording",
    };
    Ok(RecordedArtifact { cassette, metadata })
}

/// Resolve an explicit live-or-replay choice and expose only compatible offers.
pub fn choose_source(
    index: &RecordingIndex,
    provider_profile_sha256: &str,
    agent_id: &str,
    live_available: bool,
    choice: Option<&SourceChoice>,
) -> Result<crate::ExecutionSource, RecordingWorkflowError> {
    index
        .select(provider_profile_sha256, agent_id, live_available, choice)
        .map_err(RecordingWorkflowError::Selection)
}

fn validate_capture(capture: &RecordingCapture) -> Result<(), RecordingWorkflowError> {
    if capture.schema_version != RECORDING_WORKFLOW_SCHEMA_VERSION
        || capture.provider_profile_sha256.len() != 64
        || !capture
            .provider_profile_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        || capture.agent_id.is_empty()
        || capture.agent_id.len() > MAX_WORKFLOW_AGENT_BYTES
        || !capture
            .agent_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        || capture.contents.interactions.is_empty()
        || !capture.confirmation.record
        || !capture.confirmation.network
        || (capture.estimated_cost_minor > 0 && !capture.confirmation.cost)
        || matches!(capture.network, NetworkConsequence::Denied)
    {
        return Err(RecordingWorkflowError::ConfirmationRequired);
    }
    Ok(())
}

/// Recording workflow validation or source-selection failure.
#[derive(Debug, Error)]
pub enum RecordingWorkflowError {
    /// The capture did not carry complete explicit consent or bounded identity.
    #[error("recording requires explicit bounded consent and a complete capture")]
    ConfirmationRequired,
    /// The capture exceeded the interaction bound.
    #[error("recording contains too many interactions")]
    TooManyInteractions,
    /// Redaction rejected the capture.
    #[error("recording redaction failed")]
    Redaction(#[source] crate::RedactionError),
    /// Cassette validation or persistence failed.
    #[error("recording cassette could not be sealed")]
    Cassette(#[source] crate::CassetteError),
    /// Source choice was unavailable or ambiguous.
    #[error("recording source choice is unavailable")]
    Selection(#[source] SourceSelectionError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RecordingDescriptor, decode_cassette};

    fn capture() -> RecordingCapture {
        let cassette = decode_cassette(
            include_bytes!("../fixtures/v1/buffered.json"),
            CassetteLimits::default(),
        )
        .expect("fixture is valid");
        RecordingCapture {
            schema_version: RECORDING_WORKFLOW_SCHEMA_VERSION,
            provider_profile_sha256: "a".repeat(64),
            agent_id: "codex".into(),
            network: NetworkConsequence::LoopbackOnly,
            estimated_cost_minor: 0,
            confirmation: RecordingConfirmation {
                record: true,
                network: true,
                cost: false,
            },
            contents: cassette.contents,
        }
    }

    #[test]
    fn sealing_requires_consent_and_emits_only_safe_metadata() {
        let artifact = seal_recording(
            capture(),
            RedactionPolicy::default(),
            CassetteLimits::default(),
        )
        .expect("fixture records");
        assert!(artifact.metadata.complete);
        assert_eq!(artifact.metadata.source, "live_recording");
        assert_eq!(artifact.metadata.cassette_sha256.len(), 64);
        let public = serde_json::to_string(&artifact.metadata).unwrap();
        assert!(!public.contains("authorization"));
    }

    #[test]
    fn missing_record_acknowledgement_fails_closed() {
        let mut value = capture();
        value.confirmation.record = false;
        assert!(matches!(
            seal_recording(value, RedactionPolicy::default(), CassetteLimits::default()),
            Err(RecordingWorkflowError::ConfirmationRequired)
        ));
    }

    #[test]
    fn source_selection_requires_exact_compatible_digest() {
        let artifact = seal_recording(
            capture(),
            RedactionPolicy::default(),
            CassetteLimits::default(),
        )
        .unwrap();
        let mut index = RecordingIndex::new();
        index
            .insert(
                RecordingDescriptor {
                    provider_profile_sha256: "a".repeat(64),
                    agent_id: "codex".into(),
                    cassette_sha256: artifact.metadata.cassette_sha256.clone(),
                },
                &artifact.cassette,
            )
            .unwrap();
        assert!(
            choose_source(
                &index,
                &"a".repeat(64),
                "codex",
                false,
                Some(&SourceChoice::Replay {
                    cassette_sha256: artifact.metadata.cassette_sha256,
                }),
            )
            .is_ok()
        );
        assert!(
            choose_source(
                &index,
                &"b".repeat(64),
                "codex",
                false,
                Some(&SourceChoice::Replay {
                    cassette_sha256: "c".repeat(64),
                }),
            )
            .is_err()
        );
    }
}
