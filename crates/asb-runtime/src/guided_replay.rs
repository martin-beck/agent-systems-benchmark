// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Runtime-owned local strict replay entrypoint.

use asb_replay::{
    CassetteLimits, ReplayHttpRequest, ReplayLimits, ReplayRoute, StrictReplayService,
};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;

/// Maximum cassette bytes accepted by the guided local entrypoint.
pub const MAX_CASSETTE_BYTES: u64 = 64 * 1024 * 1024;

/// Credential-free result produced by one bounded local replay.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalReplayResult {
    /// Digest of the status and response segments.
    pub result_digest: String,
    /// Total response bytes returned by the replay service.
    pub output_bytes: u64,
}

/// Fail-closed reasons from the guided replay boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalReplayError {
    /// The cassette was missing, not a regular file, oversized, or unreadable.
    CassetteUnavailable,
    /// The cassette was malformed or failed strict replay validation.
    InvalidCassette,
    /// The cassette digest did not match the admitted plan.
    DigestMismatch,
}

/// Runtime-owned strict replay entrypoint.
///
/// The caller supplies only a cassette path and the digest already bound into
/// the admitted plan. The cassette is validated before the replay service is
/// created, and no provider or network fallback exists in this path.
pub fn execute_local_strict_replay(
    cassette_path: &Path,
    expected_digest: &str,
) -> Result<LocalReplayResult, LocalReplayError> {
    let metadata =
        fs::symlink_metadata(cassette_path).map_err(|_| LocalReplayError::CassetteUnavailable)?;
    if !metadata.is_file() || metadata.len() > MAX_CASSETTE_BYTES {
        return Err(LocalReplayError::CassetteUnavailable);
    }
    let bytes = fs::read(cassette_path).map_err(|_| LocalReplayError::CassetteUnavailable)?;
    let cassette = asb_replay::decode_cassette(&bytes, CassetteLimits::default())
        .map_err(|_| LocalReplayError::InvalidCassette)?;
    if cassette.integrity.digest != expected_digest {
        return Err(LocalReplayError::DigestMismatch);
    }
    let interaction = cassette
        .contents
        .interactions
        .first()
        .ok_or(LocalReplayError::InvalidCassette)?;
    let route = ReplayRoute {
        session_id: interaction.session_id.clone(),
        attempt_id: interaction.attempt_id.clone(),
        dialect: interaction.dialect,
    };
    let request = ReplayHttpRequest {
        method: interaction.request.method.clone(),
        path: interaction.request.path.clone(),
        headers: interaction.request.headers.clone(),
        body: asb_replay::canonical_json_bytes(&interaction.request.body)
            .map_err(|_| LocalReplayError::InvalidCassette)?,
    };
    let service = StrictReplayService::new(cassette, ReplayLimits::default())
        .map_err(|_| LocalReplayError::InvalidCassette)?;
    let response = service
        .handle(&route, request)
        .map_err(|_| LocalReplayError::InvalidCassette)?;
    let output_bytes = response
        .segments
        .iter()
        .try_fold(0_u64, |total, segment| {
            total.checked_add(segment.len() as u64)
        })
        .ok_or(LocalReplayError::InvalidCassette)?;
    let mut digest = Sha256::new();
    digest.update(response.status.to_be_bytes());
    for segment in &response.segments {
        digest.update(segment);
    }
    Ok(LocalReplayResult {
        result_digest: format!("{:x}", digest.finalize()),
        output_bytes,
    })
}
