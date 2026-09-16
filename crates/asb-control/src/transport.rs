// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Owner-only Linux Unix-domain transport.

use std::fs::{self, Permissions};
use std::io;
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::os::fd::AsFd;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::{Duration, Instant};

use rustix::net::sockopt::socket_peercred;
use rustix::process::geteuid;
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, ServerConfig, ServerConnection, StreamOwned};
use thiserror::Error;

use crate::ControlLimits;

/// Explicit admission policy for the optional remote control boundary.
///
/// This contract is intentionally separate from the owner-only Unix socket:
/// remote transport is never enabled by frontend location or environment.
/// A later TLS listener must consume this validated value rather than infer
/// an address or security policy from process state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RemoteTransportConfig {
    /// Explicitly opt in to the remote listener.
    pub enabled: bool,
    /// Address selected by the operator; no implicit default is permitted.
    pub bind: SocketAddr,
    /// Maximum simultaneously admitted remote connections.
    pub max_connections: u16,
    /// Maximum requests admitted on one connection.
    pub max_requests_per_connection: u16,
    /// Idle socket deadline after handshake.
    pub idle_timeout_ms: u64,
    /// Bounds negotiated frames and request deadlines.
    pub limits: ControlLimits,
}

/// Authenticated TLS configuration for the explicitly enabled remote boundary.
pub struct RemoteTlsConfig {
    server: Arc<ServerConfig>,
}

impl Clone for RemoteTlsConfig {
    fn clone(&self) -> Self {
        Self {
            server: Arc::clone(&self.server),
        }
    }
}

/// Explicitly bound remote listener with a bounded authenticated connection count.
#[derive(Debug)]
pub struct RemoteListener {
    listener: TcpListener,
    tls: RemoteTlsConfig,
    config: RemoteTransportConfig,
    active: Arc<AtomicU16>,
}

impl RemoteListener {
    /// Bind the configured address; no listener exists when remote transport is disabled.
    pub fn bind(
        tls: RemoteTlsConfig,
        config: RemoteTransportConfig,
    ) -> Result<Self, TransportError> {
        let config = config.validate()?;
        let listener = TcpListener::bind(config.bind)?;
        Ok(Self {
            listener,
            tls,
            config,
            active: Arc::new(AtomicU16::new(0)),
        })
    }

    /// Accept one mTLS connection while enforcing the configured capacity.
    pub fn accept(
        &self,
    ) -> Result<
        (
            StreamOwned<ServerConnection, TcpStream>,
            RemoteConnectionPermit,
        ),
        TransportError,
    > {
        let permit = RemoteConnectionPermit::acquire(
            Arc::clone(&self.active),
            self.config.max_connections,
            self.config.max_requests_per_connection,
        )?;
        let (stream, _) = self.listener.accept()?;
        match self.tls.accept(stream, self.config) {
            Ok(stream) => Ok((stream, permit)),
            Err(error) => {
                drop(permit);
                Err(error)
            }
        }
    }

    /// Address selected by the operator.
    #[must_use]
    pub fn local_addr(&self) -> Result<SocketAddr, TransportError> {
        self.listener.local_addr().map_err(TransportError::Io)
    }
}

/// RAII permit for one authenticated remote connection.
#[derive(Debug)]
pub struct RemoteConnectionPermit {
    active: Arc<AtomicU16>,
    requests: RemoteRequestGate,
}

/// Bounded per-connection request admission gate.
#[derive(Debug)]
pub struct RemoteRequestGate {
    remaining: AtomicU16,
}

impl RemoteRequestGate {
    /// Create a gate with a finite request ceiling.
    pub fn new(maximum: u16) -> Result<Self, TransportError> {
        if maximum == 0 || maximum > 1024 {
            return Err(TransportError::RemoteInvalidLimit);
        }
        Ok(Self {
            remaining: AtomicU16::new(maximum),
        })
    }

    /// Reserve one request, failing closed when backpressure is reached.
    pub fn acquire(&self) -> Result<RemoteRequestPermit<'_>, TransportError> {
        let mut current = self.remaining.load(Ordering::Acquire);
        loop {
            if current == 0 {
                return Err(TransportError::RemoteBackpressure);
            }
            match self.remaining.compare_exchange_weak(
                current,
                current - 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Ok(RemoteRequestPermit { gate: self }),
                Err(observed) => current = observed,
            }
        }
    }
}

