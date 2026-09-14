// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Closed binary framing and generation state for authenticated control handoff.

use std::collections::BTreeSet;
use std::fs::{self, Permissions};
use std::io::{self, IoSlice, IoSliceMut, Read};
use std::mem::MaybeUninit;
use std::os::fd::{AsFd, OwnedFd};
use std::os::linux::net::SocketAddrExt;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::fs::{OFlags, fcntl_getfl, fcntl_setfl};
use rustix::net::sockopt::{
    Timeout, set_socket_timeout, socket_acceptconn, socket_domain, socket_error, socket_passcred,
    socket_type,
};
use rustix::net::{
    AddressFamily, RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, ReturnFlags,
    SendAncillaryBuffer, SendAncillaryMessage, SendFlags, SocketAddrUnix, SocketFlags, SocketType,
    accept_with, bind, connect, getpeername, listen, recv, recvmsg, sendmsg, socket_with,
    socketpair,
};
use rustix::process::geteuid;
use rustix::rand::{GetRandomFlags, getrandom};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::endpoint::{AdmissionGuard, join_workers, reap_workers};
use crate::{
    CONTROL_MEASUREMENT_SELECTION_V1, ControlBackend, ControlCall, ControlClient, ControlLimits,
    ControlServer, EndpointError, MAX_CONTROL_ID_BYTES, PeerIdentity, RequestDeadline,
    SUPPORTED_CONTROL_VERSIONS, validate_identity,
};

/// Exact byte length of one router/frontend broker packet.
pub const BROKER_PACKET_BYTES: usize = 72;
/// Exact byte length of one service provisioning packet.
pub const PROVISIONING_PACKET_BYTES: usize = 64;
/// Maximum replacement acquisitions during one frontend lifetime.
pub const MAX_REPLACEMENTS: u8 = 8;
/// Minimum interval between replacement acquisitions.
pub const MIN_REPLACEMENT_INTERVAL: Duration = Duration::from_millis(100);
/// End-to-end budget for one broker acquisition.
pub const BROKER_ACQUISITION_TIMEOUT: Duration = Duration::from_millis(2_000);

const BROKER_MAGIC: [u8; 8] = *b"ASBHND01";
const PROVISIONING_MAGIC: [u8; 8] = *b"ASBPRV01";
const WIRE_VERSION: u16 = 1;
const RUNNER_IDENTITY_DOMAIN: &[u8] = b"asb-control-runner-instance-v1";
const PROVISIONING_BACKLOG: i32 = 16;
const PROC_STAT_MAX_BYTES: u64 = 4_096;
const ENTROPY_ATTEMPTS: usize = 4;

/// One ASB service exposing distinct ordinary-control and private provisioning endpoints.
pub struct ProvisionedControlServer<B> {
    control: ControlServer<B>,
    provisioning: ProvisioningSocket,
    runner_identity: [u8; 32],
    request_timeout: Duration,
}

impl<B: ControlBackend + Send + Sync + 'static> ProvisionedControlServer<B> {
    /// Bind distinct owner-private endpoints under separate private directories.
    pub fn bind(
        control_path: impl AsRef<Path>,
        provisioning_path: impl AsRef<Path>,
        limits: ControlLimits,
        backend: B,
    ) -> Result<Self, ProvisioningError> {
        let control_path = control_path.as_ref();
        let provisioning_path = provisioning_path.as_ref();
        validate_endpoint_topology(control_path, provisioning_path)?;
        let runner_identity = runner_identity_digest(backend.runner_instance_id())?;
        let backend = Arc::new(backend);
        let admission = AdmissionGuard::new();
        let control =
            ControlServer::bind_shared(control_path, limits, Arc::clone(&backend), admission)?;
        let provisioning = ProvisioningSocket::bind(provisioning_path)?;
        Ok(Self {
            control,
            provisioning,
            runner_identity,
            request_timeout: Duration::from_millis(2_000),
        })
    }

    /// Ordinary control endpoint path; callers must keep it private.
    #[must_use]
    pub fn control_path(&self) -> &Path {
        self.control.path()
    }

    /// Provisioning endpoint path; only the trusted lifecycle router resolves it.
    #[must_use]
    pub fn provisioning_path(&self) -> &Path {
        self.provisioning.path()
    }

    /// Serve both endpoints until the process is stopped.
    pub fn serve(&self) -> Result<(), ProvisioningError> {
        self.serve_connections(usize::MAX, usize::MAX)
    }

    /// Serve bounded endpoint counts for supervision and hostile integration tests.
    pub fn serve_connections(
        &self,
        maximum_control: usize,
        maximum_provisioning: usize,
    ) -> Result<(), ProvisioningError> {
        if maximum_control == 0 && maximum_provisioning == 0 {
            return Ok(());
        }
        self.control.set_nonblocking(true)?;
        let mut workers = Vec::new();
        let mut controls = 0_usize;
        let mut provisions = 0_usize;
        while controls < maximum_control || provisions < maximum_provisioning {
            reap_workers(&mut workers);
            let mut progressed = false;
            if controls < maximum_control && self.control.try_accept_and_spawn(&mut workers)? {
                controls += 1;
                progressed = true;
            }
            if provisions < maximum_provisioning
                && let Some(connection) = self.provisioning.try_accept(self.request_timeout)?
            {
                provisions += 1;
                progressed = true;
                // A malformed same-user request is isolated to its one-shot connection.
                let _ = self.provision_one(connection, &mut workers);
            }
            if !progressed {
                thread::sleep(Duration::from_millis(1));
            }
        }
        join_workers(workers);
        Ok(())
    }

    fn provision_one(
        &self,
        connection: OwnedFd,
        workers: &mut Vec<thread::JoinHandle<()>>,
    ) -> Result<(), ProvisioningError> {
        let peer = PeerIdentity::from_fd(&connection)
            .and_then(|identity| identity.require_owner(geteuid().as_raw()))
            .map_err(|_| ProvisioningError::Rejected)?;
        if peer.pid() <= 0 || socket_passcred(&connection).map_err(io::Error::from)? {
            return Err(ProvisioningError::Rejected);
        }
        let request = recv_provisioning_request(&connection)?;
        let (service_fd, frontend_fd) = socketpair(
            AddressFamily::UNIX,
            SocketType::STREAM,
            SocketFlags::CLOEXEC,
            None,
        )
        .map_err(io::Error::from)?;
        let service_stream = UnixStream::from(service_fd);
        let Some(worker) = self.control.try_spawn_anonymous(service_stream)? else {
            let response = ProvisioningPacket {
                status: HandoffStatus::Unavailable,
                nonce: request.nonce,
                runner_identity: [0; 32],
            };
            send_provisioning_response(&connection, response, None)?;
            return Ok(());
        };
        let response = ProvisioningPacket {
            status: HandoffStatus::Success,
            nonce: request.nonce,
            runner_identity: self.runner_identity,
        };
        if let Err(error) = send_provisioning_response(&connection, response, Some(&frontend_fd)) {
            drop(frontend_fd);
            let _ = worker.join();
            return Err(error);
        }
        drop(frontend_fd);
        workers.push(worker);
        Ok(())
    }
}

fn validate_endpoint_topology(
    control: &Path,
    provisioning: &Path,
) -> Result<(), ProvisioningError> {
    let control_parent = control.parent().ok_or(ProvisioningError::UnsafeTopology)?;
    let provisioning_parent = provisioning
        .parent()
        .ok_or(ProvisioningError::UnsafeTopology)?;
    if !control.is_absolute()
        || !provisioning.is_absolute()
        || control == provisioning
        || control.starts_with(provisioning)
        || provisioning.starts_with(control)
        || control_parent == provisioning_parent
        || control_parent.starts_with(provisioning_parent)
        || provisioning_parent.starts_with(control_parent)
    {
        return Err(ProvisioningError::UnsafeTopology);
    }
    Ok(())
}

struct ProvisioningSocket {
    listener: OwnedFd,
    path: PathBuf,
    device: u64,
    inode: u64,
}

