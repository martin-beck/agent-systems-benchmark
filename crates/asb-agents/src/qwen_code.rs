// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Bounded adapter for Qwen Code's headless stream-JSON interface.

use asb_protocol::{
    Capability, Event, ExtensionEvent, ExtensionKind, ExtensionManifest, Id, PROTOCOL_V1, RpcError,
    TerminalStatus, Usage,
};
use asb_runtime::{ProcessError, ProcessLimits, RunningProcess, Termination};
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::Value;
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

/// Qwen Code release implemented by this adapter.
pub const SUPPORTED_VERSION: &str = "0.23.0";
/// Immutable upstream release commit inspected for this adapter.
pub const UPSTREAM_REVISION: &str = "98a9c964158697dd5631d15a62174684ff7bbb53";
/// SHA-256 of the official Linux x86_64 standalone release archive.
pub const LINUX_X86_64_ARCHIVE_SHA256: &str =
    "da20f7227e1daa0834c6dd0b46f23a72a2eeb083b9b21e73cb53e182674b05bd";
/// SHA-256 of the launcher script in that archive.
pub const LINUX_X86_64_LAUNCHER_SHA256: &str =
    "1461b5ef9149b86e58ea22ddb9cc0616679437f9ff0011cf7c3c102f94568abf";
/// SHA-256 of the bundled Node 22.23.2 executable exercised natively.
pub const LINUX_X86_64_NODE_SHA256: &str =
    "3517c2df0b2f8cd7f422b4b8450ef81c6889f08eb03e281d6de9079b15e6a327";
/// SHA-256 of the launcher-imported CLI entry module.
pub const LINUX_X86_64_CLI_ENTRY_SHA256: &str =
    "68cb29eb7ccc936d78ece5564ef55cae41a55b630e6657dc417c1f2e561cf4c9";
/// SHA-256 of the primary bundled CLI module imported by the entry module.
pub const LINUX_X86_64_CLI_SHA256: &str =
    "c85176761861172aa41003fed2b80b99ee73d797a1c60fca9e160086e5390524";
/// Largest prompt accepted by the adapter.
pub const MAX_PROMPT_BYTES: usize = 4 * 1024 * 1024;
/// Largest individual upstream stream-JSON line.
pub const MAX_EVENT_LINE_BYTES: usize = 16 * 1024 * 1024;
/// Largest number of upstream events accepted per attempt.
pub const MAX_UPSTREAM_EVENTS: usize = 65_536;
const MAX_ARTIFACT_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_ENDPOINT_BYTES: usize = 4 * 1024;
const MAX_MODEL_BYTES: usize = 256;
const MAX_ID_BYTES: usize = 1024;
const MAX_TOOL_NAME_BYTES: usize = 1024;
const MAX_SESSION_TURNS: u64 = 32;
const MAX_TOOL_CALLS: usize = 64;
const MAX_WORKSPACE_FILES: usize = 4_096;
const MAX_WORKSPACE_ENTRIES: usize = 16_384;
const MAX_WORKSPACE_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_WORKSPACE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_PATH_BYTES: usize = 4 * 1024;
const CLOSED_PROXY: &str = "http://127.0.0.1:9";

/// Content-pinned Qwen Code artifact understood by this adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QwenCodeArtifact {
    /// Official v0.23.0 standalone archive on native glibc Linux x86_64.
    LinuxX86_64V0_23_0,
}

impl QwenCodeArtifact {
    const fn digest(self) -> &'static str {
        match self {
            Self::LinuxX86_64V0_23_0 => LINUX_X86_64_ARCHIVE_SHA256,
        }
    }
}

/// Validated immutable inputs for one Qwen Code installation.
#[derive(Clone, Debug)]
pub struct QwenCodeConfig {
    executable: PathBuf,
    archive: PathBuf,
    workspace: PathBuf,
    state_root: PathBuf,
    endpoint: Url,
    model: String,
    artifact: QwenCodeArtifact,
    #[cfg(test)]
    digest_overrides: Option<[String; 5]>,
}

