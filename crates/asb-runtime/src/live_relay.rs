// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Runtime-owned live-provider relay protocol.
//!
//! The child is given only a private Unix endpoint and an opaque capability.
//! It cannot select a provider address, route, namespace, or credential.  The
//! runtime selects the concrete destination before the listener is created and
//! authenticates the first child request before opening that destination.

use crate::live_namespace::{LiveProviderNamespaceHandoff, NamespaceIdentity};
use crate::provider_egress::{
    ProviderEgressAllowlist, ProviderEgressAuthorization, ProviderEgressError,
    ProviderEgressHandoff, ProviderEgressPolicy, ProviderEgressRelay, ProviderEgressTarget,
};
use std::fs;
use std::io::{self, Read};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const PROTOCOL: &[u8] = b"ASB-LIVE/1 ";
const MAX_REQUEST: usize = 256;

/// Errors returned by the bounded live relay.
#[derive(Debug)]
pub enum LiveRelayError {
    /// Filesystem or stream failure.
    Io(io::Error),
    /// A child request was malformed or exceeded the protocol bound.
    InvalidRequest,
    /// The request did not match the runtime-issued launch capability.
    Unauthorized,
    /// The runtime capability is stale, expired, revoked, or namespace-mismatched.
    Handoff(crate::live_namespace::LiveNamespaceError),
    /// The selected egress route rejected the request.
    Egress(ProviderEgressError),
    /// A second child attempted to consume this one-shot relay.
    Duplicate,
}

impl From<io::Error> for LiveRelayError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}
impl From<ProviderEgressError> for LiveRelayError {
    fn from(error: ProviderEgressError) -> Self {
        Self::Egress(error)
    }
}
impl PartialEq for LiveRelayError {
    fn eq(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (Self::InvalidRequest, Self::InvalidRequest)
                | (Self::Unauthorized, Self::Unauthorized)
                | (Self::Duplicate, Self::Duplicate)
                | (Self::Handoff(_), Self::Handoff(_))
                | (Self::Egress(_), Self::Egress(_))
                | (Self::Io(_), Self::Io(_))
        )
    }
}
impl Eq for LiveRelayError {}
impl std::fmt::Display for LiveRelayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Io(_) => "live relay I/O failed",
            Self::InvalidRequest => "invalid live relay request",
            Self::Unauthorized => "live relay request is unauthorized",
            Self::Handoff(_) => "live relay handoff is unavailable",
            Self::Egress(_) => "live relay egress is unauthorized",
            Self::Duplicate => "live relay is already consumed",
        })
    }
}
impl std::error::Error for LiveRelayError {}

/// Runtime-owned one-shot listener.  The concrete target is fixed at
/// construction and is never accepted from child bytes.
pub struct LiveProviderRelay {
    listener: UnixListener,
    socket: PathBuf,
    generation: String,
    capability_sha256: String,
    namespace: NamespaceIdentity,
    handoff: LiveProviderNamespaceHandoff,
    authorization: Option<ProviderEgressAuthorization>,
    connector: ProviderEgressRelay,
    target: ProviderEgressTarget,
    deadline: Instant,
    consumed: bool,
}

