// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Runtime-owned, one-shot Unix transport for bounded replay envelopes.

use asb_core::replay_transport::{
    MAX_PAYLOAD, ReplayRequest, ReplayResponse, ReplayTransportError,
};
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const IO_TIMEOUT: Duration = Duration::from_secs(5);
static OPERATION_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
/// Runtime-owned one-shot server which authenticates one launch generation.
pub struct ReplayTransportIssuer {
    listener: UnixListener,
    path: PathBuf,
    generation: String,
    consumed: bool,
    accepted_request_id: Option<String>,
    responded: bool,
}

impl ReplayTransportIssuer {
    /// Bind a private Unix endpoint for one generation.
    pub fn bind(
        path: impl Into<PathBuf>,
        generation: String,
    ) -> Result<Self, ReplayTransportError> {
        ReplayRequest::new(generation.clone(), "bind-check".into(), Vec::new())?;
        let path = path.into();
        if !path.is_absolute()
            || path.parent().is_none_or(|parent| {
                std::fs::symlink_metadata(parent).is_err()
                    || std::fs::symlink_metadata(parent).is_ok_and(|m| m.file_type().is_symlink())
                    || std::fs::metadata(parent).is_err_and(|_| true)
                    || std::fs::metadata(parent)
                        .is_ok_and(|m| !m.is_dir() || m.permissions().mode() & 0o777 != 0o700)
            })
        {
            return Err(ReplayTransportError::InvalidEnvelope);
        }
        let listener =
            UnixListener::bind(&path).map_err(|_| ReplayTransportError::InvalidEnvelope)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .map_err(|_| ReplayTransportError::InvalidEnvelope)?;
        Ok(Self {
            listener,
            path,
            generation,
            consumed: false,
            accepted_request_id: None,
            responded: false,
        })
    }
    /// Return the private endpoint path.
    pub fn path(&self) -> &Path {
        &self.path
    }
    /// Accept and authenticate exactly one request.
    pub fn accept_once(&mut self) -> Result<(ReplayRequest, UnixStream), ReplayTransportError> {
        if self.consumed {
            return Err(ReplayTransportError::DuplicateRequest);
        }
        let (mut stream, _) = self
            .listener
            .accept()
            .map_err(|_| ReplayTransportError::InvalidEnvelope)?;
        stream
            .set_read_timeout(Some(IO_TIMEOUT))
            .and_then(|_| stream.set_write_timeout(Some(IO_TIMEOUT)))
            .map_err(|_| ReplayTransportError::InvalidEnvelope)?;
        let request = read_request(&mut stream)?;
        if request.generation != self.generation {
            return Err(ReplayTransportError::StaleGeneration);
        }
        self.consumed = true;
        self.accepted_request_id = Some(request.request_id.clone());
        Ok((request, stream))
    }
    /// Send one response on the authenticated stream.
    pub fn respond(
        &mut self,
        mut stream: UnixStream,
        response: ReplayResponse,
    ) -> Result<(), ReplayTransportError> {
        if self.responded {
            return Err(ReplayTransportError::DuplicateRequest);
        }
        if response.generation != self.generation
            || self.accepted_request_id.as_deref() != Some(response.request_id.as_str())
        {
            return Err(ReplayTransportError::StaleGeneration);
        }
        write_frame(&mut stream, &response.encode()?)?;
        self.responded = true;
        Ok(())
    }
}

/// One-shot client issued for one runtime launch generation.
pub struct ReplayTransportClient {
    stream: UnixStream,
    generation: String,
    used: bool,
}

/// Runtime-issued, one-shot operation context for a strict-replay dispatch.
///
/// The context owns both ends of a private authenticated transport.  A caller
/// can submit exactly one bounded request through it; the runtime verifies the
/// generation and request identity before the response is returned.
pub struct ReplayOperation {
    root: PathBuf,
    issuer: ReplayTransportIssuer,
    client: ReplayTransportClient,
}

impl ReplayOperation {
    /// Create a private operation endpoint for one runtime launch generation.
    pub(crate) fn issue(generation: String) -> Result<Self, ReplayTransportError> {
        let root = std::env::temp_dir().join(format!(
            "asb-replay-operation-{}-{}",
            std::process::id(),
            OPERATION_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).map_err(|_| ReplayTransportError::InvalidEnvelope)?;
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| ReplayTransportError::InvalidEnvelope)?;
        let issuer = ReplayTransportIssuer::bind(root.join("operation.sock"), generation)?;
        let client = ReplayTransportClient::issue(&issuer)?;
        Ok(Self {
            root,
            issuer,
            client,
        })
    }

