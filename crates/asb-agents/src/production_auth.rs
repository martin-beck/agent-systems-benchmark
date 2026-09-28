// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Production credential enrollment contracts.
//!
//! This module is the production boundary for credentials.  It deliberately
//! contains no operating-system keychain implementation, network client, or
//! provider SDK: a platform adapter must implement [`SecureSecretStore`] and
//! [`RemoteVerifier`].  The adapters receive secret bytes only for the
//! one-shot store operation and must not return, log, serialize, or retain
//! them.  Development enrollment remains in [`crate::auth`] and does not use
//! this module.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use thiserror::Error;

/// Production authentication contract version.
pub const PRODUCTION_AUTH_V1: u16 = 1;
/// Maximum secret accepted at the secure-store boundary.
pub const MAX_PRODUCTION_SECRET_BYTES: usize = 16 * 1024;
/// Maximum number of retained audit events.
pub const MAX_AUDIT_EVENTS: usize = 128;
/// Maximum serialized audit log size.
pub const MAX_AUDIT_BYTES: usize = 64 * 1024;
/// Maximum public identifier length.
const MAX_ID_BYTES: usize = 128;

/// Secure storage class selected for a production enrollment.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretStorageKind {
    /// An OS keychain, secret service, or equivalent protected vault.
    OsKeychain,
    /// An already-open protected descriptor supplied by a trusted launcher.
    ProtectedDescriptor,
    /// A content-pinned credential helper owned by the deployment.
    PinnedHelper,
}

/// Non-secret identity of one value in a secure store.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SecretReferenceV1 {
    /// Contract version.
    pub version: u16,
    /// Storage class selected by the deployment.
    pub storage: SecretStorageKind,
    /// Digest of the logical service/namespace locator.
    pub locator_sha256: String,
}

impl SecretReferenceV1 {
    /// Create a reference from a non-secret logical locator.
    pub fn new(storage: SecretStorageKind, locator: &str) -> Result<Self, ProductionAuthError> {
        validate_identifier(locator)?;
        let mut digest = Sha256::new();
        digest.update(b"asb-production-secret-reference-v1\0");
        digest.update(storage_tag(storage).as_bytes());
        digest.update([0]);
        digest.update(locator.as_bytes());
        Ok(Self {
            version: PRODUCTION_AUTH_V1,
            storage,
            locator_sha256: digest_hex(digest.finalize()),
        })
    }

    fn validate(&self) -> Result<(), ProductionAuthError> {
        if self.version != PRODUCTION_AUTH_V1 || !is_sha256(&self.locator_sha256) {
            return Err(ProductionAuthError::InvalidReference);
        }
        Ok(())
    }
}

/// Platform-owned secure secret storage boundary.
///
/// Implementations should delegate to the host keychain or another reviewed
/// protected store.  They must wipe temporary buffers and return only bounded
/// public outcome classes.  ASB supplies no ambient environment fallback.
pub trait SecureSecretStore {
    /// Store or replace a secret under the supplied reference.
    fn put(
        &mut self,
        reference: &SecretReferenceV1,
        secret: &[u8],
    ) -> Result<(), ProductionAuthError>;
    /// Permanently revoke the value under a reference.
    fn revoke(&mut self, reference: &SecretReferenceV1) -> Result<(), ProductionAuthError>;
    /// Check that the protected store can retrieve the reference without
    /// returning its value to ASB.
    fn probe(&mut self, reference: &SecretReferenceV1) -> Result<(), ProductionAuthError>;
}

/// Deterministic provider-free store for contract and recovery tests.
///
/// The mock records only a digest of the supplied value, never the value
/// itself. It is not a production adapter and cannot authorize provider use.
#[derive(Clone, Debug, Default)]
pub struct DeterministicMockSecureSecretStore {
    references: std::collections::BTreeMap<String, String>,
    fail_next_revoke: bool,
}

impl DeterministicMockSecureSecretStore {
    /// Create an empty deterministic store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Cause the next revocation to fail, for rollback testing.
    pub fn fail_next_revoke(&mut self) {
        self.fail_next_revoke = true;
    }

    /// Return whether a reference currently has a value in the mock.
    #[must_use]
    pub fn contains(&self, reference: &SecretReferenceV1) -> bool {
        self.references.contains_key(&reference.locator_sha256)
    }
}

