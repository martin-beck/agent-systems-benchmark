// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Bounded adapter for Goose's no-session stream-JSON interface.

use asb_protocol::{
    Capability, Event, ExtensionEvent, ExtensionKind, ExtensionManifest, Id, PROTOCOL_V1, RpcError,
    TerminalStatus, Usage,
};
use asb_runtime::{ProcessError, ProcessLimits, RunningProcess, Termination};
use serde::Deserialize;
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};
use url::Url;

/// Goose release whose stream-JSON dialect this module implements.
pub const SUPPORTED_VERSION: &str = "1.49.0";
/// Immutable upstream Git revision inspected for this adapter.
pub const UPSTREAM_REVISION: &str = "71fc4be1ed729e26b1dc0a4466abdd03be548a53";
/// SHA-256 of the natively exercised Linux x86_64 musl executable.
pub const TESTED_LINUX_X86_64_MUSL_SHA256: &str =
    "c055ef50579c9ecf90329c12e87af0d136fe723cbaef834fdaac013d410b377d";
/// SHA-256 of the upstream Linux x86_64 musl release archive.
pub const LINUX_X86_64_MUSL_ARCHIVE_SHA256: &str =
    "2715ecb28a2f82ac6298941db40574a44729f0607d87f77953f2690d10fe7cb6";
/// SHA-256 of the upstream Linux aarch64 musl release archive.
pub const LINUX_AARCH64_MUSL_ARCHIVE_SHA256: &str =
    "e0614bcc944b48dc2f3b2a171946a745ad6a57abd0a972ae714adfecb9104b36";
/// SHA-256 of the Linux aarch64 musl executable selected for native CI.
pub const TESTED_LINUX_AARCH64_MUSL_SHA256: &str =
    "aeb66147b9ec9379b384083bec9e58ab3f0f39d1c9faa5a4403c368662d2a046";
/// Largest accepted prompt in UTF-8 bytes.
pub const MAX_PROMPT_BYTES: usize = 4 * 1024 * 1024;
/// Largest individual Goose JSON line.
pub const MAX_EVENT_LINE_BYTES: usize = 16 * 1024 * 1024;
/// Largest number of Goose JSON events per attempt.
pub const MAX_UPSTREAM_EVENTS: usize = 65_536;
/// Largest number of privacy-filtered ASB events retained per attempt.
pub const MAX_MAPPED_EVENTS: usize = 65_536;
/// Largest configured agent turn bound.
pub const MAX_TURNS: u32 = 1_000;

const MAX_EXECUTABLE_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_ENDPOINT_BYTES: usize = 4 * 1024;
const MAX_MODEL_BYTES: usize = 256;
const MAX_ID_BYTES: usize = 4 * 1024;
const MAX_CORRELATION_ID_BYTES: usize = 4 * 1024;
const MAX_CONTENT_ITEMS_PER_MESSAGE: usize = 4_096;
const MAX_TOOL_NAME_BYTES: usize = 256;
const O_NOFOLLOW_CLOEXEC: i32 = 0x000a_0000;
const CLOSED_PROXY: &str = "http://127.0.0.1:9";
const KNOWN_GOOSE_EGRESS: &[&str] = &["us.i.posthog.com"];
const FAILURE_CODE: i32 = -32_100;

/// Content-pinned Goose artifact understood by this adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GooseArtifact {
    /// Goose 1.49.0 static PIE musl executable for Linux x86_64.
    LinuxX86_64MuslV1_49_0,
    /// Goose 1.49.0 static executable for Linux aarch64 musl.
    LinuxAarch64MuslV1_49_0,
}

impl GooseArtifact {
    const fn digest(self) -> &'static str {
        match self {
            Self::LinuxX86_64MuslV1_49_0 => TESTED_LINUX_X86_64_MUSL_SHA256,
            Self::LinuxAarch64MuslV1_49_0 => TESTED_LINUX_AARCH64_MUSL_SHA256,
        }
    }
}

/// Validated immutable inputs for one Goose installation.
#[derive(Clone, Debug)]
pub struct GooseConfig {
    binary: PathBuf,
    workspace: PathBuf,
    state_root: PathBuf,
    endpoint: Url,
    model: String,
    max_turns: u32,
    artifact: GooseArtifact,
    #[cfg(test)]
    verification_digest_override: Option<String>,
    #[cfg(test)]
    replacement_before_spawn: Option<Vec<u8>>,
}

/// Configuration or production-boundary failure.
#[derive(Debug)]
pub enum AdapterError {
    /// A required path was not absolute.
    RelativePath(&'static str),
    /// A caller-owned path was not an exact canonical regular object.
    UnsafePath(&'static str),
    /// The provider endpoint was not safe.
    InvalidEndpoint,
    /// The provider model was malformed.
    InvalidModel,
    /// The turn bound was zero or above the hard ceiling.
    InvalidMaxTurns,
    /// Prompt input exceeded its byte ceiling.
    PromptTooLarge,
    /// A caller correlation identifier was empty or above its byte ceiling.
    InvalidCorrelationId,
    /// Local preparation or inspection failed.
    Io(io::Error),
    /// The executable did not match its pin.
    ExecutableMismatch,
    /// Process execution failed.
    Process(ProcessError),
    /// Structured output or diagnostics were truncated.
    TruncatedOutput,
    /// A required Goose extension did not start cleanly.
    RequiredExtensionUnavailable,
    /// Goose emitted malformed or unsupported structured output.
    InvalidEvent,
}

impl fmt::Display for AdapterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RelativePath(v) => write!(f, "{v} must be absolute"),
            Self::UnsafePath(v) => write!(f, "unsafe {v}"),
            Self::InvalidEndpoint => f.write_str("invalid provider endpoint"),
            Self::InvalidModel => f.write_str("invalid provider model"),
            Self::InvalidMaxTurns => f.write_str("invalid Goose turn bound"),
            Self::PromptTooLarge => f.write_str("Goose prompt exceeds byte limit"),
            Self::InvalidCorrelationId => f.write_str("invalid attempt correlation identifier"),
            Self::Io(e) => write!(f, "adapter I/O failed: {e}"),
            Self::ExecutableMismatch => f.write_str("Goose executable pin mismatch"),
            Self::Process(e) => write!(f, "Goose process failed: {e}"),
            Self::TruncatedOutput => f.write_str("Goose output was truncated"),
            Self::RequiredExtensionUnavailable => {
                f.write_str("required Goose extension unavailable")
            }
            Self::InvalidEvent => f.write_str("invalid Goose structured event"),
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

impl GooseConfig {
    /// Validate inputs and select a pinned artifact.
    pub fn new(
        binary: impl Into<PathBuf>,
        workspace: impl Into<PathBuf>,
        state_root: impl Into<PathBuf>,
        endpoint: Url,
        model: impl Into<String>,
        max_turns: u32,
        artifact: GooseArtifact,
    ) -> Result<Self, AdapterError> {
        let binary = binary.into();
        let workspace = workspace.into();
        let state_root = state_root.into();
        for (label, path) in [
            ("binary", binary.as_path()),
            ("workspace", workspace.as_path()),
            ("state root", state_root.as_path()),
        ] {
            if !path.is_absolute() {
                return Err(AdapterError::RelativePath(label));
            }
        }
        if !valid_endpoint(&endpoint) {
            return Err(AdapterError::InvalidEndpoint);
        }
        let model = model.into();
        if model.is_empty()
            || model.len() > MAX_MODEL_BYTES
            || !model.bytes().all(model_byte)
            || model
                .as_bytes()
                .first()
                .is_some_and(|byte| matches!(byte, b'/' | b'.' | b'-'))
            || model.ends_with('/')
            || model.contains("//")
        {
            return Err(AdapterError::InvalidModel);
        }
        if max_turns == 0 || max_turns > MAX_TURNS {
            return Err(AdapterError::InvalidMaxTurns);
        }
        Ok(Self {
            binary,
            workspace,
            state_root,
            endpoint,
            model,
            max_turns,
            artifact,
            #[cfg(test)]
            verification_digest_override: None,
            #[cfg(test)]
            replacement_before_spawn: None,
        })
    }

    /// Describe capabilities proven for this exact executable.
    #[must_use]
    pub fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            extension_id: Id("agent.goose".into()),
            kind: ExtensionKind::Agent,
            implementation_version: format!("goose-{SUPPORTED_VERSION}+asb-0.1.0"),
            protocol: PROTOCOL_V1,
            capabilities: BTreeSet::from([Capability::Cancellation, Capability::Usage]),
            executable_sha256: self.artifact.digest().into(),
        }
    }