impl ProvisioningSocket {
    fn bind(path: &Path) -> Result<Self, ProvisioningError> {
        let parent = path.parent().ok_or(ProvisioningError::UnsafeTopology)?;
        validate_private_directory(parent)?;
        match fs::symlink_metadata(path) {
            Ok(_) => return Err(ProvisioningError::SocketPathExists),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let address = SocketAddrUnix::new(path).map_err(io::Error::from)?;
        let listener = socket_with(
            AddressFamily::UNIX,
            SocketType::SEQPACKET,
            SocketFlags::CLOEXEC | SocketFlags::NONBLOCK,
            None,
        )
        .map_err(io::Error::from)?;
        bind(&listener, &address).map_err(io::Error::from)?;
        let initial = fs::symlink_metadata(path)?;
        let identity = (initial.dev(), initial.ino());
        if !initial.file_type().is_socket() || initial.uid() != geteuid().as_raw() {
            remove_if_same_socket(path, identity.0, identity.1);
            return Err(ProvisioningError::UnsafeSocket);
        }
        let setup = (|| {
            fs::set_permissions(path, Permissions::from_mode(0o600))?;
            let metadata = fs::symlink_metadata(path)?;
            if !metadata.file_type().is_socket()
                || metadata.uid() != geteuid().as_raw()
                || (metadata.dev(), metadata.ino()) != identity
                || metadata.mode() & 0o177 != 0
            {
                return Err(ProvisioningError::UnsafeSocket);
            }
            listen(&listener, PROVISIONING_BACKLOG).map_err(io::Error::from)?;
            if socket_passcred(&listener).map_err(io::Error::from)? {
                return Err(ProvisioningError::UnsafeSocket);
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
        })
    }

    fn try_accept(&self, timeout: Duration) -> Result<Option<OwnedFd>, ProvisioningError> {
        let connection = match accept_with(&self.listener, SocketFlags::CLOEXEC) {
            Ok(connection) => connection,
            Err(error) if error == rustix::io::Errno::AGAIN => return Ok(None),
            Err(error) => return Err(io::Error::from(error).into()),
        };
        set_socket_timeout(&connection, Timeout::Recv, Some(timeout)).map_err(io::Error::from)?;
        set_socket_timeout(&connection, Timeout::Send, Some(timeout)).map_err(io::Error::from)?;
        Ok(Some(connection))
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ProvisioningSocket {
    fn drop(&mut self) {
        remove_if_same_socket(&self.path, self.device, self.inode);
    }
}

fn validate_private_directory(path: &Path) -> Result<(), ProvisioningError> {
    // `symlink_metadata` protects the final directory entry only.  A trusted
    // endpoint authority also requires every ancestor to remain the exact
    // path supplied by the caller: otherwise a replaceable symlink in an
    // ancestor could redirect the endpoint between validation and bind.
    // Callers provide existing runtime directories, so canonicalization is a
    // bounded, read-only check and rejecting aliases is fail-closed.
    if fs::canonicalize(path).ok().as_deref() != Some(path) {
        return Err(ProvisioningError::UnsafeDirectory);
    }
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink()
        || !metadata.is_dir()
        || metadata.uid() != geteuid().as_raw()
        || metadata.mode() & 0o077 != 0
    {
        return Err(ProvisioningError::UnsafeDirectory);
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

fn recv_provisioning_request(
    connection: &OwnedFd,
) -> Result<ProvisioningPacket, ProvisioningError> {
    let mut payload = [0_u8; PROVISIONING_PACKET_BYTES];
    let mut iov = [IoSliceMut::new(&mut payload)];
    let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(2), ScmCredentials(1))];
    let mut ancillary = RecvAncillaryBuffer::new(&mut space);
    let received = recvmsg(
        connection,
        &mut iov,
        &mut ancillary,
        RecvFlags::CMSG_CLOEXEC | RecvFlags::TRUNC,
    )
    .map_err(io::Error::from)?;
    let has_ancillary = ancillary.drain().next().is_some();
    if received.bytes != PROVISIONING_PACKET_BYTES
        || received
            .flags
            .intersects(ReturnFlags::TRUNC | ReturnFlags::CTRUNC)
        || has_ancillary
    {
        return Err(ProvisioningError::Rejected);
    }
    let packet = ProvisioningPacket::decode(&payload)?;
    if packet.status != HandoffStatus::Request {
        return Err(ProvisioningError::Rejected);
    }
    Ok(packet)
}

fn send_provisioning_response(
    connection: &OwnedFd,
    response: ProvisioningPacket,
    frontend: Option<&OwnedFd>,
) -> Result<(), ProvisioningError> {
    let payload = response.encode();
    let iov = [IoSlice::new(&payload)];
    let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
    let mut ancillary = SendAncillaryBuffer::new(&mut space);
    let descriptors = frontend.map(|descriptor| [descriptor.as_fd()]);
    if let Some(descriptors) = descriptors.as_ref()
        && !ancillary.push(SendAncillaryMessage::ScmRights(descriptors))
    {
        return Err(ProvisioningError::Rejected);
    }
    let sent =
        sendmsg(connection, &iov, &mut ancillary, SendFlags::NOSIGNAL).map_err(io::Error::from)?;
    if sent != PROVISIONING_PACKET_BYTES {
        return Err(ProvisioningError::Rejected);
    }
    Ok(())
}

/// Closed provisioning failure without endpoint or credential disclosure.
#[derive(Debug, Error)]
pub enum ProvisioningError {
    /// Ordinary control endpoint setup or service failed.
    #[error("control service unavailable")]
    Control(#[from] EndpointError),
    /// Handoff framing or identity derivation failed.
    #[error("control handoff rejected")]
    Handoff(#[from] HandoffError),
    /// Local transport operation failed.
    #[error("control provisioning unavailable")]
    Io(#[from] io::Error),
    /// Endpoint topology could expose or alias private authority.
    #[error("control endpoint topology is unsafe")]
    UnsafeTopology,
    /// Provisioning runtime directory is not a private same-user directory.
    #[error("control provisioning directory is unsafe")]
    UnsafeDirectory,
    /// Provisioning path already exists and is never replaced.
    #[error("control provisioning endpoint already exists")]
    SocketPathExists,
    /// Bound provisioning node or socket options are unsafe.
    #[error("control provisioning endpoint is unsafe")]
    UnsafeSocket,
    /// Peer, packet, ancillary data, or descriptor transition was rejected.
    #[error("control provisioning request rejected")]
    Rejected,
}

/// Operation encoded by a broker packet.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BrokerOperation {
    /// First acquisition for a new frontend lifetime.
    Initial,
    /// Replacement after an authenticated service disconnect.
    Replacement,
}

impl BrokerOperation {
    const fn wire(self) -> u8 {
        match self {
            Self::Initial => 1,
            Self::Replacement => 2,
        }
    }

    fn parse(value: u8) -> Result<Self, HandoffError> {
        match value {
            1 => Ok(Self::Initial),
            2 => Ok(Self::Replacement),
            _ => Err(HandoffError::InvalidPacket),
        }
    }
}

/// Closed status encoded by broker and provisioning replies.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HandoffStatus {
    /// Request packet; never a reply.
    Request,
    /// One authenticated descriptor accompanies the reply.
    Success,
    /// The authoritative service is temporarily unavailable.
    Unavailable,
    /// The request is invalid or unauthorized.
    Rejected,
    /// The frontend lifetime exhausted its replacement ceiling.
    Exhausted,
}

impl HandoffStatus {
    const fn wire(self) -> u8 {
        match self {
            Self::Request => 0,
            Self::Success => 1,
            Self::Unavailable => 2,
            Self::Rejected => 3,
            Self::Exhausted => 4,
        }
    }

    fn parse_broker(value: u8) -> Result<Self, HandoffError> {
        match value {
            0 => Ok(Self::Request),
            1 => Ok(Self::Success),
            2 => Ok(Self::Unavailable),
            3 => Ok(Self::Rejected),
            4 => Ok(Self::Exhausted),
            _ => Err(HandoffError::InvalidPacket),
        }
    }

    fn parse_provisioning(value: u8) -> Result<Self, HandoffError> {
        match value {
            0 => Ok(Self::Request),
            1 => Ok(Self::Success),
            2 => Ok(Self::Unavailable),
            3 => Ok(Self::Rejected),
            _ => Err(HandoffError::InvalidPacket),
        }
    }

    const fn is_closed_failure(self) -> bool {
        matches!(self, Self::Unavailable | Self::Rejected | Self::Exhausted)
    }
}

/// Monotonic generation accepted during one live broker lifetime.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BrokerGeneration {
    epoch: [u8; 16],
    sequence: u64,
}

impl BrokerGeneration {
    /// Fresh nonzero broker epoch.
    #[must_use]
    pub const fn epoch(self) -> [u8; 16] {
        self.epoch
    }

    /// Nonzero sequence within this live epoch.
    #[must_use]
    pub const fn sequence(self) -> u64 {
        self.sequence
    }
}

/// Exact 72-byte closed broker request or reply.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BrokerPacket {
    /// Packet operation.
    pub operation: BrokerOperation,
    /// Request or closed reply status.
    pub status: HandoffStatus,
    /// Broker epoch; zero only for an initial request or pre-generation failure.
    pub epoch: [u8; 16],
    /// Sequence; zero only for an initial request or pre-generation failure.
    pub sequence: u64,
    /// Expected runner identity; nonzero only in a successful reply.
    pub expected_runner_identity: [u8; 32],
}

impl BrokerPacket {
    /// Return the nonzero generation carried by a successful broker reply.
    pub fn generation(self) -> Result<BrokerGeneration, HandoffError> {
        self.validate_shape()?;
        if self.status != HandoffStatus::Success {
            return Err(HandoffError::UnexpectedState);
        }
        Ok(BrokerGeneration {
            epoch: self.epoch,
            sequence: self.sequence,
        })
    }

    /// Encode the frozen v1 representation.
    #[must_use]
    pub fn encode(self) -> [u8; BROKER_PACKET_BYTES] {
        let mut bytes = [0_u8; BROKER_PACKET_BYTES];
        bytes[..8].copy_from_slice(&BROKER_MAGIC);
        bytes[8..10].copy_from_slice(&WIRE_VERSION.to_be_bytes());
        bytes[10] = self.operation.wire();
        bytes[11] = self.status.wire();
        bytes[16..32].copy_from_slice(&self.epoch);
        bytes[32..40].copy_from_slice(&self.sequence.to_be_bytes());
        bytes[40..].copy_from_slice(&self.expected_runner_identity);
        bytes
    }

    /// Decode the exact frozen v1 representation and reject reserved bits.
    pub fn decode(bytes: &[u8]) -> Result<Self, HandoffError> {
        if bytes.len() != BROKER_PACKET_BYTES
            || bytes[..8] != BROKER_MAGIC
            || bytes[8..10] != WIRE_VERSION.to_be_bytes()
            || bytes[12..16] != [0; 4]
        {
            return Err(HandoffError::InvalidPacket);
        }
        let mut epoch = [0_u8; 16];
        epoch.copy_from_slice(&bytes[16..32]);
        let mut sequence = [0_u8; 8];
        sequence.copy_from_slice(&bytes[32..40]);
        let mut identity = [0_u8; 32];
        identity.copy_from_slice(&bytes[40..72]);
        let packet = Self {
            operation: BrokerOperation::parse(bytes[10])?,
            status: HandoffStatus::parse_broker(bytes[11])?,
            epoch,
            sequence: u64::from_be_bytes(sequence),
            expected_runner_identity: identity,
        };
        packet.validate_shape()?;
        Ok(packet)
    }

    fn validate_shape(self) -> Result<(), HandoffError> {
        let zero_tuple = self.epoch == [0; 16] && self.sequence == 0;
        let zero_identity = self.expected_runner_identity == [0; 32];
        let valid = match (self.status, self.operation) {
            (HandoffStatus::Request, BrokerOperation::Initial) => zero_tuple && zero_identity,
            (HandoffStatus::Request, BrokerOperation::Replacement) => {
                !zero_tuple && self.epoch != [0; 16] && self.sequence != 0 && zero_identity
            }
            (HandoffStatus::Success, _) => {
                self.epoch != [0; 16] && self.sequence != 0 && !zero_identity
            }
            (status, BrokerOperation::Initial) if status.is_closed_failure() => {
                zero_tuple && zero_identity
            }
            (status, BrokerOperation::Replacement) if status.is_closed_failure() => {
                self.epoch != [0; 16] && self.sequence != 0 && zero_identity
            }
            _ => false,
        };
        if valid {
            Ok(())
        } else {
            Err(HandoffError::InvalidPacket)
        }
    }
}

/// ASB-side producer for one authenticated anonymous control generation.
///
/// The endpoint paths remain private inputs to the trusted router. The returned
/// object contains only the anonymous stream and bounded continuity evidence.
pub struct AuthenticatedGenerationProducer {
    control_path: PathBuf,
    provisioning_path: PathBuf,
    limits: ControlLimits,
}

impl AuthenticatedGenerationProducer {
    /// Configure the private endpoints used for a single acquisition.
    pub fn new(
        control_path: impl AsRef<Path>,
        provisioning_path: impl AsRef<Path>,
        limits: ControlLimits,
    ) -> Result<Self, ProvisioningError> {
        let control_path = control_path.as_ref();
        let provisioning_path = provisioning_path.as_ref();
        validate_endpoint_topology(control_path, provisioning_path)?;
        validate_private_directory(
            control_path
                .parent()
                .ok_or(ProvisioningError::UnsafeTopology)?,
        )?;
        validate_private_directory(
            provisioning_path
                .parent()
                .ok_or(ProvisioningError::UnsafeTopology)?,
        )?;
        Ok(Self {
            control_path: control_path.to_path_buf(),
            provisioning_path: provisioning_path.to_path_buf(),
            limits: limits.validate().map_err(EndpointError::from)?,
        })
    }

    /// Acquire one generation under the broker request's diminishing deadline.
    pub fn acquire(
        &self,
        pending: &PendingBrokerSuccess,
    ) -> Result<AuthenticatedGeneration, ProvisioningError> {
        let broker_generation = pending.generation;
        if broker_generation.epoch == [0; 16] || broker_generation.sequence == 0 {
            return Err(ProvisioningError::Rejected);
        }
        let deadline = pending.deadline;
        deadline.check().map_err(EndpointError::from)?;
        let expected_uid = geteuid().as_raw();

        let probe_stream =
            connect_private_socket(&self.control_path, SocketType::STREAM, deadline)?;
        let probe_stream = UnixStream::from(probe_stream);
        let mut probe = ControlClient::from_stream_with_versions_until(
            probe_stream,
            self.limits,
            expected_uid,
            BTreeSet::from(SUPPORTED_CONTROL_VERSIONS),
            deadline,
        )?;
        if probe.negotiated().version != CONTROL_MEASUREMENT_SELECTION_V1 {
            return Err(ProvisioningError::Rejected);
        }
        let probe_evidence = process_evidence(probe.peer_identity(expected_uid)?)?;
        let expected_runner_identity =
            runner_identity_digest(&probe.negotiated().runner_instance_id)?;

        let provisioning =
            connect_private_socket(&self.provisioning_path, SocketType::SEQPACKET, deadline)?;
        if socket_passcred(&provisioning).map_err(io::Error::from)? {
            return Err(ProvisioningError::Rejected);
        }
        let provisioning_evidence = process_evidence(
            PeerIdentity::from_fd(&provisioning)
                .and_then(|identity| identity.require_owner(expected_uid))
                .map_err(|_| ProvisioningError::Rejected)?,
        )?;
        if provisioning_evidence != probe_evidence {
            return Err(ProvisioningError::Rejected);
        }

        let nonce = fresh_nonce()?;
        let request = ProvisioningPacket {
            status: HandoffStatus::Request,
            nonce,
            runner_identity: [0; 32],
        };
        set_deadline_timeouts(&provisioning, deadline)?;
        let payload = request.encode();
        let sent = rustix::net::send(&provisioning, &payload, SendFlags::NOSIGNAL)
            .map_err(io::Error::from)?;
        if sent != PROVISIONING_PACKET_BYTES {
            return Err(ProvisioningError::Rejected);
        }
        set_deadline_timeouts(&provisioning, deadline)?;
        let (response, descriptor) = recv_provisioning_response(&provisioning)?;
        if response.status != HandoffStatus::Success
            || response.nonce != nonce
            || response.runner_identity != expected_runner_identity
        {
            return Err(ProvisioningError::Rejected);
        }
        let kernel_peer = validate_anonymous_stream(&descriptor, probe_evidence, expected_uid)?;
        let stream = UnixStream::from(descriptor);

        let terminal = probe.call_until(ControlCall::Capabilities, deadline)?;
        if terminal.error().is_some()
            || probe.negotiated().runner_instance_id.is_empty()
            || process_evidence(probe.peer_identity(expected_uid)?)? != probe_evidence
            || runner_identity_digest(&probe.negotiated().runner_instance_id)?
                != expected_runner_identity
        {
            return Err(ProvisioningError::Rejected);
        }
        deadline.check().map_err(EndpointError::from)?;
        drop(probe);
        AuthenticatedGeneration::new(
            stream,
            broker_generation,
            kernel_peer,
            expected_runner_identity,
        )
        .map_err(ProvisioningError::from)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ProcessEvidence {
    peer: PeerIdentity,
    start_time: u64,
}

fn process_evidence(peer: PeerIdentity) -> Result<ProcessEvidence, ProvisioningError> {
    if peer.pid() <= 0 {
        return Err(ProvisioningError::Rejected);
    }
    Ok(ProcessEvidence {
        peer,
        start_time: read_process_start_time(peer.pid())?,
    })
}

fn read_process_start_time(pid: i32) -> Result<u64, ProvisioningError> {
    let file = fs::File::open(format!("/proc/{pid}/stat"))?;
    let mut bytes = Vec::new();
    file.take(PROC_STAT_MAX_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.is_empty() || bytes.len() as u64 > PROC_STAT_MAX_BYTES {
        return Err(ProvisioningError::Rejected);
    }
    parse_process_start_time(pid, &bytes).ok_or(ProvisioningError::Rejected)
}

fn parse_process_start_time(pid: i32, bytes: &[u8]) -> Option<u64> {
    let record = std::str::from_utf8(bytes).ok()?.trim_end_matches('\n');
    let prefix = format!("{pid} (");
    let fields = record.strip_prefix(&prefix)?;
    let delimiter = fields.rfind(") ")?;
    let suffix = &fields[delimiter + 2..];
    let fields = suffix.split_ascii_whitespace().collect::<Vec<_>>();
    if fields.len() < 20
        || fields[0].len() != 1
        || !fields[0].as_bytes()[0].is_ascii_alphabetic()
        || fields[1..19]
            .iter()
            .any(|field| field.parse::<i128>().is_err())
    {
        return None;
    }
    // Field 22 is the twentieth token after the command delimiter (field 3).
    let start_time = fields[19].parse::<u64>().ok()?;
    (start_time != 0).then_some(start_time)
}

fn fresh_nonce() -> Result<[u8; 16], ProvisioningError> {
    fresh_random_value().map_err(ProvisioningError::from)
}

fn fresh_random_value() -> io::Result<[u8; 16]> {
    for _ in 0..ENTROPY_ATTEMPTS {
        let mut nonce = [0_u8; 16];
        let filled = getrandom(&mut nonce, GetRandomFlags::empty()).map_err(io::Error::from)?;
        if filled == nonce.len() && nonce != [0; 16] {
            return Ok(nonce);
        }
    }
    Err(io::Error::other("bounded kernel entropy unavailable"))
}

fn connect_private_socket(
    path: &Path,
    socket_kind: SocketType,
    deadline: RequestDeadline,
) -> Result<OwnedFd, ProvisioningError> {
    let descriptor = socket_with(
        AddressFamily::UNIX,
        socket_kind,
        SocketFlags::CLOEXEC | SocketFlags::NONBLOCK,
        None,
    )
    .map_err(io::Error::from)?;
    let address = SocketAddrUnix::new(path).map_err(io::Error::from)?;
    match connect(&descriptor, &address) {
        Ok(()) => {}
        Err(error) if error == rustix::io::Errno::INPROGRESS => {
            let timeout = Timespec::try_from(deadline.remaining().map_err(EndpointError::from)?)
                .map_err(|_| ProvisioningError::Rejected)?;
            let mut fds = [PollFd::new(&descriptor, PollFlags::OUT)];
            if poll(&mut fds, Some(&timeout)).map_err(io::Error::from)? != 1
                || fds[0]
                    .revents()
                    .intersects(PollFlags::ERR | PollFlags::HUP | PollFlags::NVAL)
                || !fds[0].revents().contains(PollFlags::OUT)
            {
                return Err(ProvisioningError::Rejected);
            }
        }
        Err(error) => return Err(io::Error::from(error).into()),
    }
    if socket_error(&descriptor).map_err(io::Error::from)?.is_err() {
        return Err(ProvisioningError::Rejected);
    }
    let flags = fcntl_getfl(&descriptor).map_err(io::Error::from)?;
    fcntl_setfl(&descriptor, flags - OFlags::NONBLOCK).map_err(io::Error::from)?;
    set_deadline_timeouts(&descriptor, deadline)?;
    Ok(descriptor)
}

fn set_deadline_timeouts(
    descriptor: &OwnedFd,
    deadline: RequestDeadline,
) -> Result<(), ProvisioningError> {
    let remaining = socket_timeout_floor(deadline.remaining().map_err(EndpointError::from)?)?;
    set_socket_timeout(descriptor, Timeout::Recv, Some(remaining)).map_err(io::Error::from)?;
    set_socket_timeout(descriptor, Timeout::Send, Some(remaining)).map_err(io::Error::from)?;
    Ok(())
}

fn socket_timeout_floor(duration: Duration) -> io::Result<Duration> {
    let micros = u64::try_from(duration.as_micros())
        .map_err(|_| io::Error::other("socket timeout exceeds supported bound"))?;
    if micros == 0 {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "socket deadline exhausted",
        ));
    }
    Ok(Duration::from_micros(micros))
}

fn recv_provisioning_response(
    connection: &OwnedFd,
) -> Result<(ProvisioningPacket, OwnedFd), ProvisioningError> {
    let mut payload = [0_u8; PROVISIONING_PACKET_BYTES];
    let mut iov = [IoSliceMut::new(&mut payload)];
    let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(2), ScmCredentials(1))];
    let mut ancillary = RecvAncillaryBuffer::new(&mut space);
    let received = recvmsg(
        connection,
        &mut iov,
        &mut ancillary,
        RecvFlags::CMSG_CLOEXEC | RecvFlags::TRUNC,
    )
    .map_err(io::Error::from)?;
    let mut rights_messages = 0_usize;
    let mut descriptors = Vec::new();
    let mut other_message = false;
    for message in ancillary.drain() {
        match message {
            RecvAncillaryMessage::ScmRights(rights) => {
                rights_messages += 1;
                descriptors.extend(rights);
            }
            _ => other_message = true,
        }
    }
    let packet = ProvisioningPacket::decode(&payload)?;
    if received.bytes != PROVISIONING_PACKET_BYTES
        || received
            .flags
            .intersects(ReturnFlags::TRUNC | ReturnFlags::CTRUNC)
        || rights_messages != 1
        || descriptors.len() != 1
        || other_message
        || packet.status != HandoffStatus::Success
    {
        return Err(ProvisioningError::Rejected);
    }
    Ok((packet, descriptors.pop().expect("one descriptor checked")))
}

fn validate_anonymous_stream(
    descriptor: &OwnedFd,
    expected: ProcessEvidence,
    expected_uid: u32,
) -> Result<PeerIdentity, ProvisioningError> {
    if socket_domain(descriptor).map_err(io::Error::from)? != AddressFamily::UNIX
        || socket_type(descriptor).map_err(io::Error::from)? != SocketType::STREAM
        || socket_acceptconn(descriptor).map_err(io::Error::from)?
        || socket_error(descriptor).map_err(io::Error::from)?.is_err()
        || !rustix::io::fcntl_getfd(descriptor)
            .map_err(io::Error::from)?
            .contains(rustix::io::FdFlags::CLOEXEC)
    {
        return Err(ProvisioningError::Rejected);
    }
    let address_probe = UnixStream::from(descriptor.as_fd().try_clone_to_owned()?);
    let local = address_probe.local_addr()?;
    let peer_address = address_probe.peer_addr()?;
    if !local.is_unnamed()
        || local.as_pathname().is_some()
        || local.as_abstract_name().is_some()
        || !peer_address.is_unnamed()
        || peer_address.as_pathname().is_some()
        || peer_address.as_abstract_name().is_some()
    {
        return Err(ProvisioningError::Rejected);
    }
    let peer = PeerIdentity::from_fd(descriptor)
        .and_then(|identity| identity.require_owner(expected_uid))
        .map_err(|_| ProvisioningError::Rejected)?;
    if peer != expected.peer {
        return Err(ProvisioningError::Rejected);
    }
    let mut byte = [0_u8; 1];
    match recv(descriptor, &mut byte, RecvFlags::PEEK | RecvFlags::DONTWAIT) {
        Err(error) if error == rustix::io::Errno::AGAIN => {}
        _ => return Err(ProvisioningError::Rejected),
    }
    Ok(peer)
}

/// Exact 64-byte request or response on the private provisioning endpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProvisioningPacket {
    /// Request or closed reply status.
    pub status: HandoffStatus,
    /// Caller-generated nonzero request nonce.
    pub nonce: [u8; 16],
    /// Service runner identity digest; nonzero only on success.
    pub runner_identity: [u8; 32],
}

impl ProvisioningPacket {
    /// Encode the frozen v1 representation.
    #[must_use]
    pub fn encode(self) -> [u8; PROVISIONING_PACKET_BYTES] {
        let mut bytes = [0_u8; PROVISIONING_PACKET_BYTES];
        bytes[..8].copy_from_slice(&PROVISIONING_MAGIC);
        bytes[8..10].copy_from_slice(&WIRE_VERSION.to_be_bytes());
        bytes[10] = 1;
        bytes[11] = self.status.wire();
        bytes[16..32].copy_from_slice(&self.nonce);
        bytes[32..].copy_from_slice(&self.runner_identity);
        bytes
    }