    /// Dispatch one request to a bounded offline handler and return its response.
    pub fn dispatch<F>(
        &mut self,
        request_id: String,
        payload: Vec<u8>,
        handler: F,
    ) -> Result<Vec<u8>, ReplayTransportError>
    where
        F: FnOnce(Vec<u8>) -> Result<Vec<u8>, ReplayTransportError> + Send,
    {
        std::thread::scope(|scope| {
            let issuer = &mut self.issuer;
            let client = &mut self.client;
            let server = scope.spawn(move || {
                let (request, stream) = issuer.accept_once()?;
                let response_payload = handler(request.payload)?;
                let response =
                    ReplayResponse::new(request.generation, request.request_id, response_payload)?;
                issuer.respond(stream, response)?;
                Ok::<(), ReplayTransportError>(())
            });
            let response = client.request(request_id, payload);
            let server_result = server
                .join()
                .map_err(|_| ReplayTransportError::InvalidEnvelope)?;
            server_result?;
            response.map(|value| value.payload)
        })
    }
}

impl Drop for ReplayOperation {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.root);
    }
}
impl ReplayTransportClient {
    /// Issue a client from the runtime-owned issuer.
    pub fn issue(issuer: &ReplayTransportIssuer) -> Result<Self, ReplayTransportError> {
        let stream =
            UnixStream::connect(&issuer.path).map_err(|_| ReplayTransportError::InvalidEnvelope)?;
        stream
            .set_read_timeout(Some(IO_TIMEOUT))
            .and_then(|_| stream.set_write_timeout(Some(IO_TIMEOUT)))
            .map_err(|_| ReplayTransportError::InvalidEnvelope)?;
        Ok(Self {
            stream,
            generation: issuer.generation.clone(),
            used: false,
        })
    }
    #[cfg(test)]
    fn connect(path: &Path, generation: String) -> Result<Self, ReplayTransportError> {
        let stream =
            UnixStream::connect(path).map_err(|_| ReplayTransportError::InvalidEnvelope)?;
        Ok(Self {
            stream,
            generation,
            used: false,
        })
    }
    /// Send one request and validate its matching response.
    pub fn request(
        &mut self,
        request_id: String,
        payload: Vec<u8>,
    ) -> Result<ReplayResponse, ReplayTransportError> {
        if self.used {
            return Err(ReplayTransportError::DuplicateRequest);
        }
        let request = ReplayRequest::new(self.generation.clone(), request_id, payload)?;
        write_frame(&mut self.stream, &request.encode()?)?;
        let response = ReplayResponse::decode(&read_frame(&mut self.stream)?)?;
        if response.generation != self.generation || response.request_id != request.request_id {
            return Err(ReplayTransportError::StaleGeneration);
        }
        self.used = true;
        Ok(response)
    }
}

fn write_frame(stream: &mut UnixStream, bytes: &[u8]) -> Result<(), ReplayTransportError> {
    if bytes.len() > MAX_PAYLOAD + 256 {
        return Err(ReplayTransportError::InvalidEnvelope);
    }
    stream
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .and_then(|_| stream.write_all(bytes))
        .map_err(|_| ReplayTransportError::InvalidEnvelope)
}
fn read_frame(stream: &mut UnixStream) -> Result<Vec<u8>, ReplayTransportError> {
    let mut len = [0; 4];
    stream
        .read_exact(&mut len)
        .map_err(|_| ReplayTransportError::InvalidEnvelope)?;
    let n = u32::from_be_bytes(len) as usize;
    if n > MAX_PAYLOAD + 256 {
        return Err(ReplayTransportError::InvalidEnvelope);
    }
    let mut bytes = vec![0; n];
    stream
        .read_exact(&mut bytes)
        .map_err(|_| ReplayTransportError::InvalidEnvelope)?;
    Ok(bytes)
}
fn read_request(stream: &mut UnixStream) -> Result<ReplayRequest, ReplayTransportError> {
    ReplayRequest::decode(&read_frame(stream)?)
}