/// RAII request reservation that returns capacity on completion.
#[derive(Debug)]
pub struct RemoteRequestPermit<'a> {
    gate: &'a RemoteRequestGate,
}

impl Drop for RemoteRequestPermit<'_> {
    fn drop(&mut self) {
        self.gate.remaining.fetch_add(1, Ordering::Release);
    }
}

impl RemoteConnectionPermit {
    fn acquire(
        active: Arc<AtomicU16>,
        maximum: u16,
        request_maximum: u16,
    ) -> Result<Self, TransportError> {
        let mut current = active.load(Ordering::Acquire);
        loop {
            if current >= maximum {
                return Err(TransportError::RemoteCapacity);
            }
            match active.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    let requests = RemoteRequestGate::new(request_maximum)?;
                    return Ok(Self { active, requests });
                }
                Err(observed) => current = observed,
            }
        }
    }

    /// Admit one request on this connection under its bounded backpressure gate.
    pub fn admit_request(&self) -> Result<RemoteRequestPermit<'_>, TransportError> {
        self.requests.acquire()
    }
}

impl Drop for RemoteConnectionPermit {
    fn drop(&mut self) {
        let _ = self
            .active
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                count.checked_sub(1)
            });
    }
}

impl std::fmt::Debug for RemoteTlsConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RemoteTlsConfig")
            .finish_non_exhaustive()
    }
}

impl RemoteTlsConfig {
    /// Build a server configuration and pin the ASB protocol ALPN identifier.
    pub fn new(mut server: ServerConfig) -> Result<Self, TransportError> {
        server.alpn_protocols = vec![b"asb-control/1".to_vec()];
        Ok(Self {
            server: Arc::new(server),
        })
    }

    /// Complete one bounded TLS handshake before application framing.
    pub fn accept(
        &self,
        stream: TcpStream,
        config: RemoteTransportConfig,
    ) -> Result<StreamOwned<ServerConnection, TcpStream>, TransportError> {
        let config = config.validate()?;
        let deadline = Duration::from_millis(config.limits.max_timeout_ms);
        let expires = Instant::now() + deadline;
        let connection = ServerConnection::new(Arc::clone(&self.server))
            .map_err(|_| TransportError::RemoteTlsConfiguration)?;
        let mut tls = StreamOwned::new(connection, stream);
        while tls.conn.is_handshaking() {
            let remaining = expires.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(TransportError::RemoteHandshakeTimeout);
            }
            tls.sock.set_read_timeout(Some(remaining))?;
            tls.sock.set_write_timeout(Some(remaining))?;
            tls.conn.complete_io(&mut tls.sock).map_err(|error| {
                if matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) {
                    TransportError::RemoteHandshakeTimeout
                } else {
                    TransportError::Io(error)
                }
            })?;
        }
        if tls.conn.alpn_protocol() != Some(b"asb-control/1") {
            return Err(TransportError::RemoteAlpnMismatch);
        }
        Ok(tls)
    }
}

/// Client side of the authenticated remote control TLS boundary.
#[derive(Debug)]
pub struct RemoteTlsClient {
    config: Arc<ClientConfig>,
}

impl RemoteTlsClient {
    /// Build a client configuration pinned to the ASB control ALPN.
    pub fn new(mut config: ClientConfig) -> Self {
        config.alpn_protocols = vec![b"asb-control/1".to_vec()];
        Self {
            config: Arc::new(config),
        }
    }

