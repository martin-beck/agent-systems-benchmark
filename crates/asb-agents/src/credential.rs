// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Bounded credential-reference resolution and isolated process injection.

use asb_protocol::{CredentialSource, ProviderProfileV1};
use asb_runtime::{ProcessError, ProcessLimits, RunningProcess, Termination};
use rustix::fs::{
    MemfdFlags, Mode, SealFlags, fchmod, fcntl_add_seals, fcntl_get_seals, memfd_create,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

/// Largest credential value accepted at the provider process boundary.
pub const MAX_CREDENTIAL_BYTES: usize = 16 * 1024;
/// Largest explicit environment-reference or target name.
pub const MAX_ENVIRONMENT_NAME_BYTES: usize = 128;
/// Largest complete JSON document accepted from a credential helper.
pub const MAX_HELPER_RESPONSE_BYTES: usize = MAX_CREDENTIAL_BYTES + 128;
/// Longest credential-helper execution accepted by this boundary.
pub const MAX_HELPER_TIMEOUT: Duration = Duration::from_secs(30);
/// Largest allowlisted helper executable accepted for identity hashing.
pub const MAX_HELPER_EXECUTABLE_BYTES: u64 = 64 * 1024 * 1024;

/// Resolver owning one already-open, one-shot credential descriptor.
pub struct FileDescriptorCredentialResolver {
    file: File,
    reference_sha256: String,
}

impl fmt::Debug for FileDescriptorCredentialResolver {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter
            .debug_struct("FileDescriptorCredentialResolver")
            .field("reference_sha256", &self.reference_sha256)
            .finish_non_exhaustive()
    }
}

impl FileDescriptorCredentialResolver {
    /// Bind an owned descriptor to a public logical locator.
    ///
    /// No path is opened here. The caller transfers an already-open descriptor,
    /// so this boundary makes no claim about traversal before that open.
    pub fn new(
        descriptor: OwnedFd,
        logical_locator: &str,
    ) -> Result<Self, CredentialResolutionError> {
        validate_logical_locator(logical_locator)?;
        let file = File::from(descriptor);
        validate_credential_file(&file)?;
        Ok(Self {
            file,
            reference_sha256: locator_reference_sha256(
                b"file_descriptor",
                logical_locator.as_bytes(),
            ),
        })
    }

    /// Credential-free digest to place in a provider profile.
    pub fn reference_sha256(&self) -> &str {
        &self.reference_sha256
    }

    /// Revalidate and consume the descriptor exactly once.
    pub fn resolve(
        mut self,
        profile: &ProviderProfileV1,
    ) -> Result<ResolvedCredential, CredentialResolutionError> {
        validate_profile_reference(
            profile,
            CredentialSource::FileDescriptor,
            &self.reference_sha256,
        )?;
        validate_credential_file(&self.file)?;
        let mut bytes = Vec::with_capacity(MAX_CREDENTIAL_BYTES.saturating_add(1));
        self.file
            .by_ref()
            .take((MAX_CREDENTIAL_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| CredentialResolutionError::Unavailable)?;
        if bytes.len() > MAX_CREDENTIAL_BYTES {
            bytes.fill(0);
            return Err(CredentialResolutionError::InvalidValue);
        }
        ResolvedCredential::new(OsString::from_vec(bytes))
    }
}

/// Content-allowlisted executable for the credential-helper v1 protocol.
pub struct HelperCredentialResolver {
    executable: File,
    executable_sha256: String,
    reference_sha256: String,
}

impl fmt::Debug for HelperCredentialResolver {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter
            .debug_struct("HelperCredentialResolver")
            .field("executable_sha256", &self.executable_sha256)
            .field("reference_sha256", &self.reference_sha256)
            .finish_non_exhaustive()
    }
}

impl HelperCredentialResolver {
    /// Bind an already-open executable to its expected public content identity.
    ///
    /// The caller owns safe opening. This boundary neither stores nor reports a
    /// source path and makes no claim about traversal before the descriptor open.
    pub fn new(
        executable: OwnedFd,
        logical_locator: &str,
        expected_executable_sha256: &str,
    ) -> Result<Self, CredentialResolutionError> {
        validate_logical_locator(logical_locator)?;
        if !is_sha256(expected_executable_sha256) {
            return Err(CredentialResolutionError::InvalidHelper);
        }
        let executable =
            stage_helper_executable(File::from(executable), expected_executable_sha256)?;
        let executable_sha256 = expected_executable_sha256.to_owned();
        let reference_sha256 = helper_reference_sha256(
            logical_locator.as_bytes(),
            expected_executable_sha256.as_bytes(),
        );
        Ok(Self {
            executable,
            executable_sha256,
            reference_sha256,
        })
    }

    /// Credential-free digest to place in a provider profile.
    pub fn reference_sha256(&self) -> &str {
        &self.reference_sha256
    }

