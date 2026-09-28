// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Development-only credential enrollment for provider-free setup.
//!
//! This module deliberately models setup state, not production credential
//! security.  It never accepts a secret, private key, endpoint, or executable
//! path.  The generated identity and signature are deterministic public
//! fixtures used to bind a local/mock selection to a generation.  Production
//! credential storage and trust remain outside this contract.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use thiserror::Error;

/// Version of the development enrollment contract.
pub const DEVELOPMENT_ENROLLMENT_SCHEMA_VERSION: u16 = 1;
/// Stable provider identifier for the checked-in provider-free fixture.
pub const DEVELOPMENT_MOCK_PROVIDER: &str = "asb-development-mock";
/// Stable model identifier for the checked-in provider-free fixture.
pub const DEVELOPMENT_MOCK_MODEL: &str = "fixture-model-v1";
/// Maximum public identity/request string size.
const MAX_TEXT_BYTES: usize = 128;
/// Maximum retained idempotency entries in one development store.
const MAX_IDEMPOTENCY_ENTRIES: usize = 64;

/// Authentication route used by the development contract.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DevelopmentAuthMethod {
    /// No provider secret is used; the local fixture identity is generated.
    Generated,
    /// No authentication is needed by the selected provider.
    None,
    /// A future runtime-owned reference may be selected, but no secret is
    /// accepted by this contract.
    CredentialReference,
    /// A future runtime-owned local daemon may be selected, without exposing
    /// its endpoint or credential.
    LocalDaemon,
}

/// One bounded enrollment operation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DevelopmentCredentialOperation {
    /// Create the first development identity.
    Enroll,
    /// Run the deterministic local/mock qualification.
    Test,
    /// Replace the identity at the next generation.
    Rotate,
    /// Clear the selected identity while fencing all older operations.
    Reset,
    /// Read the current public projection.
    Status,
}

/// Versioned, generation-fenced development enrollment request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DevelopmentCredentialRequest {
    /// Contract version.
    pub schema_version: u16,
    /// Operation requested by the setup flow.
    pub operation: DevelopmentCredentialOperation,
    /// Stable provider catalog identifier.
    pub provider_id: String,
    /// Stable provider model identifier.
    pub model_id: String,
    /// Selected authentication route.
    pub auth_method: DevelopmentAuthMethod,
    /// Retry identity; it is never a secret or credential locator.
    pub idempotency_key: String,
    /// Generation observed by the caller. Initial enroll uses zero; re-enroll
    /// after reset and all other mutations must match the active generation.
    pub expected_generation: u64,
}

/// Public lifecycle state of a development enrollment.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DevelopmentEnrollmentStatus {
    /// No development identity has been enrolled.
    Unenrolled,
    /// A local identity is ready for setup and mock qualification.
    Ready,
    /// The local/mock qualification succeeded.
    Tested,
    /// Setup completed with a development-only warning.
    Warning,
    /// The enrollment was reset; the generation remains fenced.
    Reset,
}

/// Deterministic public identity generated for one provider/model generation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DevelopmentIdentity {
    /// Public development key identity.
    pub key_id: String,
    /// Digest of the generated public-key fixture.
    pub public_key_sha256: String,
    /// Digest of the deterministic self-signature fixture.
    pub signature_sha256: String,
}

/// Privacy-safe public enrollment projection.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DevelopmentCredentialStatus {
    /// Contract version.
    pub schema_version: u16,
    /// Stable provider identifier.
    pub provider_id: String,
    /// Stable model identifier.
    pub model_id: String,
    /// Authentication route selected for this generation.
    pub auth_method: DevelopmentAuthMethod,
    /// Causal enrollment generation.
    pub generation: u64,
    /// Public lifecycle state.
    pub status: DevelopmentEnrollmentStatus,
    /// Deterministic identity, if enrolled.
    pub identity: Option<DevelopmentIdentity>,
    /// Bounded user-visible warnings.  Warning text contains no host data.
    pub warnings: Vec<String>,
}

