// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Strict-replay launch handoff validation.

use crate::strict_replay::{StrictReplayError, StrictReplayLaunchRecord};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;
use std::time::Duration;

/// Runtime-owned view of a sidecar handoff.
///
/// Implementations must be backed by an opaque runtime-issued value.  The
/// bridge deliberately accepts this narrow projection rather than any
/// frontend launch metadata, so endpoint, socket, generation and executable
/// identities have one source of truth.
pub trait RuntimeReplayHandoff {
    /// Per-launch generation fence.
    fn generation(&self) -> &str;
    /// Authenticated route digest.
    fn route_digest(&self) -> &str;
    /// Private Unix relay socket.
    fn relay_path(&self) -> &Path;
    /// Child-visible loopback endpoint.
    fn endpoint(&self) -> SocketAddr;
    /// Bounded launch deadline.
    fn deadline(&self) -> Duration;
    /// Content digest of the sidecar executable.
    fn sidecar_command_digest(&self) -> &str;
    /// Content digest of the adapter executable.
    fn adapter_command_digest(&self) -> &str;
}

/// Immutable launch data derived exclusively from a runtime handoff.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplayLaunchMetadata {
    /// Runtime generation.
    pub generation: String,
    /// Authenticated route digest.
    pub route_digest: String,
    /// Private relay path.
    pub relay_path: std::path::PathBuf,
    /// Child-visible endpoint.
    pub endpoint: SocketAddr,
    /// Bounded deadline.
    pub deadline: Duration,
    /// Sidecar executable digest.
    pub sidecar_command_digest: String,
    /// Adapter executable digest.
    pub adapter_command_digest: String,
}

/// Validated strict-replay launch bridge.
///
/// This type does not accept endpoint or socket strings from an adapter.  It
/// copies all launch metadata from the runtime handoff and checks it against
/// the authenticated launch record before exposing the child endpoint.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StrictReplayLaunchBridge {
    metadata: ReplayLaunchMetadata,
}

impl StrictReplayLaunchBridge {
    /// Consume one runtime handoff for the authenticated launch record.
    pub fn new<H: RuntimeReplayHandoff>(
        record: &StrictReplayLaunchRecord,
        handoff: &H,
    ) -> Result<Self, StrictReplayError> {
        record.validate()?;
        if handoff.endpoint().ip() != IpAddr::V4(Ipv4Addr::LOCALHOST)
            || handoff.endpoint().port() == 0
            || handoff.relay_path().is_relative()
            || handoff.generation().is_empty()
            || handoff.generation().len() > 128
            || !handoff
                .generation()
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
            || handoff.route_digest() != record.input.route_sha256
            || handoff.deadline() != Duration::from_millis(record.input.timeout_ms)
            || !valid_digest(handoff.sidecar_command_digest())
            || !valid_digest(handoff.adapter_command_digest())
        {
            return Err(StrictReplayError::HandoffMismatch);
        }
        let metadata = ReplayLaunchMetadata {
            generation: handoff.generation().to_owned(),
            route_digest: handoff.route_digest().to_owned(),
            relay_path: handoff.relay_path().to_owned(),
            endpoint: handoff.endpoint(),
            deadline: handoff.deadline(),
            sidecar_command_digest: handoff.sidecar_command_digest().to_owned(),
            adapter_command_digest: handoff.adapter_command_digest().to_owned(),
        };
        Ok(Self { metadata })
    }

    /// Child-visible endpoint URI derived from the handoff.
    pub fn endpoint(&self) -> String {
        format!("http://{}/replay", self.metadata.endpoint)
    }

    /// Validated launch metadata for the runtime supervisor adapter.
    pub fn metadata(&self) -> &ReplayLaunchMetadata {
        &self.metadata
    }
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strict_replay::{EgressPolicy, STRICT_REPLAY_LAUNCH_V1};

    struct Handoff {
        generation: String,
        route_digest: String,
        relay_path: std::path::PathBuf,
        endpoint: SocketAddr,
        deadline: Duration,
        sidecar: String,
        adapter: String,
    }

    impl RuntimeReplayHandoff for Handoff {
        fn generation(&self) -> &str {
            &self.generation
        }
        fn route_digest(&self) -> &str {
            &self.route_digest
        }
        fn relay_path(&self) -> &Path {
            &self.relay_path
        }
        fn endpoint(&self) -> SocketAddr {
            self.endpoint
        }
        fn deadline(&self) -> Duration {
            self.deadline
        }
        fn sidecar_command_digest(&self) -> &str {
            &self.sidecar
        }
        fn adapter_command_digest(&self) -> &str {
            &self.adapter
        }
    }

    fn record(route: &str) -> crate::strict_replay::StrictReplayLaunchRecord {
        let input = crate::strict_replay::StrictReplayLaunchV1 {
            schema_version: STRICT_REPLAY_LAUNCH_V1,
            cassette_sha256: "a".repeat(64),
            route_sha256: route.to_owned(),
            provider_dialect: "openai-chat-v1".into(),
            adapter: "adapter".into(),
            run_id: "run".into(),
            attempt_id: "attempt".into(),
            workload_sha256: "b".repeat(64),
            egress: EgressPolicy::LoopbackOnly,
            timeout_ms: 5_000,
        };
        let launch_sha256 = input.digest().unwrap();
        crate::strict_replay::StrictReplayLaunchRecord {
            input,
            launch_sha256,
        }
    }

    fn handoff(route: &str) -> Handoff {
        Handoff {
            generation: "generation-1".into(),
            route_digest: route.into(),
            relay_path: "/run/user/1000/asb/relay.sock".into(),
            endpoint: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 4317),
            deadline: Duration::from_secs(5),
            sidecar: "c".repeat(64),
            adapter: "d".repeat(64),
        }
    }

    #[test]
    fn bridge_copies_only_runtime_handoff_metadata() {
        let route = "e".repeat(64);
        let record = record(&route);
        let handoff = handoff(&route);
        let bridge = StrictReplayLaunchBridge::new(&record, &handoff).unwrap();
        assert_eq!(bridge.endpoint(), "http://127.0.0.1:4317/replay");
        assert_eq!(bridge.metadata().relay_path, handoff.relay_path);
    }

    #[test]
    fn bridge_rejects_external_endpoint_and_mismatched_route() {
        let route = "e".repeat(64);
        let record = record(&route);
        let mut handoff = handoff(&route);
        handoff.endpoint = "192.0.2.1:4317".parse().unwrap();
        assert_eq!(
            StrictReplayLaunchBridge::new(&record, &handoff),
            Err(StrictReplayError::HandoffMismatch)
        );
        handoff.endpoint = "127.0.0.1:4317".parse().unwrap();
        handoff.route_digest = "f".repeat(64);
        assert_eq!(
            StrictReplayLaunchBridge::new(&record, &handoff),
            Err(StrictReplayError::HandoffMismatch)
        );
    }
}
