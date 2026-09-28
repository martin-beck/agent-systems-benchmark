// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Deterministic, provider-free development qualification.
//!
//! This module is deliberately a fixture boundary.  It connects the
//! development enrollment projection to a provider/model choice, produces
//! bounded local mock traffic, and hands that traffic to the existing
//! authenticated cassette and strict replay contracts.  It never reads a
//! credential, starts a process, opens a socket, or contacts a provider.

use asb_config::{
    DEVELOPMENT_ENROLLMENT_SCHEMA_VERSION, DevelopmentCredentialStore, DevelopmentEnrollmentStatus,
};
use asb_replay::{
    Cassette, CassetteLimits, NetworkConsequence, ProviderCaptureExchange, ProviderDialect,
    RecordingCapture, RecordingConfirmation, ReplayHttpRequest, ReplayHttpResponse, ReplayLimits,
    ReplayRoute, StrictReplayService, seal_recording,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::time::Duration;
use thiserror::Error;

/// Version of the development provider fixture contract.
pub const DEVELOPMENT_FIXTURE_SCHEMA_VERSION: u16 = 1;
/// Stable default seed for the checked-in local fixture.
pub const DEVELOPMENT_FIXTURE_SEED: &str = "asb-development-fixture-v1";
const MAX_TEXT_BYTES: usize = 128;
const MAX_AGENTS: usize = 32;
const MAX_CAPTURES: usize = 256;
const MAX_FIXTURE_STATE_BYTES: usize = 1024 * 1024;

/// Provider/model and all-agent application selected by the development setup.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DevelopmentProviderSelection {
    /// Fixture contract version.
    pub schema_version: u16,
    /// Provider identifier bound by enrollment.
    pub provider_id: String,
    /// Model identifier bound by enrollment.
    pub model_id: String,
    /// Enrollment generation bound to this choice.
    pub generation: u64,
    /// Stable selected agent identities.
    pub agent_ids: Vec<String>,
    /// Apply this provider/model choice to every selected agent.
    pub apply_to_all: bool,
    /// Whether this is the setup default selection.
    pub default_selection: bool,
}

/// Public receipt binding a fixture seed to one enrolled identity.
///
/// It contains only digests and stable identifiers.  It is not a production
/// credential, signature, or trust decision.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DevelopmentCredentialReceipt {
    /// Fixture contract version.
    pub schema_version: u16,
    /// Provider identity from enrollment.
    pub provider_id: String,
    /// Model identity from enrollment.
    pub model_id: String,
    /// Enrollment generation that issued this receipt.
    pub generation: u64,
    /// Digest of the explicit fixture seed.
    pub fixture_seed_sha256: String,
    /// Public development key identity from enrollment.
    pub key_id: String,
    /// Generated public-key fixture digest.
    pub public_key_sha256: String,
    /// Self-signature digest issued by the AR-1499 enrollment store.
    pub enrollment_signature_sha256: String,
    /// Deterministic fixture signature digest.
    pub signature_sha256: String,
}

/// A bounded provider request handled by the local fixture.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DevelopmentCaptureRequest {
    /// Retry/idempotency identity.
    pub idempotency_key: String,
    /// Stable session identity.
    pub session_id: String,
    /// Stable attempt identity.
    pub attempt_id: String,
    /// Selected built-in agent identity.
    pub agent_id: String,
    /// Digest of the prompt; prompt contents never enter the fixture.
    pub prompt_sha256: String,
    /// Enrollment receipt presented by the caller.
    pub receipt: DevelopmentCredentialReceipt,
}

/// Safe result of one local mock capture.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DevelopmentCaptureResult {
    /// Fixture contract version.
    pub schema_version: u16,
    /// Agent that produced the capture.
    pub agent_id: String,
    /// Authenticated cassette digest.
    pub cassette_sha256: String,
    /// Whether this was an idempotent retry of a previous capture.
    pub replayed: bool,
    /// Warnings are explicitly non-blocking in development mode.
    pub warnings: Vec<String>,
}