/// Configuration or production-boundary failure.
#[derive(Debug)]
pub enum AdapterError {
    /// A required path was relative.
    RelativePath(&'static str),
    /// Endpoint syntax or placement was unsafe.
    InvalidEndpoint,
    /// Model identifier was malformed.
    InvalidModel,
    /// Prompt contained a NUL byte.
    InvalidPrompt,
    /// Prompt exceeded the byte ceiling.
    PromptTooLarge,
    /// Filesystem or cleanup operation failed.
    Io(io::Error),
    /// Pinned archive or exercised runtime component did not match.
    ArtifactMismatch,
    /// Workspace or state topology was unsafe.
    UnsafeFilesystem,
    /// Native process execution failed.
    Process(ProcessError),
    /// Structured stdout exceeded the process retention limit.
    TruncatedOutput,
    /// Stream-JSON was malformed, inconsistent, or unsupported.
    InvalidEvent,
}

impl fmt::Display for AdapterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RelativePath(label) => write!(f, "{label} must be absolute"),
            Self::InvalidEndpoint => f.write_str("invalid Qwen Code provider endpoint"),
            Self::InvalidModel => f.write_str("invalid Qwen Code model identifier"),
            Self::InvalidPrompt => f.write_str("invalid Qwen Code prompt"),
            Self::PromptTooLarge => f.write_str("Qwen Code prompt exceeds byte limit"),
            Self::Io(error) => write!(f, "Qwen Code adapter I/O failed: {error}"),
            Self::ArtifactMismatch => f.write_str("Qwen Code artifact pin mismatch"),
            Self::UnsafeFilesystem => f.write_str("unsafe Qwen Code workspace or state root"),
            Self::Process(error) => write!(f, "Qwen Code process failed: {error}"),
            Self::TruncatedOutput => f.write_str("Qwen Code structured output was truncated"),
            Self::InvalidEvent => f.write_str("invalid Qwen Code structured event"),
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

impl QwenCodeConfig {
    /// Construct a credential-free, explicit provider boundary.
    pub fn new(
        executable: impl Into<PathBuf>,
        archive: impl Into<PathBuf>,
        workspace: impl Into<PathBuf>,
        state_root: impl Into<PathBuf>,
        endpoint: Url,
        model: impl Into<String>,
        artifact: QwenCodeArtifact,
    ) -> Result<Self, AdapterError> {
        let executable = executable.into();
        let archive = archive.into();
        let workspace = workspace.into();
        let state_root = state_root.into();
        for (label, path) in [
            ("Qwen Code executable", executable.as_path()),
            ("Qwen Code archive", archive.as_path()),
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
            || (endpoint.scheme() == "http" && !endpoint.host_str().is_some_and(is_loopback))
        {
            return Err(AdapterError::InvalidEndpoint);
        }
        let model = model.into();
        if model.is_empty() || model.len() > MAX_MODEL_BYTES || !model.bytes().all(model_byte) {
            return Err(AdapterError::InvalidModel);
        }
        Ok(Self {
            executable,
            archive,
            workspace,
            state_root,
            endpoint,
            model,
            artifact,
            #[cfg(test)]
            digest_overrides: None,
        })
    }

    /// Explicit capabilities of this exact adapter revision.
    #[must_use]
    pub fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            extension_id: Id("agent.qwen-code".into()),
            kind: ExtensionKind::Agent,
            implementation_version: format!("qwen-code-{SUPPORTED_VERSION}+asb-0.1.0"),
            protocol: PROTOCOL_V1,
            capabilities: BTreeSet::from([
                Capability::Cancellation,
                Capability::StreamingEvents,
                Capability::Usage,
            ]),
            executable_sha256: self.artifact.digest().into(),
        }
    }

    /// Verify the release archive and the directly executed runtime components.
    pub fn verify_executable(&self) -> Result<(), AdapterError> {
        let root = self
            .executable
            .parent()
            .and_then(Path::parent)
            .ok_or(AdapterError::ArtifactMismatch)?;
        let owned = [
            self.archive.clone(),
            self.executable.clone(),
            root.join("node/bin/node"),
            root.join("lib/cli-entry.js"),
            root.join("lib/cli.js"),
        ];
        let expected = self.expected_digests();
        for (path, digest) in owned.iter().zip(expected.iter()) {
            if fs::symlink_metadata(path)?.file_type().is_symlink()
                || !fs::metadata(path)?.is_file()
                || digest_file(path)? != *digest
            {
                return Err(AdapterError::ArtifactMismatch);
            }
        }
        Ok(())
    }

    fn expected_digests(&self) -> [String; 5] {
        #[cfg(test)]
        if let Some(values) = &self.digest_overrides {
            return values.clone();
        }
        [
            self.artifact.digest().into(),
            LINUX_X86_64_LAUNCHER_SHA256.into(),
            LINUX_X86_64_NODE_SHA256.into(),
            LINUX_X86_64_CLI_ENTRY_SHA256.into(),
            LINUX_X86_64_CLI_SHA256.into(),
        ]
    }

