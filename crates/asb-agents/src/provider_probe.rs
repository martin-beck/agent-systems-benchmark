// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Bounded provider authentication probe result classification.

/// Maximum response body accepted by a provider authentication probe.
pub const MAX_PROBE_BODY_BYTES: usize = 16 * 1024;

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
}
