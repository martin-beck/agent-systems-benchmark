// SPDX-License-Identifier: MIT
//! End-to-end local runner endpoint and reusable frontend client.

use std::io;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use thiserror::Error;

use crate::{
    BoundControlResult, CONTROL_V1, ControlCall, ControlLimits, ControlRequest, ControlResponse,
    ControlSession, ControlSuccess, FrameError, Negotiated, OwnerSocket, ProtocolError,
    RequestDeadline, RequestId, Revision, SessionError, error_code, read_frame_until,
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
    /// Execute one admitted operation against authoritative runner state.
    fn execute(
        &self,
        call: &ControlCall,
        deadline: RequestDeadline,
    ) -> Result<BoundControlResult, BackendFailure>;
}

/// One local endpoint. It owns no run lifetime and may be recreated safely.
pub struct ControlServer<B> {
    socket: OwnerSocket,
    backend: Arc<B>,
    limits: ControlLimits,
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
        Ok(Self {
            socket: OwnerSocket::bind(path, limits)?,
            backend: Arc::new(backend),
            limits,
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
        self.serve_stream(&mut stream)
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
        let worker_limit = MAX_CONNECTION_WORKERS;
        let mut workers: Vec<thread::JoinHandle<()>> = Vec::new();
        let mut accepted = 0_usize;
        while accepted < maximum {
            reap_workers(&mut workers);
            while workers.len() >= worker_limit {
                thread::sleep(Duration::from_millis(1));
                reap_workers(&mut workers);
            }
            let (mut stream, _) = match self.socket.accept() {
                Ok(connection) => connection,
                Err(error) => {
                    join_workers(workers);
                    return Err(error.into());
                }
            };
            accepted += 1;
            let backend = Arc::clone(&self.backend);
            let limits = self.limits;
            let worker = thread::Builder::new()
                .name("asb-control-connection".into())
                .spawn(move || {
                    let _ = Self::serve_stream_with(&backend, limits, &mut stream);
                });
            match worker {
                Ok(worker) => workers.push(worker),
                Err(error) => {
                    join_workers(workers);
                    return Err(EndpointError::Io(error));
                }
            }
        }
        join_workers(workers);
        Ok(())
    }

    fn serve_stream(&self, stream: &mut UnixStream) -> Result<(), EndpointError> {
        Self::serve_stream_with(&self.backend, self.limits, stream)
    }

    fn serve_stream_with(
        backend: &B,
        limits: ControlLimits,
        stream: &mut UnixStream,
    ) -> Result<(), EndpointError> {
        let mut session = ControlSession::new(limits)?;
        let ingress = RequestDeadline::start(limits.max_timeout_ms)?;
        let first: ControlRequest = read_frame_until(stream, limits, ingress)?;
        let (_, effective) = session.negotiate_request(&first)?;
        let deadline = ingress.tighten(first.timeout_ms)?;
        let negotiated = Negotiated {
            version: CONTROL_V1,
            limits: effective,
            runner_instance_id: backend.runner_instance_id().to_owned(),
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
            let admission = session.admit_with_deadline(&request, ingress)?;
            let id = admission.id;
            let deadline = admission.deadline();
            let outcome = backend.execute(&request.call, deadline);
            let response = match outcome {
                Ok(result) => {
                    deadline.check()?;
                    result.validate_for_call(&request.call, effective)?;
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

fn reap_workers(workers: &mut Vec<thread::JoinHandle<()>>) {
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

fn join_workers(workers: Vec<thread::JoinHandle<()>>) {
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
        let limits = limits.validate()?;
        let mut stream = UnixStream::connect(path)?;
        let request = ControlRequest {
            jsonrpc: crate::JSONRPC_VERSION.into(),
            id: RequestId(1),
            timeout_ms: limits.max_timeout_ms,
            call: ControlCall::Negotiate(crate::NegotiateParams {
                versions: [CONTROL_V1].into_iter().collect(),
                limits,
            }),
        };
        let deadline = RequestDeadline::start(request.timeout_ms)?;
        write_frame_until(&mut stream, &request, limits, deadline)?;
        let response: ControlResponse = read_frame_until(&mut stream, limits, deadline)?;
        response.validate()?;
        if response.id() != request.id || response.error().is_some() {
            return Err(EndpointError::UnexpectedResponse);
        }
        let Some(ControlSuccess::Negotiated(negotiated)) = response.into_result() else {
            return Err(EndpointError::UnexpectedResponse);
        };
        if negotiated.version != CONTROL_V1
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

    /// Execute one typed call and return its validated public response.
    pub fn call(
        &mut self,
        call: ControlCall,
        timeout_ms: u64,
    ) -> Result<ControlResponse, EndpointError> {
        if matches!(call, ControlCall::Negotiate(_)) {
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
            timeout_ms,
            call: call.clone(),
        };
        crate::validate_request(&request, self.limits)?;
        let deadline = RequestDeadline::start(timeout_ms)?;
        write_frame_until(&mut self.stream, &request, self.limits, deadline)?;
        let response: ControlResponse = read_frame_until(&mut self.stream, self.limits, deadline)?;
        response.validate()?;
        if response.id() != id {
            return Err(EndpointError::UnexpectedResponse);
        }
        match response.result() {
            Some(ControlSuccess::Operation(result)) => {
                result.validate_for_call(&call, self.limits)?;
            }
            None if response.error().is_some() => {}
            _ => return Err(EndpointError::UnexpectedResponse),
        }
        Ok(response)
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