/// Lifecycle state of the fixture coordinator.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DevelopmentFixtureState {
    /// Ready to capture a local fixture exchange.
    Ready,
    /// A capture has been admitted and must be reconciled after interruption.
    Capturing,
    /// At least one complete capture is available.
    Captured,
    /// The current capture was cancelled before completion.
    Cancelled,
    /// The last capture failed and requires explicit recovery.
    Failed,
}

/// Readiness of a selected or all-agent comparison campaign.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DevelopmentReadiness {
    /// True only when every selected agent has a complete cassette.
    pub ready: bool,
    /// Selected agents without a complete cassette.
    pub missing_agents: Vec<String>,
    /// Authentication/signature/key-management warnings never block this flag.
    pub warnings: Vec<String>,
}

/// Restart-safe local provider fixture bound to one development enrollment.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DevelopmentProviderFixture {
    schema_version: u16,
    store: DevelopmentCredentialStore,
    selection: DevelopmentProviderSelection,
    fixture_seed: String,
    state: DevelopmentFixtureState,
    captures: BTreeMap<String, DevelopmentStoredCapture>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DevelopmentStoredCapture {
    request_sha256: String,
    cassette: Cassette,
}

/// Fixture errors are typed and never contain prompts, credentials, or paths.
#[derive(Debug, Error, Eq, PartialEq)]
pub enum DevelopmentFixtureError {
    /// The fixture contract version is unsupported.
    #[error("unsupported development fixture version")]
    UnsupportedVersion,
    /// The enrollment has no active generated identity.
    #[error("development enrollment is not ready")]
    NotEnrolled,
    /// The receipt is malformed or does not bind to the active enrollment.
    #[error("development credential receipt is invalid")]
    InvalidReceipt,
    /// The receipt belongs to an older enrollment generation.
    #[error("development credential receipt is stale")]
    StaleGeneration,
    /// Provider/model selection differs from enrollment.
    #[error("development provider selection does not match enrollment")]
    SelectionMismatch,
    /// Selection or capture request is outside bounded contract limits.
    #[error("development fixture request is invalid")]
    InvalidRequest,
    /// Same idempotency key was used for different input.
    #[error("development fixture idempotency key conflicts")]
    IdempotencyConflict,
    /// No capture is active for cancellation or recovery.
    #[error("development fixture lifecycle transition is invalid")]
    InvalidTransition,
    /// The capture was explicitly cancelled.
    #[error("development fixture capture cancelled")]
    Cancelled,
    /// Existing cassette or state failed validation.
    #[error("development fixture state is invalid")]
    InvalidState,
    /// Existing cassette could not be used for strict replay.
    #[error("development fixture replay is unavailable")]
    ReplayUnavailable,
    /// Recording/redaction/cassette qualification failed.
    #[error("development fixture recording failed")]
    RecordingFailed,
    /// State encoding failed.
    #[error("development fixture state encoding failed")]
    Encoding,
    /// Restart state exceeded the bounded persistence contract.
    #[error("development fixture state exceeds its bound")]
    StateTooLarge,
}

impl DevelopmentProviderFixture {
    /// Construct a default all-agent fixture from an enrolled store.
    pub fn default_for_all(
        store: DevelopmentCredentialStore,
        fixture_seed: impl Into<String>,
        agent_ids: Vec<String>,
    ) -> Result<Self, DevelopmentFixtureError> {
        let status = store.status();
        let selection = DevelopmentProviderSelection {
            schema_version: DEVELOPMENT_FIXTURE_SCHEMA_VERSION,
            provider_id: status.provider_id.clone(),
            model_id: status.model_id.clone(),
            generation: status.generation,
            agent_ids,
            apply_to_all: true,
            default_selection: true,
        };
        Self::new(store, fixture_seed, selection)
    }