    /// Decode the exact frozen v1 representation and reject reserved bits.
    pub fn decode(bytes: &[u8]) -> Result<Self, HandoffError> {
        if bytes.len() != PROVISIONING_PACKET_BYTES
            || bytes[..8] != PROVISIONING_MAGIC
            || bytes[8..10] != WIRE_VERSION.to_be_bytes()
            || bytes[10] != 1
            || bytes[12..16] != [0; 4]
        {
            return Err(HandoffError::InvalidPacket);
        }
        let mut nonce = [0_u8; 16];
        nonce.copy_from_slice(&bytes[16..32]);
        let mut identity = [0_u8; 32];
        identity.copy_from_slice(&bytes[32..64]);
        let packet = Self {
            status: HandoffStatus::parse_provisioning(bytes[11])?,
            nonce,
            runner_identity: identity,
        };
        let valid = packet.nonce != [0; 16]
            && match packet.status {
                HandoffStatus::Request => packet.runner_identity == [0; 32],
                HandoffStatus::Success => packet.runner_identity != [0; 32],
                HandoffStatus::Unavailable | HandoffStatus::Rejected => {
                    packet.runner_identity == [0; 32]
                }
                HandoffStatus::Exhausted => false,
            };
        if valid {
            Ok(packet)
        } else {
            Err(HandoffError::InvalidPacket)
        }
    }
}

/// Compute the frozen, domain-separated runner instance identity.
pub fn runner_identity_digest(runner_instance_id: &str) -> Result<[u8; 32], HandoffError> {
    validate_identity(runner_instance_id).map_err(|_| HandoffError::InvalidRunnerIdentity)?;
    if runner_instance_id.len() > MAX_CONTROL_ID_BYTES {
        return Err(HandoffError::InvalidRunnerIdentity);
    }
    let length =
        u16::try_from(runner_instance_id.len()).map_err(|_| HandoffError::InvalidRunnerIdentity)?;
    let mut digest = Sha256::new();
    digest.update(RUNNER_IDENTITY_DOMAIN);
    digest.update([0]);
    digest.update(length.to_be_bytes());
    digest.update(runner_instance_id.as_bytes());
    Ok(digest.finalize().into())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BrokerPhase {
    New,
    Pending {
        operation: BrokerOperation,
        response_sequence: u64,
        deadline: RequestDeadline,
    },
    Established(BrokerGeneration),
    Terminal,
}

/// Opaque success generation prepared without advancing broker state.
#[derive(Debug, Eq, PartialEq)]
pub struct PendingBrokerSuccess {
    operation: BrokerOperation,
    generation: BrokerGeneration,
    deadline: RequestDeadline,
}

impl PendingBrokerSuccess {
    /// Generation that the ASB producer must bind to its authenticated stream.
    #[must_use]
    pub const fn generation(&self) -> BrokerGeneration {
        self.generation
    }

    const fn packet(&self, expected_runner_identity: [u8; 32]) -> BrokerPacket {
        BrokerPacket {
            operation: self.operation,
            status: HandoffStatus::Success,
            epoch: self.generation.epoch,
            sequence: self.generation.sequence,
            expected_runner_identity,
        }
    }
}

/// One authenticated broker request carrying its sole diminishing acquisition budget.
pub struct BrokerRequest {
    packet: BrokerPacket,
    deadline: RequestDeadline,
}

impl BrokerRequest {
    /// Exact validated packet received from the frontend.
    #[must_use]
    pub const fn packet(&self) -> BrokerPacket {
        self.packet
    }
}

/// Validated router endpoint for one private frontend broker lifetime.
pub struct BrokerConnection {
    socket: OwnedFd,
}

impl BrokerConnection {
    /// Create a private close-on-exec broker pair and retain the router endpoint.
    pub fn pair() -> Result<(Self, OwnedFd), HandoffError> {
        let (router, frontend) = socketpair(
            AddressFamily::UNIX,
            SocketType::SEQPACKET,
            SocketFlags::CLOEXEC,
            None,
        )
        .map_err(|_| HandoffError::TransferFailed)?;
        Ok((Self::new(router)?, frontend))
    }

    /// Adopt and validate an already-created router endpoint.
    pub fn new(socket: OwnedFd) -> Result<Self, HandoffError> {
        validate_broker_socket(&socket)?;
        Ok(Self { socket })
    }

    /// Receive exactly one descriptor-free, shape-valid request and start its sole budget.
    pub fn receive_request(&self) -> Result<BrokerRequest, HandoffError> {
        self.receive_request_with_budget(BROKER_ACQUISITION_TIMEOUT)
    }

    fn receive_request_with_budget(&self, budget: Duration) -> Result<BrokerRequest, HandoffError> {
        self.wait_for_request()?;
        let timeout_ms =
            u64::try_from(budget.as_millis()).map_err(|_| HandoffError::TransferFailed)?;
        let deadline =
            RequestDeadline::start(timeout_ms).map_err(|_| HandoffError::TransferFailed)?;
        self.receive_request_until(deadline)
    }

    fn wait_for_request(&self) -> Result<(), HandoffError> {
        loop {
            let mut descriptors = [PollFd::new(&self.socket, PollFlags::IN)];
            match poll(&mut descriptors, None) {
                Ok(1) => {
                    let events = descriptors[0].revents();
                    if events.intersects(PollFlags::ERR | PollFlags::HUP | PollFlags::NVAL)
                        || !events.contains(PollFlags::IN)
                    {
                        return Err(HandoffError::TransferFailed);
                    }
                    return Ok(());
                }
                Ok(_) => continue,
                Err(error) if error == rustix::io::Errno::INTR => continue,
                Err(_) => return Err(HandoffError::TransferFailed),
            }
        }
    }

    fn receive_request_until(
        &self,
        deadline: RequestDeadline,
    ) -> Result<BrokerRequest, HandoffError> {
        let remaining = deadline
            .remaining()
            .map_err(|_| HandoffError::TransferFailed)?;
        let timeout = socket_timeout_floor(remaining).map_err(|_| HandoffError::TransferFailed)?;
        set_socket_timeout(&self.socket, Timeout::Recv, Some(timeout))
            .map_err(|_| HandoffError::TransferFailed)?;
        let mut payload = [0_u8; BROKER_PACKET_BYTES];
        let mut iov = [IoSliceMut::new(&mut payload)];
        let mut space =
            [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(2), ScmCredentials(1))];
        let mut ancillary = RecvAncillaryBuffer::new(&mut space);
        let received = recvmsg(
            &self.socket,
            &mut iov,
            &mut ancillary,
            RecvFlags::CMSG_CLOEXEC | RecvFlags::TRUNC,
        )
        .map_err(|_| HandoffError::TransferFailed)?;
        let mut has_ancillary = false;
        for message in ancillary.drain() {
            has_ancillary = true;
            if let RecvAncillaryMessage::ScmRights(rights) = message {
                for descriptor in rights {
                    drop(descriptor);
                }
            }
        }
        if received.bytes != BROKER_PACKET_BYTES
            || received
                .flags
                .intersects(ReturnFlags::TRUNC | ReturnFlags::CTRUNC)
            || has_ancillary
        {
            return Err(HandoffError::InvalidPacket);
        }
        let packet = BrokerPacket::decode(&payload)?;
        if packet.status != HandoffStatus::Request {
            return Err(HandoffError::InvalidPacket);
        }
        deadline.check().map_err(|_| HandoffError::TransferFailed)?;
        Ok(BrokerRequest { packet, deadline })
    }

    fn send_success(
        &self,
        packet: BrokerPacket,
        stream: &UnixStream,
        deadline: RequestDeadline,
    ) -> Result<(), HandoffError> {
        self.prepare_send(deadline)?;
        let payload = packet.encode();
        let iov = [IoSlice::new(&payload)];
        let descriptors = [stream.as_fd()];
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
        let mut ancillary = SendAncillaryBuffer::new(&mut space);
        if !ancillary.push(SendAncillaryMessage::ScmRights(&descriptors)) {
            return Err(HandoffError::TransferFailed);
        }
        let sent = sendmsg(&self.socket, &iov, &mut ancillary, SendFlags::NOSIGNAL)
            .map_err(|_| HandoffError::TransferFailed)?;
        (sent == BROKER_PACKET_BYTES)
            .then_some(())
            .ok_or(HandoffError::TransferFailed)
    }

    fn send_failure(
        &self,
        packet: BrokerPacket,
        deadline: RequestDeadline,
    ) -> Result<(), HandoffError> {
        self.prepare_send(deadline)?;
        let payload = packet.encode();
        let sent = rustix::net::send(&self.socket, &payload, SendFlags::NOSIGNAL)
            .map_err(|_| HandoffError::TransferFailed)?;
        (sent == BROKER_PACKET_BYTES)
            .then_some(())
            .ok_or(HandoffError::TransferFailed)
    }

    fn prepare_send(&self, deadline: RequestDeadline) -> Result<(), HandoffError> {
        let remaining = deadline
            .remaining()
            .map_err(|_| HandoffError::TransferFailed)?;
        let timeout = socket_timeout_floor(remaining).map_err(|_| HandoffError::TransferFailed)?;
        set_socket_timeout(&self.socket, Timeout::Send, Some(timeout))
            .map_err(|_| HandoffError::TransferFailed)
    }
}

fn validate_broker_socket(socket: &OwnedFd) -> Result<(), HandoffError> {
    if socket_domain(socket).ok() != Some(AddressFamily::UNIX)
        || socket_type(socket).ok() != Some(SocketType::SEQPACKET)
        || socket_acceptconn(socket).unwrap_or(true)
        || !matches!(socket_error(socket), Ok(Ok(())))
        || socket_passcred(socket).unwrap_or(true)
        || getpeername(socket).ok().flatten().is_none()
        || !rustix::io::fcntl_getfd(socket)
            .is_ok_and(|flags| flags.contains(rustix::io::FdFlags::CLOEXEC))
    {
        return Err(HandoffError::TransferFailed);
    }
    let peer = PeerIdentity::from_fd(socket)
        .and_then(|identity| identity.require_owner(geteuid().as_raw()))
        .map_err(|_| HandoffError::TransferFailed)?;
    (peer.pid() > 0)
        .then_some(())
        .ok_or(HandoffError::TransferFailed)
}

/// Fail-closed producer state for one private router/frontend broker.
pub struct BrokerState {
    epoch: [u8; 16],
    phase: BrokerPhase,
    committed_generation: Option<BrokerGeneration>,
    replacement_attempts: u8,
    last_replacement: Option<Duration>,
}

impl BrokerState {
    /// Create state for one frontend lifetime from bounded kernel randomness.
    pub fn fresh() -> Result<Self, HandoffError> {
        let epoch = fresh_random_value().map_err(|_| HandoffError::EntropyUnavailable)?;
        Self::new(epoch)
    }