/// Response to every operation, including warnings from unavailable optional
/// authentication/signature/key-management services.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DevelopmentCredentialResponse {
    /// Contract version.
    pub schema_version: u16,
    /// The operation that produced this response.
    pub operation: DevelopmentCredentialOperation,
    /// Public current projection.
    pub status: DevelopmentCredentialStatus,
    /// True only when the deterministic local/mock provider test ran.
    pub mock_tested: bool,
    /// Whether the request was accepted without requiring production services.
    pub development_only: bool,
}

/// Optional service availability used to exercise the warning path.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DevelopmentServiceAvailability {
    /// Whether an external authentication service exists.
    pub authentication: bool,
    /// Whether a production signature verifier exists.
    pub signature_validation: bool,
    /// Whether production key management exists.
    pub key_management: bool,
}

/// Deterministic local provider fixture.  It never performs network I/O.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LocalMockProvider;

impl LocalMockProvider {
    /// Qualify one compatible generated identity without contacting a provider.
    pub fn test(
        &self,
        provider_id: &str,
        model_id: &str,
        auth_method: DevelopmentAuthMethod,
        identity: &DevelopmentIdentity,
    ) -> Result<(), DevelopmentCredentialError> {
        self.test_at_generation(provider_id, model_id, auth_method, 1, identity)
    }

    /// Qualify one generation-bound identity without contacting a provider.
    pub fn test_at_generation(
        &self,
        provider_id: &str,
        model_id: &str,
        auth_method: DevelopmentAuthMethod,
        generation: u64,
        identity: &DevelopmentIdentity,
    ) -> Result<(), DevelopmentCredentialError> {
        validate_selection(provider_id, model_id, auth_method)?;
        if generation == 0 {
            return Err(DevelopmentCredentialError::StaleGeneration);
        }
        let expected = deterministic_identity(provider_id, model_id, auth_method, generation);
        if identity.public_key_sha256 != expected.public_key_sha256
            || identity.signature_sha256 != expected.signature_sha256
        {
            return Err(DevelopmentCredentialError::MockVerificationFailed);
        }
        Ok(())
    }
}

/// Restart-safe, bounded development enrollment state machine.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DevelopmentCredentialStore {
    /// Contract version.
    pub schema_version: u16,
    /// Current public enrollment, if any.
    pub status: DevelopmentCredentialStatus,
    /// Idempotency response cache keyed by caller-provided retry identity.
    idempotency: BTreeMap<String, DevelopmentIdempotencyRecord>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct DevelopmentIdempotencyRecord {
    request_sha256: String,
    response: DevelopmentCredentialResponse,
}

impl Default for DevelopmentCredentialStore {
    fn default() -> Self {
        Self::new()
    }
}