    /// Verify exact, non-symlink executable bytes without running it.
    pub fn verify_executable(&self) -> Result<(), AdapterError> {
        if !exact_file(&self.binary)? {
            return Err(AdapterError::UnsafePath("Goose executable"));
        }
        if digest_file(&self.binary)? == self.verification_digest() {
            Ok(())
        } else {
            Err(AdapterError::ExecutableMismatch)
        }
    }

    fn verification_digest(&self) -> &str {
        #[cfg(test)]
        if let Some(value) = &self.verification_digest_override {
            return value;
        }
        self.artifact.digest()
    }

    /// Start one no-session attempt with prompt text absent from argv and disk.
    pub fn start(
        &self,
        session_id: Id,
        attempt_id: Id,
        prompt: &str,
        limits: ProcessLimits,
    ) -> Result<RunningGoose, AdapterError> {
        if !valid_correlation_id(&session_id) || !valid_correlation_id(&attempt_id) {
            return Err(AdapterError::InvalidCorrelationId);
        }
        if prompt.len() > MAX_PROMPT_BYTES {
            return Err(AdapterError::PromptTooLarge);
        }
        if !exact_dir(&self.workspace)? {
            return Err(AdapterError::UnsafePath("workspace"));
        }
        if !exact_dir(&self.state_root)? {
            return Err(AdapterError::UnsafePath("state root"));
        }
        let run_root = self.unique_run_root()?;
        fs::DirBuilder::new().mode(0o700).create(&run_root)?;
        let prepared = (|| {
            for child in ["home", "goose", "tmp"] {
                fs::DirBuilder::new()
                    .mode(0o700)
                    .create(run_root.join(child))?;
            }
            let executable = self.prepare_executable(&run_root)?;
            #[cfg(test)]
            if let Some(replacement) = &self.replacement_before_spawn {
                fs::write(&self.binary, replacement)?;
            }
            let path = run_root.join("prompt");
            let mut file = OpenOptions::new()
                .write(true)
                .read(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)?;
            fs::remove_file(path)?;
            file.write_all(prompt.as_bytes())?;
            file.sync_all()?;
            file.seek(SeekFrom::Start(0))?;
            Ok::<_, AdapterError>((file, executable))
        })();
        let prompt_file = match prepared {
            Ok(value) => value,
            Err(error) => {
                let _ = fs::remove_dir_all(&run_root);
                return Err(error);
            }
        };
        match self.spawn(&run_root, &prompt_file.1, prompt_file.0, limits) {
            Ok(process) => Ok(RunningGoose {
                process,
                session_id,
                attempt_id,
                run_root: Some(run_root),
            }),
            Err(error) => {
                let _ = fs::remove_dir_all(run_root);
                Err(error)
            }
        }
    }

    fn spawn(
        &self,
        run_root: &Path,
        executable: &Path,
        prompt: fs::File,
        limits: ProcessLimits,
    ) -> Result<RunningProcess, AdapterError> {
        let host = self
            .endpoint
            .host_str()
            .ok_or(AdapterError::InvalidEndpoint)?;
        let turns = self.max_turns.to_string();
        let mut command = Command::new(executable);
        command
            .current_dir(&self.workspace)
            .args([
                "run",
                "--no-session",
                "--no-profile",
                "--with-builtin",
                "developer",
                "--quiet",
                "--output-format",
                "stream-json",
                "--provider",
                "openai",
                "--model",
            ])
            .arg(&self.model)
            .args(["--max-turns", &turns, "--max-tool-repetitions", "8"])
            .args(["--instructions", "-"])
            .stdin(Stdio::from(prompt))
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", run_root.join("home"))
            .env("TMPDIR", run_root.join("tmp"))
            .env("GOOSE_PATH_ROOT", run_root.join("goose"))
            .env("GOOSE_MODE", "auto")
            .env("GOOSE_DISABLE_KEYRING", "true")
            .env("GOOSE_DISABLE_SESSION_NAMING", "true")
            .env("CONTEXT_FILE_NAMES", "[]")
            .env("OPENAI_API_KEY", "asb-credential-free")
            .env("OPENAI_BASE_URL", self.endpoint.as_str())
            .env("HTTP_PROXY", CLOSED_PROXY)
            .env("HTTPS_PROXY", CLOSED_PROXY)
            .env("ALL_PROXY", CLOSED_PROXY)
            .env("http_proxy", CLOSED_PROXY)
            .env("https_proxy", CLOSED_PROXY)
            .env("all_proxy", CLOSED_PROXY)
            .env("NO_PROXY", host)
            .env("no_proxy", host)
            .env("NO_COLOR", "1");
        RunningProcess::spawn(command, limits).map_err(Into::into)
    }

    fn prepare_executable(&self, run_root: &Path) -> Result<PathBuf, AdapterError> {
        if !exact_file(&self.binary)? {
            return Err(AdapterError::UnsafePath("Goose executable"));
        }
        let mut source = OpenOptions::new()
            .read(true)
            .custom_flags(O_NOFOLLOW_CLOEXEC)
            .open(&self.binary)?;
        let metadata = source.metadata()?;
        if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_EXECUTABLE_BYTES {
            return Err(AdapterError::ExecutableMismatch);
        }
        let launch = run_root.join("goose-launch");
        let mut target = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o500)
            .open(&launch)?;
        let mut digest = Sha256::new();
        let mut total = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let count = source.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            total = total
                .checked_add(count as u64)
                .filter(|value| *value <= MAX_EXECUTABLE_BYTES)
                .ok_or(AdapterError::ExecutableMismatch)?;
            digest.update(&buffer[..count]);
            target.write_all(&buffer[..count])?;
        }
        target.sync_all()?;
        if total != metadata.len()
            || format!("{:x}", digest.finalize()) != self.verification_digest()
        {
            return Err(AdapterError::ExecutableMismatch);
        }
        Ok(launch)
    }

    fn unique_run_root(&self) -> Result<PathBuf, AdapterError> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| AdapterError::Io(io::Error::other(e)))?
            .as_nanos();
        Ok(self
            .state_root
            .join(format!("attempt-{}-{nonce}", std::process::id())))
    }
}

