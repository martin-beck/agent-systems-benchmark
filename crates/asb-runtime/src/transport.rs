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
use std::time::Duration;

const IO_TIMEOUT: Duration = Duration::from_secs(5);

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
        let t = thread::spawn(move || client.request("r1".into(), b"x".to_vec()).unwrap());
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
        assert_eq!(t.join().unwrap().payload, b"ok");
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
}
