// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Bounded authenticated envelopes for the runtime-to-CLI replay transport.

const MAGIC: &[u8; 4] = b"ASBR";
const VERSION: u8 = 1;
/// Maximum generation identity bytes.
pub const MAX_GENERATION: usize = 128;
/// Maximum request identity bytes.
pub const MAX_REQUEST_ID: usize = 128;
/// Maximum envelope payload bytes.
pub const MAX_PAYLOAD: usize = 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
/// Authenticated request envelope.
pub struct ReplayRequest {
    /// Runtime launch generation.
    pub generation: String,
    /// One-shot request identity.
    pub request_id: String,
    /// Bounded opaque payload.
    pub payload: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// Authenticated response envelope.
pub struct ReplayResponse {
    /// Runtime launch generation.
    pub generation: String,
    /// Request identity being answered.
    pub request_id: String,
    /// Bounded opaque payload.
    pub payload: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// Fail-closed transport validation errors.
pub enum ReplayTransportError {
    /// Envelope bytes or fields are malformed.
    InvalidEnvelope,
    /// Envelope belongs to another launch.
    StaleGeneration,
    /// One-shot request was already consumed.
    DuplicateRequest,
}

impl ReplayRequest {
    /// Construct and validate a request envelope.
    pub fn new(
        generation: String,
        request_id: String,
        payload: Vec<u8>,
    ) -> Result<Self, ReplayTransportError> {
        let value = Self {
            generation,
            request_id,
            payload,
        };
        value.validate()?;
        Ok(value)
    }
    /// Validate envelope identity and bounds.
    pub fn validate(&self) -> Result<(), ReplayTransportError> {
        validate_parts(&self.generation, &self.request_id, self.payload.len())
    }
    /// Encode the bounded wire representation.
    pub fn encode(&self) -> Result<Vec<u8>, ReplayTransportError> {
        self.validate()?;
        encode(&self.generation, &self.request_id, &self.payload)
    }
    /// Decode and validate a wire representation.
    pub fn decode(bytes: &[u8]) -> Result<Self, ReplayTransportError> {
        let (generation, request_id, payload) = decode(bytes)?;
        Self::new(generation, request_id, payload)
    }
}

impl ReplayResponse {
    /// Construct and validate a response envelope.
    pub fn new(
        generation: String,
        request_id: String,
        payload: Vec<u8>,
    ) -> Result<Self, ReplayTransportError> {
        let value = Self {
            generation,
            request_id,
            payload,
        };
        value.validate()?;
        Ok(value)
    }
    /// Validate envelope identity and bounds.
    pub fn validate(&self) -> Result<(), ReplayTransportError> {
        validate_parts(&self.generation, &self.request_id, self.payload.len())
    }
    /// Encode the bounded wire representation.
    pub fn encode(&self) -> Result<Vec<u8>, ReplayTransportError> {
        self.validate()?;
        encode(&self.generation, &self.request_id, &self.payload)
    }
    /// Decode and validate a wire representation.
    pub fn decode(bytes: &[u8]) -> Result<Self, ReplayTransportError> {
        let (generation, request_id, payload) = decode(bytes)?;
        Self::new(generation, request_id, payload)
    }
}

fn validate_parts(
    generation: &str,
    request_id: &str,
    payload: usize,
) -> Result<(), ReplayTransportError> {
    if generation.is_empty()
        || generation.len() > MAX_GENERATION
        || !generation
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        || request_id.is_empty()
        || request_id.len() > MAX_REQUEST_ID
        || !request_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        || payload > MAX_PAYLOAD
    {
        return Err(ReplayTransportError::InvalidEnvelope);
    }
    Ok(())
}

fn encode(
    generation: &str,
    request_id: &str,
    payload: &[u8],
) -> Result<Vec<u8>, ReplayTransportError> {
    let gl = u16::try_from(generation.len()).map_err(|_| ReplayTransportError::InvalidEnvelope)?;
    let il = u16::try_from(request_id.len()).map_err(|_| ReplayTransportError::InvalidEnvelope)?;
    let pl = u32::try_from(payload.len()).map_err(|_| ReplayTransportError::InvalidEnvelope)?;
    let mut out = Vec::with_capacity(17 + generation.len() + request_id.len() + payload.len());
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    out.extend_from_slice(&gl.to_be_bytes());
    out.extend_from_slice(&il.to_be_bytes());
    out.extend_from_slice(&pl.to_be_bytes());
    out.extend_from_slice(generation.as_bytes());
    out.extend_from_slice(request_id.as_bytes());
    out.extend_from_slice(payload);
    Ok(out)
}

fn decode(bytes: &[u8]) -> Result<(String, String, Vec<u8>), ReplayTransportError> {
    if bytes.len() < 13 || &bytes[..4] != MAGIC || bytes[4] != VERSION {
        return Err(ReplayTransportError::InvalidEnvelope);
    }
    let gl = u16::from_be_bytes([bytes[5], bytes[6]]) as usize;
    let il = u16::from_be_bytes([bytes[7], bytes[8]]) as usize;
    let pl = u32::from_be_bytes([bytes[9], bytes[10], bytes[11], bytes[12]]) as usize;
    let total = 13usize
        .checked_add(gl)
        .and_then(|v| v.checked_add(il))
        .and_then(|v| v.checked_add(pl))
        .ok_or(ReplayTransportError::InvalidEnvelope)?;
    if total != bytes.len() || gl > MAX_GENERATION || il > MAX_REQUEST_ID || pl > MAX_PAYLOAD {
        return Err(ReplayTransportError::InvalidEnvelope);
    }
    let generation = String::from_utf8(bytes[13..13 + gl].to_vec())
        .map_err(|_| ReplayTransportError::InvalidEnvelope)?;
    let request_id = String::from_utf8(bytes[13 + gl..13 + gl + il].to_vec())
        .map_err(|_| ReplayTransportError::InvalidEnvelope)?;
    validate_parts(&generation, &request_id, pl)?;
    Ok((generation, request_id, bytes[13 + gl + il..].to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn round_trip_is_bounded() {
        let r = ReplayRequest::new("g1".into(), "r1".into(), b"cassette".to_vec()).unwrap();
        assert_eq!(ReplayRequest::decode(&r.encode().unwrap()), Ok(r));
    }
    #[test]
    fn rejects_malformed_stale_shape_and_oversize() {
        assert_eq!(
            ReplayRequest::new("../g".into(), "r".into(), vec![]),
            Err(ReplayTransportError::InvalidEnvelope)
        );
        assert_eq!(
            ReplayResponse::decode(b"bad"),
            Err(ReplayTransportError::InvalidEnvelope)
        );
        assert_eq!(
            ReplayRequest::new("g".into(), "r".into(), vec![0; MAX_PAYLOAD + 1]),
            Err(ReplayTransportError::InvalidEnvelope)
        );
    }

    #[test]
    fn rejects_wire_headers_lengths_and_invalid_utf8() {
        let request = ReplayRequest::new("g".into(), "r".into(), b"x".to_vec()).unwrap();
        let encoded = request.encode().unwrap();
        for malformed in [
            Vec::new(),
            b"bad".to_vec(),
            {
                let mut bytes = encoded.clone();
                bytes[0] = b'X';
                bytes
            },
            {
                let mut bytes = encoded.clone();
                bytes[4] = VERSION + 1;
                bytes
            },
            {
                let mut bytes = encoded.clone();
                bytes[5] = 0xff;
                bytes[6] = 0xff;
                bytes
            },
            {
                let mut bytes = encoded.clone();
                bytes[12] = 2;
                bytes
            },
            {
                let mut bytes = encoded.clone();
                bytes[13] = 0xff;
                bytes
            },
        ] {
            assert_eq!(
                ReplayRequest::decode(&malformed),
                Err(ReplayTransportError::InvalidEnvelope)
            );
        }
    }

    #[test]
    fn response_round_trip_and_field_validation() {
        let response = ReplayResponse::new("g1".into(), "r1".into(), b"ok".to_vec()).unwrap();
        assert_eq!(
            ReplayResponse::decode(&response.encode().unwrap()),
            Ok(response)
        );
        assert_eq!(
            ReplayResponse::new("g".into(), "bad.id".into(), vec![]),
            Err(ReplayTransportError::InvalidEnvelope)
        );
        assert_eq!(
            ReplayRequest::new("g".into(), "r".into(), vec![])
                .unwrap()
                .encode()
                .unwrap()
                .len(),
            15
        );
    }
}
