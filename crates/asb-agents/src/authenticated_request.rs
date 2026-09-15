// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Fail-closed authenticated provider-request boundary.

use crate::credential::ResolvedCredential;
use sha2::{Digest, Sha256};

/// Maximum authenticated-request timeout.
pub const MAX_AUTH_REQUEST_TIMEOUT_MS: u64 = 30_000;
/// Maximum response body accepted by an authenticated request.
pub const MAX_AUTH_RESPONSE_BYTES: usize = 16 * 1024;
/// Maximum representable monotonic deadline accepted by the public schema.
pub const MAX_AUTH_DEADLINE_MS: u64 = i64::MAX as u64;

/// Provider authentication policy, selected explicitly by the validated profile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthPolicy {
    /// `Authorization: Bearer <credential>`.
    Bearer,
    /// `X-API-Key: <credential>`.
    ApiKey,
    /// No credential header for an explicitly unauthenticated Ollama deployment.
    None,
}

/// Provider family for authentication policy validation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthProvider {
    /// OpenAI-compatible bearer authentication.
    OpenAi,
    /// Gemini API-key authentication.
    Gemini,
    /// Ollama bearer authentication when an authenticated deployment requires it.
    Ollama,
}

/// Typed, credential-free authenticated request metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthenticatedRequest {
    /// Provider family.
    pub provider: AuthProvider,
    /// Lowercase SHA-256 identity of the exact endpoint string.
    pub endpoint_identity_sha256: String,
    /// Enrollment generation bound to the request.
    pub generation: u64,
    /// One bounded end-to-end request timeout.
    pub timeout_ms: u64,
    /// Absolute monotonic deadline in milliseconds supplied by the caller.
    pub deadline_ms: u64,
    /// Maximum response body retained by the transport.
    pub max_response_bytes: usize,
    /// Explicit provider authentication placement.
    pub policy: AuthPolicy,
}

impl AuthenticatedRequest {
    /// Validate all public request bounds before credential injection.
    pub fn validate(&self) -> Result<(), AuthRequestError> {
        if self.endpoint_identity_sha256.len() != 64
            || !self
                .endpoint_identity_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(AuthRequestError::InvalidEndpointIdentity);
        }
        if self.generation == 0 {
            return Err(AuthRequestError::InvalidGeneration);
        }
        if self.timeout_ms == 0 || self.timeout_ms > MAX_AUTH_REQUEST_TIMEOUT_MS {
            return Err(AuthRequestError::InvalidTimeout);
        }
        if self.deadline_ms == 0
            || self.deadline_ms > MAX_AUTH_DEADLINE_MS
            || self.deadline_ms < self.timeout_ms
        {
            return Err(AuthRequestError::InvalidDeadline);
        }
        if self.max_response_bytes == 0 || self.max_response_bytes > MAX_AUTH_RESPONSE_BYTES {
            return Err(AuthRequestError::InvalidResponseLimit);
        }
        if matches!(
            (self.provider, self.policy),
            (AuthProvider::Gemini, AuthPolicy::Bearer | AuthPolicy::None)
                | (AuthProvider::OpenAi, AuthPolicy::ApiKey | AuthPolicy::None)
                | (AuthProvider::Ollama, AuthPolicy::ApiKey)
        ) {
            return Err(AuthRequestError::UnsupportedPolicy);
        }
        Ok(())
    }

    /// Verify that the concrete endpoint is exactly the enrolled endpoint identity.
    pub fn validate_endpoint(&self, endpoint: &str) -> Result<(), AuthRequestError> {
        self.validate()?;
        if endpoint_identity_sha256(endpoint) != self.endpoint_identity_sha256 {
            return Err(AuthRequestError::EndpointIdentityMismatch);
        }
        Ok(())
    }

    /// Bind and consume a resolved credential at the final header-writing boundary.
    pub fn inject(
        self,
        endpoint: &str,
        current_generation: u64,
        now_ms: u64,
        credential: ResolvedCredential,
        sink: &mut impl HeaderSink,
        cancelled: impl Cancellation,
    ) -> Result<(), AuthRequestError> {
        self.validate_endpoint(endpoint)?;
        if current_generation != self.generation || cancelled.is_cancelled() {
            return Err(AuthRequestError::CancelledOrStale);
        }
        if now_ms >= self.deadline_ms || self.deadline_ms.saturating_sub(now_ms) > self.timeout_ms {
            return Err(AuthRequestError::DeadlineExceeded);
        }
        let mut bytes = credential.into_transport_bytes();
        if bytes.is_empty() && !matches!(self.policy, AuthPolicy::None) {
            bytes.fill(0);
            return Err(AuthRequestError::InvalidCredential);
        }
        let result = match self.policy {
            AuthPolicy::Bearer => sink.write_header(b"authorization", b"Bearer ", &bytes),
            AuthPolicy::ApiKey => sink.write_header(b"x-api-key", b"", &bytes),
            AuthPolicy::None => Ok(()),
        };
        bytes.fill(0);
        if cancelled.is_cancelled() {
            return Err(AuthRequestError::CancelledOrStale);
        }
        result.map_err(|_| AuthRequestError::TransportRejected)
    }
}

