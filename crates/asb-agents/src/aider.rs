// SPDX-License-Identifier: MIT
//! Bounded adapter for aider's noninteractive command-line interface.

use asb_protocol::{
    Capability, Event, ExtensionEvent, ExtensionKind, ExtensionManifest, Id, PROTOCOL_V1, RpcError,
    TerminalStatus,
};
use asb_runtime::{ProcessError, ProcessLimits, RunningProcess, Termination};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use url::Url;

/// Aider release whose batch interface this module implements.
pub const SUPPORTED_VERSION: &str = "0.86.2";
/// Immutable upstream Git revision inspected for this adapter.
pub const UPSTREAM_REVISION: &str = "253f0368b873ba30d8ee26e463718f0c03614ddf";
/// SHA-256 of the inspected `aider-chat==0.86.2` universal wheel.
pub const WHEEL_SHA256: &str = "64f6a0c66c9f4633ad9f479bca3e64ebcba02b9da03c6b604b74a44736b2416e";
/// SHA-256 of the natively exercised CPython 3.12 Linux x86_64 runtime.
pub const TESTED_PYTHON_LINUX_X86_64_SHA256: &str =
    "1643dacd9feaedc58f3cc581e4d22577dfe25c09b10282936186ccf0f2e61118";
/// Largest accepted prompt, in UTF-8 bytes.
pub const MAX_PROMPT_BYTES: usize = 4 * 1024 * 1024;
const MAX_ARTIFACT_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_ENDPOINT_BYTES: usize = 4 * 1024;
const MAX_MODEL_BYTES: usize = 256;
const MAX_WORKSPACE_FILES: usize = 4_096;
const MAX_WORKSPACE_ENTRIES: usize = 16_384;
const MAX_WORKSPACE_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_WORKSPACE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_PATH_BYTES: usize = 4 * 1024;
const CLOSED_PROXY: &str = "http://127.0.0.1:9";
const KNOWN_AIDER_EGRESS: &[&str] = &[
    "aider.chat",
    "api.github.com",
    "openrouter.ai",
    "pypi.org",
    "raw.githubusercontent.com",
    "us.i.posthog.com",
];

/// Content-pinned aider artifact understood by this adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AiderArtifact {
    /// Aider 0.86.2 wheel exercised with CPython 3.12 on Linux x86_64.
    LinuxX86_64V0_86_2,
}

impl AiderArtifact {
    const fn digest(self) -> &'static str {
        match self {
            Self::LinuxX86_64V0_86_2 => WHEEL_SHA256,
        }
    }
}

/// Validated immutable inputs for one aider installation.
#[derive(Clone, Debug)]
pub struct AiderConfig {
    python: PathBuf,
    wheel: PathBuf,
    workspace: PathBuf,
    state_root: PathBuf,
    endpoint: Url,
    model: String,
    artifact: AiderArtifact,
    #[cfg(test)]
    wheel_digest_override: Option<String>,
    #[cfg(test)]
    python_digest_override: Option<String>,
}

/// Configuration or production-boundary failure.
#[derive(Debug)]
pub enum AdapterError {
    /// A path required to isolate execution was not absolute.
    RelativePath(&'static str),
    /// The provider endpoint was not a safe HTTP(S) base URL.
    InvalidEndpoint,
    /// The provider/model identifier was malformed.
    InvalidModel,
    /// Prompt input cannot be represented safely at the native process boundary.
    InvalidPrompt,
    /// Prompt input exceeded the documented byte ceiling.
    PromptTooLarge,
    /// Local preparation or digest inspection failed.
    Io(io::Error),
    /// The aider wheel did not match its content pin.
    WheelMismatch,
    /// The required Python runtime did not match its content pin.
    RuntimeMismatch,
    /// Process execution failed.
    Process(ProcessError),
    /// The editable workspace could not be represented as a bounded regular-file set.
    UnsafeWorkspace,
    /// Prompt-bearing state was not rooted at the exact canonical directory requested.
    UnsafeStateRoot,
    /// Cleanup of prompt-bearing state failed.
    CleanupFailed,
}

impl fmt::Display for AdapterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RelativePath(label) => write!(formatter, "{label} must be absolute"),
            Self::InvalidEndpoint => formatter.write_str("invalid provider endpoint"),
            Self::InvalidModel => formatter.write_str("invalid provider/model identifier"),
            Self::InvalidPrompt => formatter.write_str("invalid aider prompt"),
            Self::PromptTooLarge => formatter.write_str("aider prompt exceeds byte limit"),
            Self::Io(error) => write!(formatter, "adapter I/O failed: {error}"),
            Self::WheelMismatch => formatter.write_str("aider wheel pin mismatch"),
            Self::RuntimeMismatch => formatter.write_str("aider Python runtime pin mismatch"),
            Self::Process(error) => write!(formatter, "aider process failed: {error}"),
            Self::UnsafeWorkspace => formatter.write_str("unsafe or excessive aider workspace"),
            Self::UnsafeStateRoot => formatter.write_str("unsafe aider state root"),
            Self::CleanupFailed => formatter.write_str("aider private run-state cleanup failed"),
        }
    }
}

