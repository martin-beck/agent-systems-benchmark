// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Private, one-shot transport for strict replay launch handoff.

use std::fs;
use std::io::{self, BufRead, BufReader};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

const MAX_GENERATION_BYTES: usize = 128;
const MAX_HANDSHAKE_BYTES: usize = MAX_GENERATION_BYTES + 1;

/// Failure while creating or consuming a replay relay.
#[derive(Debug)]
pub enum RelayError {
    /// The relay path or socket operation failed.
    Io(io::Error),
    /// The generation identity is empty, too long, or contains unsafe bytes.
    InvalidGeneration,
    /// The peer presented a generation from another launch.
    StaleGeneration,
    /// This launch already handed out its sole peer connection.
    Duplicate,
    /// The peer did not provide a bounded newline-terminated handshake.
    InvalidHandshake,
    /// The handoff descriptor does not describe this private relay.
    InvalidHandoff,
}

impl PartialEq for RelayError {
    fn eq(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (Self::InvalidGeneration, Self::InvalidGeneration)
                | (Self::StaleGeneration, Self::StaleGeneration)
                | (Self::Duplicate, Self::Duplicate)
                | (Self::InvalidHandshake, Self::InvalidHandshake)
                | (Self::InvalidHandoff, Self::InvalidHandoff)
        )
    }
}

impl Eq for RelayError {}

impl From<io::Error> for RelayError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl std::fmt::Display for RelayError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(_) => formatter.write_str("replay relay I/O failed"),
            Self::InvalidGeneration => formatter.write_str("invalid replay generation"),
            Self::StaleGeneration => formatter.write_str("stale replay generation"),
            Self::Duplicate => formatter.write_str("duplicate replay relay connection"),
            Self::InvalidHandshake => formatter.write_str("invalid replay relay handshake"),
            Self::InvalidHandoff => formatter.write_str("invalid replay relay handoff"),
        }
    }
}

impl std::error::Error for RelayError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

/// A private, generation-authenticated, one-shot Unix relay.
///
/// The relay does not open a network namespace or weaken `NetworkPolicy::Deny`.
/// Its socket can be bind-mounted into a launch namespace by a future adapter
/// integration. Dropping it removes the socket path, preventing stale reuse.
#[derive(Debug)]
pub struct ReplayRelay {
    listener: UnixListener,
    root: PathBuf,
    socket_path: PathBuf,
    generation: String,
    consumed: bool,
}

/// Authenticated metadata for passing one relay into an isolated launch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplayRelayHandoff {
    socket_path: PathBuf,
    child_endpoint: PathBuf,
    generation: String,
}

impl ReplayRelayHandoff {
    /// Host-side private socket path.
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Path at which the launch namespace should expose the relay socket.
    pub fn child_endpoint(&self) -> &Path {
        &self.child_endpoint
    }

    /// Launch generation used for peer authentication.
    pub fn generation(&self) -> &str {
        &self.generation
    }

    /// Validate descriptor shape and private socket ownership.
    pub fn validate(&self) -> Result<(), RelayError> {
        if !valid_generation(&self.generation)
            || !self.socket_path.is_absolute()
            || !self.child_endpoint.is_absolute()
            || self
                .child_endpoint
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
            || !self.socket_path.file_name().is_some_and(|name| {
                name.to_string_lossy() == format!("asb-replay-{}.sock", self.generation)
            })
            || !fs::metadata(&self.socket_path).is_ok_and(|metadata| {
                metadata.file_type().is_socket() && metadata.permissions().mode() & 0o777 == 0o600
            })
        {
            return Err(RelayError::InvalidHandoff);
        }
        Ok(())
    }
}