impl SecureSecretStore for DeterministicMockSecureSecretStore {
    fn put(
        &mut self,
        reference: &SecretReferenceV1,
        secret: &[u8],
    ) -> Result<(), ProductionAuthError> {
        reference.validate()?;
        validate_secret(secret)?;
        self.references.insert(
            reference.locator_sha256.clone(),
            digest_hex(Sha256::digest(secret)),
        );
        Ok(())
    }

    fn revoke(&mut self, reference: &SecretReferenceV1) -> Result<(), ProductionAuthError> {
        reference.validate()?;
        if self.fail_next_revoke {
            self.fail_next_revoke = false;
            return Err(ProductionAuthError::BackendFailure);
        }
        self.references.remove(&reference.locator_sha256);
        Ok(())
    }

    fn probe(&mut self, reference: &SecretReferenceV1) -> Result<(), ProductionAuthError> {
        reference.validate()?;
        if self.contains(reference) {
            Ok(())
        } else {
            Err(ProductionAuthError::SecureStoreUnavailable)
        }
    }
}

/// Fail-closed storage adapter used when no reviewed platform keychain exists.
#[derive(Clone, Copy, Debug, Default)]
pub struct FailClosedSecureSecretStore;

impl SecureSecretStore for FailClosedSecureSecretStore {
    fn put(
        &mut self,
        _reference: &SecretReferenceV1,
        _secret: &[u8],
    ) -> Result<(), ProductionAuthError> {
        Err(ProductionAuthError::SecureStoreUnavailable)
    }

    fn revoke(&mut self, _reference: &SecretReferenceV1) -> Result<(), ProductionAuthError> {
        Err(ProductionAuthError::SecureStoreUnavailable)
    }

    fn probe(&mut self, _reference: &SecretReferenceV1) -> Result<(), ProductionAuthError> {
        Err(ProductionAuthError::SecureStoreUnavailable)
    }
}

/// Provider family with an explicitly qualified authentication header.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderFamily {
    /// OpenAI and OpenAI-compatible providers.
    OpenAiCompatible,
    /// Anthropic's native API.
    Anthropic,
    /// Google's Gemini API.
    Gemini,
    /// Ollama deployments, local or explicitly authorized remote.
    Ollama,
}

/// Provider-specific placement of the secret.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderAuthMethod {
    /// `Authorization: Bearer <secret>`.
    AuthorizationBearer,
    /// `x-api-key: <secret>`.
    XApiKey,
    /// Provider-specific `x-goog-api-key: <secret>` placement.
    GoogleApiKey,
    /// Explicitly unauthenticated deployment.
    None,
}

/// Provider and authentication method bound to one enrollment.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderAuthBindingV1 {
    /// Contract version.
    pub version: u16,
    /// Provider family.
    pub provider: ProviderFamily,
    /// Explicit header policy.
    pub method: ProviderAuthMethod,
    /// Digest of the exact canonical endpoint.
    pub endpoint_identity_sha256: String,
}

impl ProviderAuthBindingV1 {
    /// Construct a provider binding after validating its provider policy.
    pub fn new(
        provider: ProviderFamily,
        method: ProviderAuthMethod,
        endpoint: &str,
    ) -> Result<Self, ProductionAuthError> {
        if endpoint.is_empty() || endpoint.len() > MAX_ID_BYTES {
            return Err(ProductionAuthError::InvalidEndpoint);
        }
        if !provider_allows(provider, method) {
            return Err(ProductionAuthError::ProviderPolicyMismatch);
        }
        Ok(Self {
            version: PRODUCTION_AUTH_V1,
            provider,
            method,
            endpoint_identity_sha256: digest_hex(Sha256::digest(endpoint.as_bytes())),
        })
    }

    /// Validate the binding against the exact endpoint selected for a request.
    pub fn validate_endpoint(&self, endpoint: &str) -> Result<(), ProductionAuthError> {
        if self.version != PRODUCTION_AUTH_V1
            || !is_sha256(&self.endpoint_identity_sha256)
            || !provider_allows(self.provider, self.method)
            || digest_hex(Sha256::digest(endpoint.as_bytes())) != self.endpoint_identity_sha256
        {
            return Err(ProductionAuthError::EndpointIdentityMismatch);
        }
        Ok(())
    }
}