impl std::error::Error for AdapterError {}

impl From<io::Error> for AdapterError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<ProcessError> for AdapterError {
    fn from(value: ProcessError) -> Self {
        Self::Process(value)
    }
}

impl AiderConfig {
    /// Validate paths, endpoint, model, and select a content-pinned artifact.
    pub fn new(
        python: impl Into<PathBuf>,
        wheel: impl Into<PathBuf>,
        workspace: impl Into<PathBuf>,
        state_root: impl Into<PathBuf>,
        endpoint: Url,
        model: impl Into<String>,
        artifact: AiderArtifact,
    ) -> Result<Self, AdapterError> {
        let python = python.into();
        let wheel = wheel.into();
        let workspace = workspace.into();
        let state_root = state_root.into();
        for (label, path) in [
            ("Python runtime", python.as_path()),
            ("aider wheel", wheel.as_path()),
            ("workspace", workspace.as_path()),
            ("state root", state_root.as_path()),
        ] {
            if !path.is_absolute() {
                return Err(AdapterError::RelativePath(label));
            }
        }
        if endpoint.as_str().len() > MAX_ENDPOINT_BYTES
            || !matches!(endpoint.scheme(), "http" | "https")
            || endpoint.cannot_be_a_base()
            || endpoint.username() != ""
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
        {
            return Err(AdapterError::InvalidEndpoint);
        }
        let host = endpoint.host_str().ok_or(AdapterError::InvalidEndpoint)?;
        if endpoint.scheme() == "http" && !is_loopback(host) {
            return Err(AdapterError::InvalidEndpoint);
        }
        if KNOWN_AIDER_EGRESS
            .iter()
            .any(|known| no_proxy_scope_includes(host, known))
        {
            return Err(AdapterError::InvalidEndpoint);
        }
        let model = model.into();
        if model.is_empty()
            || model.len() > MAX_MODEL_BYTES
            || !model.bytes().all(model_component_byte)
        {
            return Err(AdapterError::InvalidModel);
        }
        Ok(Self {
            python,
            wheel,
            workspace,
            state_root,
            endpoint,
            model,
            artifact,
            #[cfg(test)]
            wheel_digest_override: None,
            #[cfg(test)]
            python_digest_override: None,
        })
    }