    /// Start one isolated headless stream-JSON attempt.
    pub fn start(
        &self,
        session_id: Id,
        attempt_id: Id,
        prompt: &str,
        limits: ProcessLimits,
    ) -> Result<RunningQwenCode, AdapterError> {
        if prompt.as_bytes().contains(&0) {
            return Err(AdapterError::InvalidPrompt);
        }
        if prompt.len() > MAX_PROMPT_BYTES {
            return Err(AdapterError::PromptTooLarge);
        }
        self.verify_executable()?;
        validate_workspace(&self.workspace)?;
        prepare_state_root(&self.state_root)?;
        let run_root = self.unique_run_root()?;
        fs::DirBuilder::new().mode(0o700).create(&run_root)?;
        let prepared = (|| {
            for name in ["home", "config", "data", "cache", "runtime", "prompt"] {
                fs::DirBuilder::new()
                    .mode(0o700)
                    .create(run_root.join(name))?;
            }
            let path = run_root.join("prompt/input");
            let mut file = OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)?;
            fs::remove_file(path)?;
            file.write_all(prompt.as_bytes())?;
            file.sync_all()?;
            file.seek(SeekFrom::Start(0))?;
            Ok::<fs::File, AdapterError>(file)
        })();
        let prompt = match prepared {
            Ok(file) => file,
            Err(error) => {
                let _ = fs::remove_dir_all(&run_root);
                return Err(error);
            }
        };
        let result = self.spawn(&run_root, prompt, limits);
        match result {
            Ok(process) => Ok(RunningQwenCode {
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
        prompt: fs::File,
        limits: ProcessLimits,
    ) -> Result<RunningProcess, AdapterError> {
        let mut command = Command::new(&self.executable);
        command
            .current_dir(&self.workspace)
            .args([
                "--safe-mode",
                "--chat-recording=false",
                "--approval-mode",
                "auto-edit",
                "--auth-type",
                "openai",
                "--model",
            ])
            .arg(&self.model)
            .args(["--openai-base-url"])
            .arg(self.endpoint.as_str())
            .args([
                "--output-format",
                "stream-json",
                "--max-session-turns",
                "32",
                "--max-wall-time",
                "300",
                "--max-tool-calls",
                "64",
                "--exclude-tools",
                "run_shell_command",
                "--exclude-tools",
                "agent",
                "--exclude-tools",
                "web_fetch",
                "--exclude-tools",
                "web_search",
                "--exclude-tools",
                "computer_use",
            ])
            .stdin(Stdio::from(prompt))
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", run_root.join("home"))
            .env("XDG_CONFIG_HOME", run_root.join("config"))
            .env("XDG_DATA_HOME", run_root.join("data"))
            .env("XDG_CACHE_HOME", run_root.join("cache"))
            .env("QWEN_RUNTIME_DIR", run_root.join("runtime"))
            .env("QWEN_CODE_SAFE_MODE", "true")
            .env("QWEN_TELEMETRY_ENABLED", "0")
            .env("QWEN_CODE_SKIP_UPDATE_CHECK_ONCE", "true")
            .env("OPENAI_API_KEY", "asb-credential-free")
            .env("HTTP_PROXY", CLOSED_PROXY)
            .env("HTTPS_PROXY", CLOSED_PROXY)
            .env("ALL_PROXY", CLOSED_PROXY)
            .env("http_proxy", CLOSED_PROXY)
            .env("https_proxy", CLOSED_PROXY)
            .env("all_proxy", CLOSED_PROXY)
            .env(
                "NO_PROXY",
                self.endpoint
                    .host_str()
                    .ok_or(AdapterError::InvalidEndpoint)?,
            )
            .env(
                "no_proxy",
                self.endpoint
                    .host_str()
                    .ok_or(AdapterError::InvalidEndpoint)?,
            );
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

fn is_loopback(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "::1" | "[::1]" | "localhost")
}

const fn model_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/' | b':')
}

fn require_exact_directory(path: &Path) -> Result<(), AdapterError> {
    if fs::symlink_metadata(path)?.file_type().is_symlink()
        || !fs::metadata(path)?.is_dir()
        || fs::canonicalize(path)? != path
    {
        return Err(AdapterError::UnsafeFilesystem);
    }
    Ok(())
}

fn validate_workspace(root: &Path) -> Result<(), AdapterError> {
    require_exact_directory(root)?;
    let mut pending = vec![root.to_path_buf()];
    let mut entries_seen = 0_usize;
    let mut files_seen = 0_usize;
    let mut bytes_seen = 0_u64;
    while let Some(directory) = pending.pop() {
        let mut entries = fs::read_dir(directory)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(AdapterError::Io)?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            entries_seen = entries_seen
                .checked_add(1)
                .ok_or(AdapterError::UnsafeFilesystem)?;
            let path = entry.path();
            if entries_seen > MAX_WORKSPACE_ENTRIES
                || path.as_os_str().as_encoded_bytes().len() > MAX_PATH_BYTES
            {
                return Err(AdapterError::UnsafeFilesystem);
            }
            let kind = entry.file_type()?;
            if kind.is_symlink() || !(kind.is_dir() || kind.is_file()) {
                return Err(AdapterError::UnsafeFilesystem);
            }
            if kind.is_dir() {
                pending.push(path);
            } else {
                let size = entry.metadata()?.len();
                files_seen = files_seen
                    .checked_add(1)
                    .ok_or(AdapterError::UnsafeFilesystem)?;
                bytes_seen = bytes_seen
                    .checked_add(size)
                    .ok_or(AdapterError::UnsafeFilesystem)?;
                if files_seen > MAX_WORKSPACE_FILES
                    || size > MAX_WORKSPACE_FILE_BYTES
                    || bytes_seen > MAX_WORKSPACE_BYTES
                {
                    return Err(AdapterError::UnsafeFilesystem);
                }
            }
        }
    }
    Ok(())
}

fn prepare_state_root(path: &Path) -> Result<(), AdapterError> {
    if path.exists() {
        return require_exact_directory(path);
    }
    let parent = path.parent().ok_or(AdapterError::UnsafeFilesystem)?;
    require_exact_directory(parent)?;
    fs::DirBuilder::new().mode(0o700).create(path)?;
    require_exact_directory(path)
}

fn digest_file(path: &Path) -> Result<String, AdapterError> {
    let mut file = fs::File::open(path)?;
    let size = file.metadata()?.len();
    if size == 0 || size > MAX_ARTIFACT_BYTES {
        return Err(AdapterError::ArtifactMismatch);
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

/// Cancellable Qwen Code process awaiting collection.
pub struct RunningQwenCode {
    process: RunningProcess,
    session_id: Id,
    attempt_id: Id,
    run_root: Option<PathBuf>,
}

impl RunningQwenCode {
    /// Native process identifier for ownership and metrics.
    #[must_use]
    pub fn pid(&self) -> u32 {
        self.process.pid()
    }

    /// Request idempotent process-group cancellation.
    pub fn cancel(&mut self) -> Result<(), AdapterError> {
        self.process.cancel().map_err(AdapterError::Process)
    }

    /// Reap, clean private state, and map content-free evidence.
    pub fn wait(&mut self) -> Result<QwenCodeOutcome, AdapterError> {
        let output = self.process.wait().cloned()?;
        self.cleanup()?;
        if output.stdout.truncated {
            return Err(AdapterError::TruncatedOutput);
        }
        let status = match output.termination {
            Termination::Cancelled => TerminalStatus::Cancelled,
            Termination::TimedOut => TerminalStatus::Failed,
            Termination::Exited if output.exit_code == Some(0) => TerminalStatus::Completed,
            Termination::Exited => TerminalStatus::Failed,
        };
        let events = map_events(
            &output.stdout.bytes,
            &self.session_id,
            &self.attempt_id,
            status,
        )?;
        Ok(QwenCodeOutcome {
            events,
            status,
            exit_code: output.exit_code,
            stderr_truncated: output.stderr.truncated,
        })
    }

    fn cleanup(&mut self) -> Result<(), AdapterError> {
        let Some(root) = self.run_root.as_ref() else {
            return Ok(());
        };
        match fs::remove_dir_all(root) {
            Ok(()) => self.run_root = None,
            Err(error) if error.kind() == io::ErrorKind::NotFound => self.run_root = None,
            Err(error) => return Err(error.into()),
        }
        Ok(())
    }
}

impl Drop for RunningQwenCode {
    fn drop(&mut self) {
        let _ = self.process.cancel();
        if self.process.wait().is_ok() {
            let _ = self.cleanup();
        }
    }
}

/// Privacy-filtered terminal evidence from one Qwen Code attempt.
#[derive(Clone, Debug, PartialEq)]
pub struct QwenCodeOutcome {
    events: Vec<ExtensionEvent>,
    status: TerminalStatus,
    exit_code: Option<i32>,
    stderr_truncated: bool,
}

impl QwenCodeOutcome {
    /// Ordered lifecycle/tool/usage events without prompts or responses.
    #[must_use]
    pub fn events(&self) -> &[ExtensionEvent] {
        &self.events
    }
    /// Terminal status derived at the native process boundary.
    #[must_use]
    pub const fn status(&self) -> TerminalStatus {
        self.status
    }
    /// Native exit code when one exists.
    #[must_use]
    pub const fn exit_code(&self) -> Option<i32> {
        self.exit_code
    }
    /// Whether retained diagnostic stderr was truncated.
    #[must_use]
    pub const fn stderr_truncated(&self) -> bool {
        self.stderr_truncated
    }
}

fn map_events(
    stdout: &[u8],
    session_id: &Id,
    attempt_id: &Id,
    status: TerminalStatus,
) -> Result<Vec<ExtensionEvent>, AdapterError> {
    // TERM may interrupt the final JSON write. Complete attempts remain strict;
    // cancellation retains only complete newline-delimited evidence.
    let structured = if status == TerminalStatus::Cancelled && !stdout.ends_with(b"\n") {
        stdout
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(&[][..], |index| &stdout[..=index])
    } else {
        stdout
    };
    let text = std::str::from_utf8(structured).map_err(|_| AdapterError::InvalidEvent)?;
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
    let mut upstream_session: Option<String> = None;
    let mut saw_start = false;
    let mut saw_response = false;
    let mut saw_result = false;
    let mut pending = BTreeMap::<String, String>::new();
    let mut completed = BTreeSet::<String>::new();
    let mut count = 0_usize;
    for line in text.lines().filter(|line| !line.is_empty()) {
        count = count.checked_add(1).ok_or(AdapterError::InvalidEvent)?;
        if count > MAX_UPSTREAM_EVENTS || line.len() > MAX_EVENT_LINE_BYTES {
            return Err(AdapterError::InvalidEvent);
        }
        let value = serde_json::from_str::<UniqueJson>(line)
            .map_err(|_| AdapterError::InvalidEvent)?
            .0;
        let object = value.as_object().ok_or(AdapterError::InvalidEvent)?;
        let kind = string(object, "type")?;
        let current = string(object, "session_id")?;
        bounded(current, MAX_ID_BYTES)?;
        match upstream_session.as_deref() {
            None => upstream_session = Some(current.into()),
            Some(expected) if expected != current => return Err(AdapterError::InvalidEvent),
            Some(_) => {}
        }
        match kind {
            "system" if !saw_start && !saw_result => {
                if string(object, "subtype")? != "init" {
                    return Err(AdapterError::InvalidEvent);
                }
                validate_init(object)?;
                saw_start = true;
            }
            "assistant" if saw_start && !saw_result => {
                if !saw_response {
                    push(
                        &mut events,
                        &mut sequence,
                        session_id,
                        attempt_id,
                        Event::FirstResponse,
                    )?;
                    saw_response = true;
                }
                let message = object
                    .get("message")
                    .and_then(Value::as_object)
                    .ok_or(AdapterError::InvalidEvent)?;
                for block in message
                    .get("content")
                    .and_then(Value::as_array)
                    .ok_or(AdapterError::InvalidEvent)?
                {
                    let block = block.as_object().ok_or(AdapterError::InvalidEvent)?;
                    match string(block, "type")? {
                        "text" | "thinking" => {}
                        "tool_use" => {
                            let id = string(block, "id")?;
                            let name = string(block, "name")?;
                            bounded(id, MAX_ID_BYTES)?;
                            bounded(name, MAX_TOOL_NAME_BYTES)?;
                            if pending.insert(id.into(), name.into()).is_some()
                                || completed.contains(id)
                                || pending.len() + completed.len() > MAX_TOOL_CALLS
                            {
                                return Err(AdapterError::InvalidEvent);
                            }
                            push(
                                &mut events,
                                &mut sequence,
                                session_id,
                                attempt_id,
                                Event::ToolStarted {
                                    tool_call_id: Id(id.into()),
                                    name: name.into(),
                                },
                            )?;
                        }
                        _ => return Err(AdapterError::InvalidEvent),
                    }
                }
            }
            "user" if saw_start && !saw_result => {
                let content = object
                    .get("message")
                    .and_then(|v| v.get("content"))
                    .and_then(Value::as_array)
                    .ok_or(AdapterError::InvalidEvent)?;
                for block in content {
                    let block = block.as_object().ok_or(AdapterError::InvalidEvent)?;
                    if string(block, "type")? != "tool_result" {
                        return Err(AdapterError::InvalidEvent);
                    }
                    let id = string(block, "tool_use_id")?;
                    if pending.remove(id).is_none() || !completed.insert(id.into()) {
                        return Err(AdapterError::InvalidEvent);
                    }
                    push(
                        &mut events,
                        &mut sequence,
                        session_id,
                        attempt_id,
                        Event::ToolFinished {
                            tool_call_id: Id(id.into()),
                            success: !block
                                .get("is_error")
                                .and_then(Value::as_bool)
                                .unwrap_or(false),
                        },
                    )?;
                }
            }
            "result" if saw_start && !saw_result => {
                saw_result = true;
                let is_error = object
                    .get("is_error")
                    .and_then(Value::as_bool)
                    .ok_or(AdapterError::InvalidEvent)?;
                let usage = parse_usage(object.get("usage"))?;
                let has_measured_usage = usage.as_ref().is_some_and(|usage| {
                    usage.input_tokens.is_some_and(|tokens| tokens > 0)
                        && usage.output_tokens.is_some_and(|tokens| tokens > 0)
                });
                if let Some(usage) = usage {
                    push(
                        &mut events,
                        &mut sequence,
                        session_id,
                        attempt_id,
                        Event::Usage(usage),
                    )?;
                }
                if object
                    .get("num_turns")
                    .and_then(Value::as_u64)
                    .is_none_or(|turns| turns > MAX_SESSION_TURNS)
                {
                    return Err(AdapterError::InvalidEvent);
                }
                match (status, string(object, "subtype")?, is_error) {
                    (TerminalStatus::Completed, "success", false) if has_measured_usage => {}
                    (TerminalStatus::Failed, _, true) => {}
                    _ => return Err(AdapterError::InvalidEvent),
                }
            }
            _ => return Err(AdapterError::InvalidEvent),
        }
    }
    match status {
        TerminalStatus::Completed
            if !saw_start || !saw_response || !saw_result || !pending.is_empty() =>
        {
            Err(AdapterError::InvalidEvent)
        }
        TerminalStatus::Completed => {
            push(
                &mut events,
                &mut sequence,
                session_id,
                attempt_id,
                Event::Completed,
            )?;
            Ok(events)
        }
        TerminalStatus::Failed => {
            push(
                &mut events,
                &mut sequence,
                session_id,
                attempt_id,
                Event::Failed(RpcError {
                    code: -32_100,
                    message: "Qwen Code attempt failed; raw diagnostics are not retained".into(),
                    data: None,
                }),
            )?;
            Ok(events)
        }
        TerminalStatus::Cancelled => Ok(events),
    }
}

fn validate_init(object: &serde_json::Map<String, Value>) -> Result<(), AdapterError> {
    if string(object, "qwen_code_version")? != SUPPORTED_VERSION
        || string(object, "permission_mode")? != "auto-edit"
    {
        return Err(AdapterError::InvalidEvent);
    }
    let tools = object
        .get("tools")
        .and_then(Value::as_array)
        .ok_or(AdapterError::InvalidEvent)?;
    let mut names = BTreeSet::new();
    for tool in tools {
        let name = tool.as_str().ok_or(AdapterError::InvalidEvent)?;
        bounded(name, MAX_TOOL_NAME_BYTES)?;
        if !names.insert(name) {
            return Err(AdapterError::InvalidEvent);
        }
    }
    for excluded in [
        "run_shell_command",
        "agent",
        "web_fetch",
        "web_search",
        "computer_use",
    ] {
        if names.contains(excluded) {
            return Err(AdapterError::InvalidEvent);
        }
    }
    if !object
        .get("mcp_servers")
        .and_then(Value::as_array)
        .is_some_and(Vec::is_empty)
    {
        return Err(AdapterError::InvalidEvent);
    }
    Ok(())
}

fn parse_usage(value: Option<&Value>) -> Result<Option<Usage>, AdapterError> {
    let Some(object) = value.and_then(Value::as_object) else {
        return Ok(None);
    };
    let input_tokens = optional_u64(object.get("input_tokens"))?;
    let output_tokens = optional_u64(object.get("output_tokens"))?;
    if input_tokens.is_none() && output_tokens.is_none() {
        return Ok(None);
    }
    Ok(Some(Usage {
        input_tokens,
        output_tokens,
        cost_micros: None,
        currency: None,
    }))
}

fn optional_u64(value: Option<&Value>) -> Result<Option<u64>, AdapterError> {
    value
        .map(|value| value.as_u64().ok_or(AdapterError::InvalidEvent))
        .transpose()
}

fn string<'a>(
    object: &'a serde_json::Map<String, Value>,
    key: &str,
) -> Result<&'a str, AdapterError> {
    object
        .get(key)
        .and_then(Value::as_str)
        .ok_or(AdapterError::InvalidEvent)
}

fn bounded(value: &str, maximum: usize) -> Result<(), AdapterError> {
    if value.is_empty() || value.len() > maximum || value.chars().any(char::is_control) {
        return Err(AdapterError::InvalidEvent);
    }
    Ok(())
}

fn push(
    events: &mut Vec<ExtensionEvent>,
    sequence: &mut u64,
    session_id: &Id,
    attempt_id: &Id,
    event: Event,
) -> Result<(), AdapterError> {
    events.push(ExtensionEvent {
        session_id: session_id.clone(),
        attempt_id: attempt_id.clone(),
        sequence: *sequence,
        event,
    });
    *sequence = sequence.checked_add(1).ok_or(AdapterError::InvalidEvent)?;
    Ok(())
}

struct UniqueJson(Value);

impl<'de> Deserialize<'de> for UniqueJson {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(UniqueJsonVisitor)
    }
}