/// Public status of a production enrollment.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductionEnrollmentStatus {
    /// A value is stored but remote authentication has not succeeded.
    Pending,
    /// Secure storage and remote verification both succeeded.
    Active,
    /// Remote verification rejected the value.
    Rejected,
    /// The enrollment was explicitly revoked and cannot be reused.
    Revoked,
    /// The secure store is unavailable.
    Unavailable,
}

/// Persistable production enrollment metadata; never contains a secret.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProductionEnrollmentV1 {
    /// Contract version.
    pub version: u16,
    /// Stable connection identifier.
    pub connection_id: String,
    /// Provider-specific authentication binding.
    pub auth: ProviderAuthBindingV1,
    /// Secure-store reference.
    pub secret: SecretReferenceV1,
    /// Compare-and-swap generation.
    pub generation: u64,
    /// Public lifecycle status.
    pub status: ProductionEnrollmentStatus,
}

impl ProductionEnrollmentV1 {
    /// Enroll one secret through a platform-provided secure store.
    pub fn enroll<S: SecureSecretStore>(
        connection_id: &str,
        auth: ProviderAuthBindingV1,
        secret: SecretReferenceV1,
        value: &[u8],
        store: &mut S,
    ) -> Result<Self, ProductionAuthError> {
        validate_identifier(connection_id)?;
        auth.validate_endpoint_hash()?;
        secret.validate()?;
        validate_secret(value)?;
        store.put(&secret, value)?;
        Ok(Self {
            version: PRODUCTION_AUTH_V1,
            connection_id: connection_id.to_owned(),
            auth,
            secret,
            generation: 1,
            status: ProductionEnrollmentStatus::Pending,
        })
    }

    /// Rotate the secret with generation compare-and-swap semantics.
    pub fn rotate<S: SecureSecretStore>(
        &mut self,
        expected_generation: u64,
        secret: SecretReferenceV1,
        value: &[u8],
        store: &mut S,
    ) -> Result<(), ProductionAuthError> {
        if self.status == ProductionEnrollmentStatus::Revoked {
            return Err(ProductionAuthError::Revoked);
        }
        if expected_generation != self.generation {
            return Err(ProductionAuthError::StaleGeneration);
        }
        secret.validate()?;
        validate_secret(value)?;
        let next = self
            .generation
            .checked_add(1)
            .ok_or(ProductionAuthError::GenerationOverflow)?;
        store.put(&secret, value)?;
        if self.secret != secret
            && let Err(error) = store.revoke(&self.secret)
        {
            let _ = store.revoke(&secret);
            return Err(error);
        }
        self.secret = secret;
        self.generation = next;
        self.status = ProductionEnrollmentStatus::Pending;
        Ok(())
    }

    /// Revoke the backend value and make this enrollment terminal.
    pub fn revoke<S: SecureSecretStore>(
        &mut self,
        expected_generation: u64,
        store: &mut S,
    ) -> Result<(), ProductionAuthError> {
        if expected_generation != self.generation {
            return Err(ProductionAuthError::StaleGeneration);
        }
        if self.status == ProductionEnrollmentStatus::Revoked {
            return Ok(());
        }
        store.revoke(&self.secret)?;
        self.status = ProductionEnrollmentStatus::Revoked;
        Ok(())
    }

    /// Validate public metadata before persistence or use.
    pub fn validate(&self) -> Result<(), ProductionAuthError> {
        if self.version != PRODUCTION_AUTH_V1
            || self.generation == 0
            || !matches!(
                self.status,
                ProductionEnrollmentStatus::Pending
                    | ProductionEnrollmentStatus::Active
                    | ProductionEnrollmentStatus::Rejected
                    | ProductionEnrollmentStatus::Revoked
                    | ProductionEnrollmentStatus::Unavailable
            )
        {
            return Err(ProductionAuthError::InvalidEnrollment);
        }
        validate_identifier(&self.connection_id)?;
        self.secret.validate()?;
        self.auth.validate_endpoint_hash()
    }

    /// Serialize bounded metadata without a secret, endpoint, or locator.
    pub fn to_json(&self) -> Result<Vec<u8>, ProductionAuthError> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(|_| ProductionAuthError::MetadataEncoding)?;
        if bytes.len() > MAX_AUDIT_BYTES {
            return Err(ProductionAuthError::MetadataTooLarge);
        }
        Ok(bytes)
    }
}

