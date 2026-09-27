// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Stable, secret-free lifecycle contract for the runtime/control process owner.

use serde::{Deserialize, Serialize};

use crate::live_service::{LocalProviderMockAttempt, LocalProviderMockBackend};

/// Current version of the runtime/control owner contract.
pub const RUNTIME_CONTROL_OWNER_CONTRACT_SCHEMA: u16 = 1;
const MAX_OWNER_ID_BYTES: usize = 128;
const MAX_CONTRACT_BYTES: usize = 4096;

/// Bounded lifecycle states owned by the runtime/control process owner.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeControlOwnerState {
    /// Owner exists but has not consumed an enrollment receipt.
    Prepared,
    /// Receipt and certificate chain have been authenticated.
    Enrolled,
    /// One opaque dispatch source has been minted.
    Issued,
    /// Owner cancellation fenced further issuance.
    Cancelled,
    /// Control authority was revoked.
    Revoked,
    /// Enrollment or owner lease expired.
    Expired,
    /// Authenticated control transport disconnected.
    Disconnected,
    /// Private inputs and resources were torn down.
    TornDown,
}

impl RuntimeControlOwnerState {
    /// Return whether no further lifecycle transition is permitted.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Cancelled | Self::Revoked | Self::Expired | Self::Disconnected | Self::TornDown
        )
    }
}

/// Fail-closed validation and transition errors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeControlOwnerContractError {
    /// The schema version is unsupported.
    UnsupportedSchema,
    /// The owner identifier is empty or exceeds its bound.
    InvalidOwner,
    /// A digest is not exactly 64 lowercase hexadecimal characters.
    InvalidDigest,
    /// Generation zero is not valid.
    InvalidGeneration,
    /// The requested transition is not permitted.
    InvalidTransition,
    /// The encoded contract exceeds its bounded size.
    TooLarge,
    /// The encoded contract is malformed or contains unknown fields.
    Malformed,
    /// The deterministic local/mock owner backend rejected an attempt.
    LocalMockUnavailable,
}

/// Runtime-owned provider-free process owner used for local/mock qualification.
/// It owns the mock backend and lifecycle projection; no production authority
/// or caller-supplied launch input can be derived from this type.
pub struct LocalMockRuntimeControlOwner {
    contract: RuntimeControlOwnerContractV1,
    backend: LocalProviderMockBackend,
}

impl std::fmt::Debug for LocalMockRuntimeControlOwner {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LocalMockRuntimeControlOwner")
            .field("contract", &self.contract)
            .finish_non_exhaustive()
    }
}

impl LocalMockRuntimeControlOwner {
    /// Provision an ephemeral owner with deterministic local/mock authority.
    pub fn provision(
        owner_id: String,
        generation: u64,
        chain_sha256: String,
        receipt_nonce_sha256: String,
    ) -> Result<Self, RuntimeControlOwnerContractError> {
        let contract = RuntimeControlOwnerContractV1::new(
            owner_id,
            generation,
            chain_sha256,
            receipt_nonce_sha256,
        )?;
        let backend = LocalProviderMockBackend::provision()
            .map_err(|_| RuntimeControlOwnerContractError::LocalMockUnavailable)?;
        Ok(Self { contract, backend })
    }

    /// Return the secret-free lifecycle projection.
    #[must_use]
    pub fn contract(&self) -> &RuntimeControlOwnerContractV1 {
        &self.contract
    }

    /// Authenticate the local/mock enrollment before issuing any attempt.
    pub fn enroll(&mut self) -> Result<(), RuntimeControlOwnerContractError> {
        self.contract.transition(RuntimeControlOwnerState::Enrolled)
    }

    /// Issue one bounded local/mock attempt after enrollment.
    pub fn issue_attempt(
        &mut self,
        attempt_id: u32,
    ) -> Result<LocalProviderMockAttempt<'_>, RuntimeControlOwnerContractError> {
        if self.contract.state != RuntimeControlOwnerState::Enrolled {
            return Err(RuntimeControlOwnerContractError::InvalidTransition);
        }
        let attempt = self
            .backend
            .issue_attempt(attempt_id)
            .map_err(|_| RuntimeControlOwnerContractError::LocalMockUnavailable)?;
        self.contract.transition(RuntimeControlOwnerState::Issued)?;
        Ok(attempt)
    }

    /// Fence local/mock attempts and close the owner.
    pub fn teardown(&mut self) -> Result<(), RuntimeControlOwnerContractError> {
        self.backend.revoke();
        self.contract.transition(RuntimeControlOwnerState::TornDown)
    }
}

