// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Metadata-only provider authentication enrollment.
//!
//! This module deliberately owns references and lifecycle state, never secret
//! bytes. Secret resolution remains at the execution boundary in the
//! [`crate::credential`] module.

use asb_protocol::CredentialSource;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

/// Version of the authentication enrollment contract.
pub const AUTH_ENROLLMENT_V1: u16 = 1;
const MAX_ID_BYTES: usize = 128;
/// Maximum API-key size accepted by the enrollment boundary.
pub const MAX_SECRET_BYTES: usize = 16 * 1024;
const MAX_METADATA_BYTES: usize = 16 * 1024;

/// A credential-free reference to one provider secret.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialReferenceV1 {
    /// Credential lookup mechanism.
    pub source: CredentialSource,
    /// Digest of the canonical logical locator, never of the secret value.
    pub reference_sha256: String,
}

impl CredentialReferenceV1 {
    /// Construct and validate a reference from a non-secret logical locator.
    pub fn new(source: CredentialSource, locator: &str) -> Result<Self, AuthError> {
        if source == CredentialSource::None || locator.is_empty() || locator.len() > MAX_ID_BYTES {
            return Err(AuthError::InvalidReference);
        }
        if !locator.bytes().all(|b| b.is_ascii_graphic() && b != b' ') {
            return Err(AuthError::InvalidReference);
        }
        let mut digest = Sha256::new();
        digest.update(b"asb-auth-reference-v1\0");
        digest.update(source_tag(source).as_bytes());
        digest.update([0]);
        digest.update(locator.as_bytes());
        Ok(Self {
            source,
            reference_sha256: format_digest(digest.finalize()),
        })
    }

    fn validate(&self) -> Result<(), AuthError> {
        if self.source == CredentialSource::None
            || self.reference_sha256.len() != 64
            || !self
                .reference_sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(AuthError::InvalidReference);
        }
        Ok(())
    }
}

/// Safe public state of one enrolled connection.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EnrollmentStatus {
    /// A reference is enrolled but has not been probed.
    Untested,
    /// The bounded authenticated probe succeeded.
    Connected,
    /// The provider rejected the credential.
    Rejected,
    /// The credential has expired.
    Expired,
    /// No qualified secret backend is available.
    Unavailable,
    /// The reference was explicitly revoked.
    Revoked,
}

/// Result of a provider authentication probe.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProbeResult {
    /// Enrollment generation used for the probe.
    pub generation: u64,
    /// Public outcome class; no provider response text is retained.
    pub status: EnrollmentStatus,
}

/// Qualified secret-storage and probe boundary.
pub trait SecretBackend {
    /// Store or replace one secret under its logical reference.
    fn enroll(&mut self, reference: &CredentialReferenceV1, secret: &[u8])
    -> Result<(), AuthError>;
    /// Remove a secret without exposing its value.
    fn revoke(&mut self, reference: &CredentialReferenceV1) -> Result<(), AuthError>;
    /// Perform a bounded authenticated probe and return only its outcome class.
    fn probe(&mut self, reference: &CredentialReferenceV1) -> Result<EnrollmentStatus, AuthError>;
}

/// Public, persistable authentication enrollment metadata.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthEnrollmentV1 {
    /// Contract version.
    pub version: u16,
    /// Stable provider family/connection identifier.
    pub connection_id: String,
    /// Logical credential reference.
    pub credential: CredentialReferenceV1,
    /// Monotonically increasing replacement generation.
    pub generation: u64,
    /// Current safe status.
    pub status: EnrollmentStatus,
}

impl AuthEnrollmentV1 {
    /// Enroll a reference without reading or persisting a secret.
    pub fn enroll(
        connection_id: &str,
        credential: CredentialReferenceV1,
    ) -> Result<Self, AuthError> {
        validate_id(connection_id)?;
        credential.validate()?;
        Ok(Self {
            version: AUTH_ENROLLMENT_V1,
            connection_id: connection_id.to_owned(),
            credential,
            generation: 1,
            status: EnrollmentStatus::Untested,
        })
    }

    /// Enroll one API key through a caller-supplied qualified backend.
    /// The key is passed once and is never represented in returned metadata.
    pub fn enroll_api_key<B: SecretBackend>(
        connection_id: &str,
        credential: CredentialReferenceV1,
        secret: &[u8],
        backend: &mut B,
    ) -> Result<Self, AuthError> {
        validate_secret(secret)?;
        let enrollment = Self::enroll(connection_id, credential.clone())?;
        if let Err(error) = backend.enroll(&credential, secret) {
            let _ = backend.revoke(&credential);
            return Err(error);
        }
        Ok(enrollment)
    }

