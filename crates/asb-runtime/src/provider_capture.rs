// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Runtime-owned provider capture handoff.
//!
//! The control service receives this trait only from the runtime.  Frontends
//! and path-based recording importers never construct a capture authority.

use asb_replay::{
    ReplayError, ReplayHttpRequest, ReplayHttpResponse, ReplayRoute, StrictReplayService,
};
use std::io::{Read, Write};

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
        cassette_sha256: cassette.integrity.digest,
        redaction_verified: true,
        replay_verified: true,
    })
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
