// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Sealed one-shot credential delivery at the sandbox process boundary.

use rustix::fs::{MemfdFlags, SealFlags, fcntl_add_seals, fcntl_get_seals, memfd_create};
use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::process::Command;

const MAX_CREDENTIAL_BYTES: usize = 16 * 1024;

/// Failure while preparing or consuming a sealed child credential channel.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxCredentialError {
    /// The binding reference is not a lowercase SHA-256 digest.
    InvalidReference,
    /// The adapter environment target is invalid or unbounded.
    InvalidTarget,
    /// The value is empty, too large, or contains a disallowed byte.
    InvalidValue,
    /// The channel was already consumed.
    AlreadyConsumed,
    /// The operating system rejected the descriptor operation.
    OperatingSystem,
}

/// Runtime-owned binding passed to the final sandbox launch boundary.
///
/// Construction is crate-private: callers cannot manufacture a provider
/// binding independently of the runtime selection service.
pub struct SandboxCredentialBinding {
    reference_sha256: String,
    target: String,
}

impl SandboxCredentialBinding {
    /// Validate a credential reference and adapter-owned target before launch.
    /// This value carries metadata only; it does not grant launch authority.
    pub(crate) fn new(
        reference_sha256: impl Into<String>,
        target: impl Into<String>,
    ) -> Result<Self, SandboxCredentialError> {
        let reference_sha256 = reference_sha256.into();
        let target = target.into();
        if !valid_digest(&reference_sha256) {
            return Err(SandboxCredentialError::InvalidReference);
        }
        if !valid_target(&target) {
            return Err(SandboxCredentialError::InvalidTarget);
        }
        Ok(Self {
            reference_sha256,
            target,
        })
    }

    pub(crate) fn revalidated(&self) -> Result<Self, SandboxCredentialError> {
        Self::new(self.reference_sha256.clone(), self.target.clone())
    }
}

/// Opaque descriptor channel created only from a runtime-owned binding.
pub struct SandboxCredentialChannel {
    descriptor: File,
    reference_sha256: String,
    target: String,
    filled: bool,
    sealed: bool,
}

impl std::fmt::Debug for SandboxCredentialChannel {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SandboxCredentialChannel")
            .field("reference_sha256", &self.reference_sha256)
            .field("target", &self.target)
            .field("filled", &self.filled)
            .field("sealed", &self.sealed)
            .finish_non_exhaustive()
    }
}

impl SandboxCredentialChannel {
    pub(crate) fn new(binding: &SandboxCredentialBinding) -> Result<Self, SandboxCredentialError> {
        let descriptor = memfd_create("asb-credential-v1", MemfdFlags::ALLOW_SEALING)
            .map_err(|_| SandboxCredentialError::OperatingSystem)?;
        Ok(Self {
            descriptor: File::from(descriptor),
            reference_sha256: binding.reference_sha256.clone(),
            target: binding.target.clone(),
            filled: false,
            sealed: false,
        })
    }

    /// Fill and seal the channel exactly once; the owned input is erased.
    pub fn write_and_seal(&mut self, mut value: Vec<u8>) -> Result<(), SandboxCredentialError> {
        if self.filled || self.sealed {
            value.fill(0);
            return Err(SandboxCredentialError::AlreadyConsumed);
        }
        if value.is_empty()
            || value.len() > MAX_CREDENTIAL_BYTES
            || value.iter().any(|byte| !matches!(*byte, 0x21..=0x7e))
        {
            value.fill(0);
            return Err(SandboxCredentialError::InvalidValue);
        }
        let mut args = Vec::with_capacity(value.len() + self.target.len() + 20);
        args.extend_from_slice(b"--setenv\0");
        args.extend_from_slice(self.target.as_bytes());
        args.push(0);
        args.extend_from_slice(&value);
        args.push(0);
        value.fill(0);
        self.descriptor
            .write_all(&args)
            .map_err(|_| SandboxCredentialError::OperatingSystem)?;
        args.fill(0);
        self.descriptor
            .flush()
            .and_then(|_| self.descriptor.seek(SeekFrom::Start(0)).map(|_| ()))
            .map_err(|_| SandboxCredentialError::OperatingSystem)?;
        let required = SealFlags::SEAL | SealFlags::SHRINK | SealFlags::GROW | SealFlags::WRITE;
        fcntl_add_seals(&self.descriptor, required)
            .map_err(|_| SandboxCredentialError::OperatingSystem)?;
        if !fcntl_get_seals(&self.descriptor)
            .map_err(|_| SandboxCredentialError::OperatingSystem)?
            .contains(required)
        {
            return Err(SandboxCredentialError::OperatingSystem);
        }
        self.filled = true;
        self.sealed = true;
        Ok(())
    }

