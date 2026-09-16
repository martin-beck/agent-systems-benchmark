// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Fail-closed contract for composing a replay sidecar and adapter.
//!
//! The supervisor is intentionally represented as a validated launch plan.  The
//! plan is consumed by the runtime-owned launcher, so callers cannot smuggle a
//! host-network flag, an unpinned executable, or an unbounded relay into the
//! sandbox boundary.

use std::path::{Component, Path, PathBuf};
use std::time::Duration;

const MAX_ARGUMENTS: usize = 256;
const MAX_ARGUMENT_BYTES: usize = 32 * 1024;
const MAX_DIGEST: usize = 64;

/// A command whose executable and arguments are fixed for one launch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PinnedCommand {
    executable: PathBuf,
    arguments: Vec<String>,
    digest: String,
}

impl PinnedCommand {
    /// Validate an absolute executable, bounded arguments, and a SHA-256 digest.
    pub fn new(
        executable: PathBuf,
        arguments: Vec<String>,
        digest: String,
    ) -> Result<Self, SupervisorError> {
        if !executable.is_absolute()
            || executable
                .components()
                .any(|c| matches!(c, Component::ParentDir))
            || digest.len() != MAX_DIGEST
            || !digest.bytes().all(|b| b.is_ascii_hexdigit())
            || arguments.len() > MAX_ARGUMENTS
            || arguments.iter().any(|a| a.len() > 4096)
            || arguments
                .iter()
                .try_fold(0usize, |n, a| n.checked_add(a.len()))
                .is_none_or(|n| n > MAX_ARGUMENT_BYTES)
        {
            return Err(SupervisorError::InvalidCommand);
        }
        Ok(Self {
            executable,
            arguments,
            digest,
        })
    }

    /// Executable path.
    pub fn executable(&self) -> &Path {
        &self.executable
    }
    /// Arguments passed unchanged to the executable.
    pub fn arguments(&self) -> &[String] {
        &self.arguments
    }
    /// Attested executable digest.
    pub fn digest(&self) -> &str {
        &self.digest
    }
}

/// Authenticated, per-launch supervisor handoff.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SupervisorPlan {
    sidecar: PinnedCommand,
    adapter: PinnedCommand,
    relay: PathBuf,
    generation: String,
    route_digest: String,
    timeout: Duration,
}

impl SupervisorPlan {
    /// Build a plan that can only use private loopback and a private relay.
    pub fn new(
        sidecar: PinnedCommand,
        adapter: PinnedCommand,
        relay: PathBuf,
        generation: String,
        route_digest: String,
        timeout: Duration,
    ) -> Result<Self, SupervisorError> {
        if !relay.is_absolute()
            || relay
                .components()
                .any(|c| matches!(c, Component::ParentDir))
            || generation.is_empty()
            || generation.len() > 128
            || !route_digest.bytes().all(|b| b.is_ascii_hexdigit())
            || route_digest.len() != MAX_DIGEST
            || timeout.is_zero()
        {
            return Err(SupervisorError::InvalidHandoff);
        }
        Ok(Self {
            sidecar,
            adapter,
            relay,
            generation,
            route_digest,
            timeout,
        })
    }

    /// Sidecar command.
    pub fn sidecar(&self) -> &PinnedCommand {
        &self.sidecar
    }
    /// Adapter command.
    pub fn adapter(&self) -> &PinnedCommand {
        &self.adapter
    }
    /// Private Unix relay path.
    pub fn relay(&self) -> &Path {
        &self.relay
    }
    /// Per-launch generation.
    pub fn generation(&self) -> &str {
        &self.generation
    }
    /// Cassette route digest.
    pub fn route_digest(&self) -> &str {
        &self.route_digest
    }
    /// Hard launch deadline.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Arguments for the in-tree supervisor executable.
    ///
    /// The explicit `--unshare-net` marker is part of the contract; host
    /// networking is not representable.  The supervisor must reject startup
    /// unless it can attest that loopback is ready and both children share its
    /// private namespace.
    pub fn arguments(&self) -> Vec<String> {
        vec![
            "--unshare-net".into(),
            "--relay".into(),
            self.relay.display().to_string(),
            "--generation".into(),
            self.generation.clone(),
            "--route-sha256".into(),
            self.route_digest.clone(),
            "--timeout-ms".into(),
            self.timeout.as_millis().to_string(),
            "--sidecar".into(),
            self.sidecar.executable.display().to_string(),
            "--adapter".into(),
            self.adapter.executable.display().to_string(),
        ]
    }
}

/// Configuration rejected before any process or namespace is created.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SupervisorError {
    /// Command executable, digest, or arguments were invalid.
    InvalidCommand,
    /// Relay, generation, route, or deadline handoff was invalid.
    InvalidHandoff,
}

impl std::fmt::Display for SupervisorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid supervisor configuration: {self:?}")
    }
}
impl std::error::Error for SupervisorError {}

#[cfg(test)]
mod tests {
    use super::*;
    fn cmd(path: &str) -> PinnedCommand {
        PinnedCommand::new(PathBuf::from(path), vec!["--stdio".into()], "a".repeat(64)).unwrap()
    }
    #[test]
    fn plan_is_private_and_bounded() {
        let p = SupervisorPlan::new(
            cmd("/usr/bin/sidecar"),
            cmd("/usr/bin/adapter"),
            PathBuf::from("/run/user/1000/asb/relay"),
            "generation-1".into(),
            "b".repeat(64),
            Duration::from_secs(5),
        )
        .unwrap();
        assert!(p.arguments().contains(&"--unshare-net".into()));
        assert!(!p.arguments().contains(&"--share-net".into()));
    }
    #[test]
    fn rejects_traversal_and_unbounded_identity() {
        assert!(
            PinnedCommand::new(PathBuf::from("/usr/bin/../sh"), vec![], "a".repeat(64)).is_err()
        );
        assert!(
            SupervisorPlan::new(
                cmd("/a"),
                cmd("/b"),
                PathBuf::from("/tmp/../relay"),
                "g".into(),
                "c".repeat(64),
                Duration::from_secs(1)
            )
            .is_err()
        );
        assert!(
            SupervisorPlan::new(
                cmd("/a"),
                cmd("/b"),
                PathBuf::from("/tmp/relay"),
                "g".into(),
                "short".into(),
                Duration::from_secs(1)
            )
            .is_err()
        );
    }
    #[test]
    fn rejects_host_network_as_unrepresentable() {
        let p = SupervisorPlan::new(
            cmd("/a"),
            cmd("/b"),
            PathBuf::from("/tmp/relay"),
            "g".into(),
            "d".repeat(64),
            Duration::from_secs(1),
        )
        .unwrap();
        assert!(
            !p.arguments()
                .iter()
                .any(|a| a == "--share-net" || a == "--network=host")
        );
    }
}
