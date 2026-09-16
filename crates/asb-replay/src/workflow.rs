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

/// Maximum recording tuples admitted by one campaign.
pub const MAX_RECORDING_CAMPAIGN_TUPLES: usize = 256;

/// One deterministic provider/workload recording tuple.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecordingTuple {
    /// Provider profile identity.
    pub provider_profile_sha256: String,
    /// Agent adapter identity.
    pub agent_id: String,
    /// Workload identity and revision.
    pub workload_id: String,
    /// Scorer revision used for the tuple.
    pub scorer_revision: String,
    /// Stable attempt identity used to prevent duplicate paid work.
    pub attempt_id: String,
    /// Bounded upper-bound cost estimate in minor currency units.
    pub estimated_cost_minor: u64,
}

/// Durable coverage state for one recording tuple.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordingCoverageState {
    /// No recording attempt has started.
    Ready,
    /// A recording attempt is active or needs reconciliation.
    InProgress,
    /// A complete authenticated cassette is available.
    Complete,
    /// Recording failed and requires a new attempt.
    Failed,
    /// The prior capture no longer matches current identity.
    Stale,
}

/// Bounded durable coverage entry without captured content.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecordingCoverage {
    /// Tuple identity.
    pub tuple: RecordingTuple,
    /// Current durable state.
    pub state: RecordingCoverageState,
}

impl RecordingCoverage {
    /// Apply one lifecycle transition, rejecting duplicate or regressive work.
    pub fn transition(
        &mut self,
        next: RecordingCoverageState,
    ) -> Result<(), RecordingWorkflowError> {
        let allowed = matches!(
            (self.state, next),
            (
                RecordingCoverageState::Ready,
                RecordingCoverageState::InProgress
            ) | (
                RecordingCoverageState::InProgress,
                RecordingCoverageState::Complete
            ) | (
                RecordingCoverageState::InProgress,
                RecordingCoverageState::Failed
            ) | (
                RecordingCoverageState::InProgress,
                RecordingCoverageState::Stale
            )
        );
        if allowed {
            self.state = next;
            Ok(())
        } else {
            Err(RecordingWorkflowError::CoverageTransition)
        }
    }

    /// Reconcile an interrupted attempt without repeating uncertain provider work.
    pub fn reconcile_after_restart(&mut self) -> Result<(), RecordingWorkflowError> {
        if self.state == RecordingCoverageState::InProgress {
            self.state = RecordingCoverageState::Stale;
            Ok(())
        } else {
            Err(RecordingWorkflowError::CoverageTransition)
        }
    }
}

/// Explicit bounded matrix of recording work.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecordingCampaign {
    /// Campaign contract version.
    pub schema_version: u16,
    /// Provider profile identity shared by the campaign.
    pub provider_profile_sha256: String,
    /// Selected adapter identities.
    pub agent_ids: Vec<String>,
    /// Selected workload/scorer pairs.
    pub workloads: Vec<(String, String)>,
    /// Maximum tuples admitted for this campaign.
    pub max_tuples: u16,
    /// Upper-bound cost estimate for each tuple in minor currency units.
    pub cost_per_tuple_minor: u64,
}

impl RecordingCampaign {
    /// Expand the campaign deterministically, rejecting unsafe or oversized matrices.
    pub fn expand(&self) -> Result<Vec<RecordingTuple>, RecordingWorkflowError> {
        if self.schema_version != RECORDING_WORKFLOW_SCHEMA_VERSION
            || self.max_tuples == 0
            || usize::from(self.max_tuples) > MAX_RECORDING_CAMPAIGN_TUPLES
            || self.agent_ids.is_empty()
            || self.workloads.is_empty()
            || !valid_digest(&self.provider_profile_sha256)
        {
            return Err(RecordingWorkflowError::CampaignInvalid);
        }
        let count = self
            .agent_ids
            .len()
            .checked_mul(self.workloads.len())
            .ok_or(RecordingWorkflowError::CampaignTooLarge)?;
        if count > usize::from(self.max_tuples) || count > MAX_RECORDING_CAMPAIGN_TUPLES {
            return Err(RecordingWorkflowError::CampaignTooLarge);
        }
        let mut tuples = Vec::with_capacity(count);
        for agent_id in &self.agent_ids {
            if !valid_identity(agent_id) {
                return Err(RecordingWorkflowError::CampaignInvalid);
            }
            for (workload_id, scorer_revision) in &self.workloads {
                if !valid_identity(workload_id) || !valid_identity(scorer_revision) {
                    return Err(RecordingWorkflowError::CampaignInvalid);
                }
                tuples.push(RecordingTuple {
                    provider_profile_sha256: self.provider_profile_sha256.clone(),
                    agent_id: agent_id.clone(),
                    workload_id: workload_id.clone(),
                    scorer_revision: scorer_revision.clone(),
                    attempt_id: format!("{}-{}-attempt", agent_id, workload_id),
                    estimated_cost_minor: self.cost_per_tuple_minor,
                });
            }
        }
        Ok(tuples)
    }

