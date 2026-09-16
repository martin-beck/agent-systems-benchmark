// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Fail-closed contract for composing a replay sidecar and adapter.
//!
//! The supervisor is intentionally represented as a validated launch plan.  The
//! plan is consumed by the runtime-owned launcher, so callers cannot smuggle a
//! host-network flag, an unpinned executable, or an unbounded relay into the
//! sandbox boundary.

use sha2::{Digest, Sha256};
use std::net::{IpAddr, SocketAddr};
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

    /// Pin an executable to its current immutable SHA-256 content.
    pub fn new_verified(
        executable: PathBuf,
        arguments: Vec<String>,
        expected_digest: &str,
    ) -> Result<Self, SupervisorError> {
        let command = Self::new(executable, arguments, expected_digest.to_owned())?;
        let mut file = std::fs::File::open(&command.executable)
            .map_err(|_| SupervisorError::ExecutableUnavailable)?;
        let mut digest = Sha256::new();
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = std::io::Read::read(&mut file, &mut buffer)
                .map_err(|_| SupervisorError::ExecutableUnavailable)?;
            if read == 0 {
                break;
            }
            digest.update(&buffer[..read]);
        }
        let observed = format!("{:x}", digest.finalize());
        if observed != expected_digest {
            return Err(SupervisorError::ExecutableDigestMismatch);
        }
        Ok(command)
    }

    /// Resolve a content-pinned executable from a verified bundle directory.
    /// The relative path is confined to the bundle and is never interpreted
    /// through a mutable system path.
    pub fn from_bundle(
        bundle_root: &Path,
        relative_path: &Path,
        arguments: Vec<String>,
        expected_digest: &str,
    ) -> Result<Self, SupervisorError> {
        if relative_path.is_absolute()
            || relative_path
                .components()
                .any(|component| matches!(component, Component::ParentDir))
        {
            return Err(SupervisorError::InvalidCommand);
        }
        let root = std::fs::canonicalize(bundle_root)
            .map_err(|_| SupervisorError::ExecutableUnavailable)?;
        let executable = root
            .join(relative_path)
            .canonicalize()
            .map_err(|_| SupervisorError::ExecutableUnavailable)?;
        if !executable.starts_with(&root) {
            return Err(SupervisorError::InvalidCommand);
        }
        Self::new_verified(executable, arguments, expected_digest)
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
    supervisor: Option<PinnedCommand>,
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
            supervisor: None,
            sidecar,
            adapter,
            relay,
            generation,
            route_digest,
            timeout,
        })
    }

    /// Attach the content-pinned supervisor executable from the trusted bundle.
    pub fn with_supervisor(mut self, supervisor: PinnedCommand) -> Self {
        self.supervisor = Some(supervisor);
        self
    }

    /// Content-pinned supervisor executable, when configured.
    pub fn supervisor(&self) -> Option<&PinnedCommand> {
        self.supervisor.as_ref()
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

    /// Validate that the sidecar has an explicit loopback-only listener.
    ///
    /// The listener is created inside the Bubblewrap network namespace.  A
    /// missing or non-loopback address would either make the cassette
    /// unreachable or accidentally widen the boundary, so both are rejected
    /// before any child is spawned.
    pub fn validate_loopback_listener(&self) -> Result<SocketAddr, SupervisorError> {
        if self.supervisor.is_none() {
            return Err(SupervisorError::InvalidHandoff);
        }
        let mut listener = None;
        let mut args = self.sidecar.arguments.iter();
        while let Some(argument) = args.next() {
            if argument == "--listen" {
                let value = args.next().ok_or(SupervisorError::InvalidHandoff)?;
                let address = value
                    .parse::<SocketAddr>()
                    .map_err(|_| SupervisorError::InvalidHandoff)?;
                if address.ip() != IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
                    || address.port() == 0
                    || listener.replace(address).is_some()
                {
                    return Err(SupervisorError::InvalidHandoff);
                }
            }
        }
        listener.ok_or(SupervisorError::InvalidHandoff)
    }

    /// Arguments for the in-tree supervisor executable.
    ///
    /// The explicit `--unshare-net` marker is part of the contract; host
    /// networking is not representable.  The supervisor must reject startup
    /// unless it can attest that loopback is ready and both children share its
    /// private namespace.
    pub fn arguments(&self) -> Vec<String> {
        self.arguments_for_relay(&self.relay)
    }

    /// Arguments with the relay path visible inside a sandbox.
    pub fn arguments_for_relay(&self, relay: &Path) -> Vec<String> {
        self.arguments_for_paths(relay, &self.sidecar.executable)
    }

    /// Arguments with both relay and sidecar paths rewritten to namespace paths.
    pub fn arguments_for_paths(&self, relay: &Path, sidecar: &Path) -> Vec<String> {
        let mut args = vec![
            "--unshare-net".into(),
            "--relay".into(),
            relay.display().to_string(),
            "--generation".into(),
            self.generation.clone(),
            "--route-sha256".into(),
            self.route_digest.clone(),
            "--timeout-ms".into(),
            self.timeout.as_millis().to_string(),
            "--sidecar".into(),
            sidecar.display().to_string(),
            "--sidecar-digest".into(),
            self.sidecar.digest.clone(),
            "--adapter".into(),
            self.adapter.executable.display().to_string(),
            "--adapter-digest".into(),
            self.adapter.digest.clone(),
        ];
        for arg in &self.sidecar.arguments {
            args.extend(["--sidecar-arg".to_owned(), arg.clone()]);
        }
        for arg in &self.adapter.arguments {
            args.extend(["--adapter-arg".to_owned(), arg.clone()]);
        }
        args
    }
}

