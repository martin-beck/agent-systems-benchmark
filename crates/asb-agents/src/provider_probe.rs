// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Bounded provider authentication probe result classification.

/// Maximum response body accepted by a provider authentication probe.
pub const MAX_PROBE_BODY_BYTES: usize = 16 * 1024;
/// Maximum probe deadline in milliseconds.
pub const MAX_PROBE_TIMEOUT_MS: u64 = 30_000;

/// Bounded, endpoint-pinned probe request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProbeRequest {
    /// Provider protocol family.
    pub provider: ProbeProvider,
    /// Credential-free endpoint identity digest.
    pub endpoint_identity_sha256: String,
    /// Enrollment generation being probed.
    pub generation: u64,
    /// End-to-end deadline.
    pub timeout_ms: u64,
    /// Maximum response body the transport may buffer.
    pub max_response_bytes: usize,
}

impl ProbeRequest {
    /// Validate bounds before a transport is opened.
    pub fn validate(&self) -> Result<(), ProbeRequestError> {
        if self.endpoint_identity_sha256.len() != 64
            || !self
                .endpoint_identity_sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(ProbeRequestError::InvalidEndpointIdentity);
        }
        if self.generation == 0 {
            return Err(ProbeRequestError::InvalidGeneration);
        }
        if self.timeout_ms == 0 || self.timeout_ms > MAX_PROBE_TIMEOUT_MS {
            return Err(ProbeRequestError::InvalidTimeout);
        }
        if self.max_response_bytes == 0 || self.max_response_bytes > MAX_PROBE_BODY_BYTES {
            return Err(ProbeRequestError::InvalidResponseLimit);
        }
        Ok(())
    }
}

/// Probe request validation failure without endpoint or provider disclosure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProbeRequestError {
    /// Endpoint identity is not a lowercase SHA-256 digest.
    InvalidEndpointIdentity,
    /// Generation zero is never a valid enrollment.
    InvalidGeneration,
    /// Deadline is outside the bounded range.
    InvalidTimeout,
    /// Response limit is outside the bounded range.
    InvalidResponseLimit,
}

/// Provider protocol family whose authentication response is being classified.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProbeProvider {
    /// OpenAI-compatible JSON endpoint.
    OpenAi,
    /// Gemini JSON endpoint.
    Gemini,
    /// Ollama local API endpoint.
    Ollama,
}

/// Public, body-free probe outcome.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProbeOutcome {
    /// Authentication and protocol identity were accepted.
    Connected,
    /// The provider explicitly rejected authentication.
    Rejected,
    /// The credential is expired or no longer usable.
    Expired,
    /// The endpoint or qualified transport was unavailable.
    Unavailable,
}

/// Classify one already-bounded HTTP probe response.
///
/// Callers must perform endpoint pinning, TLS/transport policy, timeout and
/// response-size enforcement before invoking this function. The response body
/// is represented only by its length, so no provider text can enter evidence.
pub fn classify_probe(
    provider: ProbeProvider,
    status: u16,
    body_len: usize,
    content_type_json: bool,
) -> ProbeOutcome {
    if body_len > MAX_PROBE_BODY_BYTES || !content_type_json {
        return ProbeOutcome::Unavailable;
    }
    match provider {
        ProbeProvider::OpenAi | ProbeProvider::Gemini => match status {
            200..=299 => ProbeOutcome::Connected,
            401 | 403 => ProbeOutcome::Rejected,
            408 | 425 | 429 | 500..=599 => ProbeOutcome::Unavailable,
            _ => ProbeOutcome::Rejected,
        },
        ProbeProvider::Ollama => match status {
            200..=299 => ProbeOutcome::Connected,
            401 | 403 => ProbeOutcome::Rejected,
            404 => ProbeOutcome::Unavailable,
            408 | 425 | 429 | 500..=599 => ProbeOutcome::Unavailable,
            _ => ProbeOutcome::Rejected,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_specific_success_and_auth_failures_are_body_free() {
        assert_eq!(
            classify_probe(ProbeProvider::OpenAi, 200, 128, true),
            ProbeOutcome::Connected
        );
        assert_eq!(
            classify_probe(ProbeProvider::Gemini, 401, 12, true),
            ProbeOutcome::Rejected
        );
        assert_eq!(
            classify_probe(ProbeProvider::Ollama, 404, 12, true),
            ProbeOutcome::Unavailable
        );
    }

    #[test]
    fn malformed_or_oversized_responses_fail_closed() {
        assert_eq!(
            classify_probe(ProbeProvider::OpenAi, 200, MAX_PROBE_BODY_BYTES + 1, true),
            ProbeOutcome::Unavailable
        );
        assert_eq!(
            classify_probe(ProbeProvider::Gemini, 200, 0, false),
            ProbeOutcome::Unavailable
        );
        assert_eq!(
            classify_probe(ProbeProvider::Ollama, 503, 10, true),
            ProbeOutcome::Unavailable
        );
    }

    #[test]
    fn probe_requests_validate_before_transport() {
        let valid = ProbeRequest {
            provider: ProbeProvider::OpenAi,
            endpoint_identity_sha256: "a".repeat(64),
            generation: 1,
            timeout_ms: 1_000,
            max_response_bytes: 1024,
        };
        assert_eq!(valid.validate(), Ok(()));
        let mut invalid = valid.clone();
        invalid.generation = 0;
        assert_eq!(
            invalid.validate(),
            Err(ProbeRequestError::InvalidGeneration)
        );
        invalid = valid.clone();
        invalid.timeout_ms = MAX_PROBE_TIMEOUT_MS + 1;
        assert_eq!(invalid.validate(), Err(ProbeRequestError::InvalidTimeout));
        invalid = valid.clone();
        invalid.max_response_bytes = MAX_PROBE_BODY_BYTES + 1;
        assert_eq!(
            invalid.validate(),
            Err(ProbeRequestError::InvalidResponseLimit)
        );
    }
}