    /// Connect and complete the TLS handshake under the supplied control bound.
    pub fn connect(
        &self,
        stream: TcpStream,
        server_name: &str,
        config: RemoteTransportConfig,
    ) -> Result<StreamOwned<ClientConnection, TcpStream>, TransportError> {
        let config = config.validate()?;
        let name = ServerName::try_from(server_name.to_owned())
            .map_err(|_| TransportError::RemoteTlsConfiguration)?;
        let expires = Instant::now() + Duration::from_millis(config.limits.max_timeout_ms);
        let connection = ClientConnection::new(Arc::clone(&self.config), name)
            .map_err(|_| TransportError::RemoteTlsConfiguration)?;
        let mut tls = StreamOwned::new(connection, stream);
        while tls.conn.is_handshaking() {
            let remaining = expires.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(TransportError::RemoteHandshakeTimeout);
            }
            tls.sock.set_read_timeout(Some(remaining))?;
            tls.sock.set_write_timeout(Some(remaining))?;
            tls.conn.complete_io(&mut tls.sock).map_err(|error| {
                if matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) {
                    TransportError::RemoteHandshakeTimeout
                } else {
                    TransportError::Io(error)
                }
            })?;
        }
        if tls.conn.alpn_protocol() != Some(b"asb-control/1") {
            return Err(TransportError::RemoteAlpnMismatch);
        }
        Ok(tls)
    }
}

impl RemoteTransportConfig {
    /// Validate the explicit remote admission boundary.
    pub fn validate(self) -> Result<Self, TransportError> {
        if !self.enabled {
            return Err(TransportError::RemoteDisabled);
        }
        if self.bind.port() == 0 {
            return Err(TransportError::RemoteInvalidBind);
        }
        if self.bind.ip().is_unspecified() {
            return Err(TransportError::RemoteWildcardBind);
        }
        if matches!(self.bind.ip(), IpAddr::V4(ip) if ip.is_broadcast()) {
            return Err(TransportError::RemoteInvalidBind);
        }
        if self.max_connections == 0 || self.max_connections > 256 {
            return Err(TransportError::RemoteInvalidLimit);
        }
        if self.max_requests_per_connection == 0 || self.max_requests_per_connection > 1024 {
            return Err(TransportError::RemoteInvalidLimit);
        }
        if self.idle_timeout_ms == 0 || self.idle_timeout_ms > self.limits.max_timeout_ms {
            return Err(TransportError::RemoteInvalidLimit);
        }
        self.limits.validate().map_err(TransportError::Protocol)?;
        Ok(self)
    }
}

/// Authenticated local peer identity obtained from the kernel, never client data.
///
/// Its fields are intentionally private: callers can inspect an identity but
/// cannot construct evidence that did not come from `SO_PEERCRED`.
///
/// ```compile_fail
/// use asb_control::PeerIdentity;
/// let _forged = PeerIdentity { uid: 1000, gid: 1000, pid: 1 };
/// ```
///
/// ```compile_fail
/// fn rewrite(identity: &mut asb_control::PeerIdentity) {
///     identity.uid = 0;
/// }
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PeerIdentity {
    /// Effective Linux user ID.
    uid: u32,
    /// Effective Linux group ID.
    gid: u32,
    /// Linux peer process ID at connection time.
    pid: i32,
}

impl PeerIdentity {
    fn from_stream(stream: &UnixStream) -> Result<Self, TransportError> {
        Self::from_fd(stream)
    }

    pub(crate) fn from_fd(fd: impl AsFd) -> Result<Self, TransportError> {
        let credentials = socket_peercred(fd).map_err(io::Error::from)?;
        Ok(Self {
            uid: credentials.uid.as_raw(),
            gid: credentials.gid.as_raw(),
            pid: credentials.pid.as_raw_pid(),
        })
    }

    /// Effective Linux user ID authenticated by the kernel.
    #[must_use]
    pub fn uid(self) -> u32 {
        self.uid
    }

    /// Effective Linux group ID authenticated by the kernel.
    #[must_use]
    pub fn gid(self) -> u32 {
        self.gid
    }

    /// Linux peer process ID observed when the connection was authenticated.
    #[must_use]
    pub fn pid(self) -> i32 {
        self.pid
    }

    /// Require the kernel-authenticated peer to match the listener owner.
    pub fn require_owner(self, expected_uid: u32) -> Result<Self, TransportError> {
        if self.uid == expected_uid {
            Ok(self)
        } else {
            Err(TransportError::UnauthorizedPeer {
                expected_uid,
                actual_uid: self.uid,
            })
        }
    }
}