    /// Produce the explicit capabilities for this exact installation.
    #[must_use]
    pub fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            extension_id: Id("agent.aider".into()),
            kind: ExtensionKind::Agent,
            implementation_version: format!("aider-{SUPPORTED_VERSION}+asb-0.1.0"),
            protocol: PROTOCOL_V1,
            capabilities: BTreeSet::from([Capability::Cancellation]),
            executable_sha256: self.artifact.digest().into(),
        }
    }

    /// Verify the exact wheel and Python runtime without importing aider.
    pub fn verify_executable(&self) -> Result<(), AdapterError> {
        match digest_file(&self.wheel) {
            Ok(digest) if digest == self.wheel_verification_digest() => {}
            Ok(_) | Err(AdapterError::WheelMismatch) => return Err(AdapterError::WheelMismatch),
            Err(error) => return Err(error),
        }
        match digest_file(&self.python) {
            Ok(digest) if digest == self.python_verification_digest() => Ok(()),
            Ok(_) | Err(AdapterError::WheelMismatch) => Err(AdapterError::RuntimeMismatch),
            Err(error) => Err(error),
        }
    }

    fn wheel_verification_digest(&self) -> &str {
        #[cfg(test)]
        if let Some(digest) = &self.wheel_digest_override {
            return digest;
        }
        self.artifact.digest()
    }

    fn python_verification_digest(&self) -> &str {
        #[cfg(test)]
        if let Some(digest) = &self.python_digest_override {
            return digest;
        }
        TESTED_PYTHON_LINUX_X86_64_SHA256
    }

    /// Start one noninteractive edit attempt using a private prompt file.
    pub fn start(
        &self,
        session_id: Id,
        attempt_id: Id,
        prompt: &str,
        limits: ProcessLimits,
    ) -> Result<RunningAider, AdapterError> {
        if prompt.len() > MAX_PROMPT_BYTES {
            return Err(AdapterError::PromptTooLarge);
        }
        if prompt.contains('\0') {
            return Err(AdapterError::InvalidPrompt);
        }
        self.verify_executable()?;
        if !has_canonical_existing_ancestor(&self.workspace)? {
            return Err(AdapterError::UnsafeWorkspace);
        }
        fs::create_dir_all(&self.workspace)?;
        let editable_files = collect_editable_files(&self.workspace)?;
        if !has_canonical_existing_ancestor(&self.state_root)? {
            return Err(AdapterError::UnsafeStateRoot);
        }
        fs::create_dir_all(&self.state_root)?;
        if !is_exact_canonical_directory(&self.state_root)? {
            return Err(AdapterError::UnsafeStateRoot);
        }
        let run_root = self.unique_run_root()?;
        fs::DirBuilder::new().mode(0o700).create(&run_root)?;
        let prepared = (|| {
            for directory in ["home", "cache", "config", "data", "tmp"] {
                fs::DirBuilder::new()
                    .mode(0o700)
                    .create(run_root.join(directory))?;
            }
            for file in ["empty.yml", "empty.env"] {
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(run_root.join(file))?;
            }
            fs::write(run_root.join("empty.yml"), "{}\n")?;
            let prompt_path = run_root.join("prompt.txt");
            let mut prompt_file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&prompt_path)?;
            prompt_file.write_all(prompt.as_bytes())?;
            prompt_file.sync_all()?;
            Ok::<PathBuf, AdapterError>(prompt_path)
        })();
        let prompt_path = match prepared {
            Ok(path) => path,
            Err(error) => {
                let _ = fs::remove_dir_all(&run_root);
                return Err(error);
            }
        };
        let process = match self.spawn_process(&run_root, &prompt_path, &editable_files, limits) {
            Ok(process) => process,
            Err(error) => {
                let _ = fs::remove_dir_all(&run_root);
                return Err(error);
            }
        };
        Ok(RunningAider {
            process,
            session_id,
            attempt_id,
            run_root: Some(run_root),
        })
    }

    fn spawn_process(
        &self,
        run_root: &Path,
        prompt_path: &Path,
        editable_files: &[PathBuf],
        limits: ProcessLimits,
    ) -> Result<RunningProcess, AdapterError> {
        let host = self
            .endpoint
            .host_str()
            .ok_or(AdapterError::InvalidEndpoint)?;
        let site_packages = self
            .python
            .parent()
            .and_then(Path::parent)
            .map(|root| root.join("lib/python3.12/site-packages"))
            .ok_or(AdapterError::RuntimeMismatch)?;
        let python_path = format!("{}:{}", self.wheel.display(), site_packages.display());
        let mut command = Command::new(&self.python);
        command
            .current_dir(&self.workspace)
            .args(["-P", "-m", "aider.main"])
            .args(["--model", &format!("openai/{}", self.model)])
            .args(["--openai-api-base", self.endpoint.as_str()])
            .arg("--message-file")
            .arg(prompt_path)
            .arg("--config")
            .arg(run_root.join("empty.yml"))
            .arg("--env-file")
            .arg(run_root.join("empty.env"))
            .args([
                "--no-git",
                "--no-gitignore",
                "--no-auto-commits",
                "--no-dirty-commits",
                "--no-analytics",
                "--no-check-update",
                "--no-show-release-notes",
                "--no-pretty",
                "--no-fancy-input",
                "--no-stream",
                "--yes-always",
                "--no-suggest-shell-commands",
                "--disable-playwright",
                "--no-detect-urls",
                "--no-auto-lint",
                "--no-auto-test",
                "--no-cache-prompts",
            ])
            .arg("--input-history-file")
            .arg(run_root.join("input.history"))
            .arg("--chat-history-file")
            .arg(run_root.join("chat.history"))
            .args(editable_files)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", run_root.join("home"))
            .env("XDG_CONFIG_HOME", run_root.join("config"))
            .env("XDG_DATA_HOME", run_root.join("data"))
            .env("XDG_CACHE_HOME", run_root.join("cache"))
            .env("TMPDIR", run_root.join("tmp"))
            .env("PYTHONPATH", python_path)
            .env("OPENAI_API_KEY", "asb-credential-free")
            .env("HTTP_PROXY", CLOSED_PROXY)
            .env("HTTPS_PROXY", CLOSED_PROXY)
            .env("ALL_PROXY", CLOSED_PROXY)
            .env("http_proxy", CLOSED_PROXY)
            .env("https_proxy", CLOSED_PROXY)
            .env("all_proxy", CLOSED_PROXY)
            .env("NO_PROXY", host)
            .env("no_proxy", host);
        RunningProcess::spawn(command, limits).map_err(AdapterError::Process)
    }

    fn unique_run_root(&self) -> Result<PathBuf, AdapterError> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| AdapterError::Io(io::Error::other(error)))?
            .as_nanos();
        Ok(self
            .state_root
            .join(format!("attempt-{}-{nonce}", std::process::id())))
    }
}