impl LiveProviderRelay {
    /// Bind a private relay below `root` using the runtime-selected target.
    #[allow(clippy::too_many_arguments)]
    pub fn bind(
        root: &Path,
        policy: &ProviderEgressPolicy,
        egress_handoff: &ProviderEgressHandoff,
        namespace_handoff: LiveProviderNamespaceHandoff,
        namespace: NamespaceIdentity,
        allowlist: ProviderEgressAllowlist,
        target: ProviderEgressTarget,
        generation: impl Into<String>,
        route_sha256: impl Into<String>,
        io_timeout: Duration,
        max_forward_bytes: usize,
        now_unix_ms: u64,
    ) -> Result<Self, LiveRelayError> {
        let generation = generation.into();
        let authorization = ProviderEgressAuthorization::authorize(
            policy,
            egress_handoff,
            now_unix_ms,
            &generation,
            &route_sha256.into(),
        )?;
        if !allowlist.permits(target.address()) {
            return Err(LiveRelayError::Unauthorized);
        }
        if !root.is_absolute() || !root.is_dir() {
            return Err(LiveRelayError::InvalidRequest);
        }
        let root = fs::canonicalize(root)?;
        let socket = root.join(format!("asb-live-{generation}.sock"));
        let listener = UnixListener::bind(&socket)?;
        // Accept is deliberately non-blocking so cancellation and the finite
        // relay deadline also cover a child that never connects.
        listener.set_nonblocking(true)?;
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))?;
        if let Err(error) = namespace_handoff.validate(&namespace, now_unix_ms) {
            let _ = fs::remove_file(&socket);
            return Err(LiveRelayError::Handoff(error));
        }
        let deadline = Instant::now()
            .checked_add(io_timeout)
            .ok_or(LiveRelayError::InvalidRequest)?;
        let connector = ProviderEgressRelay::new(
            policy,
            allowlist,
            generation.clone(),
            authorization.route_sha256().to_owned(),
            io_timeout,
            max_forward_bytes,
        )?;
        Ok(Self {
            listener,
            socket,
            generation,
            capability_sha256: namespace_handoff.capability_sha256().to_owned(),
            namespace,
            handoff: namespace_handoff,
            authorization: Some(authorization),
            connector,
            target,
            deadline,
            consumed: false,
        })
    }

    /// Host path to expose through the runtime-owned sandbox bind mount.
    pub fn socket(&self) -> &Path {
        &self.socket
    }
    /// Accept one authenticated child and connect only the runtime-selected target.
    pub fn accept_and_connect(
        &mut self,
        now_unix_ms: u64,
    ) -> Result<(UnixStream, std::net::TcpStream), LiveRelayError> {
        if self.consumed {
            return Err(LiveRelayError::Duplicate);
        }
        let mut stream = loop {
            match self.listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    let remaining = self
                        .deadline
                        .checked_duration_since(Instant::now())
                        .ok_or_else(|| {
                            LiveRelayError::Io(io::Error::new(
                                io::ErrorKind::TimedOut,
                                "live relay accept deadline",
                            ))
                        })?;
                    std::thread::sleep(remaining.min(Duration::from_millis(2)));
                }
                Err(error) => return Err(error.into()),
            }
        };
        let remaining = self
            .deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| {
                LiveRelayError::Io(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "live relay request deadline",
                ))
            })?;
        stream.set_read_timeout(Some(remaining))?;
        let mut request = Vec::with_capacity(MAX_REQUEST);
        let mut byte = [0_u8; 1];
        loop {
            let count = stream.read(&mut byte)?;
            if count == 0 {
                return Err(LiveRelayError::InvalidRequest);
            }
            request.push(byte[0]);
            if byte[0] == b'\n' {
                break;
            }
            if request.len() >= MAX_REQUEST {
                return Err(LiveRelayError::InvalidRequest);
            }
        }
        let expected = [
            PROTOCOL,
            self.generation.as_bytes(),
            b" ",
            self.capability_sha256.as_bytes(),
            b"\n",
        ]
        .concat();
        if request != expected {
            return Err(LiveRelayError::Unauthorized);
        }
        self.handoff
            .validate(&self.namespace, now_unix_ms)
            .map_err(LiveRelayError::Handoff)?;
        let authorization = self.authorization.take().ok_or(LiveRelayError::Duplicate)?;
        let tcp = self
            .connector
            .connect(authorization, self.target, self.deadline)?;
        self.consumed = true;
        Ok((stream, tcp))
    }
    /// Revoke and remove the relay on cancellation or teardown.
    pub fn revoke(&self) {
        self.handoff.revoke();
    }
}