/// A cancellable Goose process whose output is not yet collected.
pub struct RunningGoose {
    process: RunningProcess,
    session_id: Id,
    attempt_id: Id,
    run_root: Option<PathBuf>,
}

impl RunningGoose {
    /// Native process identifier.
    #[must_use]
    pub fn pid(&self) -> u32 {
        self.process.pid()
    }

    /// Request idempotent process-group cancellation.
    pub fn cancel(&mut self) -> Result<(), AdapterError> {
        self.process.cancel().map_err(Into::into)
    }

    /// Reap, clean private state, and map bounded structured output.
    pub fn wait(&mut self) -> Result<GooseOutcome, AdapterError> {
        let output = self.process.wait()?.clone();
        self.cleanup()?;
        if output.stdout.truncated || output.stderr.truncated {
            return Err(AdapterError::TruncatedOutput);
        }
        // v1.49.0 exits zero after a requested extension fails to load. Quiet,
        // isolated successful runs emit no stderr, so any diagnostic fails closed.
        if !output.stderr.bytes.is_empty() {
            return Err(AdapterError::RequiredExtensionUnavailable);
        }
        let process_status = match output.termination {
            Termination::Cancelled => TerminalStatus::Cancelled,
            Termination::TimedOut => TerminalStatus::Failed,
            Termination::Exited if output.exit_code == Some(0) => TerminalStatus::Completed,
            Termination::Exited => TerminalStatus::Failed,
        };
        let events = map_events(
            &output.stdout.bytes,
            &self.session_id,
            &self.attempt_id,
            process_status,
        )?;
        // Goose v1.49.0 reports provider failures as assistant messages and
        // exits zero. map_events recognizes those messages by the absence of
        // provider inference metadata and emits a privacy-filtered failure.
        let status = if events
            .last()
            .is_some_and(|event| matches!(event.event, Event::Failed(_)))
        {
            TerminalStatus::Failed
        } else {
            process_status
        };
        Ok(GooseOutcome {
            events,
            status,
            exit_code: output.exit_code,
        })
    }

    fn cleanup(&mut self) -> Result<(), AdapterError> {
        if let Some(path) = &self.run_root {
            match fs::remove_dir_all(path) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        self.run_root = None;
        Ok(())
    }
}

impl Drop for RunningGoose {
    fn drop(&mut self) {
        let _ = self.process.cancel();
        if self.process.wait().is_ok() {
            let _ = self.cleanup();
        }
    }
}

/// Privacy-filtered terminal evidence for one Goose attempt.
///
/// Fields are private so external code cannot construct successful evidence.
///
/// ```compile_fail
/// use asb_agents::goose::GooseOutcome;
/// let _ = GooseOutcome {
///     events: Vec::new(),
///     status: asb_protocol::TerminalStatus::Completed,
///     exit_code: Some(0),
/// };
/// ```
///
/// ```compile_fail
/// fn forge(mut value: asb_agents::goose::GooseOutcome) {
///     value.status = asb_protocol::TerminalStatus::Completed;
/// }
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct GooseOutcome {
    events: Vec<ExtensionEvent>,
    status: TerminalStatus,
    exit_code: Option<i32>,
}

impl GooseOutcome {
    /// Ordered lifecycle events without text, reasoning, arguments, or output.
    #[must_use]
    pub fn events(&self) -> &[ExtensionEvent] {
        &self.events
    }
    /// Terminal status derived from the process boundary.
    #[must_use]
    pub const fn status(&self) -> TerminalStatus {
        self.status
    }
    /// Native exit code, when available.
    #[must_use]
    pub const fn exit_code(&self) -> Option<i32> {
        self.exit_code
    }
}

fn valid_endpoint(endpoint: &Url) -> bool {
    if endpoint.as_str().len() > MAX_ENDPOINT_BYTES
        || !matches!(endpoint.scheme(), "http" | "https")
        || endpoint.cannot_be_a_base()
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
    {
        return false;
    }
    let Some(host) = endpoint.host_str() else {
        return false;
    };
    if KNOWN_GOOSE_EGRESS
        .iter()
        .any(|target| no_proxy_scope_includes(host, target))
    {
        return false;
    }
    endpoint.scheme() != "http" || matches!(host, "127.0.0.1" | "::1" | "[::1]" | "localhost")
}

fn no_proxy_scope_includes(provider_host: &str, target: &str) -> bool {
    let provider_host = provider_host.trim_end_matches(".").to_ascii_lowercase();
    target == provider_host
        || target
            .strip_suffix(&provider_host)
            .is_some_and(|prefix| prefix.ends_with("."))
}

fn valid_correlation_id(value: &Id) -> bool {
    !value.0.is_empty() && value.0.len() <= MAX_CORRELATION_ID_BYTES
}

const fn model_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/' | b':')
}

fn exact_file(path: &Path) -> Result<bool, AdapterError> {
    Ok(fs::symlink_metadata(path)?.file_type().is_file() && fs::canonicalize(path)? == path)
}

fn exact_dir(path: &Path) -> Result<bool, AdapterError> {
    Ok(fs::symlink_metadata(path)?.file_type().is_dir() && fs::canonicalize(path)? == path)
}

fn digest_file(path: &Path) -> Result<String, AdapterError> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(O_NOFOLLOW_CLOEXEC)
        .open(path)?;
    let metadata = file.metadata()?;
    let size = metadata.len();
    if !metadata.is_file() || size == 0 || size > MAX_EXECUTABLE_BYTES {
        return Err(AdapterError::ExecutableMismatch);
    }
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn map_events(
    stdout: &[u8],
    session_id: &Id,
    attempt_id: &Id,
    status: TerminalStatus,
) -> Result<Vec<ExtensionEvent>, AdapterError> {
    let text = std::str::from_utf8(stdout).map_err(|_| AdapterError::InvalidEvent)?;
    let mut events = Vec::new();
    let mut sequence = 0;
    push(
        &mut events,
        &mut sequence,
        session_id,
        attempt_id,
        Event::Ready,
    )?;
    push(
        &mut events,
        &mut sequence,
        session_id,
        attempt_id,
        Event::RequestStarted,
    )?;
    let mut count = 0_usize;
    let mut first = false;
    let mut complete = false;
    let mut failed = false;
    let mut outstanding = BTreeMap::<String, String>::new();
    let mut finished = BTreeSet::<String>::new();
    for line in text.lines().filter(|line| !line.is_empty()) {
        count = count.checked_add(1).ok_or(AdapterError::InvalidEvent)?;
        if count > MAX_UPSTREAM_EVENTS || line.len() > MAX_EVENT_LINE_BYTES || complete {
            return Err(AdapterError::InvalidEvent);
        }
        let value = serde_json::from_str::<UniqueJson>(line)
            .map_err(|_| AdapterError::InvalidEvent)?
            .0;
        let object = value.as_object().ok_or(AdapterError::InvalidEvent)?;
        match string(object, "type")? {
            "message" if !failed => {
                if !first {
                    push(
                        &mut events,
                        &mut sequence,
                        session_id,
                        attempt_id,
                        Event::FirstResponse,
                    )?;
                    first = true;
                }
                failed = map_message(
                    object_value(object, "message")?,
                    &mut outstanding,
                    &mut finished,
                    &mut events,
                    &mut sequence,
                    session_id,
                    attempt_id,
                )?;
            }
            "notification" if !failed => validate_notification(object)?,
            "error" if !failed => {
                bounded_string(object, "error", MAX_EVENT_LINE_BYTES)?;
                failed = true;
            }
            "complete" if outstanding.is_empty() && first => {
                if let Some(usage) = complete_usage(object)? {
                    push(
                        &mut events,
                        &mut sequence,
                        session_id,
                        attempt_id,
                        Event::Usage(usage),
                    )?;
                }
                complete = true;
            }
            _ => return Err(AdapterError::InvalidEvent),
        }
    }
    if !outstanding.is_empty() {
        return Err(AdapterError::InvalidEvent);
    }
    let effective_status = if failed {
        TerminalStatus::Failed
    } else {
        status
    };
    match effective_status {
        TerminalStatus::Completed if complete && !failed => push(
            &mut events,
            &mut sequence,
            session_id,
            attempt_id,
            Event::Completed,
        )?,
        TerminalStatus::Completed => return Err(AdapterError::InvalidEvent),
        TerminalStatus::Failed => push(
            &mut events,
            &mut sequence,
            session_id,
            attempt_id,
            Event::Failed(RpcError {
                code: FAILURE_CODE,
                message: "Goose attempt failed; raw diagnostics are not retained".into(),
                data: None,
            }),
        )?,
        TerminalStatus::Cancelled => {}
    }
    Ok(events)
}