fn collect_editable_files(root: &Path) -> Result<Vec<PathBuf>, AdapterError> {
    if !is_exact_canonical_directory(root)? {
        return Err(AdapterError::UnsafeWorkspace);
    }
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    let mut entries_seen = 0_usize;
    let mut bytes_seen = 0_u64;
    while let Some(directory) = pending.pop() {
        let mut entries = fs::read_dir(directory)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(AdapterError::Io)?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            entries_seen = entries_seen
                .checked_add(1)
                .ok_or(AdapterError::UnsafeWorkspace)?;
            if entries_seen > MAX_WORKSPACE_ENTRIES {
                return Err(AdapterError::UnsafeWorkspace);
            }
            let path = entry.path();
            if path.as_os_str().as_encoded_bytes().len() > MAX_PATH_BYTES {
                return Err(AdapterError::UnsafeWorkspace);
            }
            let kind = entry.file_type()?;
            if kind.is_symlink() || !(kind.is_dir() || kind.is_file()) {
                return Err(AdapterError::UnsafeWorkspace);
            }
            if kind.is_dir() {
                pending.push(path);
            } else {
                let bytes = entry.metadata()?.len();
                if bytes > MAX_WORKSPACE_FILE_BYTES {
                    return Err(AdapterError::UnsafeWorkspace);
                }
                bytes_seen = bytes_seen
                    .checked_add(bytes)
                    .ok_or(AdapterError::UnsafeWorkspace)?;
                if bytes_seen > MAX_WORKSPACE_BYTES {
                    return Err(AdapterError::UnsafeWorkspace);
                }
                files.push(path);
                if files.len() > MAX_WORKSPACE_FILES {
                    return Err(AdapterError::UnsafeWorkspace);
                }
            }
        }
    }
    files.sort();
    Ok(files)
}

fn is_exact_canonical_directory(path: &Path) -> Result<bool, AdapterError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
        return Ok(false);
    }
    Ok(fs::canonicalize(path)? == path)
}

fn has_canonical_existing_ancestor(path: &Path) -> Result<bool, AdapterError> {
    let mut candidate = path;
    loop {
        match fs::symlink_metadata(candidate) {
            Ok(metadata) => {
                return Ok(metadata.file_type().is_dir()
                    && !metadata.file_type().is_symlink()
                    && fs::canonicalize(candidate)? == candidate);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                candidate = candidate.parent().ok_or(AdapterError::UnsafeWorkspace)?;
            }
            Err(error) => return Err(AdapterError::Io(error)),
        }
    }
}

fn is_loopback(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "::1" | "[::1]" | "localhost")
}

