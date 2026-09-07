// SPDX-License-Identifier: MIT
//! Fail-closed contracts for the optional CSB subprocess boundary.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};

use asb_protocol::{CallLimits, PROTOCOL_V1, ProtocolVersion};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Contract generation implemented by this crate.
pub const CSB_CONTRACT_V1: ProtocolVersion = PROTOCOL_V1;
/// Maximum count of arguments in one request.
pub const MAX_ARGUMENTS: usize = 128;
/// Maximum combined UTF-8 bytes of all arguments.
pub const MAX_ARGUMENT_BYTES: usize = 32 * 1024;
/// Maximum count of declared output artifacts.
pub const MAX_ARTIFACTS: usize = 256;
/// Maximum count of explicitly allowed environment entries.
pub const MAX_ENVIRONMENT: usize = 32;
/// Maximum byte length of any single identifier or relative path.
pub const MAX_COMPONENT_BYTES: usize = 1024;
/// Maximum operation identities retained by one negotiated subprocess session.
pub const MAX_SESSION_OPERATIONS: usize = 4096;

/// Only the independently reviewed CSB execution mode is admitted.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    /// Execute an already prepared external application without a nested scheduler.
    ExternalApplication,
}

/// Immutable public identities that must match the bytes used for execution.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CsbPin {
    /// Exact public CSB Git commit.
    pub source_commit: String,
    /// Exact Git tree for that commit.
    pub source_tree: String,
    /// SHA-256 of the executable file supplied to the sandbox.
    pub executable_sha256: String,
}

/// One bounded CSB invocation transported inside an ASB JSON-RPC request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunRequest {
    /// Contract version selected during negotiation.
    pub contract: ProtocolVersion,
    /// Stable causal identity for this effect.
    pub operation_id: String,
    /// Stable attempt identity used to fence stale retries.
    pub attempt_id: String,
    /// Immutable source and executable identities.
    pub pin: CsbPin,
    /// CSB operation admitted by this boundary.
    pub mode: ExecutionMode,
    /// Direct executable arguments; never interpreted by a shell.
    pub arguments: Vec<String>,
    /// Explicit non-secret environment passed after clearing the ambient environment.
    pub environment: BTreeMap<String, String>,
    /// Relative regular-file outputs that may be committed after verification.
    pub artifacts: Vec<String>,
}

/// Validated request whose private fields cannot be changed after admission.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedRun {
    request: RunRequest,
    request_sha256: String,
}

impl ValidatedRun {
    /// Return the immutable request.
    pub fn request(&self) -> &RunRequest {
        &self.request
    }

    /// Return the canonical JSON digest bound to durable pre-effect intent.
    pub fn request_sha256(&self) -> &str {
        &self.request_sha256
    }
}

/// Why a request was rejected before any external effect.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ValidationError {
    /// The request did not use the negotiated v1 contract.
    #[error("unsupported CSB contract version")]
    Version,
    /// A causal identity was empty, oversized, or unsafe.
    #[error("invalid {0} identity")]
    Identity(&'static str),
    /// A Git or executable digest was not lowercase hexadecimal of the exact size.
    #[error("invalid {0} digest")]
    Digest(&'static str),
    /// Argument count or encoded size exceeded the boundary.
    #[error("argument boundary exceeded")]
    Arguments,
    /// An environment name/value was not explicitly safe and bounded.
    #[error("invalid environment")]
    Environment,
    /// An output path was absolute, escaping, duplicated, or oversized.
    #[error("invalid artifact path")]
    Artifact,
    /// Canonical request serialization unexpectedly failed.
    #[error("request serialization failed")]
    Serialization,
}

/// A bounded protocol-session admission failure before any external effect.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum AdmissionError {
    /// A run was offered before the mandatory negotiation exchange.
    #[error("CSB protocol negotiation is required")]
    NegotiationRequired,
    /// A peer attempted to renegotiate a live session.
    #[error("CSB protocol is already negotiated")]
    AlreadyNegotiated,
    /// The peer offered an incompatible protocol generation.
    #[error("unsupported CSB protocol version")]
    Version,
    /// Either peer offered invalid call limits.
    #[error("invalid CSB protocol limits")]
    Limits,
    /// The in-flight or lifetime session bound is exhausted.
    #[error("CSB protocol capacity exhausted")]
    Capacity,
    /// The exact operation and attempt identity has already been admitted.
    #[error("duplicate CSB operation")]
    Duplicate,
    /// An operation identity was reused for a different attempt.
    #[error("stale CSB attempt")]
    StaleAttempt,
    /// Completion did not name an active operation.
    #[error("unknown CSB operation")]
    UnknownOperation,
    /// The request body violated the execution contract.
    #[error(transparent)]
    Validation(#[from] ValidationError),
}

/// Negotiation and causal-ID fence for one bounded subprocess session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundarySession {
    local_limits: CallLimits,
    negotiated: Option<CallLimits>,
    active: BTreeMap<String, String>,
    admitted: BTreeSet<(String, String)>,
}