/// Authenticated remote verification request with no secret material.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteVerificationRequestV1 {
    /// Connection being verified.
    pub connection_id: String,
    /// Provider family bound to the enrollment.
    pub provider: ProviderFamily,
    /// Authentication method bound to the enrollment.
    pub auth_method: ProviderAuthMethod,
    /// Secure-store reference bound to the enrollment.
    pub secret: SecretReferenceV1,
    /// Enrollment generation being verified.
    pub generation: u64,
    /// Exact endpoint identity expected by the caller.
    pub endpoint_identity_sha256: String,
    /// Fresh challenge digest generated by the runtime.
    pub challenge_sha256: String,
    /// Maximum provider response bytes the verifier may inspect.
    pub max_response_bytes: u32,
}

/// Result returned by a qualified authenticated remote verifier.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteVerificationResultV1 {
    /// Generation observed by the verifier.
    pub generation: u64,
    /// Endpoint identity observed by the verifier.
    pub endpoint_identity_sha256: String,
    /// Digest of the bounded authenticated response, never response contents.
    pub response_sha256: String,
    /// Whether the provider accepted the credential and challenge.
    pub authenticated: bool,
}

/// Authenticated remote verification adapter supplied by the platform/runtime.
pub trait RemoteVerifier {
    /// Execute one bounded provider-specific verification request.
    fn verify(
        &mut self,
        request: &RemoteVerificationRequestV1,
    ) -> Result<RemoteVerificationResultV1, ProductionAuthError>;
}

impl ProductionEnrollmentV1 {
    /// Verify one remote endpoint and activate only an exact current result.
    pub fn verify_remote<V: RemoteVerifier>(
        &mut self,
        challenge_sha256: &str,
        max_response_bytes: u32,
        verifier: &mut V,
    ) -> Result<RemoteVerificationResultV1, ProductionAuthError> {
        self.validate()?;
        if self.status == ProductionEnrollmentStatus::Revoked {
            return Err(ProductionAuthError::Revoked);
        }
        if !is_sha256(challenge_sha256)
            || max_response_bytes == 0
            || max_response_bytes > 16 * 1024 * 1024
        {
            return Err(ProductionAuthError::InvalidVerificationRequest);
        }
        let request = RemoteVerificationRequestV1 {
            connection_id: self.connection_id.clone(),
            provider: self.auth.provider,
            auth_method: self.auth.method,
            secret: self.secret.clone(),
            generation: self.generation,
            endpoint_identity_sha256: self.auth.endpoint_identity_sha256.clone(),
            challenge_sha256: challenge_sha256.to_owned(),
            max_response_bytes,
        };
        let result = verifier.verify(&request)?;
        if result.generation != self.generation
            || result.endpoint_identity_sha256 != self.auth.endpoint_identity_sha256
            || !is_sha256(&result.response_sha256)
        {
            return Err(ProductionAuthError::StaleVerification);
        }
        self.status = if result.authenticated {
            ProductionEnrollmentStatus::Active
        } else {
            ProductionEnrollmentStatus::Rejected
        };
        if !result.authenticated {
            return Err(ProductionAuthError::RemoteRejected);
        }
        Ok(result)
    }
}

/// Public audit operation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditOperation {
    /// A secret was enrolled.
    Enroll,
    /// A secret was rotated.
    Rotate,
    /// A secret was revoked.
    Revoke,
    /// A remote authentication verification completed.
    Verify,
}

/// Bounded credential-free audit event.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuditEventV1 {
    /// Contract version.
    pub version: u16,
    /// Mutation or verification operation.
    pub operation: AuditOperation,
    /// Stable connection identifier.
    pub connection_id: String,
    /// Digest of the authenticated principal.
    pub principal_sha256: String,
    /// Digest of the request/idempotency identity.
    pub request_sha256: String,
    /// Enrollment generation affected.
    pub generation: u64,
}

impl AuditEventV1 {
    /// Construct a validated event from public digests.
    pub fn new(
        operation: AuditOperation,
        connection_id: &str,
        principal_sha256: &str,
        request_sha256: &str,
        generation: u64,
    ) -> Result<Self, ProductionAuthError> {
        validate_identifier(connection_id)?;
        if !is_sha256(principal_sha256) || !is_sha256(request_sha256) || generation == 0 {
            return Err(ProductionAuthError::InvalidAuditEvent);
        }
        Ok(Self {
            version: PRODUCTION_AUTH_V1,
            operation,
            connection_id: connection_id.to_owned(),
            principal_sha256: principal_sha256.to_owned(),
            request_sha256: request_sha256.to_owned(),
            generation,
        })
    }