/// Listener bound in a private runtime directory.
#[derive(Debug)]
pub struct OwnerSocket {
    listener: UnixListener,
    path: PathBuf,
    device: u64,
    inode: u64,
    limits: ControlLimits,
    expected_uid: u32,
}

impl OwnerSocket {
    /// Bind a new socket, refusing symlinks, existing paths, and shared parents.
    pub fn bind(path: impl AsRef<Path>, limits: ControlLimits) -> Result<Self, TransportError> {
        let limits = limits.validate().map_err(TransportError::Protocol)?;
        let path = path.as_ref();
        if path.file_name().is_none() {
            return Err(TransportError::InvalidSocketPath);
        }
        let parent = path.parent().ok_or(TransportError::InvalidSocketPath)?;
        validate_private_directory(parent)?;
        match fs::symlink_metadata(path) {
            Ok(_) => return Err(TransportError::SocketPathExists),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(TransportError::Io(error)),
        }
        let listener = UnixListener::bind(path)?;
        let initial = fs::symlink_metadata(path)?;
        if !initial.file_type().is_socket() || initial.uid() != geteuid().as_raw() {
            return Err(TransportError::UnsafeSocket);
        }
        let identity = (initial.dev(), initial.ino());
        let setup = (|| {
            fs::set_permissions(path, Permissions::from_mode(0o600))?;
            let metadata = fs::symlink_metadata(path)?;
            if !metadata.file_type().is_socket()
                || metadata.uid() != geteuid().as_raw()
                || (metadata.dev(), metadata.ino()) != identity
            {
                return Err(TransportError::UnsafeSocket);
            }
            if metadata.mode() & 0o177 != 0 {
                return Err(TransportError::UnsafeSocket);
            }
            Ok((metadata.dev(), metadata.ino()))
        })();
        let (device, inode) = match setup {
            Ok(identity) => identity,
            Err(error) => {
                remove_if_same_socket(path, identity.0, identity.1);
                return Err(error);
            }
        };
        Ok(Self {
            listener,
            path: path.to_path_buf(),
            device,
            inode,
            limits,
            expected_uid: geteuid().as_raw(),
        })
    }

    /// Accept one same-user peer and install bounded blocking I/O deadlines.
    pub fn accept(&self) -> Result<(UnixStream, PeerIdentity), TransportError> {
        let (stream, _) = self.listener.accept()?;
        let identity = authenticate_owner(&stream, self.expected_uid)?;
        let deadline = Duration::from_millis(self.limits.max_timeout_ms);
        stream.set_read_timeout(Some(deadline))?;
        stream.set_write_timeout(Some(deadline))?;
        Ok((stream, identity))
    }

    /// Borrow the listener for event-loop registration without changing ownership.
    #[must_use]
    pub fn listener(&self) -> &UnixListener {
        &self.listener
    }

