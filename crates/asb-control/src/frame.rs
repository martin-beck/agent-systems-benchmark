// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Length-prefixed framing that never allocates from an untrusted length first.

use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;

use serde::Serialize;
use serde::de::DeserializeOwned;
use thiserror::Error;

use crate::{ControlLimits, ProtocolError, RequestDeadline};

/// Decode one four-byte big-endian length-prefixed JSON frame.
pub fn read_frame<T: DeserializeOwned>(
    input: &mut impl Read,
    limits: ControlLimits,
) -> Result<T, FrameError> {
    let limits = limits.validate()?;
    let mut prefix = [0_u8; 4];
    input.read_exact(&mut prefix).map_err(classify_read)?;
    let length = u32::from_be_bytes(prefix);
    if length == 0 || length > limits.max_frame_bytes {
        return Err(FrameError::InvalidLength {
            length,
            maximum: limits.max_frame_bytes,
        });
    }
    let length = usize::try_from(length).map_err(|_| FrameError::LengthOverflow)?;
    let mut body = vec![0_u8; length];
    input.read_exact(&mut body).map_err(classify_read)?;
    serde_json::from_slice(&body).map_err(FrameError::MalformedJson)
}

/// Encode one four-byte big-endian length-prefixed JSON frame.
pub fn write_frame<T: Serialize>(
    output: &mut impl Write,
    value: &T,
    limits: ControlLimits,
) -> Result<(), FrameError> {
    let limits = limits.validate()?;
    let body = serde_json::to_vec(value).map_err(FrameError::MalformedJson)?;
    let length = u32::try_from(body.len()).map_err(|_| FrameError::LengthOverflow)?;
    if length == 0 || length > limits.max_frame_bytes {
        return Err(FrameError::InvalidLength {
            length,
            maximum: limits.max_frame_bytes,
        });
    }
    output.write_all(&length.to_be_bytes())?;
    output.write_all(&body)?;
    output.flush()?;
    Ok(())
}

/// Decode a Unix-stream frame under one absolute monotonic budget.
pub fn read_frame_until<T: DeserializeOwned>(
    input: &mut UnixStream,
    limits: ControlLimits,
    deadline: RequestDeadline,
) -> Result<T, FrameError> {
    let limits = limits.validate()?;
    let mut prefix = [0_u8; 4];
    read_exact_until(input, &mut prefix, deadline, true)?;
    let length = u32::from_be_bytes(prefix);
    if length == 0 || length > limits.max_frame_bytes {
        return Err(FrameError::InvalidLength {
            length,
            maximum: limits.max_frame_bytes,
        });
    }
    let length = usize::try_from(length).map_err(|_| FrameError::LengthOverflow)?;
    let mut body = vec![0_u8; length];
    read_exact_until(input, &mut body, deadline, false)?;
    serde_json::from_slice(&body).map_err(FrameError::MalformedJson)
}

/// Encode a Unix-stream frame under one absolute monotonic budget.
pub fn write_frame_until<T: Serialize>(
    output: &mut UnixStream,
    value: &T,
    limits: ControlLimits,
    deadline: RequestDeadline,
) -> Result<(), FrameError> {
    let limits = limits.validate()?;
    let body = serde_json::to_vec(value).map_err(FrameError::MalformedJson)?;
    let length = u32::try_from(body.len()).map_err(|_| FrameError::LengthOverflow)?;
    if length == 0 || length > limits.max_frame_bytes {
        return Err(FrameError::InvalidLength {
            length,
            maximum: limits.max_frame_bytes,
        });
    }
    write_all_until(output, &length.to_be_bytes(), deadline)?;
    write_all_until(output, &body, deadline)
}

fn read_exact_until(
    stream: &mut UnixStream,
    mut bytes: &mut [u8],
    deadline: RequestDeadline,
    allow_clean_close: bool,
) -> Result<(), FrameError> {
    let initial = bytes.len();
    while !bytes.is_empty() {
        stream.set_read_timeout(Some(
            deadline
                .remaining()
                .map_err(|_| FrameError::DeadlineExceeded)?,
        ))?;
        match stream.read(bytes) {
            Ok(0) if allow_clean_close && bytes.len() == initial => return Err(FrameError::Closed),
            Ok(0) => return Err(FrameError::Truncated),
            Ok(count) => bytes = &mut bytes[count..],
            Err(error) => return Err(classify_read(error)),
        }
    }
    Ok(())
}

fn write_all_until(
    stream: &mut UnixStream,
    mut bytes: &[u8],
    deadline: RequestDeadline,
) -> Result<(), FrameError> {
    while !bytes.is_empty() {
        stream.set_write_timeout(Some(
            deadline
                .remaining()
                .map_err(|_| FrameError::DeadlineExceeded)?,
        ))?;
        match stream.write(bytes) {
            Ok(0) => return Err(FrameError::Truncated),
            Ok(count) => bytes = &bytes[count..],
            Err(error) => return Err(classify_read(error)),
        }
    }
    Ok(())
}

fn classify_read(error: io::Error) -> FrameError {
    if error.kind() == io::ErrorKind::UnexpectedEof {
        FrameError::Truncated
    } else if matches!(
        error.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    ) {
        FrameError::DeadlineExceeded
    } else {
        FrameError::Io(error)
    }
}

/// Bounded frame processing failure.
#[derive(Debug, Error)]
pub enum FrameError {
    /// Framing I/O failed.
    #[error("control frame I/O failed")]
    Io(#[from] io::Error),
    /// The peer closed cleanly between frames.
    #[error("control connection closed")]
    Closed,
    /// The peer stopped in the middle of a prefix or body.
    #[error("control frame is truncated")]
    Truncated,
    /// Configured socket deadline expired.
    #[error("control frame deadline exceeded")]
    DeadlineExceeded,
    /// Length was zero or above the negotiated maximum.
    #[error("control frame length {length} is outside 1..={maximum}")]
    InvalidLength {
        /// Untrusted advertised length.
        length: u32,
        /// Effective maximum.
        maximum: u32,
    },
    /// Host representation cannot contain the advertised length.
    #[error("control frame length cannot be represented")]
    LengthOverflow,
    /// Body is not the expected JSON shape.
    #[error("control frame contains malformed JSON")]
    MalformedJson(#[from] serde_json::Error),
    /// Negotiated limits were invalid.
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
}