    fn new(epoch: [u8; 16]) -> Result<Self, HandoffError> {
        if epoch == [0; 16] {
            return Err(HandoffError::InvalidEpoch);
        }
        Ok(Self {
            epoch,
            phase: BrokerPhase::New,
            committed_generation: None,
            replacement_attempts: 0,
            last_replacement: None,
        })
    }

    /// Admit exactly one ordered request and reserve its response generation.
    pub fn begin_request(
        &mut self,
        request: BrokerRequest,
        now: Duration,
    ) -> Result<PendingBrokerSuccess, HandoffError> {
        if request.deadline.check().is_err() {
            return self.terminate(HandoffError::TransferFailed);
        }
        self.begin_packet(request.packet, request.deadline, now)
    }

    fn begin_packet(
        &mut self,
        packet: BrokerPacket,
        deadline: RequestDeadline,
        now: Duration,
    ) -> Result<PendingBrokerSuccess, HandoffError> {
        if packet.status != HandoffStatus::Request {
            return self.terminate(HandoffError::UnexpectedState);
        }
        match (self.phase, packet.operation) {
            (BrokerPhase::New, BrokerOperation::Initial) => {
                if packet.validate_shape().is_err() {
                    return self.terminate(HandoffError::InvalidPacket);
                }
                self.phase = BrokerPhase::Pending {
                    operation: BrokerOperation::Initial,
                    response_sequence: 1,
                    deadline,
                };
            }
            (BrokerPhase::Established(current), BrokerOperation::Replacement) => {
                if packet.validate_shape().is_err() {
                    return self.terminate(HandoffError::InvalidPacket);
                }
                if packet.epoch != current.epoch || packet.sequence != current.sequence {
                    return self.terminate(HandoffError::GenerationMismatch);
                }
                if self.replacement_attempts >= MAX_REPLACEMENTS {
                    return self.terminate(HandoffError::ReplacementExhausted);
                }
                if self.last_replacement.is_some_and(|last| {
                    now.checked_sub(last)
                        .is_none_or(|elapsed| elapsed < MIN_REPLACEMENT_INTERVAL)
                }) {
                    return self.terminate(HandoffError::RateLimited);
                }
                let Some(response_sequence) = current.sequence.checked_add(1) else {
                    return self.terminate(HandoffError::SequenceExhausted);
                };
                self.replacement_attempts += 1;
                self.last_replacement = Some(now);
                self.phase = BrokerPhase::Pending {
                    operation: BrokerOperation::Replacement,
                    response_sequence,
                    deadline,
                };
            }
            _ => return self.terminate(HandoffError::UnexpectedState),
        }
        let BrokerPhase::Pending {
            operation,
            response_sequence,
            deadline,
        } = self.phase
        else {
            unreachable!();
        };
        Ok(PendingBrokerSuccess {
            operation,
            generation: BrokerGeneration {
                epoch: self.epoch,
                sequence: response_sequence,
            },
            deadline,
        })
    }

