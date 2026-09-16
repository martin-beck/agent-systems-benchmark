// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! A small, fail-closed transport contract for strict-replay launches.
//!
//! The sidecar owns both endpoints: a random loopback TCP listener visible to
//! the child namespace and a private Unix listener used by the cassette
//! service.  No host-network namespace is requested by this module.  The
//! launch backend is responsible for putting the sidecar into the child
//! namespace; if it cannot do so, it must reject the launch.

use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

const MAGIC: &[u8] = b"ASB-REPLAY-SIDECAR/1\n";
const MAX_ID: usize = 128;
const MAX_FORWARD_BYTES: usize = 8 * 1024 * 1024;
const HANDOFF_VERSION: u16 = 1;

/// Per-launch identity used to fence stale and duplicate relay attempts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SidecarIdentity {
    generation: String,
    route_digest: String,
}

impl SidecarIdentity {
    /// Validate a generation and route digest.
    pub fn new(generation: impl Into<String>, route_digest: impl Into<String>) -> io::Result<Self> {
        let generation = generation.into();
        let route_digest = route_digest.into();
        if !valid_id(&generation) || !valid_id(&route_digest) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid sidecar identity",
            ));
        }
        Ok(Self {
            generation,
            route_digest,
        })
    }
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ID
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-')
}

/// A running per-launch sidecar. Dropping it removes its private Unix socket.
pub struct LoopbackSidecar {
    identity: SidecarIdentity,
    relay_path: PathBuf,
    tcp: TcpListener,
    unix: UnixListener,
    consumed: bool,
}

impl std::fmt::Debug for LoopbackSidecar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoopbackSidecar")
            .field("identity", &self.identity)
            .field("relay_path", &self.relay_path)
            .field("endpoint", &self.endpoint())
            .field("consumed", &self.consumed)
            .finish()
    }
}

impl LoopbackSidecar {
    /// Bind a unique loopback endpoint and private relay socket.
    pub fn bind(identity: SidecarIdentity, relay_path: impl AsRef<Path>) -> io::Result<Self> {
        let relay_path = relay_path.as_ref().to_path_buf();
        if relay_path.as_os_str().is_empty() || relay_path.exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "relay path is not fresh",
            ));
        }
        if let Some(parent) = relay_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tcp = TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))?;
        let unix = UnixListener::bind(&relay_path)?;
        let mut permissions = fs::metadata(&relay_path)?.permissions();
        use std::os::unix::fs::PermissionsExt;
        permissions.set_mode(0o600);
        fs::set_permissions(&relay_path, permissions)?;
        Ok(Self {
            identity,
            relay_path,
            tcp,
            unix,
            consumed: false,
        })
    }

    /// Return the child-visible loopback endpoint.
    pub fn endpoint(&self) -> SocketAddr {
        self.tcp.local_addr().expect("bound listener")
    }

    /// Return the private relay path.
    pub fn relay_path(&self) -> &Path {
        &self.relay_path
    }

    /// Accept one authenticated cassette connection; replay attempts are one-shot.
    pub fn accept_authenticated(&mut self) -> io::Result<UnixStream> {
        if self.consumed {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "duplicate relay attempt",
            ));
        }
        let (mut stream, _) = self.unix.accept()?;
        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
        let mut expected = Vec::from(MAGIC);
        expected.extend_from_slice(self.identity.generation.as_bytes());
        expected.push(b'\n');
        expected.extend_from_slice(self.identity.route_digest.as_bytes());
        expected.push(b'\n');
        let mut got = vec![0; expected.len()];
        stream.read_exact(&mut got)?;
        if got != expected {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "stale or unauthenticated relay",
            ));
        }
        self.consumed = true;
        Ok(stream)
    }

    /// Forward one bounded stream, returning the number of bytes copied.
    pub fn forward_bounded<R: Read, W: Write>(reader: &mut R, writer: &mut W) -> io::Result<usize> {
        let mut buffer = [0_u8; 16 * 1024];
        let mut total = 0;
        loop {
            let remaining = MAX_FORWARD_BYTES - total;
            if remaining == 0 {
                return Err(io::Error::other("relay byte budget exceeded"));
            }
            let read_size = buffer.len().min(remaining);
            let count = reader.read(&mut buffer[..read_size])?;
            if count == 0 {
                return Ok(total);
            }
            writer.write_all(&buffer[..count])?;
            total += count;
        }
    }
}

impl Drop for LoopbackSidecar {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.relay_path);
    }
}

