// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Strict, offline replay plan resolution for the CLI.

use asb_agents::strict_replay::{EgressPolicy, StrictReplayLaunchRecord, StrictReplayLaunchV1};
use asb_replay::{Cassette, CassetteLimits, decode_cassette};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Component, Path, PathBuf};
use thiserror::Error;

/// Current CLI strict-replay plan schema.
pub const STRICT_REPLAY_PLAN_V1: u16 = 1;
/// Maximum accepted cassette artifact size.
pub const MAX_CASSETTE_BYTES: u64 = 64 * 1024 * 1024;

/// A bounded plan whose artifact path is resolved only below an explicit root.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StrictReplayPlanV1 {
    /// Contract version.
    pub schema_version: u16,
    /// Relative cassette path below the caller-provided artifact root.
    pub cassette_path: String,
    /// Expected cassette content digest.
    pub cassette_sha256: String,
    /// Expected stable cassette identity.
    pub cassette_id: String,
    /// Authenticated route digest.
    pub route_sha256: String,
    /// Provider dialect selected by the recording.
    pub provider_dialect: String,
    /// Adapter identity.
    pub adapter: String,
    /// Stable run identity.
    pub run_id: String,
    /// Stable attempt identity.
    pub attempt_id: String,
    /// Workload content digest.
    pub workload_sha256: String,
    /// Pinned adapter command digest.
    pub command_sha256: String,
    /// Required process egress policy.
    pub egress: EgressPolicy,
    /// Hard process lifetime in milliseconds.
    pub timeout_ms: u64,
}

/// Verified cassette and authenticated runtime launch record.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedStrictReplay {
    /// Decoded, integrity-checked cassette.
    pub cassette: Cassette,
    /// Launch record consumed by the runtime-owned handoff seam.
    pub launch: StrictReplayLaunchRecord,
    /// Canonical artifact path used for the bounded resolution.
    pub artifact_path: PathBuf,
}

/// Fail-closed plan or artifact resolution error.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ReplayContractError {
    /// Plan schema or identity is invalid.
    #[error("strict replay plan is invalid")]
    InvalidPlan,
    /// Artifact path is not confined below the supplied root.
    #[error("cassette artifact path is unsafe")]
    UnsafePath,
    /// Artifact is missing, a symlink, or not a regular file.
    #[error("cassette artifact is unavailable")]
    Unavailable,
    /// Artifact exceeds the bounded size.
    #[error("cassette artifact exceeds the size bound")]
    Oversized,
    /// Artifact digest differs from the declared identity.
    #[error("cassette artifact digest differs")]
    DigestMismatch,
    /// Cassette decoding or strict validation failed.
    #[error("cassette artifact is malformed")]
    Malformed,
}

/// Resolve and authenticate one strict replay plan without ambient configuration.
pub fn resolve_strict_replay(
    plan: &StrictReplayPlanV1,
    artifact_root: &Path,
) -> Result<ResolvedStrictReplay, ReplayContractError> {
    validate_plan(plan)?;
    let root = fs::canonicalize(artifact_root).map_err(|_| ReplayContractError::Unavailable)?;
    let relative = Path::new(&plan.cassette_path);
    if relative.is_absolute()
        || relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(ReplayContractError::UnsafePath);
    }
    let mut current = root.clone();
    for part in relative.components() {
        current.push(part);
        let metadata =
            fs::symlink_metadata(&current).map_err(|_| ReplayContractError::Unavailable)?;
        if metadata.file_type().is_symlink() {
            return Err(ReplayContractError::UnsafePath);
        }
    }
    let path = current;
    let metadata = fs::metadata(&path).map_err(|_| ReplayContractError::Unavailable)?;
    if !metadata.is_file() {
        return Err(ReplayContractError::Unavailable);
    }
    if metadata.len() > MAX_CASSETTE_BYTES {
        return Err(ReplayContractError::Oversized);
    }
    let bytes = fs::read(&path).map_err(|_| ReplayContractError::Unavailable)?;
    if format!("{:x}", Sha256::digest(&bytes)) != plan.cassette_sha256 {
        return Err(ReplayContractError::DigestMismatch);
    }
    let cassette = decode_cassette(&bytes, CassetteLimits::default())
        .map_err(|_| ReplayContractError::Malformed)?;
    if cassette.contents.cassette_id != plan.cassette_id {
        return Err(ReplayContractError::DigestMismatch);
    }
    let input = StrictReplayLaunchV1 {
        schema_version: 1,
        cassette_sha256: plan.cassette_sha256.clone(),
        route_sha256: plan.route_sha256.clone(),
        provider_dialect: plan.provider_dialect.clone(),
        adapter: plan.adapter.clone(),
        run_id: plan.run_id.clone(),
        attempt_id: plan.attempt_id.clone(),
        workload_sha256: plan.workload_sha256.clone(),
        command_sha256: plan.command_sha256.clone(),
        egress: plan.egress,
        timeout_ms: plan.timeout_ms,
    };
    let launch = StrictReplayLaunchRecord {
        launch_sha256: input
            .digest()
            .map_err(|_| ReplayContractError::InvalidPlan)?,
        input,
    };
    launch
        .validate()
        .map_err(|_| ReplayContractError::InvalidPlan)?;
    Ok(ResolvedStrictReplay {
        cassette,
        launch,
        artifact_path: path,
    })
}

