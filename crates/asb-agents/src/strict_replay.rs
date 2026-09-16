// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Versioned adapter-facing strict replay launch contract.

use asb_replay::{
    Cassette, ReplayHttpRequest, ReplayHttpResponse, ReplayLimits, ReplayRoute, StrictReplayService,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

/// Current strict replay launch contract.
pub const STRICT_REPLAY_LAUNCH_V1: u16 = 1;

/// Process policy required for replay.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EgressPolicy {
    /// Provider egress is denied.
    LoopbackOnly,
}

/// Capability attested by the selected runtime sandbox.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessIsolationCapability {
    /// The child process boundary denies provider egress.
    Verified,
}

/// Bounded lifecycle state for one strict replay attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplayLifecycle {
    /// No adapter request has been served.
    Ready,
    /// The attempt was cancelled and cannot be reused.
    Cancelled,
    /// The attempt requires a fresh launch after restart recovery.
    NeedsRestart,
}

/// Credential-free, content-bound adapter launch input.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StrictReplayLaunchV1 {
    /// Contract version.
    pub schema_version: u16,
    /// Authenticated cassette digest.
    pub cassette_sha256: String,
    /// Authenticated local route digest.
    pub route_sha256: String,
    /// Provider protocol dialect.
    pub provider_dialect: String,
    /// Adapter identity.
    pub adapter: String,
    /// Stable run identity.
    pub run_id: String,
    /// Stable attempt identity.
    pub attempt_id: String,
    /// Workload digest.
    pub workload_sha256: String,
    /// Required process boundary policy.
    pub egress: EgressPolicy,
    /// Bounded process lifetime in milliseconds.
    pub timeout_ms: u64,
}

/// Authenticated launch record.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StrictReplayLaunchRecord {
    /// Exact launch input.
    pub input: StrictReplayLaunchV1,
    /// Canonical input digest.
    pub launch_sha256: String,
}

/// Launch validation failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StrictReplayError {
    /// The contract version is unsupported.
    UnsupportedVersion,
    /// An identity is malformed.
    InvalidIdentity,
    /// The timeout is outside its bound.
    InvalidTimeout,
    /// The input digest does not match.
    DigestMismatch,
    /// Provider egress was not denied.
    EgressNotDenied,
    /// The cassette could not be made available as a strict service.
    InvalidCassette,
    /// The route or request was rejected by the strict service.
    ServiceUnavailable,
    /// The route attempt does not match the launch attempt.
    AttemptMismatch,
    /// The supplied route is not the authenticated launch route.
    RouteMismatch,
    /// The adapter endpoint is outside the pinned local replay boundary.
    ExternalEndpoint,
    /// The runtime did not attest process-level egress isolation.
    IsolationUnavailable,
    /// The attempt lifecycle does not permit another operation.
    LifecycleClosed,
}

/// Bounded replay executor that has no live-provider fallback.
pub struct StrictReplayExecutor {
    record: StrictReplayLaunchRecord,
    service: StrictReplayService,
}

impl StrictReplayExecutor {
    /// Reject provider endpoints before an adapter process can be launched.
    pub fn validate_endpoint(endpoint: &str) -> Result<(), StrictReplayError> {
        let url = Url::parse(endpoint).map_err(|_| StrictReplayError::ExternalEndpoint)?;
        if url.scheme() != "http"
            || url.username() != ""
            || url.host_str() != Some("127.0.0.1")
            || url.port().is_none()
        {
            return Err(StrictReplayError::ExternalEndpoint);
        }
        Ok(())
    }

    /// Authenticate a launch record and bind one cassette to its local service.
    pub fn new(
        record: StrictReplayLaunchRecord,
        cassette: Cassette,
        isolation: Option<ProcessIsolationCapability>,
    ) -> Result<Self, StrictReplayError> {
        record.validate()?;
        if isolation != Some(ProcessIsolationCapability::Verified) {
            return Err(StrictReplayError::IsolationUnavailable);
        }
        if cassette.integrity.digest != record.input.cassette_sha256 {
            return Err(StrictReplayError::InvalidIdentity);
        }
        let service = StrictReplayService::new(cassette, ReplayLimits::default())
            .map_err(|_| StrictReplayError::InvalidCassette)?;
        Ok(Self { record, service })
    }

    /// Serve one adapter request only from the authenticated local cassette route.
    pub fn execute(
        &self,
        route: &ReplayRoute,
        request: ReplayHttpRequest,
    ) -> Result<ReplayHttpResponse, StrictReplayError> {
        if route.attempt_id != self.record.input.attempt_id {
            return Err(StrictReplayError::AttemptMismatch);
        }
        if route_digest(route) != self.record.input.route_sha256 {
            return Err(StrictReplayError::RouteMismatch);
        }
        self.service
            .handle(route, request)
            .map_err(|_| StrictReplayError::ServiceUnavailable)
    }
}