    /// Atomically transfer the prepared packet and stream, then advance state.
    ///
    /// Any validation or send failure consumes the generation, terminates this
    /// broker lifetime and leaves `committed_generation` unchanged.
    pub fn commit_success(
        &mut self,
        pending: PendingBrokerSuccess,
        broker: &BrokerConnection,
        authenticated: AuthenticatedGeneration,
    ) -> Result<BrokerPacket, HandoffError> {
        let BrokerPhase::Pending {
            operation,
            response_sequence,
            deadline,
        } = self.phase
        else {
            return self.terminate(HandoffError::UnexpectedState);
        };
        if pending.operation != operation
            || pending.generation.epoch != self.epoch
            || pending.generation.sequence != response_sequence
            || authenticated.broker_generation != pending.generation
            || pending.deadline != deadline
        {
            return self.terminate(HandoffError::GenerationMismatch);
        }
        let packet = pending.packet(authenticated.expected_runner_identity);
        if broker
            .send_success(packet, &authenticated.stream, pending.deadline)
            .is_err()
        {
            return self.terminate(HandoffError::TransferFailed);
        }
        self.phase = BrokerPhase::Established(pending.generation);
        self.committed_generation = Some(pending.generation);
        Ok(packet)
    }

    /// Last generation whose complete packet and descriptor transfer succeeded.
    #[must_use]
    pub const fn committed_generation(&self) -> Option<BrokerGeneration> {
        self.committed_generation
    }

    /// Complete a descriptor-free closed failure without advancing a generation.
    pub fn commit_failure(
        &mut self,
        pending: PendingBrokerSuccess,
        status: HandoffStatus,
        broker: &BrokerConnection,
    ) -> Result<BrokerPacket, HandoffError> {
        if !status.is_closed_failure() {
            return self.terminate(HandoffError::UnexpectedState);
        }
        let BrokerPhase::Pending {
            operation,
            response_sequence,
            deadline,
        } = self.phase
        else {
            return self.terminate(HandoffError::UnexpectedState);
        };
        if pending.operation != operation
            || pending.generation.epoch != self.epoch
            || pending.generation.sequence != response_sequence
            || pending.deadline != deadline
        {
            return self.terminate(HandoffError::GenerationMismatch);
        }
        let generation = match operation {
            BrokerOperation::Initial => BrokerGeneration {
                epoch: [0; 16],
                sequence: 0,
            },
            BrokerOperation::Replacement => BrokerGeneration {
                epoch: self.epoch,
                sequence: self.pending_previous_sequence(),
            },
        };
        let packet = BrokerPacket {
            operation,
            status,
            epoch: generation.epoch,
            sequence: generation.sequence,
            expected_runner_identity: [0; 32],
        };
        if broker.send_failure(packet, pending.deadline).is_err() {
            return self.terminate(HandoffError::TransferFailed);
        }
        self.phase = match operation {
            BrokerOperation::Initial => BrokerPhase::Terminal,
            BrokerOperation::Replacement => BrokerPhase::Established(generation),
        };
        Ok(packet)
    }

    fn pending_previous_sequence(&self) -> u64 {
        match self.phase {
            BrokerPhase::Pending {
                response_sequence, ..
            } => response_sequence - 1,
            _ => 0,
        }
    }

    fn terminate<T>(&mut self, error: HandoffError) -> Result<T, HandoffError> {
        self.phase = BrokerPhase::Terminal;
        Err(error)
    }
}

/// Authenticated anonymous control stream produced by ASB.
pub struct AuthenticatedGeneration {
    stream: UnixStream,
    broker_generation: BrokerGeneration,
    kernel_peer: PeerIdentity,
    expected_runner_identity: [u8; 32],
}

impl AuthenticatedGeneration {
    /// Create a generation only from kernel-authenticated ASB-side evidence.
    pub(crate) fn new(
        stream: UnixStream,
        broker_generation: BrokerGeneration,
        kernel_peer: PeerIdentity,
        expected_runner_identity: [u8; 32],
    ) -> Result<Self, HandoffError> {
        if broker_generation.epoch == [0; 16]
            || broker_generation.sequence == 0
            || expected_runner_identity == [0; 32]
        {
            return Err(HandoffError::UnexpectedState);
        }
        Ok(Self {
            stream,
            broker_generation,
            kernel_peer,
            expected_runner_identity,
        })
    }

    /// Accepted broker generation.
    #[must_use]
    pub const fn broker_generation(&self) -> BrokerGeneration {
        self.broker_generation
    }

    /// Short-lived kernel peer evidence captured during acquisition.
    #[must_use]
    pub const fn kernel_peer(&self) -> PeerIdentity {
        self.kernel_peer
    }

    /// Independently recomputable runner identity digest.
    #[must_use]
    pub const fn expected_runner_identity(&self) -> [u8; 32] {
        self.expected_runner_identity
    }

    /// Consume the evidence wrapper and return the anonymous control stream.
    #[must_use]
    pub fn into_stream(self) -> UnixStream {
        self.stream
    }
}

/// Closed handoff failure without endpoint, credential, or descriptor details.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum HandoffError {
    /// Exact packet shape or frozen field is invalid.
    #[error("invalid handoff packet")]
    InvalidPacket,
    /// Runner identity is not a bounded protocol identity.
    #[error("invalid runner identity")]
    InvalidRunnerIdentity,
    /// Kernel-random broker epoch is invalid.
    #[error("invalid broker epoch")]
    InvalidEpoch,
    /// Packet is stale, reordered, or from another broker lifetime.
    #[error("handoff generation mismatch")]
    GenerationMismatch,
    /// State transition is not legal for this broker lifetime.
    #[error("unexpected handoff state")]
    UnexpectedState,
    /// Replacement ceiling was reached.
    #[error("handoff replacement limit exhausted")]
    ReplacementExhausted,
    /// Replacement arrived before the fixed minimum interval.
    #[error("handoff replacement rate exceeded")]
    RateLimited,
    /// Generation sequence cannot advance without wrapping.
    #[error("handoff sequence exhausted")]
    SequenceExhausted,
    /// The packet and descriptor were not completely transferred.
    #[error("control handoff transfer failed")]
    TransferFailed,
    /// Bounded kernel entropy could not produce a fresh nonzero epoch.
    #[error("control handoff entropy unavailable")]
    EntropyUnavailable,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SessionError;
    use std::collections::BTreeSet;
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    use rustix::io::{FdFlags, fcntl_getfd};
    use rustix::net::{RecvAncillaryMessage, connect, send};

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TestRoot(PathBuf);

    impl TestRoot {
        fn new() -> Self {
            let path = std::env::var_os("ASB_TEST_ROOT")
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir)
                .join(format!(
                    "asb-handoff-{}-{}",
                    std::process::id(),
                    TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
                ));
            fs::create_dir_all(&path).unwrap();
            fs::set_permissions(&path, Permissions::from_mode(0o700)).unwrap();
            Self(path)
        }