    /// Construct a fixture with an explicit provider/model and agent selection.
    pub fn new(
        store: DevelopmentCredentialStore,
        fixture_seed: impl Into<String>,
        selection: DevelopmentProviderSelection,
    ) -> Result<Self, DevelopmentFixtureError> {
        let fixture_seed = fixture_seed.into();
        validate_selection(&selection, &fixture_seed)?;
        if store.schema_version != DEVELOPMENT_ENROLLMENT_SCHEMA_VERSION {
            return Err(DevelopmentFixtureError::UnsupportedVersion);
        }
        let status = store.status();
        if status.status == DevelopmentEnrollmentStatus::Unenrolled
            || status.status == DevelopmentEnrollmentStatus::Reset
            || status.identity.is_none()
        {
            return Err(DevelopmentFixtureError::NotEnrolled);
        }
        if status.provider_id != selection.provider_id
            || status.model_id != selection.model_id
            || status.generation != selection.generation
        {
            return Err(DevelopmentFixtureError::SelectionMismatch);
        }
        Ok(Self {
            schema_version: DEVELOPMENT_FIXTURE_SCHEMA_VERSION,
            store,
            selection,
            fixture_seed,
            state: DevelopmentFixtureState::Ready,
            captures: BTreeMap::new(),
        })
    }

    /// Return the current all-agent/default selection without private state.
    pub fn selection(&self) -> &DevelopmentProviderSelection {
        &self.selection
    }

    /// Return the current lifecycle state.
    pub fn state(&self) -> DevelopmentFixtureState {
        self.state
    }

    /// Return explicit non-blocking development warnings.
    pub fn warnings(&self) -> Vec<String> {
        vec![
            "development-only: authentication service unavailable; local fixture used".into(),
            "development-only: signature validation unavailable; fixture digest only".into(),
            "development-only: key management unavailable; generated identity is not production trust".into(),
        ]
    }

    /// Issue the deterministic public receipt for the current enrollment.
    pub fn receipt(&self) -> Result<DevelopmentCredentialReceipt, DevelopmentFixtureError> {
        let identity = self
            .store
            .status()
            .identity
            .as_ref()
            .ok_or(DevelopmentFixtureError::NotEnrolled)?;
        let seed_digest = digest(self.fixture_seed.as_bytes());
        let signature_sha256 = digest(
            format!(
                "asb-development-signature-v1\0{}\0{}\0{}\0{}\0{}",
                self.fixture_seed,
                self.selection.provider_id,
                self.selection.model_id,
                self.selection.generation,
                identity.public_key_sha256,
            )
            .as_bytes(),
        );
        Ok(DevelopmentCredentialReceipt {
            schema_version: DEVELOPMENT_FIXTURE_SCHEMA_VERSION,
            provider_id: self.selection.provider_id.clone(),
            model_id: self.selection.model_id.clone(),
            generation: self.selection.generation,
            fixture_seed_sha256: seed_digest,
            key_id: identity.key_id.clone(),
            public_key_sha256: identity.public_key_sha256.clone(),
            enrollment_signature_sha256: identity.signature_sha256.clone(),
            signature_sha256,
        })
    }

    /// Apply a new complete agent set while retaining the enrolled provider/model.
    pub fn apply_to_all_defaults(
        &mut self,
        agent_ids: Vec<String>,
    ) -> Result<(), DevelopmentFixtureError> {
        validate_agents(&agent_ids)?;
        let next = DevelopmentProviderSelection {
            schema_version: DEVELOPMENT_FIXTURE_SCHEMA_VERSION,
            provider_id: self.selection.provider_id.clone(),
            model_id: self.selection.model_id.clone(),
            generation: self.selection.generation,
            agent_ids,
            apply_to_all: true,
            default_selection: true,
        };
        self.selection = next;
        Ok(())
    }

