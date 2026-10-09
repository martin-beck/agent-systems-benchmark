// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Runtime-owned lifecycle for the development-only cli2key bridge.
//!
//! The bridge receives a per-invocation key through an invocation-private file.
//! The key is never an argument, environment value, log field, or receipt
//! field.  The child is a process-group leader and is always reaped on drop.

use sha2::{Digest, Sha256};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime};

const KEY_BYTES: usize = 32;
const KEY_HEX_BYTES: usize = KEY_BYTES * 2;
const MAX_READY: usize = 256;
const MAX_ARGUMENTS: usize = 128;
const READY_TIMEOUT: Duration = Duration::from_secs(5);

/// A stable, secret-free lifecycle failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Cli2KeyError {
    /// The local setup key was absent, malformed, or stale.
    InvalidLocalKey,
    /// The executable or its binding was not acceptable.
    InvalidExecutable,
    /// Secure random bytes were unavailable.
    RandomUnavailable,
    /// Private staging could not be created or removed.
    Staging(io::ErrorKind),
    /// The bridge did not provide a valid loopback readiness line.
    Readiness,
    /// The process could not be started or reaped.
    Process(io::ErrorKind),
}

impl fmt::Display for Cli2KeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLocalKey => f.write_str("cli2key local key is unavailable"),
            Self::InvalidExecutable => f.write_str("cli2key executable is invalid"),
            Self::RandomUnavailable => f.write_str("cli2key random source is unavailable"),
            Self::Staging(_) => f.write_str("cli2key private staging failed"),
            Self::Readiness => f.write_str("cli2key readiness was invalid"),
            Self::Process(_) => f.write_str("cli2key process lifecycle failed"),
        }
    }
}

impl std::error::Error for Cli2KeyError {}

/// Where the invocation key came from, without exposing the key itself.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyOrigin {
    /// A valid local setup key was used.
    Local,
    /// A fresh key was generated because setup was absent or unusable.
    Generated,
}

/// Public, redacted evidence for a running sidecar.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Cli2KeyReceipt {
    /// Validated loopback endpoint.
    pub endpoint: SocketAddr,
    /// Origin of the invocation key.
    pub key_origin: KeyOrigin,
    /// Digest of the key, not the key value.
    pub key_sha256: String,
}

/// A pinned bridge launch. Arguments are copied verbatim and never augmented
/// with the key value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Cli2KeyCommand {
    executable: PathBuf,
    arguments: Vec<String>,
    _digest: String,
}

impl Cli2KeyCommand {
    /// Bind an absolute executable to its expected SHA-256 digest.
    pub fn new(
        executable: PathBuf,
        arguments: Vec<String>,
        digest: String,
    ) -> Result<Self, Cli2KeyError> {
        if !executable.is_absolute()
            || executable
                .components()
                .any(|c| c == std::path::Component::ParentDir)
            || arguments.len() > MAX_ARGUMENTS
            || digest.len() != 64
            || !digest
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err(Cli2KeyError::InvalidExecutable);
        }
        let metadata =
            fs::symlink_metadata(&executable).map_err(|_| Cli2KeyError::InvalidExecutable)?;
        if !metadata.file_type().is_file() || metadata.permissions().mode() & 0o111 == 0 {
            return Err(Cli2KeyError::InvalidExecutable);
        }
        let mut file = File::open(&executable).map_err(|_| Cli2KeyError::InvalidExecutable)?;
        let mut hasher = Sha256::new();
        io::copy(&mut file, &mut DigestWriter(&mut hasher))
            .map_err(|_| Cli2KeyError::InvalidExecutable)?;
        if format!("{:x}", hasher.finalize()) != digest {
            return Err(Cli2KeyError::InvalidExecutable);
        }
        Ok(Self {
            executable,
            arguments,
            _digest: digest,
        })
    }
}

struct DigestWriter<'a>(&'a mut Sha256);
impl Write for DigestWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn decode_key(bytes: &[u8]) -> Option<[u8; KEY_BYTES]> {
    let value = bytes.strip_suffix(b"\n").unwrap_or(bytes);
    if value.len() != KEY_HEX_BYTES {
        return None;
    }
    let mut out = [0; KEY_BYTES];
    for (index, chunk) in value.chunks_exact(2).enumerate() {
        let high = (chunk[0] as char).to_digit(16)? as u8;
        let low = (chunk[1] as char).to_digit(16)? as u8;
        out[index] = high * 16 + low;
    }
    Some(out)
}