        fn endpoint_dirs(&self) -> (PathBuf, PathBuf) {
            let control = self.0.join("control");
            let provisioning = self.0.join("provisioning");
            fs::create_dir(&control).unwrap();
            fs::create_dir(&provisioning).unwrap();
            fs::set_permissions(&control, Permissions::from_mode(0o700)).unwrap();
            fs::set_permissions(&provisioning, Permissions::from_mode(0o700)).unwrap();
            (control, provisioning)
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    struct HandoffBackend;

    impl ControlBackend for HandoffBackend {
        fn runner_instance_id(&self) -> &str {
            "runner-handoff-test"
        }

        fn oldest_revision(&self) -> crate::Revision {
            crate::Revision(0)
        }

        fn latest_revision(&self) -> crate::Revision {
            crate::Revision(0)
        }

        fn execute(
            &self,
            call: &crate::ControlCall,
            _deadline: crate::RequestDeadline,
        ) -> Result<crate::BoundControlResult, crate::BackendFailure> {
            match call {
                crate::ControlCall::Capabilities => Ok(crate::BoundControlResult::new(
                    call,
                    crate::ControlResult::Capabilities(crate::Capabilities {
                        validate_settings: true,
                        run_control: true,
                        repeat: true,
                        analysis: true,
                        events: true,
                    }),
                )
                .unwrap()),
                _ => Err(crate::BackendFailure::Rejected),
            }
        }
    }

    fn digest_hex(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    fn bytes_hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn fixture_bytes<const N: usize>(fixture: &str) -> [u8; N] {
        let fixture = fixture.trim();
        assert_eq!(fixture.len(), N * 2);
        std::array::from_fn(|index| {
            u8::from_str_radix(&fixture[index * 2..index * 2 + 2], 16).unwrap()
        })
    }

    fn fixture_vec(fixture: &str) -> Vec<u8> {
        let fixture = fixture.trim();
        assert_eq!(fixture.len() % 2, 0);
        (0..fixture.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&fixture[index..index + 2], 16).unwrap())
            .collect()
    }

    fn initial_request() -> BrokerPacket {
        BrokerPacket {
            operation: BrokerOperation::Initial,
            status: HandoffStatus::Request,
            epoch: [0; 16],
            sequence: 0,
            expected_runner_identity: [0; 32],
        }
    }

    fn test_broker_request(packet: BrokerPacket) -> BrokerRequest {
        BrokerRequest {
            packet,
            deadline: RequestDeadline::start(2_000).unwrap(),
        }
    }

    fn provisioning_request() -> ProvisioningPacket {
        ProvisioningPacket {
            status: HandoffStatus::Request,
            nonce: std::array::from_fn(|index| u8::try_from(index + 1).unwrap()),
            runner_identity: [0; 32],
        }
    }

    fn connect_seqpacket(path: &Path) -> OwnedFd {
        let socket = socket_with(
            AddressFamily::UNIX,
            SocketType::SEQPACKET,
            SocketFlags::CLOEXEC,
            None,
        )
        .unwrap();
        connect(&socket, &SocketAddrUnix::new(path).unwrap()).unwrap();
        socket
    }

    fn receive_generation(connection: &OwnedFd) -> (ProvisioningPacket, OwnedFd) {
        let mut payload = [0_u8; PROVISIONING_PACKET_BYTES];
        let mut iov = [IoSliceMut::new(&mut payload)];
        let mut space =
            [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(2), ScmCredentials(1))];
        let mut ancillary = RecvAncillaryBuffer::new(&mut space);
        let received = recvmsg(
            connection,
            &mut iov,
            &mut ancillary,
            RecvFlags::CMSG_CLOEXEC | RecvFlags::TRUNC,
        )
        .unwrap();
        assert_eq!(received.bytes, PROVISIONING_PACKET_BYTES);
        assert!(
            !received
                .flags
                .intersects(ReturnFlags::TRUNC | ReturnFlags::CTRUNC)
        );
        let mut descriptors = Vec::new();
        for message in ancillary.drain() {
            match message {
                RecvAncillaryMessage::ScmRights(rights) => descriptors.extend(rights),
                _ => panic!("unexpected ancillary message"),
            }
        }
        assert_eq!(descriptors.len(), 1);
        let descriptor = descriptors.pop().unwrap();
        assert!(fcntl_getfd(&descriptor).unwrap().contains(FdFlags::CLOEXEC));
        (ProvisioningPacket::decode(&payload).unwrap(), descriptor)
    }

