// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Versioned, credential-free certificate-chain authorization boundary.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use rustls::RootCertStore;
use rustls::pki_types::{CertificateDer, UnixTime};
use rustls::server::WebPkiClientVerifier;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use x509_parser::parse_x509_certificate;

const MAX_CHAIN_LENGTH: usize = 8;
const CLOCK_SKEW_SECONDS: u64 = 300;
const MAX_ROLE_BYTES: usize = 32;

/// Public certificate identity metadata. Private key material and certificate bytes never cross
/// this boundary; each certificate is represented by its immutable SHA-256 digest.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CertificateIdentityV1 {
    /// Protocol version.
    pub schema_version: u16,
    /// Stable subject identity digest.
    pub subject_sha256: String,
    /// Issuer certificate digest, or the trust anchor digest for the root.
    pub issuer_sha256: String,
    /// Immutable certificate content digest.
    pub certificate_sha256: String,
    /// Explicit trust anchor digest selected by the runtime.
    pub trust_anchor_sha256: String,
    /// Monotonic enrollment generation.
    pub generation: u64,
    /// Inclusive validity start in Unix seconds.
    pub not_before: u64,
    /// Exclusive validity end in Unix seconds.
    pub not_after: u64,
    /// Least-privilege role bound to this identity.
    pub role: String,
    /// Endpoint identity digest bound during enrollment.
    pub endpoint_identity_sha256: String,
}

/// Runtime-issued, immutable chain authorization result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IssuedCertificateChainV1 {
    identity: CertificateIdentityV1,
    chain_sha256: String,
}

impl IssuedCertificateChainV1 {
    /// Validated identity metadata.
    #[must_use]
    pub fn identity(&self) -> &CertificateIdentityV1 {
        &self.identity
    }

    /// Digest of the canonical validated chain metadata.
    #[must_use]
    pub fn chain_sha256(&self) -> &str {
        &self.chain_sha256
    }
}

/// Runtime-owned authority for one trust anchor and enrollment generation.
#[derive(Clone, Debug)]
pub struct CertificateAuthorityV1 {
    trust_anchor_sha256: String,
    generation: u64,
    trust_anchor_der: Option<Vec<u8>>,
    endpoint_identity_sha256: Option<String>,
    revoked_generations: Arc<Mutex<BTreeSet<u64>>>,
}

impl CertificateAuthorityV1 {
    /// Construct an authority only from a validated trust-anchor digest.
    pub fn new(trust_anchor_sha256: String, generation: u64) -> Result<Self, CertificateError> {
        validate_digest(&trust_anchor_sha256)?;
        if generation == 0 {
            return Err(CertificateError::InvalidGeneration);
        }
        Ok(Self {
            trust_anchor_sha256,
            generation,
            trust_anchor_der: None,
            endpoint_identity_sha256: None,
            revoked_generations: Arc::new(Mutex::new(BTreeSet::new())),
        })
    }

    /// Construct an authority with a pinned DER trust anchor for cryptographic validation.
    pub fn with_trust_anchor(
        trust_anchor_der: Vec<u8>,
        generation: u64,
    ) -> Result<Self, CertificateError> {
        if trust_anchor_der.is_empty() {
            return Err(CertificateError::InvalidCertificateDer);
        }
        Ok(Self {
            trust_anchor_sha256: digest_bytes(&trust_anchor_der),
            generation: validate_generation(generation)?,
            trust_anchor_der: Some(trust_anchor_der),
            endpoint_identity_sha256: None,
            revoked_generations: Arc::new(Mutex::new(BTreeSet::new())),
        })
    }

    /// Construct an authority bound to the enrolled endpoint identity.
    pub fn with_trust_anchor_and_endpoint(
        trust_anchor_der: Vec<u8>,
        generation: u64,
        endpoint_identity_sha256: String,
    ) -> Result<Self, CertificateError> {
        validate_digest(&endpoint_identity_sha256)?;
        let mut authority = Self::with_trust_anchor(trust_anchor_der, generation)?;
        authority.endpoint_identity_sha256 = Some(endpoint_identity_sha256);
        Ok(authority)
    }

    /// Revoke a generation atomically; all subsequent issuance attempts fail closed.
    pub fn revoke_generation(&self, generation: u64) -> Result<(), CertificateError> {
        if generation == 0 {
            return Err(CertificateError::InvalidGeneration);
        }
        self.revoked_generations
            .lock()
            .map_err(|_| CertificateError::RevocationStateUnavailable)?
            .insert(generation);
        Ok(())
    }