    /// Capture one deterministic local exchange and seal it as a cassette.
    pub fn capture(
        &mut self,
        request: DevelopmentCaptureRequest,
    ) -> Result<DevelopmentCaptureResult, DevelopmentFixtureError> {
        if matches!(
            self.state,
            DevelopmentFixtureState::Capturing
                | DevelopmentFixtureState::Cancelled
                | DevelopmentFixtureState::Failed
        ) {
            return Err(DevelopmentFixtureError::InvalidTransition);
        }
        self.validate_capture_request(&request)?;
        let request_sha256 = request_digest(&request)?;
        if let Some(previous) = self.captures.get(&request.idempotency_key) {
            if previous.request_sha256 == request_sha256 {
                return Ok(self.result(&request.agent_id, &previous.cassette, true));
            }
            return Err(DevelopmentFixtureError::IdempotencyConflict);
        }
        if self.captures.len() >= MAX_CAPTURES {
            return Err(DevelopmentFixtureError::InvalidRequest);
        }
        self.state = DevelopmentFixtureState::Capturing;
        let cassette = match self.build_cassette(&request) {
            Ok(cassette) => cassette,
            Err(error) => {
                self.state = DevelopmentFixtureState::Failed;
                return Err(error);
            }
        };
        self.captures.insert(
            request.idempotency_key.clone(),
            DevelopmentStoredCapture {
                request_sha256,
                cassette: cassette.clone(),
            },
        );
        self.state = DevelopmentFixtureState::Captured;
        Ok(self.result(&request.agent_id, &cassette, false))
    }

    /// Cancel an admitted capture before it is persisted.
    pub fn cancel_capture(&mut self) -> Result<(), DevelopmentFixtureError> {
        if self.state != DevelopmentFixtureState::Capturing {
            return Err(DevelopmentFixtureError::InvalidTransition);
        }
        self.state = DevelopmentFixtureState::Cancelled;
        Ok(())
    }

    /// Reconcile an interrupted/cancelled fixture after process restart.
    pub fn recover_after_restart(&mut self) -> Result<(), DevelopmentFixtureError> {
        match self.state {
            DevelopmentFixtureState::Capturing
            | DevelopmentFixtureState::Cancelled
            | DevelopmentFixtureState::Failed => {
                self.state = if self.captures.is_empty() {
                    DevelopmentFixtureState::Ready
                } else {
                    DevelopmentFixtureState::Captured
                };
                Ok(())
            }
            _ => Err(DevelopmentFixtureError::InvalidTransition),
        }
    }

    /// Return strict replay service for one selected agent; no provider fallback exists.
    pub fn strict_replay_for(
        &self,
        agent_id: &str,
    ) -> Result<StrictReplayService, DevelopmentFixtureError> {
        let cassette = self
            .captures
            .values()
            .find(|capture| cassette_agent(&capture.cassette) == Some(agent_id))
            .map(|capture| capture.cassette.clone())
            .ok_or(DevelopmentFixtureError::ReplayUnavailable)?;
        StrictReplayService::new(cassette, ReplayLimits::default())
            .map_err(|_| DevelopmentFixtureError::ReplayUnavailable)
    }

    /// Check whether selected/all-agent comparison is ready from complete captures.
    pub fn comparison_readiness(&self) -> DevelopmentReadiness {
        let mut missing_agents = self
            .selection
            .agent_ids
            .iter()
            .filter(|agent| {
                !self
                    .captures
                    .values()
                    .any(|capture| cassette_agent(&capture.cassette) == Some(agent.as_str()))
            })
            .cloned()
            .collect::<Vec<_>>();
        missing_agents.sort();
        DevelopmentReadiness {
            ready: missing_agents.is_empty() && !self.captures.is_empty(),
            missing_agents,
            warnings: self.warnings(),
        }
    }