    /// Execute each tuple once, persisting only bounded coverage metadata.
    pub fn execute<F, C>(
        &self,
        mut capture: F,
        mut cancelled: C,
    ) -> Result<Vec<RecordingCoverage>, RecordingWorkflowError>
    where
        F: FnMut(&RecordingTuple) -> Result<RecordingCoverageState, RecordingWorkflowError>,
        C: FnMut() -> bool,
    {
        let tuples = self.expand()?;
        let mut coverage = Vec::with_capacity(tuples.len());
        for tuple in tuples {
            if cancelled() {
                break;
            }
            let mut entry = RecordingCoverage {
                tuple,
                state: RecordingCoverageState::Ready,
            };
            entry.transition(RecordingCoverageState::InProgress)?;
            let terminal = capture(&entry.tuple)?;
            entry.transition(terminal)?;
            coverage.push(entry);
        }
        Ok(coverage)
    }
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
    /// The campaign matrix is malformed or lacks required bounds.
    #[error("recording campaign is invalid")]
    CampaignInvalid,
    /// The campaign matrix exceeds its tuple bound.
    #[error("recording campaign exceeds its tuple bound")]
    CampaignTooLarge,
    /// A coverage transition would repeat or regress durable work.
    #[error("recording coverage transition is invalid")]
    CoverageTransition,
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn valid_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_WORKFLOW_AGENT_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RecordingDescriptor, decode_cassette};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

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
    fn campaign_expansion_is_deterministic_and_bounded() {
        let campaign = RecordingCampaign {
            schema_version: RECORDING_WORKFLOW_SCHEMA_VERSION,
            provider_profile_sha256: "a".repeat(64),
            agent_ids: vec!["codex".into(), "aider".into()],
            workloads: vec![("bug-fix".into(), "scorer-v1".into())],
            max_tuples: 2,
            cost_per_tuple_minor: 7,
        };
        let tuples = campaign.expand().unwrap();
        assert_eq!(tuples.len(), 2);
        assert_eq!(tuples[0].agent_id, "codex");
        assert_eq!(tuples[1].agent_id, "aider");
    }

    #[test]
    fn campaign_rejects_oversized_and_invalid_matrices() {
        let mut campaign = RecordingCampaign {
            schema_version: RECORDING_WORKFLOW_SCHEMA_VERSION,
            provider_profile_sha256: "a".repeat(64),
            agent_ids: vec!["codex".into(), "aider".into()],
            workloads: vec![("unsafe id".into(), "v1".into())],
            max_tuples: 4,
            cost_per_tuple_minor: 7,
        };
        assert!(matches!(
            campaign.expand(),
            Err(RecordingWorkflowError::CampaignInvalid)
        ));
        campaign.workloads = vec![("bug-fix".into(), "scorer-v1".into())];
        campaign.max_tuples = 1;
        assert!(matches!(
            campaign.expand(),
            Err(RecordingWorkflowError::CampaignTooLarge)
        ));
    }

    #[test]
    fn coverage_transitions_are_single_use_and_fail_closed() {
        let tuple = RecordingCampaign {
            schema_version: 1,
            provider_profile_sha256: "a".repeat(64),
            agent_ids: vec!["codex".into()],
            workloads: vec![("bug-fix".into(), "v1".into())],
            max_tuples: 1,
            cost_per_tuple_minor: 7,
        }
        .expand()
        .unwrap()
        .remove(0);
        let mut coverage = RecordingCoverage {
            tuple,
            state: RecordingCoverageState::Ready,
        };
        coverage
            .transition(RecordingCoverageState::InProgress)
            .unwrap();
        coverage
            .transition(RecordingCoverageState::Complete)
            .unwrap();
        assert!(matches!(
            coverage.transition(RecordingCoverageState::InProgress),
            Err(RecordingWorkflowError::CoverageTransition)
        ));
        assert!(matches!(
            coverage.reconcile_after_restart(),
            Err(RecordingWorkflowError::CoverageTransition)
        ));
    }

    #[test]
    fn interrupted_attempt_becomes_stale_and_is_not_replayed() {
        let tuple = RecordingCampaign {
            schema_version: 1,
            provider_profile_sha256: "a".repeat(64),
            agent_ids: vec!["codex".into()],
            workloads: vec![("bug-fix".into(), "v1".into())],
            max_tuples: 1,
            cost_per_tuple_minor: 7,
        }
        .expand()
        .unwrap()
        .remove(0);
        let mut coverage = RecordingCoverage {
            tuple,
            state: RecordingCoverageState::Ready,
        };
        coverage
            .transition(RecordingCoverageState::InProgress)
            .unwrap();
        coverage.reconcile_after_restart().unwrap();
        assert_eq!(coverage.state, RecordingCoverageState::Stale);
        assert!(
            coverage
                .transition(RecordingCoverageState::InProgress)
                .is_err()
        );
    }

    #[test]
    fn campaign_executor_is_bounded_costed_and_cancellable() {
        let campaign = RecordingCampaign {
            schema_version: 1,
            provider_profile_sha256: "a".repeat(64),
            agent_ids: vec!["codex".into(), "aider".into()],
            workloads: vec![("bug-fix".into(), "v1".into())],
            max_tuples: 2,
            cost_per_tuple_minor: 9,
        };
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_for_capture = Arc::clone(&calls);
        let calls_for_cancel = Arc::clone(&calls);
        let result = campaign
            .execute(
                |_| {
                    calls_for_capture.fetch_add(1, Ordering::Relaxed);
                    Ok(RecordingCoverageState::Complete)
                },
                || calls_for_cancel.load(Ordering::Relaxed) > 0,
            )
            .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].tuple.estimated_cost_minor, 9);
        assert_eq!(result[0].state, RecordingCoverageState::Complete);
    }

    #[test]
    fn buffered_and_stream_captures_emit_only_redacted_metadata() {
        for fixture in [
            include_bytes!("../fixtures/v1/buffered.json").as_slice(),
            include_bytes!("../fixtures/v1/events.json").as_slice(),
        ] {
            let cassette = decode_cassette(fixture, CassetteLimits::default()).unwrap();
            let capture = RecordingCapture {
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
            };
            let artifact = seal_recording(
                capture,
                RedactionPolicy::default(),
                CassetteLimits::default(),
            )
            .unwrap();
            let metadata = serde_json::to_string(&artifact.metadata).unwrap();
            assert!(!metadata.contains("authorization"));
            assert!(artifact.metadata.complete);
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
