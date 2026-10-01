// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! End-to-end local runner endpoint and reusable frontend client.

use std::io;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
#[cfg(test)]
use std::time::Duration;

use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{
    BoundControlResult, CONTROL_AGENT_LIFECYCLE_V1, CONTROL_V1, ControlCall, ControlLimits,
    ControlRequest, ControlResponse, ControlSession, ControlSuccess, ControlVersion, FrameError,
    Negotiated, OwnerSocket, PeerIdentity, ProtocolError, RequestDeadline, RequestId, Revision,
    SUPPORTED_CONTROL_VERSIONS, SessionError, authenticate_owner, error_code, read_frame_until,
    validate_identity, write_frame_until,
};

const MAX_REQUESTS_PER_CONNECTION: usize = 1024;
const MAX_CONNECTION_WORKERS: usize = 16;

/// Stable public failure from the authoritative runner backend.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendFailure {
    /// Requested durable object does not exist.
    NotFound,
    /// A causal run or attempt identity is stale.
    StaleIdentity,
    /// Reconnect cursor is outside retained runner history.
    StaleCursor,
    /// Requested capability is unavailable on this runner.
    CapabilityUnavailable,
    /// Mutation effect is uncertain and must be reconciled runner-side.
    NeedsReconciliation,
    /// Valid request was rejected by runner policy.
    Rejected,
}

impl BackendFailure {
    fn response(self, id: RequestId) -> ControlResponse {
        let (code, message) = match self {
            Self::NotFound => (error_code::STALE_IDENTITY, "durable object not found"),
            Self::StaleIdentity => (error_code::STALE_IDENTITY, "causal identity is stale"),
            Self::StaleCursor => (error_code::STALE_CURSOR, "reconnect cursor is stale"),
            Self::CapabilityUnavailable => (
                error_code::CAPABILITY_UNAVAILABLE,
                "requested capability is unavailable",
            ),
            Self::NeedsReconciliation => (
                error_code::IDEMPOTENCY_CONFLICT,
                "runner reconciliation is required",
            ),
            Self::Rejected => (
                error_code::RESOURCE_EXHAUSTED,
                "request rejected by runner policy",
            ),
        };
        ControlResponse::failure(id, code, message)
    }
}

/// Runner-owned implementation behind the transport boundary.
///
/// Implementations must durably fence mutations before returning and must stop
/// or record `needs_reconciliation` when `deadline` expires. They return only a
/// privacy-reviewed projection; the endpoint independently validates it.
pub trait ControlBackend {
    /// Stable, transport-neutral identity of this recovered runner instance.
    fn runner_instance_id(&self) -> &str;
    /// Oldest retained public event revision.
    fn oldest_revision(&self) -> Revision;
    /// Latest committed public event revision.
    fn latest_revision(&self) -> Revision;
    /// Authoritative live broker continuity, when this endpoint is serving an
    /// adopted frontend generation. Ordinary control endpoints return `None`.
    fn broker_generation(&self) -> Option<crate::NegotiatedBrokerGeneration> {
        None
    }
    /// Execute one admitted operation against authoritative runner state.
    fn execute(
        &self,
        call: &ControlCall,
        deadline: RequestDeadline,
    ) -> Result<BoundControlResult, BackendFailure>;
    /// Execute with the negotiated wire version available for additive result projection.
    fn execute_versioned(
        &self,
        call: &ControlCall,
        deadline: RequestDeadline,
        _version: ControlVersion,
    ) -> Result<BoundControlResult, BackendFailure> {
        self.execute(call, deadline)
    }

    /// Execute with the kernel-authenticated local peer identity.
    ///
    /// The default preserves existing backends; authority-sensitive backends
    /// override this hook rather than accepting caller-supplied identity data.
    fn execute_authenticated(
        &self,
        call: &ControlCall,
        deadline: RequestDeadline,
        version: ControlVersion,
        _peer: PeerIdentity,
    ) -> Result<BoundControlResult, BackendFailure> {
        self.execute_versioned(call, deadline, version)
    }
}

/// One local endpoint. It owns no run lifetime and may be recreated safely.
pub struct ControlServer<B> {
    socket: OwnerSocket,
    backend: Arc<B>,
    limits: ControlLimits,
    admission: AdmissionGuard,
}

#[derive(Clone)]
pub(crate) struct AdmissionGuard {
    state: Arc<AdmissionState>,
}

struct AdmissionState {
    active: Mutex<usize>,
    available: Condvar,
}

pub(crate) struct AdmissionPermit {
    state: Arc<AdmissionState>,
}

