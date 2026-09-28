// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Runtime-owned provider capture handoff.
//!
//! The control service receives this trait only from the runtime.  Frontends
//! and path-based recording importers never construct a capture authority.

use crate::live_service::{LOCAL_PROVIDER_MOCK_MODEL, LocalProviderMockBackend};
use asb_replay::{
    ReplayError, ReplayHttpRequest, ReplayHttpResponse, ReplayLimits, ReplayRoute,
    StrictReplayService,
};
use sha2::{Digest, Sha256};
use std::io::{Cursor, Read, Write};
use std::sync::Mutex;

/// Secret-free identity handed from the runtime launch to the capture seam.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderCaptureRequest {
    /// Authenticated provider profile identity.
    pub provider_profile_sha256: String,
    /// Selected adapter identity.
    pub agent_id: String,
    /// Workload identity.
    pub workload_id: String,
    /// Scorer revision bound to the workload.
    pub scorer_revision: String,
    /// Runtime-issued attempt identity.
    pub attempt_id: String,
    /// Runtime launch generation.
    pub generation: u64,
}

/// Non-sensitive result returned after the runtime has redacted and strict-
/// replay-validated one captured cassette.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderCaptureResult {
    /// Content-addressed cassette digest.
    pub cassette_sha256: String,
    /// The redaction boundary completed successfully.
    pub redaction_verified: bool,
    /// Strict replay accepted the sealed cassette.
    pub replay_verified: bool,
    /// Authenticated cassette bytes, retained only for runtime-owned storage.
    /// The control layer validates the digest again before persisting them.
    pub cassette_json: Vec<u8>,
}

/// Runtime-owned provider capture callback.
pub trait ProviderCapture: Send + Sync {
    /// Capture one tuple through an already authenticated provider launch.
    fn capture(
        &self,
        request: &ProviderCaptureRequest,
    ) -> Result<ProviderCaptureResult, ProviderCaptureError>;
}

/// Run one runtime-authenticated exchange through the strict replay capture
/// seam and return only public cassette verification metadata. The stream and
/// forwarding callback are supplied by the runtime launch; callers cannot
/// provide a path-based capture or bypass redaction and sealing.
pub fn capture_authenticated_connection<S, F>(
    service: &StrictReplayService,
    request: &ProviderCaptureRequest,
    stream: &mut S,
    route: &ReplayRoute,
    forward: F,
) -> Result<ProviderCaptureResult, ProviderCaptureError>
where
    S: Read + Write,
    F: FnOnce(&ReplayRoute, &ReplayHttpRequest) -> Result<ReplayHttpResponse, ReplayError>,
{
    if request.attempt_id != route.attempt_id {
        return Err(ProviderCaptureError::IdentityMismatch);
    }
    let (cassette, _report) = service
        .capture_and_seal_authenticated_connection(
            stream,
            route,
            format!("{}-capture", request.attempt_id),
            forward,
        )
        .map_err(|_| ProviderCaptureError::Verification)?;
    // Re-open through the strict service constructor before reporting the
    // cassette as replay-ready. This validates route, dialect, redaction and
    // integrity contracts without contacting a provider or retaining payloads.
    StrictReplayService::new(cassette.clone(), Default::default())
        .map_err(|_| ProviderCaptureError::Verification)?;
    Ok(ProviderCaptureResult {
        cassette_sha256: cassette.integrity.digest.clone(),
        redaction_verified: true,
        replay_verified: true,
        cassette_json: serde_json::to_vec(&cassette)
            .map_err(|_| ProviderCaptureError::Verification)?,
    })
}

/// Runtime-owned deterministic provider capture used by the local/mock lane.
///
/// It exercises the same bounded local authority used by local benchmark runs,
/// then sends the resulting credential-free exchange through the strict replay
/// capture boundary. It never accepts endpoint, credential, or egress input.
pub struct LocalMockProviderCapture {
    backend: Mutex<LocalProviderMockBackend>,
}

impl std::fmt::Debug for LocalMockProviderCapture {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("LocalMockProviderCapture(..)")
    }
}

impl LocalMockProviderCapture {
    /// Provision a fresh runtime-owned local/mock capture authority.
    pub fn provision() -> Result<Self, ProviderCaptureError> {
        LocalProviderMockBackend::provision()
            .map(|backend| Self {
                backend: Mutex::new(backend),
            })
            .map_err(|_| ProviderCaptureError::Unavailable)
    }
}