/// Read a local setup key without following symlinks and reject stale values.
pub fn load_local_key(
    path: &Path,
    now: SystemTime,
    max_age: Duration,
) -> Result<[u8; KEY_BYTES], Cli2KeyError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| Cli2KeyError::InvalidLocalKey)?;
    if !metadata.file_type().is_file() || metadata.permissions().mode() & 0o077 != 0 {
        return Err(Cli2KeyError::InvalidLocalKey);
    }
    let modified = metadata
        .modified()
        .map_err(|_| Cli2KeyError::InvalidLocalKey)?;
    if now
        .duration_since(modified)
        .ok()
        .is_none_or(|age| age > max_age)
    {
        return Err(Cli2KeyError::InvalidLocalKey);
    }
    let mut bytes = Vec::new();
    rustix::fs::open(
        path,
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map(File::from)
    .map_err(|_| Cli2KeyError::InvalidLocalKey)?
    .take(128)
    .read_to_end(&mut bytes)
    .map_err(|_| Cli2KeyError::InvalidLocalKey)?;
    decode_key(&bytes).ok_or(Cli2KeyError::InvalidLocalKey)
}

fn fresh_key() -> Result<[u8; KEY_BYTES], Cli2KeyError> {
    let mut key = [0; KEY_BYTES];
    File::open("/dev/urandom")
        .map_err(|_| Cli2KeyError::RandomUnavailable)?
        .read_exact(&mut key)
        .map_err(|_| Cli2KeyError::RandomUnavailable)?;
    Ok(key)
}

struct PrivateStaging {
    root: PathBuf,
    key_path: PathBuf,
}
impl PrivateStaging {
    fn create(key: &[u8; KEY_BYTES]) -> Result<Self, Cli2KeyError> {
        let base = std::env::temp_dir();
        for attempt in 0..16u8 {
            let root = base.join(format!("asb-cli2key-{}-{attempt}", std::process::id()));
            match fs::create_dir(&root) {
                Ok(()) => {
                    let key_path = root.join("client.key");
                    let result = (|| {
                        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))
                            .map_err(|e| Cli2KeyError::Staging(e.kind()))?;
                        let mut file = OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .mode(0o600)
                            .open(&key_path)
                            .map_err(|e| Cli2KeyError::Staging(e.kind()))?;
                        file.write_all(
                            &key.iter()
                                .flat_map(|byte| format!("{byte:02x}").into_bytes())
                                .collect::<Vec<_>>(),
                        )
                        .map_err(|e| Cli2KeyError::Staging(e.kind()))?;
                        file.sync_all()
                            .map_err(|e| Cli2KeyError::Staging(e.kind()))?;
                        Ok(Self {
                            root: root.clone(),
                            key_path: key_path.clone(),
                        })
                    })();
                    if result.is_err() {
                        let _ = fs::remove_file(&key_path);
                        let _ = fs::remove_dir(&root);
                    }
                    return result;
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(Cli2KeyError::Staging(error.kind())),
            }
        }
        Err(Cli2KeyError::Staging(io::ErrorKind::AlreadyExists))
    }
}
impl Drop for PrivateStaging {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.key_path);
        let _ = fs::remove_dir(&self.root);
    }
}

/// A running runtime-owned cli2key sidecar.
pub struct Cli2KeySidecar {
    child: Child,
    pid: rustix::process::Pid,
    _staging: PrivateStaging,
    receipt: Cli2KeyReceipt,
}
impl fmt::Debug for Cli2KeySidecar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Cli2KeySidecar")
            .field("receipt", &self.receipt)
            .finish()
    }
}
impl Cli2KeySidecar {
    /// Start a pinned bridge, choosing a valid setup key or generating a fresh one.
    pub fn start(command: &Cli2KeyCommand, local_key: Option<&Path>) -> Result<Self, Cli2KeyError> {
        let (key, origin) = match local_key.and_then(|path| {
            load_local_key(path, SystemTime::now(), Duration::from_secs(24 * 60 * 60)).ok()
        }) {
            Some(key) => (key, KeyOrigin::Local),
            None => (fresh_key()?, KeyOrigin::Generated),
        };
        let staging = PrivateStaging::create(&key)?;
        let mut argv = command.arguments.clone();
        argv.extend([
            "--host".into(),
            "127.0.0.1".into(),
            "--port".into(),
            "0".into(),
            "--key-file".into(),
            staging.key_path.display().to_string(),
        ]);
        let mut child_command = Command::new(&command.executable);
        child_command
            .args(&argv)
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .process_group(0);
        let mut child = child_command
            .spawn()
            .map_err(|e| Cli2KeyError::Process(e.kind()))?;
        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => {
                let _ = rustix::process::kill_process_group(
                    rustix::process::Pid::from_child(&child),
                    rustix::process::Signal::KILL,
                );
                let _ = child.wait();
                return Err(Cli2KeyError::Readiness);
            }
        };
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let mut line = String::new();
            let result = BufReader::new(stdout).read_line(&mut line).map(|_| line);
            let _ = sender.send(result);
        });
        let line = match receiver.recv_timeout(READY_TIMEOUT) {
            Ok(Ok(line)) => line,
            _ => {
                let _ = rustix::process::kill_process_group(
                    rustix::process::Pid::from_child(&child),
                    rustix::process::Signal::KILL,
                );
                let _ = child.wait();
                return Err(Cli2KeyError::Readiness);
            }
        };
        let endpoint = match parse_ready(&line) {
            Some(endpoint) => endpoint,
            None => {
                let _ = rustix::process::kill_process_group(
                    rustix::process::Pid::from_child(&child),
                    rustix::process::Signal::KILL,
                );
                let _ = child.wait();
                return Err(Cli2KeyError::Readiness);
            }
        };
        let pid = rustix::process::Pid::from_child(&child);
        let mut digest = Sha256::new();
        digest.update(key);
        let key_sha256 = format!("{:x}", digest.finalize());
        Ok(Self {
            child,
            pid,
            _staging: staging,
            receipt: Cli2KeyReceipt {
                endpoint,
                key_origin: origin,
                key_sha256,
            },
        })
    }
    /// Return redacted lifecycle evidence.
    pub fn receipt(&self) -> &Cli2KeyReceipt {
        &self.receipt
    }
    /// Return the validated loopback endpoint.
    pub fn endpoint(&self) -> SocketAddr {
        self.receipt.endpoint
    }
}
impl Drop for Cli2KeySidecar {
    fn drop(&mut self) {
        let _ = rustix::process::kill_process_group(self.pid, rustix::process::Signal::KILL);
        let _ = self.child.wait();
    }
}