    /// Start one bounded helper request with no ambient environment or input.
    pub fn start(
        self,
        profile: &ProviderProfileV1,
        timeout: Duration,
    ) -> Result<PendingHelperCredential, HelperCredentialError> {
        validate_profile_reference(profile, CredentialSource::Helper, &self.reference_sha256)
            .map_err(HelperCredentialError::Resolution)?;
        validate_staged_helper(&self.executable, &self.executable_sha256)
            .map_err(HelperCredentialError::Resolution)?;
        if timeout.is_zero() || timeout > MAX_HELPER_TIMEOUT {
            return Err(HelperCredentialError::Resolution(
                CredentialResolutionError::InvalidHelper,
            ));
        }
        let limits = ProcessLimits::new(
            MAX_HELPER_RESPONSE_BYTES,
            4096,
            timeout,
            Duration::from_millis(50),
            Duration::from_millis(5),
        )
        .map_err(|_| HelperCredentialError::Resolution(CredentialResolutionError::InvalidHelper))?;
        // Keep the sealed image open in this parent until the helper is reaped. A
        // parent procfd lets script interpreters reopen the immutable image while
        // CLOEXEC remains set in both parent and child descriptor tables.
        let program = format!(
            "/proc/{}/fd/{}",
            std::process::id(),
            self.executable.as_raw_fd()
        );
        let mut command = Command::new(program);
        command
            .arg("--asb-credential-helper-v1")
            .env_clear()
            .stdin(Stdio::null());
        let process =
            RunningProcess::spawn(command, limits).map_err(HelperCredentialError::Process)?;
        Ok(PendingHelperCredential {
            process,
            _executable: self.executable,
        })
    }
}

/// Cancellable, one-shot helper request.
pub struct PendingHelperCredential {
    process: RunningProcess,
    _executable: File,
}

impl PendingHelperCredential {
    /// Request idempotent process-group cancellation.
    pub fn cancel(&mut self) -> Result<(), HelperCredentialError> {
        self.process
            .cancel()
            .map_err(HelperCredentialError::Process)
    }

    /// Reap the helper and consume its exact v1 response.
    pub fn wait(mut self) -> Result<ResolvedCredential, HelperCredentialError> {
        let output = self
            .process
            .wait()
            .map_err(HelperCredentialError::Process)?;
        match output.termination {
            Termination::Cancelled => return Err(HelperCredentialError::Cancelled),
            Termination::TimedOut => return Err(HelperCredentialError::TimedOut),
            Termination::Exited => {}
        }
        if output.exit_code != Some(0) || output.stdout.truncated || output.stderr.total_bytes != 0
        {
            return Err(HelperCredentialError::Failed);
        }
        let response: HelperResponse = serde_json::from_slice(&output.stdout.bytes)
            .map_err(|_| HelperCredentialError::InvalidResponse)?;
        if response.version != 1 {
            return Err(HelperCredentialError::InvalidResponse);
        }
        ResolvedCredential::new(OsString::from(response.credential))
            .map_err(|_| HelperCredentialError::InvalidResponse)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HelperResponse {
    version: u8,
    credential: String,
}

/// Resolver for one explicit logical environment credential reference.
#[derive(Clone, Eq, PartialEq)]
pub struct EnvironmentCredentialResolver {
    variable: OsString,
    reference_sha256: String,
}

impl fmt::Debug for EnvironmentCredentialResolver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EnvironmentCredentialResolver")
            .field("reference_sha256", &self.reference_sha256)
            .finish_non_exhaustive()
    }
}

impl EnvironmentCredentialResolver {
    /// Bind a resolver to one explicit environment variable name.
    pub fn new(variable: impl Into<OsString>) -> Result<Self, CredentialResolutionError> {
        let variable = variable.into();
        validate_environment_name(&variable)?;
        let reference_sha256 = environment_reference_sha256(&variable)?;
        Ok(Self {
            variable,
            reference_sha256,
        })
    }

    /// Credential-free digest to place in [`ProviderProfileV1`].
    pub fn reference_sha256(&self) -> &str {
        &self.reference_sha256
    }

    /// Resolve with an injected lookup boundary, useful for isolated callers and tests.
    pub fn resolve_with<F>(
        &self,
        profile: &ProviderProfileV1,
        mut lookup: F,
    ) -> Result<ResolvedCredential, CredentialResolutionError>
    where
        F: FnMut(&OsStr) -> Option<OsString>,
    {
        validate_profile_reference(
            profile,
            CredentialSource::Environment,
            &self.reference_sha256,
        )?;
        let value = lookup(&self.variable).ok_or(CredentialResolutionError::Unavailable)?;
        ResolvedCredential::new(value)
    }