#[allow(clippy::too_many_arguments)]
fn map_message(
    message: &Map<String, Value>,
    outstanding: &mut BTreeMap<String, String>,
    finished: &mut BTreeSet<String>,
    events: &mut Vec<ExtensionEvent>,
    sequence: &mut u64,
    session_id: &Id,
    attempt_id: &Id,
) -> Result<bool, AdapterError> {
    bounded_string(message, "id", MAX_ID_BYTES)?;
    let role = string(message, "role")?;
    let inference = message
        .get("metadata")
        .and_then(Value::as_object)
        .and_then(|metadata| metadata.get("inference"))
        .filter(|value| !value.is_null());
    let upstream_failure = role == "assistant" && inference.is_none();
    if let Some(inference) = inference {
        let inference = inference.as_object().ok_or(AdapterError::InvalidEvent)?;
        if string(inference, "provider")? != "openai" {
            return Err(AdapterError::InvalidEvent);
        }
        bounded_string(inference, "requestedModel", MAX_MODEL_BYTES)?;
    }
    let contents = message
        .get("content")
        .and_then(Value::as_array)
        .filter(|values| !values.is_empty())
        .ok_or(AdapterError::InvalidEvent)?;
    if contents.len() > MAX_CONTENT_ITEMS_PER_MESSAGE {
        return Err(AdapterError::InvalidEvent);
    }
    for content in contents {
        let content = content.as_object().ok_or(AdapterError::InvalidEvent)?;
        match string(content, "type")? {
            "text" if role == "assistant" => {
                bounded_string(content, "text", MAX_EVENT_LINE_BYTES)?;
            }
            "thinking" if role == "assistant" && !upstream_failure => {
                bounded_string(content, "thinking", MAX_EVENT_LINE_BYTES)?;
                bounded_string(content, "signature", MAX_EVENT_LINE_BYTES)?;
            }
            "toolRequest" if role == "assistant" && !upstream_failure => {
                let id = bounded_string(content, "id", MAX_ID_BYTES)?;
                if outstanding.contains_key(id) || finished.contains(id) {
                    return Err(AdapterError::InvalidEvent);
                }
                let wrapper = object_value(content, "toolCall")?;
                if string(wrapper, "status")? != "success" {
                    return Err(AdapterError::InvalidEvent);
                }
                let call = object_value(wrapper, "value")?;
                let name = bounded_string(call, "name", MAX_TOOL_NAME_BYTES)?;
                if !matches!(name, "write" | "edit" | "shell" | "tree" | "read_image")
                    || !call.get("arguments").is_some_and(Value::is_object)
                {
                    return Err(AdapterError::InvalidEvent);
                }
                outstanding.insert(id.into(), name.into());
                push(
                    events,
                    sequence,
                    session_id,
                    attempt_id,
                    Event::ToolStarted {
                        tool_call_id: Id(id.into()),
                        name: name.into(),
                    },
                )?;
            }
            "toolResponse" if role == "user" => {
                let id = bounded_string(content, "id", MAX_ID_BYTES)?;
                if outstanding.remove(id).is_none() || !finished.insert(id.into()) {
                    return Err(AdapterError::InvalidEvent);
                }
                let wrapper = object_value(content, "toolResult")?;
                let success = match string(wrapper, "status")? {
                    "success" => !object_value(wrapper, "value")?
                        .get("isError")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    "error" => {
                        bounded_string(wrapper, "error", MAX_EVENT_LINE_BYTES)?;
                        false
                    }
                    _ => return Err(AdapterError::InvalidEvent),
                };
                push(
                    events,
                    sequence,
                    session_id,
                    attempt_id,
                    Event::ToolFinished {
                        tool_call_id: Id(id.into()),
                        success,
                    },
                )?;
            }
            "error" if upstream_failure => {
                match string(content, "kind")? {
                    "authentication" | "contextLengthExceeded" | "creditsExhausted" | "other" => {}
                    _ => return Err(AdapterError::InvalidEvent),
                }
                bounded_string(content, "message", MAX_EVENT_LINE_BYTES)?;
            }
            "systemNotification" if upstream_failure => {
                match string(content, "notificationType")? {
                    "thinkingMessage" | "progressMessage" | "inlineMessage"
                    | "creditsExhausted" => {}
                    _ => return Err(AdapterError::InvalidEvent),
                }
                bounded_string(content, "msg", MAX_EVENT_LINE_BYTES)?;
                if let Some(data) = content.get("data")
                    && !data.is_object()
                {
                    return Err(AdapterError::InvalidEvent);
                }
            }
            _ => return Err(AdapterError::InvalidEvent),
        }
    }
    Ok(upstream_failure)
}

fn validate_notification(object: &Map<String, Value>) -> Result<(), AdapterError> {
    bounded_string(object, "extension_id", MAX_TOOL_NAME_BYTES)?;
    // NotificationData is externally tagged and flattened in v1.49.0.
    match (object.get("log"), object.get("progress")) {
        (Some(log), None) if object.len() == 3 => {
            bounded_string(
                log.as_object().ok_or(AdapterError::InvalidEvent)?,
                "message",
                MAX_EVENT_LINE_BYTES,
            )?;
        }
        (None, Some(progress)) if object.len() == 3 => {
            let progress = progress.as_object().ok_or(AdapterError::InvalidEvent)?;
            progress
                .get("progress")
                .and_then(Value::as_f64)
                .filter(|value| value.is_finite() && *value >= 0.0)
                .ok_or(AdapterError::InvalidEvent)?;
            if let Some(total) = progress.get("total").filter(|value| !value.is_null()) {
                total
                    .as_f64()
                    .filter(|value| value.is_finite() && *value >= 0.0)
                    .ok_or(AdapterError::InvalidEvent)?;
            }
            if let Some(message) = progress.get("message").filter(|value| !value.is_null()) {
                let message = message.as_str().ok_or(AdapterError::InvalidEvent)?;
                if message.len() > MAX_EVENT_LINE_BYTES {
                    return Err(AdapterError::InvalidEvent);
                }
            }
        }
        _ => return Err(AdapterError::InvalidEvent),
    }
    Ok(())
}