    fn validate_metadata(
        &self,
        chain: &[CertificateIdentityV1],
        pairing_fingerprint_sha256: &str,
        now: u64,
    ) -> Result<IssuedCertificateChainV1, CertificateError> {
        validate_digest(pairing_fingerprint_sha256)?;
        if chain.is_empty() || chain.len() > MAX_CHAIN_LENGTH {
            return Err(CertificateError::InvalidChainLength);
        }
        for (index, identity) in chain.iter().enumerate() {
            validate_identity(identity, now, self.generation)?;
            if identity.trust_anchor_sha256 != self.trust_anchor_sha256 {
                return Err(CertificateError::TrustAnchorMismatch);
            }
            if index + 1 < chain.len()
                && identity.issuer_sha256 != chain[index + 1].certificate_sha256
            {
                return Err(CertificateError::IssuerMismatch);
            }
        }
        let leaf = chain.first().ok_or(CertificateError::InvalidChainLength)?;
        if leaf.subject_sha256 != pairing_fingerprint_sha256 {
            return Err(CertificateError::PairingMismatch);
        }
        let chain_sha256 = canonical_chain_digest(chain);
        Ok(IssuedCertificateChainV1 {
            identity: leaf.clone(),
            chain_sha256,
        })
    }

    /// Validate an actual DER chain to the pinned trust anchor before issuing authorization.
    pub fn issue_der(
        &self,
        chain: &[CertificateIdentityV1],
        leaf_der: Vec<u8>,
        intermediates_der: Vec<Vec<u8>>,
        pairing_fingerprint_sha256: &str,
        now: u64,
    ) -> Result<IssuedCertificateChainV1, CertificateError> {
        let issued = self.validate_metadata(chain, pairing_fingerprint_sha256, now)?;
        if self
            .revoked_generations
            .lock()
            .map_err(|_| CertificateError::RevocationStateUnavailable)?
            .contains(&self.generation)
        {
            return Err(CertificateError::RevokedGeneration);
        }
        let endpoint = self
            .endpoint_identity_sha256
            .as_deref()
            .ok_or(CertificateError::EndpointBindingUnavailable)?;
        if chain[0].endpoint_identity_sha256 != endpoint {
            return Err(CertificateError::EndpointBindingMismatch);
        }
        if digest_bytes(&leaf_der) != issued.identity.certificate_sha256 {
            return Err(CertificateError::CertificateDigestMismatch);
        }
        if intermediates_der.len() != chain.len().saturating_sub(1)
            || chain
                .iter()
                .skip(1)
                .zip(&intermediates_der)
                .any(|(identity, der)| digest_bytes(der) != identity.certificate_sha256)
        {
            return Err(CertificateError::CertificateDigestMismatch);
        }
        let (_, parsed_leaf) = parse_x509_certificate(&leaf_der)
            .map_err(|_| CertificateError::InvalidCertificateDer)?;
        if digest_bytes(parsed_leaf.subject().as_raw()) != issued.identity.subject_sha256 {
            return Err(CertificateError::PairingMismatch);
        }
        let anchor = self
            .trust_anchor_der
            .as_deref()
            .ok_or(CertificateError::TrustAnchorUnavailable)?;
        let mut roots = RootCertStore::empty();
        roots
            .add(CertificateDer::from(anchor.to_vec()))
            .map_err(|_| CertificateError::InvalidCertificateDer)?;
        let verifier = WebPkiClientVerifier::builder(Arc::new(roots))
            .build()
            .map_err(|_| CertificateError::InvalidCertificateDer)?;
        let leaf = CertificateDer::from(leaf_der);
        let intermediates: Vec<CertificateDer<'static>> = intermediates_der
            .into_iter()
            .map(CertificateDer::from)
            .collect();
        verifier
            .verify_client_cert(
                &leaf,
                &intermediates,
                UnixTime::since_unix_epoch(std::time::Duration::from_secs(now)),
            )
            .map_err(|_| CertificateError::ChainValidationFailed)?;
        Ok(issued)
    }
}