fn route_digest(route: &ReplayRoute) -> String {
    let mut digest = Sha256::new();
    digest.update(b"asb-strict-replay-route-v1");
    digest.update(route.session_id.as_bytes());
    digest.update([0]);
    digest.update(route.attempt_id.as_bytes());
    digest.update([0]);
    digest.update(format!("{:?}", route.dialect).as_bytes());
    format!("{:x}", digest.finalize())
}

impl std::fmt::Display for StrictReplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("strict replay launch validation failed")
    }
}
impl std::error::Error for StrictReplayError {}

impl StrictReplayLaunchV1 {
    /// Validate and compute the canonical launch digest.
    pub fn digest(&self) -> Result<String, StrictReplayError> {
        self.validate()?;
        let mut digest = Sha256::new();
        digest.update(b"asb-strict-replay-launch-v1");
        digest.update(serde_json::to_vec(self).map_err(|_| StrictReplayError::DigestMismatch)?);
        Ok(format!("{:x}", digest.finalize()))
    }
    /// Validate all identities, bounds, and egress policy.
    pub fn validate(&self) -> Result<(), StrictReplayError> {
        if self.schema_version != STRICT_REPLAY_LAUNCH_V1 {
            return Err(StrictReplayError::UnsupportedVersion);
        }
        if [
            self.cassette_sha256.as_str(),
            self.route_sha256.as_str(),
            self.workload_sha256.as_str(),
        ]
        .iter()
        .any(|value| {
            value.len() != 64
                || !value
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        }) {
            return Err(StrictReplayError::InvalidIdentity);
        }
        if [
            self.provider_dialect.as_str(),
            self.adapter.as_str(),
            self.run_id.as_str(),
            self.attempt_id.as_str(),
        ]
        .iter()
        .any(|value| value.is_empty() || value.len() > 128)
        {
            return Err(StrictReplayError::InvalidIdentity);
        }
        if self.timeout_ms == 0 || self.timeout_ms > 86_400_000 {
            return Err(StrictReplayError::InvalidTimeout);
        }
        if self.egress != EgressPolicy::LoopbackOnly {
            return Err(StrictReplayError::EgressNotDenied);
        }
        Ok(())
    }
}

impl StrictReplayLaunchRecord {
    /// Validate the input and its content digest.
    pub fn validate(&self) -> Result<(), StrictReplayError> {
        self.input.validate()?;
        if self.input.digest()? != self.launch_sha256 {
            return Err(StrictReplayError::DigestMismatch);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use asb_replay::{CassetteLimits, decode_cassette};
    fn input() -> StrictReplayLaunchV1 {
        StrictReplayLaunchV1 {
            schema_version: 1,
            cassette_sha256: "a".repeat(64),
            route_sha256: "b".repeat(64),
            provider_dialect: "openai-chat-v1".into(),
            adapter: "codex".into(),
            run_id: "run".into(),
            attempt_id: "attempt".into(),
            workload_sha256: "c".repeat(64),
            egress: EgressPolicy::LoopbackOnly,
            timeout_ms: 5000,
        }
    }
    #[test]
    fn valid_record() {
        let input = input();
        let record = StrictReplayLaunchRecord {
            launch_sha256: input.digest().unwrap(),
            input,
        };
        assert!(record.validate().is_ok());
    }
    #[test]
    fn invalid_input_and_digest_fail_closed() {
        let mut bad = input();
        bad.timeout_ms = 0;
        assert_eq!(bad.validate(), Err(StrictReplayError::InvalidTimeout));
        let input = input();
        assert_eq!(
            StrictReplayLaunchRecord {
                input,
                launch_sha256: "d".repeat(64)
            }
            .validate(),
            Err(StrictReplayError::DigestMismatch)
        );
    }

    #[test]
    fn endpoint_policy_rejects_provider_egress() {
        assert!(StrictReplayExecutor::validate_endpoint("https://api.openai.com/v1").is_err());
        assert!(StrictReplayExecutor::validate_endpoint("http://127.0.0.1:4317/replay").is_ok());
    }

    #[test]
    fn executor_rejects_invalid_cassette_without_fallback() {
        let cassette = decode_cassette(
            include_bytes!("../../asb-replay/fixtures/v1/buffered.json"),
            CassetteLimits::default(),
        )
        .unwrap();
        let mut launch = input();
        launch.cassette_sha256 = cassette.integrity.digest.clone();
        let record = StrictReplayLaunchRecord {
            launch_sha256: launch.digest().unwrap(),
            input: launch,
        };
        assert!(matches!(
            StrictReplayExecutor::new(record, cassette, Some(ProcessIsolationCapability::Verified)),
            Err(StrictReplayError::InvalidCassette)
        ));
    }
}