fn complete_usage(object: &Map<String, Value>) -> Result<Option<Usage>, AdapterError> {
    let input = optional_u64(object, "input_tokens")?;
    let output = optional_u64(object, "output_tokens")?;
    let total = optional_u64(object, "total_tokens")?;
    let cache_read = optional_u64(object, "cache_read_input_tokens")?;
    let cache_write = optional_u64(object, "cache_write_input_tokens")?;
    if let (Some(total), Some(input), Some(output)) = (total, input, output)
        && total
            < input
                .checked_add(output)
                .ok_or(AdapterError::InvalidEvent)?
    {
        return Err(AdapterError::InvalidEvent);
    }
    if cache_read.is_some_and(|value| input.is_some_and(|input| value > input))
        || cache_write.is_some_and(|value| input.is_some_and(|input| value > input))
    {
        return Err(AdapterError::InvalidEvent);
    }
    if let Some(value) = object.get("cost_usd").filter(|value| !value.is_null()) {
        value
            .as_f64()
            .filter(|cost| cost.is_finite() && *cost >= 0.0)
            .ok_or(AdapterError::InvalidEvent)?;
    }
    Ok((input.is_some() || output.is_some()).then_some(Usage {
        input_tokens: input,
        output_tokens: output,
        cost_micros: None,
        currency: None,
    }))
}

fn optional_u64(object: &Map<String, Value>, key: &str) -> Result<Option<u64>, AdapterError> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value.as_u64().map(Some).ok_or(AdapterError::InvalidEvent),
    }
}

fn object_value<'a>(
    object: &'a Map<String, Value>,
    key: &str,
) -> Result<&'a Map<String, Value>, AdapterError> {
    object
        .get(key)
        .and_then(Value::as_object)
        .ok_or(AdapterError::InvalidEvent)
}

fn string<'a>(object: &'a Map<String, Value>, key: &str) -> Result<&'a str, AdapterError> {
    object
        .get(key)
        .and_then(Value::as_str)
        .ok_or(AdapterError::InvalidEvent)
}

fn bounded_string<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    maximum: usize,
) -> Result<&'a str, AdapterError> {
    string(object, key).and_then(|value| {
        (!value.is_empty() && value.len() <= maximum)
            .then_some(value)
            .ok_or(AdapterError::InvalidEvent)
    })
}

fn push(
    events: &mut Vec<ExtensionEvent>,
    sequence: &mut u64,
    session_id: &Id,
    attempt_id: &Id,
    event: Event,
) -> Result<(), AdapterError> {
    if events.len() >= MAX_MAPPED_EVENTS {
        return Err(AdapterError::InvalidEvent);
    }
    let current = *sequence;
    *sequence = sequence.checked_add(1).ok_or(AdapterError::InvalidEvent)?;
    events.push(ExtensionEvent {
        session_id: session_id.clone(),
        attempt_id: attempt_id.clone(),
        sequence: current,
        event,
    });
    Ok(())
}

struct UniqueJson(Value);