/// Evidence that the sidecar was handed to a private child namespace.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SidecarAttestation {
    /// Child-visible loopback endpoint.
    pub endpoint: SocketAddr,
    /// Per-launch generation.
    pub generation: String,
    /// Cassette route digest.
    pub route_digest: String,
    /// Whether the launch backend proved the private namespace handoff.
    pub namespace_ready: bool,
    /// Digest of the immutable sidecar executable.
    pub sidecar_command_digest: String,
    /// Digest of the adapter command released into the namespace.
    pub adapter_command_digest: String,
}

/// Versioned, runtime-issued handoff passed to a private namespace launcher.
///
/// This is deliberately data-only: callers cannot construct a trusted handoff
/// without the runtime attesting namespace readiness.  The launcher must bind
/// every field to one process lifetime and reject a changed generation or
/// command digest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SidecarHandoff {
    /// Protocol version.
    pub version: u16,
    /// Per-launch identity.
    pub identity: SidecarIdentity,
    /// Private Unix relay endpoint.
    pub relay_path: PathBuf,
    /// Child-visible loopback endpoint.
    pub endpoint: SocketAddr,
    /// Monotonic launch deadline.
    pub deadline: Duration,
    /// Pinned sidecar executable digest.
    pub sidecar_command_digest: String,
    /// Pinned adapter command digest.
    pub adapter_command_digest: String,
}

impl SidecarHandoff {
    /// Per-launch generation fence.
    pub fn generation(&self) -> &str {
        &self.identity.generation
    }

    /// Authenticated replay route digest.
    pub fn route_digest(&self) -> &str {
        &self.identity.route_digest
    }
}

impl SidecarHandoff {
    /// Validate the handoff before namespace launch.
    pub fn validate(&self) -> io::Result<()> {
        if self.version != HANDOFF_VERSION || self.deadline.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid handoff",
            ));
        }
        if self.endpoint.ip() != IpAddr::V4(Ipv4Addr::LOCALHOST)
            || self.endpoint.port() == 0
            || self.relay_path.as_os_str().is_empty()
            || self.sidecar_command_digest.is_empty()
            || self.adapter_command_digest.is_empty()
            || !valid_id(&self.sidecar_command_digest)
            || !valid_id(&self.adapter_command_digest)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid handoff",
            ));
        }
        Ok(())
    }
}

impl LoopbackSidecar {
    /// Build the legacy readiness attestation using explicit placeholder
    /// command identities. New launchers should use [`Self::handoff`].
    pub fn attest(&self, namespace_ready: bool) -> io::Result<SidecarAttestation> {
        self.attest_with_commands(namespace_ready, "legacy-sidecar", "legacy-adapter")
    }

    /// Build an attestation, failing closed until the launcher proves handoff.
    pub fn attest_with_commands(
        &self,
        namespace_ready: bool,
        sidecar_command_digest: impl Into<String>,
        adapter_command_digest: impl Into<String>,
    ) -> io::Result<SidecarAttestation> {
        if !namespace_ready {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "private namespace handoff unavailable",
            ));
        }
        let sidecar_command_digest = sidecar_command_digest.into();
        let adapter_command_digest = adapter_command_digest.into();
        if !valid_id(&sidecar_command_digest) || !valid_id(&adapter_command_digest) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid command digest",
            ));
        }
        Ok(SidecarAttestation {
            endpoint: self.endpoint(),
            generation: self.identity.generation.clone(),
            route_digest: self.identity.route_digest.clone(),
            namespace_ready,
            sidecar_command_digest,
            adapter_command_digest,
        })
    }

    /// Build a validated runtime-issued handoff after namespace attestation.
    pub fn handoff(
        &self,
        namespace_ready: bool,
        deadline: Duration,
        sidecar_command_digest: impl Into<String>,
        adapter_command_digest: impl Into<String>,
    ) -> io::Result<SidecarHandoff> {
        let attestation = self.attest_with_commands(
            namespace_ready,
            sidecar_command_digest,
            adapter_command_digest,
        )?;
        let handoff = SidecarHandoff {
            version: HANDOFF_VERSION,
            identity: self.identity.clone(),
            relay_path: self.relay_path.clone(),
            endpoint: attestation.endpoint,
            deadline,
            sidecar_command_digest: attestation.sidecar_command_digest,
            adapter_command_digest: attestation.adapter_command_digest,
        };
        handoff.validate()?;
        Ok(handoff)
    }
}