impl DevelopmentCredentialStore {
    /// Create an empty development-only enrollment store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            schema_version: DEVELOPMENT_ENROLLMENT_SCHEMA_VERSION,
            status: empty_status(),
            idempotency: BTreeMap::new(),
        }
    }

    /// Read the bounded public projection.
    #[must_use]
    pub fn status(&self) -> &DevelopmentCredentialStatus {
        &self.status
    }

    /// Validate a request without changing store state.
    pub fn validate_request(
        request: &DevelopmentCredentialRequest,
    ) -> Result<(), DevelopmentCredentialError> {
        validate_request(request)
    }

    /// Apply one operation with generation and idempotency fencing.
    pub fn apply(
        &mut self,
        request: DevelopmentCredentialRequest,
        services: DevelopmentServiceAvailability,
    ) -> Result<DevelopmentCredentialResponse, DevelopmentCredentialError> {
        validate_request(&request)?;
        let request_digest = digest_request(&request)?;
        if let Some(previous) = self.idempotency.get(&request.idempotency_key) {
            if previous.request_sha256 == request_digest {
                return Ok(previous.response.clone());
            }
            return Err(DevelopmentCredentialError::IdempotencyConflict);
        }
        validate_selection(&request.provider_id, &request.model_id, request.auth_method)?;
        validate_generation(&self.status, &request)?;

        let mut mock_tested = false;
        let status = match request.operation {
            DevelopmentCredentialOperation::Enroll | DevelopmentCredentialOperation::Rotate => {
                if request.operation == DevelopmentCredentialOperation::Rotate
                    && self.status.identity.is_none()
                {
                    return Err(DevelopmentCredentialError::NotEnrolled);
                }
                let generation = if request.operation == DevelopmentCredentialOperation::Enroll {
                    if self.status.generation == 0 {
                        1
                    } else {
                        self.status
                            .generation
                            .checked_add(1)
                            .ok_or(DevelopmentCredentialError::GenerationOverflow)?
                    }
                } else {
                    self.status
                        .generation
                        .checked_add(1)
                        .ok_or(DevelopmentCredentialError::GenerationOverflow)?
                };
                let identity = deterministic_identity(
                    &request.provider_id,
                    &request.model_id,
                    request.auth_method,
                    generation,
                );
                let warnings = development_warnings(services);
                DevelopmentCredentialStatus {
                    schema_version: DEVELOPMENT_ENROLLMENT_SCHEMA_VERSION,
                    provider_id: request.provider_id.clone(),
                    model_id: request.model_id.clone(),
                    auth_method: request.auth_method,
                    generation,
                    status: if warnings.is_empty() {
                        DevelopmentEnrollmentStatus::Ready
                    } else {
                        DevelopmentEnrollmentStatus::Warning
                    },
                    identity: Some(identity),
                    warnings,
                }
            }
            DevelopmentCredentialOperation::Test => {
                let identity = self
                    .status
                    .identity
                    .as_ref()
                    .ok_or(DevelopmentCredentialError::NotEnrolled)?;
                // The local fixture derives the generation-bound identity.  It
                // accepts every generation by comparing the stable fields after
                // the store has fenced the request.
                LocalMockProvider.test_at_generation(
                    &self.status.provider_id,
                    &self.status.model_id,
                    self.status.auth_method,
                    self.status.generation,
                    identity,
                )?;
                mock_tested = true;
                let mut result = self.status.clone();
                result.status = DevelopmentEnrollmentStatus::Tested;
                result
            }
            DevelopmentCredentialOperation::Reset => {
                let next_generation = self
                    .status
                    .generation
                    .checked_add(1)
                    .ok_or(DevelopmentCredentialError::GenerationOverflow)?;
                let mut result = empty_status();
                result.generation = next_generation;
                result.status = DevelopmentEnrollmentStatus::Reset;
                result.warnings = development_warnings(services);
                result
            }
            DevelopmentCredentialOperation::Status => self.status.clone(),
        };
        self.status = status;
        let response = DevelopmentCredentialResponse {
            schema_version: DEVELOPMENT_ENROLLMENT_SCHEMA_VERSION,
            operation: request.operation,
            status: self.status.clone(),
            mock_tested,
            development_only: true,
        };
        if self.idempotency.len() >= MAX_IDEMPOTENCY_ENTRIES {
            self.idempotency.pop_first();
        }
        self.idempotency.insert(
            request.idempotency_key,
            DevelopmentIdempotencyRecord {
                request_sha256: request_digest,
                response: response.clone(),
            },
        );
        Ok(response)
    }

    /// Cancel a not-yet-applied request without changing enrollment state.
    pub fn cancel(
        &self,
        request: &DevelopmentCredentialRequest,
    ) -> Result<(), DevelopmentCredentialError> {
        validate_request(request)?;
        Ok(())
    }

    /// Encode bounded restart state.  The encoding contains no key material.
    pub fn to_json(&self) -> Result<Vec<u8>, DevelopmentCredentialError> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|_| DevelopmentCredentialError::Encoding)
    }

    /// Restore and validate restart state.
    pub fn from_json(bytes: &[u8]) -> Result<Self, DevelopmentCredentialError> {
        if bytes.is_empty() || bytes.len() > 64 * 1024 {
            return Err(DevelopmentCredentialError::Encoding);
        }
        let store: Self =
            serde_json::from_slice(bytes).map_err(|_| DevelopmentCredentialError::InvalidState)?;
        store.validate()?;
        Ok(store)
    }

    fn validate(&self) -> Result<(), DevelopmentCredentialError> {
        if self.schema_version != DEVELOPMENT_ENROLLMENT_SCHEMA_VERSION
            || self.idempotency.len() > MAX_IDEMPOTENCY_ENTRIES
        {
            return Err(DevelopmentCredentialError::InvalidState);
        }
        for (key, record) in &self.idempotency {
            if key.is_empty()
                || key.len() > MAX_TEXT_BYTES
                || !key
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(&byte))
                || record.request_sha256.len() != 64
                || !record
                    .request_sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            {
                return Err(DevelopmentCredentialError::InvalidState);
            }
            validate_status(&record.response.status)?;
        }
        validate_status(&self.status)
    }
}