    /// Filesystem socket path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Authenticate the peer at either end of a connected Unix stream.
///
/// Servers use this after `accept`; clients use it immediately after
/// `connect`, before writing negotiation data to an untrusted socket.
pub fn authenticate_owner(
    stream: &UnixStream,
    expected_uid: u32,
) -> Result<PeerIdentity, TransportError> {
    PeerIdentity::from_stream(stream)?.require_owner(expected_uid)
}

/// Apply a request-specific I/O deadline no weaker than negotiated limits.
pub fn apply_request_deadline(
    stream: &UnixStream,
    timeout_ms: u64,
    limits: ControlLimits,
) -> Result<(), TransportError> {
    let limits = limits.validate().map_err(TransportError::Protocol)?;
    if timeout_ms == 0 || timeout_ms > limits.max_timeout_ms {
        return Err(TransportError::InvalidDeadline);
    }
    let deadline = Duration::from_millis(timeout_ms);
    stream.set_read_timeout(Some(deadline))?;
    stream.set_write_timeout(Some(deadline))?;
    Ok(())
}

impl Drop for OwnerSocket {
    fn drop(&mut self) {
        let Ok(metadata) = fs::symlink_metadata(&self.path) else {
            return;
        };
        if metadata.file_type().is_socket()
            && metadata.dev() == self.device
            && metadata.ino() == self.inode
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn validate_private_directory(path: &Path) -> Result<(), TransportError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(TransportError::UnsafeRuntimeDirectory);
    }
    if metadata.uid() != geteuid().as_raw() || metadata.mode() & 0o077 != 0 {
        return Err(TransportError::UnsafeRuntimeDirectory);
    }
    Ok(())
}

fn remove_if_same_socket(path: &Path, device: u64, inode: u64) {
    if fs::symlink_metadata(path).is_ok_and(|metadata| {
        metadata.file_type().is_socket() && metadata.dev() == device && metadata.ino() == inode
    }) {
        let _ = fs::remove_file(path);
    }
}

/// Local control transport setup or authentication failure.
#[derive(Debug, Error)]
pub enum TransportError {
    /// Filesystem or socket operation failed.
    #[error("local control transport I/O failed")]
    Io(#[from] io::Error),
    /// Negotiated control limits were invalid.
    #[error("local control transport limits are invalid")]
    Protocol(#[source] crate::ProtocolError),
    /// Per-request deadline is zero or above the negotiated maximum.
    #[error("local control request deadline is invalid")]
    InvalidDeadline,
    /// Socket path has no safe parent/name split.
    #[error("local control socket path is invalid")]
    InvalidSocketPath,
    /// Runtime directory is not a private same-user real directory.
    #[error("local control runtime directory is unsafe")]
    UnsafeRuntimeDirectory,
    /// Refuse to replace or unlink any pre-existing path.
    #[error("local control socket path already exists")]
    SocketPathExists,
    /// Bound node is not a private same-user socket.
    #[error("local control socket permissions or ownership are unsafe")]
    UnsafeSocket,
    /// Remote control was not explicitly enabled.
    #[error("remote control transport is disabled")]
    RemoteDisabled,
    /// Remote control bind address or port is invalid.
    #[error("remote control bind address is invalid")]
    RemoteInvalidBind,
    /// Wildcard remote binds require a separately reviewed policy.
    #[error("remote control wildcard bind is not permitted")]
    RemoteWildcardBind,
    /// Remote connection limit is outside the implementation bound.
    #[error("remote control connection limit is invalid")]
    RemoteInvalidLimit,
    /// TLS server configuration could not be initialized.
    #[error("remote TLS configuration is invalid")]
    RemoteTlsConfiguration,
    /// TLS negotiation exceeded the configured deadline.
    #[error("remote TLS handshake deadline exceeded")]
    RemoteHandshakeTimeout,
    /// The peer did not negotiate the ASB control ALPN.
    #[error("remote TLS ALPN negotiation failed")]
    RemoteAlpnMismatch,
    /// Remote listener connection capacity has been exhausted.
    #[error("remote control connection capacity is exhausted")]
    RemoteCapacity,
    /// Per-connection request capacity has been exhausted.
    #[error("remote control request backpressure limit reached")]
    RemoteBackpressure,
    /// Kernel-authenticated peer belongs to another user.
    #[error("local control peer is not owned by the expected user")]
    UnauthorizedPeer {
        /// Listener owner.
        expected_uid: u32,
        /// Kernel-reported peer.
        actual_uid: u32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CONTROL_V1, ControlVersion, FrameError, read_frame, write_frame};
    use rcgen::generate_simple_self_signed;
    use rustls::RootCertStore;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
    use rustls::server::WebPkiClientVerifier;
    use std::io::Write;
    use std::net::{Shutdown, TcpListener};
    use std::thread;

    #[test]
    fn guarded_cleanup_removes_only_matching_socket_nodes() {
        let root = std::env::var_os("ASB_TEST_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join(format!("asb-control-cleanup-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let regular = root.join("regular");
        fs::write(&regular, b"keep").unwrap();
        remove_if_same_socket(&regular, 0, 0);
        assert_eq!(fs::read(&regular).unwrap(), b"keep");
        let socket = root.join("socket");
        let listener = UnixListener::bind(&socket).unwrap();
        let metadata = fs::symlink_metadata(&socket).unwrap();
        remove_if_same_socket(&socket, metadata.dev(), metadata.ino() + 1);
        assert!(socket.exists());
        remove_if_same_socket(&socket, metadata.dev(), metadata.ino());
        assert!(!socket.exists());
        drop(listener);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn peer_identity_is_kernel_derived_and_owner_checked() {
        let (left, right) = UnixStream::pair().expect("socket pair");
        let expected = geteuid().as_raw();
        let identity = authenticate_owner(&left, expected).expect("same-user peer");
        assert_eq!(identity.uid(), expected);
        assert_eq!(identity.gid(), rustix::process::getegid().as_raw());
        assert!(identity.pid() > 0);
        assert!(matches!(
            authenticate_owner(&right, expected.wrapping_add(1)),
            Err(TransportError::UnauthorizedPeer {
                expected_uid,
                actual_uid
            }) if expected_uid == expected.wrapping_add(1) && actual_uid == expected
        ));
        let rejected = authenticate_owner(&right, expected.wrapping_add(1))
            .expect_err("wrong owner must fail");
        assert_eq!(
            rejected.to_string(),
            "local control peer is not owned by the expected user"
        );
    }

    fn remote_config() -> RemoteTransportConfig {
        RemoteTransportConfig {
            enabled: true,
            bind: "127.0.0.1:9443".parse().unwrap(),
            max_connections: 8,
            max_requests_per_connection: 32,
            idle_timeout_ms: 5_000,
            limits: ControlLimits::default(),
        }
    }

    #[test]
    fn remote_admission_requires_explicit_non_wildcard_bind() {
        assert!(matches!(
            RemoteTransportConfig {
                enabled: false,
                ..remote_config()
            }
            .validate(),
            Err(TransportError::RemoteDisabled)
        ));
        assert!(matches!(
            RemoteTransportConfig {
                bind: "0.0.0.0:9443".parse().unwrap(),
                ..remote_config()
            }
            .validate(),
            Err(TransportError::RemoteWildcardBind)
        ));
        assert!(remote_config().validate().is_ok());
    }

    #[test]
    fn remote_admission_rejects_unbounded_connection_limits() {
        assert!(matches!(
            RemoteTransportConfig {
                max_connections: 0,
                ..remote_config()
            }
            .validate(),
            Err(TransportError::RemoteInvalidLimit)
        ));
        assert!(matches!(
            RemoteTransportConfig {
                max_connections: 257,
                ..remote_config()
            }
            .validate(),
            Err(TransportError::RemoteInvalidLimit)
        ));
    }

    #[test]
    fn remote_connection_capacity_is_raii_bounded() {
        let active = Arc::new(AtomicU16::new(0));
        let first = RemoteConnectionPermit::acquire(Arc::clone(&active), 1, 1).unwrap();
        let request = first.admit_request().unwrap();
        assert!(matches!(
            first.admit_request(),
            Err(TransportError::RemoteBackpressure)
        ));
        drop(request);
        assert!(matches!(
            RemoteConnectionPermit::acquire(Arc::clone(&active), 1, 1),
            Err(TransportError::RemoteCapacity)
        ));
        drop(first);
        assert!(RemoteConnectionPermit::acquire(active, 1, 1).is_ok());
    }

    #[test]
    fn remote_request_gate_applies_backpressure_and_recovers() {
        let gate = RemoteRequestGate::new(1).unwrap();
        let permit = gate.acquire().unwrap();
        assert!(matches!(
            gate.acquire(),
            Err(TransportError::RemoteBackpressure)
        ));
        drop(permit);
        assert!(gate.acquire().is_ok());
        assert!(matches!(
            RemoteRequestGate::new(0),
            Err(TransportError::RemoteInvalidLimit)
        ));
    }

    fn test_tls_configs() -> (RemoteTlsConfig, RemoteTlsClient, RemoteTlsClient) {
        let generated = generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let certificate = CertificateDer::from(generated.cert.der().to_vec());
        let key =
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(generated.key_pair.serialize_der()));
        let mut roots = RootCertStore::empty();
        roots.add(certificate.clone()).unwrap();
        let verifier = WebPkiClientVerifier::builder(Arc::new(roots.clone()))
            .build()
            .unwrap();
        let server = ServerConfig::builder()
            .with_client_cert_verifier(verifier)
            .with_single_cert(vec![certificate.clone()], key.clone_key())
            .unwrap();
        let client = ClientConfig::builder().with_root_certificates(roots);
        let authenticated = client
            .clone()
            .with_client_auth_cert(vec![certificate], key)
            .unwrap();
        let unauthenticated = client.with_no_client_auth();
        (
            RemoteTlsConfig::new(server).unwrap(),
            RemoteTlsClient::new(authenticated),
            RemoteTlsClient::new(unauthenticated),
        )
    }

    #[test]
    fn tls_frame_round_trip_uses_authenticated_mtls_and_alpn() {
        let (server, client, _) = test_tls_configs();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server_thread = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut tls = server
                .accept(stream, remote_config())
                .expect("mutual TLS handshake");
            let version: ControlVersion = read_frame(&mut tls, remote_config().limits).unwrap();
            assert_eq!(version, CONTROL_V1);
            write_frame(&mut tls, &version, remote_config().limits).unwrap();
        });
        let stream = TcpStream::connect(address).unwrap();
        let mut tls = client
            .connect(stream, "localhost", remote_config())
            .expect("mutual TLS client handshake");
        write_frame(&mut tls, &CONTROL_V1, remote_config().limits).unwrap();
        let response: ControlVersion = read_frame(&mut tls, remote_config().limits).unwrap();
        assert_eq!(response, CONTROL_V1);
        server_thread.join().unwrap();
    }

    #[test]
    fn tls_frame_rejects_oversized_length_without_allocating() {
        let (server, client, _) = test_tls_configs();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server_thread = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut tls = server.accept(stream, remote_config()).unwrap();
            matches!(
                read_frame::<ControlVersion>(&mut tls, remote_config().limits),
                Err(FrameError::InvalidLength { .. })
            )
        });
        let stream = TcpStream::connect(address).unwrap();
        let mut tls = client
            .connect(stream, "localhost", remote_config())
            .unwrap();
        tls.write_all(&(u32::MAX.to_be_bytes())).unwrap();
        assert!(server_thread.join().unwrap());
    }