impl<'de> Deserialize<'de> for UniqueJson {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Value;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("JSON without duplicate object keys")
            }
            fn visit_bool<E>(self, v: bool) -> Result<Value, E> {
                Ok(Value::Bool(v))
            }
            fn visit_i64<E>(self, v: i64) -> Result<Value, E> {
                Ok(Value::Number(v.into()))
            }
            fn visit_u64<E>(self, v: u64) -> Result<Value, E> {
                Ok(Value::Number(v.into()))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Value, E> {
                serde_json::Number::from_f64(v)
                    .map(Value::Number)
                    .ok_or_else(|| E::custom("non-finite number"))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Value, E> {
                Ok(Value::String(v.into()))
            }
            fn visit_string<E>(self, v: String) -> Result<Value, E> {
                Ok(Value::String(v))
            }
            fn visit_none<E>(self) -> Result<Value, E> {
                Ok(Value::Null)
            }
            fn visit_unit<E>(self) -> Result<Value, E> {
                Ok(Value::Null)
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut values: A) -> Result<Value, A::Error> {
                let mut result = Vec::new();
                while let Some(value) = values.next_element::<UniqueJson>()? {
                    result.push(value.0);
                }
                Ok(Value::Array(result))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut values: A) -> Result<Value, A::Error> {
                let mut result = Map::new();
                while let Some(key) = values.next_key::<String>()? {
                    let value = values.next_value::<UniqueJson>()?.0;
                    if result.insert(key.clone(), value).is_some() {
                        return Err(de::Error::custom(format!("duplicate key {key}")));
                    }
                }
                Ok(Value::Object(result))
            }
        }
        deserializer.deserialize_any(V).map(Self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;
    use std::time::Duration;

    static FIXTURE_NONCE: AtomicU64 = AtomicU64::new(0);

    struct FixtureRoot(PathBuf);

    impl std::ops::Deref for FixtureRoot {
        type Target = Path;

        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    impl AsRef<Path> for FixtureRoot {
        fn as_ref(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for FixtureRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn fixture_base(configured: Option<std::ffi::OsString>) -> PathBuf {
        fs::canonicalize(
            configured
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir),
        )
        .expect("canonical fixture base")
    }

    fn root(label: &str) -> FixtureRoot {
        let target = fixture_base(std::env::var_os("CARGO_TARGET_DIR"));
        FixtureRoot(target.join("asb-unit-fixtures").join(format!(
            "goose-{label}-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            FIXTURE_NONCE.fetch_add(1, Ordering::Relaxed)
        )))
    }

    fn config(binary: &Path, workspace: &Path, state: &Path) -> GooseConfig {
        GooseConfig::new(
            binary,
            workspace,
            state,
            Url::parse("http://127.0.0.1:12345/v1").unwrap(),
            "fixture/model:1",
            3,
            GooseArtifact::LinuxX86_64MuslV1_49_0,
        )
        .unwrap()
    }

    fn ids() -> (Id, Id) {
        (Id("session".into()), Id("attempt".into()))
    }

    #[test]
    fn fixture_base_supports_configured_and_portable_temp_roots() {
        let configured = root("configured-base");
        fs::create_dir_all(&*configured).unwrap();
        assert_eq!(
            fixture_base(Some(configured.as_os_str().to_owned())),
            fs::canonicalize(&*configured).unwrap()
        );
        assert_eq!(
            fixture_base(None),
            fs::canonicalize(std::env::temp_dir()).unwrap()
        );
    }

    fn text_message(text: &str) -> Value {
        json!({
            "type": "message",
            "message": {
                "id": "message-1",
                "role": "assistant",
                "metadata": {"inference": {
                    "provider": "openai", "requestedModel": "fixture/model:1"
                }},
                "content": [{"type": "text", "text": text}]
            }
        })
    }

    fn complete() -> Value {
        json!({
            "type": "complete",
            "total_tokens": 9,
            "input_tokens": 7,
            "output_tokens": 2,
            "cache_read_input_tokens": 0,
            "cache_write_input_tokens": 0
        })
    }

    fn lines(values: &[Value]) -> Vec<u8> {
        values
            .iter()
            .map(|value| serde_json::to_string(value).unwrap())
            .collect::<Vec<_>>()
            .join("\n")
            .into_bytes()
    }

    #[test]
    fn validates_configuration_and_manifest() {
        let endpoint = Url::parse("http://127.0.0.1:1234/v1").unwrap();
        let cfg = GooseConfig::new(
            "/bin/goose",
            "/work",
            "/state",
            endpoint,
            "org/model",
            4,
            GooseArtifact::LinuxX86_64MuslV1_49_0,
        )
        .unwrap();
        let manifest = cfg.manifest();
        assert_eq!(manifest.extension_id, Id("agent.goose".into()));
        assert_eq!(manifest.executable_sha256, TESTED_LINUX_X86_64_MUSL_SHA256);
        assert!(!manifest.capabilities.contains(&Capability::StreamingEvents));
        assert!(manifest.capabilities.contains(&Capability::Usage));
        let aarch_manifest = GooseConfig::new(
            "/bin/goose",
            "/work",
            "/state",
            Url::parse("https://example.com/v1").unwrap(),
            "org/model",
            4,
            GooseArtifact::LinuxAarch64MuslV1_49_0,
        )
        .unwrap()
        .manifest();
        assert_eq!(
            aarch_manifest.executable_sha256,
            TESTED_LINUX_AARCH64_MUSL_SHA256
        );
        assert!(matches!(
            GooseConfig::new(
                "relative",
                "/work",
                "/state",
                Url::parse("https://example.com/v1").unwrap(),
                "model",
                1,
                GooseArtifact::LinuxX86_64MuslV1_49_0,
            ),
            Err(AdapterError::RelativePath("binary"))
        ));
        for endpoint in [
            "http://example.com/v1",
            "https://user@example.com/v1",
            "https://example.com/v1?secret=x",
            "https://com/v1",
            "https://posthog.com/v1",
            "https://i.posthog.com/v1",
            "https://us.i.posthog.com/v1",
        ] {
            assert!(matches!(
                GooseConfig::new(
                    "/bin/goose",
                    "/work",
                    "/state",
                    Url::parse(endpoint).unwrap(),
                    "model",
                    1,
                    GooseArtifact::LinuxX86_64MuslV1_49_0,
                ),
                Err(AdapterError::InvalidEndpoint)
            ));
        }
        assert!(
            GooseConfig::new(
                "/bin/goose",
                "/work",
                "/state",
                Url::parse("https://notposthog.com/v1").unwrap(),
                "model",
                1,
                GooseArtifact::LinuxX86_64MuslV1_49_0,
            )
            .is_ok()
        );
        for model in ["", "/model", "model/", "bad model", "a//b"] {
            assert!(matches!(
                GooseConfig::new(
                    "/bin/goose",
                    "/work",
                    "/state",
                    Url::parse("https://example.com/v1").unwrap(),
                    model,
                    1,
                    GooseArtifact::LinuxX86_64MuslV1_49_0,
                ),
                Err(AdapterError::InvalidModel)
            ));
        }
        for turns in [0, MAX_TURNS + 1] {
            assert!(matches!(
                GooseConfig::new(
                    "/bin/goose",
                    "/work",
                    "/state",
                    Url::parse("https://example.com/v1").unwrap(),
                    "model",
                    turns,
                    GooseArtifact::LinuxX86_64MuslV1_49_0,
                ),
                Err(AdapterError::InvalidMaxTurns)
            ));
        }
    }

    #[test]
    fn public_errors_are_typed_and_stable() {
        let errors = [
            AdapterError::RelativePath("binary"),
            AdapterError::UnsafePath("workspace"),
            AdapterError::InvalidEndpoint,
            AdapterError::InvalidModel,
            AdapterError::InvalidMaxTurns,
            AdapterError::PromptTooLarge,
            AdapterError::InvalidCorrelationId,
            AdapterError::ExecutableMismatch,
            AdapterError::TruncatedOutput,
            AdapterError::RequiredExtensionUnavailable,
            AdapterError::InvalidEvent,
        ];
        assert!(errors.iter().all(|error| !error.to_string().is_empty()));
        let io_error: AdapterError = io::Error::other("bounded fixture").into();
        assert!(io_error.to_string().contains("I/O"));
        let process_error: AdapterError = ProcessError::OutputThreadPanicked.into();
        assert!(process_error.to_string().contains("process"));
    }

    #[test]
    fn committed_provenance_matches_compiled_pins() {
        let value: Value =
            serde_json::from_str(include_str!("../fixtures/goose-v1.49.0-provenance.json"))
                .unwrap();
        assert_eq!(value["source_revision"], UPSTREAM_REVISION);
        let assets = value["assets"].as_array().unwrap();
        assert_eq!(assets.len(), 2);
        assert!(assets.iter().any(|asset| {
            asset["architecture"] == "x86_64"
                && asset["archive_sha256"] == LINUX_X86_64_MUSL_ARCHIVE_SHA256
                && asset["executable_sha256"] == TESTED_LINUX_X86_64_MUSL_SHA256
        }));
        assert!(assets.iter().any(|asset| {
            asset["architecture"] == "aarch64"
                && asset["archive_sha256"] == LINUX_AARCH64_MUSL_ARCHIVE_SHA256
                && asset["executable_sha256"] == TESTED_LINUX_AARCH64_MUSL_SHA256
        }));
    }

    #[test]
    fn maps_tool_lifecycle_usage_without_content() {
        let values = [
            json!({"type":"message","message":{"id":"one","role":"assistant",
                "metadata":{"inference":{"provider":"openai","requestedModel":"fixture/model:1"}},"content":[{
                "type":"toolRequest","id":"call-1","toolCall":{"status":"success","value":{
                    "name":"write","arguments":{"path":"secret","content":"private"}
                }}
            }]}}),
            json!({"type":"message","message":{"id":"two","role":"user","content":[{
                "type":"toolResponse","id":"call-1","toolResult":{"status":"success","value":{
                    "resultType":"complete","content":[{"type":"text","text":"private output"}],
                    "isError":false
                }}
            }]}}),
            text_message("private response"),
            complete(),
        ];
        let (session, attempt) = ids();
        let events = map_events(
            &lines(&values),
            &session,
            &attempt,
            TerminalStatus::Completed,
        )
        .unwrap();
        assert_eq!(events.len(), 7);
        assert!(matches!(events[0].event, Event::Ready));
        assert!(matches!(events[1].event, Event::RequestStarted));
        assert!(matches!(events[2].event, Event::FirstResponse));
        assert!(matches!(
            &events[3].event,
            Event::ToolStarted { tool_call_id, name }
                if tool_call_id == &Id("call-1".into()) && name == "write"
        ));
        assert!(matches!(
            events[4].event,
            Event::ToolFinished { success: true, .. }
        ));
        assert!(matches!(
            events[5].event,
            Event::Usage(Usage {
                input_tokens: Some(7),
                output_tokens: Some(2),
                ..
            })
        ));
        assert!(matches!(events[6].event, Event::Completed));
        let retained = serde_json::to_string(&events).unwrap();
        assert!(!retained.contains("private"));
        assert!(
            events
                .iter()
                .enumerate()
                .all(|(index, event)| event.sequence == index as u64)
        );
    }

    #[test]
    fn rejects_malformed_and_inconsistent_streams() {
        let (session, attempt) = ids();
        let bad = [
            b"{\"type\":\"complete\",\"type\":\"complete\"}".as_slice(),
            b"{\"type\":\"unknown\"}".as_slice(),
            b"{\"type\":\"complete\",\"total_tokens\":-1}".as_slice(),
        ];
        for value in bad {
            assert!(matches!(
                map_events(value, &session, &attempt, TerminalStatus::Completed),
                Err(AdapterError::InvalidEvent)
            ));
        }
        let missing_complete = lines(&[text_message("done")]);
        assert!(matches!(
            map_events(
                &missing_complete,
                &session,
                &attempt,
                TerminalStatus::Completed
            ),
            Err(AdapterError::InvalidEvent)
        ));
        let trailing = lines(&[text_message("done"), complete(), text_message("late")]);
        assert!(matches!(
            map_events(&trailing, &session, &attempt, TerminalStatus::Completed),
            Err(AdapterError::InvalidEvent)
        ));
    }

    #[test]
    fn rejects_content_and_mapped_event_amplification() {
        let (session, attempt) = ids();
        let content =
            vec![json!({"type":"text","text":"bounded"}); MAX_CONTENT_ITEMS_PER_MESSAGE + 1];
        let amplified = lines(&[json!({"type":"message","message":{
            "id":"one","role":"assistant",
            "metadata":{"inference":{"provider":"openai","requestedModel":"fixture/model:1"}},
            "content":content
        }})]);
        assert!(matches!(
            map_events(&amplified, &session, &attempt, TerminalStatus::Completed),
            Err(AdapterError::InvalidEvent)
        ));

        let mut events = (0..MAX_MAPPED_EVENTS)
            .map(|sequence| ExtensionEvent {
                session_id: session.clone(),
                attempt_id: attempt.clone(),
                sequence: sequence as u64,
                event: Event::Ready,
            })
            .collect::<Vec<_>>();
        let mut sequence = MAX_MAPPED_EVENTS as u64;
        assert!(matches!(
            push(
                &mut events,
                &mut sequence,
                &session,
                &attempt,
                Event::Completed
            ),
            Err(AdapterError::InvalidEvent)
        ));
    }

    #[test]
    fn rejects_tool_forgery_and_unknown_content() {
        let (session, attempt) = ids();
        let unmatched = lines(&[
            json!({"type":"message","message":{"id":"one","role":"user","content":[{
                "type":"toolResponse","id":"missing","toolResult":{"status":"error","error":"x"}
            }]}}),
            complete(),
        ]);
        let unknown = lines(&[
            json!({"type":"message","message":{"id":"one","role":"assistant","content":[{
                "type":"image","data":"not retained"
            }]}}),
            complete(),
        ]);
        for value in [unmatched, unknown] {
            assert!(matches!(
                map_events(&value, &session, &attempt, TerminalStatus::Completed),
                Err(AdapterError::InvalidEvent)
            ));
        }
        let duplicate = lines(&[
            json!({"type":"message","message":{"id":"one","role":"assistant","content":[{
                "type":"toolRequest","id":"same","toolCall":{"status":"success","value":{
                    "name":"write","arguments":{}
                }}
            },{
                "type":"toolRequest","id":"same","toolCall":{"status":"success","value":{
                    "name":"write","arguments":{}
                }}
            }]}}),
            complete(),
        ]);
        assert!(matches!(
            map_events(&duplicate, &session, &attempt, TerminalStatus::Completed),
            Err(AdapterError::InvalidEvent)
        ));
    }

    #[test]
    fn validates_bounded_notification_shapes() {
        let (session, attempt) = ids();
        let good = lines(&[
            json!({"type":"notification","extension_id":"developer",
                "log":{"message":"bounded"}}),
            text_message("done"),
            complete(),
        ]);
        assert!(map_events(&good, &session, &attempt, TerminalStatus::Completed).is_ok());
        for bad in [
            json!({"type":"notification","extension_id":"developer"}),
            json!({"type":"notification","extension_id":"developer",
                "progress":{"progress":-1.0,"total":null,"message":null}}),
            json!({"type":"notification","extension_id":"developer",
                "log":{"message":"x"},"unexpected":true}),
        ] {
            assert!(matches!(
                map_events(
                    &lines(&[bad, text_message("done"), complete()]),
                    &session,
                    &attempt,
                    TerminalStatus::Completed
                ),
                Err(AdapterError::InvalidEvent)
            ));
        }
    }

    #[test]
    fn failed_and_cancelled_status_are_explicit() {
        let (session, attempt) = ids();
        let failed = map_events(
            b"{\"type\":\"error\",\"error\":\"private\"}",
            &session,
            &attempt,
            TerminalStatus::Failed,
        )
        .unwrap();
        assert!(matches!(failed.last().unwrap().event, Event::Failed(_)));
        assert!(!serde_json::to_string(&failed).unwrap().contains("private"));
        let cancelled = map_events(b"", &session, &attempt, TerminalStatus::Cancelled).unwrap();
        assert_eq!(cancelled.len(), 2);
    }

    #[test]
    fn exit_zero_provider_diagnostic_is_failed_without_retaining_text() {
        let (session, attempt) = ids();
        let private = "Ran into this error: private provider response";
        let output = lines(&[
            json!({"type":"message","message":{
                "id":"failure","role":"assistant",
                "metadata":{"userVisible":true,"agentVisible":true},
                "content":[{"type":"text","text":private}]
            }}),
            json!({"type":"complete","total_tokens":0,"input_tokens":0,"output_tokens":0}),
        ]);
        let events = map_events(&output, &session, &attempt, TerminalStatus::Completed).unwrap();
        assert!(matches!(events.last().unwrap().event, Event::Failed(_)));
        assert!(!serde_json::to_string(&events).unwrap().contains(private));
    }

    fn executable(path: &Path, body: &str) -> String {
        fs::write(path, body).unwrap();
        let mut permissions = fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(path, permissions).unwrap();
        digest_file(path).unwrap()
    }

    fn limits() -> ProcessLimits {
        ProcessLimits::new(
            1024 * 1024,
            1024 * 1024,
            Duration::from_secs(3),
            Duration::from_millis(100),
            Duration::from_millis(2),
        )
        .unwrap()
    }

    #[test]
    fn process_boundary_unlinks_prompt_and_cleans_state() {
        let root = root("process");
        let workspace = root.join("work");
        let state = root.join("state");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&state).unwrap();
        let binary = root.join("goose");
        let digest = executable(
            &binary,
            "#!/bin/sh\nread prompt\ntest \"$prompt\" = \"private prompt\" || exit 9\nprintf '%s\\n' '{\"type\":\"message\",\"message\":{\"id\":\"m\",\"role\":\"assistant\",\"metadata\":{\"inference\":{\"provider\":\"openai\",\"requestedModel\":\"fixture/model:1\"}},\"content\":[{\"type\":\"text\",\"text\":\"private response\"}]}}' '{\"type\":\"complete\",\"total_tokens\":2,\"input_tokens\":1,\"output_tokens\":1}'\n",
        );
        let mut cfg = config(&binary, &workspace, &state);
        cfg.verification_digest_override = Some(digest);
        let mut running = cfg
            .start(Id("s".into()), Id("a".into()), "private prompt", limits())
            .unwrap();
        assert!(running.pid() > 0);
        let outcome = running.wait().unwrap();
        assert_eq!(outcome.status(), TerminalStatus::Completed);
        assert_eq!(outcome.exit_code(), Some(0));
        assert_eq!(fs::read_dir(&state).unwrap().count(), 0);
        assert!(
            !serde_json::to_string(outcome.events())
                .unwrap()
                .contains("private")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn verified_private_launch_ignores_configured_path_replacement() {
        let root = root("replacement");
        let workspace = root.join("work");
        let state = root.join("state");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&state).unwrap();
        let binary = root.join("goose");
        let digest = executable(
            &binary,
            "#!/bin/sh\nread prompt\ncat <<EOF\n{\"type\":\"message\",\"message\":{\"id\":\"m\",\"role\":\"assistant\",\"metadata\":{\"inference\":{\"provider\":\"openai\",\"requestedModel\":\"fixture/model:1\"}},\"content\":[{\"type\":\"text\",\"text\":\"done\"}]}}\n{\"type\":\"complete\",\"total_tokens\":2,\"input_tokens\":1,\"output_tokens\":1}\nEOF\n",
        );
        let marker = workspace.join("replacement-executed");
        let mut cfg = config(&binary, &workspace, &state);
        cfg.verification_digest_override = Some(digest);
        cfg.replacement_before_spawn =
            Some(format!("#!/bin/sh\ntouch \"{}\"\nexit 77\n", marker.display()).into_bytes());
        let outcome = cfg
            .start(Id("s".into()), Id("a".into()), "prompt", limits())
            .unwrap()
            .wait()
            .unwrap();
        assert_eq!(outcome.status(), TerminalStatus::Completed);
        assert!(!marker.exists());
        assert!(fs::read_to_string(&binary).unwrap().contains("exit 77"));
        assert_eq!(fs::read_dir(&state).unwrap().count(), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn diagnostic_and_symlink_fail_closed() {
        let root = root("negative");
        let workspace = root.join("work");
        let state = root.join("state");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&state).unwrap();
        let binary = root.join("goose");
        let digest = executable(
            &binary,
            "#!/bin/sh\nprintf '%s\\n' '{\"type\":\"message\",\"message\":{\"id\":\"m\",\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"done\"}]}}' '{\"type\":\"complete\"}'\necho extension-warning >&2\n",
        );
        let mut cfg = config(&binary, &workspace, &state);
        cfg.verification_digest_override = Some(digest.clone());
        let error = cfg
            .start(Id("s".into()), Id("a".into()), "prompt", limits())
            .unwrap()
            .wait()
            .unwrap_err();
        assert!(matches!(error, AdapterError::RequiredExtensionUnavailable));
        assert!(matches!(
            config(&binary, &workspace, &state).verify_executable(),
            Err(AdapterError::ExecutableMismatch)
        ));
        let empty = root.join("empty");
        fs::write(&empty, []).unwrap();
        assert!(matches!(
            digest_file(&empty),
            Err(AdapterError::ExecutableMismatch)
        ));
        let link = root.join("linked-goose");
        symlink(&binary, &link).unwrap();
        let mut linked = config(&link, &workspace, &state);
        linked.verification_digest_override = Some(digest_file(&binary).unwrap());
        assert!(matches!(
            linked.verify_executable(),
            Err(AdapterError::UnsafePath("Goose executable"))
        ));
        let bad_workspace = root.join("not-a-workspace");
        fs::write(&bad_workspace, []).unwrap();
        let mut invalid_workspace = config(&binary, &bad_workspace, &state);
        invalid_workspace.verification_digest_override = Some(digest.clone());
        assert!(matches!(
            invalid_workspace.start(Id("s".into()), Id("a".into()), "prompt", limits()),
            Err(AdapterError::UnsafePath("workspace"))
        ));
        let bad_state = root.join("not-a-state-root");
        fs::write(&bad_state, []).unwrap();
        let mut invalid_state = config(&binary, &workspace, &bad_state);
        invalid_state.verification_digest_override = Some(digest);
        assert!(matches!(
            invalid_state.start(Id("s".into()), Id("a".into()), "prompt", limits()),
            Err(AdapterError::UnsafePath("state root"))
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cancellation_is_terminal_and_cleans_state() {
        let root = root("cancel");
        let workspace = root.join("work");
        let state = root.join("state");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&state).unwrap();
        let binary = root.join("goose");
        let digest = executable(&binary, "#!/bin/sh\nread prompt\nsleep 30\n");
        let mut cfg = config(&binary, &workspace, &state);
        cfg.verification_digest_override = Some(digest);
        let mut running = cfg
            .start(Id("s".into()), Id("a".into()), "prompt", limits())
            .unwrap();
        thread::sleep(Duration::from_millis(20));
        running.cancel().unwrap();
        running.cancel().unwrap();
        let outcome = running.wait().unwrap();
        assert_eq!(outcome.status(), TerminalStatus::Cancelled);
        assert_eq!(fs::read_dir(&state).unwrap().count(), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn oversized_prompt_rejected_before_execution() {
        let root = root("prompt");
        let workspace = root.join("work");
        let state = root.join("state");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&state).unwrap();
        let binary = root.join("goose");
        let digest = executable(&binary, "#!/bin/sh\nexit 99\n");
        let mut cfg = config(&binary, &workspace, &state);
        cfg.verification_digest_override = Some(digest);
        assert!(matches!(
            cfg.start(
                Id("s".into()),
                Id("a".into()),
                &"x".repeat(MAX_PROMPT_BYTES + 1),
                limits()
            ),
            Err(AdapterError::PromptTooLarge)
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn invalid_correlation_ids_fail_before_state_mutation() {
        let root = root("correlation");
        let workspace = root.join("work");
        let state = root.join("state");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&state).unwrap();
        let binary = root.join("goose");
        let digest = executable(&binary, "#!/bin/sh\nexit 99\n");
        let mut cfg = config(&binary, &workspace, &state);
        cfg.verification_digest_override = Some(digest);
        for (session, attempt) in [
            (Id(String::new()), Id("attempt".into())),
            (Id("session".into()), Id(String::new())),
            (
                Id("s".repeat(MAX_CORRELATION_ID_BYTES + 1)),
                Id("attempt".into()),
            ),
            (
                Id("session".into()),
                Id("a".repeat(MAX_CORRELATION_ID_BYTES + 1)),
            ),
        ] {
            assert!(matches!(
                cfg.start(session, attempt, "prompt", limits()),
                Err(AdapterError::InvalidCorrelationId)
            ));
            assert_eq!(fs::read_dir(&state).unwrap().count(), 0);
        }
        fs::remove_dir_all(root).unwrap();
    }
}