/// Errors intentionally omit secrets, paths, and provider response bodies.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum DevelopmentCredentialError {
    /// Request schema or bounds are invalid.
    #[error("development credential request is invalid")]
    InvalidRequest,
    /// Provider/model/auth combination is not in the local compatibility matrix.
    #[error("provider, model, and authentication method are incompatible")]
    IncompatibleSelection,
    /// Caller supplied a stale or invalid generation.
    #[error("development credential generation is stale")]
    StaleGeneration,
    /// Retry key was reused for a different request.
    #[error("development credential idempotency key conflicts")]
    IdempotencyConflict,
    /// A test or rotate operation needs an enrollment first.
    #[error("development credential is not enrolled")]
    NotEnrolled,
    /// The deterministic fixture identity does not match its selection.
    #[error("local mock provider verification failed")]
    MockVerificationFailed,
    /// Generation cannot be increased safely.
    #[error("development credential generation overflow")]
    GenerationOverflow,
    /// Persisted state is malformed or unsupported.
    #[error("development credential state is invalid")]
    InvalidState,
    /// State could not be serialized within the bounded contract.
    #[error("development credential state encoding failed")]
    Encoding,
}

fn empty_status() -> DevelopmentCredentialStatus {
    DevelopmentCredentialStatus {
        schema_version: DEVELOPMENT_ENROLLMENT_SCHEMA_VERSION,
        provider_id: String::new(),
        model_id: String::new(),
        auth_method: DevelopmentAuthMethod::Generated,
        generation: 0,
        status: DevelopmentEnrollmentStatus::Unenrolled,
        identity: None,
        warnings: Vec::new(),
    }
}

fn validate_request(
    request: &DevelopmentCredentialRequest,
) -> Result<(), DevelopmentCredentialError> {
    if request.schema_version != DEVELOPMENT_ENROLLMENT_SCHEMA_VERSION
        || request.idempotency_key.is_empty()
        || request.idempotency_key.len() > MAX_TEXT_BYTES
        || !request
            .idempotency_key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(&byte))
    {
        return Err(DevelopmentCredentialError::InvalidRequest);
    }
    if request.operation != DevelopmentCredentialOperation::Enroll
        && request.expected_generation == 0
    {
        return Err(DevelopmentCredentialError::InvalidRequest);
    }
    Ok(())
}