    /// Resolve only the explicitly bound variable from the current process environment.
    pub fn resolve_environment(
        &self,
        profile: &ProviderProfileV1,
    ) -> Result<ResolvedCredential, CredentialResolutionError> {
        self.resolve_with(profile, |name| std::env::var_os(name))
    }
}

/// Compute the canonical credential-free identity of one environment reference.
pub fn environment_reference_sha256(variable: &OsStr) -> Result<String, CredentialResolutionError> {
    validate_environment_name(variable)?;
    let mut digest = Sha256::new();
    digest.update(b"asb-credential-reference-v1\0environment\0");
    digest.update(variable.as_bytes());
    Ok(format!("{:x}", digest.finalize()))
}

/// Opaque resolved credential. It is neither cloneable, printable, nor serializable.
pub struct ResolvedCredential {
    bytes: Vec<u8>,
}

impl ResolvedCredential {
    fn new(value: OsString) -> Result<Self, CredentialResolutionError> {
        let mut bytes = value.into_vec();
        if bytes.is_empty()
            || bytes.len() > MAX_CREDENTIAL_BYTES
            || bytes.iter().any(|byte| !matches!(*byte, 0x21..=0x7e))
        {
            bytes.fill(0);
            return Err(CredentialResolutionError::InvalidValue);
        }
        Ok(Self { bytes })
    }

    /// Spawn a child with an otherwise empty environment and one credential target.
    ///
    /// The credential is never placed in arguments, output, or an ASB evidence type.
    pub fn spawn<I, S>(
        mut self,
        program: &Path,
        arguments: I,
        credential_target: &OsStr,
        limits: ProcessLimits,
    ) -> Result<RunningProcess, CredentialProcessError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        validate_environment_name(credential_target).map_err(CredentialProcessError::Resolution)?;
        let mut command = Command::new(program);
        command
            .args(arguments)
            .env_clear()
            .env(credential_target, OsStr::from_bytes(&self.bytes))
            .stdin(Stdio::null());
        let result = RunningProcess::spawn(command, limits).map_err(CredentialProcessError::Spawn);
        self.erase();
        result
    }

    fn erase(&mut self) {
        self.bytes.fill(0);
    }
}

impl Drop for ResolvedCredential {
    fn drop(&mut self) {
        self.erase();
    }
}

/// Credential resolution failure without source names or values.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CredentialResolutionError {
    /// Explicit environment variable name is malformed or unbounded.
    InvalidEnvironmentName,
    /// Public logical locator metadata is malformed or unbounded.
    InvalidLocator,
    /// Descriptor is not a private regular file owned by the current user.
    InvalidDescriptor,
    /// Helper executable, identity, or execution limit is invalid.
    InvalidHelper,
    /// Provider profile failed its common validation.
    InvalidProfile,
    /// Profile source or logical reference does not match this resolver.
    ReferenceMismatch,
    /// The explicitly bound source did not return a value.
    Unavailable,
    /// Credential value is empty, unbounded, non-ASCII, or contains whitespace/control bytes.
    InvalidValue,
}

impl fmt::Display for CredentialResolutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidEnvironmentName => {
                formatter.write_str("credential environment reference is invalid")
            }
            Self::InvalidLocator => formatter.write_str("credential logical locator is invalid"),
            Self::InvalidDescriptor => {
                formatter.write_str("credential descriptor is not a private owned regular file")
            }
            Self::InvalidHelper => formatter.write_str("credential helper is invalid"),
            Self::InvalidProfile => formatter.write_str("provider credential profile is invalid"),
            Self::ReferenceMismatch => {
                formatter.write_str("provider credential reference does not match resolver")
            }
            Self::Unavailable => formatter.write_str("provider credential is unavailable"),
            Self::InvalidValue => formatter.write_str("provider credential value is invalid"),
        }
    }
}

impl std::error::Error for CredentialResolutionError {}

/// Credential-bearing process boundary failure.
#[derive(Debug)]
pub enum CredentialProcessError {
    /// Credential target name was invalid.
    Resolution(CredentialResolutionError),
    /// Isolated process creation failed.
    Spawn(ProcessError),
}

impl fmt::Display for CredentialProcessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Resolution(error) => error.fmt(formatter),
            Self::Spawn(_) => {
                formatter.write_str("credential-isolated provider process failed to start")
            }
        }
    }
}

impl std::error::Error for CredentialProcessError {}

/// Credential-helper lifecycle failure without output, paths, or secret bytes.
#[derive(Debug)]
pub enum HelperCredentialError {
    /// Profile, executable identity, or limit validation failed.
    Resolution(CredentialResolutionError),
    /// Process creation, signalling, observation, or reaping failed.
    Process(ProcessError),
    /// The caller cancelled and reaped the helper.
    Cancelled,
    /// The helper exceeded its monotonic deadline and was reaped.
    TimedOut,
    /// The helper exited unsuccessfully or wrote stderr.
    Failed,
    /// The helper response did not match the exact bounded v1 schema.
    InvalidResponse,
}