/// Secret-free identity and lifecycle projection owned by runtime/control.
/// It never contains credentials, endpoints, policy, roots, tools, namespace
/// paths, or launch tokens.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeControlOwnerContractV1 {
    /// Closed schema version.
    pub schema_version: u16,
    /// Stable owner identity, not a user-provided authority token.
    pub owner_id: String,
    /// Monotonic enrollment generation.
    pub generation: u64,
    /// Digest of the authenticated certificate chain.
    pub chain_sha256: String,
    /// Digest of the one-shot receipt nonce.
    pub receipt_nonce_sha256: String,
    /// Current owner lifecycle state.
    pub state: RuntimeControlOwnerState,
}

impl RuntimeControlOwnerContractV1 {
    /// Construct a validated prepared owner contract.
    pub fn new(
        owner_id: String,
        generation: u64,
        chain_sha256: String,
        receipt_nonce_sha256: String,
    ) -> Result<Self, RuntimeControlOwnerContractError> {
        let contract = Self {
            schema_version: RUNTIME_CONTROL_OWNER_CONTRACT_SCHEMA,
            owner_id,
            generation,
            chain_sha256,
            receipt_nonce_sha256,
            state: RuntimeControlOwnerState::Prepared,
        };
        contract.validate()?;
        Ok(contract)
    }

    /// Validate identity and lifecycle schema without inspecting authority.
    pub fn validate(&self) -> Result<(), RuntimeControlOwnerContractError> {
        if self.schema_version != RUNTIME_CONTROL_OWNER_CONTRACT_SCHEMA {
            return Err(RuntimeControlOwnerContractError::UnsupportedSchema);
        }
        if self.owner_id.is_empty() || self.owner_id.len() > MAX_OWNER_ID_BYTES {
            return Err(RuntimeControlOwnerContractError::InvalidOwner);
        }
        if self.generation == 0 {
            return Err(RuntimeControlOwnerContractError::InvalidGeneration);
        }
        if !valid_digest(&self.chain_sha256) || !valid_digest(&self.receipt_nonce_sha256) {
            return Err(RuntimeControlOwnerContractError::InvalidDigest);
        }
        Ok(())
    }

    /// Advance the owner through an allowed transition, fail closed otherwise.
    pub fn transition(
        &mut self,
        next: RuntimeControlOwnerState,
    ) -> Result<(), RuntimeControlOwnerContractError> {
        let allowed = matches!(
            (self.state, next),
            (
                RuntimeControlOwnerState::Prepared,
                RuntimeControlOwnerState::Enrolled
            ) | (
                RuntimeControlOwnerState::Enrolled,
                RuntimeControlOwnerState::Issued
            ) | (
                RuntimeControlOwnerState::Prepared,
                RuntimeControlOwnerState::Cancelled
            ) | (
                RuntimeControlOwnerState::Prepared,
                RuntimeControlOwnerState::Revoked
            ) | (
                RuntimeControlOwnerState::Prepared,
                RuntimeControlOwnerState::Expired
            ) | (
                RuntimeControlOwnerState::Prepared,
                RuntimeControlOwnerState::Disconnected
            ) | (
                RuntimeControlOwnerState::Prepared,
                RuntimeControlOwnerState::TornDown
            ) | (
                RuntimeControlOwnerState::Enrolled,
                RuntimeControlOwnerState::Cancelled
            ) | (
                RuntimeControlOwnerState::Enrolled,
                RuntimeControlOwnerState::Revoked
            ) | (
                RuntimeControlOwnerState::Enrolled,
                RuntimeControlOwnerState::Expired
            ) | (
                RuntimeControlOwnerState::Enrolled,
                RuntimeControlOwnerState::Disconnected
            ) | (
                RuntimeControlOwnerState::Enrolled,
                RuntimeControlOwnerState::TornDown
            ) | (
                RuntimeControlOwnerState::Issued,
                RuntimeControlOwnerState::Cancelled
            ) | (
                RuntimeControlOwnerState::Issued,
                RuntimeControlOwnerState::Revoked
            ) | (
                RuntimeControlOwnerState::Issued,
                RuntimeControlOwnerState::Expired
            ) | (
                RuntimeControlOwnerState::Issued,
                RuntimeControlOwnerState::Disconnected
            ) | (
                RuntimeControlOwnerState::Issued,
                RuntimeControlOwnerState::TornDown
            )
        );
        if !allowed || self.state.is_terminal() {
            return Err(RuntimeControlOwnerContractError::InvalidTransition);
        }
        self.state = next;
        Ok(())
    }