impl ProviderCapture for LocalMockProviderCapture {
    fn capture(
        &self,
        request: &ProviderCaptureRequest,
    ) -> Result<ProviderCaptureResult, ProviderCaptureError> {
        if request.agent_id.is_empty()
            || request.workload_id.is_empty()
            || request.scorer_revision.is_empty()
            || request.attempt_id.is_empty()
        {
            return Err(ProviderCaptureError::IdentityMismatch);
        }
        let mut attempt_digest = Sha256::new();
        attempt_digest.update(request.attempt_id.as_bytes());
        let bytes = attempt_digest.finalize();
        let mut attempt_id = u32::from_le_bytes(bytes[..4].try_into().unwrap_or([0; 4]));
        if attempt_id == 0 {
            attempt_id = 1;
        }
        let body = serde_json::to_vec(&serde_json::json!({
            "model": LOCAL_PROVIDER_MOCK_MODEL,
            "messages": [{
                "role": "user",
                "content": format!("{}:{}:{}", request.agent_id, request.workload_id, request.scorer_revision)
            }]
        }))
        .map_err(|_| ProviderCaptureError::Verification)?;
        let response_sha256 = self
            .backend
            .lock()
            .map_err(|_| ProviderCaptureError::Unavailable)?
            .issue_attempt(attempt_id)
            .map_err(|_| ProviderCaptureError::Unavailable)?
            .execute_default(&body)
            .map_err(|_| ProviderCaptureError::Unavailable)?
            .response_sha256()
            .to_owned();
        let request_bytes = format!(
            "POST /v1/chat/completions HTTP/1.1\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
            body.len(),
            String::from_utf8(body.clone()).map_err(|_| ProviderCaptureError::Verification)?
        )
        .into_bytes();
        let route = ReplayRoute {
            session_id: request.provider_profile_sha256.clone(),
            attempt_id: request.attempt_id.clone(),
            dialect: asb_replay::ProviderDialect::OpenaiChatCompletions,
        };
        let service = StrictReplayService::capture_only(ReplayLimits::default())
            .map_err(|_| ProviderCaptureError::Verification)?;
        let mut stream = Cursor::new(request_bytes);
        capture_authenticated_connection(&service, request, &mut stream, &route, |_, _| {
            Ok(ReplayHttpResponse {
                status: 200,
                headers: vec![("content-type", "application/json")]
                    .into_iter()
                    .map(|(name, value)| asb_replay::Header {
                        name: name.to_owned(),
                        value: value.to_owned(),
                    })
                    .collect(),
                segments: vec![
                    serde_json::to_vec(&serde_json::json!({
                        "id": format!("mock-{}", request.attempt_id),
                        "choices": [],
                        "asb_response_sha256": response_sha256,
                    }))
                    .map_err(|_| ReplayError::InvalidHttp)?,
                ],
                recorded_offsets: vec![std::time::Duration::ZERO],
            })
        })
    }
}

/// Fail-closed capture errors.  No provider details or credentials are carried.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderCaptureError {
    /// No authenticated runtime launch was supplied.
    Unavailable,
    /// The runtime launch identity did not match the tuple.
    IdentityMismatch,
    /// Capture exceeded a bounded runtime policy.
    Bounds,
    /// Redaction or strict replay verification failed.
    Verification,
}

/// Default implementation used by the standalone control service until a
/// runtime-issued provider launch is attached.
#[derive(Debug, Default)]
pub struct UnavailableProviderCapture;

impl ProviderCapture for UnavailableProviderCapture {
    fn capture(
        &self,
        _request: &ProviderCaptureRequest,
    ) -> Result<ProviderCaptureResult, ProviderCaptureError> {
        Err(ProviderCaptureError::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_mock_capture_seals_replayable_secret_free_cassette() {
        let capture = LocalMockProviderCapture::provision().expect("local authority");
        let result = capture
            .capture(&ProviderCaptureRequest {
                provider_profile_sha256: "a".repeat(64),
                agent_id: "aider".into(),
                workload_id: "original.bug-fix".into(),
                scorer_revision: "scorer-v1".into(),
                attempt_id: "campaign-aider-original.bug-fix-attempt".into(),
                generation: 1,
            })
            .expect("capture");
        assert!(result.redaction_verified);
        assert!(result.replay_verified);
        assert_eq!(result.cassette_sha256.len(), 64);
        let cassette = asb_replay::decode_cassette(&result.cassette_json, Default::default())
            .expect("sealed cassette");
        assert_eq!(cassette.integrity.digest, result.cassette_sha256);
        let encoded = String::from_utf8(result.cassette_json).expect("json");
        assert!(!encoded.contains("Bearer"));
        assert!(!encoded.contains("secret"));
    }

    #[test]
    fn local_mock_capture_fences_route_identity() {
        let capture = LocalMockProviderCapture::provision().expect("local authority");
        let error = capture.capture(&ProviderCaptureRequest {
            provider_profile_sha256: "b".repeat(64),
            agent_id: "aider".into(),
            workload_id: "w".into(),
            scorer_revision: "s".into(),
            attempt_id: "".into(),
            generation: 1,
        });
        assert_eq!(error, Err(ProviderCaptureError::IdentityMismatch));
    }
}