    /// Replace the reference, invalidating the old probe result.
    pub fn rotate(&mut self, credential: CredentialReferenceV1) -> Result<(), AuthError> {
        credential.validate()?;
        if self.status == EnrollmentStatus::Revoked {
            return Err(AuthError::Revoked);
        }
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or(AuthError::GenerationOverflow)?;
        self.credential = credential;
        self.status = EnrollmentStatus::Untested;
        Ok(())
    }

    /// Atomically stage a replacement secret before switching the metadata generation.
    pub fn rotate_api_key<B: SecretBackend>(
        &mut self,
        credential: CredentialReferenceV1,
        secret: &[u8],
        backend: &mut B,
    ) -> Result<(), AuthError> {
        validate_secret(secret)?;
        credential.validate()?;
        let next_generation = self
            .generation
            .checked_add(1)
            .ok_or(AuthError::GenerationOverflow)?;
        if self.status == EnrollmentStatus::Revoked {
            return Err(AuthError::Revoked);
        }
        let old = self.credential.clone();
        backend.enroll(&credential, secret)?;
        if old != credential {
            if let Err(error) = backend.revoke(&old) {
                // The old enrollment remains authoritative until both backend
                // effects have completed. Roll back the staged replacement.
                return match backend.revoke(&credential) {
                    Ok(()) => Err(error),
                    Err(_) => {
                        self.status = EnrollmentStatus::Unavailable;
                        Err(AuthError::RollbackFailed)
                    }
                };
            }
        }
        self.generation = next_generation;
        self.credential = credential;
        self.status = EnrollmentStatus::Untested;
        Ok(())
    }

    /// Record only the outcome class of a bounded authenticated probe.
    pub fn record_probe(&mut self, status: EnrollmentStatus) -> Result<(), AuthError> {
        if self.status == EnrollmentStatus::Revoked {
            return Err(AuthError::Revoked);
        }
        if !matches!(
            status,
            EnrollmentStatus::Connected
                | EnrollmentStatus::Rejected
                | EnrollmentStatus::Expired
                | EnrollmentStatus::Unavailable
                | EnrollmentStatus::Untested
        ) {
            return Err(AuthError::InvalidStatus);
        }
        self.status = status;
        Ok(())
    }

    /// Record a probe only when it belongs to this enrollment generation.
    pub fn record_probe_result(&mut self, result: ProbeResult) -> Result<(), AuthError> {
        if result.generation != self.generation {
            return Err(AuthError::StaleProbe);
        }
        self.record_probe(result.status)
    }

    /// Probe through a qualified backend and fence the result to this generation.
    pub fn probe<B: SecretBackend>(&mut self, backend: &mut B) -> Result<ProbeResult, AuthError> {
        if self.status == EnrollmentStatus::Revoked {
            return Err(AuthError::Revoked);
        }
        let result = ProbeResult {
            generation: self.generation,
            status: backend.probe(&self.credential)?,
        };
        self.record_probe_result(result)?;
        Ok(result)
    }

    /// Serialize bounded metadata for an application-owned durable store.
    pub fn to_json(&self) -> Result<Vec<u8>, AuthError> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(|_| AuthError::MetadataEncoding)?;
        if bytes.len() > MAX_METADATA_BYTES {
            return Err(AuthError::MetadataTooLarge);
        }
        Ok(bytes)
    }

    /// Restore and validate metadata from a bounded durable store.
    pub fn from_json(bytes: &[u8]) -> Result<Self, AuthError> {
        if bytes.is_empty() || bytes.len() > MAX_METADATA_BYTES {
            return Err(AuthError::MetadataTooLarge);
        }
        let enrollment: Self =
            serde_json::from_slice(bytes).map_err(|_| AuthError::InvalidEnrollment)?;
        enrollment.validate()?;
        Ok(enrollment)
    }

    /// Revoke the reference and make future probes impossible.
    pub fn revoke(&mut self) {
        self.status = EnrollmentStatus::Revoked;
    }

    /// Revoke backend material and then mark this enrollment revoked.
    pub fn revoke_from_backend<B: SecretBackend>(
        &mut self,
        backend: &mut B,
    ) -> Result<(), AuthError> {
        backend.revoke(&self.credential)?;
        self.revoke();
        Ok(())
    }

    /// Validate persisted metadata before use.
    pub fn validate(&self) -> Result<(), AuthError> {
        if self.version != AUTH_ENROLLMENT_V1 || self.generation == 0 {
            return Err(AuthError::InvalidEnrollment);
        }
        validate_id(&self.connection_id)?;
        self.credential.validate()
    }
}

