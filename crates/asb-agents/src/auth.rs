// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Metadata-only provider authentication enrollment.
//!
//! This module deliberately owns references and lifecycle state, never secret
//! bytes. Secret resolution remains at the execution boundary in [`credential`].

use asb_protocol::CredentialSource;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

/// Version of the authentication enrollment contract.
pub const AUTH_ENROLLMENT_V1: u16 = 1;
const MAX_ID_BYTES: usize = 128;

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

    /// Revoke the reference and make future probes impossible.
    pub fn revoke(&mut self) {
        self.status = EnrollmentStatus::Revoked;
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
}

impl fmt::Display for AuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidReference => "credential reference is invalid",
            Self::InvalidEnrollment => "authentication enrollment is invalid",
            Self::Revoked => "authentication enrollment is revoked",
            Self::InvalidStatus => "authentication probe status is invalid",
            Self::GenerationOverflow => "authentication generation overflow",
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
}