/// Fail-closed certificate issuance/chain validation errors.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum CertificateError {
    /// A digest was not exactly 64 hexadecimal characters.
    #[error("certificate digest is invalid")]
    InvalidDigest,
    /// A chain was empty or exceeded the bounded length.
    #[error("certificate chain length is invalid")]
    InvalidChainLength,
    /// The identity schema version is unsupported.
    #[error("certificate schema version is unsupported")]
    UnsupportedSchema,
    /// The chain generation does not match the authority.
    #[error("certificate generation is invalid")]
    InvalidGeneration,
    /// The validity interval is malformed or outside the bounded clock-skew window.
    #[error("certificate validity interval is invalid")]
    InvalidValidity,
    /// The issuer digest does not name the next certificate in the chain.
    #[error("certificate issuer does not match chain")]
    IssuerMismatch,
    /// The chain does not use the authority's explicit trust anchor.
    #[error("certificate trust anchor does not match authority")]
    TrustAnchorMismatch,
    /// The pairing fingerprint does not identify the leaf subject.
    #[error("certificate pairing fingerprint does not match leaf")]
    PairingMismatch,
    /// The role is empty, oversized, or not one of the supported roles.
    #[error("certificate role is invalid")]
    InvalidRole,
    /// The DER certificate could not be used as a trust anchor.
    #[error("certificate DER is invalid")]
    InvalidCertificateDer,
    /// The authority has no pinned DER trust anchor for cryptographic validation.
    #[error("certificate trust anchor is unavailable")]
    TrustAnchorUnavailable,
    /// The leaf digest does not match the identity metadata.
    #[error("certificate digest does not match identity")]
    CertificateDigestMismatch,
    /// The presented chain does not validate to the pinned trust anchor.
    #[error("certificate chain validation failed")]
    ChainValidationFailed,
    /// The authority has no enrolled endpoint binding.
    #[error("certificate endpoint binding is unavailable")]
    EndpointBindingUnavailable,
    /// The certificate endpoint binding does not match enrollment.
    #[error("certificate endpoint binding does not match enrollment")]
    EndpointBindingMismatch,
    /// The certificate generation has been revoked.
    #[error("certificate generation is revoked")]
    RevokedGeneration,
    /// Revocation state could not be read or updated safely.
    #[error("certificate revocation state is unavailable")]
    RevocationStateUnavailable,
}

fn validate_generation(generation: u64) -> Result<u64, CertificateError> {
    if generation == 0 {
        return Err(CertificateError::InvalidGeneration);
    }
    Ok(generation)
}

fn digest_bytes(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}

fn validate_identity(
    identity: &CertificateIdentityV1,
    now: u64,
    generation: u64,
) -> Result<(), CertificateError> {
    if identity.schema_version != 1 {
        return Err(CertificateError::UnsupportedSchema);
    }
    validate_digest(&identity.subject_sha256)?;
    validate_digest(&identity.issuer_sha256)?;
    validate_digest(&identity.certificate_sha256)?;
    validate_digest(&identity.trust_anchor_sha256)?;
    validate_digest(&identity.endpoint_identity_sha256)?;
    if identity.generation == 0 || identity.generation != generation {
        return Err(CertificateError::InvalidGeneration);
    }
    if identity.not_before == 0
        || identity.not_after == 0
        || identity.not_before > identity.not_after
        || now.saturating_add(CLOCK_SKEW_SECONDS) < identity.not_before
        || now > identity.not_after.saturating_add(CLOCK_SKEW_SECONDS)
    {
        return Err(CertificateError::InvalidValidity);
    }
    if identity.role.is_empty()
        || identity.role.len() > MAX_ROLE_BYTES
        || !matches!(
            identity.role.as_str(),
            "observer" | "operator" | "administrator"
        )
    {
        return Err(CertificateError::InvalidRole);
    }
    Ok(())
}

fn validate_digest(value: &str) -> Result<(), CertificateError> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(CertificateError::InvalidDigest);
    }
    Ok(())
}