    fn validate(&self) -> Result<(), ProductionAuthError> {
        if self.version != PRODUCTION_AUTH_V1 {
            return Err(ProductionAuthError::InvalidAuditEvent);
        }
        Self::new(
            self.operation,
            &self.connection_id,
            &self.principal_sha256,
            &self.request_sha256,
            self.generation,
        )?;
        Ok(())
    }
}

/// Bounded in-memory audit policy; callers persist only its JSON projection.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuditLogV1 {
    events: VecDeque<AuditEventV1>,
}

impl AuditLogV1 {
    /// Append an event, evicting only the oldest bounded event.
    pub fn append(&mut self, event: AuditEventV1) -> Result<(), ProductionAuthError> {
        event.validate()?;
        if self.events.len() == MAX_AUDIT_EVENTS {
            self.events.pop_front();
        }
        self.events.push_back(event);
        Ok(())
    }

    /// Return events in chronological order.
    pub fn events(&self) -> impl Iterator<Item = &AuditEventV1> {
        self.events.iter()
    }

    /// Serialize the bounded audit projection.
    pub fn to_json(&self) -> Result<Vec<u8>, ProductionAuthError> {
        for event in &self.events {
            event.validate()?;
        }
        let bytes = serde_json::to_vec(self).map_err(|_| ProductionAuthError::MetadataEncoding)?;
        if bytes.len() > MAX_AUDIT_BYTES {
            return Err(ProductionAuthError::MetadataTooLarge);
        }
        Ok(bytes)
    }
}

/// Production authentication failures without secret/path/response disclosure.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum ProductionAuthError {
    /// A reference or digest is malformed.
    #[error("production authentication reference is invalid")]
    InvalidReference,
    /// The endpoint identity is malformed or does not match.
    #[error("production authentication endpoint identity is invalid")]
    InvalidEndpoint,
    /// A request selected an endpoint different from the enrolled endpoint.
    #[error("production authentication endpoint identity does not match enrollment")]
    EndpointIdentityMismatch,
    /// The provider and authentication header policy are incompatible.
    #[error("provider authentication policy is incompatible")]
    ProviderPolicyMismatch,
    /// A persisted enrollment is malformed.
    #[error("production authentication enrollment is invalid")]
    InvalidEnrollment,
    /// Secret storage is unavailable; no fallback is permitted.
    #[error("secure credential store is unavailable")]
    SecureStoreUnavailable,
    /// Secret input violates its bounded one-shot contract.
    #[error("production authentication secret is invalid")]
    InvalidSecret,
    /// A rotation or verification used an old generation.
    #[error("production authentication generation is stale")]
    StaleGeneration,
    /// Generation cannot be incremented safely.
    #[error("production authentication generation overflow")]
    GenerationOverflow,
    /// Enrollment is terminally revoked.
    #[error("production authentication enrollment is revoked")]
    Revoked,
    /// Store or verifier operation failed without exposing details.
    #[error("production authentication backend operation failed")]
    BackendFailure,
    /// Remote verification request is outside its bounds.
    #[error("remote verification request is invalid")]
    InvalidVerificationRequest,
    /// Remote verification result was stale or mismatched.
    #[error("remote verification result is stale or mismatched")]
    StaleVerification,
    /// The remote endpoint rejected authentication.
    #[error("remote provider rejected authentication")]
    RemoteRejected,
    /// An audit record is malformed.
    #[error("production authentication audit event is invalid")]
    InvalidAuditEvent,
    /// Metadata could not be encoded.
    #[error("production authentication metadata cannot be encoded")]
    MetadataEncoding,
    /// Metadata exceeded its bounded size.
    #[error("production authentication metadata is too large")]
    MetadataTooLarge,
}

impl ProviderAuthBindingV1 {
    fn validate_endpoint_hash(&self) -> Result<(), ProductionAuthError> {
        if self.version != PRODUCTION_AUTH_V1
            || !is_sha256(&self.endpoint_identity_sha256)
            || !provider_allows(self.provider, self.method)
        {
            return Err(ProductionAuthError::InvalidEndpoint);
        }
        Ok(())
    }
}