fn no_proxy_scope_includes(provider_host: &str, target: &str) -> bool {
    let provider_host = provider_host.trim_end_matches('.').to_ascii_lowercase();
    target == provider_host
        || target
            .strip_suffix(&provider_host)
            .is_some_and(|prefix| prefix.ends_with('.'))
}

const fn model_component_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':' | b'/')
}

/// A cancellable aider process whose raw output is never exposed.
pub struct RunningAider {
    process: RunningProcess,
    session_id: Id,
    attempt_id: Id,
    run_root: Option<PathBuf>,
}

impl RunningAider {
    /// Native process identifier for metrics and ownership evidence.
    #[must_use]
    pub fn pid(&self) -> u32 {
        self.process.pid()
    }

    /// Request idempotent cancellation of the whole owned process group.
    pub fn cancel(&mut self) -> Result<(), AdapterError> {
        self.process.cancel().map_err(AdapterError::Process)
    }

    /// Reap the process, discard raw output, and remove prompt-bearing state.
    pub fn wait(&mut self) -> Result<AiderOutcome, AdapterError> {
        let waited = self.process.wait().cloned();
        let output = match waited {
            Ok(output) => output,
            Err(error) => {
                let _ = self.remove_run_root();
                return Err(error.into());
            }
        };
        self.remove_run_root()?;
        let status = match output.termination {
            Termination::Cancelled => TerminalStatus::Cancelled,
            Termination::TimedOut => TerminalStatus::Failed,
            Termination::Exited if output.exit_code == Some(0) => TerminalStatus::Completed,
            Termination::Exited => TerminalStatus::Failed,
        };
        Ok(AiderOutcome {
            events: terminal_events(&self.session_id, &self.attempt_id, status),
            status,
            exit_code: output.exit_code,
            stdout_truncated: output.stdout.truncated,
            stderr_truncated: output.stderr.truncated,
            retry_observation: RetryObservation::Unavailable {
                reason: RetryUnavailableReason::UnstructuredBatchDiagnostics,
            },
        })
    }

    fn remove_run_root(&mut self) -> Result<(), AdapterError> {
        let Some(path) = self.run_root.as_ref() else {
            return Ok(());
        };
        match fs::remove_dir_all(path) {
            Ok(()) => self.run_root = None,
            Err(error) if error.kind() == io::ErrorKind::NotFound => self.run_root = None,
            Err(_) => return Err(AdapterError::CleanupFailed),
        }
        Ok(())
    }
}

impl Drop for RunningAider {
    fn drop(&mut self) {
        let _ = self.process.cancel();
        if self.process.wait().is_ok() {
            let _ = self.remove_run_root();
        }
    }
}

/// Privacy-filtered terminal evidence for one aider attempt.
#[derive(Clone, Debug, PartialEq)]
pub struct AiderOutcome {
    events: Vec<ExtensionEvent>,
    status: TerminalStatus,
    exit_code: Option<i32>,
    stdout_truncated: bool,
    stderr_truncated: bool,
    retry_observation: RetryObservation,
}

/// Evidence available for retries performed inside aider's unstructured batch contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetryObservation {
    /// The native process may retry, but its supported interface exposes no safe structured count.
    Unavailable {
        /// Why ASB cannot report a retry count for this attempt.
        reason: RetryUnavailableReason,
    },
}

/// Stable reason that retry evidence is unavailable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetryUnavailableReason {
    /// aider 0.86.2 reports retries only in content-bearing human diagnostics.
    UnstructuredBatchDiagnostics,
}

impl AiderOutcome {
    /// Ordered ASB lifecycle events without prompt or model content.
    #[must_use]
    pub fn events(&self) -> &[ExtensionEvent] {
        &self.events
    }

    /// Terminal status derived from the native process boundary.
    #[must_use]
    pub const fn status(&self) -> TerminalStatus {
        self.status
    }

    /// Native exit code when the process exited normally.
    #[must_use]
    pub const fn exit_code(&self) -> Option<i32> {
        self.exit_code
    }

    /// Whether discarded standard output exceeded its retention limit.
    #[must_use]
    pub const fn stdout_truncated(&self) -> bool {
        self.stdout_truncated
    }