impl ReplayRelay {
    /// Bind a private relay socket below an existing launch-owned directory.
    pub fn bind(root: &Path, generation: impl Into<String>) -> Result<Self, RelayError> {
        let generation = generation.into();
        if !valid_generation(&generation) || !root.is_absolute() || !root.is_dir() {
            return Err(RelayError::InvalidGeneration);
        }
        let root = fs::canonicalize(root).map_err(RelayError::Io)?;
        let socket_path = root.join(format!("asb-replay-{generation}.sock"));
        let listener = UnixListener::bind(&socket_path)?;
        fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))?;
        Ok(Self {
            listener,
            root,
            socket_path,
            generation,
            consumed: false,
        })
    }

    /// Return the filesystem endpoint to expose to the isolated child.
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Return the authenticated launch generation.
    pub fn generation(&self) -> &str {
        &self.generation
    }

    /// Describe this relay for a future isolated child launch.
    pub fn handoff(
        &self,
        child_endpoint: impl Into<PathBuf>,
    ) -> Result<ReplayRelayHandoff, RelayError> {
        let handoff = ReplayRelayHandoff {
            socket_path: self.socket_path.clone(),
            child_endpoint: child_endpoint.into(),
            generation: self.generation.clone(),
        };
        if !handoff.socket_path.starts_with(&self.root) {
            return Err(RelayError::InvalidHandoff);
        }
        handoff.validate()?;
        Ok(handoff)
    }

    /// Accept exactly one peer presenting this launch's generation.
    pub fn accept_authenticated(&mut self) -> Result<UnixStream, RelayError> {
        if self.consumed {
            return Err(RelayError::Duplicate);
        }
        let (stream, _) = self.listener.accept()?;
        let mut handshake = Vec::with_capacity(MAX_HANDSHAKE_BYTES);
        let mut reader = BufReader::new(stream);
        let size = reader
            .read_until(b'\n', &mut handshake)
            .map_err(RelayError::Io)?;
        if size == 0 || size > MAX_HANDSHAKE_BYTES || handshake.last() != Some(&b'\n') {
            return Err(RelayError::InvalidHandshake);
        }
        handshake.pop();
        if handshake == self.generation.as_bytes() {
            self.consumed = true;
            Ok(reader.into_inner())
        } else {
            Err(RelayError::StaleGeneration)
        }
    }
}

impl Drop for ReplayRelay {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.socket_path);
    }
}

fn valid_generation(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_GENERATION_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn root() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "asb-relay-test-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn authenticated_peer_is_one_shot_and_socket_is_removed() {
        let directory = root();
        let mut relay = ReplayRelay::bind(&directory, "generation-1").unwrap();
        let path = relay.socket_path().to_owned();
        let peer = UnixStream::connect(&path).unwrap();
        let worker = std::thread::spawn(move || {
            let mut peer = peer;
            peer.write_all(b"generation-1\n").unwrap();
        });
        let _accepted = relay.accept_authenticated().unwrap();
        worker.join().unwrap();
        assert!(matches!(
            relay.accept_authenticated(),
            Err(RelayError::Duplicate)
        ));
        drop(relay);
        assert!(!path.exists());
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn stale_and_malformed_peers_do_not_consume_generation() {
        let directory = root();
        let mut relay = ReplayRelay::bind(&directory, "current").unwrap();
        let stale = UnixStream::connect(relay.socket_path()).unwrap();
        let worker = std::thread::spawn(move || {
            let mut stale = stale;
            stale.write_all(b"old\n").unwrap();
        });
        assert!(matches!(
            relay.accept_authenticated(),
            Err(RelayError::StaleGeneration)
        ));
        worker.join().unwrap();
        let malformed = UnixStream::connect(relay.socket_path()).unwrap();
        let worker = std::thread::spawn(move || {
            let mut malformed = malformed;
            malformed.write_all(b"current").unwrap();
        });
        assert!(matches!(
            relay.accept_authenticated(),
            Err(RelayError::InvalidHandshake)
        ));
        worker.join().unwrap();
        assert!(!relay.consumed);
        drop(relay);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn invalid_generation_and_unrelated_socket_are_rejected() {
        let directory = root();
        assert_eq!(
            ReplayRelay::bind(&directory, "../escape").unwrap_err(),
            RelayError::InvalidGeneration
        );
        assert_eq!(
            ReplayRelay::bind(Path::new("relative"), "ok").unwrap_err(),
            RelayError::InvalidGeneration
        );
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn handoff_is_generation_fenced_and_private() {
        let directory = root();
        let relay = ReplayRelay::bind(&directory, "generation-2").unwrap();
        let handoff = relay
            .handoff(PathBuf::from("/run/asb/replay.sock"))
            .unwrap();
        assert_eq!(handoff.generation(), "generation-2");
        assert_eq!(handoff.child_endpoint(), Path::new("/run/asb/replay.sock"));
        assert!(handoff.validate().is_ok());
        let mut stale = handoff.clone();
        stale.generation = "stale".into();
        assert_eq!(stale.validate(), Err(RelayError::InvalidHandoff));
        drop(relay);
        assert_eq!(handoff.validate(), Err(RelayError::InvalidHandoff));
        let _ = fs::remove_dir_all(directory);
    }
}