    pub(crate) fn append_bwrap_args(
        &self,
        command: &mut Command,
    ) -> Result<(), SandboxCredentialError> {
        if !self.filled || !self.sealed {
            return Err(SandboxCredentialError::AlreadyConsumed);
        }
        command
            .arg("--args")
            .arg(self.descriptor.as_raw_fd().to_string());
        Ok(())
    }
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn valid_target(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_uppercase() || byte == b'_')
        && bytes.all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
        && value.len() <= 128
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn binding_and_value_validation_are_fail_closed() {
        assert!(matches!(
            SandboxCredentialBinding::new("bad", "OPENROUTER_API_KEY"),
            Err(SandboxCredentialError::InvalidReference)
        ));
        assert!(matches!(
            SandboxCredentialBinding::new("a".repeat(64), "bad-target"),
            Err(SandboxCredentialError::InvalidTarget)
        ));
        let binding = SandboxCredentialBinding::new("a".repeat(64), "OPENROUTER_API_KEY").unwrap();
        let mut channel = SandboxCredentialChannel::new(&binding).unwrap();
        assert_eq!(
            channel.write_and_seal(Vec::new()),
            Err(SandboxCredentialError::InvalidValue)
        );
        assert_eq!(
            channel.write_and_seal(vec![b'\n']),
            Err(SandboxCredentialError::InvalidValue)
        );
        assert_eq!(
            channel.write_and_seal(vec![b'x'; MAX_CREDENTIAL_BYTES + 1]),
            Err(SandboxCredentialError::InvalidValue)
        );
        let mut command = Command::new("/bin/true");
        assert_eq!(
            channel.append_bwrap_args(&mut command),
            Err(SandboxCredentialError::AlreadyConsumed)
        );
        assert!(SandboxCredentialBinding::new("a".repeat(64), "A").is_ok());
        assert!(SandboxCredentialBinding::new("A".repeat(64), "A").is_err());
        assert!(SandboxCredentialBinding::new("g".repeat(64), "A").is_err());
        assert!(SandboxCredentialBinding::new("a".repeat(64), "").is_err());
        assert!(SandboxCredentialBinding::new("a".repeat(64), "a").is_err());
        assert!(SandboxCredentialBinding::new("a".repeat(64), "1A").is_err());
        assert!(SandboxCredentialBinding::new("a".repeat(64), "A".repeat(129)).is_err());
    }

    #[test]
    fn channel_is_one_shot_and_debug_is_metadata_only() {
        let binding = SandboxCredentialBinding::new("c".repeat(64), "TARGET").unwrap();
        let rebound = binding.revalidated().unwrap();
        let mut channel = SandboxCredentialChannel::new(&rebound).unwrap();
        let debug = format!("{channel:?}");
        assert!(debug.contains("TARGET"));
        assert!(debug.contains("filled: false"));
        channel.write_and_seal(b"value".to_vec()).unwrap();
        assert_eq!(
            channel.write_and_seal(b"again".to_vec()),
            Err(SandboxCredentialError::AlreadyConsumed)
        );
        let mut command = Command::new("/bin/true");
        channel.append_bwrap_args(&mut command).unwrap();
        assert!(command.get_args().any(|arg| arg == "--args"));
    }

    #[test]
    fn bubblewrap_receives_the_bound_target_without_argv_secret() {
        if !Path::new("/usr/bin/bwrap").is_file() {
            return;
        }
        let binding = SandboxCredentialBinding::new("b".repeat(64), "OPENROUTER_API_KEY").unwrap();
        let mut channel = SandboxCredentialChannel::new(&binding).unwrap();
        channel
            .write_and_seal(b"child-only-secret".to_vec())
            .unwrap();
        let mut command = Command::new("/usr/bin/bwrap");
        command.args([
            "--die-with-parent",
            "--new-session",
            "--clearenv",
            "--ro-bind",
            "/usr",
            "/usr",
            "--ro-bind",
            "/bin",
            "/bin",
            "--ro-bind",
            "/lib",
            "/lib",
            "--ro-bind",
            "/lib64",
            "/lib64",
            "--setenv",
            "PATH",
            "/usr/bin:/bin",
        ]);
        channel.append_bwrap_args(&mut command).unwrap();
        command.args([
            "--",
            "/bin/sh",
            "-c",
            "test \"$OPENROUTER_API_KEY\" = child-only-secret",
        ]);
        assert!(command.output().unwrap().status.success());
    }
}