/// Configuration rejected before any process or namespace is created.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SupervisorError {
    /// Command executable, digest, or arguments were invalid.
    InvalidCommand,
    /// Relay, generation, route, or deadline handoff was invalid.
    InvalidHandoff,
    /// The pinned executable could not be read.
    ExecutableUnavailable,
    /// The executable content changed after its digest was selected.
    ExecutableDigestMismatch,
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
        let args = p.arguments_for_relay(Path::new("/tmp/asb-replay-relay.sock"));
        assert!(
            args.windows(2)
                .any(|pair| pair[0] == "--relay" && pair[1] == "/tmp/asb-replay-relay.sock")
        );
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

    #[test]
    fn loopback_listener_requires_one_ipv4_loopback_endpoint() {
        let sidecar = PinnedCommand::new(
            PathBuf::from("/usr/bin/sidecar"),
            vec!["--listen".into(), "127.0.0.1:43123".into()],
            "a".repeat(64),
        )
        .unwrap();
        let plan = SupervisorPlan::new(
            sidecar,
            cmd("/usr/bin/adapter"),
            PathBuf::from("/tmp/relay.sock"),
            "generation-1".into(),
            "b".repeat(64),
            Duration::from_secs(5),
        )
        .unwrap()
        .with_supervisor(cmd("/usr/bin/supervisor"));
        assert_eq!(
            plan.validate_loopback_listener().unwrap(),
            "127.0.0.1:43123".parse().unwrap()
        );

        let non_loopback = PinnedCommand::new(
            PathBuf::from("/usr/bin/sidecar"),
            vec!["--listen".into(), "0.0.0.0:43123".into()],
            "a".repeat(64),
        )
        .unwrap();
        let rejected = SupervisorPlan::new(
            non_loopback,
            cmd("/usr/bin/adapter"),
            PathBuf::from("/tmp/relay.sock"),
            "generation-1".into(),
            "b".repeat(64),
            Duration::from_secs(5),
        )
        .unwrap()
        .with_supervisor(cmd("/usr/bin/supervisor"));
        assert!(rejected.validate_loopback_listener().is_err());
    }
}