impl fmt::Display for HelperCredentialError {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Resolution(error) => error.fmt(formatter),
            Self::Process(_) => formatter.write_str("credential helper process failed"),
            Self::Cancelled => formatter.write_str("credential helper was cancelled"),
            Self::TimedOut => formatter.write_str("credential helper timed out"),
            Self::Failed => formatter.write_str("credential helper failed"),
            Self::InvalidResponse => formatter.write_str("credential helper response is invalid"),
        }
    }
}

impl std::error::Error for HelperCredentialError {}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, 97..=102))
}

fn helper_reference_sha256(locator: &[u8], executable_sha256: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(b"asb-credential-reference-v1\0helper-v1\0");
    digest.update(locator);
    digest.update(b"\0");
    digest.update(executable_sha256);
    format!("{:x}", digest.finalize())
}

fn validate_helper_executable(file: &File) -> Result<(), CredentialResolutionError> {
    let metadata = file
        .metadata()
        .map_err(|_| CredentialResolutionError::InvalidHelper)?;
    let current_uid = std::fs::metadata("/proc/self")
        .map_err(|_| CredentialResolutionError::InvalidHelper)?
        .uid();
    if !metadata.file_type().is_file()
        || metadata.uid() != current_uid
        || metadata.mode() & 0o022 != 0
        || metadata.mode() & 0o100 == 0
        || metadata.len() == 0
        || metadata.len() > MAX_HELPER_EXECUTABLE_BYTES
    {
        Err(CredentialResolutionError::InvalidHelper)
    } else {
        Ok(())
    }
}

fn stage_helper_executable(
    mut source: File,
    expected_sha256: &str,
) -> Result<File, CredentialResolutionError> {
    validate_helper_executable(&source)?;
    source
        .seek(SeekFrom::Start(0))
        .map_err(|_| CredentialResolutionError::InvalidHelper)?;
    let mut bytes = Vec::with_capacity(64 * 1024);
    source
        .take(MAX_HELPER_EXECUTABLE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| CredentialResolutionError::InvalidHelper)?;
    if bytes.is_empty()
        || bytes.len() as u64 > MAX_HELPER_EXECUTABLE_BYTES
        || format!("{:x}", Sha256::digest(&bytes)) != expected_sha256
    {
        bytes.fill(0);
        return Err(CredentialResolutionError::InvalidHelper);
    }

    let descriptor = memfd_create(
        "asb-credential-helper-v1",
        MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING,
    )
    .map_err(|_| CredentialResolutionError::InvalidHelper)?;
    let mut staged = File::from(descriptor);
    if std::io::Write::write_all(&mut staged, &bytes).is_err()
        || std::io::Write::flush(&mut staged).is_err()
    {
        bytes.fill(0);
        return Err(CredentialResolutionError::InvalidHelper);
    }
    bytes.fill(0);
    fchmod(&staged, Mode::RUSR | Mode::XUSR)
        .map_err(|_| CredentialResolutionError::InvalidHelper)?;
    let required = SealFlags::SEAL | SealFlags::SHRINK | SealFlags::GROW | SealFlags::WRITE;
    fcntl_add_seals(&staged, required).map_err(|_| CredentialResolutionError::InvalidHelper)?;
    if !fcntl_get_seals(&staged)
        .map_err(|_| CredentialResolutionError::InvalidHelper)?
        .contains(required)
    {
        return Err(CredentialResolutionError::InvalidHelper);
    }
    staged
        .seek(SeekFrom::Start(0))
        .map_err(|_| CredentialResolutionError::InvalidHelper)?;
    Ok(staged)
}

fn validate_staged_helper(
    staged: &File,
    expected_sha256: &str,
) -> Result<(), CredentialResolutionError> {
    validate_helper_executable(staged)?;
    let required = SealFlags::SEAL | SealFlags::SHRINK | SealFlags::GROW | SealFlags::WRITE;
    if !fcntl_get_seals(staged)
        .map_err(|_| CredentialResolutionError::InvalidHelper)?
        .contains(required)
        || file_sha256(staged)? != expected_sha256
    {
        return Err(CredentialResolutionError::InvalidHelper);
    }
    Ok(())
}

fn file_sha256(file: &File) -> Result<String, CredentialResolutionError> {
    let mut reader = file
        .try_clone()
        .map_err(|_| CredentialResolutionError::InvalidHelper)?;
    reader
        .seek(SeekFrom::Start(0))
        .map_err(|_| CredentialResolutionError::InvalidHelper)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|_| CredentialResolutionError::InvalidHelper)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn locator_reference_sha256(source: &[u8], locator: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(b"asb-credential-reference-v1\0");
    digest.update(source);
    digest.update(b"\0");
    digest.update(locator);
    format!("{:x}", digest.finalize())
}

