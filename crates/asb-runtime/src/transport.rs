// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Runtime-owned, one-shot Unix transport for bounded replay envelopes.

use asb_core::replay_transport::{
    MAX_PAYLOAD, ReplayRequest, ReplayResponse, ReplayTransportError,
};
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

#[derive(Debug)]
/// Runtime-owned one-shot server which authenticates one launch generation.
pub struct ReplayTransportIssuer {
    listener: UnixListener,
    path: PathBuf,
    generation: String,
    consumed: bool,
}

impl ReplayTransportIssuer {
    /// Bind a private Unix endpoint for one generation.
    pub fn bind(
        path: impl Into<PathBuf>,
        generation: String,
    ) -> Result<Self, ReplayTransportError> {
        ReplayRequest::new(generation.clone(), "bind-check".into(), Vec::new())?;
        let path = path.into();
        let listener =
            UnixListener::bind(&path).map_err(|_| ReplayTransportError::InvalidEnvelope)?;
        Ok(Self {
            listener,
            path,
            generation,
            consumed: false,
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
        let request = read_request(&mut stream)?;
        if request.generation != self.generation {
            return Err(ReplayTransportError::StaleGeneration);
        }
        self.consumed = true;
        Ok((request, stream))
    }
    /// Send one response on the authenticated stream.
    pub fn respond(
        &self,
        mut stream: UnixStream,
        response: ReplayResponse,
    ) -> Result<(), ReplayTransportError> {
        if response.generation != self.generation {
            return Err(ReplayTransportError::StaleGeneration);
        }
        write_frame(&mut stream, &response.encode()?)
    }
}

/// One-shot client issued for one runtime launch generation.
pub struct ReplayTransportClient {
    stream: UnixStream,
    generation: String,
    used: bool,
}
impl ReplayTransportClient {
    /// Connect to a runtime-issued endpoint.
    pub fn connect(path: &Path, generation: String) -> Result<Self, ReplayTransportError> {
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
        request: ReplayRequest,
    ) -> Result<ReplayResponse, ReplayTransportError> {
        if self.used || request.generation != self.generation {
            return Err(if self.used {
                ReplayTransportError::DuplicateRequest
            } else {
                ReplayTransportError::StaleGeneration
            });
        }
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
    #[test]
    fn one_shot_authenticates_and_replies() {
        let path = std::env::temp_dir().join(format!("asb-transport-{}.sock", std::process::id()));
        let mut issuer = ReplayTransportIssuer::bind(&path, "g1".into()).unwrap();
        let p = path.clone();
        let t = thread::spawn(move || {
            let mut c = ReplayTransportClient::connect(&p, "g1".into()).unwrap();
            c.request(ReplayRequest::new("g1".into(), "r1".into(), b"x".to_vec()).unwrap())
                .unwrap()
        });
        let (request, stream) = issuer.accept_once().unwrap();
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
            issuer.accept_once(),
            Err(ReplayTransportError::DuplicateRequest)
        ));
    }

    #[test]
    fn stale_generation_is_rejected_before_response() {
        let path =
            std::env::temp_dir().join(format!("asb-transport-stale-{}.sock", std::process::id()));
        let issuer = ReplayTransportIssuer::bind(&path, "current".into()).unwrap();
        drop(issuer);
        assert!(matches!(
            ReplayTransportIssuer::bind(&path, "../escape".into()),
            Err(ReplayTransportError::InvalidEnvelope)
        ));
    }
}