impl Drop for ReplayTransportIssuer {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::thread;
    fn private_path(label: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("asb-transport-{label}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        dir.join("relay.sock")
    }
    #[test]
    fn one_shot_authenticates_and_replies() {
        let path = private_path("roundtrip");
        let mut issuer = ReplayTransportIssuer::bind(&path, "g1".into()).unwrap();
        let mut client = ReplayTransportClient::issue(&issuer).unwrap();
        let t = thread::spawn(move || {
            let response = client.request("r1".into(), b"x".to_vec()).unwrap();
            (client, response)
        });
        let (request, stream) = issuer.accept_once().unwrap();
        assert!(matches!(
            issuer.respond(
                stream.try_clone().unwrap(),
                ReplayResponse::new("g1".into(), "wrong".into(), b"bad".to_vec()).unwrap()
            ),
            Err(ReplayTransportError::StaleGeneration)
        ));
        issuer
            .respond(
                stream,
                ReplayResponse::new(
                    request.generation.clone(),
                    request.request_id.clone(),
                    b"ok".to_vec(),
                )
                .unwrap(),
            )
            .unwrap();
        let (mut client, response) = t.join().unwrap();
        assert_eq!(response.payload, b"ok");
        assert!(matches!(
            client.request("r2".into(), Vec::new()),
            Err(ReplayTransportError::DuplicateRequest)
        ));
        assert!(matches!(
            issuer.respond(
                UnixStream::pair().unwrap().0,
                ReplayResponse::new("g1".into(), "r1".into(), b"again".to_vec()).unwrap()
            ),
            Err(ReplayTransportError::DuplicateRequest)
        ));
        assert!(matches!(
            issuer.accept_once(),
            Err(ReplayTransportError::DuplicateRequest)
        ));
    }

    #[test]
    fn stale_generation_is_rejected_before_response() {
        let path = private_path("stale");
        let issuer = ReplayTransportIssuer::bind(&path, "current".into()).unwrap();
        drop(issuer);
        assert!(matches!(
            ReplayTransportIssuer::bind(&path, "../escape".into()),
            Err(ReplayTransportError::InvalidEnvelope)
        ));
        assert!(matches!(
            ReplayTransportIssuer::bind(
                std::env::temp_dir().join("asb-transport-arbitrary.sock"),
                "valid".into()
            ),
            Err(ReplayTransportError::InvalidEnvelope)
        ));
    }

    #[test]
    fn stale_generation_peer_is_rejected_on_transport() {
        let path = private_path("peer-stale");
        let mut issuer = ReplayTransportIssuer::bind(&path, "current".into()).unwrap();
        let p = path.clone();
        let t = thread::spawn(move || {
            let mut client = ReplayTransportClient::connect(&p, "stale".into()).unwrap();
            client.request("r1".into(), b"x".to_vec())
        });
        assert!(matches!(
            issuer.accept_once(),
            Err(ReplayTransportError::StaleGeneration)
        ));
        assert!(matches!(
            t.join().unwrap(),
            Err(ReplayTransportError::InvalidEnvelope)
        ));
    }

    #[test]
    fn malformed_and_oversized_frames_fail_closed() {
        let (mut writer, mut reader) = UnixStream::pair().unwrap();
        writer.write_all(&3u32.to_be_bytes()).unwrap();
        writer.write_all(b"bad").unwrap();
        assert!(matches!(
            read_request(&mut reader),
            Err(ReplayTransportError::InvalidEnvelope)
        ));

        let (mut writer, mut reader) = UnixStream::pair().unwrap();
        writer
            .write_all(&((MAX_PAYLOAD + 257) as u32).to_be_bytes())
            .unwrap();
        assert!(matches!(
            read_frame(&mut reader),
            Err(ReplayTransportError::InvalidEnvelope)
        ));

        let (mut writer, _reader) = UnixStream::pair().unwrap();
        assert!(matches!(
            write_frame(&mut writer, &vec![0; MAX_PAYLOAD + 257]),
            Err(ReplayTransportError::InvalidEnvelope)
        ));
    }

    #[test]
    fn missing_endpoint_and_unreadable_peer_fail_closed() {
        let missing = std::env::temp_dir().join(format!("asb-missing-{}", std::process::id()));
        assert!(matches!(
            ReplayTransportClient::connect(&missing, "g".into()),
            Err(ReplayTransportError::InvalidEnvelope)
        ));

        let (writer, mut reader) = UnixStream::pair().unwrap();
        drop(writer);
        assert!(matches!(
            read_frame(&mut reader),
            Err(ReplayTransportError::InvalidEnvelope)
        ));
    }

    #[test]
    fn runtime_operation_round_trip_is_single_use_and_bounded() {
        let mut operation = ReplayOperation::issue("generation-1".into()).unwrap();
        let response = operation
            .dispatch("request-1".into(), b"request".to_vec(), |payload| {
                assert_eq!(payload, b"request");
                Ok(b"response".to_vec())
            })
            .unwrap();
        assert_eq!(response, b"response");
        assert!(matches!(
            operation.dispatch("request-2".into(), Vec::new(), |_| Ok(Vec::new())),
            Err(ReplayTransportError::DuplicateRequest)
        ));
    }

    #[test]
    fn runtime_operation_handler_failure_has_no_fallback_response() {
        let mut operation = ReplayOperation::issue("generation-2".into()).unwrap();
        assert!(matches!(
            operation.dispatch("request-1".into(), b"bad".to_vec(), |_| {
                Err(ReplayTransportError::InvalidEnvelope)
            }),
            Err(ReplayTransportError::InvalidEnvelope)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn bind_rejects_non_private_parent() {
        let dir = std::env::temp_dir().join(format!("asb-transport-public-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(matches!(
            ReplayTransportIssuer::bind(dir.join("relay.sock"), "g".into()),
            Err(ReplayTransportError::InvalidEnvelope)
        ));
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let _ = std::fs::remove_dir(&dir);
    }
}