fn validate_selection(
    provider_id: &str,
    model_id: &str,
    auth_method: DevelopmentAuthMethod,
) -> Result<(), DevelopmentCredentialError> {
    if provider_id.is_empty()
        || model_id.is_empty()
        || provider_id.len() > MAX_TEXT_BYTES
        || model_id.len() > MAX_TEXT_BYTES
        || !provider_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:-/".contains(&byte))
        || !model_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:-/".contains(&byte))
    {
        return Err(DevelopmentCredentialError::InvalidRequest);
    }
    let compatible = match provider_id {
        DEVELOPMENT_MOCK_PROVIDER => {
            model_id == DEVELOPMENT_MOCK_MODEL
                && matches!(
                    auth_method,
                    DevelopmentAuthMethod::Generated | DevelopmentAuthMethod::None
                )
        }
        "openrouter" => {
            model_id.ends_with(":free")
                && matches!(
                    auth_method,
                    DevelopmentAuthMethod::Generated
                        | DevelopmentAuthMethod::CredentialReference
                        | DevelopmentAuthMethod::None
                )
        }
        "openai" | "anthropic" | "gemini" | "ollama" => {
            matches!(
                auth_method,
                DevelopmentAuthMethod::Generated
                    | DevelopmentAuthMethod::CredentialReference
                    | DevelopmentAuthMethod::LocalDaemon
                    | DevelopmentAuthMethod::None
            )
        }
        _ => false,
    };
    compatible
        .then_some(())
        .ok_or(DevelopmentCredentialError::IncompatibleSelection)
}

fn validate_generation(
    status: &DevelopmentCredentialStatus,
    request: &DevelopmentCredentialRequest,
) -> Result<(), DevelopmentCredentialError> {
    if request.operation == DevelopmentCredentialOperation::Enroll {
        let initial = status.generation == 0
            && status.status == DevelopmentEnrollmentStatus::Unenrolled
            && request.expected_generation == 0;
        let after_reset = status.status == DevelopmentEnrollmentStatus::Reset
            && status.generation > 0
            && request.expected_generation == status.generation;
        if !initial && !after_reset {
            return Err(DevelopmentCredentialError::StaleGeneration);
        }
    } else if request.expected_generation != status.generation {
        return Err(DevelopmentCredentialError::StaleGeneration);
    }
    Ok(())
}

fn development_warnings(services: DevelopmentServiceAvailability) -> Vec<String> {
    let mut warnings = Vec::new();
    if !services.authentication {
        warnings.push(
            "development-only: authentication service unavailable; local identity used".into(),
        );
    }
    if !services.signature_validation {
        warnings.push(
            "development-only: signature validation unavailable; deterministic fixture used".into(),
        );
    }
    if !services.key_management {
        warnings.push(
            "development-only: key management unavailable; generated identity is ephemeral".into(),
        );
    }
    warnings
}

fn deterministic_identity(
    provider_id: &str,
    model_id: &str,
    auth_method: DevelopmentAuthMethod,
    generation: u64,
) -> DevelopmentIdentity {
    let mut seed = Sha256::new();
    seed.update(b"asb-development-credential-v1\0");
    for value in [provider_id, model_id, auth_method_tag(auth_method)] {
        seed.update((value.len() as u64).to_le_bytes());
        seed.update(value.as_bytes());
    }
    seed.update(generation.to_le_bytes());
    let seed = seed.finalize();
    let public_key_sha256 = hex_digest(b"public-key", seed);
    let signature_sha256 = hex_digest(b"signature", public_key_sha256.as_bytes());
    let key_id = format!("dev-{}", &public_key_sha256[..16]);
    DevelopmentIdentity {
        key_id,
        public_key_sha256,
        signature_sha256,
    }
}

fn hex_digest(label: &[u8], value: impl AsRef<[u8]>) -> String {
    let mut digest = Sha256::new();
    digest.update(b"asb-development-identity-v1\0");
    digest.update(label);
    digest.update([0]);
    digest.update(value.as_ref());
    format!("{:x}", digest.finalize())
}

fn auth_method_tag(value: DevelopmentAuthMethod) -> &'static str {
    match value {
        DevelopmentAuthMethod::Generated => "generated",
        DevelopmentAuthMethod::None => "none",
        DevelopmentAuthMethod::CredentialReference => "credential_reference",
        DevelopmentAuthMethod::LocalDaemon => "local_daemon",
    }
}