/// Final transport boundary; implementations must not retain the value after return.
pub trait HeaderSink {
    /// Write one authenticated header without exposing it to logs or evidence.
    fn write_header(
        &mut self,
        name: &[u8],
        prefix: &[u8],
        value: &[u8],
    ) -> Result<(), HeaderWriteError>;
}

/// Typed cancellation source checked before and after secret injection.
pub trait Cancellation {
    /// Return true when the request must fail closed.
    fn is_cancelled(&self) -> bool;
}

impl<F> Cancellation for F
where
    F: Fn() -> bool,
{
    fn is_cancelled(&self) -> bool {
        self()
    }
}

/// Failure reported by a final transport header sink.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HeaderWriteError;

/// Credential-free request validation/injection failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthRequestError {
    /// Endpoint identity is not a lowercase SHA-256 digest.
    InvalidEndpointIdentity,
    /// Generation zero is invalid.
    InvalidGeneration,
    /// Timeout is outside the bounded range.
    InvalidTimeout,
    /// Response limit is outside the bounded range.
    InvalidResponseLimit,
    /// Provider/policy combination is not qualified.
    UnsupportedPolicy,
    /// The final transport refused the header.
    TransportRejected,
    /// The opaque credential was empty.
    InvalidCredential,
    /// The concrete endpoint did not match its enrolled identity.
    EndpointIdentityMismatch,
    /// The request was cancelled or its enrollment generation became stale.
    CancelledOrStale,
    /// The absolute deadline has elapsed or is inconsistent with the timeout.
    InvalidDeadline,
    /// The absolute deadline has expired or exceeds the bounded timeout window.
    DeadlineExceeded,
}