    /// Serialize bounded restart state without secrets or raw prompts.
    pub fn to_json(&self) -> Result<Vec<u8>, DevelopmentFixtureError> {
        let bytes = serde_json::to_vec(self).map_err(|_| DevelopmentFixtureError::Encoding)?;
        if bytes.len() > MAX_FIXTURE_STATE_BYTES {
            return Err(DevelopmentFixtureError::StateTooLarge);
        }
        Ok(bytes)
    }

    /// Restore and validate bounded restart state and all cassette integrity roots.
    pub fn from_json(bytes: &[u8]) -> Result<Self, DevelopmentFixtureError> {
        if bytes.is_empty() || bytes.len() > MAX_FIXTURE_STATE_BYTES {
            return Err(DevelopmentFixtureError::StateTooLarge);
        }
        let fixture: Self =
            serde_json::from_slice(bytes).map_err(|_| DevelopmentFixtureError::InvalidState)?;
        fixture.validate_state()?;
        Ok(fixture)
    }

    fn validate_capture_request(
        &self,
        request: &DevelopmentCaptureRequest,
    ) -> Result<(), DevelopmentFixtureError> {
        if !valid_id(&request.idempotency_key)
            || !valid_id(&request.session_id)
            || !valid_id(&request.attempt_id)
            || !valid_id(&request.agent_id)
            || !valid_digest(&request.prompt_sha256)
            || !self
                .selection
                .agent_ids
                .iter()
                .any(|agent| agent == &request.agent_id)
        {
            return Err(DevelopmentFixtureError::InvalidRequest);
        }
        self.validate_receipt(&request.receipt)
    }

    fn validate_receipt(
        &self,
        receipt: &DevelopmentCredentialReceipt,
    ) -> Result<(), DevelopmentFixtureError> {
        let expected = self.receipt()?;
        if receipt.schema_version != DEVELOPMENT_FIXTURE_SCHEMA_VERSION {
            return Err(DevelopmentFixtureError::InvalidReceipt);
        }
        if receipt.generation != expected.generation {
            return Err(DevelopmentFixtureError::StaleGeneration);
        }
        if receipt.provider_id != expected.provider_id || receipt.model_id != expected.model_id {
            return Err(DevelopmentFixtureError::SelectionMismatch);
        }
        if receipt != &expected {
            return Err(DevelopmentFixtureError::InvalidReceipt);
        }
        Ok(())
    }