    #[test]
    fn tls_frame_rejects_malformed_and_truncated_payloads() {
        for truncated in [false, true] {
            let (server, client, _) = test_tls_configs();
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let server_thread = thread::spawn(move || {
                let (stream, _) = listener.accept().unwrap();
                let mut tls = server.accept(stream, remote_config()).unwrap();
                read_frame::<ControlVersion>(&mut tls, remote_config().limits).unwrap_err()
            });
            let stream = TcpStream::connect(address).unwrap();
            let mut tls = client
                .connect(stream, "localhost", remote_config())
                .unwrap();
            if truncated {
                tls.write_all(&[0, 0, 0]).unwrap();
                tls.sock.shutdown(Shutdown::Write).unwrap();
            } else {
                tls.write_all(&[0, 0, 0, 2, b'{', b'}']).unwrap();
            }
            let error = server_thread.join().unwrap();
            assert!(matches!(
                (truncated, error),
                (true, FrameError::Truncated) | (false, FrameError::MalformedJson(_))
            ));
        }
    }

    #[test]
    fn tls_handshake_rejects_slow_peer_at_bounded_deadline() {
        let (server, client, _) = test_tls_configs();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let bounded = RemoteTransportConfig {
            limits: ControlLimits {
                max_timeout_ms: 100,
                ..ControlLimits::default()
            },
            ..remote_config()
        };
        let server_thread = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            thread::sleep(Duration::from_millis(300));
            server.accept(stream, bounded).is_err()
        });
        let stream = TcpStream::connect(address).unwrap();
        let started = Instant::now();
        let _ = client.connect(stream, "localhost", bounded);
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(server_thread.join().unwrap());
    }

    #[test]
    fn tls_server_rejects_client_without_certificate() {
        let (server, _, unauthenticated_client) = test_tls_configs();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server_thread = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            server.accept(stream, remote_config()).is_err()
        });
        let stream = TcpStream::connect(address).unwrap();
        let _ = unauthenticated_client.connect(stream, "localhost", remote_config());
        assert!(server_thread.join().unwrap());
    }
}