impl BoundarySession {
    /// Create a session with validated local hard limits.
    pub fn new(local_limits: CallLimits) -> Result<Self, AdmissionError> {
        local_limits
            .validate()
            .map_err(|_| AdmissionError::Limits)?;
        Ok(Self {
            local_limits,
            negotiated: None,
            active: BTreeMap::new(),
            admitted: BTreeSet::new(),
        })
    }

    /// Select the common v1 protocol and the stricter peer limits exactly once.
    pub fn negotiate(
        &mut self,
        protocol: ProtocolVersion,
        peer_limits: CallLimits,
    ) -> Result<CallLimits, AdmissionError> {
        if self.negotiated.is_some() {
            return Err(AdmissionError::AlreadyNegotiated);
        }
        protocol.negotiate().map_err(|_| AdmissionError::Version)?;
        let limits = self
            .local_limits
            .intersect(peer_limits)
            .map_err(|_| AdmissionError::Limits)?;
        self.negotiated = Some(limits);
        Ok(limits)
    }

    /// Validate and reserve one causal operation before durable execution intent.
    pub fn admit(&mut self, request: RunRequest) -> Result<ValidatedRun, AdmissionError> {
        let limits = self.negotiated.ok_or(AdmissionError::NegotiationRequired)?;
        let key = (request.operation_id.clone(), request.attempt_id.clone());
        if let Some(active_attempt) = self.active.get(&request.operation_id) {
            return Err(if active_attempt == &request.attempt_id {
                AdmissionError::Duplicate
            } else {
                AdmissionError::StaleAttempt
            });
        }
        if self.admitted.contains(&key) {
            return Err(AdmissionError::Duplicate);
        }
        if self.active.len() >= usize::from(limits.max_in_flight)
            || self.admitted.len() >= MAX_SESSION_OPERATIONS
        {
            return Err(AdmissionError::Capacity);
        }
        let validated = request.validate(limits)?;
        self.active.insert(key.0.clone(), key.1.clone());
        self.admitted.insert(key);
        Ok(validated)
    }

    /// Release exactly the active causal identity after terminal evidence is durable.
    pub fn finish(&mut self, operation_id: &str, attempt_id: &str) -> Result<(), AdmissionError> {
        match self.active.get(operation_id) {
            Some(active_attempt) if active_attempt == attempt_id => {
                self.active.remove(operation_id);
                Ok(())
            }
            Some(_) => Err(AdmissionError::StaleAttempt),
            None => Err(AdmissionError::UnknownOperation),
        }
    }

    /// Return negotiated limits, when negotiation has completed.
    pub fn negotiated_limits(&self) -> Option<CallLimits> {
        self.negotiated
    }

    /// Return the count of operations awaiting durable terminal evidence.
    pub fn active_operations(&self) -> usize {
        self.active.len()
    }
}

impl RunRequest {
    /// Validate all fields before durable intent or process creation.
    pub fn validate(self, negotiated: CallLimits) -> Result<ValidatedRun, ValidationError> {
        if self.contract != CSB_CONTRACT_V1 || negotiated.validate().is_err() {
            return Err(ValidationError::Version);
        }
        validate_identity("operation", &self.operation_id)?;
        validate_identity("attempt", &self.attempt_id)?;
        if !is_hex(&self.pin.source_commit, 40) {
            return Err(ValidationError::Digest("source commit"));
        }
        if !is_hex(&self.pin.source_tree, 40) {
            return Err(ValidationError::Digest("source tree"));
        }
        if !is_hex(&self.pin.executable_sha256, 64) {
            return Err(ValidationError::Digest("executable"));
        }
        validate_arguments(&self.arguments)?;
        validate_environment(&self.environment)?;
        validate_artifacts(&self.artifacts)?;

        let encoded = serde_json::to_vec(&self).map_err(|_| ValidationError::Serialization)?;
        if encoded.len() > negotiated.max_frame_bytes as usize {
            return Err(ValidationError::Arguments);
        }
        let request_sha256 = format!("{:x}", Sha256::digest(&encoded));
        Ok(ValidatedRun {
            request: self,
            request_sha256,
        })
    }
}