/// Authentication enrollment failures without secret or path disclosure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthError {
    /// A stable identifier or digest is malformed.
    InvalidReference,
    /// Enrollment metadata is malformed or unsupported.
    InvalidEnrollment,
    /// A revoked enrollment cannot be reused.
    Revoked,
    /// Probe result is not a public lifecycle state.
    InvalidStatus,
    /// Generation cannot be increased safely.
    GenerationOverflow,
    /// Probe belongs to a replaced enrollment generation.
    StaleProbe,
    /// Secret is empty, contains a NUL, or exceeds the bounded input limit.
    InvalidSecret,
    /// Metadata could not be encoded.
    MetadataEncoding,
    /// Metadata exceeds its size limit.
    MetadataTooLarge,
    /// Backend could not roll back a failed rotation; state is unavailable.
    RollbackFailed,
    /// A qualified backend rejected an operation.
    BackendFailure,
}

impl fmt::Display for AuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidReference => "credential reference is invalid",
            Self::InvalidEnrollment => "authentication enrollment is invalid",
            Self::Revoked => "authentication enrollment is revoked",
            Self::InvalidStatus => "authentication probe status is invalid",
            Self::GenerationOverflow => "authentication generation overflow",
            Self::StaleProbe => "authentication probe belongs to a stale generation",
            Self::InvalidSecret => "authentication secret is empty or too large",
            Self::MetadataEncoding => "authentication metadata cannot be encoded",
            Self::MetadataTooLarge => "authentication metadata exceeds its size limit",
            Self::RollbackFailed => "authentication rotation rollback failed; state is unavailable",
            Self::BackendFailure => "authentication backend operation failed",
        })
    }
}

impl std::error::Error for AuthError {}

fn validate_id(value: &str) -> Result<(), AuthError> {
    if value.is_empty()
        || value.len() > MAX_ID_BYTES
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        return Err(AuthError::InvalidReference);
    }
    Ok(())
}

fn source_tag(source: CredentialSource) -> &'static str {
    match source {
        CredentialSource::None => "none",
        CredentialSource::Environment => "environment",
        CredentialSource::FileDescriptor => "file_descriptor",
        CredentialSource::Helper => "helper",
    }
}

fn validate_secret(secret: &[u8]) -> Result<(), AuthError> {
    if secret.is_empty() || secret.len() > MAX_SECRET_BYTES || secret.contains(&0) {
        return Err(AuthError::InvalidSecret);
    }
    Ok(())
}

