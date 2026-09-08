// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Owner-only Linux Unix-domain transport.

use std::fs::{self, Permissions};
use std::io;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use rustix::net::sockopt::socket_peercred;
use rustix::process::geteuid;
use thiserror::Error;

use crate::ControlLimits;

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
        let credentials = socket_peercred(stream).map_err(io::Error::from)?;
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
}