/// Compute the endpoint identity used by [`AuthenticatedRequest`].
pub fn endpoint_identity_sha256(endpoint: &str) -> String {
    format!("{:x}", Sha256::digest(endpoint.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Sink {
        output: Vec<u8>,
        fail: bool,
    }
    impl HeaderSink for Sink {
        fn write_header(
            &mut self,
            name: &[u8],
            prefix: &[u8],
            value: &[u8],
        ) -> Result<(), HeaderWriteError> {
            if self.fail {
                return Err(HeaderWriteError);
            }
            self.output.extend_from_slice(name);
            self.output.extend_from_slice(b": ");
            self.output.extend_from_slice(prefix);
            self.output.extend_from_slice(value);
            Ok(())
        }
    }
    #[test]
    fn request_validates_before_injection() {
        let request = AuthenticatedRequest {
            provider: AuthProvider::OpenAi,
            endpoint_identity_sha256: endpoint_identity_sha256("http://127.0.0.1:9/v1/models"),
            generation: 1,
            timeout_ms: 1_000,
            deadline_ms: 2_000,
            max_response_bytes: 1024,
            policy: AuthPolicy::Bearer,
        };
        assert_eq!(request.validate(), Ok(()));
        assert!(
            request
                .validate_endpoint("http://127.0.0.1:9/v1/models")
                .is_ok()
        );
        assert_eq!(
            request.validate_endpoint("http://127.0.0.1:9/v1/chat"),
            Err(AuthRequestError::EndpointIdentityMismatch)
        );
        let mut sink = Sink {
            output: Vec::new(),
            fail: false,
        };
        request
            .clone()
            .inject(
                "http://127.0.0.1:9/v1/models",
                1,
                1_500,
                crate::credential::ResolvedCredential::from_test(b"secret"),
                &mut sink,
                || false,
            )
            .unwrap();
        assert_eq!(sink.output, b"authorization: Bearer secret");
        let mut failing = Sink {
            output: Vec::new(),
            fail: true,
        };
        assert_eq!(
            request.clone().inject(
                "http://127.0.0.1:9/v1/models",
                1,
                1_500,
                crate::credential::ResolvedCredential::from_test(b"secret"),
                &mut failing,
                || false,
            ),
            Err(AuthRequestError::TransportRejected)
        );
        assert_eq!(
            request.clone().inject(
                "http://127.0.0.1:9/v1/models",
                1,
                1_500,
                crate::credential::ResolvedCredential::from_test(b""),
                &mut sink,
                || false,
            ),
            Err(AuthRequestError::InvalidCredential)
        );
        assert_eq!(
            request.clone().inject(
                "http://127.0.0.1:9/v1/chat",
                1,
                1_500,
                crate::credential::ResolvedCredential::from_test(b"secret"),
                &mut sink,
                || false,
            ),
            Err(AuthRequestError::EndpointIdentityMismatch)
        );
        assert_eq!(
            request.clone().inject(
                "http://127.0.0.1:9/v1/models",
                2,
                1_500,
                crate::credential::ResolvedCredential::from_test(b"secret"),
                &mut sink,
                || false,
            ),
            Err(AuthRequestError::CancelledOrStale)
        );
        assert_eq!(
            request.clone().inject(
                "http://127.0.0.1:9/v1/models",
                1,
                2_000,
                crate::credential::ResolvedCredential::from_test(b"secret"),
                &mut sink,
                || false,
            ),
            Err(AuthRequestError::DeadlineExceeded)
        );
    }

    #[test]
    fn rejects_unqualified_policy_and_bounds() {
        let mut request = AuthenticatedRequest {
            provider: AuthProvider::Gemini,
            endpoint_identity_sha256: "a".repeat(64),
            generation: 1,
            timeout_ms: 1_000,
            deadline_ms: 2_000,
            max_response_bytes: 1024,
            policy: AuthPolicy::Bearer,
        };
        assert_eq!(request.validate(), Err(AuthRequestError::UnsupportedPolicy));
        request.policy = AuthPolicy::ApiKey;
        request.timeout_ms = 0;
        assert_eq!(request.validate(), Err(AuthRequestError::InvalidTimeout));
        request.timeout_ms = 1_000;
        request.deadline_ms = 0;
        assert_eq!(request.validate(), Err(AuthRequestError::InvalidDeadline));
        request.deadline_ms = MAX_AUTH_DEADLINE_MS + 1;
        assert_eq!(request.validate(), Err(AuthRequestError::InvalidDeadline));
    }

    #[test]
    fn schema_shape_rejects_unknown_fields() {
        let value: serde_json::Value = serde_json::json!({
            "provider": "open_ai",
            "endpoint_identity_sha256": "a".repeat(64),
            "generation": 1,
            "timeout_ms": 1000,
            "deadline_ms": 2000,
            "max_response_bytes": 1024,
            "policy": "bearer",
            "secret": "must-not-be-present"
        });
        let allowed = [
            "provider",
            "endpoint_identity_sha256",
            "generation",
            "timeout_ms",
            "deadline_ms",
            "max_response_bytes",
            "policy",
        ];
        assert!(
            !value
                .as_object()
                .unwrap()
                .keys()
                .all(|key| allowed.contains(&key.as_str()))
        );
        let schema: serde_json::Value = serde_json::from_str(include_str!(
            "../schema/authenticated-request-v1.schema.json"
        ))
        .unwrap();
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(schema["allOf"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn schema_instances_match_provider_policy_and_deadline_rules() {
        fn valid(instance: &serde_json::Value) -> bool {
            let object = instance.as_object().unwrap();
            let provider = object["provider"].as_str().unwrap();
            let policy = object["policy"].as_str().unwrap();
            let deadline = object["deadline_ms"].as_u64().unwrap_or(0);
            let timeout = object["timeout_ms"].as_u64().unwrap_or(0);
            deadline > 0
                && deadline <= MAX_AUTH_DEADLINE_MS
                && deadline >= timeout
                && match provider {
                    "open_ai" => policy == "bearer",
                    "gemini" => policy == "api_key",
                    "ollama" => policy == "bearer" || policy == "none",
                    _ => false,
                }
        }
        let base = serde_json::json!({
            "endpoint_identity_sha256": "a".repeat(64), "generation": 1,
            "timeout_ms": 1000, "deadline_ms": 2000, "max_response_bytes": 1024
        });
        for (provider, policy) in [
            ("open_ai", "bearer"),
            ("gemini", "api_key"),
            ("ollama", "none"),
        ] {
            let mut instance = base.clone();
            instance["provider"] = serde_json::json!(provider);
            instance["policy"] = serde_json::json!(policy);
            assert!(valid(&instance));
        }
        let mut invalid = base.clone();
        invalid["provider"] = serde_json::json!("gemini");
        invalid["policy"] = serde_json::json!("bearer");
        assert!(!valid(&invalid));
        invalid["policy"] = serde_json::json!("api_key");
        invalid["deadline_ms"] = serde_json::json!(0);
        assert!(!valid(&invalid));
        invalid["deadline_ms"] = serde_json::json!(MAX_AUTH_DEADLINE_MS + 1);
        assert!(!valid(&invalid));
    }
}