fn format_digest(bytes: impl AsRef<[u8]>) -> String {
    bytes.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference(source: CredentialSource) -> CredentialReferenceV1 {
        CredentialReferenceV1::new(source, "provider.primary").unwrap()
    }

    #[test]
    fn enrollment_is_metadata_only_and_rotates() {
        let mut enrollment =
            AuthEnrollmentV1::enroll("openai-primary", reference(CredentialSource::Helper))
                .unwrap();
        assert_eq!(enrollment.status, EnrollmentStatus::Untested);
        enrollment
            .record_probe(EnrollmentStatus::Connected)
            .unwrap();
        enrollment
            .rotate(reference(CredentialSource::FileDescriptor))
            .unwrap();
        assert_eq!(enrollment.generation, 2);
        assert_eq!(enrollment.status, EnrollmentStatus::Untested);
        enrollment.revoke();
        assert_eq!(
            enrollment.rotate(reference(CredentialSource::Helper)),
            Err(AuthError::Revoked)
        );
    }

    #[test]
    fn malformed_and_sensitive_locators_are_rejected() {
        assert_eq!(
            CredentialReferenceV1::new(CredentialSource::Environment, "API KEY"),
            Err(AuthError::InvalidReference)
        );
        assert_eq!(
            CredentialReferenceV1::new(CredentialSource::None, "none"),
            Err(AuthError::InvalidReference)
        );
        assert_eq!(
            AuthEnrollmentV1::enroll("bad/id", reference(CredentialSource::Helper)),
            Err(AuthError::InvalidReference)
        );
    }

    #[test]
    fn public_serialization_contains_no_locator_or_secret() {
        let enrollment =
            AuthEnrollmentV1::enroll("openai-primary", reference(CredentialSource::Environment))
                .unwrap();
        let encoded = serde_json::to_string(&enrollment).unwrap();
        assert!(!encoded.contains("provider.primary"));
        assert!(!encoded.contains("secret"));
        assert!(enrollment.validate().is_ok());
    }

    struct Backend {
        stored: bool,
        fail_first_revoke: bool,
        revoke_calls: u8,
    }

    impl SecretBackend for Backend {
        fn enroll(&mut self, _: &CredentialReferenceV1, secret: &[u8]) -> Result<(), AuthError> {
            validate_secret(secret)?;
            self.stored = true;
            Ok(())
        }

        fn revoke(&mut self, _: &CredentialReferenceV1) -> Result<(), AuthError> {
            self.revoke_calls = self.revoke_calls.saturating_add(1);
            if self.fail_first_revoke && self.revoke_calls == 1 {
                return Err(AuthError::BackendFailure);
            }
            self.stored = false;
            Ok(())
        }

        fn probe(&mut self, _: &CredentialReferenceV1) -> Result<EnrollmentStatus, AuthError> {
            Ok(if self.stored {
                EnrollmentStatus::Connected
            } else {
                EnrollmentStatus::Unavailable
            })
        }
    }

    #[test]
    fn backend_probe_is_generation_fenced_and_metadata_persistable() {
        let mut backend = Backend {
            stored: false,
            fail_first_revoke: false,
            revoke_calls: 0,
        };
        let mut enrollment = AuthEnrollmentV1::enroll_api_key(
            "openai-primary",
            reference(CredentialSource::Helper),
            b"one-shot-secret",
            &mut backend,
        )
        .unwrap();
        let result = enrollment.probe(&mut backend).unwrap();
        assert_eq!(result.generation, 1);
        assert_eq!(result.status, EnrollmentStatus::Connected);
        assert_eq!(
            enrollment.record_probe_result(ProbeResult {
                generation: 0,
                status: EnrollmentStatus::Connected,
            }),
            Err(AuthError::StaleProbe)
        );
        let restored = AuthEnrollmentV1::from_json(&enrollment.to_json().unwrap()).unwrap();
        assert_eq!(restored, enrollment);
        assert!(
            !enrollment
                .to_json()
                .unwrap()
                .windows(16)
                .any(|w| w == b"one-shot-secret")
        );
    }

    #[test]
    fn secret_and_unknown_metadata_fail_closed() {
        let mut backend = Backend {
            stored: false,
            fail_first_revoke: false,
            revoke_calls: 0,
        };
        assert_eq!(
            AuthEnrollmentV1::enroll_api_key(
                "openai-primary",
                reference(CredentialSource::Helper),
                &[1, 0, 2],
                &mut backend,
            ),
            Err(AuthError::InvalidSecret)
        );
        let mut malformed =
            AuthEnrollmentV1::enroll("openai-primary", reference(CredentialSource::Helper))
                .unwrap()
                .to_json()
                .unwrap();
        malformed.truncate(malformed.len() - 1);
        malformed.extend_from_slice(b",\"unknown\":1}");
        assert_eq!(
            AuthEnrollmentV1::from_json(&malformed),
            Err(AuthError::InvalidEnrollment)
        );
    }

    #[test]
    fn failed_old_revoke_rolls_back_staged_replacement() {
        let old = reference(CredentialSource::Helper);
        let new = reference(CredentialSource::FileDescriptor);
        let mut backend = Backend {
            stored: true,
            fail_first_revoke: true,
            revoke_calls: 0,
        };
        let mut enrollment = AuthEnrollmentV1::enroll("openai-primary", old.clone()).unwrap();
        let before = enrollment.clone();
        assert_eq!(
            enrollment.rotate_api_key(new, b"replacement", &mut backend),
            Err(AuthError::BackendFailure)
        );
        assert_eq!(enrollment, before);
        assert!(!backend.stored, "staged replacement must be rolled back");
        assert_eq!(enrollment.credential, old);
    }
}
