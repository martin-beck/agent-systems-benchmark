// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Runtime-owned admission and lifecycle authority for ASB benchmark attempts.
//!
//! Frontends submit [`RunRequest`] values only. They never construct a
//! credential, endpoint, namespace, relay, lease, sandbox backend, launch
//! token, or attempt capability. A concrete runtime adapter supplies those
//! effects behind [`AuthoritySource`].

use asb_protocol::Id;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use thiserror::Error;

/// Immutable orchestration contract generation.
pub const SCHEMA_VERSION: u16 = 1;
const MAX_IDEMPOTENCY_KEY: usize = 128;
const MAX_EVENTS: usize = 1024;
const MAX_ARTIFACTS: u32 = 1024;
const MAX_ARTIFACT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_ARTIFACT_TOTAL_BYTES: u64 = 256 * 1024 * 1024;
const MAX_TIMEOUT_MS: u64 = 86_400_000;
const MAX_OUTPUT_BYTES: u64 = 64 * 1024 * 1024;

/// Execution mode admitted by the central service.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExecutionMode {
    /// Credential-free deterministic loopback model.
    LocalMock,
    /// Strict cassette replay with network denial.
    StrictReplay,
    /// Explicit production provider mode; runtime attestation is mandatory.
    Live,
}

/// Bounded per-run resource and evidence ceilings.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunLimits {
    /// Maximum attempt duration in milliseconds.
    pub timeout_ms: u64,
    /// Maximum retained process output bytes.
    pub max_output_bytes: u64,
    /// Maximum journal events.
    pub max_events: u32,
    /// Maximum artifacts.
    pub max_artifacts: u32,
    /// Maximum bytes in one artifact.
    pub max_artifact_bytes: u64,
    /// Maximum bytes across all artifacts.
    pub max_artifact_total_bytes: u64,
}

impl RunLimits {
    fn validate(self) -> Result<Self, OrchestratorError> {
        if self.timeout_ms == 0 || self.timeout_ms > MAX_TIMEOUT_MS {
            return Err(OrchestratorError::InvalidLimit("timeout_ms"));
        }
        if self.max_output_bytes == 0 || self.max_output_bytes > MAX_OUTPUT_BYTES {
            return Err(OrchestratorError::InvalidLimit("max_output_bytes"));
        }
        if self.max_events == 0 || self.max_events as usize > MAX_EVENTS {
            return Err(OrchestratorError::InvalidLimit("max_events"));
        }
        if self.max_artifacts == 0 || self.max_artifacts > MAX_ARTIFACTS {
            return Err(OrchestratorError::InvalidLimit("max_artifacts"));
        }
        if self.max_artifact_bytes == 0 || self.max_artifact_bytes > MAX_ARTIFACT_BYTES {
            return Err(OrchestratorError::InvalidLimit("max_artifact_bytes"));
        }
        if self.max_artifact_total_bytes == 0
            || self.max_artifact_total_bytes > MAX_ARTIFACT_TOTAL_BYTES
        {
            return Err(OrchestratorError::InvalidLimit("max_artifact_total_bytes"));
        }
        Ok(self)
    }
}

/// Declarative request accepted by the orchestrator.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunRequest {
    /// Contract generation.
    pub schema_version: u16,
    /// Stable idempotency key supplied by a frontend.
    pub idempotency_key: String,
    /// Selected agent catalog identity.
    pub agent_id: Id,
    /// Selected provider catalog identity.
    pub provider_id: Id,
    /// Selected model catalog identity.
    pub model_id: Id,
    /// Selected workload catalog identity.
    pub workload_id: Id,
    /// Provider/workload catalog digest.
    pub catalog_digest: String,
    /// Content-pinned workload revision.
    pub workload_revision: String,
    /// Content-pinned scorer revision.
    pub scorer_revision: String,
    /// Requested mode.
    pub mode: ExecutionMode,
    /// Required cassette for strict replay.
    pub cassette_digest: Option<String>,
    /// Enrolled credential reference for live mode; never secret bytes.
    pub credential_ref_digest: Option<String>,
    /// Bounded execution and evidence limits.
    pub limits: RunLimits,
}

/// Server-issued opaque run handle with a causal fence.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RunHandle {
    id: Id,
    generation: u32,
    fence: String,
}

impl RunHandle {
    /// Stable opaque run identity.
    #[must_use]
    pub fn id(&self) -> &Id {
        &self.id
    }
    /// Monotonic handle generation.
    #[must_use]
    pub const fn generation(&self) -> u32 {
        self.generation
    }
    /// Causal fence digest.
    #[must_use]
    pub fn fence(&self) -> &str {
        &self.fence
    }
}

/// Server-issued opaque attempt handle with a causal fence.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct AttemptHandle {
    id: Id,
    generation: u32,
    fence: String,
}