fn validate_identity(kind: &'static str, value: &str) -> Result<(), ValidationError> {
    if value.is_empty()
        || value.len() > MAX_COMPONENT_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(ValidationError::Identity(kind));
    }
    Ok(())
}

fn is_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate_arguments(arguments: &[String]) -> Result<(), ValidationError> {
    let size = arguments.iter().try_fold(0_usize, |total, argument| {
        if argument.as_bytes().contains(&0) || argument.len() > MAX_COMPONENT_BYTES {
            None
        } else {
            total.checked_add(argument.len())
        }
    });
    if arguments.len() > MAX_ARGUMENTS || size.is_none_or(|size| size > MAX_ARGUMENT_BYTES) {
        return Err(ValidationError::Arguments);
    }
    Ok(())
}

fn validate_environment(environment: &BTreeMap<String, String>) -> Result<(), ValidationError> {
    const ALLOWED: &[&str] = &["CSB_RUN_ID", "CSB_SEED", "LANG", "LC_ALL", "TZ"];
    if environment.len() > MAX_ENVIRONMENT
        || environment.iter().any(|(name, value)| {
            !ALLOWED.contains(&name.as_str())
                || value.len() > MAX_COMPONENT_BYTES
                || value.as_bytes().contains(&0)
        })
    {
        return Err(ValidationError::Environment);
    }
    Ok(())
}