    fn receive_broker_generation(connection: &OwnedFd) -> (BrokerPacket, OwnedFd) {
        let mut payload = [0_u8; BROKER_PACKET_BYTES];
        let mut iov = [IoSliceMut::new(&mut payload)];
        let mut space =
            [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(2), ScmCredentials(1))];
        let mut ancillary = RecvAncillaryBuffer::new(&mut space);
        let received = recvmsg(
            connection,
            &mut iov,
            &mut ancillary,
            RecvFlags::CMSG_CLOEXEC | RecvFlags::TRUNC,
        )
        .unwrap();
        assert_eq!(received.bytes, BROKER_PACKET_BYTES);
        assert!(
            !received
                .flags
                .intersects(ReturnFlags::TRUNC | ReturnFlags::CTRUNC)
        );
        let mut descriptors = Vec::new();
        for message in ancillary.drain() {
            match message {
                RecvAncillaryMessage::ScmRights(rights) => descriptors.extend(rights),
                _ => panic!("unexpected ancillary message"),
            }
        }
        assert_eq!(descriptors.len(), 1);
        let descriptor = descriptors.pop().unwrap();
        assert!(fcntl_getfd(&descriptor).unwrap().contains(FdFlags::CLOEXEC));
        (BrokerPacket::decode(&payload).unwrap(), descriptor)
    }

    fn receive_broker_failure(connection: &OwnedFd) -> BrokerPacket {
        let mut payload = [0_u8; BROKER_PACKET_BYTES];
        let mut iov = [IoSliceMut::new(&mut payload)];
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
        let mut ancillary = RecvAncillaryBuffer::new(&mut space);
        let received = recvmsg(
            connection,
            &mut iov,
            &mut ancillary,
            RecvFlags::CMSG_CLOEXEC | RecvFlags::TRUNC,
        )
        .unwrap();
        assert_eq!(received.bytes, BROKER_PACKET_BYTES);
        assert!(!received.flags.intersects(ReturnFlags::TRUNC));
        assert!(ancillary.drain().next().is_none());
        BrokerPacket::decode(&payload).unwrap()
    }

    fn authenticated_generation(
        generation: BrokerGeneration,
        identity: [u8; 32],
    ) -> (AuthenticatedGeneration, UnixStream) {
        let (stream, service) = UnixStream::pair().unwrap();
        let peer = PeerIdentity::from_fd(&stream).unwrap();
        (
            AuthenticatedGeneration::new(stream, generation, peer, identity).unwrap(),
            service,
        )
    }

    fn commit_broker_success(
        broker: &mut BrokerState,
        pending: PendingBrokerSuccess,
        identity: [u8; 32],
    ) -> BrokerPacket {
        let (authenticated, service) = authenticated_generation(pending.generation(), identity);
        let (connection, frontend) = BrokerConnection::pair().unwrap();
        let packet = broker
            .commit_success(pending, &connection, authenticated)
            .unwrap();
        let (received, descriptor) = receive_broker_generation(&frontend);
        assert_eq!(received, packet);
        assert_eq!(
            PeerIdentity::from_fd(&descriptor).unwrap(),
            PeerIdentity::from_fd(&service).unwrap()
        );
        packet
    }

    #[test]
    fn frozen_packets_match_canonical_hashes() {
        let initial = initial_request().encode();
        assert_eq!(
            initial,
            fixture_bytes(include_str!("../fixtures/handoff-v1/broker-initial.hex"))
        );
        assert_eq!(
            digest_hex(&initial),
            "e1dc9317afbc4bce748f0ce65424ad185c8a29e5ab40eb3e724d8930e77f216a"
        );
        assert_eq!(BrokerPacket::decode(&initial).unwrap(), initial_request());

        let success = BrokerPacket {
            operation: BrokerOperation::Initial,
            status: HandoffStatus::Success,
            epoch: std::array::from_fn(|index| u8::try_from(index).unwrap()),
            sequence: 1,
            expected_runner_identity: std::array::from_fn(|index| {
                0xa0 + u8::try_from(index).unwrap()
            }),
        };
        assert_eq!(
            success.encode(),
            fixture_bytes(include_str!("../fixtures/handoff-v1/broker-success.hex"))
        );
        assert_eq!(
            digest_hex(&success.encode()),
            "0936d182cde09230bcb3fd3f40c9f0aaa886e23fdb42ac68cfb1e098b8cf98f5"
        );

        let request = ProvisioningPacket {
            status: HandoffStatus::Request,
            nonce: std::array::from_fn(|index| u8::try_from(index).unwrap()),
            runner_identity: [0; 32],
        };
        assert_eq!(
            request.encode(),
            fixture_bytes(include_str!("../fixtures/handoff-v1/provision-request.hex"))
        );
        assert_eq!(
            digest_hex(&request.encode()),
            "b3d2dc48c3c4afffbf2be9081974d72be4ed4ec3f737c51ad07af4aeab68ef4d"
        );
        let response = ProvisioningPacket {
            status: HandoffStatus::Success,
            nonce: request.nonce,
            runner_identity: std::array::from_fn(|index| 0xa0 + u8::try_from(index).unwrap()),
        };
        assert_eq!(
            response.encode(),
            fixture_bytes(include_str!("../fixtures/handoff-v1/provision-success.hex"))
        );
        assert_eq!(
            digest_hex(&response.encode()),
            "72c2bd598a591bf033db321e9e2acccea451a69ccabfc27498a9d8cdb2c032e5"
        );
        assert_eq!(
            ProvisioningPacket::decode(&request.encode()).unwrap(),
            request
        );
        assert_eq!(
            ProvisioningPacket::decode(&response.encode()).unwrap(),
            response
        );
    }

    #[test]
    fn runner_identity_digest_matches_frozen_golden() {
        let manifest: serde_json::Value =
            serde_json::from_str(include_str!("../fixtures/handoff-v1/manifest.json")).unwrap();
        assert_eq!(manifest["schema_version"], 1);
        assert_eq!(manifest["wire_version"], u64::from(WIRE_VERSION));
        assert_eq!(manifest["encoding"], "lowercase_hex");
        assert_eq!(manifest["fixtures"].as_array().unwrap().len(), 4);
        assert_eq!(
            manifest["runner_identity"]["domain"],
            std::str::from_utf8(RUNNER_IDENTITY_DOMAIN).unwrap()
        );
        assert_eq!(manifest["runner_identity"]["input"], "runner-handoff-test");
        assert_eq!(
            bytes_hex(&runner_identity_digest("runner-handoff-test").unwrap()),
            manifest["runner_identity"]["sha256"].as_str().unwrap()
        );
    }

    #[test]
    fn fixture_manifest_binds_every_canonical_vector() {
        let manifest: serde_json::Value =
            serde_json::from_str(include_str!("../fixtures/handoff-v1/manifest.json")).unwrap();
        let entries = manifest["fixtures"].as_array().unwrap();
        let fixtures = [
            (
                "broker-initial.hex",
                include_str!("../fixtures/handoff-v1/broker-initial.hex"),
            ),
            (
                "broker-success.hex",
                include_str!("../fixtures/handoff-v1/broker-success.hex"),
            ),
            (
                "provision-request.hex",
                include_str!("../fixtures/handoff-v1/provision-request.hex"),
            ),
            (
                "provision-success.hex",
                include_str!("../fixtures/handoff-v1/provision-success.hex"),
            ),
        ];
        assert_eq!(entries.len(), fixtures.len());
        for (entry, (name, fixture)) in entries.iter().zip(fixtures) {
            let bytes = fixture_vec(fixture);
            assert_eq!(entry["name"], name);
            assert_eq!(entry["bytes"], bytes.len());
            assert_eq!(entry["sha256"], digest_hex(&bytes));
        }
    }

    #[test]
    fn broker_advances_only_after_descriptor_transfer_and_preserves_failed_generation() {
        let epoch = [7; 16];
        let identity = [9; 32];
        let mut broker = BrokerState::new(epoch).unwrap();
        let initial = test_broker_request(initial_request());
        let pending = broker.begin_request(initial, Duration::ZERO).unwrap();
        assert_eq!(pending.generation().sequence(), 1);
        assert_eq!(broker.committed_generation(), None);
        let first = commit_broker_success(&mut broker, pending, identity);
        assert_eq!((first.epoch, first.sequence), (epoch, 1));
        assert_eq!(broker.committed_generation(), first.generation().ok());

        let replacement = BrokerPacket {
            operation: BrokerOperation::Replacement,
            status: HandoffStatus::Request,
            epoch,
            sequence: 1,
            expected_runner_identity: [0; 32],
        };
        let pending = broker
            .begin_request(test_broker_request(replacement), MIN_REPLACEMENT_INTERVAL)
            .unwrap();
        let (connection, frontend) = BrokerConnection::pair().unwrap();
        let failed = broker
            .commit_failure(pending, HandoffStatus::Unavailable, &connection)
            .unwrap();
        assert_eq!(receive_broker_failure(&frontend), failed);
        assert_eq!((failed.epoch, failed.sequence), (epoch, 1));
        let pending = broker
            .begin_request(
                test_broker_request(replacement),
                MIN_REPLACEMENT_INTERVAL * 2,
            )
            .unwrap();
        let second = commit_broker_success(&mut broker, pending, identity);
        assert_eq!((second.epoch, second.sequence), (epoch, 2));
        assert_eq!(broker.committed_generation(), second.generation().ok());
    }

    #[test]
    fn failed_descriptor_transfer_never_commits_pending_generation() {
        let identity = [9; 32];
        let mut broker = BrokerState::new([7; 16]).unwrap();
        let initial = test_broker_request(initial_request());
        let pending = broker.begin_request(initial, Duration::ZERO).unwrap();
        let first = commit_broker_success(&mut broker, pending, identity);
        let committed = first.generation().unwrap();
        let replacement = BrokerPacket {
            operation: BrokerOperation::Replacement,
            status: HandoffStatus::Request,
            epoch: committed.epoch(),
            sequence: committed.sequence(),
            expected_runner_identity: [0; 32],
        };
        let pending = broker
            .begin_request(test_broker_request(replacement), MIN_REPLACEMENT_INTERVAL)
            .unwrap();
        assert_eq!(pending.generation().sequence(), 2);
        let (authenticated, _service) = authenticated_generation(pending.generation(), identity);
        let (router, frontend) = socketpair(
            AddressFamily::UNIX,
            SocketType::SEQPACKET,
            SocketFlags::CLOEXEC,
            None,
        )
        .unwrap();
        let connection = BrokerConnection::new(router).unwrap();
        drop(frontend);
        assert_eq!(
            broker.commit_success(pending, &connection, authenticated),
            Err(HandoffError::TransferFailed)
        );
        assert_eq!(broker.committed_generation(), Some(committed));
        assert_eq!(
            broker.begin_request(
                test_broker_request(replacement),
                MIN_REPLACEMENT_INTERVAL * 2
            ),
            Err(HandoffError::UnexpectedState)
        );
    }

    #[test]
    fn typed_broker_connection_receives_only_exact_descriptor_free_requests() {
        let (stream, _peer) = UnixStream::pair().unwrap();
        assert!(matches!(
            BrokerConnection::new(stream.into()),
            Err(HandoffError::TransferFailed)
        ));

        let (connection, frontend) = BrokerConnection::pair().unwrap();
        assert_eq!(
            send(&frontend, &initial_request().encode(), SendFlags::NOSIGNAL).unwrap(),
            BROKER_PACKET_BYTES
        );
        assert_eq!(
            connection.receive_request().unwrap().packet(),
            initial_request()
        );

        let (connection, frontend) = BrokerConnection::pair().unwrap();
        let (extra, _peer) = UnixStream::pair().unwrap();
        let payload = initial_request().encode();
        let iov = [IoSlice::new(&payload)];
        let descriptors = [extra.as_fd()];
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
        let mut ancillary = SendAncillaryBuffer::new(&mut space);
        assert!(ancillary.push(SendAncillaryMessage::ScmRights(&descriptors)));
        assert_eq!(
            sendmsg(&frontend, &iov, &mut ancillary, SendFlags::NOSIGNAL).unwrap(),
            BROKER_PACKET_BYTES
        );
        assert_eq!(
            connection.receive_request().map(|request| request.packet()),
            Err(HandoffError::InvalidPacket)
        );

        let (connection, frontend) = BrokerConnection::pair().unwrap();
        let oversized = [0_u8; BROKER_PACKET_BYTES + 1];
        assert_eq!(
            send(&frontend, &oversized, SendFlags::NOSIGNAL).unwrap(),
            oversized.len()
        );
        assert_eq!(
            connection.receive_request().map(|request| request.packet()),
            Err(HandoffError::InvalidPacket)
        );
    }

    #[test]
    fn idle_wait_does_not_spend_the_acquisition_budget() {
        let (connection, frontend) = BrokerConnection::pair().unwrap();
        let receiver = thread::spawn(move || {
            connection.receive_request_with_budget(Duration::from_millis(25))
        });

        thread::sleep(Duration::from_millis(40));
        assert_eq!(
            send(&frontend, &initial_request().encode(), SendFlags::NOSIGNAL).unwrap(),
            BROKER_PACKET_BYTES
        );
        let request = receiver.join().unwrap().unwrap();
        assert_eq!(request.packet(), initial_request());
        assert!(request.deadline.remaining().is_ok());
    }

    #[test]
    fn failed_failure_reply_is_terminal_without_generation_advance() {
        let identity = [9; 32];
        let mut broker = BrokerState::new([7; 16]).unwrap();
        let initial = test_broker_request(initial_request());
        let pending = broker.begin_request(initial, Duration::ZERO).unwrap();
        let committed = commit_broker_success(&mut broker, pending, identity)
            .generation()
            .unwrap();
        let replacement = BrokerPacket {
            operation: BrokerOperation::Replacement,
            status: HandoffStatus::Request,
            epoch: committed.epoch(),
            sequence: committed.sequence(),
            expected_runner_identity: [0; 32],
        };
        let pending = broker
            .begin_request(test_broker_request(replacement), MIN_REPLACEMENT_INTERVAL)
            .unwrap();
        let (connection, frontend) = BrokerConnection::pair().unwrap();
        drop(frontend);
        assert_eq!(
            broker.commit_failure(pending, HandoffStatus::Unavailable, &connection),
            Err(HandoffError::TransferFailed)
        );
        assert_eq!(broker.committed_generation(), Some(committed));
        assert_eq!(
            broker.begin_request(
                test_broker_request(replacement),
                MIN_REPLACEMENT_INTERVAL * 2
            ),
            Err(HandoffError::UnexpectedState)
        );
    }

    #[test]
    fn fresh_broker_state_reserves_a_nonzero_initial_generation() {
        let mut broker = BrokerState::fresh().unwrap();
        let initial = test_broker_request(initial_request());
        let generation = broker
            .begin_request(initial, Duration::ZERO)
            .unwrap()
            .generation();
        assert_ne!(generation.epoch(), [0; 16]);
        assert_eq!(generation.sequence(), 1);
        assert_eq!(broker.committed_generation(), None);
    }

    #[test]
    fn malformed_and_out_of_order_packets_fail_closed() {
        let mut bytes = initial_request().encode();
        for index in 0..BROKER_PACKET_BYTES {
            let saved = bytes[index];
            bytes[index] ^= 0x80;
            assert!(BrokerPacket::decode(&bytes).is_err(), "byte {index}");
            bytes[index] = saved;
        }

        let mut broker = BrokerState::new([1; 16]).unwrap();
        let stale = BrokerPacket {
            operation: BrokerOperation::Replacement,
            status: HandoffStatus::Request,
            epoch: [1; 16],
            sequence: 1,
            expected_runner_identity: [0; 32],
        };
        let stale = test_broker_request(stale);
        assert_eq!(
            broker.begin_request(stale, Duration::ZERO),
            Err(HandoffError::UnexpectedState)
        );
        let initial = test_broker_request(initial_request());
        assert_eq!(
            broker.begin_request(initial, Duration::ZERO),
            Err(HandoffError::UnexpectedState)
        );
    }

    #[test]
    fn private_provisioning_transfers_one_cloexec_anonymous_control_stream() {
        let root = TestRoot::new();
        let (control_dir, provisioning_dir) = root.endpoint_dirs();
        let control_path = control_dir.join("control.sock");
        let provisioning_path = provisioning_dir.join("provision.sock");
        let server = ProvisionedControlServer::bind(
            &control_path,
            &provisioning_path,
            ControlLimits::default(),
            HandoffBackend,
        )
        .unwrap();
        assert_eq!(server.control_path(), control_path);
        assert_eq!(server.provisioning_path(), provisioning_path);
        let service = thread::spawn(move || server.serve_connections(0, 1));

        let connection = connect_seqpacket(&provisioning_path);
        let request = provisioning_request();
        assert_eq!(
            send(&connection, &request.encode(), SendFlags::NOSIGNAL).unwrap(),
            PROVISIONING_PACKET_BYTES
        );
        let (response, descriptor) = receive_generation(&connection);
        assert_eq!(response.status, HandoffStatus::Success);
        assert_eq!(response.nonce, request.nonce);
        assert_eq!(
            response.runner_identity,
            runner_identity_digest("runner-handoff-test").unwrap()
        );

        let mut stream = UnixStream::from(descriptor);
        let negotiation = crate::ControlRequest {
            jsonrpc: crate::JSONRPC_VERSION.into(),
            id: crate::RequestId(1),
            timeout_ms: 1_000,
            call: crate::ControlCall::Negotiate(crate::NegotiateParams {
                versions: BTreeSet::from(crate::SUPPORTED_CONTROL_VERSIONS),
                limits: ControlLimits::default(),
            }),
        };
        crate::write_frame(&mut stream, &negotiation, ControlLimits::default()).unwrap();
        let response: crate::ControlResponse =
            crate::read_frame(&mut stream, ControlLimits::default()).unwrap();
        let Some(crate::ControlSuccess::Negotiated(negotiated)) = response.result() else {
            panic!("expected negotiated response");
        };
        assert_eq!(negotiated.version, crate::CONTROL_MEASUREMENT_SELECTION_V1);
        assert_eq!(negotiated.runner_instance_id, "runner-handoff-test");
        drop(stream);
        drop(connection);
        service.join().unwrap().unwrap();
        assert!(!control_path.exists());
        assert!(!provisioning_path.exists());
    }

    #[test]
    fn provisioning_rejects_ancillary_input_before_packet_decode() {
        let (receiver, sender) = socketpair(
            AddressFamily::UNIX,
            SocketType::SEQPACKET,
            SocketFlags::CLOEXEC,
            None,
        )
        .unwrap();
        let (injected, _peer) = UnixStream::pair().unwrap();
        let payload = provisioning_request().encode();
        let iov = [IoSlice::new(&payload)];
        let descriptors = [injected.as_fd()];
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
        let mut ancillary = SendAncillaryBuffer::new(&mut space);
        assert!(ancillary.push(SendAncillaryMessage::ScmRights(&descriptors)));
        assert_eq!(
            sendmsg(&sender, &iov, &mut ancillary, SendFlags::NOSIGNAL).unwrap(),
            PROVISIONING_PACKET_BYTES
        );
        assert!(matches!(
            recv_provisioning_request(&receiver),
            Err(ProvisioningError::Rejected)
        ));
    }

    #[test]
    fn endpoint_topology_and_packet_excess_fail_before_binding_or_decode() {
        let root = TestRoot::new();
        let shared = root.0.join("shared");
        fs::create_dir(&shared).unwrap();
        fs::set_permissions(&shared, Permissions::from_mode(0o700)).unwrap();
        assert!(matches!(
            ProvisionedControlServer::bind(
                shared.join("control.sock"),
                shared.join("provision.sock"),
                ControlLimits::default(),
                HandoffBackend,
            ),
            Err(ProvisioningError::UnsafeTopology)
        ));

        let (receiver, sender) = socketpair(
            AddressFamily::UNIX,
            SocketType::SEQPACKET,
            SocketFlags::CLOEXEC,
            None,
        )
        .unwrap();
        let oversized = [0_u8; PROVISIONING_PACKET_BYTES + 1];
        assert_eq!(
            send(&sender, &oversized, SendFlags::NOSIGNAL).unwrap(),
            oversized.len()
        );
        assert!(matches!(
            recv_provisioning_request(&receiver),
            Err(ProvisioningError::Rejected)
        ));
    }

    #[test]
    fn endpoint_ancestor_symlink_is_rejected_before_bind() {
        let root = TestRoot::new();
        let real = root.0.join("real");
        let alias = root.0.join("alias");
        fs::create_dir(&real).unwrap();
        fs::set_permissions(&real, Permissions::from_mode(0o700)).unwrap();
        let control_dir = real.join("control");
        let provisioning_dir = real.join("provisioning");
        fs::create_dir(&control_dir).unwrap();
        fs::create_dir(&provisioning_dir).unwrap();
        fs::set_permissions(&control_dir, Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&provisioning_dir, Permissions::from_mode(0o700)).unwrap();
        std::os::unix::fs::symlink(&real, &alias).unwrap();

        let result = ProvisionedControlServer::bind(
            alias.join("control/control.sock"),
            alias.join("provisioning/provision.sock"),
            ControlLimits::default(),
            HandoffBackend,
        );
        assert!(matches!(result, Err(ProvisioningError::UnsafeDirectory)));
        assert!(!control_dir.join("control.sock").exists());
        assert!(!provisioning_dir.join("provision.sock").exists());
    }

    #[test]
    fn producer_returns_only_authenticated_anonymous_generation() {
        let root = TestRoot::new();
        let (control_dir, provisioning_dir) = root.endpoint_dirs();
        let control_path = control_dir.join("control.sock");
        let provisioning_path = provisioning_dir.join("provision.sock");
        let server = ProvisionedControlServer::bind(
            &control_path,
            &provisioning_path,
            ControlLimits::default(),
            HandoffBackend,
        )
        .unwrap();
        let service = thread::spawn(move || server.serve_connections(1, 1));

        let producer = AuthenticatedGenerationProducer::new(
            &control_path,
            &provisioning_path,
            ControlLimits::default(),
        )
        .unwrap();
        let mut broker = BrokerState::new([7; 16]).unwrap();
        let request = test_broker_request(initial_request());
        let pending = broker.begin_request(request, Duration::ZERO).unwrap();
        let generation = pending.generation();
        let authenticated = producer.acquire(&pending).unwrap();
        assert_eq!(authenticated.broker_generation(), generation);
        assert_eq!(authenticated.kernel_peer().uid(), geteuid().as_raw());
        assert_eq!(
            authenticated.expected_runner_identity(),
            runner_identity_digest("runner-handoff-test").unwrap()
        );
        drop(authenticated);
        service.join().unwrap().unwrap();
    }

    #[test]
    fn producer_cannot_reset_the_broker_request_deadline() {
        let root = TestRoot::new();
        let (control_dir, provisioning_dir) = root.endpoint_dirs();
        let producer = AuthenticatedGenerationProducer::new(
            control_dir.join("missing-control.sock"),
            provisioning_dir.join("missing-provision.sock"),
            ControlLimits::default(),
        )
        .unwrap();
        let request = BrokerRequest {
            packet: initial_request(),
            deadline: RequestDeadline::start(25).unwrap(),
        };
        let mut broker = BrokerState::new([7; 16]).unwrap();
        let pending = broker.begin_request(request, Duration::ZERO).unwrap();
        thread::sleep(Duration::from_millis(40));
        assert!(matches!(
            producer.acquire(&pending),
            Err(ProvisioningError::Control(EndpointError::Session(
                SessionError::DeadlineExceeded
            )))
        ));
        assert_eq!(broker.committed_generation(), None);
    }

    #[test]
    fn same_operation_deadline_substitution_is_rejected() {
        let mut broker = BrokerState::new([7; 16]).unwrap();
        let pending = broker
            .begin_request(test_broker_request(initial_request()), Duration::ZERO)
            .unwrap();
        let generation = pending.generation();
        thread::sleep(Duration::from_millis(1));
        let substituted = PendingBrokerSuccess {
            operation: BrokerOperation::Initial,
            generation,
            deadline: RequestDeadline::start(2_000).unwrap(),
        };
        assert_ne!(substituted.deadline, pending.deadline);
        let (authenticated, _service) = authenticated_generation(generation, [9; 32]);
        let (connection, _frontend) = BrokerConnection::pair().unwrap();
        assert_eq!(
            broker.commit_success(substituted, &connection, authenticated),
            Err(HandoffError::GenerationMismatch)
        );
        assert_eq!(broker.committed_generation(), None);
    }

    #[test]
    fn expired_commit_budget_sends_nothing_and_never_advances_generation() {
        let request = BrokerRequest {
            packet: initial_request(),
            deadline: RequestDeadline::start(25).unwrap(),
        };
        let mut broker = BrokerState::new([7; 16]).unwrap();
        let pending = broker.begin_request(request, Duration::ZERO).unwrap();
        let (authenticated, _service) = authenticated_generation(pending.generation(), [9; 32]);
        let (connection, frontend) = BrokerConnection::pair().unwrap();
        thread::sleep(Duration::from_millis(40));
        assert_eq!(
            broker.commit_success(pending, &connection, authenticated),
            Err(HandoffError::TransferFailed)
        );
        assert_eq!(broker.committed_generation(), None);
        let mut payload = [0_u8; BROKER_PACKET_BYTES];
        assert_eq!(
            recv(&frontend, &mut payload, RecvFlags::DONTWAIT),
            Err(rustix::io::Errno::AGAIN)
        );
        assert_eq!(
            broker.begin_request(
                test_broker_request(initial_request()),
                Duration::from_millis(50)
            ),
            Err(HandoffError::UnexpectedState)
        );
    }

    #[test]
    fn backpressured_commit_expires_without_advancing_generation() {
        let request = BrokerRequest {
            packet: initial_request(),
            deadline: RequestDeadline::start(250).unwrap(),
        };
        let mut broker = BrokerState::new([7; 16]).unwrap();
        let pending = broker.begin_request(request, Duration::ZERO).unwrap();
        let generation = pending.generation();
        let (authenticated, _service) = authenticated_generation(generation, [9; 32]);
        let (connection, _frontend) = BrokerConnection::pair().unwrap();
        let filler = [0_u8; BROKER_PACKET_BYTES];
        let mut packets = 0_usize;
        loop {
            match send(
                &connection.socket,
                &filler,
                SendFlags::NOSIGNAL | SendFlags::DONTWAIT,
            ) {
                Ok(BROKER_PACKET_BYTES) => {
                    packets += 1;
                    assert!(packets < 100_000, "socket never became backpressured");
                }
                Err(error) if error == rustix::io::Errno::AGAIN => break,
                result => panic!("unexpected backpressure fill result: {result:?}"),
            }
        }
        assert!(packets > 0);
        assert_eq!(
            broker.commit_success(pending, &connection, authenticated),
            Err(HandoffError::TransferFailed)
        );
        assert_eq!(broker.committed_generation(), None);
        assert_eq!(broker.phase, BrokerPhase::Terminal);
    }

    #[test]
    fn proc_stat_parser_uses_final_command_delimiter() {
        let mut record = b"42 (hostile ) name) R".to_vec();
        for value in 1..=18 {
            record.extend_from_slice(format!(" {value}").as_bytes());
        }
        record.extend_from_slice(b" 987654\n");
        assert_eq!(parse_process_start_time(42, &record), Some(987654));
        assert_eq!(parse_process_start_time(41, &record), None);
    }

    #[test]
    fn router_rejects_multiple_descriptors_and_preloaded_streams() {
        let (receiver, sender) = socketpair(
            AddressFamily::UNIX,
            SocketType::SEQPACKET,
            SocketFlags::CLOEXEC,
            None,
        )
        .unwrap();
        let (first, mut first_peer) = UnixStream::pair().unwrap();
        let (second, _second_peer) = UnixStream::pair().unwrap();
        let packet = ProvisioningPacket {
            status: HandoffStatus::Success,
            nonce: [3; 16],
            runner_identity: [4; 32],
        };
        let payload = packet.encode();
        let iov = [IoSlice::new(&payload)];
        let descriptors = [first.as_fd(), second.as_fd()];
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(2))];
        let mut ancillary = SendAncillaryBuffer::new(&mut space);
        assert!(ancillary.push(SendAncillaryMessage::ScmRights(&descriptors)));
        assert_eq!(
            sendmsg(&sender, &iov, &mut ancillary, SendFlags::NOSIGNAL).unwrap(),
            PROVISIONING_PACKET_BYTES
        );
        assert!(matches!(
            recv_provisioning_response(&receiver),
            Err(ProvisioningError::Rejected)
        ));

        let peer = PeerIdentity::from_fd(&first).unwrap();
        let evidence = process_evidence(peer).unwrap();
        first_peer.write_all(b"x").unwrap();
        let descriptor: OwnedFd = first.into();
        assert!(matches!(
            validate_anonymous_stream(&descriptor, evidence, geteuid().as_raw()),
            Err(ProvisioningError::Rejected)
        ));
    }
}