impl AttemptHandle {
    /// Stable opaque attempt identity.
    #[must_use]
    pub fn id(&self) -> &Id {
        &self.id
    }
    /// Monotonic handle generation.
    #[must_use]
    pub const fn generation(&self) -> u32 {
        self.generation
    }
    /// Causal fence digest.
    #[must_use]
    pub fn fence(&self) -> &str {
        &self.fence
    }
}

/// Closed run lifecycle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunStatus {
    /// Request was accepted and journal intent exists.
    Admitted,
    /// Mode authority was prepared but execution has not begun.
    Prepared,
    /// Attempt is running.
    Running,
    /// Output and evidence are being collected.
    Collecting,
    /// Terminal successful outcome.
    Completed,
    /// Terminal failure.
    Failed,
    /// Terminal cancellation.
    Cancelled,
    /// Interrupted state requires inspection before retry.
    NeedsReconciliation,
}

impl RunStatus {
    fn terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

/// Bounded service event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunEvent {
    /// Monotonic sequence within a run.
    pub sequence: u32,
    /// State reached after this event.
    pub status: RunStatus,
}

/// Opaque mode authority held only by an attempt session.
#[derive(Debug)]
pub struct AttemptCapability {
    mode: ExecutionMode,
    binding: String,
}

impl AttemptCapability {
    /// Mode authorized by the runtime source.
    #[must_use]
    pub const fn mode(&self) -> ExecutionMode {
        self.mode
    }
    /// Binding digest without secret material.
    #[must_use]
    pub fn binding(&self) -> &str {
        &self.binding
    }
}

/// Runtime-owned source of mode-specific authority.
pub trait AuthoritySource {
    /// Prepare one authority after the orchestrator has journaled intent.
    fn prepare(&mut self, request: &RunRequest) -> Result<AttemptCapability, AuthorityError>;
}

/// Deterministic source used by development and CI.
#[derive(Debug, Default)]
pub struct DeterministicAuthoritySource;

impl AuthoritySource for DeterministicAuthoritySource {
    fn prepare(&mut self, request: &RunRequest) -> Result<AttemptCapability, AuthorityError> {
        if request.mode == ExecutionMode::Live {
            return Err(AuthorityError::LiveUnavailable);
        }
        if request.mode == ExecutionMode::StrictReplay && request.cassette_digest.is_none() {
            return Err(AuthorityError::MissingCassette);
        }
        let binding = request_digest(request).map_err(|_| AuthorityError::InvalidBinding)?;
        Ok(AttemptCapability {
            mode: request.mode,
            binding,
        })
    }
}

