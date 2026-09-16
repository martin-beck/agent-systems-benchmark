// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Versioned adapter-facing strict replay launch contract.

use asb_replay::{
    Cassette, ReplayHttpRequest, ReplayHttpResponse, ReplayLimits, ReplayRoute, StrictReplayService,
};
use asb_runtime::sandbox::{
    NetworkPolicy, ResourceLease, SandboxBackend, SandboxLaunchInput, SandboxProcess,
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

impl ProcessIsolationCapability {
    /// Convert the runtime network policy into an attested capability.
    pub fn from_network_policy(policy: NetworkPolicy) -> Option<Self> {
        (policy == NetworkPolicy::Deny).then_some(Self::Verified)
    }
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
    /// Pinned adapter command digest.
    pub command_sha256: String,
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
    /// The launch command differs from its pinned identity.
    CommandMismatch,
    /// The runtime sandbox rejected or could not supervise the launch.
    SandboxUnavailable,
    /// The attempt lifecycle does not permit another operation.
    LifecycleClosed,
}

/// Cross-crate consumer seam for launching a strict replay adapter.
#[derive(Debug)]
pub struct StrictReplaySandboxLaunch {
    record: StrictReplayLaunchRecord,
}

impl StrictReplaySandboxLaunch {
    /// Validate a launch record and bind it to a pinned adapter command digest.
    pub fn new(record: StrictReplayLaunchRecord) -> Result<Self, StrictReplayError> {
        record.validate()?;
        if !is_digest(&record.input.command_sha256) {
            return Err(StrictReplayError::CommandMismatch);
        }
        Ok(Self { record })
    }

    /// Spawn only under the runtime-owned denied-network sandbox.
    pub fn spawn(
        self,
        backend: &SandboxBackend,
        input: SandboxLaunchInput,
        lease: ResourceLease,
    ) -> Result<SandboxProcess, StrictReplayError> {
        if input.spec().network_policy() != NetworkPolicy::Deny {
            return Err(StrictReplayError::IsolationUnavailable);
        }
        self.validate_route_environment(input.spec().environment())?;
        if input.limits().timeout().as_millis() > u128::from(self.record.input.timeout_ms) {
            return Err(StrictReplayError::InvalidTimeout);
        }
        if command_digest(input.spec().program(), input.spec().arguments())
            != self.record.input.command_sha256
        {
            return Err(StrictReplayError::CommandMismatch);
        }
        backend
            .spawn_launch(input, lease)
            .map_err(|_| StrictReplayError::SandboxUnavailable)
    }

    /// Validate the authenticated route digest and pinned local endpoint.
    pub fn validate_route_environment(
        &self,
        environment: &std::collections::BTreeMap<String, String>,
    ) -> Result<(), StrictReplayError> {
        if environment.get("ASB_REPLAY_ROUTE_SHA256") != Some(&self.record.input.route_sha256) {
            return Err(StrictReplayError::RouteMismatch);
        }
        let endpoint = environment
            .get("ASB_REPLAY_ENDPOINT")
            .ok_or(StrictReplayError::ExternalEndpoint)?;
        StrictReplayExecutor::validate_endpoint(endpoint)
    }

    /// Authenticated launch record consumed by this seam.
    pub fn record(&self) -> &StrictReplayLaunchRecord {
        &self.record
    }

    /// Check a candidate process budget against the authenticated deadline.
    pub fn validate_timeout(&self, timeout: std::time::Duration) -> Result<(), StrictReplayError> {
        if timeout.as_millis() > u128::from(self.record.input.timeout_ms) {
            Err(StrictReplayError::InvalidTimeout)
        } else {
            Ok(())
        }
    }
}

fn is_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn command_digest(program: &str, arguments: &[String]) -> String {
    let mut digest = Sha256::new();
    digest.update(b"asb-strict-replay-command-v1");
    digest.update(program.as_bytes());
    digest.update([0]);
    for argument in arguments {
        digest.update(argument.as_bytes());
        digest.update([0]);
    }
    format!("{:x}", digest.finalize())
}

/// Bounded replay executor that has no live-provider fallback.
pub struct StrictReplayExecutor {
    record: StrictReplayLaunchRecord,
    service: StrictReplayService,
    lifecycle: std::sync::Mutex<ReplayLifecycle>,
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
        Ok(Self {
            record,
            service,
            lifecycle: std::sync::Mutex::new(ReplayLifecycle::Ready),
        })
    }

    /// Cancel this attempt; subsequent requests fail closed.
    pub fn cancel(&self) -> Result<(), StrictReplayError> {
        let mut state = self
            .lifecycle
            .lock()
            .map_err(|_| StrictReplayError::LifecycleClosed)?;
        if *state != ReplayLifecycle::Ready {
            return Err(StrictReplayError::LifecycleClosed);
        }
        *state = ReplayLifecycle::Cancelled;
        Ok(())
    }

    /// Mark an interrupted attempt as requiring a fresh launch.
    pub fn recover_after_restart(&self) -> Result<(), StrictReplayError> {
        let mut state = self
            .lifecycle
            .lock()
            .map_err(|_| StrictReplayError::LifecycleClosed)?;
        if *state != ReplayLifecycle::Ready {
            return Err(StrictReplayError::LifecycleClosed);
        }
        *state = ReplayLifecycle::NeedsRestart;
        Ok(())
    }

    /// Serve one adapter request only from the authenticated local cassette route.
    pub fn execute(
        &self,
        route: &ReplayRoute,
        request: ReplayHttpRequest,
    ) -> Result<ReplayHttpResponse, StrictReplayError> {
        if *self
            .lifecycle
            .lock()
            .map_err(|_| StrictReplayError::LifecycleClosed)?
            != ReplayLifecycle::Ready
        {
            return Err(StrictReplayError::LifecycleClosed);
        }
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
    use asb_replay::{CassetteLimits, ProviderDialect, decode_cassette};
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
            command_sha256: "d".repeat(64),
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
    fn isolation_capability_is_only_attested_by_deny_policy() {
        assert_eq!(
            ProcessIsolationCapability::from_network_policy(NetworkPolicy::Deny),
            Some(ProcessIsolationCapability::Verified)
        );
        assert_eq!(
            ProcessIsolationCapability::from_network_policy(NetworkPolicy::Host),
            None
        );
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

    #[test]
    fn serde_rejects_credentials_and_ambient_configuration() {
        let value = serde_json::json!({
            "schema_version": 1, "cassette_sha256": "a".repeat(64),
            "route_sha256": "b".repeat(64), "provider_dialect": "openai-chat-v1",
            "adapter": "codex", "run_id": "run", "attempt_id": "attempt",
            "workload_sha256": "c".repeat(64), "command_sha256": "d".repeat(64), "egress": "loopback_only",
            "timeout_ms": 5000, "credential": "secret", "environment": {"API_KEY": "secret"}
        });
        assert!(serde_json::from_value::<StrictReplayLaunchV1>(value).is_err());
    }

    #[test]
    fn route_digest_binds_session_attempt_and_dialect() {
        let route = ReplayRoute {
            session_id: "session".into(),
            attempt_id: "attempt".into(),
            dialect: ProviderDialect::OpenaiChatCompletions,
        };
        let mut changed = route.clone();
        changed.attempt_id = "other".into();
        assert_ne!(route_digest(&route), route_digest(&changed));
    }

    #[test]
    fn executor_accepts_qualified_cassette_service() {
        let cassette = decode_cassette(
            include_bytes!("../../asb-replay/fixtures/v1/gemini-generate-content.json"),
            CassetteLimits::default(),
        )
        .unwrap();
        let mut launch = input();
        launch.cassette_sha256 = cassette.integrity.digest.clone();
        let record = StrictReplayLaunchRecord {
            launch_sha256: launch.digest().unwrap(),
            input: launch,
        };
        assert!(
            StrictReplayExecutor::new(record, cassette, Some(ProcessIsolationCapability::Verified))
                .is_ok()
        );
    }

    fn qualified_executor_fixture() -> (StrictReplayExecutor, ReplayRoute, ReplayHttpRequest) {
        let cassette = decode_cassette(
            include_bytes!("../../asb-replay/fixtures/v1/gemini-generate-content.json"),
            CassetteLimits::default(),
        )
        .unwrap();
        let interaction = &cassette.contents.interactions[0];
        let route = ReplayRoute {
            session_id: interaction.session_id.clone(),
            attempt_id: interaction.attempt_id.clone(),
            dialect: interaction.dialect,
        };
        let request = ReplayHttpRequest {
            method: interaction.request.method.clone(),
            path: interaction.request.path.clone(),
            headers: interaction.request.headers.clone(),
            body: serde_json::to_vec(&interaction.request.body).unwrap(),
        };
        let mut launch = input();
        launch.cassette_sha256 = cassette.integrity.digest.clone();
        launch.route_sha256 = route_digest(&route);
        launch.provider_dialect = "gemini-generate-content".into();
        launch.attempt_id = route.attempt_id.clone();
        let record = StrictReplayLaunchRecord {
            launch_sha256: launch.digest().unwrap(),
            input: launch,
        };
        (
            StrictReplayExecutor::new(record, cassette, Some(ProcessIsolationCapability::Verified))
                .unwrap(),
            route,
            request,
        )
    }

    #[test]
    fn executor_executes_a_qualified_cassette_request() {
        let (executor, route, request) = qualified_executor_fixture();
        let response = executor.execute(&route, request).unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.segments.len(), 1);
    }

    #[test]
    fn executor_rejects_unmatched_stale_route_before_service() {
        let (executor, mut route, request) = qualified_executor_fixture();
        route.attempt_id = "stale-attempt".into();
        assert_eq!(
            executor.execute(&route, request),
            Err(StrictReplayError::AttemptMismatch)
        );
    }

    #[test]
    fn sandbox_launch_requires_record_bound_command_digest() {
        let launch = input();
        let record = StrictReplayLaunchRecord {
            launch_sha256: launch.digest().unwrap(),
            input: launch,
        };
        assert!(StrictReplaySandboxLaunch::new(record).is_ok());

        let mut invalid = input();
        invalid.command_sha256 = "not-a-digest".into();
        let record = StrictReplayLaunchRecord {
            launch_sha256: invalid.digest().unwrap(),
            input: invalid,
        };
        assert_eq!(
            StrictReplaySandboxLaunch::new(record).unwrap_err(),
            StrictReplayError::CommandMismatch
        );
    }

    #[test]
    fn sandbox_launch_rejects_budget_beyond_authenticated_timeout() {
        let launch = input();
        let record = StrictReplayLaunchRecord {
            launch_sha256: launch.digest().unwrap(),
            input: launch,
        };
        let consumer = StrictReplaySandboxLaunch::new(record).unwrap();
        assert!(
            consumer
                .validate_timeout(std::time::Duration::from_secs(5))
                .is_ok()
        );
        assert_eq!(
            consumer.validate_timeout(std::time::Duration::from_millis(5001)),
            Err(StrictReplayError::InvalidTimeout)
        );
    }
}