    /// Encode a bounded public projection.
    pub fn encode(&self) -> Result<Vec<u8>, RuntimeControlOwnerContractError> {
        self.validate()?;
        let bytes =
            serde_json::to_vec(self).map_err(|_| RuntimeControlOwnerContractError::Malformed)?;
        if bytes.len() > MAX_CONTRACT_BYTES {
            return Err(RuntimeControlOwnerContractError::TooLarge);
        }
        Ok(bytes)
    }

    /// Decode and validate a bounded projection, rejecting unknown fields.
    pub fn decode(bytes: &[u8]) -> Result<Self, RuntimeControlOwnerContractError> {
        if bytes.len() > MAX_CONTRACT_BYTES {
            return Err(RuntimeControlOwnerContractError::TooLarge);
        }
        let contract: Self = serde_json::from_slice(bytes)
            .map_err(|_| RuntimeControlOwnerContractError::Malformed)?;
        contract.validate()?;
        Ok(contract)
    }
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contract() -> RuntimeControlOwnerContractV1 {
        RuntimeControlOwnerContractV1::new(
            "owner-local-mock".into(),
            7,
            "a".repeat(64),
            "b".repeat(64),
        )
        .unwrap()
    }

    #[test]
    fn lifecycle_allows_issue_then_terminal_teardown() {
        let mut value = contract();
        value
            .transition(RuntimeControlOwnerState::Enrolled)
            .unwrap();
        value.transition(RuntimeControlOwnerState::Issued).unwrap();
        value
            .transition(RuntimeControlOwnerState::TornDown)
            .unwrap();
        assert!(value.state.is_terminal());
        assert_eq!(
            RuntimeControlOwnerContractV1::decode(&value.encode().unwrap()).unwrap(),
            value
        );
    }

    #[test]
    fn hostile_identity_and_transition_fail_closed() {
        assert_eq!(
            RuntimeControlOwnerContractV1::new("".into(), 1, "a".repeat(64), "b".repeat(64)),
            Err(RuntimeControlOwnerContractError::InvalidOwner)
        );
        let mut value = contract();
        assert_eq!(
            value.transition(RuntimeControlOwnerState::Issued),
            Err(RuntimeControlOwnerContractError::InvalidTransition)
        );
        value
            .transition(RuntimeControlOwnerState::Cancelled)
            .unwrap();
        assert_eq!(
            value.transition(RuntimeControlOwnerState::Enrolled),
            Err(RuntimeControlOwnerContractError::InvalidTransition)
        );
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let mut bytes = contract().encode().unwrap();
        bytes.splice(
            bytes.len() - 1..bytes.len() - 1,
            br#",\"credential\":\"secret\""#.iter().copied(),
        );
        assert_eq!(
            RuntimeControlOwnerContractV1::decode(&bytes),
            Err(RuntimeControlOwnerContractError::Malformed)
        );
    }

    #[test]
    fn local_mock_owner_fences_issue_until_enrollment_and_teardown() {
        let mut owner = LocalMockRuntimeControlOwner::provision(
            "owner-local-mock".into(),
            7,
            "a".repeat(64),
            "b".repeat(64),
        )
        .unwrap();
        assert_eq!(
            owner.issue_attempt(1).map(|_| ()),
            Err(RuntimeControlOwnerContractError::InvalidTransition)
        );
        owner.enroll().unwrap();
        let mut attempt = owner.issue_attempt(1).unwrap();
        attempt.cancel();
        owner.teardown().unwrap();
        assert_eq!(owner.contract().state, RuntimeControlOwnerState::TornDown);
        assert_eq!(
            owner.issue_attempt(2).map(|_| ()),
            Err(RuntimeControlOwnerContractError::InvalidTransition)
        );
    }
}