/// Service-level failures; no external effect is authorized on admission error.
#[derive(Debug, Error, Eq, PartialEq)]
pub enum OrchestratorError {
    /// Request field or digest is malformed.
    #[error("invalid request field: {0}")]
    InvalidRequest(&'static str),
    /// Limit exceeds the implementation ceiling.
    #[error("invalid run limit: {0}")]
    InvalidLimit(&'static str),
    /// Same idempotency key was bound to a different request.
    #[error("idempotency key conflict")]
    IdempotencyConflict,
    /// Handle is copied, stale, or bound to another run.
    #[error("stale or invalid handle")]
    StaleHandle,
    /// Requested lifecycle transition is not allowed.
    #[error("invalid lifecycle transition")]
    InvalidTransition,
    /// Authority source rejected preparation.
    #[error("authority preparation failed: {0}")]
    Authority(#[from] AuthorityError),
    /// No such run exists.
    #[error("run not found")]
    NotFound,
}

/// Mode-specific authority failures.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum AuthorityError {
    /// Real provider authority is not available from the runtime source.
    #[error("live provider authority unavailable")]
    LiveUnavailable,
    /// Replay requires a cassette identity.
    #[error("strict replay requires a cassette")]
    MissingCassette,
    /// Source could not compute a safe binding.
    #[error("invalid authority binding")]
    InvalidBinding,
}

struct RunRecord {
    request_digest: String,
    run: RunHandle,
    attempt: AttemptHandle,
    status: RunStatus,
    capability: Option<AttemptCapability>,
    events: Vec<RunEvent>,
}

/// Central runtime-owned orchestration service.
pub struct Orchestrator<S> {
    source: S,
    next_id: u64,
    runs: BTreeMap<Id, RunRecord>,
    idempotency: BTreeMap<String, Id>,
}

impl<S: AuthoritySource> Orchestrator<S> {
    /// Create an empty service with a runtime-owned authority source.
    #[must_use]
    pub fn new(source: S) -> Self {
        Self {
            source,
            next_id: 1,
            runs: BTreeMap::new(),
            idempotency: BTreeMap::new(),
        }
    }

    /// Admit and prepare one request; duplicate identical requests are idempotent.
    pub fn admit(&mut self, request: RunRequest) -> Result<RunHandle, OrchestratorError> {
        validate_request(&request)?;
        let digest =
            request_digest(&request).map_err(|_| OrchestratorError::InvalidRequest("digest"))?;
        if let Some(existing_id) = self.idempotency.get(&request.idempotency_key) {
            let record = self
                .runs
                .get(existing_id)
                .ok_or(OrchestratorError::NotFound)?;
            if record.request_digest == digest {
                return Ok(record.run.clone());
            }
            return Err(OrchestratorError::IdempotencyConflict);
        }
        let run_id = Id(format!("run-{}", self.next_id));
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or(OrchestratorError::InvalidRequest("run id"))?;
        let attempt_id = Id(format!("attempt-{}", self.next_id));
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or(OrchestratorError::InvalidRequest("attempt id"))?;
        let run = RunHandle {
            id: run_id.clone(),
            generation: 1,
            fence: digest.clone(),
        };
        let attempt = AttemptHandle {
            id: attempt_id,
            generation: 1,
            fence: digest.clone(),
        };
        let capability = self.source.prepare(&request)?;
        let mut record = RunRecord {
            request_digest: digest,
            run: run.clone(),
            attempt,
            status: RunStatus::Admitted,
            capability: Some(capability),
            events: Vec::new(),
        };
        push_event(&mut record, RunStatus::Prepared)?;
        self.idempotency
            .insert(request.idempotency_key, run_id.clone());
        self.runs.insert(run_id, record);
        Ok(run)
    }

    /// Start an admitted attempt using its exact server-issued fence.
    pub fn start(&mut self, handle: &RunHandle) -> Result<RunStatus, OrchestratorError> {
        let record = self.record_mut(handle)?;
        if record.status != RunStatus::Prepared {
            return Err(OrchestratorError::InvalidTransition);
        }
        push_event(record, RunStatus::Running)?;
        Ok(record.status)
    }

    /// Request cancellation; cleanup remains owned by the attempt session.
    pub fn cancel(&mut self, handle: &RunHandle) -> Result<RunStatus, OrchestratorError> {
        let record = self.record_mut(handle)?;
        if record.status.terminal() {
            return Ok(record.status);
        }
        push_event(record, RunStatus::Cancelled)?;
        record.capability = None;
        Ok(record.status)
    }

    /// Mark a running attempt as terminal after bounded collection.
    pub fn complete(&mut self, handle: &RunHandle) -> Result<RunStatus, OrchestratorError> {
        let record = self.record_mut(handle)?;
        if !matches!(record.status, RunStatus::Running | RunStatus::Collecting) {
            return Err(OrchestratorError::InvalidTransition);
        }
        push_event(record, RunStatus::Completed)?;
        record.capability = None;
        Ok(record.status)
    }

    /// Read the authoritative current status.
    pub fn status(&self, handle: &RunHandle) -> Result<RunStatus, OrchestratorError> {
        Ok(self.record(handle)?.status)
    }

    /// Read the server-issued attempt handle paired with a run.
    pub fn attempt_handle(&self, handle: &RunHandle) -> Result<AttemptHandle, OrchestratorError> {
        Ok(self.record(handle)?.attempt.clone())
    }

    /// Read bounded lifecycle events for a valid handle.
    pub fn events(&self, handle: &RunHandle) -> Result<Vec<RunEvent>, OrchestratorError> {
        Ok(self.record(handle)?.events.clone())
    }

    fn record(&self, handle: &RunHandle) -> Result<&RunRecord, OrchestratorError> {
        let record = self
            .runs
            .get(&handle.id)
            .ok_or(OrchestratorError::NotFound)?;
        if record.run != *handle {
            return Err(OrchestratorError::StaleHandle);
        }
        Ok(record)
    }

    fn record_mut(&mut self, handle: &RunHandle) -> Result<&mut RunRecord, OrchestratorError> {
        let record = self
            .runs
            .get_mut(&handle.id)
            .ok_or(OrchestratorError::NotFound)?;
        if record.run != *handle {
            return Err(OrchestratorError::StaleHandle);
        }
        Ok(record)
    }
}

fn push_event(record: &mut RunRecord, status: RunStatus) -> Result<(), OrchestratorError> {
    if record.events.len() >= MAX_EVENTS {
        return Err(OrchestratorError::InvalidLimit("max_events"));
    }
    let sequence = record.events.len() as u32 + 1;
    record.status = status;
    record.events.push(RunEvent { sequence, status });
    Ok(())
}

fn validate_request(request: &RunRequest) -> Result<(), OrchestratorError> {
    if request.schema_version != SCHEMA_VERSION {
        return Err(OrchestratorError::InvalidRequest("schema_version"));
    }
    if request.idempotency_key.is_empty() || request.idempotency_key.len() > MAX_IDEMPOTENCY_KEY {
        return Err(OrchestratorError::InvalidRequest("idempotency_key"));
    }
    for (name, value) in [
        ("catalog_digest", &request.catalog_digest),
        ("workload_revision", &request.workload_revision),
        ("scorer_revision", &request.scorer_revision),
    ] {
        if !valid_digest(value) {
            return Err(OrchestratorError::InvalidRequest(name));
        }
    }
    match request.mode {
        ExecutionMode::LocalMock
            if request.cassette_digest.is_some() || request.credential_ref_digest.is_some() =>
        {
            return Err(OrchestratorError::InvalidRequest("local authority fields"));
        }
        ExecutionMode::StrictReplay
            if request
                .cassette_digest
                .as_deref()
                .is_none_or(|value| !valid_digest(value))
                || request.credential_ref_digest.is_some() =>
        {
            return Err(OrchestratorError::InvalidRequest("replay authority fields"));
        }
        ExecutionMode::Live
            if request
                .credential_ref_digest
                .as_deref()
                .is_none_or(|value| !valid_digest(value))
                || request.cassette_digest.is_some() =>
        {
            return Err(OrchestratorError::InvalidRequest("live authority fields"));
        }
        _ => {}
    }
    request.limits.validate()?;
    Ok(())
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn request_digest(request: &RunRequest) -> Result<String, serde_json::Error> {
    let bytes = serde_json::to_vec(request)?;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(mode: ExecutionMode) -> RunRequest {
        RunRequest {
            schema_version: SCHEMA_VERSION,
            idempotency_key: "one".into(),
            agent_id: Id("aider".into()),
            provider_id: Id("openrouter".into()),
            model_id: Id("cohere/north-mini-code:free".into()),
            workload_id: Id("repo".into()),
            catalog_digest: "a".repeat(64),
            workload_revision: "b".repeat(64),
            scorer_revision: "c".repeat(64),
            mode,
            cassette_digest: (mode == ExecutionMode::StrictReplay).then(|| "d".repeat(64)),
            credential_ref_digest: (mode == ExecutionMode::Live).then(|| "e".repeat(64)),
            limits: RunLimits {
                timeout_ms: 1000,
                max_output_bytes: 1024,
                max_events: 16,
                max_artifacts: 4,
                max_artifact_bytes: 1024,
                max_artifact_total_bytes: 4096,
            },
        }
    }

    #[test]
    fn local_mock_runs_through_one_service_owned_lifecycle() {
        let mut service = Orchestrator::new(DeterministicAuthoritySource);
        let handle = service.admit(request(ExecutionMode::LocalMock)).unwrap();
        assert_eq!(service.attempt_handle(&handle).unwrap().generation(), 1);
        assert_eq!(service.start(&handle), Ok(RunStatus::Running));
        assert_eq!(service.complete(&handle), Ok(RunStatus::Completed));
        assert_eq!(service.events(&handle).unwrap().len(), 3);
    }

    #[test]
    fn live_is_fail_closed_without_a_runtime_source() {
        let mut service = Orchestrator::new(DeterministicAuthoritySource);
        assert_eq!(
            service.admit(request(ExecutionMode::Live)),
            Err(OrchestratorError::Authority(
                AuthorityError::LiveUnavailable
            ))
        );
    }

    #[test]
    fn duplicate_request_is_idempotent_but_conflicting_reuse_is_rejected() {
        let mut service = Orchestrator::new(DeterministicAuthoritySource);
        let first = request(ExecutionMode::LocalMock);
        let handle = service.admit(first.clone()).unwrap();
        assert_eq!(service.admit(first), Ok(handle.clone()));
        let mut conflict = request(ExecutionMode::LocalMock);
        conflict.model_id = Id("other".into());
        assert_eq!(
            service.admit(conflict),
            Err(OrchestratorError::IdempotencyConflict)
        );
    }

    #[test]
    fn copied_fence_cannot_cancel_another_run() {
        let mut service = Orchestrator::new(DeterministicAuthoritySource);
        let handle = service.admit(request(ExecutionMode::LocalMock)).unwrap();
        let stale = RunHandle {
            id: handle.id.clone(),
            generation: handle.generation,
            fence: "f".repeat(64),
        };
        assert_eq!(service.cancel(&stale), Err(OrchestratorError::StaleHandle));
    }

    #[test]
    fn serde_rejects_unknown_request_fields() {
        let mut value = serde_json::to_value(request(ExecutionMode::LocalMock)).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("authority".into(), serde_json::Value::Null);
        assert!(serde_json::from_value::<RunRequest>(value).is_err());
    }
}