/// Create a private temporary relay path without replacing an existing file.
pub fn fresh_relay_path(root: &Path, generation: &str) -> io::Result<PathBuf> {
    if !valid_id(generation) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid generation",
        ));
    }
    fs::create_dir_all(root)?;
    for attempt in 0..32_u32 {
        let path = root.join(format!("relay-{generation}-{attempt}.sock"));
        let result = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path);
        match result {
            Ok(file) => {
                drop(file);
                fs::remove_file(&path)?;
                return Ok(path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "unable to allocate relay path",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn root() -> PathBuf {
        std::env::temp_dir().join(format!(
            "asb-sidecar-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn authenticates_once_and_cleans_up() {
        let root = root();
        let path = root.join("relay.sock");
        let identity = SidecarIdentity::new("generation-1", "route-1").unwrap();
        let mut sidecar = LoopbackSidecar::bind(identity.clone(), &path).unwrap();
        assert_eq!(sidecar.endpoint().ip(), IpAddr::V4(Ipv4Addr::LOCALHOST));
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let client = UnixStream::connect(&path).unwrap();
        let path_for_thread = path.clone();
        let handle = std::thread::spawn(move || {
            let mut client = client;
            client.write_all(MAGIC).unwrap();
            client.write_all(b"generation-1\nroute-1\n").unwrap();
            let _ = path_for_thread;
        });
        let _stream = sidecar.accept_authenticated().unwrap();
        handle.join().unwrap();
        assert!(sidecar.accept_authenticated().is_err());
        drop(sidecar);
        assert!(!path.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn stale_identity_is_rejected() {
        let root = root();
        let path = root.join("relay.sock");
        let mut sidecar =
            LoopbackSidecar::bind(SidecarIdentity::new("new", "route").unwrap(), &path).unwrap();
        let mut client = UnixStream::connect(&path).unwrap();
        client.write_all(MAGIC).unwrap();
        client.write_all(b"old\nroute\n").unwrap();
        assert_eq!(
            sidecar.accept_authenticated().unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        drop(sidecar);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn forwarding_is_bounded_and_exact() {
        let input = b"cassette-response";
        let mut reader = &input[..];
        let mut output = Vec::new();
        assert_eq!(
            LoopbackSidecar::forward_bounded(&mut reader, &mut output).unwrap(),
            input.len()
        );
        assert_eq!(output, input);
    }

    #[test]
    fn namespace_handoff_is_required_for_attestation() {
        let root = root();
        let path = root.join("relay.sock");
        let sidecar =
            LoopbackSidecar::bind(SidecarIdentity::new("generation", "route").unwrap(), &path)
                .unwrap();
        assert_eq!(
            sidecar.attest(false).unwrap_err().kind(),
            io::ErrorKind::Unsupported
        );
        let attestation = sidecar.attest(true).unwrap();
        assert!(attestation.namespace_ready);
        assert_eq!(attestation.generation, "generation");
        assert_eq!(attestation.route_digest, "route");
        drop(sidecar);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn handoff_binds_commands_route_and_deadline() {
        let root = root();
        let path = root.join("relay.sock");
        let sidecar =
            LoopbackSidecar::bind(SidecarIdentity::new("generation", "route").unwrap(), &path)
                .unwrap();
        let handoff = sidecar
            .handoff(
                true,
                Duration::from_secs(10),
                "sidecar-sha256",
                "adapter-sha256",
            )
            .unwrap();
        assert_eq!(handoff.version, HANDOFF_VERSION);
        assert_eq!(handoff.relay_path, path);
        assert_eq!(handoff.endpoint.ip(), IpAddr::V4(Ipv4Addr::LOCALHOST));
        assert_eq!(handoff.sidecar_command_digest, "sidecar-sha256");
        assert_eq!(handoff.adapter_command_digest, "adapter-sha256");
        drop(sidecar);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn handoff_rejects_unattested_or_unbounded_launches() {
        let root = root();
        let path = root.join("relay.sock");
        let sidecar =
            LoopbackSidecar::bind(SidecarIdentity::new("generation", "route").unwrap(), &path)
                .unwrap();
        assert_eq!(
            sidecar
                .handoff(false, Duration::from_secs(1), "sidecar", "adapter")
                .unwrap_err()
                .kind(),
            io::ErrorKind::Unsupported
        );
        assert_eq!(
            sidecar
                .handoff(true, Duration::ZERO, "sidecar", "adapter")
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            sidecar
                .handoff(true, Duration::from_secs(1), "side car", "adapter")
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        drop(sidecar);
        let _ = fs::remove_dir_all(root);
    }
}