fn parse_ready(line: &str) -> Option<SocketAddr> {
    let value = line.strip_prefix("ASB_CLI2KEY_READY ")?.trim();
    if value.len() > MAX_READY {
        return None;
    }
    let endpoint: SocketAddr = value.parse().ok()?;
    match endpoint.ip() {
        IpAddr::V4(ip) if ip == Ipv4Addr::LOCALHOST => Some(endpoint),
        IpAddr::V6(ip) if ip == Ipv6Addr::LOCALHOST => Some(endpoint),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::time::UNIX_EPOCH;

    #[test]
    fn key_decoder_requires_exact_256_bits() {
        assert!(decode_key(b"00").is_none());
        assert!(decode_key(&[b'a'; 64]).is_some());
    }

    #[test]
    fn generated_keys_are_fresh_256_bit_values() {
        let first = fresh_key().unwrap();
        let second = fresh_key().unwrap();
        assert_ne!(first, second);
        assert_eq!(first.len() * 8, 256);
    }
    #[test]
    fn readiness_rejects_wildcard_and_accepts_loopback() {
        assert!(parse_ready("ASB_CLI2KEY_READY 0.0.0.0:7\n").is_none());
        assert_eq!(
            parse_ready("ASB_CLI2KEY_READY 127.0.0.1:7\n")
                .unwrap()
                .port(),
            7
        );
    }
    #[test]
    fn staging_is_private_and_removed() {
        let staging = PrivateStaging::create(&[7; KEY_BYTES]).unwrap();
        assert_eq!(
            fs::metadata(&staging.root).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&staging.key_path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let root = staging.root.clone();
        drop(staging);
        assert!(!root.exists());
    }

    #[test]
    fn local_key_absent_malformed_stale_and_valid_are_distinct() {
        let root = std::env::temp_dir().join(format!("asb-cli2key-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).unwrap();
        let path = root.join("key");
        assert_eq!(
            load_local_key(&path, SystemTime::now(), Duration::from_secs(1)),
            Err(Cli2KeyError::InvalidLocalKey)
        );
        fs::write(&path, b"short").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            load_local_key(&path, SystemTime::now(), Duration::from_secs(1)),
            Err(Cli2KeyError::InvalidLocalKey)
        );
        fs::write(&path, vec![b'a'; KEY_HEX_BYTES]).unwrap();
        let old = UNIX_EPOCH + Duration::from_secs(1);
        assert_eq!(
            load_local_key(&path, SystemTime::now(), Duration::from_secs(1)),
            Ok([0xaa; KEY_BYTES])
        );
        assert_eq!(
            load_local_key(&path, old, Duration::from_secs(1)),
            Err(Cli2KeyError::InvalidLocalKey)
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn symlink_setup_key_is_rejected() {
        let root = std::env::temp_dir().join(format!("asb-cli2key-link-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).unwrap();
        let target = root.join("target");
        fs::write(&target, vec![b'a'; KEY_HEX_BYTES]).unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
        let link = root.join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert_eq!(
            load_local_key(&link, SystemTime::now(), Duration::from_secs(60)),
            Err(Cli2KeyError::InvalidLocalKey)
        );
        let _ = fs::remove_dir_all(root);
    }
}