fn provider_allows(provider: ProviderFamily, method: ProviderAuthMethod) -> bool {
    matches!(
        (provider, method),
        (
            ProviderFamily::OpenAiCompatible,
            ProviderAuthMethod::AuthorizationBearer
        ) | (ProviderFamily::Anthropic, ProviderAuthMethod::XApiKey)
            | (ProviderFamily::Gemini, ProviderAuthMethod::GoogleApiKey)
            | (
                ProviderFamily::Ollama,
                ProviderAuthMethod::AuthorizationBearer
            )
            | (ProviderFamily::Ollama, ProviderAuthMethod::None)
    )
}

fn storage_tag(storage: SecretStorageKind) -> &'static str {
    match storage {
        SecretStorageKind::OsKeychain => "os_keychain",
        SecretStorageKind::ProtectedDescriptor => "protected_descriptor",
        SecretStorageKind::PinnedHelper => "pinned_helper",
    }
}

fn validate_identifier(value: &str) -> Result<(), ProductionAuthError> {
    if value.is_empty()
        || value.len() > MAX_ID_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
    {
        return Err(ProductionAuthError::InvalidReference);
    }
    Ok(())
}

fn validate_secret(value: &[u8]) -> Result<(), ProductionAuthError> {
    if value.is_empty() || value.len() > MAX_PRODUCTION_SECRET_BYTES || value.contains(&0) {
        return Err(ProductionAuthError::InvalidSecret);
    }
    Ok(())
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn digest_hex(value: impl AsRef<[u8]>) -> String {
    value
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Verifier {
        result: RemoteVerificationResultV1,
        request: Option<RemoteVerificationRequestV1>,
    }

    impl RemoteVerifier for Verifier {
        fn verify(
            &mut self,
            request: &RemoteVerificationRequestV1,
        ) -> Result<RemoteVerificationResultV1, ProductionAuthError> {
            self.request = Some(request.clone());
            Ok(self.result.clone())
        }
    }

    fn setup() -> (ProviderAuthBindingV1, SecretReferenceV1) {
        (
            ProviderAuthBindingV1::new(
                ProviderFamily::OpenAiCompatible,
                ProviderAuthMethod::AuthorizationBearer,
                "https://provider.example/v1",
            )
            .unwrap(),
            SecretReferenceV1::new(SecretStorageKind::OsKeychain, "asb/provider/primary").unwrap(),
        )
    }

    #[test]
    fn provider_policy_and_endpoint_are_bound() {
        assert!(
            ProviderAuthBindingV1::new(
                ProviderFamily::Anthropic,
                ProviderAuthMethod::AuthorizationBearer,
                "https://provider.example",
            )
            .is_err()
        );
        let (auth, _) = setup();
        assert!(
            auth.validate_endpoint("https://provider.example/v1")
                .is_ok()
        );
        assert!(
            auth.validate_endpoint("https://alternate.example/v1")
                .is_err()
        );
    }

    #[test]
    fn enrollment_rotation_and_revoke_are_generation_fenced() {
        let (auth, old_secret) = setup();
        let new_secret =
            SecretReferenceV1::new(SecretStorageKind::OsKeychain, "asb/provider/new").unwrap();
        let mut store = DeterministicMockSecureSecretStore::new();
        let mut enrollment = ProductionEnrollmentV1::enroll(
            "primary",
            auth,
            old_secret,
            b"first-secret",
            &mut store,
        )
        .unwrap();
        assert_eq!(enrollment.status, ProductionEnrollmentStatus::Pending);
        enrollment
            .rotate(1, new_secret, b"second-secret", &mut store)
            .unwrap();
        assert_eq!(enrollment.generation, 2);
        assert_eq!(enrollment.status, ProductionEnrollmentStatus::Pending);
        assert_eq!(
            enrollment.revoke(1, &mut store),
            Err(ProductionAuthError::StaleGeneration)
        );
        enrollment.revoke(2, &mut store).unwrap();
        assert_eq!(enrollment.status, ProductionEnrollmentStatus::Revoked);
        let json = String::from_utf8(enrollment.to_json().unwrap()).unwrap();
        assert!(!json.contains("first-secret"));
        assert!(!json.contains("second-secret"));
        assert!(!json.contains("asb/provider"));
    }

    #[test]
    fn rotation_rolls_back_new_reference_when_old_revoke_fails() {
        let (auth, old_secret) = setup();
        let new_secret =
            SecretReferenceV1::new(SecretStorageKind::OsKeychain, "asb/provider/new").unwrap();
        let mut store = DeterministicMockSecureSecretStore::new();
        let mut enrollment = ProductionEnrollmentV1::enroll(
            "primary",
            auth,
            old_secret.clone(),
            b"first-secret",
            &mut store,
        )
        .unwrap();
        store.fail_next_revoke();

        assert_eq!(
            enrollment.rotate(1, new_secret.clone(), b"second-secret", &mut store),
            Err(ProductionAuthError::BackendFailure)
        );
        assert_eq!(enrollment.generation, 1);
        assert_eq!(enrollment.secret, old_secret);
        assert_eq!(enrollment.status, ProductionEnrollmentStatus::Pending);
        assert!(store.contains(&old_secret));
        assert!(!store.contains(&new_secret));
    }

    #[test]
    fn remote_verification_requires_exact_current_result() {
        let (auth, secret) = setup();
        let endpoint = auth.endpoint_identity_sha256.clone();
        let mut store = DeterministicMockSecureSecretStore::new();
        let mut enrollment =
            ProductionEnrollmentV1::enroll("primary", auth, secret, b"provider-secret", &mut store)
                .unwrap();
        let mut verifier = Verifier {
            result: RemoteVerificationResultV1 {
                generation: 1,
                endpoint_identity_sha256: endpoint,
                response_sha256: "a".repeat(64),
                authenticated: true,
            },
            request: None,
        };
        enrollment
            .verify_remote("b".repeat(64).as_str(), 4096, &mut verifier)
            .unwrap();
        assert_eq!(enrollment.status, ProductionEnrollmentStatus::Active);
        let request = verifier.request.as_ref().unwrap();
        assert_eq!(request.connection_id, "primary");
        assert_eq!(request.provider, ProviderFamily::OpenAiCompatible);
        assert_eq!(request.auth_method, ProviderAuthMethod::AuthorizationBearer);
        assert_eq!(request.secret, enrollment.secret);
        assert_eq!(request.generation, enrollment.generation);
        enrollment
            .rotate(
                1,
                SecretReferenceV1::new(SecretStorageKind::OsKeychain, "new").unwrap(),
                b"new",
                &mut store,
            )
            .unwrap();
        assert_eq!(
            enrollment.verify_remote("b".repeat(64).as_str(), 4096, &mut verifier),
            Err(ProductionAuthError::StaleVerification)
        );
    }

    #[test]
    fn remote_verification_validates_persisted_binding_before_dispatch() {
        let (mut auth, secret) = setup();
        auth.method = ProviderAuthMethod::XApiKey;
        let mut store = DeterministicMockSecureSecretStore::new();
        let mut enrollment = ProductionEnrollmentV1::enroll(
            "primary",
            setup().0,
            secret,
            b"provider-secret",
            &mut store,
        )
        .unwrap();
        enrollment.auth = auth;
        let mut verifier = Verifier {
            result: RemoteVerificationResultV1 {
                generation: 1,
                endpoint_identity_sha256: enrollment.auth.endpoint_identity_sha256.clone(),
                response_sha256: "a".repeat(64),
                authenticated: true,
            },
            request: None,
        };
        assert_eq!(
            enrollment.verify_remote("b".repeat(64).as_str(), 4096, &mut verifier),
            Err(ProductionAuthError::InvalidEndpoint)
        );
        assert!(verifier.request.is_none());
    }

    #[test]
    fn audit_log_is_bounded_and_credential_free() {
        let mut log = AuditLogV1::default();
        for generation in 1..=(MAX_AUDIT_EVENTS as u64 + 1) {
            log.append(
                AuditEventV1::new(
                    AuditOperation::Verify,
                    "primary",
                    &"c".repeat(64),
                    &format!("{generation:064x}"),
                    generation,
                )
                .unwrap(),
            )
            .unwrap();
        }
        assert_eq!(log.events().count(), MAX_AUDIT_EVENTS);
        let json = String::from_utf8(log.to_json().unwrap()).unwrap();
        assert!(!json.contains("provider-secret"));
        assert!(!json.contains("asb/provider"));
    }

    #[test]
    fn fail_closed_store_does_not_allow_production_fallback() {
        let (auth, secret) = setup();
        let mut store = FailClosedSecureSecretStore;
        assert_eq!(
            ProductionEnrollmentV1::enroll("primary", auth, secret, b"secret", &mut store),
            Err(ProductionAuthError::SecureStoreUnavailable)
        );
    }
}