impl Drop for LiveProviderRelay {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.socket);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn root() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "asb-live-relay-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn relay_with_timeout(
        io_timeout: Duration,
        selected_target: Option<ProviderEgressTarget>,
    ) -> (LiveProviderRelay, PathBuf) {
        let root = root();
        let now_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let policy =
            ProviderEgressPolicy::new("https://openrouter.ai/api/v1", "openrouter.ai").unwrap();
        let egress =
            ProviderEgressHandoff::issue_bound(&policy, "generation-1", "a".repeat(64), u64::MAX);
        let namespace = NamespaceIdentity::current().unwrap();
        let socket = root.join("asb-live-generation-1.sock");
        let placeholder = UnixListener::bind(&socket).unwrap();
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
        let handoff = LiveProviderNamespaceHandoff::issue(
            &policy,
            &egress,
            namespace.clone(),
            "generation-1",
            "a".repeat(64),
            "b".repeat(64),
            "c".repeat(64),
            socket.clone(),
            root.join("child.sock"),
            now_unix_ms + 3_600_000,
            now_unix_ms,
        )
        .unwrap();
        drop(placeholder);
        fs::remove_file(&socket).unwrap();
        let target = selected_target.unwrap_or_else(|| {
            ProviderEgressTarget::test_only("198.51.100.10:443".parse().unwrap())
        });
        let allowlist = ProviderEgressAllowlist::new(vec![target]).unwrap();
        let relay = LiveProviderRelay::bind(
            &root,
            &policy,
            &egress,
            handoff,
            namespace,
            allowlist,
            target,
            "generation-1",
            "a".repeat(64),
            io_timeout,
            4096,
            now_unix_ms,
        )
        .unwrap();
        (relay, root)
    }

    fn relay() -> (LiveProviderRelay, PathBuf) {
        relay_with_timeout(Duration::from_secs(1), None)
    }

    #[test]
    fn wrong_capability_does_not_consume_listener() {
        let (mut relay, root) = relay();
        let peer = UnixStream::connect(relay.socket()).unwrap();
        let socket = relay.socket().to_owned();
        let worker = thread::spawn(move || {
            let mut peer = peer;
            std::io::Write::write_all(&mut peer, b"ASB-LIVE/1 generation-1 bad\n").unwrap();
        });
        assert!(matches!(
            relay.accept_and_connect(0),
            Err(LiveRelayError::Unauthorized)
        ));
        worker.join().unwrap();
        assert!(socket.exists());
        drop(relay);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn teardown_removes_socket_and_revokes_capability() {
        let (relay, root) = relay();
        let socket = relay.socket().to_owned();
        relay.revoke();
        drop(relay);
        assert!(!socket.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn malformed_oversized_request_is_rejected_without_consuming() {
        let (mut relay, root) = relay();
        let mut peer = UnixStream::connect(relay.socket()).unwrap();
        let worker = thread::spawn(move || {
            let request = vec![b'x'; MAX_REQUEST];
            std::io::Write::write_all(&mut peer, &request).unwrap();
        });
        assert!(matches!(
            relay.accept_and_connect(2_000),
            Err(LiveRelayError::InvalidRequest)
        ));
        worker.join().unwrap();
        assert!(!relay.consumed);
        drop(relay);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn expired_and_revoked_handoffs_are_rejected() {
        let (mut relay, root) = relay();
        let capability = relay.capability_sha256.clone();
        let generation = relay.generation.clone();
        let peer = UnixStream::connect(relay.socket()).unwrap();
        let worker = thread::spawn(move || {
            let mut peer = peer;
            let request = format!("ASB-LIVE/1 {generation} {capability}\n");
            std::io::Write::write_all(&mut peer, request.as_bytes()).unwrap();
        });
        assert!(matches!(
            relay.accept_and_connect(u64::MAX),
            Err(LiveRelayError::Handoff(_))
        ));
        worker.join().unwrap();
        relay.revoke();
        let peer = UnixStream::connect(relay.socket()).unwrap();
        let capability = relay.capability_sha256.clone();
        let generation = relay.generation.clone();
        let worker = thread::spawn(move || {
            let mut peer = peer;
            let request = format!("ASB-LIVE/1 {generation} {capability}\n");
            std::io::Write::write_all(&mut peer, request.as_bytes()).unwrap();
        });
        assert!(matches!(
            relay.accept_and_connect(2_000),
            Err(LiveRelayError::Handoff(_))
        ));
        worker.join().unwrap();
        drop(relay);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn expired_accept_deadline_fails_closed_without_a_child() {
        let (mut relay, root) = relay_with_timeout(Duration::from_millis(1), None);
        assert!(matches!(
            relay.accept_and_connect(2_000),
            Err(LiveRelayError::Io(error)) if error.kind() == io::ErrorKind::TimedOut
        ));
        drop(relay);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn consumed_relay_rejects_duplicate_attempts() {
        let (mut relay, root) = relay();
        relay.consumed = true;
        assert!(matches!(
            relay.accept_and_connect(2_000),
            Err(LiveRelayError::Duplicate)
        ));
        drop(relay);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn authenticated_child_reaches_runtime_selected_synthetic_target() {
        let tcp_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let target = ProviderEgressTarget::test_only(tcp_listener.local_addr().unwrap());
        let (mut relay, root) = relay_with_timeout(Duration::from_secs(1), Some(target));
        let capability = relay.capability_sha256.clone();
        let generation = relay.generation.clone();
        let peer = UnixStream::connect(relay.socket()).unwrap();
        let worker = thread::spawn(move || {
            let mut peer = peer;
            let request = format!("ASB-LIVE/1 {generation} {capability}\n");
            std::io::Write::write_all(&mut peer, request.as_bytes()).unwrap();
        });
        let (_child, _provider) = relay.accept_and_connect(2_000).unwrap();
        let (_provider_peer, _) = tcp_listener.accept().unwrap();
        worker.join().unwrap();
        drop(relay);
        let _ = fs::remove_dir_all(root);
    }
}