    /// Whether discarded diagnostic output exceeded its retention limit.
    #[must_use]
    pub const fn stderr_truncated(&self) -> bool {
        self.stderr_truncated
    }

    /// Typed evidence boundary for retries internal to aider.
    #[must_use]
    pub const fn retry_observation(&self) -> RetryObservation {
        self.retry_observation
    }
}

fn terminal_events(
    session_id: &Id,
    attempt_id: &Id,
    status: TerminalStatus,
) -> Vec<ExtensionEvent> {
    let mut events = vec![
        event(session_id, attempt_id, 0, Event::Ready),
        event(session_id, attempt_id, 1, Event::RequestStarted),
    ];
    match status {
        TerminalStatus::Completed => {
            events.push(event(session_id, attempt_id, 2, Event::Completed));
        }
        TerminalStatus::Failed => events.push(event(
            session_id,
            attempt_id,
            2,
            Event::Failed(RpcError {
                code: -32_100,
                message: "aider attempt failed; raw diagnostics are not retained".into(),
                data: None,
            }),
        )),
        TerminalStatus::Cancelled => {}
    }
    events
}

fn event(session_id: &Id, attempt_id: &Id, sequence: u64, event: Event) -> ExtensionEvent {
    ExtensionEvent {
        session_id: session_id.clone(),
        attempt_id: attempt_id.clone(),
        sequence,
        event,
    }
}