fn validate_logical_locator(value: &str) -> Result<(), CredentialResolutionError> {
    let bytes = value.as_bytes();
    if bytes.is_empty()
        || bytes.len() > MAX_ENVIRONMENT_NAME_BYTES
        || bytes
            .iter()
            .any(|byte| !(byte.is_ascii_alphanumeric() || matches!(*byte, 95 | 45 | 46)))
    {
        Err(CredentialResolutionError::InvalidLocator)
    } else {
        Ok(())
    }
}

fn validate_profile_reference(
    profile: &ProviderProfileV1,
    source: CredentialSource,
    reference_sha256: &str,
) -> Result<(), CredentialResolutionError> {
    profile
        .validate()
        .map_err(|_| CredentialResolutionError::InvalidProfile)?;
    if profile.credential.source != source
        || profile.credential.reference_sha256.as_deref() != Some(reference_sha256)
    {
        Err(CredentialResolutionError::ReferenceMismatch)
    } else {
        Ok(())
    }
}

fn validate_credential_file(file: &File) -> Result<(), CredentialResolutionError> {
    let metadata = file
        .metadata()
        .map_err(|_| CredentialResolutionError::InvalidDescriptor)?;
    let current_uid = std::fs::metadata("/proc/self")
        .map_err(|_| CredentialResolutionError::InvalidDescriptor)?
        .uid();
    if !metadata.file_type().is_file()
        || metadata.uid() != current_uid
        || metadata.mode() & 0o077 != 0
        || metadata.len() > MAX_CREDENTIAL_BYTES as u64
    {
        Err(CredentialResolutionError::InvalidDescriptor)
    } else {
        Ok(())
    }
}

