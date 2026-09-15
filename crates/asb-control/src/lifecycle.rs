// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Renderer-neutral, generation-bound local-agent lifecycle contract.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{ProtocolError, Revision, validate_digest, validate_identity};

/// Maximum lifecycle progress percentage.
const MAX_PROGRESS: u8 = 100;

/// Lifecycle operation state exposed to a frontend.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentLifecycleState {
    /// Verification and extraction have not started.
    Pending,
    /// The package is being staged and verified.
    Staging,
    /// The package is atomically active and passed its self-test.
    Active,
    /// The operation was explicitly cancelled.
    Cancelled,
    /// Verification or self-test failed; no partial activation is exposed.
    Failed,
    /// The agent was removed from the active set.
    Removed,
}

/// Closed public reason for a failed lifecycle operation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentLifecycleFailure {
    /// The catalog generation is no longer current.
    StaleCatalog,
    /// The selected package is not authenticated by the catalog.
    UnauthenticatedCatalog,
    /// Package digest, signature, SBOM, license, or provenance failed.
    VerificationFailed,
    /// The package target is incompatible with this runner.
    IncompatibleTarget,
    /// A self-test failed before activation.
    SelfTestFailed,
    /// Another operation currently owns the agent.
    Busy,
    /// An interruption requires reconciliation before retrying.
    NeedsReconciliation,
}

/// Common identity binding required by every lifecycle mutation.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentLifecycleBinding {
    /// Stable agent identity from the authenticated catalog.
    pub agent_id: String,
    /// Runner identity obtained during protocol negotiation.
    pub runner_instance_id: String,
    /// Catalog content address used for the operation.
    pub catalog_sha256: String,
}

/// Request to stage, verify, self-test, and atomically activate one agent.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentInstallRequest {
    /// Catalog and runner identity binding.
    pub binding: AgentLifecycleBinding,
    /// Catalog generation that authorized this selection.
    pub catalog_generation: Revision,
    /// Retry-safe mutation key.
    pub idempotency_key: String,
}

/// Request for durable status of an agent or operation.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentStatusRequest {
    /// Catalog and runner identity binding.
    pub binding: AgentLifecycleBinding,
    /// Specific lifecycle operation, or none for the current agent status.
    pub operation_id: Option<String>,
}

/// Request to cancel an in-progress install/retry operation.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentCancelRequest {
    /// Catalog and runner identity binding.
    pub binding: AgentLifecycleBinding,
    /// Exact operation to cancel.
    pub operation_id: String,
    /// Retry-safe mutation key.
    pub idempotency_key: String,
}

/// Request to retry a failed or reconciled operation.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentRetryRequest {
    /// Catalog and runner identity binding.
    pub binding: AgentLifecycleBinding,
    /// Exact operation to retry.
    pub operation_id: String,
    /// Retry-safe mutation key.
    pub idempotency_key: String,
}

/// Request to remove one active agent after durable status checks.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentRemoveRequest {
    /// Catalog and runner identity binding.
    pub binding: AgentLifecycleBinding,
    /// Retry-safe mutation key.
    pub idempotency_key: String,
}

/// Durable lifecycle result with bounded progress and explicit failure state.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentLifecycleResponse {
    /// Catalog and runner identity binding.
    pub binding: AgentLifecycleBinding,
    /// Durable operation identity.
    pub operation_id: String,
    /// Current durable state.
    pub state: AgentLifecycleState,
    /// Monotonic operation generation.
    pub generation: Revision,
    /// Bounded progress indicator; terminal states must be 0 or 100 as specified below.
    pub progress_percent: u8,
    /// Explicit failure reason when state is failed.
    pub failure: Option<AgentLifecycleFailure>,
}

impl AgentLifecycleBinding {
    /// Validate identity and digest fields without inspecting private input.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        validate_identity(&self.agent_id)?;
        validate_identity(&self.runner_instance_id)?;
        validate_digest(&self.catalog_sha256)
    }
}

impl AgentInstallRequest {
    /// Validate an authenticated install request.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        self.binding.validate()?;
        validate_identity(&self.idempotency_key)
    }
}

impl AgentStatusRequest {
    /// Validate a read-only status request.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        self.binding.validate()?;
        if let Some(operation_id) = &self.operation_id {
            validate_identity(operation_id)?;
        }
        Ok(())
    }
}

impl AgentCancelRequest {
    /// Validate a causally bound cancellation request.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        self.binding.validate()?;
        validate_identity(&self.operation_id)?;
        validate_identity(&self.idempotency_key)
    }
}

impl AgentRetryRequest {
    /// Validate a causally bound retry request.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        self.binding.validate()?;
        validate_identity(&self.operation_id)?;
        validate_identity(&self.idempotency_key)
    }
}

impl AgentRemoveRequest {
    /// Validate an authenticated removal request.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        self.binding.validate()?;
        validate_identity(&self.idempotency_key)
    }
}

impl AgentLifecycleResponse {
    /// Validate bounded state/progress and fail-closed failure semantics.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        self.binding.validate()?;
        validate_identity(&self.operation_id)?;
        if self.progress_percent > MAX_PROGRESS {
            return Err(ProtocolError::InvalidResponse);
        }
        let terminal = matches!(
            self.state,
            AgentLifecycleState::Active
                | AgentLifecycleState::Cancelled
                | AgentLifecycleState::Failed
                | AgentLifecycleState::Removed
        );
        if terminal && self.progress_percent != 100 && self.state != AgentLifecycleState::Failed {
            return Err(ProtocolError::InvalidResponse);
        }
        if (self.state == AgentLifecycleState::Failed) != self.failure.is_some() {
            return Err(ProtocolError::InvalidResponse);
        }
        if self.state != AgentLifecycleState::Failed && self.failure.is_some() {
            return Err(ProtocolError::InvalidResponse);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding() -> AgentLifecycleBinding {
        AgentLifecycleBinding {
            agent_id: "codex".into(),
            runner_instance_id: "runner-1".into(),
            catalog_sha256: "a".repeat(64),
        }
    }

    #[test]
    fn valid_install_and_terminal_status_are_bounded() {
        AgentInstallRequest {
            binding: binding(),
            catalog_generation: Revision(2),
            idempotency_key: "install-1".into(),
        }
        .validate()
        .unwrap();
        AgentLifecycleResponse {
            binding: binding(),
            operation_id: "operation-1".into(),
            state: AgentLifecycleState::Active,
            generation: Revision(3),
            progress_percent: 100,
            failure: None,
        }
        .validate()
        .unwrap();
    }

    #[test]
    fn partial_activation_and_unexplained_failure_are_rejected() {
        let mut active = AgentLifecycleResponse {
            binding: binding(),
            operation_id: "operation-1".into(),
            state: AgentLifecycleState::Active,
            generation: Revision(3),
            progress_percent: 50,
            failure: None,
        };
        assert!(active.validate().is_err());
        active.state = AgentLifecycleState::Failed;
        assert!(active.validate().is_err());
    }
}