struct UniqueJsonVisitor;

impl<'de> Visitor<'de> for UniqueJsonVisitor {
    type Value = UniqueJson;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("JSON without duplicate object members")
    }
    fn visit_bool<E: de::Error>(self, value: bool) -> Result<Self::Value, E> {
        Ok(UniqueJson(Value::Bool(value)))
    }
    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
        Ok(UniqueJson(Value::Number(value.into())))
    }
    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
        Ok(UniqueJson(Value::Number(value.into())))
    }
    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Self::Value, E> {
        serde_json::Number::from_f64(value)
            .map(Value::Number)
            .map(UniqueJson)
            .ok_or_else(|| E::custom("non-finite JSON number"))
    }
    fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
        Ok(UniqueJson(Value::String(value.into())))
    }
    fn visit_string<E: de::Error>(self, value: String) -> Result<Self::Value, E> {
        Ok(UniqueJson(Value::String(value)))
    }
    fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(UniqueJson(Value::Null))
    }
    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(UniqueJson(Value::Null))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut values: A) -> Result<Self::Value, A::Error> {
        let mut output = Vec::new();
        while let Some(value) = values.next_element::<UniqueJson>()? {
            output.push(value.0);
        }
        Ok(UniqueJson(Value::Array(output)))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut values: A) -> Result<Self::Value, A::Error> {
        let mut output = serde_json::Map::new();
        while let Some(key) = values.next_key::<String>()? {
            if output.contains_key(&key) {
                return Err(de::Error::custom("duplicate JSON object member"));
            }
            output.insert(key, values.next_value::<UniqueJson>()?.0);
        }
        Ok(UniqueJson(Value::Object(output)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::fs::symlink;
    use std::time::Duration;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir()
                .join(format!("asb-qwen-{label}-{}-{nonce}", std::process::id()));
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
            1024 * 1024,
            1024 * 1024,
            Duration::from_secs(2),
            Duration::from_millis(100),
            Duration::from_millis(5),
        )
        .unwrap()
    }

    fn process_config(script: &str) -> (Scratch, QwenCodeConfig) {
        let scratch = Scratch::new("process");
        let runtime = scratch.0.join("runtime");
        fs::create_dir_all(runtime.join("bin")).unwrap();
        fs::create_dir_all(runtime.join("node/bin")).unwrap();
        fs::create_dir_all(runtime.join("lib")).unwrap();
        let executable = runtime.join("bin/qwen");
        let node = runtime.join("node/bin/node");
        let cli = runtime.join("lib/cli-entry.js");
        let cli_main = runtime.join("lib/cli.js");
        let archive = scratch.0.join("qwen.tar.gz");
        fs::write(&executable, script).unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(&node, "node").unwrap();
        fs::write(&cli, "cli").unwrap();
        fs::write(&cli_main, "cli-main").unwrap();
        fs::write(&archive, "archive").unwrap();
        fs::create_dir_all(scratch.0.join("workspace")).unwrap();
        let mut config = QwenCodeConfig::new(
            &executable,
            &archive,
            scratch.0.join("workspace"),
            scratch.0.join("state"),
            Url::parse("http://127.0.0.1:1/v1").unwrap(),
            "fixture-model",
            QwenCodeArtifact::LinuxX86_64V0_23_0,
        )
        .unwrap();
        config.digest_overrides = Some([
            digest_file(&archive).unwrap(),
            digest_file(&executable).unwrap(),
            digest_file(&node).unwrap(),
            digest_file(&cli).unwrap(),
            digest_file(&cli_main).unwrap(),
        ]);
        (scratch, config)
    }

    #[test]
    fn configuration_and_manifest_are_bounded() {
        let config = QwenCodeConfig::new(
            "/runtime/bin/qwen",
            "/runtime.tar.gz",
            "/work",
            "/state",
            Url::parse("http://127.0.0.1:1234/v1").unwrap(),
            "fixture-model",
            QwenCodeArtifact::LinuxX86_64V0_23_0,
        )
        .unwrap();
        assert_eq!(
            config.manifest().executable_sha256,
            LINUX_X86_64_ARCHIVE_SHA256
        );
        assert!(config.manifest().capabilities.contains(&Capability::Usage));
        assert!(matches!(
            QwenCodeConfig::new(
                "relative",
                "/a",
                "/w",
                "/s",
                Url::parse("http://127.0.0.1/v1").unwrap(),
                "m",
                QwenCodeArtifact::LinuxX86_64V0_23_0,
            ),
            Err(AdapterError::RelativePath(_))
        ));
        assert!(matches!(
            QwenCodeConfig::new(
                "/q",
                "/a",
                "/w",
                "/s",
                Url::parse("http://example.invalid/v1").unwrap(),
                "m",
                QwenCodeArtifact::LinuxX86_64V0_23_0,
            ),
            Err(AdapterError::InvalidEndpoint)
        ));
        let _ = limits();
    }

    #[test]
    fn complete_tool_and_usage_stream_maps_without_content() {
        let input = concat!(
            "{\"type\":\"system\",\"subtype\":\"init\",\"uuid\":\"u\",\"session_id\":\"up\",\"qwen_code_version\":\"0.23.0\",\"permission_mode\":\"auto-edit\",\"tools\":[\"write_file\"],\"mcp_servers\":[]}\n",
            "{\"type\":\"assistant\",\"uuid\":\"a\",\"session_id\":\"up\",\"message\":{\"content\":[{\"type\":\"tool_use\",\"id\":\"call\",\"name\":\"write_file\",\"input\":{\"content\":\"secret\"}}]}}\n",
            "{\"type\":\"user\",\"session_id\":\"up\",\"message\":{\"content\":[{\"type\":\"tool_result\",\"tool_use_id\":\"call\",\"content\":\"secret\"}]}}\n",
            "{\"type\":\"assistant\",\"uuid\":\"b\",\"session_id\":\"up\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"secret\"}]}}\n",
            "{\"type\":\"result\",\"subtype\":\"success\",\"uuid\":\"r\",\"session_id\":\"up\",\"is_error\":false,\"num_turns\":2,\"usage\":{\"input_tokens\":11,\"output_tokens\":7}}\n",
        );
        let events = map_events(
            input.as_bytes(),
            &Id("s".into()),
            &Id("a".into()),
            TerminalStatus::Completed,
        )
        .unwrap();
        assert!(matches!(events[2].event, Event::FirstResponse));
        assert!(matches!(events[3].event, Event::ToolStarted { .. }));
        assert!(matches!(
            events[4].event,
            Event::ToolFinished { success: true, .. }
        ));
        assert!(matches!(
            events[5].event,
            Event::Usage(Usage {
                input_tokens: Some(11),
                output_tokens: Some(7),
                ..
            })
        ));
        assert!(matches!(events[6].event, Event::Completed));
        assert!(!format!("{events:?}").contains("secret"));
    }

    #[test]
    fn malformed_duplicate_and_causal_streams_fail_closed() {
        for input in [
            "{\"type\":\"system\",\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"s\",\"qwen_code_version\":\"0.23.0\",\"permission_mode\":\"auto-edit\",\"tools\":[],\"mcp_servers\":[]}\n",
            "{\"type\":\"assistant\",\"session_id\":\"s\",\"message\":{\"content\":[]}}\n",
            "{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"a\",\"qwen_code_version\":\"0.23.0\",\"permission_mode\":\"auto-edit\",\"tools\":[],\"mcp_servers\":[]}\n{\"type\":\"result\",\"subtype\":\"success\",\"session_id\":\"b\",\"is_error\":false}\n",
            "{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"s\",\"qwen_code_version\":\"0.23.0\",\"permission_mode\":\"auto-edit\",\"tools\":[],\"mcp_servers\":[]}\n{\"type\":\"user\",\"session_id\":\"s\",\"message\":{\"content\":[{\"type\":\"tool_result\",\"tool_use_id\":\"missing\"}]}}\n",
            "{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"s\",\"qwen_code_version\":\"0.23.0\",\"permission_mode\":\"auto-edit\",\"tools\":[\"run_shell_command\"],\"mcp_servers\":[]}\n",
            "{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"s\",\"qwen_code_version\":\"0.23.0\",\"permission_mode\":\"auto-edit\",\"tools\":[],\"mcp_servers\":[]}\n{\"type\":\"assistant\",\"session_id\":\"s\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"provider error disguised as success\"}]}}\n{\"type\":\"result\",\"subtype\":\"success\",\"session_id\":\"s\",\"is_error\":false,\"num_turns\":1,\"usage\":{\"input_tokens\":0,\"output_tokens\":0}}\n",
        ] {
            assert!(
                map_events(
                    input.as_bytes(),
                    &Id("s".into()),
                    &Id("a".into()),
                    TerminalStatus::Completed,
                )
                .is_err()
            );
        }
        let cancelled = b"{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"s\",\"qwen_code_version\":\"0.23.0\",\"permission_mode\":\"auto-edit\",\"tools\":[],\"mcp_servers\":[]}\n{\"type\":";
        assert!(
            map_events(
                cancelled,
                &Id("s".into()),
                &Id("a".into()),
                TerminalStatus::Cancelled,
            )
            .is_ok()
        );
    }

    #[test]
    fn filesystem_preflight_rejects_redirection_and_excess() {
        let scratch = Scratch::new("filesystem");
        let outside = scratch.0.join("outside");
        fs::create_dir_all(&outside).unwrap();

        let state_alias = scratch.0.join("state-alias");
        symlink(&outside, &state_alias).unwrap();
        assert!(matches!(
            prepare_state_root(&state_alias),
            Err(AdapterError::UnsafeFilesystem)
        ));
        fs::remove_file(&state_alias).unwrap();

        let parent_alias = scratch.0.join("parent-alias");
        symlink(&outside, &parent_alias).unwrap();
        assert!(matches!(
            prepare_state_root(&parent_alias.join("state")),
            Err(AdapterError::UnsafeFilesystem)
        ));
        assert!(!outside.join("state").exists());
        fs::remove_file(&parent_alias).unwrap();

        let workspace = scratch.0.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let alias = workspace.join("escape");
        symlink(&outside, &alias).unwrap();
        assert!(matches!(
            validate_workspace(&workspace),
            Err(AdapterError::UnsafeFilesystem)
        ));
        fs::remove_file(alias).unwrap();

        let oversized = workspace.join("oversized");
        fs::File::create(&oversized)
            .unwrap()
            .set_len(MAX_WORKSPACE_FILE_BYTES + 1)
            .unwrap();
        assert!(matches!(
            validate_workspace(&workspace),
            Err(AdapterError::UnsafeFilesystem)
        ));
        fs::remove_file(oversized).unwrap();

        for index in 0..=(MAX_WORKSPACE_BYTES / MAX_WORKSPACE_FILE_BYTES) {
            fs::File::create(workspace.join(format!("aggregate-{index}")))
                .unwrap()
                .set_len(MAX_WORKSPACE_FILE_BYTES)
                .unwrap();
        }
        assert!(matches!(
            validate_workspace(&workspace),
            Err(AdapterError::UnsafeFilesystem)
        ));
    }

    #[test]
    fn process_boundary_completes_cancels_and_cleans_private_state() {
        let script = r#"#!/bin/sh
cat >/dev/null
printf '%s\n' '{"type":"system","subtype":"init","session_id":"up","qwen_code_version":"0.23.0","permission_mode":"auto-edit","tools":["write_file"],"mcp_servers":[]}'
printf '%s\n' '{"type":"assistant","session_id":"up","message":{"content":[{"type":"text","text":"private"}]}}'
printf '%s\n' '{"type":"result","subtype":"success","session_id":"up","is_error":false,"num_turns":1,"usage":{"input_tokens":1,"output_tokens":1}}'
"#;
        let (scratch, config) = process_config(script);
        config.verify_executable().unwrap();
        let mut running = config
            .start(Id("s".into()), Id("a".into()), "private prompt", limits())
            .unwrap();
        assert!(running.pid() > 0);
        let outcome = running.wait().unwrap();
        assert_eq!(outcome.status(), TerminalStatus::Completed);
        assert_eq!(outcome.exit_code(), Some(0));
        assert!(!outcome.stderr_truncated());
        assert!(!format!("{:?}", outcome.events()).contains("private"));
        assert!(
            fs::read_dir(scratch.0.join("state"))
                .unwrap()
                .next()
                .is_none()
        );

        let cancel_script = r#"#!/bin/sh
printf '%s\n' '{"type":"system","subtype":"init","session_id":"up","qwen_code_version":"0.23.0","permission_mode":"auto-edit","tools":[],"mcp_servers":[]}'
sleep 60
"#;
        let (scratch, config) = process_config(cancel_script);
        let mut running = config
            .start(Id("s".into()), Id("cancel".into()), "prompt", limits())
            .unwrap();
        running.cancel().unwrap();
        running.cancel().unwrap();
        let outcome = running.wait().unwrap();
        assert_eq!(outcome.status(), TerminalStatus::Cancelled);
        assert!(
            fs::read_dir(scratch.0.join("state"))
                .unwrap()
                .next()
                .is_none()
        );
    }

    #[test]
    fn public_error_and_artifact_boundaries_are_explicit() {
        for error in [
            AdapterError::RelativePath("workspace"),
            AdapterError::InvalidEndpoint,
            AdapterError::InvalidModel,
            AdapterError::InvalidPrompt,
            AdapterError::PromptTooLarge,
            AdapterError::Io(io::Error::other("fixture")),
            AdapterError::ArtifactMismatch,
            AdapterError::UnsafeFilesystem,
            AdapterError::Process(ProcessError::Spawn(io::Error::other("fixture"))),
            AdapterError::TruncatedOutput,
            AdapterError::InvalidEvent,
        ] {
            assert!(!error.to_string().is_empty());
        }
        let (_scratch, mut config) = process_config("#!/bin/sh\nexit 0\n");
        config.digest_overrides.as_mut().unwrap()[0] = "0".repeat(64);
        assert!(matches!(
            config.verify_executable(),
            Err(AdapterError::ArtifactMismatch)
        ));
        assert!(matches!(
            config.start(Id("s".into()), Id("a".into()), "bad\0prompt", limits()),
            Err(AdapterError::InvalidPrompt)
        ));
        assert!(matches!(
            config.start(
                Id("s".into()),
                Id("a".into()),
                &"x".repeat(MAX_PROMPT_BYTES + 1),
                limits(),
            ),
            Err(AdapterError::PromptTooLarge)
        ));
    }
}