fn validate_plan(plan: &StrictReplayPlanV1) -> Result<(), ReplayContractError> {
    if plan.schema_version != STRICT_REPLAY_PLAN_V1
        || plan.cassette_path.is_empty()
        || plan.cassette_path.len() > 4096
        || plan.cassette_id.is_empty()
        || plan.cassette_id.len() > 128
        || plan.provider_dialect.is_empty()
        || plan.provider_dialect.len() > 128
        || plan.adapter.is_empty()
        || plan.adapter.len() > 128
        || plan.run_id.is_empty()
        || plan.run_id.len() > 128
        || plan.attempt_id.is_empty()
        || plan.attempt_id.len() > 128
        || !digest(&plan.cassette_sha256)
        || !digest(&plan.route_sha256)
        || !digest(&plan.workload_sha256)
        || !digest(&plan.command_sha256)
        || plan.timeout_ms == 0
        || plan.timeout_ms > 86_400_000
    {
        return Err(ReplayContractError::InvalidPlan);
    }
    Ok(())
}

fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use asb_replay::decode_cassette;

    fn plan(root: &Path) -> StrictReplayPlanV1 {
        let bytes = fs::read(root.join("cassette.json")).unwrap();
        let cassette = decode_cassette(&bytes, CassetteLimits::default()).unwrap();
        StrictReplayPlanV1 {
            schema_version: 1,
            cassette_path: "cassette.json".into(),
            cassette_sha256: format!("{:x}", Sha256::digest(&bytes)),
            cassette_id: cassette.contents.cassette_id,
            route_sha256: "b".repeat(64),
            provider_dialect: "openai-chat-v1".into(),
            adapter: "codex".into(),
            run_id: "run-1".into(),
            attempt_id: "attempt-1".into(),
            workload_sha256: "c".repeat(64),
            command_sha256: "d".repeat(64),
            egress: EgressPolicy::LoopbackOnly,
            timeout_ms: 5_000,
        }
    }

    fn fixture() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("cassette.json"),
            include_bytes!("../../asb-replay/fixtures/v1/buffered.json"),
        )
        .unwrap();
        root
    }

    #[test]
    fn resolves_verified_cassette_and_authenticated_launch() {
        let root = fixture();
        let resolved = resolve_strict_replay(&plan(root.path()), root.path()).unwrap();
        assert_eq!(resolved.launch.input.egress, EgressPolicy::LoopbackOnly);
        assert_eq!(resolved.launch.input.attempt_id, "attempt-1");
    }

    #[test]
    fn rejects_traversal_symlink_digest_and_unknown_egress() {
        let root = fixture();
        let mut candidate = plan(root.path());
        candidate.cassette_path = "../cassette.json".into();
        assert_eq!(
            resolve_strict_replay(&candidate, root.path()),
            Err(ReplayContractError::UnsafePath)
        );
        candidate = plan(root.path());
        candidate.cassette_sha256 = "e".repeat(64);
        assert_eq!(
            resolve_strict_replay(&candidate, root.path()),
            Err(ReplayContractError::DigestMismatch)
        );
        candidate = plan(root.path());
        candidate.schema_version = 2;
        assert_eq!(
            resolve_strict_replay(&candidate, root.path()),
            Err(ReplayContractError::InvalidPlan)
        );
    }
}