fn digest_file(path: &Path) -> Result<String, AdapterError> {
    let mut file = fs::File::open(path)?;
    let size = file.metadata()?.len();
    if size == 0 || size > MAX_ARTIFACT_BYTES {
        return Err(AdapterError::WheelMismatch);
    }
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::time::Duration;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir()
                .join(format!("asb-aider-{label}-{}-{nonce}", std::process::id()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn limits() -> ProcessLimits {
        ProcessLimits::new(
            64 * 1024,
            64 * 1024,
            Duration::from_secs(2),
            Duration::from_millis(200),
            Duration::from_millis(5),
        )
        .unwrap()
    }

    fn adapter(script: &str) -> (Scratch, AiderConfig) {
        let scratch = Scratch::new("process");
        let python = scratch.0.join("python");
        let wheel = scratch.0.join("aider.whl");
        fs::write(&python, script).unwrap();
        fs::set_permissions(&python, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(&wheel, "wheel").unwrap();
        let mut config = AiderConfig::new(
            &python,
            &wheel,
            scratch.0.join("workspace"),
            scratch.0.join("state"),
            Url::parse("http://127.0.0.1:1/v1").unwrap(),
            "fixture-model",
            AiderArtifact::LinuxX86_64V0_86_2,
        )
        .unwrap();
        config.python_digest_override = Some(digest_file(&python).unwrap());
        config.wheel_digest_override = Some(digest_file(&wheel).unwrap());
        (scratch, config)
    }

    #[test]
    fn configuration_manifest_and_pins_are_bounded() {
        let (_scratch, config) = adapter("#!/bin/sh\nexit 0\n");
        assert_eq!(config.manifest().extension_id.0, "agent.aider");
        assert_eq!(
            config.manifest().capabilities,
            BTreeSet::from([Capability::Cancellation])
        );
        for endpoint in [
            "ftp://example.invalid/v1",
            "http://example.invalid/v1",
            "https://user@example.invalid/v1",
            "https://example.invalid/v1?secret=value",
            "https://us.i.posthog.com/v1",
            "https://posthog.com/v1",
            "https://github.com/v1",
        ] {
            assert!(matches!(
                AiderConfig::new(
                    "/python",
                    "/aider.whl",
                    "/workspace",
                    "/state",
                    Url::parse(endpoint).unwrap(),
                    "model",
                    AiderArtifact::LinuxX86_64V0_86_2,
                ),
                Err(AdapterError::InvalidEndpoint)
            ));
        }
        assert!(matches!(
            AiderConfig::new(
                "python",
                "/aider.whl",
                "/workspace",
                "/state",
                Url::parse("https://example.invalid/v1").unwrap(),
                "model",
                AiderArtifact::LinuxX86_64V0_86_2,
            ),
            Err(AdapterError::RelativePath("Python runtime"))
        ));
        assert!(matches!(
            AiderConfig::new(
                "/python",
                "/aider.whl",
                "/workspace",
                "/state",
                Url::parse("https://example.invalid/v1").unwrap(),
                "bad model\n",
                AiderArtifact::LinuxX86_64V0_86_2,
            ),
            Err(AdapterError::InvalidModel)
        ));
    }

    #[test]
    fn process_boundary_uses_private_prompt_and_disables_ambient_features() {
        let script = r#"#!/bin/sh
[ "$1" = -P ] || exit 91
[ "$2" = -m ] || exit 92
[ "$3" = aider.main ] || exit 93
[ "$OPENAI_API_KEY" = asb-credential-free ] || exit 94
[ "$HTTP_PROXY" = http://127.0.0.1:9 ] || exit 95
[ "$NO_PROXY" = 127.0.0.1 ] || exit 96
case "$HOME" in */home) ;; *) exit 97;; esac
args="$*"
case "$args" in *--message-file*--no-git*--no-auto-commits*--no-analytics*--no-check-update*--no-suggest-shell-commands*--disable-playwright*) ;; *) exit 98;; esac
prompt=
previous=
for item in "$@"; do
  if [ "$previous" = --message-file ]; then prompt="$item"; break; fi
  previous="$item"
done
[ -f "$prompt" ] || exit 99
[ "$(cat "$prompt")" = private-prompt ] || exit 100
printf edited > result.txt
"#;
        let (scratch, config) = adapter(script);
        let mut running = config
            .start(
                Id("session".into()),
                Id("attempt".into()),
                "private-prompt",
                limits(),
            )
            .unwrap();
        let run_root = running.run_root.clone().unwrap();
        let prompt_mode = fs::metadata(run_root.join("prompt.txt"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(prompt_mode, 0o600);
        let outcome = running.wait().unwrap();
        assert_eq!(outcome.status(), TerminalStatus::Completed);
        assert_eq!(outcome.exit_code(), Some(0));
        assert_eq!(
            fs::read_to_string(scratch.0.join("workspace/result.txt")).unwrap(),
            "edited"
        );
        assert!(!run_root.exists());
        assert!(matches!(
            outcome.events().last().unwrap().event,
            Event::Completed
        ));
    }

    #[test]
    fn failure_and_cancellation_are_terminal_and_redacted() {
        let (_scratch, config) = adapter("#!/bin/sh\nprintf secret >&2\nexit 17\n");
        let mut failed = config
            .start(Id("s".into()), Id("a".into()), "secret", limits())
            .unwrap();
        let outcome = failed.wait().unwrap();
        assert_eq!(outcome.status(), TerminalStatus::Failed);
        assert_eq!(outcome.exit_code(), Some(17));
        assert!(!format!("{outcome:?}").contains("secret"));
        assert_eq!(
            outcome.retry_observation(),
            RetryObservation::Unavailable {
                reason: RetryUnavailableReason::UnstructuredBatchDiagnostics,
            }
        );
        let (_scratch, config) =
            adapter("#!/bin/sh\ntrap 'exit 0' TERM\nwhile :; do sleep 1; done\n");
        let mut cancelled = config
            .start(Id("s".into()), Id("a".into()), "prompt", limits())
            .unwrap();
        cancelled.cancel().unwrap();
        let outcome = cancelled.wait().unwrap();
        assert_eq!(outcome.status(), TerminalStatus::Cancelled);
        assert_eq!(outcome.events().len(), 2);
    }

    #[test]
    fn prompt_and_artifact_failures_precede_process_effects() {
        let (scratch, mut config) = adapter("#!/bin/sh\nexit 0\n");
        assert!(matches!(
            config.start(Id("s".into()), Id("a".into()), "bad\0prompt", limits()),
            Err(AdapterError::InvalidPrompt)
        ));
        assert!(matches!(
            config.start(
                Id("s".into()),
                Id("a".into()),
                &"x".repeat(MAX_PROMPT_BYTES + 1),
                limits()
            ),
            Err(AdapterError::PromptTooLarge)
        ));
        config.wheel_digest_override = Some("00".repeat(32));
        assert!(matches!(
            config.verify_executable(),
            Err(AdapterError::WheelMismatch)
        ));
        assert!(!scratch.0.join("state").exists());
    }

    #[test]
    fn workspace_enumeration_is_sorted_bounded_and_rejects_symlinks() {
        let scratch = Scratch::new("workspace");
        let root = scratch.0.join("root");
        fs::create_dir_all(root.join("nested")).unwrap();
        fs::write(root.join("z.txt"), "").unwrap();
        fs::write(root.join("nested/a.txt"), "").unwrap();
        assert_eq!(
            collect_editable_files(&root).unwrap(),
            vec![root.join("nested/a.txt"), root.join("z.txt")]
        );
        std::os::unix::fs::symlink(root.join("z.txt"), root.join("alias")).unwrap();
        assert!(matches!(
            collect_editable_files(&root),
            Err(AdapterError::UnsafeWorkspace)
        ));
        fs::remove_file(root.join("alias")).unwrap();
        let alias_root = scratch.0.join("alias-root");
        std::os::unix::fs::symlink(&root, &alias_root).unwrap();
        assert!(matches!(
            collect_editable_files(&alias_root),
            Err(AdapterError::UnsafeWorkspace)
        ));
        fs::remove_file(&alias_root).unwrap();
        let actual_parent = scratch.0.join("actual-parent");
        fs::create_dir_all(actual_parent.join("child")).unwrap();
        let alias_parent = scratch.0.join("alias-parent");
        std::os::unix::fs::symlink(&actual_parent, &alias_parent).unwrap();
        assert!(matches!(
            collect_editable_files(&alias_parent.join("child")),
            Err(AdapterError::UnsafeWorkspace)
        ));
        fs::remove_file(&alias_parent).unwrap();

        let oversized = root.join("oversized");
        fs::File::create(&oversized)
            .unwrap()
            .set_len(MAX_WORKSPACE_FILE_BYTES + 1)
            .unwrap();
        assert!(matches!(
            collect_editable_files(&root),
            Err(AdapterError::UnsafeWorkspace)
        ));
        fs::remove_file(oversized).unwrap();
        for index in 0..=(MAX_WORKSPACE_BYTES / MAX_WORKSPACE_FILE_BYTES) {
            fs::File::create(root.join(format!("aggregate-{index}")))
                .unwrap()
                .set_len(MAX_WORKSPACE_FILE_BYTES)
                .unwrap();
        }
        assert!(matches!(
            collect_editable_files(&root),
            Err(AdapterError::UnsafeWorkspace)
        ));
    }

    #[test]
    fn state_root_symlink_fails_before_process_execution() {
        let (scratch, mut config) = adapter("#!/bin/sh\nprintf ran > marker\n");
        let outside = scratch.0.join("outside");
        fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, &config.state_root).unwrap();
        assert!(matches!(
            config.start(Id("s".into()), Id("a".into()), "prompt", limits()),
            Err(AdapterError::UnsafeStateRoot)
        ));
        assert!(!scratch.0.join("workspace/marker").exists());
        fs::remove_file(&config.state_root).unwrap();
        let actual_parent = scratch.0.join("actual-state-parent");
        fs::create_dir_all(&actual_parent).unwrap();
        let alias_parent = scratch.0.join("alias-state-parent");
        std::os::unix::fs::symlink(&actual_parent, &alias_parent).unwrap();
        config.state_root = alias_parent.join("state");
        assert!(matches!(
            config.start(Id("s".into()), Id("a".into()), "prompt", limits()),
            Err(AdapterError::UnsafeStateRoot)
        ));
        assert!(!actual_parent.join("state").exists());
        assert!(!scratch.0.join("workspace/marker").exists());

        let actual_workspace_parent = scratch.0.join("actual-workspace-parent");
        fs::create_dir_all(&actual_workspace_parent).unwrap();
        let alias_workspace_parent = scratch.0.join("alias-workspace-parent");
        std::os::unix::fs::symlink(&actual_workspace_parent, &alias_workspace_parent).unwrap();
        config.workspace = alias_workspace_parent.join("workspace");
        config.state_root = scratch.0.join("safe-state");
        assert!(matches!(
            config.start(Id("s".into()), Id("a".into()), "prompt", limits()),
            Err(AdapterError::UnsafeWorkspace)
        ));
        assert!(!actual_workspace_parent.join("workspace").exists());
    }
}