fn canonical_chain_digest(chain: &[CertificateIdentityV1]) -> String {
    let mut hasher = Sha256::new();
    for identity in chain {
        hasher.update(identity.schema_version.to_le_bytes());
        let values = [
            identity.subject_sha256.clone(),
            identity.issuer_sha256.clone(),
            identity.certificate_sha256.clone(),
            identity.trust_anchor_sha256.clone(),
            identity.generation.to_string(),
            identity.not_before.to_string(),
            identity.not_after.to_string(),
            identity.role.clone(),
            identity.endpoint_identity_sha256.clone(),
        ];
        for value in values {
            hasher.update((value.len() as u64).to_le_bytes());
            hasher.update(value.as_bytes());
        }
    }
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rcgen::{
        BasicConstraints, CertificateParams, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
        KeyUsagePurpose, date_time_ymd,
    };

    fn identity(subject: &str, issuer: &str, certificate: &str) -> CertificateIdentityV1 {
        CertificateIdentityV1 {
            schema_version: 1,
            subject_sha256: subject.into(),
            issuer_sha256: issuer.into(),
            certificate_sha256: certificate.into(),
            trust_anchor_sha256: "a".repeat(64),
            generation: 7,
            not_before: 900,
            not_after: 1_100,
            role: "operator".into(),
            endpoint_identity_sha256: "b".repeat(64),
        }
    }

    #[test]
    fn issues_valid_pairing_bound_chain() {
        let authority = CertificateAuthorityV1::new("a".repeat(64), 7).unwrap();
        let chain = vec![identity(&"c".repeat(64), &"d".repeat(64), &"e".repeat(64))];
        let issued = authority
            .validate_metadata(&chain, &"c".repeat(64), 1_000)
            .unwrap();
        assert_eq!(issued.identity().subject_sha256, "c".repeat(64));
        assert_eq!(issued.chain_sha256().len(), 64);
    }

    #[test]
    fn rejects_wrong_pairing_anchor_issuer_and_expiry() {
        let authority = CertificateAuthorityV1::new("a".repeat(64), 7).unwrap();
        let chain = vec![identity(&"c".repeat(64), &"d".repeat(64), &"e".repeat(64))];
        assert_eq!(
            authority.validate_metadata(&chain, &"f".repeat(64), 1_000),
            Err(CertificateError::PairingMismatch)
        );
        let mut wrong_anchor = chain[0].clone();
        wrong_anchor.trust_anchor_sha256 = "f".repeat(64);
        assert_eq!(
            authority.validate_metadata(&[wrong_anchor], &"c".repeat(64), 1_000),
            Err(CertificateError::TrustAnchorMismatch)
        );
        let mut expired = chain[0].clone();
        expired.not_after = 500;
        assert_eq!(
            authority.validate_metadata(&[expired], &"c".repeat(64), 1_000),
            Err(CertificateError::InvalidValidity)
        );
    }

    #[test]
    fn validates_actual_der_chain_to_pinned_anchor() {
        let mut root_params = CertificateParams::new(vec!["asb-root".into()]).unwrap();
        root_params.not_before = date_time_ymd(2020, 1, 1);
        root_params.not_after = date_time_ymd(2035, 1, 1);
        root_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        root_params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
        let root_key = KeyPair::generate().unwrap();
        let root_cert = root_params.self_signed(&root_key).unwrap();
        let issuer = Issuer::from_params(&root_params, &root_key);

        let mut leaf_params = CertificateParams::new(vec!["asb-runner".into()]).unwrap();
        leaf_params.not_before = date_time_ymd(2020, 1, 1);
        leaf_params.not_after = date_time_ymd(2035, 1, 1);
        leaf_params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        leaf_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        let leaf_key = KeyPair::generate().unwrap();
        let leaf_cert = leaf_params.signed_by(&leaf_key, &issuer).unwrap();

        let root_der = root_cert.der().to_vec();
        let leaf_der = leaf_cert.der().to_vec();
        let root_digest = digest_bytes(&root_der);
        let leaf_digest = digest_bytes(&leaf_der);
        let (_, parsed_leaf) = parse_x509_certificate(&leaf_der).unwrap();
        let subject = digest_bytes(parsed_leaf.subject().as_raw());
        let identity = CertificateIdentityV1 {
            schema_version: 1,
            subject_sha256: subject.clone(),
            issuer_sha256: root_digest.clone(),
            certificate_sha256: leaf_digest,
            trust_anchor_sha256: root_digest,
            generation: 7,
            not_before: 1_600_000_000,
            not_after: 2_000_000_000,
            role: "operator".into(),
            endpoint_identity_sha256: "b".repeat(64),
        };
        let authority = CertificateAuthorityV1::with_trust_anchor_and_endpoint(
            root_der.clone(),
            7,
            "b".repeat(64),
        )
        .unwrap();
        let issued = authority
            .issue_der(&[identity], leaf_der, Vec::new(), &subject, 1_700_000_000)
            .unwrap();
        assert_eq!(issued.identity().generation, 7);
        let mismatched_endpoint = CertificateAuthorityV1::with_trust_anchor_and_endpoint(
            root_cert.der().to_vec(),
            7,
            "f".repeat(64),
        )
        .unwrap();
        assert_eq!(
            mismatched_endpoint.issue_der(
                &[issued.identity().clone()],
                vec![1, 2, 3],
                Vec::new(),
                &subject,
                1_700_000_000
            ),
            Err(CertificateError::EndpointBindingMismatch)
        );
        let mut malformed = root_der;
        malformed[0] ^= 1;
        assert_eq!(
            authority.issue_der(
                &[issued.identity().clone()],
                vec![1, 2, 3],
                Vec::new(),
                &subject,
                1_700_000_000
            ),
            Err(CertificateError::CertificateDigestMismatch)
        );
        let _ = malformed;
        authority.revoke_generation(7).unwrap();
        assert_eq!(
            authority.issue_der(
                &[issued.identity().clone()],
                vec![1, 2, 3],
                Vec::new(),
                &subject,
                1_700_000_000
            ),
            Err(CertificateError::RevokedGeneration)
        );
    }

    #[test]
    fn der_validation_requires_pinned_anchor() {
        let authority = CertificateAuthorityV1::new("a".repeat(64), 7).unwrap();
        let mut identity = identity(&"c".repeat(64), &"d".repeat(64), &"e".repeat(64));
        identity.certificate_sha256 = digest_bytes(&[1, 2, 3]);
        assert_eq!(
            authority.issue_der(
                &[identity],
                vec![1, 2, 3],
                Vec::new(),
                &"c".repeat(64),
                1_000
            ),
            Err(CertificateError::EndpointBindingUnavailable)
        );
    }

    #[test]
    fn rejects_malformed_chain_identity_and_unknown_role() {
        let authority = CertificateAuthorityV1::new("a".repeat(64), 7).unwrap();
        let base = identity(&"c".repeat(64), &"d".repeat(64), &"e".repeat(64));
        let mut unsupported = base.clone();
        unsupported.schema_version = 2;
        assert_eq!(
            authority.validate_metadata(&[unsupported], &"c".repeat(64), 1_000),
            Err(CertificateError::UnsupportedSchema)
        );
        let mut wrong_generation = base.clone();
        wrong_generation.generation = 8;
        assert_eq!(
            authority.validate_metadata(&[wrong_generation], &"c".repeat(64), 1_000),
            Err(CertificateError::InvalidGeneration)
        );
        let mut wrong_role = base.clone();
        wrong_role.role = "administrator\0".into();
        assert_eq!(
            authority.validate_metadata(&[wrong_role], &"c".repeat(64), 1_000),
            Err(CertificateError::InvalidRole)
        );
        let mut wrong_issuer = base.clone();
        wrong_issuer.issuer_sha256 = "f".repeat(64);
        let parent = identity(&"d".repeat(64), &"a".repeat(64), &"d".repeat(64));
        assert_eq!(
            authority.validate_metadata(&[wrong_issuer, parent], &"c".repeat(64), 1_000),
            Err(CertificateError::IssuerMismatch)
        );
        let too_long = vec![base; MAX_CHAIN_LENGTH + 1];
        assert_eq!(
            authority.validate_metadata(&too_long, &"c".repeat(64), 1_000),
            Err(CertificateError::InvalidChainLength)
        );
    }

    #[test]
    fn identity_schema_rejects_unknown_fields_and_endpoint_confusion() {
        let identity = identity(&"c".repeat(64), &"d".repeat(64), &"e".repeat(64));
        let mut value = serde_json::to_value(&identity).unwrap();
        value["untrusted"] = serde_json::json!(true);
        assert!(serde_json::from_value::<CertificateIdentityV1>(value).is_err());
        let schema = serde_json::to_value(crate::certificate_identity_schema()).unwrap();
        assert_eq!(
            schema["properties"]["schema_version"]["const"],
            serde_json::json!(1)
        );
        assert_eq!(
            schema["properties"]["generation"]["minimum"],
            serde_json::json!(1)
        );
        assert_eq!(
            schema["properties"]["subject_sha256"]["pattern"],
            serde_json::json!("^[0-9a-f]{64}$")
        );
        assert_eq!(
            schema["properties"]["role"]["enum"],
            serde_json::json!(["observer", "operator", "administrator"])
        );
    }
}