fn validate_status(status: &DevelopmentCredentialStatus) -> Result<(), DevelopmentCredentialError> {
    if status.schema_version != DEVELOPMENT_ENROLLMENT_SCHEMA_VERSION
        || status.provider_id.len() > MAX_TEXT_BYTES
        || status.model_id.len() > MAX_TEXT_BYTES
        || status.warnings.len() > 3
    {
        return Err(DevelopmentCredentialError::InvalidState);
    }
    if status.generation == 0 {
        if status.status != DevelopmentEnrollmentStatus::Unenrolled
            || status.identity.is_some()
            || !status.provider_id.is_empty()
            || !status.model_id.is_empty()
        {
            return Err(DevelopmentCredentialError::InvalidState);
        }
    } else if status.status != DevelopmentEnrollmentStatus::Reset {
        validate_selection(&status.provider_id, &status.model_id, status.auth_method)?;
        if status.identity.is_none() {
            return Err(DevelopmentCredentialError::InvalidState);
        }
    }
    Ok(())
}

fn digest_request(
    request: &DevelopmentCredentialRequest,
) -> Result<String, DevelopmentCredentialError> {
    let bytes = serde_json::to_vec(request).map_err(|_| DevelopmentCredentialError::Encoding)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(
        operation: DevelopmentCredentialOperation,
        key: &str,
        generation: u64,
    ) -> DevelopmentCredentialRequest {
        DevelopmentCredentialRequest {
            schema_version: DEVELOPMENT_ENROLLMENT_SCHEMA_VERSION,
            operation,
            provider_id: DEVELOPMENT_MOCK_PROVIDER.into(),
            model_id: DEVELOPMENT_MOCK_MODEL.into(),
            auth_method: DevelopmentAuthMethod::Generated,
            idempotency_key: key.into(),
            expected_generation: generation,
        }
    }

    #[test]
    fn enroll_test_rotate_reset_status_are_versioned_and_fenced() {
        let mut store = DevelopmentCredentialStore::new();
        let unavailable = DevelopmentServiceAvailability::default();
        let enrolled = store
            .apply(
                request(DevelopmentCredentialOperation::Enroll, "enroll-1", 0),
                unavailable,
            )
            .unwrap();
        assert_eq!(enrolled.status.status, DevelopmentEnrollmentStatus::Warning);
        assert_eq!(enrolled.status.generation, 1);
        assert!(
            enrolled
                .status
                .warnings
                .iter()
                .all(|warning| warning.starts_with("development-only:"))
        );
        let tested = store
            .apply(
                request(DevelopmentCredentialOperation::Test, "test-1", 1),
                unavailable,
            )
            .unwrap();
        assert!(tested.mock_tested);
        assert_eq!(tested.status.status, DevelopmentEnrollmentStatus::Tested);
        let rotated = store
            .apply(
                request(DevelopmentCredentialOperation::Rotate, "rotate-1", 1),
                unavailable,
            )
            .unwrap();
        assert_eq!(rotated.status.generation, 2);
        assert_ne!(rotated.status.identity, tested.status.identity);
        assert_eq!(
            store
                .apply(
                    request(DevelopmentCredentialOperation::Status, "status-1", 2),
                    unavailable
                )
                .unwrap()
                .status
                .generation,
            2
        );
        assert_eq!(
            store
                .apply(
                    request(DevelopmentCredentialOperation::Reset, "reset-1", 2),
                    unavailable
                )
                .unwrap()
                .status
                .status,
            DevelopmentEnrollmentStatus::Reset
        );
        assert_eq!(
            store
                .apply(
                    request(DevelopmentCredentialOperation::Enroll, "reenroll-1", 3),
                    unavailable
                )
                .unwrap()
                .status
                .generation,
            4
        );
        assert!(matches!(
            store.apply(
                request(DevelopmentCredentialOperation::Test, "stale-1", 3),
                unavailable
            ),
            Err(DevelopmentCredentialError::StaleGeneration)
        ));
    }

    #[test]
    fn deterministic_identity_and_local_mock_are_stable() {
        let first = deterministic_identity(
            DEVELOPMENT_MOCK_PROVIDER,
            DEVELOPMENT_MOCK_MODEL,
            DevelopmentAuthMethod::Generated,
            1,
        );
        let second = deterministic_identity(
            DEVELOPMENT_MOCK_PROVIDER,
            DEVELOPMENT_MOCK_MODEL,
            DevelopmentAuthMethod::Generated,
            1,
        );
        assert_eq!(first, second);
        assert!(first.key_id.starts_with("dev-"));
        LocalMockProvider
            .test(
                DEVELOPMENT_MOCK_PROVIDER,
                DEVELOPMENT_MOCK_MODEL,
                DevelopmentAuthMethod::Generated,
                &first,
            )
            .unwrap();
        let mut tampered = first;
        tampered.signature_sha256.replace_range(..1, "0");
        assert_eq!(
            LocalMockProvider.test(
                DEVELOPMENT_MOCK_PROVIDER,
                DEVELOPMENT_MOCK_MODEL,
                DevelopmentAuthMethod::Generated,
                &tampered
            ),
            Err(DevelopmentCredentialError::MockVerificationFailed)
        );
    }

    #[test]
    fn restart_and_retry_are_idempotent_without_sensitive_material() {
        let mut store = DevelopmentCredentialStore::new();
        let request = request(DevelopmentCredentialOperation::Enroll, "restart-1", 0);
        let response = store
            .apply(request.clone(), DevelopmentServiceAvailability::default())
            .unwrap();
        let bytes = store.to_json().unwrap();
        let encoded = String::from_utf8(bytes.clone()).unwrap();
        assert!(encoded.len() < 16 * 1024);
        assert!(!encoded.contains("secret"));
        assert!(!encoded.contains("private"));
        let mut restored = DevelopmentCredentialStore::from_json(&bytes).unwrap();
        assert_eq!(
            restored
                .apply(request.clone(), DevelopmentServiceAvailability::default())
                .unwrap(),
            response
        );
        assert_eq!(restored.status(), store.status());
        let mut conflicting = request;
        conflicting.model_id = "fixture-model-v1".into();
        conflicting.auth_method = DevelopmentAuthMethod::None;
        assert_eq!(
            restored.apply(conflicting, DevelopmentServiceAvailability::default()),
            Err(DevelopmentCredentialError::IdempotencyConflict)
        );
    }

    #[test]
    fn cancellation_does_not_change_state_and_errors_are_bounded() {
        let store = DevelopmentCredentialStore::new();
        let pending = request(DevelopmentCredentialOperation::Enroll, "cancel-1", 0);
        store.cancel(&pending).unwrap();
        assert_eq!(
            store.status().status,
            DevelopmentEnrollmentStatus::Unenrolled
        );
        let mut invalid = pending;
        invalid.schema_version = 2;
        assert_eq!(
            store.cancel(&invalid),
            Err(DevelopmentCredentialError::InvalidRequest)
        );
        let mut incompatible = request(DevelopmentCredentialOperation::Enroll, "bad-1", 0);
        incompatible.provider_id = "unknown-provider".into();
        assert_eq!(
            DevelopmentCredentialStore::new()
                .apply(incompatible, DevelopmentServiceAvailability::default()),
            Err(DevelopmentCredentialError::IncompatibleSelection)
        );
    }

    #[test]
    fn compatibility_accepts_provider_auth_model_and_rejects_mismatch() {
        let mut store = DevelopmentCredentialStore::new();
        let mut openrouter = request(DevelopmentCredentialOperation::Enroll, "openrouter-1", 0);
        openrouter.provider_id = "openrouter".into();
        openrouter.model_id = "cohere/north-mini-code:free".into();
        openrouter.auth_method = DevelopmentAuthMethod::CredentialReference;
        assert!(
            store
                .apply(openrouter, DevelopmentServiceAvailability::default())
                .is_ok()
        );
        let mut mismatch = request(DevelopmentCredentialOperation::Enroll, "mismatch-1", 0);
        mismatch.model_id = "not-free".into();
        assert_eq!(
            DevelopmentCredentialStore::new()
                .apply(mismatch, DevelopmentServiceAvailability::default()),
            Err(DevelopmentCredentialError::IncompatibleSelection)
        );
    }
}