fn validate_environment_name(value: &OsStr) -> Result<(), CredentialResolutionError> {
    let bytes = value.as_bytes();
    if bytes.is_empty()
        || bytes.len() > MAX_ENVIRONMENT_NAME_BYTES
        || !bytes[0].is_ascii_uppercase()
        || bytes
            .iter()
            .any(|byte| !(byte.is_ascii_uppercase() || byte.is_ascii_digit() || *byte == b'_'))
    {
        Err(CredentialResolutionError::InvalidEnvironmentName)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use asb_protocol::{
        CredentialProvenance, EndpointClass, EndpointProvenance, PROVIDER_PROFILE_V1, ProviderKind,
        ProviderSettings, ProviderTransportLimits,
    };
    use std::fs;
    use std::io::{Seek, Write};
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    static NEXT_DESCRIPTOR_FIXTURE: AtomicU64 = AtomicU64::new(0);

    fn profile_for(source: CredentialSource, reference: String) -> ProviderProfileV1 {
        let mut profile = ProviderProfileV1 {
            version: PROVIDER_PROFILE_V1,
            settings_sha256: String::new(),
            provider: ProviderKind::OpenAi,
            endpoint: EndpointProvenance {
                class: EndpointClass::PublicService,
                identity_sha256: "a".repeat(64),
            },
            model: "pinned-model".into(),
            settings: ProviderSettings {
                temperature_milli: None,
                top_p_millionth: None,
                seed: None,
                max_output_tokens: None,
                reasoning_effort: None,
                additional_settings_sha256: None,
            },
            transport: ProviderTransportLimits {
                max_request_bytes: 1024,
                max_response_bytes: 1024,
                connect_timeout_ms: 1000,
                request_timeout_ms: 1000,
                max_concurrent_requests: 1,
            },
            credential: CredentialProvenance {
                source,
                reference_sha256: Some(reference),
            },
        };
        profile.refresh_settings_sha256().unwrap();
        profile
    }

    fn profile(reference: String) -> ProviderProfileV1 {
        profile_for(CredentialSource::Environment, reference)
    }

    fn descriptor_file(contents: &[u8]) -> (OwnedFd, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "asb-credential-fd-{}-{}",
            std::process::id(),
            NEXT_DESCRIPTOR_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .mode(0o600)
            .open(&path)
            .unwrap();
        file.write_all(contents).unwrap();
        file.rewind().unwrap();
        (file.into(), path)
    }

    fn helper_file(script: &str) -> (OwnedFd, std::path::PathBuf, String) {
        let path = std::env::temp_dir().join(format!(
            "asb-credential-helper-{}-{}",
            std::process::id(),
            NEXT_DESCRIPTOR_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        let mut writable = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o700)
            .open(&path)
            .unwrap();
        writable.write_all(script.as_bytes()).unwrap();
        writable.sync_all().unwrap();
        drop(writable);
        let file = File::open(&path).unwrap();
        let digest = format!("{:x}", Sha256::digest(script.as_bytes()));
        (file.into(), path, digest)
    }

    fn helper(script: &str) -> (HelperCredentialResolver, std::path::PathBuf) {
        let (descriptor, path, digest) = helper_file(script);
        (
            HelperCredentialResolver::new(descriptor, "provider.primary", &digest).unwrap(),
            path,
        )
    }

    #[test]
    fn helper_exact_v1_response_resolves_without_ambient_environment() {
        let script = "#!/bin/sh\ntest \"$1\" = \"--asb-credential-helper-v1\" || exit 8\ntest -z \"${HOME+x}\" || exit 9\nfor link in /proc/self/fd/*; do case \"$(readlink \"$link\" 2>/dev/null)\" in *asb-credential-helper-v1*) exit 10;; esac; done\nprintf \"{\\\"version\\\":1,\\\"credential\\\":\\\"synthetic-value\\\"}\"\n";
        let (resolver, path) = helper(script);
        assert!(!format!("{resolver:?}").contains(path.to_string_lossy().as_ref()));
        let profile = profile_for(CredentialSource::Helper, resolver.reference_sha256().into());
        let credential = resolver
            .start(&profile, Duration::from_secs(1))
            .unwrap()
            .wait()
            .unwrap();
        assert_eq!(credential.bytes, b"synthetic-value");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn helper_stages_a_sealed_image_before_source_mutation_and_replacement() {
        let original =
            "#!/bin/sh\nprintf \"{\\\"version\\\":1,\\\"credential\\\":\\\"bound-value\\\"}\"\n";
        let replacement =
            "#!/bin/sh\nprintf \"{\\\"version\\\":1,\\\"credential\\\":\\\"changed-value\\\"}\"\n";
        let (descriptor, path, digest) = helper_file(original);
        let resolver =
            HelperCredentialResolver::new(descriptor, "provider.primary", &digest).unwrap();
        let required = SealFlags::SEAL | SealFlags::SHRINK | SealFlags::GROW | SealFlags::WRITE;
        assert!(
            fcntl_get_seals(&resolver.executable)
                .unwrap()
                .contains(required)
        );
        assert!(
            rustix::io::fcntl_getfd(&resolver.executable)
                .unwrap()
                .contains(rustix::io::FdFlags::CLOEXEC)
        );
        let mut writable_view = resolver.executable.try_clone().unwrap();
        assert!(std::io::Write::write_all(&mut writable_view, b"tamper").is_err());

        fs::write(&path, replacement).unwrap();
        fs::remove_file(&path).unwrap();
        fs::write(&path, "#!/bin/sh\nexit 77\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();

        let profile = profile_for(CredentialSource::Helper, resolver.reference_sha256().into());
        let credential = resolver
            .start(&profile, Duration::from_secs(1))
            .unwrap()
            .wait()
            .unwrap();
        assert_eq!(credential.bytes, b"bound-value");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn helper_stage_is_not_inherited_by_concurrent_unrelated_children() {
        let script = "#!/bin/sh\n/bin/sleep 0.1\nprintf \"{\\\"version\\\":1,\\\"credential\\\":\\\"bound-value\\\"}\"\n";
        let (resolver, path) = helper(script);
        let profile = profile_for(CredentialSource::Helper, resolver.reference_sha256().into());
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let helper_barrier = barrier.clone();
        let helper_thread = std::thread::spawn(move || {
            helper_barrier.wait();
            resolver
                .start(&profile, Duration::from_secs(1))
                .unwrap()
                .wait()
                .unwrap()
        });
        let child_barrier = barrier.clone();
        let child_thread = std::thread::spawn(move || {
            child_barrier.wait();
            for _ in 0..16 {
                let status = Command::new("/bin/sh")
                    .arg("-c")
                    .arg("for link in /proc/self/fd/*; do case \"$(readlink \"$link\" 2>/dev/null)\" in *asb-credential-helper-v1*) exit 31;; esac; done")
                    .env_clear()
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .unwrap();
                assert!(status.success());
            }
        });
        barrier.wait();
        let credential = helper_thread.join().unwrap();
        child_thread.join().unwrap();
        assert_eq!(credential.bytes, b"bound-value");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn helper_rejects_identity_mode_profile_and_deadline() {
        let script = "#!/bin/sh\nprintf \"{\\\"version\\\":1,\\\"credential\\\":\\\"synthetic-value\\\"}\"\n";
        let (descriptor, path, _) = helper_file(script);
        assert!(matches!(
            HelperCredentialResolver::new(descriptor, "provider.primary", &"0".repeat(64)),
            Err(CredentialResolutionError::InvalidHelper)
        ));
        fs::remove_file(path).unwrap();

        let (descriptor, path, digest) = helper_file(script);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o720)).unwrap();
        assert!(matches!(
            HelperCredentialResolver::new(descriptor, "provider.primary", &digest),
            Err(CredentialResolutionError::InvalidHelper)
        ));
        fs::remove_file(path).unwrap();

        let (resolver, path) = helper(script);
        let wrong = profile(resolver.reference_sha256().into());
        assert!(matches!(
            resolver.start(&wrong, Duration::from_secs(1)),
            Err(HelperCredentialError::Resolution(
                CredentialResolutionError::ReferenceMismatch
            ))
        ));
        fs::remove_file(path).unwrap();

        let (resolver, path) = helper(script);
        let profile = profile_for(CredentialSource::Helper, resolver.reference_sha256().into());
        assert!(matches!(
            resolver.start(&profile, Duration::ZERO),
            Err(HelperCredentialError::Resolution(
                CredentialResolutionError::InvalidHelper
            ))
        ));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn helper_rejects_malformed_extra_stderr_nonzero_and_oversize() {
        let cases = [
            (
                "#!/bin/sh\nprintf invalid\n",
                HelperCredentialError::InvalidResponse,
            ),
            (
                "#!/bin/sh\nprintf \"{\\\"version\\\":1,\\\"credential\\\":\\\"value\\\",\\\"extra\\\":1}\"\n",
                HelperCredentialError::InvalidResponse,
            ),
            (
                "#!/bin/sh\nprintf rejected >&2\nprintf \"{\\\"version\\\":1,\\\"credential\\\":\\\"value\\\"}\"\n",
                HelperCredentialError::Failed,
            ),
            ("#!/bin/sh\nexit 7\n", HelperCredentialError::Failed),
            (
                "#!/bin/sh\n/usr/bin/head -c 20000 /dev/zero\n",
                HelperCredentialError::Failed,
            ),
        ];
        for (script, expected) in cases {
            let (resolver, path) = helper(script);
            let profile = profile_for(CredentialSource::Helper, resolver.reference_sha256().into());
            let error = match resolver
                .start(&profile, Duration::from_secs(1))
                .unwrap()
                .wait()
            {
                Ok(_) => panic!("invalid helper response was accepted"),
                Err(error) => error,
            };
            assert_eq!(
                std::mem::discriminant(&error),
                std::mem::discriminant(&expected)
            );
            assert!(!error.to_string().contains("value"));
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn helper_timeout_and_double_cancel_reap_process() {
        let script = "#!/bin/sh\n/bin/sleep 2\n";
        let (resolver, path) = helper(script);
        let profile = profile_for(CredentialSource::Helper, resolver.reference_sha256().into());
        assert!(matches!(
            resolver
                .start(&profile, Duration::from_millis(20))
                .unwrap()
                .wait(),
            Err(HelperCredentialError::TimedOut)
        ));
        fs::remove_file(path).unwrap();

        let (resolver, path) = helper(script);
        let profile = profile_for(CredentialSource::Helper, resolver.reference_sha256().into());
        let mut pending = resolver.start(&profile, Duration::from_secs(1)).unwrap();
        pending.cancel().unwrap();
        pending.cancel().unwrap();
        assert!(matches!(
            pending.wait(),
            Err(HelperCredentialError::Cancelled)
        ));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn owned_descriptor_resolves_once_without_exposing_locator() {
        let (descriptor, path) = descriptor_file(b"synthetic-value");
        let resolver =
            FileDescriptorCredentialResolver::new(descriptor, "provider.primary").unwrap();
        assert!(!format!("{resolver:?}").contains("provider.primary"));
        let profile = profile_for(
            CredentialSource::FileDescriptor,
            resolver.reference_sha256().into(),
        );
        let resolved = resolver.resolve(&profile).unwrap();
        assert_eq!(resolved.bytes, b"synthetic-value");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn descriptor_rejects_wrong_source_type_mode_size_and_locator() {
        let (descriptor, path) = descriptor_file(b"synthetic-value");
        let resolver =
            FileDescriptorCredentialResolver::new(descriptor, "provider.primary").unwrap();
        assert!(matches!(
            resolver.resolve(&profile("a".repeat(64))),
            Err(CredentialResolutionError::ReferenceMismatch)
        ));
        fs::remove_file(path).unwrap();

        for locator in ["", "has space", "has/slash"] {
            let (descriptor, path) = descriptor_file(b"synthetic-value");
            assert!(matches!(
                FileDescriptorCredentialResolver::new(descriptor, locator),
                Err(CredentialResolutionError::InvalidLocator)
            ));
            fs::remove_file(path).unwrap();
        }

        let (descriptor, path) = descriptor_file(b"synthetic-value");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        assert!(matches!(
            FileDescriptorCredentialResolver::new(descriptor, "provider.primary"),
            Err(CredentialResolutionError::InvalidDescriptor)
        ));
        fs::remove_file(path).unwrap();

        let (descriptor, path) = descriptor_file(&vec![120; MAX_CREDENTIAL_BYTES + 1]);
        assert!(matches!(
            FileDescriptorCredentialResolver::new(descriptor, "provider.primary"),
            Err(CredentialResolutionError::InvalidDescriptor)
        ));
        fs::remove_file(path).unwrap();

        let directory = File::open(std::env::temp_dir()).unwrap();
        assert!(matches!(
            FileDescriptorCredentialResolver::new(directory.into(), "provider.primary"),
            Err(CredentialResolutionError::InvalidDescriptor)
        ));
    }

    #[test]
    fn explicit_reference_resolves_and_is_stable() {
        let resolver = EnvironmentCredentialResolver::new("ASB_TEST_KEY").unwrap();
        assert_eq!(
            resolver.reference_sha256(),
            environment_reference_sha256(OsStr::new("ASB_TEST_KEY")).unwrap()
        );
        let resolved = resolver
            .resolve_with(&profile(resolver.reference_sha256().into()), |name| {
                assert_eq!(name, OsStr::new("ASB_TEST_KEY"));
                Some(OsString::from("synthetic-value"))
            })
            .unwrap();
        assert_eq!(resolved.bytes.len(), 15);
    }

    #[test]
    fn mismatched_absent_and_unsupported_sources_fail_without_lookup() {
        let resolver = EnvironmentCredentialResolver::new("ASB_TEST_KEY").unwrap();
        let mut calls = 0;
        assert!(matches!(
            resolver.resolve_with(&profile("b".repeat(64)), |_| {
                calls += 1;
                None
            }),
            Err(CredentialResolutionError::ReferenceMismatch)
        ));
        assert_eq!(calls, 0);
        assert!(matches!(
            resolver.resolve_with(&profile(resolver.reference_sha256().into()), |_| None),
            Err(CredentialResolutionError::Unavailable)
        ));
        for source in [CredentialSource::FileDescriptor, CredentialSource::Helper] {
            let mut unsupported = profile(resolver.reference_sha256().into());
            unsupported.credential.source = source;
            unsupported.refresh_settings_sha256().unwrap();
            assert!(matches!(
                resolver.resolve_with(&unsupported, |_| {
                    calls += 1;
                    None
                }),
                Err(CredentialResolutionError::ReferenceMismatch)
            ));
        }
        assert_eq!(calls, 0);
    }

    #[test]
    fn names_and_values_are_strictly_bounded() {
        for name in ["", "lower", "1START", "HAS-DASH"] {
            assert_eq!(
                EnvironmentCredentialResolver::new(name),
                Err(CredentialResolutionError::InvalidEnvironmentName)
            );
        }
        let resolver = EnvironmentCredentialResolver::new("ASB_TEST_KEY").unwrap();
        let profile = profile(resolver.reference_sha256().into());
        for value in [
            OsString::new(),
            OsString::from("has space"),
            OsString::from("line\nbreak"),
        ] {
            assert!(matches!(
                resolver.resolve_with(&profile, |_| Some(value.clone())),
                Err(CredentialResolutionError::InvalidValue)
            ));
        }
        assert!(matches!(
            resolver.resolve_with(&profile, |_| Some(OsString::from(
                "x".repeat(MAX_CREDENTIAL_BYTES + 1)
            ))),
            Err(CredentialResolutionError::InvalidValue)
        ));
    }

    #[test]
    fn child_gets_only_target_credential_and_public_output_is_clean() {
        let resolver = EnvironmentCredentialResolver::new("ASB_TEST_KEY").unwrap();
        let profile = profile(resolver.reference_sha256().into());
        let resolved = resolver
            .resolve_with(&profile, |_| Some(OsString::from("synthetic-value")))
            .unwrap();
        let scratch = std::env::temp_dir().join(format!("asb-credential-{}", std::process::id()));
        fs::create_dir_all(&scratch).unwrap();
        let script = scratch.join("check.sh");
        fs::write(
            &script,
            "test \"$TARGET_KEY\" = synthetic-value\ntest \"$(env | wc -l)\" -eq 1\nprintf isolated\n",
        )
        .unwrap();
        let limits = ProcessLimits::new(
            1024,
            1024,
            Duration::from_secs(2),
            Duration::from_millis(50),
            Duration::from_millis(5),
        )
        .unwrap();
        let mut child = resolved
            .spawn(
                Path::new("/bin/sh"),
                [&script],
                OsStr::new("TARGET_KEY"),
                limits,
            )
            .unwrap();
        let output = child.wait().unwrap();
        assert_eq!(output.exit_code, Some(0));
        assert_eq!(output.stdout.bytes, b"isolated");
        assert!(
            !output
                .stdout
                .bytes
                .windows(15)
                .any(|window| window == b"synthetic-value")
        );
        fs::remove_dir_all(scratch).unwrap();
    }
}