impl AdmissionGuard {
    pub(crate) fn new() -> Self {
        Self {
            state: Arc::new(AdmissionState {
                active: Mutex::new(0),
                available: Condvar::new(),
            }),
        }
    }

    fn acquire(&self) -> AdmissionPermit {
        let mut active = self
            .state
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while *active >= MAX_CONNECTION_WORKERS {
            active = self
                .state
                .available
                .wait(active)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        *active += 1;
        AdmissionPermit {
            state: Arc::clone(&self.state),
        }
    }

    fn try_acquire(&self) -> Option<AdmissionPermit> {
        let mut active = self
            .state
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *active >= MAX_CONNECTION_WORKERS {
            return None;
        }
        *active += 1;
        Some(AdmissionPermit {
            state: Arc::clone(&self.state),
        })
    }

    #[cfg(test)]
    fn active(&self) -> usize {
        *self
            .state
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl Drop for AdmissionPermit {
    fn drop(&mut self) {
        let mut active = self
            .state
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *active = active.saturating_sub(1);
        self.state.available.notify_one();
    }
}

impl<B: ControlBackend + Send + Sync + 'static> ControlServer<B> {
    /// Bind a fail-closed owner-only local endpoint.
    pub fn bind(
        path: impl AsRef<Path>,
        limits: ControlLimits,
        backend: B,
    ) -> Result<Self, EndpointError> {
        let mut limits = limits.validate()?;
        // v1 preserves response ordering with one synchronous request per
        // connection. Multiplexing is reserved for a future negotiated version.
        limits.max_in_flight = 1;
        validate_identity(backend.runner_instance_id())?;
        Self::bind_shared(path, limits, Arc::new(backend), AdmissionGuard::new())
    }

    pub(crate) fn bind_shared(
        path: impl AsRef<Path>,
        mut limits: ControlLimits,
        backend: Arc<B>,
        admission: AdmissionGuard,
    ) -> Result<Self, EndpointError> {
        limits = limits.validate()?;
        limits.max_in_flight = 1;
        validate_identity(backend.runner_instance_id())?;
        Ok(Self {
            socket: OwnerSocket::bind(path, limits)?,
            backend,
            limits,
            admission,
        })
    }

    /// Socket path for discovery by an independently started frontend.
    #[must_use]
    pub fn path(&self) -> &Path {
        self.socket.path()
    }

    /// Accept and serve one frontend connection. Disconnect has no backend effect.
    pub fn serve_one(&mut self) -> Result<(), EndpointError> {
        let (mut stream, _) = self.socket.accept()?;
        let _permit = self.admission.acquire();
        self.serve_stream(&mut stream)
    }

    /// Serve one authenticated development broker generation.
    ///
    /// The router uses this bounded seam after it has transferred a stream
    /// carrying a validated broker generation.  It deliberately reuses the
    /// ordinary negotiation and request loop: the negotiated protocol minor
    /// and runner identity are still checked by the control endpoint, and no
    /// stable authentication or signature gate is bypassed.  The generation
    /// is continuity evidence for the caller and is checked here before any
    /// bytes are accepted.
    pub fn serve_authenticated_generation(
        &self,
        authenticated: crate::AuthenticatedGeneration,
    ) -> Result<(), EndpointError> {
        if authenticated.protocol_version() != CONTROL_AGENT_LIFECYCLE_V1 {
            return Err(EndpointError::UnsupportedAdoptedProtocol);
        }
        let expected_runner_identity = authenticated.expected_runner_identity();
        let generation = authenticated.broker_generation();
        let expected_peer = authenticated.kernel_peer();
        let stream = authenticated.into_stream();
        let peer = PeerIdentity::from_fd(&stream).map_err(EndpointError::Transport)?;
        if peer != expected_peer {
            return Err(EndpointError::AdoptedPeerMismatch);
        }
        self.serve_adopted_stream(stream, generation, expected_runner_identity)
    }

    /// Serve an adopted stream after the handoff layer has authenticated it.
    ///
    /// This remains private so callers cannot fabricate a stream, generation,
    /// or identity tuple. Use [`Self::serve_authenticated_generation`] with
    /// the capability returned by the authenticated handoff producer.
    fn serve_adopted_stream(
        &self,
        mut stream: UnixStream,
        generation: crate::BrokerGeneration,
        expected_runner_identity: [u8; 32],
    ) -> Result<(), EndpointError> {
        if generation.epoch() == [0; 16]
            || generation.sequence() == 0
            || expected_runner_identity == [0; 32]
        {
            return Err(EndpointError::InvalidAdoptedGeneration);
        }
        let actual_runner_identity =
            crate::runner_identity_digest(self.backend.runner_instance_id())
                .map_err(|_| EndpointError::InvalidAdoptedGeneration)?;
        if actual_runner_identity != expected_runner_identity {
            return Err(EndpointError::AdoptedIdentityMismatch);
        }
        let _permit = self.admission.acquire();
        Self::serve_stream_with(&self.backend, self.limits, &mut stream)
    }

    /// Serve frontend connections until the process is stopped.
    ///
    /// Each connection is request-serial; a separate fixed worker ceiling lets
    /// healthy peers proceed while one same-user peer is slow.
    pub fn serve(&mut self) -> Result<(), EndpointError> {
        self.serve_connections(usize::MAX)
    }

    /// Serve a bounded number of accepted connections.
    ///
    /// This is primarily useful for supervised service integration and tests.
    /// Connection failures are isolated and do not terminate the accept loop.
    pub fn serve_connections(&mut self, maximum: usize) -> Result<(), EndpointError> {
        if maximum == 0 {
            return Ok(());
        }
        let mut workers: Vec<thread::JoinHandle<()>> = Vec::new();
        let mut accepted = 0_usize;
        while accepted < maximum {
            reap_workers(&mut workers);
            let (mut stream, _) = match self.socket.accept() {
                Ok(connection) => connection,
                Err(error) => {
                    join_workers(workers);
                    return Err(error.into());
                }
            };
            accepted += 1;
            let permit = self.admission.acquire();
            match self.spawn_stream_with_permit(&mut stream, permit) {
                Ok(worker) => workers.push(worker),
                Err(error) => {
                    join_workers(workers);
                    return Err(error);
                }
            }
        }
        join_workers(workers);
        Ok(())
    }

    fn serve_stream(&self, stream: &mut UnixStream) -> Result<(), EndpointError> {
        Self::serve_stream_with(&self.backend, self.limits, stream)
    }

    pub(crate) fn set_nonblocking(&self, nonblocking: bool) -> Result<(), EndpointError> {
        self.socket.listener().set_nonblocking(nonblocking)?;
        Ok(())
    }

    pub(crate) fn try_accept_and_spawn(
        &self,
        workers: &mut Vec<thread::JoinHandle<()>>,
    ) -> Result<bool, EndpointError> {
        let (mut stream, _) = match self.socket.accept() {
            Ok(connection) => connection,
            Err(crate::TransportError::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => {
                return Ok(false);
            }
            Err(error) => return Err(error.into()),
        };
        let Some(permit) = self.admission.try_acquire() else {
            return Ok(true);
        };
        workers.push(self.spawn_stream_with_permit(&mut stream, permit)?);
        Ok(true)
    }

    pub(crate) fn try_spawn_anonymous(
        &self,
        mut stream: UnixStream,
    ) -> Result<Option<thread::JoinHandle<()>>, EndpointError> {
        let Some(permit) = self.admission.try_acquire() else {
            return Ok(None);
        };
        self.spawn_stream_with_permit(&mut stream, permit).map(Some)
    }

    fn spawn_stream_with_permit(
        &self,
        stream: &mut UnixStream,
        permit: AdmissionPermit,
    ) -> Result<thread::JoinHandle<()>, EndpointError> {
        let backend = Arc::clone(&self.backend);
        let limits = self.limits;
        let mut stream = stream.try_clone()?;
        thread::Builder::new()
            .name("asb-control-connection".into())
            .spawn(move || {
                let _permit = permit;
                let _ = Self::serve_stream_with(&backend, limits, &mut stream);
            })
            .map_err(EndpointError::Io)
    }

    fn serve_stream_with(
        backend: &B,
        limits: ControlLimits,
        stream: &mut UnixStream,
    ) -> Result<(), EndpointError> {
        let peer = PeerIdentity::from_fd(&mut *stream).map_err(EndpointError::Transport)?;
        peer.require_owner(rustix::process::geteuid().as_raw())
            .map_err(EndpointError::Transport)?;
        let mut session = ControlSession::new(limits)?;
        let ingress = RequestDeadline::start(limits.max_timeout_ms)?;
        let first: ControlRequest = read_frame_until(stream, limits, ingress)?;
        let (version, effective) = session.negotiate_request(&first)?;
        let deadline = ingress.tighten(first.timeout_ms)?;
        let negotiated = Negotiated {
            version,
            limits: effective,
            runner_instance_id: backend.runner_instance_id().to_owned(),
            broker_generation: backend.broker_generation(),
            oldest_revision: backend.oldest_revision(),
            latest_revision: backend.latest_revision(),
        };
        if negotiated.oldest_revision > negotiated.latest_revision {
            return Err(EndpointError::UnexpectedResponse);
        }
        let response = ControlResponse::success(first.id, ControlSuccess::Negotiated(negotiated));
        write_public_response(stream, &response, effective, deadline)?;

        for _ in 0..MAX_REQUESTS_PER_CONNECTION {
            let ingress = RequestDeadline::start(effective.max_timeout_ms)?;
            let request: ControlRequest = match read_frame_until(stream, effective, ingress) {
                Ok(request) => request,
                Err(FrameError::Closed) => return Ok(()),
                Err(error) => return Err(error.into()),
            };
            let admission = match session.admit_with_deadline(&request, ingress) {
                Ok(admission) => admission,
                Err(SessionError::CapabilityUnavailable) => {
                    let response = BackendFailure::CapabilityUnavailable.response(request.id);
                    let deadline = ingress.tighten(request.timeout_ms)?;
                    write_public_response(stream, &response, effective, deadline)?;
                    continue;
                }
                Err(error) => return Err(error.into()),
            };
            let id = admission.id;
            let deadline = admission.deadline();
            let outcome = backend.execute_authenticated(&request.call, deadline, version, peer);
            let response = match outcome {
                Ok(result) => {
                    deadline.check()?;
                    result.validate_for_call_and_version(&request.call, effective, version)?;
                    ControlResponse::success(id, ControlSuccess::Operation(result))
                }
                Err(failure) => failure.response(id),
            };
            let written = write_public_response(stream, &response, effective, deadline);
            session.finish(id)?;
            written?;
        }
        Ok(())
    }
}

pub(crate) fn reap_workers(workers: &mut Vec<thread::JoinHandle<()>>) {
    let mut index = 0;
    while index < workers.len() {
        if workers[index].is_finished() {
            let worker = workers.swap_remove(index);
            let _ = worker.join();
        } else {
            index += 1;
        }
    }
}

pub(crate) fn join_workers(workers: Vec<thread::JoinHandle<()>>) {
    for worker in workers {
        let _ = worker.join();
    }
}

/// Reusable frontend connection. Dropping it never sends cancellation.
pub struct ControlClient {
    stream: UnixStream,
    limits: ControlLimits,
    next_id: u64,
    negotiated: Negotiated,
}

impl ControlClient {
    /// Connect and negotiate the exact supported protocol before other calls.
    pub fn connect(path: impl AsRef<Path>, limits: ControlLimits) -> Result<Self, EndpointError> {
        let stream = UnixStream::connect(path)?;
        Self::from_stream(stream, limits, rustix::process::geteuid().as_raw())
    }

    /// Connect while explicitly offering exact protocol versions.
    pub fn connect_with_versions(
        path: impl AsRef<Path>,
        limits: ControlLimits,
        versions: impl IntoIterator<Item = ControlVersion>,
    ) -> Result<Self, EndpointError> {
        let versions = versions
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>();
        if versions.is_empty()
            || versions
                .iter()
                .any(|version| !SUPPORTED_CONTROL_VERSIONS.contains(version))
        {
            return Err(EndpointError::UnexpectedResponse);
        }
        let stream = UnixStream::connect(path)?;
        Self::from_stream_with_versions(
            stream,
            limits,
            rustix::process::geteuid().as_raw(),
            versions,
        )
    }

    fn from_stream(
        stream: UnixStream,
        limits: ControlLimits,
        expected_uid: u32,
    ) -> Result<Self, EndpointError> {
        Self::from_stream_with_versions(
            stream,
            limits,
            expected_uid,
            [CONTROL_V1].into_iter().collect(),
        )
    }

    pub(crate) fn from_stream_with_versions(
        stream: UnixStream,
        limits: ControlLimits,
        expected_uid: u32,
        versions: std::collections::BTreeSet<ControlVersion>,
    ) -> Result<Self, EndpointError> {
        let deadline = RequestDeadline::start(limits.max_timeout_ms)?;
        Self::from_stream_with_versions_until(stream, limits, expected_uid, versions, deadline)
    }

    pub(crate) fn from_stream_with_versions_until(
        mut stream: UnixStream,
        limits: ControlLimits,
        expected_uid: u32,
        versions: std::collections::BTreeSet<ControlVersion>,
        deadline: RequestDeadline,
    ) -> Result<Self, EndpointError> {
        let limits = limits.validate()?;
        if versions.is_empty()
            || versions
                .iter()
                .any(|version| !SUPPORTED_CONTROL_VERSIONS.contains(version))
        {
            return Err(EndpointError::UnexpectedResponse);
        }
        // A private server socket authenticates clients, but the independently
        // started frontend must also reject a socket owned by another user.
        // Perform this check before sending any request content.
        authenticate_owner(&stream, expected_uid)?;
        let request = ControlRequest {
            jsonrpc: crate::JSONRPC_VERSION.into(),
            id: RequestId(1),
            timeout_ms: limits.max_timeout_ms,
            call: ControlCall::Negotiate(crate::NegotiateParams {
                versions: versions.clone(),
                limits,
            }),
        };
        write_frame_until(&mut stream, &request, limits, deadline)?;
        let response: ControlResponse = read_frame_until(&mut stream, limits, deadline)?;
        response.validate()?;
        if response.id() != request.id || response.error().is_some() {
            return Err(EndpointError::UnexpectedResponse);
        }
        let Some(ControlSuccess::Negotiated(negotiated)) = response.into_result() else {
            return Err(EndpointError::UnexpectedResponse);
        };
        if !versions.contains(&negotiated.version)
            || !SUPPORTED_CONTROL_VERSIONS.contains(&negotiated.version)
            || negotiated.limits.validate().is_err()
            || !limits_include(limits, negotiated.limits)
            || negotiated.oldest_revision > negotiated.latest_revision
        {
            return Err(EndpointError::UnexpectedResponse);
        }
        validate_identity(&negotiated.runner_instance_id)?;
        Ok(Self {
            stream,
            limits: negotiated.limits,
            next_id: 2,
            negotiated,
        })
    }

    /// Negotiated runner identity, revisions, and effective bounds.
    #[must_use]
    pub fn negotiated(&self) -> &Negotiated {
        &self.negotiated
    }

    /// Return the server-bound session identity digest for authority requests.
    ///
    /// The digest binds the negotiated runner identity to this client process
    /// and kernel credentials. It contains no path, credential, or host data.
    pub fn session_identity_sha256(&self) -> Result<String, EndpointError> {
        let uid = rustix::process::geteuid().as_raw();
        let gid = rustix::process::getegid().as_raw();
        let pid = std::process::id();
        let mut digest = Sha256::new();
        digest.update(b"asb-control-session-v1\0");
        digest.update((self.negotiated.runner_instance_id.len() as u64).to_le_bytes());
        digest.update(self.negotiated.runner_instance_id.as_bytes());
        digest.update(uid.to_le_bytes());
        digest.update(gid.to_le_bytes());
        digest.update(pid.to_le_bytes());
        Ok(format!("{:x}", digest.finalize()))
    }

    /// Execute one typed call and return its validated public response.
    pub fn call(
        &mut self,
        call: ControlCall,
        timeout_ms: u64,
    ) -> Result<ControlResponse, EndpointError> {
        let deadline = RequestDeadline::start(timeout_ms)?;
        self.call_until(call, deadline)
    }

    pub(crate) fn call_until(
        &mut self,
        call: ControlCall,
        deadline: RequestDeadline,
    ) -> Result<ControlResponse, EndpointError> {
        if matches!(call, ControlCall::Negotiate(_)) {
            return Err(EndpointError::UnexpectedResponse);
        }
        if self.negotiated.version < call.minimum_version() {
            return Err(EndpointError::UnexpectedResponse);
        }
        let id = RequestId(self.next_id);
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or(EndpointError::RequestIdExhausted)?;
        let request = ControlRequest {
            jsonrpc: crate::JSONRPC_VERSION.into(),
            id,
            timeout_ms: u64::try_from(deadline.remaining()?.as_millis())
                .map_err(|_| EndpointError::UnexpectedResponse)?
                .max(1),
            call: call.clone(),
        };
        crate::validate_request(&request, self.limits)?;
        write_frame_until(&mut self.stream, &request, self.limits, deadline)?;
        let response: ControlResponse = read_frame_until(&mut self.stream, self.limits, deadline)?;
        response.validate()?;
        if response.id() != id {
            return Err(EndpointError::UnexpectedResponse);
        }
        match response.result() {
            Some(ControlSuccess::Operation(result)) => {
                result.validate_for_call_and_version(
                    &call,
                    self.limits,
                    self.negotiated.version,
                )?;
            }
            None if response.error().is_some() => {}
            _ => return Err(EndpointError::UnexpectedResponse),
        }
        Ok(response)
    }

    pub(crate) fn peer_identity(&self, expected_uid: u32) -> Result<PeerIdentity, EndpointError> {
        Ok(authenticate_owner(&self.stream, expected_uid)?)
    }
}

fn limits_include(offered: ControlLimits, selected: ControlLimits) -> bool {
    selected.max_frame_bytes <= offered.max_frame_bytes
        && selected.max_timeout_ms <= offered.max_timeout_ms
        && selected.max_page_items <= offered.max_page_items
        && selected.max_in_flight <= offered.max_in_flight
}

fn write_public_response(
    stream: &mut UnixStream,
    response: &ControlResponse,
    limits: ControlLimits,
    deadline: RequestDeadline,
) -> Result<(), EndpointError> {
    response.validate()?;
    write_frame_until(stream, response, limits, deadline)?;
    Ok(())
}

/// Local endpoint/client failure without backend-private diagnostic content.
#[derive(Debug, Error)]
pub enum EndpointError {
    /// Local transport failure.
    #[error(transparent)]
    Transport(#[from] crate::TransportError),
    /// Frame failure.
    #[error(transparent)]
    Frame(#[from] FrameError),
    /// Protocol failure.
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
    /// Connection state failure.
    #[error(transparent)]
    Session(#[from] SessionError),
    /// JSON conversion failure.
    #[error("control message conversion failed")]
    Json(#[from] serde_json::Error),
    /// Kernel socket operation failed.
    #[error("control endpoint I/O failed")]
    Io(#[from] io::Error),
    /// Server response did not match the outstanding request/negotiation.
    #[error("unexpected control response")]
    UnexpectedResponse,
    /// Client exhausted its non-repeating request ID space.
    #[error("control request identity exhausted")]
    RequestIdExhausted,
    /// The broker generation supplied by an adopted development stream was invalid.
    #[error("adopted control generation is invalid")]
    InvalidAdoptedGeneration,
    /// The adopted stream was bound to a different runner identity.
    #[error("adopted control identity mismatch")]
    AdoptedIdentityMismatch,
    /// The handoff capability was negotiated for an unsupported protocol minor.
    #[error("adopted control protocol is unsupported")]
    UnsupportedAdoptedProtocol,
    /// The stream peer differs from the handoff capability's peer evidence.
    #[error("adopted control peer mismatch")]
    AdoptedPeerMismatch,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::os::unix::net::UnixListener;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SOCKET_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct AdoptedBackend;

    impl ControlBackend for AdoptedBackend {
        fn runner_instance_id(&self) -> &str {
            "runner-adopted-test"
        }

        fn oldest_revision(&self) -> Revision {
            Revision(0)
        }

        fn latest_revision(&self) -> Revision {
            Revision(0)
        }

        fn execute(
            &self,
            _call: &ControlCall,
            _deadline: RequestDeadline,
        ) -> Result<BoundControlResult, BackendFailure> {
            Err(BackendFailure::Rejected)
        }
    }

    fn adopted_generation() -> crate::BrokerGeneration {
        let (router, frontend) = crate::BrokerConnection::pair().expect("broker pair");
        let request = crate::BrokerPacket {
            operation: crate::BrokerOperation::Initial,
            status: crate::HandoffStatus::Request,
            epoch: [0; 16],
            sequence: 0,
            expected_runner_identity: [0; 32],
        };
        rustix::net::send(
            &frontend,
            &request.encode(),
            rustix::net::SendFlags::NOSIGNAL,
        )
        .expect("request");
        let request = router.receive_request().expect("receive request");
        let mut state = crate::BrokerState::fresh().expect("generation");
        state
            .begin_request(request, std::time::Duration::ZERO)
            .expect("pending")
            .generation()
    }

    fn adopted_socket_path(label: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!(
            "asb-control-adopted-root-{}-{}",
            std::process::id(),
            SOCKET_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).expect("private root");
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
            .expect("private permissions");
        (root.clone(), root.join(format!("{label}.sock")))
    }

    #[test]
    fn adopted_stream_reuses_negotiation_and_binds_identity() {
        let (root, path) = adopted_socket_path("valid");
        let _ = std::fs::remove_file(&path);
        let server =
            ControlServer::bind(&path, ControlLimits::default(), AdoptedBackend).expect("server");
        let (client_stream, server_stream) = UnixStream::pair().expect("stream pair");
        let generation = adopted_generation();
        let expected = crate::runner_identity_digest("runner-adopted-test").expect("identity");
        let peer = PeerIdentity::from_fd(&server_stream).expect("peer");
        let authenticated = crate::AuthenticatedGeneration::for_test(
            server_stream,
            generation,
            peer,
            expected,
            CONTROL_AGENT_LIFECYCLE_V1,
        );
        let worker =
            std::thread::spawn(move || server.serve_authenticated_generation(authenticated));
        let client = ControlClient::from_stream(
            client_stream,
            ControlLimits::default(),
            rustix::process::geteuid().as_raw(),
        )
        .expect("negotiation");
        assert_eq!(
            client.negotiated().runner_instance_id,
            "runner-adopted-test"
        );
        drop(client);
        worker.join().expect("worker join").expect("serve");
        let _ = std::fs::remove_file(path);
        std::fs::remove_dir(root).expect("remove root");
    }

    #[test]
    fn adopted_stream_rejects_mismatched_identity_before_io() {
        let (root, path) = adopted_socket_path("mismatch");
        let _ = std::fs::remove_file(&path);
        let server =
            ControlServer::bind(&path, ControlLimits::default(), AdoptedBackend).expect("server");
        let (_client_stream, server_stream) = UnixStream::pair().expect("stream pair");
        let error = server
            .serve_adopted_stream(server_stream, adopted_generation(), [7; 32])
            .expect_err("mismatch must fail closed");
        assert!(matches!(error, EndpointError::AdoptedIdentityMismatch));
        let _ = std::fs::remove_file(path);
        std::fs::remove_dir(root).expect("remove root");
    }

    #[test]
    fn adopted_stream_rejects_zero_generation_and_identity() {
        let (root, path) = adopted_socket_path("invalid");
        let _ = std::fs::remove_file(&path);
        let server =
            ControlServer::bind(&path, ControlLimits::default(), AdoptedBackend).expect("server");
        let (_client_stream, server_stream) = UnixStream::pair().expect("stream pair");
        let zero = crate::BrokerGeneration::from_parts([0; 16], 0);
        let error = server
            .serve_adopted_stream(server_stream, zero, [0; 32])
            .expect_err("zero evidence must fail closed");
        assert!(matches!(error, EndpointError::InvalidAdoptedGeneration));
        let _ = std::fs::remove_file(path);
        std::fs::remove_dir(root).expect("remove root");
    }

    #[test]
    fn adopted_stream_returns_bounded_error_for_malformed_input() {
        let (root, path) = adopted_socket_path("malformed");
        let _ = std::fs::remove_file(&path);
        let limits = ControlLimits {
            max_timeout_ms: 50,
            ..ControlLimits::default()
        };
        let server = ControlServer::bind(&path, limits, AdoptedBackend).expect("server");
        let (mut client_stream, server_stream) = UnixStream::pair().expect("stream pair");
        let generation = adopted_generation();
        let expected = crate::runner_identity_digest("runner-adopted-test").expect("identity");
        let worker = std::thread::spawn(move || {
            server.serve_adopted_stream(server_stream, generation, expected)
        });
        std::io::Write::write_all(&mut client_stream, b"not-json").expect("malformed frame");
        let error = worker
            .join()
            .expect("worker join")
            .expect_err("malformed input");
        assert!(matches!(
            error,
            EndpointError::Frame(_) | EndpointError::Json(_)
        ));
        let _ = std::fs::remove_file(path);
        std::fs::remove_dir(root).expect("remove root");
    }

    #[test]
    fn adopted_stream_times_out_without_a_negotiation_frame() {
        let (root, path) = adopted_socket_path("timeout");
        let _ = std::fs::remove_file(&path);
        let limits = ControlLimits {
            max_timeout_ms: 25,
            ..ControlLimits::default()
        };
        let server = ControlServer::bind(&path, limits, AdoptedBackend).expect("server");
        let (_client_stream, server_stream) = UnixStream::pair().expect("stream pair");
        let generation = adopted_generation();
        let expected = crate::runner_identity_digest("runner-adopted-test").expect("identity");
        let worker = std::thread::spawn(move || {
            server.serve_adopted_stream(server_stream, generation, expected)
        });
        let error = worker.join().expect("worker join").expect_err("timeout");
        assert!(matches!(error, EndpointError::Frame(_)));
        let _ = std::fs::remove_file(path);
        std::fs::remove_dir(root).expect("remove root");
    }

    #[test]
    fn shared_admission_has_one_exact_sixteen_session_ceiling() {
        let admission = AdmissionGuard::new();
        let permits: Vec<_> = (0..MAX_CONNECTION_WORKERS)
            .map(|_| admission.try_acquire().expect("slot below ceiling"))
            .collect();
        assert_eq!(admission.active(), MAX_CONNECTION_WORKERS);
        assert!(admission.try_acquire().is_none());
        drop(permits);
        assert_eq!(admission.active(), 0);
        assert!(admission.try_acquire().is_some());
    }

    #[test]
    fn every_backend_failure_has_a_fixed_public_response() {
        let cases = [
            (BackendFailure::NotFound, error_code::STALE_IDENTITY),
            (BackendFailure::StaleIdentity, error_code::STALE_IDENTITY),
            (BackendFailure::StaleCursor, error_code::STALE_CURSOR),
            (
                BackendFailure::CapabilityUnavailable,
                error_code::CAPABILITY_UNAVAILABLE,
            ),
            (
                BackendFailure::NeedsReconciliation,
                error_code::IDEMPOTENCY_CONFLICT,
            ),
            (BackendFailure::Rejected, error_code::RESOURCE_EXHAUSTED),
        ];
        for (failure, code) in cases {
            let response = failure.response(RequestId(9));
            assert_eq!(response.id(), RequestId(9));
            assert_eq!(response.error().expect("error").code, code);
            response.validate().expect("fixed public response");
        }
    }

    #[test]
    fn client_rejects_renegotiation_and_request_id_exhaustion_before_io() {
        let (stream, _peer) = UnixStream::pair().expect("socket pair");
        let negotiated = Negotiated {
            version: CONTROL_V1,
            limits: ControlLimits::default(),
            runner_instance_id: "runner-test".into(),
            broker_generation: None,
            oldest_revision: Revision(0),
            latest_revision: Revision(0),
        };
        let mut client = ControlClient {
            stream,
            limits: ControlLimits::default(),
            next_id: 2,
            negotiated,
        };
        assert!(matches!(
            client.call(
                ControlCall::Negotiate(crate::NegotiateParams {
                    versions: [CONTROL_V1].into_iter().collect(),
                    limits: ControlLimits::default(),
                }),
                10,
            ),
            Err(EndpointError::UnexpectedResponse)
        ));
        client.next_id = u64::MAX;
        assert!(matches!(
            client.call(ControlCall::Capabilities, 10),
            Err(EndpointError::RequestIdExhausted)
        ));
    }

    #[test]
    fn client_rejects_wrong_owner_before_sending_negotiation() {
        let (client, mut peer) = UnixStream::pair().expect("socket pair");
        let wrong_uid = rustix::process::geteuid().as_raw().wrapping_add(1);
        assert!(matches!(
            ControlClient::from_stream(client, ControlLimits::default(), wrong_uid),
            Err(EndpointError::Transport(
                crate::TransportError::UnauthorizedPeer { .. }
            ))
        ));
        peer.set_read_timeout(Some(Duration::from_millis(20)))
            .expect("read timeout");
        let mut byte = [0_u8; 1];
        assert_eq!(peer.read(&mut byte).expect("peer closed without data"), 0);
    }

    #[test]
    fn client_rejects_unimplemented_offers_before_sending_negotiation() {
        let (client, mut peer) = UnixStream::pair().expect("socket pair");
        assert!(matches!(
            ControlClient::from_stream_with_versions(
                client,
                ControlLimits::default(),
                rustix::process::geteuid().as_raw(),
                [crate::ControlVersion {
                    major: 1,
                    minor: 99
                }]
                .into_iter()
                .collect(),
            ),
            Err(EndpointError::UnexpectedResponse)
        ));
        peer.set_read_timeout(Some(Duration::from_millis(20)))
            .expect("read timeout");
        let mut byte = [0_u8; 1];
        assert_eq!(peer.read(&mut byte).expect("peer closed without data"), 0);
    }

    #[test]
    fn public_client_rejects_invalid_offers_before_connecting() {
        let path = std::env::temp_dir().join(format!(
            "asb-control-invalid-offer-{}-{}.sock",
            std::process::id(),
            SOCKET_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).expect("bind test listener");
        listener
            .set_nonblocking(true)
            .expect("nonblocking listener");

        for versions in [
            Vec::new(),
            vec![crate::ControlVersion {
                major: 1,
                minor: 99,
            }],
        ] {
            assert!(matches!(
                ControlClient::connect_with_versions(&path, ControlLimits::default(), versions,),
                Err(EndpointError::UnexpectedResponse)
            ));
            assert!(matches!(
                listener.accept(),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock
            ));
        }

        drop(listener);
        std::fs::remove_file(path).expect("remove test socket");
    }
}