    fn build_cassette(
        &self,
        request: &DevelopmentCaptureRequest,
    ) -> Result<Cassette, DevelopmentFixtureError> {
        let body = serde_json::to_vec(&json!({
            "model": self.selection.model_id,
            "messages": [{"role": "user", "content_digest": request.prompt_sha256}],
        }))
        .map_err(|_| DevelopmentFixtureError::RecordingFailed)?;
        let response = serde_json::to_vec(&json!({
            "id": format!("dev-response-{}", &request.attempt_id),
            "object": "chat.completion",
            "choices": [{"index": 0, "message": {"role": "assistant", "content": format!("development-fixture-response-{}", &self.receipt()?.fixture_seed_sha256[..16])}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2},
        }))
        .map_err(|_| DevelopmentFixtureError::RecordingFailed)?;
        let exchange = ProviderCaptureExchange {
            request: ReplayHttpRequest {
                method: "POST".into(),
                path: "/v1/chat/completions".into(),
                headers: Vec::new(),
                body,
            },
            response: ReplayHttpResponse {
                status: 200,
                headers: Vec::new(),
                segments: vec![response],
                recorded_offsets: vec![Duration::ZERO],
            },
        };
        let route = ReplayRoute {
            session_id: request.session_id.clone(),
            attempt_id: request.attempt_id.clone(),
            dialect: ProviderDialect::OpenaiChatCompletions,
        };
        let contents = exchange
            .into_cassette_contents(
                &route,
                format!(
                    "dev-agent-{}--capture-{}",
                    request.agent_id, request.idempotency_key
                ),
            )
            .map_err(|_| DevelopmentFixtureError::RecordingFailed)?;
        let capture = RecordingCapture {
            schema_version: asb_replay::RECORDING_WORKFLOW_SCHEMA_VERSION,
            provider_profile_sha256: digest(
                format!(
                    "{}\0{}\0{}",
                    self.selection.provider_id, self.selection.model_id, self.selection.generation
                )
                .as_bytes(),
            ),
            agent_id: request.agent_id.clone(),
            network: NetworkConsequence::LoopbackOnly,
            estimated_cost_minor: 0,
            confirmation: RecordingConfirmation {
                record: true,
                network: true,
                cost: false,
            },
            contents,
        };
        seal_recording(
            capture,
            asb_replay::RedactionPolicy::default(),
            CassetteLimits::default(),
        )
        .map(|artifact| artifact.cassette)
        .map_err(|_| DevelopmentFixtureError::RecordingFailed)
    }

    fn result(
        &self,
        agent_id: &str,
        cassette: &Cassette,
        replayed: bool,
    ) -> DevelopmentCaptureResult {
        DevelopmentCaptureResult {
            schema_version: DEVELOPMENT_FIXTURE_SCHEMA_VERSION,
            agent_id: agent_id.to_owned(),
            cassette_sha256: cassette.integrity.digest.clone(),
            replayed,
            warnings: self.warnings(),
        }
    }

    fn validate_state(&self) -> Result<(), DevelopmentFixtureError> {
        if self.schema_version != DEVELOPMENT_FIXTURE_SCHEMA_VERSION
            || self.store.schema_version != DEVELOPMENT_ENROLLMENT_SCHEMA_VERSION
        {
            return Err(DevelopmentFixtureError::UnsupportedVersion);
        }
        validate_selection(&self.selection, &self.fixture_seed)?;
        let status = self.store.status();
        if status.status == DevelopmentEnrollmentStatus::Unenrolled
            || status.status == DevelopmentEnrollmentStatus::Reset
            || status.identity.is_none()
            || status.provider_id != self.selection.provider_id
            || status.model_id != self.selection.model_id
            || status.generation != self.selection.generation
        {
            return Err(DevelopmentFixtureError::InvalidState);
        }
        if self.captures.len() > MAX_CAPTURES {
            return Err(DevelopmentFixtureError::InvalidState);
        }
        for (key, stored) in &self.captures {
            if !valid_id(key) || !valid_digest(&stored.request_sha256) {
                return Err(DevelopmentFixtureError::InvalidState);
            }
            let bytes = serde_json::to_vec(&stored.cassette)
                .map_err(|_| DevelopmentFixtureError::InvalidState)?;
            asb_replay::decode_cassette(&bytes, CassetteLimits::default())
                .map_err(|_| DevelopmentFixtureError::InvalidState)?;
        }
        Ok(())
    }
}

fn validate_selection(
    selection: &DevelopmentProviderSelection,
    fixture_seed: &str,
) -> Result<(), DevelopmentFixtureError> {
    if selection.schema_version != DEVELOPMENT_FIXTURE_SCHEMA_VERSION
        || selection.generation == 0
        || !valid_id(&selection.provider_id)
        || !valid_id(&selection.model_id)
        || !valid_seed(fixture_seed)
    {
        return Err(DevelopmentFixtureError::InvalidRequest);
    }
    validate_agents(&selection.agent_ids)
}

fn validate_agents(agent_ids: &[String]) -> Result<(), DevelopmentFixtureError> {
    if agent_ids.is_empty() || agent_ids.len() > MAX_AGENTS {
        return Err(DevelopmentFixtureError::InvalidRequest);
    }
    let mut sorted = agent_ids.to_vec();
    sorted.sort();
    sorted.dedup();
    if sorted.len() != agent_ids.len() || sorted.iter().any(|agent| !valid_id(agent)) {
        return Err(DevelopmentFixtureError::InvalidRequest);
    }
    Ok(())
}

fn request_digest(request: &DevelopmentCaptureRequest) -> Result<String, DevelopmentFixtureError> {
    serde_json::to_vec(request)
        .map(|bytes| digest(&bytes))
        .map_err(|_| DevelopmentFixtureError::Encoding)
}

fn cassette_agent(cassette: &Cassette) -> Option<&str> {
    cassette
        .contents
        .cassette_id
        .strip_prefix("dev-agent-")
        .and_then(|value| value.split_once("--capture-").map(|(agent, _)| agent))
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_TEXT_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn valid_seed(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_TEXT_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn digest(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use asb_config::{
        DEVELOPMENT_ENROLLMENT_SCHEMA_VERSION, DEVELOPMENT_MOCK_MODEL, DEVELOPMENT_MOCK_PROVIDER,
        DevelopmentAuthMethod, DevelopmentCredentialOperation, DevelopmentCredentialRequest,
        DevelopmentServiceAvailability,
    };

    fn enrolled() -> DevelopmentCredentialStore {
        let mut store = DevelopmentCredentialStore::new();
        store
            .apply(
                DevelopmentCredentialRequest {
                    schema_version: DEVELOPMENT_ENROLLMENT_SCHEMA_VERSION,
                    operation: DevelopmentCredentialOperation::Enroll,
                    provider_id: DEVELOPMENT_MOCK_PROVIDER.into(),
                    model_id: DEVELOPMENT_MOCK_MODEL.into(),
                    auth_method: DevelopmentAuthMethod::Generated,
                    idempotency_key: "enroll".into(),
                    expected_generation: 0,
                },
                DevelopmentServiceAvailability::default(),
            )
            .unwrap();
        store
    }

    fn request(
        fixture: &DevelopmentProviderFixture,
        key: &str,
        agent: &str,
    ) -> DevelopmentCaptureRequest {
        DevelopmentCaptureRequest {
            idempotency_key: key.into(),
            session_id: format!("session-{key}"),
            attempt_id: format!("attempt-{key}"),
            agent_id: agent.into(),
            prompt_sha256: "a".repeat(64),
            receipt: fixture.receipt().unwrap(),
        }
    }

    #[test]
    fn applies_one_default_selection_to_all_agents_and_records_cassettes() {
        let mut fixture = DevelopmentProviderFixture::default_for_all(
            enrolled(),
            DEVELOPMENT_FIXTURE_SEED,
            vec!["codex".into(), "aider".into()],
        )
        .unwrap();
        assert!(fixture.selection().apply_to_all);
        assert!(fixture.selection().default_selection);
        let first_request = request(&fixture, "one", "codex");
        let first = fixture.capture(first_request).unwrap();
        assert!(!first.replayed);
        assert!(!fixture.comparison_readiness().ready);
        let second_request = request(&fixture, "two", "aider");
        fixture.capture(second_request).unwrap();
        let readiness = fixture.comparison_readiness();
        assert!(readiness.ready);
        assert_eq!(readiness.missing_agents, Vec::<String>::new());
        assert_eq!(readiness.warnings.len(), 3);
    }

    #[test]
    fn retries_are_idempotent_and_conflicts_are_rejected() {
        let mut fixture = DevelopmentProviderFixture::default_for_all(
            enrolled(),
            DEVELOPMENT_FIXTURE_SEED,
            vec!["codex".into()],
        )
        .unwrap();
        let original = request(&fixture, "same", "codex");
        let first = fixture.capture(original.clone()).unwrap();
        let second = fixture.capture(original).unwrap();
        assert!(second.replayed);
        assert_eq!(first.cassette_sha256, second.cassette_sha256);
        let mut conflict = request(&fixture, "same", "codex");
        conflict.prompt_sha256 = "b".repeat(64);
        assert_eq!(
            fixture.capture(conflict),
            Err(DevelopmentFixtureError::IdempotencyConflict)
        );
    }

    #[test]
    fn stale_malformed_and_mismatched_receipts_fail_closed() {
        let mut fixture = DevelopmentProviderFixture::default_for_all(
            enrolled(),
            DEVELOPMENT_FIXTURE_SEED,
            vec!["codex".into()],
        )
        .unwrap();
        let mut malformed = request(&fixture, "malformed", "codex");
        malformed.receipt.signature_sha256 = "not-a-digest".into();
        assert_eq!(
            fixture.capture(malformed),
            Err(DevelopmentFixtureError::InvalidReceipt)
        );
        let mut enrollment_tampered = request(&fixture, "enrollment-tampered", "codex");
        enrollment_tampered.receipt.enrollment_signature_sha256 = "0".repeat(64);
        assert_eq!(
            fixture.capture(enrollment_tampered),
            Err(DevelopmentFixtureError::InvalidReceipt)
        );
        let mut wrong = request(&fixture, "wrong", "codex");
        wrong.receipt.model_id = "other-model".into();
        assert_eq!(
            fixture.capture(wrong),
            Err(DevelopmentFixtureError::SelectionMismatch)
        );
        let mut stale = request(&fixture, "stale", "codex");
        stale.receipt.generation = 99;
        assert_eq!(
            fixture.capture(stale),
            Err(DevelopmentFixtureError::StaleGeneration)
        );
    }

    #[test]
    fn restart_recovery_and_strict_replay_remain_offline_ready() {
        let mut fixture = DevelopmentProviderFixture::default_for_all(
            enrolled(),
            "explicit-fixture-seed",
            vec!["codex".into()],
        )
        .unwrap();
        let request = request(&fixture, "restart", "codex");
        fixture.state = DevelopmentFixtureState::Capturing;
        let bytes = fixture.to_json().unwrap();
        let mut restored = DevelopmentProviderFixture::from_json(&bytes).unwrap();
        assert_eq!(restored.state(), DevelopmentFixtureState::Capturing);
        restored.recover_after_restart().unwrap();
        assert_eq!(restored.state(), DevelopmentFixtureState::Ready);
        restored.capture(request.clone()).unwrap();
        let replay = restored.strict_replay_for("codex").unwrap();
        let request_body = serde_json::to_vec(&json!({
            "model": DEVELOPMENT_MOCK_MODEL,
            "messages": [{"role": "user", "content_digest": request.prompt_sha256}],
        }))
        .unwrap();
        let response = replay
            .handle(
                &ReplayRoute {
                    session_id: request.session_id,
                    attempt_id: request.attempt_id,
                    dialect: ProviderDialect::OpenaiChatCompletions,
                },
                ReplayHttpRequest {
                    method: "POST".into(),
                    path: "/v1/chat/completions".into(),
                    headers: Vec::new(),
                    body: request_body,
                },
            )
            .unwrap();
        assert_eq!(response.status, 200);
    }

    #[test]
    fn cancellation_is_recoverable_without_persisting_capture() {
        let mut fixture = DevelopmentProviderFixture::default_for_all(
            enrolled(),
            DEVELOPMENT_FIXTURE_SEED,
            vec!["codex".into()],
        )
        .unwrap();
        fixture.state = DevelopmentFixtureState::Capturing;
        fixture.cancel_capture().unwrap();
        assert_eq!(fixture.state(), DevelopmentFixtureState::Cancelled);
        fixture.recover_after_restart().unwrap();
        assert_eq!(fixture.state(), DevelopmentFixtureState::Ready);
        assert!(!fixture.comparison_readiness().ready);
    }

    #[test]
    fn restart_state_is_bounded_before_deserialization() {
        let oversized = vec![b' '; MAX_FIXTURE_STATE_BYTES + 1];
        assert!(matches!(
            DevelopmentProviderFixture::from_json(&oversized),
            Err(DevelopmentFixtureError::StateTooLarge)
        ));
    }
}