fn validate_artifacts(artifacts: &[String]) -> Result<(), ValidationError> {
    if artifacts.len() > MAX_ARTIFACTS {
        return Err(ValidationError::Artifact);
    }
    let mut prior: Option<&str> = None;
    for artifact in artifacts {
        let path = Path::new(artifact);
        if artifact.is_empty()
            || artifact.len() > MAX_COMPONENT_BYTES
            || path.is_absolute()
            || path
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
            || prior.is_some_and(|value| value >= artifact.as_str())
        {
            return Err(ValidationError::Artifact);
        }
        prior = Some(artifact);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> CallLimits {
        CallLimits {
            max_frame_bytes: 64 * 1024,
            timeout_ms: 1_000,
            max_in_flight: 1,
        }
    }

    fn request() -> RunRequest {
        RunRequest {
            contract: CSB_CONTRACT_V1,
            operation_id: "operation-1".into(),
            attempt_id: "attempt-1".into(),
            pin: CsbPin {
                source_commit: "d577c5249501b29e33a87524a677a101477d5579".into(),
                source_tree: "97d08b39026f7c7d3e6f748b26a20e819a92184f".into(),
                executable_sha256:
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            },
            mode: ExecutionMode::ExternalApplication,
            arguments: vec!["--json".into()],
            environment: BTreeMap::from([("TZ".into(), "UTC".into())]),
            artifacts: vec!["result/summary.json".into()],
        }
    }

    #[test]
    fn accepts_bounded_pinned_external_application() {
        let validated = request().validate(limits()).unwrap();
        assert_eq!(validated.request(), &request());
        assert_eq!(validated.request_sha256().len(), 64);
    }

    #[test]
    fn rejects_unknown_contract_and_invalid_limits() {
        let mut value = request();
        value.contract.major = 2;
        assert_eq!(value.validate(limits()), Err(ValidationError::Version));
        let mut invalid = limits();
        invalid.max_frame_bytes = 0;
        assert_eq!(request().validate(invalid), Err(ValidationError::Version));
    }

    #[test]
    fn rejects_untrusted_identity_and_pin_shapes() {
        let mut value = request();
        value.operation_id = "../effect".into();
        assert_eq!(
            value.validate(limits()),
            Err(ValidationError::Identity("operation"))
        );
        let mut value = request();
        value.pin.executable_sha256 = "A".repeat(64);
        assert_eq!(
            value.validate(limits()),
            Err(ValidationError::Digest("executable"))
        );
    }

    #[test]
    fn rejects_argument_floods_and_nul() {
        let mut value = request();
        value.arguments = vec!["x".into(); MAX_ARGUMENTS + 1];
        assert_eq!(value.validate(limits()), Err(ValidationError::Arguments));
        let mut value = request();
        value.arguments = vec!["bad\0argument".into()];
        assert_eq!(value.validate(limits()), Err(ValidationError::Arguments));
    }

    #[test]
    fn rejects_credentials_and_ambient_configuration() {
        for name in ["HOME", "PATH", "AWS_ACCESS_KEY_ID", "GITHUB_TOKEN"] {
            let mut value = request();
            value.environment = BTreeMap::from([(name.into(), "public-sentinel".into())]);
            assert_eq!(value.validate(limits()), Err(ValidationError::Environment));
        }
    }

    #[test]
    fn rejects_escaping_duplicate_and_unsorted_artifacts() {
        for artifacts in [
            vec!["../secret".into()],
            vec!["/absolute".into()],
            vec!["same".into(), "same".into()],
            vec!["z".into(), "a".into()],
        ] {
            let mut value = request();
            value.artifacts = artifacts;
            assert_eq!(value.validate(limits()), Err(ValidationError::Artifact));
        }
    }

    #[test]
    fn serde_rejects_unknown_fields_and_modes() {
        let mut json = serde_json::to_value(request()).unwrap();
        json.as_object_mut()
            .unwrap()
            .insert("secret".into(), true.into());
        assert!(serde_json::from_value::<RunRequest>(json).is_err());
        let json = serde_json::to_string(&request())
            .unwrap()
            .replace("external_application", "generator");
        assert!(serde_json::from_str::<RunRequest>(&json).is_err());
    }

    #[test]
    fn negotiation_is_mandatory_single_use_and_strict() {
        let mut session = BoundarySession::new(limits()).unwrap();
        assert_eq!(
            session.admit(request()),
            Err(AdmissionError::NegotiationRequired)
        );
        let peer = CallLimits {
            max_frame_bytes: 4096,
            timeout_ms: 500,
            max_in_flight: 1,
        };
        assert_eq!(session.negotiate(CSB_CONTRACT_V1, peer), Ok(peer));
        assert_eq!(session.negotiated_limits(), Some(peer));
        assert_eq!(
            session.negotiate(CSB_CONTRACT_V1, peer),
            Err(AdmissionError::AlreadyNegotiated)
        );

        let mut wrong = BoundarySession::new(limits()).unwrap();
        assert_eq!(
            wrong.negotiate(ProtocolVersion { major: 2, minor: 0 }, peer),
            Err(AdmissionError::Version)
        );
        let mut invalid = BoundarySession::new(limits()).unwrap();
        assert_eq!(
            invalid.negotiate(
                CSB_CONTRACT_V1,
                CallLimits {
                    max_frame_bytes: 0,
                    ..peer
                }
            ),
            Err(AdmissionError::Limits)
        );
        assert_eq!(
            BoundarySession::new(CallLimits {
                max_in_flight: 0,
                ..limits()
            }),
            Err(AdmissionError::Limits)
        );
    }

    #[test]
    fn duplicate_stale_and_unknown_operations_fail_closed() {
        let mut session = BoundarySession::new(limits()).unwrap();
        session.negotiate(CSB_CONTRACT_V1, limits()).unwrap();
        session.admit(request()).unwrap();
        assert_eq!(session.active_operations(), 1);
        assert_eq!(session.admit(request()), Err(AdmissionError::Duplicate));
        let mut stale = request();
        stale.attempt_id = "attempt-2".into();
        assert_eq!(session.admit(stale), Err(AdmissionError::StaleAttempt));
        assert_eq!(
            session.finish("operation-1", "attempt-2"),
            Err(AdmissionError::StaleAttempt)
        );
        session.finish("operation-1", "attempt-1").unwrap();
        assert_eq!(session.active_operations(), 0);
        assert_eq!(session.admit(request()), Err(AdmissionError::Duplicate));
        assert_eq!(
            session.finish("missing", "attempt-1"),
            Err(AdmissionError::UnknownOperation)
        );
    }

    #[test]
    fn negotiated_in_flight_capacity_is_enforced() {
        let mut session = BoundarySession::new(limits()).unwrap();
        session.negotiate(CSB_CONTRACT_V1, limits()).unwrap();
        session.admit(request()).unwrap();
        let mut second = request();
        second.operation_id = "operation-2".into();
        second.attempt_id = "attempt-2".into();
        assert_eq!(session.admit(second.clone()), Err(AdmissionError::Capacity));
        session.finish("operation-1", "attempt-1").unwrap();
        assert!(session.admit(second).is_ok());
    }

    #[test]
    fn every_identity_and_digest_boundary_is_rejected() {
        for operation in [
            String::new(),
            "x".repeat(MAX_COMPONENT_BYTES + 1),
            "bad/id".into(),
        ] {
            let mut value = request();
            value.operation_id = operation;
            assert_eq!(
                value.validate(limits()),
                Err(ValidationError::Identity("operation"))
            );
        }
        let mut value = request();
        value.attempt_id = "bad attempt".into();
        assert_eq!(
            value.validate(limits()),
            Err(ValidationError::Identity("attempt"))
        );
        for (field, replacement) in [
            ("source commit", "a".repeat(39)),
            ("source tree", format!("{}g", "a".repeat(39))),
            ("executable", "a".repeat(63)),
        ] {
            let mut value = request();
            match field {
                "source commit" => value.pin.source_commit = replacement,
                "source tree" => value.pin.source_tree = replacement,
                "executable" => value.pin.executable_sha256 = replacement,
                _ => unreachable!(),
            }
            assert_eq!(
                value.validate(limits()),
                Err(ValidationError::Digest(field))
            );
        }
    }

    #[test]
    fn encoded_frame_and_each_argument_bound_fail_closed() {
        let mut tiny = limits();
        tiny.max_frame_bytes = 1;
        assert_eq!(request().validate(tiny), Err(ValidationError::Arguments));

        let mut value = request();
        value.arguments = vec!["x".repeat(MAX_COMPONENT_BYTES + 1)];
        assert_eq!(value.validate(limits()), Err(ValidationError::Arguments));
        let mut value = request();
        value.arguments = vec!["x".repeat(MAX_COMPONENT_BYTES); MAX_ARGUMENTS];
        assert_eq!(value.validate(limits()), Err(ValidationError::Arguments));
        let mut value = request();
        value.arguments = vec!["ok".into(); MAX_ARGUMENTS];
        assert!(value.validate(limits()).is_ok());
    }

    #[test]
    fn environment_values_are_bounded_and_nul_free() {
        for value_text in ["x".repeat(MAX_COMPONENT_BYTES + 1), "bad\0value".into()] {
            let mut value = request();
            value.environment = BTreeMap::from([("TZ".into(), value_text)]);
            assert_eq!(value.validate(limits()), Err(ValidationError::Environment));
        }
        let mut value = request();
        value.environment = BTreeMap::from([
            ("CSB_RUN_ID".into(), "run-1".into()),
            ("CSB_SEED".into(), "7".into()),
            ("LANG".into(), "C.UTF-8".into()),
            ("LC_ALL".into(), "C.UTF-8".into()),
            ("TZ".into(), "UTC".into()),
        ]);
        assert!(value.validate(limits()).is_ok());
    }

    #[test]
    fn every_artifact_shape_and_count_boundary_is_rejected() {
        for artifact in [
            String::new(),
            "x".repeat(MAX_COMPONENT_BYTES + 1),
            "./result".into(),
            "result/../secret".into(),
        ] {
            let mut value = request();
            value.artifacts = vec![artifact];
            assert_eq!(value.validate(limits()), Err(ValidationError::Artifact));
        }
        let mut value = request();
        value.artifacts = (0..=MAX_ARTIFACTS)
            .map(|index| format!("artifact-{index:03}"))
            .collect();
        assert_eq!(value.validate(limits()), Err(ValidationError::Artifact));
    }

    #[test]
    fn lifetime_operation_history_is_bounded() {
        let roomy = CallLimits {
            max_in_flight: 1,
            ..limits()
        };
        let mut session = BoundarySession::new(roomy).unwrap();
        session.negotiate(CSB_CONTRACT_V1, roomy).unwrap();
        for index in 0..MAX_SESSION_OPERATIONS {
            let mut value = request();
            value.operation_id = format!("operation-{index}");
            value.attempt_id = format!("attempt-{index}");
            session.admit(value).unwrap();
            session
                .finish(&format!("operation-{index}"), &format!("attempt-{index}"))
                .unwrap();
        }
        let mut value = request();
        value.operation_id = "operation-overflow".into();
        value.attempt_id = "attempt-overflow".into();
        assert_eq!(session.admit(value), Err(AdmissionError::Capacity));
    }
}
