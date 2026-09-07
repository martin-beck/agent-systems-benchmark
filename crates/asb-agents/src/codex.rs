// SPDX-License-Identifier: MIT
//! Bounded adapter for the Codex structured command-line interface.

use asb_protocol::{
    Capability, Event, ExtensionEvent, ExtensionKind, ExtensionManifest, Id, PROTOCOL_V1, RpcError,
    TerminalStatus, Usage,
};
use asb_runtime::{ProcessError, ProcessLimits, RunningProcess, Termination};
use serde::Deserialize;
use serde::de::{self, MapAccess, SeqAccess, Visitor};
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

/// Codex release whose JSON event dialect this module implements.
pub const SUPPORTED_VERSION: &str = "0.153.4";
/// SHA-256 of the natively exercised Linux x86_64 Codex 0.153.4 executable.
pub const TESTED_LINUX_X86_64_SHA256: &str =
    "56ef98ab4032d317ab26e9b5e5a175650717351edb16ed9cde0cb6d1734d62da";
const MAX_EXECUTABLE_BYTES: u64 = 1024 * 1024 * 1024;
/// Largest accepted prompt, in UTF-8 bytes.
pub const MAX_PROMPT_BYTES: usize = 4 * 1024 * 1024;
/// Largest individual Codex JSON output line.
pub const MAX_EVENT_LINE_BYTES: usize = 16 * 1024 * 1024;
/// Largest number of upstream JSON events accepted per attempt.
pub const MAX_UPSTREAM_EVENTS: usize = 65_536;
const MAX_ENDPOINT_BYTES: usize = 4 * 1024;
const MAX_MODEL_BYTES: usize = 256;
const MAX_WORKSPACE_FILES: usize = 4_096;
const MAX_WORKSPACE_ENTRIES: usize = 16_384;
const MAX_WORKSPACE_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_WORKSPACE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_PATH_BYTES: usize = 4 * 1024;

/// Content-pinned Codex artifact understood by this adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodexArtifact {
    /// Codex 0.153.4 native Linux x86_64 release executable.
    LinuxX86_64V0_153_4,
}

impl CodexArtifact {
    const fn digest(self) -> &'static str {
        match self {
            Self::LinuxX86_64V0_153_4 => TESTED_LINUX_X86_64_SHA256,
        }
    }
}

/// Validated immutable inputs for one Codex installation.
#[derive(Clone, Debug)]
pub struct CodexConfig {
    binary: PathBuf,
    workspace: PathBuf,
    state_root: PathBuf,
    endpoint: Url,
    model: String,
    artifact: CodexArtifact,
    #[cfg(test)]
    verification_digest_override: Option<String>,
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
    /// Prompt input exceeded the documented byte ceiling.
    PromptTooLarge,
    /// Local preparation or digest inspection failed.
    Io(io::Error),
    /// The executable did not match its content pin.
    ExecutableMismatch,
    /// The editable workspace was redirected or exceeded a declared bound.
    UnsafeWorkspace,
    /// Prompt-bearing state was not rooted at the exact requested directory.
    UnsafeStateRoot,
    /// Process execution failed.
    Process(ProcessError),
    /// Structured output was truncated.
    TruncatedOutput,
    /// Codex emitted malformed, inconsistent, or unsupported structured output.
    InvalidEvent,
}

impl fmt::Display for AdapterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RelativePath(label) => write!(formatter, "{label} must be absolute"),
            Self::InvalidEndpoint => formatter.write_str("invalid provider endpoint"),
            Self::InvalidModel => formatter.write_str("invalid provider/model identifier"),
            Self::PromptTooLarge => formatter.write_str("Codex prompt exceeds byte limit"),
            Self::Io(error) => write!(formatter, "adapter I/O failed: {error}"),
            Self::ExecutableMismatch => formatter.write_str("Codex executable pin mismatch"),
            Self::UnsafeWorkspace => formatter.write_str("unsafe or excessive Codex workspace"),
            Self::UnsafeStateRoot => formatter.write_str("unsafe Codex state root"),
            Self::Process(error) => write!(formatter, "Codex process failed: {error}"),
            Self::TruncatedOutput => formatter.write_str("Codex structured output was truncated"),
            Self::InvalidEvent => formatter.write_str("invalid Codex structured event"),
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

impl CodexConfig {
    /// Validate paths, endpoint, model, and select a content-pinned artifact.
    pub fn new(
        binary: impl Into<PathBuf>,
        workspace: impl Into<PathBuf>,
        state_root: impl Into<PathBuf>,
        endpoint: Url,
        model: impl Into<String>,
        artifact: CodexArtifact,
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
        if endpoint.scheme() == "http"
            && !endpoint
                .host_str()
                .is_some_and(|host| matches!(host, "127.0.0.1" | "::1" | "[::1]" | "localhost"))
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
            binary,
            workspace,
            state_root,
            endpoint,
            model,
            artifact,
            #[cfg(test)]
            verification_digest_override: None,
        })
    }

    /// Produce the explicit capabilities for this exact installation.
    #[must_use]
    pub fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            extension_id: Id("agent.codex".into()),
            kind: ExtensionKind::Agent,
            implementation_version: format!("codex-{SUPPORTED_VERSION}+asb-0.1.0"),
            protocol: PROTOCOL_V1,
            capabilities: BTreeSet::from([Capability::Cancellation]),
            executable_sha256: self.artifact.digest().into(),
        }
    }

    /// Verify the executable bytes without running untrusted configuration.
    pub fn verify_executable(&self) -> Result<(), AdapterError> {
        let digest = digest_file(&self.binary)?;
        if digest == self.verification_digest() {
            Ok(())
        } else {
            Err(AdapterError::ExecutableMismatch)
        }
    }

    fn verification_digest(&self) -> &str {
        #[cfg(test)]
        if let Some(digest) = &self.verification_digest_override {
            return digest;
        }
        self.artifact.digest()
    }

    /// Start a noninteractive attempt with prompt text absent from process arguments.
    pub fn start(
        &self,
        session_id: Id,
        attempt_id: Id,
        prompt: &str,
        limits: ProcessLimits,
    ) -> Result<RunningCodex, AdapterError> {
        if prompt.len() > MAX_PROMPT_BYTES {
            return Err(AdapterError::PromptTooLarge);
        }
        self.verify_executable()?;
        if !has_canonical_existing_ancestor(&self.workspace)? {
            return Err(AdapterError::UnsafeWorkspace);
        }
        if !has_canonical_existing_ancestor(&self.state_root)? {
            return Err(AdapterError::UnsafeStateRoot);
        }
        fs::create_dir_all(&self.workspace)?;
        validate_workspace(&self.workspace)?;
        fs::create_dir_all(&self.state_root)?;
        if !is_exact_canonical_directory(&self.state_root)? {
            return Err(AdapterError::UnsafeStateRoot);
        }
        let run_root = self.unique_run_root()?;
        fs::DirBuilder::new().mode(0o700).create(&run_root)?;
        let prepared = (|| {
            for directory in ["home", "config", "data", "cache", "prompts"] {
                fs::DirBuilder::new()
                    .mode(0o700)
                    .create(run_root.join(directory))?;
            }
            let prompt_path = run_root.join("prompts/prompt.txt");
            let mut prompt_file = OpenOptions::new()
                .write(true)
                .read(true)
                .create_new(true)
                .mode(0o600)
                .open(&prompt_path)?;
            // Unlink before copying any prompt bytes. The inherited descriptor
            // remains readable, while every preparation/spawn failure closes it
            // without leaving prompt material for directory cleanup to recover.
            fs::remove_file(&prompt_path)?;
            prompt_file.write_all(prompt.as_bytes())?;
            prompt_file.sync_all()?;
            prompt_file.seek(SeekFrom::Start(0))?;
            Ok::<fs::File, AdapterError>(prompt_file)
        })();
        let prompt_file = match prepared {
            Ok(file) => file,
            Err(error) => {
                let _ = fs::remove_dir_all(&run_root);
                return Err(error);
            }
        };

        let result = self.spawn_process(&run_root, prompt_file, limits);
        match result {
            Ok(process) => Ok(RunningCodex {
                process,
                session_id,
                attempt_id,
                prompt_path: None,
                run_root: Some(run_root),
            }),
            Err(error) => {
                let _ = fs::remove_dir_all(run_root);
                Err(error)
            }
        }
    }

    fn spawn_process(
        &self,
        run_root: &Path,
        prompt_file: fs::File,
        limits: ProcessLimits,
    ) -> Result<RunningProcess, AdapterError> {
        let mut command = Command::new(&self.binary);
        command
            .args([
                "exec",
                "--json",
                "--ephemeral",
                "--ignore-user-config",
                "--ignore-rules",
                "--strict-config",
                "--sandbox",
                "workspace-write",
                "--skip-git-repo-check",
                "-C",
            ])
            .arg(&self.workspace)
            .args(["--model", &self.model])
            .args(["-c", "model_provider=\"asb_fixture\""])
            .args([
                "-c",
                "model_providers.asb_fixture.name=\"ASB explicit provider\"",
            ])
            .arg("-c")
            .arg(format!(
                "model_providers.asb_fixture.base_url={:?}",
                self.endpoint.as_str()
            ))
            .args([
                "-c",
                "model_providers.asb_fixture.env_key=\"ASB_CODEX_PROVIDER_KEY\"",
            ])
            .args(["-c", "model_providers.asb_fixture.wire_api=\"responses\""])
            .args([
                "-c",
                "model_providers.asb_fixture.requires_openai_auth=false",
            ])
            .args(["-c", "model_providers.asb_fixture.request_max_retries=0"])
            .args(["-c", "model_providers.asb_fixture.stream_max_retries=0"])
            .args(["-c", "approval_policy=\"never\""])
            .arg("-")
            .stdin(Stdio::from(prompt_file))
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", run_root.join("home"))
            .env("XDG_CONFIG_HOME", run_root.join("config"))
            .env("XDG_DATA_HOME", run_root.join("data"))
            .env("XDG_CACHE_HOME", run_root.join("cache"))
            .env("ASB_CODEX_PROVIDER_KEY", "asb-credential-free-fixture");
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

const fn model_component_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')
}

/// A cancellable Codex process whose output has not yet been collected.
pub struct RunningCodex {
    process: RunningProcess,
    session_id: Id,
    attempt_id: Id,
    prompt_path: Option<PathBuf>,
    run_root: Option<PathBuf>,
}

impl RunningCodex {
    /// Native process identifier for metrics and ownership evidence.
    #[must_use]
    pub fn pid(&self) -> u32 {
        self.process.pid()
    }

    /// Request idempotent cancellation of the whole owned process group.
    pub fn cancel(&mut self) -> Result<(), AdapterError> {
        self.process.cancel().map_err(AdapterError::Process)
    }

    /// Reap the process, remove prompt material, and map bounded structured output.
    pub fn wait(&mut self) -> Result<CodexOutcome, AdapterError> {
        let waited = self.process.wait().cloned();
        let output = match waited {
            Ok(output) => output,
            Err(error) => {
                let _ = self.remove_prompt();
                return Err(error.into());
            }
        };
        self.remove_run_root()?;
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
        Ok(CodexOutcome {
            events,
            status,
            exit_code: output.exit_code,
            stderr_truncated: output.stderr.truncated,
        })
    }

    fn remove_prompt(&mut self) -> Result<(), AdapterError> {
        if let Some(path) = self.prompt_path.as_ref() {
            match fs::remove_file(path) {
                Ok(()) => self.prompt_path = None,
                Err(error) if error.kind() == io::ErrorKind::NotFound => self.prompt_path = None,
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    fn remove_run_root(&mut self) -> Result<(), AdapterError> {
        remove_run_tree(&mut self.run_root, &mut self.prompt_path)
    }
}

fn remove_run_tree(
    run_root: &mut Option<PathBuf>,
    prompt_path: &mut Option<PathBuf>,
) -> Result<(), AdapterError> {
    let Some(path) = run_root.as_ref() else {
        return Ok(());
    };
    match fs::remove_dir_all(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    *run_root = None;
    *prompt_path = None;
    Ok(())
}

impl Drop for RunningCodex {
    fn drop(&mut self) {
        let _ = self.process.cancel();
        let terminal = self.process.wait().is_ok();
        if terminal {
            let _ = self.remove_run_root();
        } else {
            let _ = self.remove_prompt();
        }
    }
}

/// Privacy-filtered terminal evidence for one Codex attempt.
#[derive(Clone, Debug, PartialEq)]
pub struct CodexOutcome {
    events: Vec<ExtensionEvent>,
    status: TerminalStatus,
    exit_code: Option<i32>,
    stderr_truncated: bool,
}

impl CodexOutcome {
    /// Ordered ASB lifecycle events without text/reasoning content.
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

    /// Whether diagnostic stderr exceeded its retention limit.
    #[must_use]
    pub const fn stderr_truncated(&self) -> bool {
        self.stderr_truncated
    }
}

fn digest_file(path: &Path) -> Result<String, AdapterError> {
    let mut file = fs::File::open(path)?;
    let size = file.metadata()?.len();
    if size == 0 || size > MAX_EXECUTABLE_BYTES {
        return Err(AdapterError::ExecutableMismatch);
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

fn validate_workspace(root: &Path) -> Result<(), AdapterError> {
    if !is_exact_canonical_directory(root)? {
        return Err(AdapterError::UnsafeWorkspace);
    }
    let mut pending = vec![root.to_path_buf()];
    let mut files_seen = 0_usize;
    let mut entries_seen = 0_usize;
    let mut bytes_seen = 0_u64;
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
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
                continue;
            }
            files_seen = files_seen
                .checked_add(1)
                .ok_or(AdapterError::UnsafeWorkspace)?;
            if files_seen > MAX_WORKSPACE_FILES {
                return Err(AdapterError::UnsafeWorkspace);
            }
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
        }
    }
    Ok(())
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
                let Some(parent) = candidate.parent() else {
                    return Ok(false);
                };
                candidate = parent;
            }
            Err(error) => return Err(AdapterError::Io(error)),
        }
    }
}

fn map_events(
    stdout: &[u8],
    session_id: &Id,
    attempt_id: &Id,
    status: TerminalStatus,
) -> Result<Vec<ExtensionEvent>, AdapterError> {
    let text = std::str::from_utf8(stdout).map_err(|_| AdapterError::InvalidEvent)?;
    let mut sequence = 0_u64;
    let mut events = Vec::new();
    push_event(
        &mut events,
        &mut sequence,
        session_id,
        attempt_id,
        Event::Ready,
    )?;
    push_event(
        &mut events,
        &mut sequence,
        session_id,
        attempt_id,
        Event::RequestStarted,
    )?;
    let mut first = false;
    let mut thread_started = false;
    let mut turn_started = false;
    let mut turn_terminal = false;
    let mut upstream_failed = false;
    let mut active_tools = BTreeMap::new();
    let mut upstream_events = 0_usize;
    for line in text.lines().filter(|line| !line.is_empty()) {
        upstream_events = upstream_events
            .checked_add(1)
            .ok_or(AdapterError::InvalidEvent)?;
        if upstream_events > MAX_UPSTREAM_EVENTS || line.len() > MAX_EVENT_LINE_BYTES {
            return Err(AdapterError::InvalidEvent);
        }
        let value = serde_json::from_str::<UniqueJson>(line)
            .map_err(|_| AdapterError::InvalidEvent)?
            .0;
        let object = value.as_object().ok_or(AdapterError::InvalidEvent)?;
        let kind = object
            .get("type")
            .and_then(Value::as_str)
            .ok_or(AdapterError::InvalidEvent)?;
        match kind {
            "thread.started" if !thread_started && !turn_started => {
                bounded_string(object, "thread_id")?;
                thread_started = true;
                push_event(
                    &mut events,
                    &mut sequence,
                    session_id,
                    attempt_id,
                    Event::FirstResponse,
                )?;
                first = true;
            }
            "turn.started" if thread_started && !turn_started && !turn_terminal => {
                turn_started = true;
            }
            "item.completed" if thread_started && !turn_started => {
                let item = required_object(object, "item")?;
                bounded_string(item, "id")?;
                if bounded_string(item, "type")? != "error" {
                    return Err(AdapterError::InvalidEvent);
                }
                bounded_string(item, "message")?;
            }
            "item.started" if turn_started && !turn_terminal && !upstream_failed => {
                let item = required_object(object, "item")?;
                let id = bounded_string(item, "id")?;
                let item_type = bounded_string(item, "type")?;
                let Some(name) = tool_name(item_type, item)? else {
                    if !matches!(item_type, "agent_message" | "reasoning" | "plan_update") {
                        return Err(AdapterError::InvalidEvent);
                    }
                    if !first {
                        push_event(
                            &mut events,
                            &mut sequence,
                            session_id,
                            attempt_id,
                            Event::FirstResponse,
                        )?;
                        first = true;
                    }
                    continue;
                };
                if active_tools
                    .insert(id.to_owned(), name.to_owned())
                    .is_some()
                {
                    return Err(AdapterError::InvalidEvent);
                }
                if !first {
                    push_event(
                        &mut events,
                        &mut sequence,
                        session_id,
                        attempt_id,
                        Event::FirstResponse,
                    )?;
                    first = true;
                }
                let tool_call_id = Id(id.into());
                push_event(
                    &mut events,
                    &mut sequence,
                    session_id,
                    attempt_id,
                    Event::ToolStarted {
                        tool_call_id,
                        name: name.into(),
                    },
                )?;
            }
            "item.completed" if turn_started && !turn_terminal && !upstream_failed => {
                let item = required_object(object, "item")?;
                let id = bounded_string(item, "id")?;
                let item_type = bounded_string(item, "type")?;
                if let Some(expected_name) = active_tools.remove(id) {
                    if tool_name(item_type, item)? != Some(expected_name.as_str()) {
                        return Err(AdapterError::InvalidEvent);
                    }
                    let status = bounded_string(item, "status")?;
                    let success = matches!(status, "completed");
                    if !success && !matches!(status, "failed" | "declined") {
                        return Err(AdapterError::InvalidEvent);
                    }
                    push_event(
                        &mut events,
                        &mut sequence,
                        session_id,
                        attempt_id,
                        Event::ToolFinished {
                            tool_call_id: Id(id.into()),
                            success,
                        },
                    )?;
                } else {
                    if !matches!(item_type, "agent_message" | "reasoning" | "plan_update") {
                        return Err(AdapterError::InvalidEvent);
                    }
                    if !first {
                        push_event(
                            &mut events,
                            &mut sequence,
                            session_id,
                            attempt_id,
                            Event::FirstResponse,
                        )?;
                        first = true;
                    }
                }
            }
            "turn.completed" if turn_started && !turn_terminal && active_tools.is_empty() => {
                if let Some(usage) = parse_usage(object)? {
                    push_event(
                        &mut events,
                        &mut sequence,
                        session_id,
                        attempt_id,
                        Event::Usage(usage),
                    )?;
                }
                turn_terminal = true;
            }
            "turn.failed" if turn_started && !turn_terminal => {
                required_object(object, "error")?;
                upstream_failed = true;
                turn_terminal = true;
            }
            "error"
                if !upstream_failed && object.get("message").and_then(Value::as_str).is_some() =>
            {
                upstream_failed = true;
            }
            _ => return Err(AdapterError::InvalidEvent),
        }
    }
    match status {
        TerminalStatus::Completed
            if upstream_failed
                || !first
                || !thread_started
                || !turn_started
                || !turn_terminal
                || !active_tools.is_empty() =>
        {
            return Err(AdapterError::InvalidEvent);
        }
        TerminalStatus::Completed => push_event(
            &mut events,
            &mut sequence,
            session_id,
            attempt_id,
            Event::Completed,
        )?,
        TerminalStatus::Failed => push_event(
            &mut events,
            &mut sequence,
            session_id,
            attempt_id,
            Event::Failed(RpcError {
                code: -32_100,
                message: "Codex attempt failed; raw diagnostics are not retained".into(),
                data: None,
            }),
        )?,
        TerminalStatus::Cancelled => {}
    }
    Ok(events)
}

fn required_object<'a>(
    object: &'a serde_json::Map<String, Value>,
    key: &str,
) -> Result<&'a serde_json::Map<String, Value>, AdapterError> {
    object
        .get(key)
        .and_then(Value::as_object)
        .ok_or(AdapterError::InvalidEvent)
}

struct UniqueJson(Value);

impl<'de> Deserialize<'de> for UniqueJson {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(UniqueJsonVisitor)
    }
}

struct UniqueJsonVisitor;

impl<'de> Visitor<'de> for UniqueJsonVisitor {
    type Value = UniqueJson;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("JSON without duplicate object members")
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
        Ok(UniqueJson(Value::String(value.to_owned())))
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
            let value = values.next_value::<UniqueJson>()?;
            output.insert(key, value.0);
        }
        Ok(UniqueJson(Value::Object(output)))
    }
}

fn parse_usage(value: &serde_json::Map<String, Value>) -> Result<Option<Usage>, AdapterError> {
    let tokens = value.get("usage").and_then(Value::as_object);
    let Some(tokens) = tokens else {
        return Ok(None);
    };
    let input_tokens = optional_u64(tokens.get("input_tokens"))?;
    let output_tokens = optional_u64(tokens.get("output_tokens"))?;
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

fn required_string<'a>(
    object: &'a serde_json::Map<String, Value>,
    key: &str,
) -> Result<&'a str, AdapterError> {
    object
        .get(key)
        .and_then(Value::as_str)
        .ok_or(AdapterError::InvalidEvent)
}

fn bounded_string<'a>(
    object: &'a serde_json::Map<String, Value>,
    key: &str,
) -> Result<&'a str, AdapterError> {
    let value = required_string(object, key)?;
    if value.is_empty() || value.len() > 4_096 || value.bytes().any(|byte| byte < b' ') {
        return Err(AdapterError::InvalidEvent);
    }
    Ok(value)
}

fn tool_name<'a>(
    item_type: &'a str,
    _item: &'a serde_json::Map<String, Value>,
) -> Result<Option<&'static str>, AdapterError> {
    Ok(match item_type {
        "command_execution" => Some("command_execution"),
        "file_change" => Some("file_change"),
        "mcp_tool_call" => Some("mcp_tool_call"),
        "web_search" => Some("web_search"),
        "agent_message" | "reasoning" | "plan_update" => None,
        _ => return Err(AdapterError::InvalidEvent),
    })
}

fn push_event(
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    const ROOT: &str = "/srv/data/projects/.asb-local/ar0304-unit";
    fn config(endpoint: &str) -> CodexConfig {
        CodexConfig::new(
            "/bin/true",
            format!("{ROOT}/work"),
            format!("{ROOT}/state"),
            Url::parse(endpoint).unwrap(),
            "fixture-model",
            CodexArtifact::LinuxX86_64V0_153_4,
        )
        .unwrap()
    }
    fn completed() -> &'static [u8] {
        br#"{"type":"thread.started","thread_id":"thread-1"}
{"type":"turn.started"}
{"type":"item.started","item":{"id":"tool-1","type":"command_execution","status":"in_progress","command":"private"}}
{"type":"item.completed","item":{"id":"tool-1","type":"command_execution","status":"completed","aggregated_output":"private"}}
{"type":"item.completed","item":{"id":"message-1","type":"agent_message","text":"private response"}}
{"type":"turn.completed","usage":{"input_tokens":7,"cached_input_tokens":2,"output_tokens":3}}
"#
    }
    #[test]
    fn configuration_and_manifest_are_narrow() {
        for endpoint in [
            "ftp://example.invalid/v1",
            "https://user@example.invalid/v1",
            "https://example.invalid/v1?q=x",
            "http://example.invalid/v1",
        ] {
            assert!(matches!(
                CodexConfig::new(
                    "/bin/true",
                    format!("{ROOT}/work"),
                    format!("{ROOT}/state"),
                    Url::parse(endpoint).unwrap(),
                    "fixture-model",
                    CodexArtifact::LinuxX86_64V0_153_4
                ),
                Err(AdapterError::InvalidEndpoint)
            ));
        }
        for model in ["", "bad model", "bad/model"] {
            assert!(matches!(
                CodexConfig::new(
                    "/bin/true",
                    format!("{ROOT}/work"),
                    format!("{ROOT}/state"),
                    Url::parse("https://example.invalid/v1").unwrap(),
                    model,
                    CodexArtifact::LinuxX86_64V0_153_4
                ),
                Err(AdapterError::InvalidModel)
            ));
        }
        let manifest = config("http://127.0.0.1:1/v1").manifest();
        assert_eq!(
            manifest.capabilities,
            BTreeSet::from([Capability::Cancellation])
        );
        assert_eq!(manifest.executable_sha256, TESTED_LINUX_X86_64_SHA256);
    }
    #[test]
    fn maps_tools_usage_and_discards_content() {
        let events = map_events(
            completed(),
            &Id("s".into()),
            &Id("a".into()),
            TerminalStatus::Completed,
        )
        .unwrap();
        assert!(
            events
                .iter()
                .any(|e| matches!(&e.event, Event::ToolStarted {
            tool_call_id, name } if tool_call_id.0 == "tool-1" && name == "command_execution"))
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e.event, Event::ToolFinished { success: true, .. }))
        );
        assert!(events.iter().any(|e| matches!(
            e.event,
            Event::Usage(Usage {
                input_tokens: Some(7),
                output_tokens: Some(3),
                ..
            })
        )));
        assert!(matches!(events.last().unwrap().event, Event::Completed));
        assert!(!serde_json::to_string(&events).unwrap().contains("private"));
    }
    #[test]
    fn accepts_content_free_pre_turn_model_metadata_warning() {
        let input = br#"{"type":"thread.started","thread_id":"thread-1"}
{"type":"item.completed","item":{"id":"item-0","type":"error","message":"fixture warning"}}
{"type":"turn.started"}
{"type":"item.completed","item":{"id":"message-1","type":"agent_message","text":"private"}}
{"type":"turn.completed","usage":null}
"#;
        let events = map_events(
            input,
            &Id("s".into()),
            &Id("a".into()),
            TerminalStatus::Completed,
        )
        .unwrap();
        assert!(matches!(events.last().unwrap().event, Event::Completed));
        assert!(!serde_json::to_string(&events).unwrap().contains("warning"));
    }
    #[test]
    fn malformed_and_inconsistent_streams_fail_closed() {
        for input in [
            r#"{"type":"turn.started"}"#,
            r#"{"type":"thread.started","thread_id":"a","thread_id":"b"}"#,
            "{\"type\":\"thread.started\",\"thread_id\":\"a\"}\n{\"type\":\"turn.started\"}\n{\"type\":\"item.started\",\"item\":{\"id\":\"x\",\"type\":\"unknown\"}}",
            "{\"type\":\"thread.started\",\"thread_id\":\"a\"}\n{\"type\":\"turn.started\"}\n{\"type\":\"item.completed\",\"item\":{\"id\":\"x\",\"type\":\"command_execution\",\"status\":\"completed\"}}",
        ] {
            assert!(matches!(
                map_events(
                    input.as_bytes(),
                    &Id("s".into()),
                    &Id("a".into()),
                    TerminalStatus::Completed
                ),
                Err(AdapterError::InvalidEvent)
            ));
        }
        let oversized = format!(
            "{{\"type\":\"thread.started\",\"thread_id\":\"{}\"}}",
            "x".repeat(MAX_EVENT_LINE_BYTES)
        );
        assert!(matches!(
            map_events(
                oversized.as_bytes(),
                &Id("s".into()),
                &Id("a".into()),
                TerminalStatus::Completed
            ),
            Err(AdapterError::InvalidEvent)
        ));
    }
    #[test]
    fn failures_are_redacted_and_cancellation_is_terminal() {
        let failed = br#"{"type":"thread.started","thread_id":"t"}
{"type":"turn.started"}
{"type":"turn.failed","error":{"message":"secret provider error"}}"#;
        let events = map_events(
            failed,
            &Id("s".into()),
            &Id("a".into()),
            TerminalStatus::Failed,
        )
        .unwrap();
        assert!(matches!(events.last().unwrap().event, Event::Failed(_)));
        assert!(!serde_json::to_string(&events).unwrap().contains("secret"));
        let cancelled = map_events(
            br#"{"type":"thread.started","thread_id":"t"}
{"type":"turn.started"}"#,
            &Id("s".into()),
            &Id("a".into()),
            TerminalStatus::Cancelled,
        )
        .unwrap();
        assert_eq!(cancelled.len(), 3);
    }
    #[test]
    fn process_boundary_unlinks_prompt_and_cancels_owned_child() {
        let root = PathBuf::from(ROOT).join(format!("process-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let binary = root.join("codex-fixture");
        fs::write(&binary, "#!/bin/sh\nread prompt\nsleep 60 &\nwait\n").unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let mut adapter = CodexConfig::new(
            &binary,
            root.join("work"),
            root.join("state"),
            Url::parse("http://127.0.0.1:1/v1").unwrap(),
            "fixture-model",
            CodexArtifact::LinuxX86_64V0_153_4,
        )
        .unwrap();
        adapter.verification_digest_override = Some(digest_file(&binary).unwrap());
        let limits = ProcessLimits::default();
        let mut running = adapter
            .start(Id("s".into()), Id("a".into()), "private", limits)
            .unwrap();
        let pid = running.pid();
        std::thread::sleep(std::time::Duration::from_millis(100));
        running.cancel().unwrap();
        let outcome = running.wait().unwrap();
        assert_eq!(outcome.status(), TerminalStatus::Cancelled);
        assert!(!Path::new(&format!("/proc/{pid}")).exists());
        assert!(fs::read_dir(root.join("state")).unwrap().next().is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn public_errors_outcome_and_digest_boundaries_are_covered() {
        let io_error = AdapterError::from(io::Error::other("fixture"));
        let process_error = AdapterError::from(ProcessError::Spawn(io::Error::other("fixture")));
        for error in [
            AdapterError::RelativePath("workspace"),
            AdapterError::InvalidEndpoint,
            AdapterError::InvalidModel,
            AdapterError::PromptTooLarge,
            io_error,
            AdapterError::ExecutableMismatch,
            AdapterError::UnsafeWorkspace,
            AdapterError::UnsafeStateRoot,
            process_error,
            AdapterError::TruncatedOutput,
            AdapterError::InvalidEvent,
        ] {
            assert!(!error.to_string().is_empty());
        }
        let outcome = CodexOutcome {
            events: Vec::new(),
            status: TerminalStatus::Failed,
            exit_code: Some(17),
            stderr_truncated: true,
        };
        assert!(outcome.events().is_empty());
        assert_eq!(outcome.exit_code(), Some(17));
        assert!(outcome.stderr_truncated());

        for (label, binary, workspace, state) in [
            ("binary", "relative", "/work", "/state"),
            ("workspace", "/bin/true", "relative", "/state"),
            ("state root", "/bin/true", "/work", "relative"),
        ] {
            assert!(matches!(
                CodexConfig::new(
                    binary,
                    workspace,
                    state,
                    Url::parse("https://example.invalid/v1").unwrap(),
                    "model",
                    CodexArtifact::LinuxX86_64V0_153_4,
                ),
                Err(AdapterError::RelativePath(found)) if found == label
            ));
        }
        let root = PathBuf::from(ROOT).join("digest-boundaries");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let empty = root.join("empty");
        fs::write(&empty, []).unwrap();
        assert!(matches!(
            digest_file(&empty),
            Err(AdapterError::ExecutableMismatch)
        ));
        let huge = root.join("huge");
        fs::File::create(&huge)
            .unwrap()
            .set_len(MAX_EXECUTABLE_BYTES + 1)
            .unwrap();
        assert!(matches!(
            digest_file(&huge),
            Err(AdapterError::ExecutableMismatch)
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn real_wait_paths_cover_success_failure_truncation_and_prompt_limit() {
        let root = PathBuf::from(ROOT).join("wait-paths");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let success = root.join("success");
        fs::write(
            &success,
            "#!/bin/sh\nread prompt\nprintf '%s\\n' '{\"type\":\"thread.started\",\"thread_id\":\"t\"}' '{\"type\":\"turn.started\"}' '{\"type\":\"item.completed\",\"item\":{\"id\":\"m\",\"type\":\"agent_message\",\"text\":\"x\"}}' '{\"type\":\"turn.completed\",\"usage\":null}'\n",
        )
        .unwrap();
        fs::set_permissions(&success, fs::Permissions::from_mode(0o700)).unwrap();
        let mut adapter = CodexConfig::new(
            &success,
            root.join("work"),
            root.join("state"),
            Url::parse("http://127.0.0.1:1/v1").unwrap(),
            "model",
            CodexArtifact::LinuxX86_64V0_153_4,
        )
        .unwrap();
        adapter.verification_digest_override = Some(digest_file(&success).unwrap());
        adapter.verify_executable().unwrap();
        let mut running = adapter
            .start(
                Id("s".into()),
                Id("a".into()),
                "prompt",
                ProcessLimits::default(),
            )
            .unwrap();
        let output = running.wait().unwrap();
        assert_eq!(output.status(), TerminalStatus::Completed);
        assert_eq!(output.exit_code(), Some(0));
        assert!(!output.stderr_truncated());

        let failed = root.join("failed");
        fs::write(&failed, "#!/bin/sh\nexit 9\n").unwrap();
        fs::set_permissions(&failed, fs::Permissions::from_mode(0o700)).unwrap();
        adapter.binary = failed.clone();
        adapter.verification_digest_override = Some(digest_file(&failed).unwrap());
        let mut running = adapter
            .start(
                Id("s".into()),
                Id("failed".into()),
                "prompt",
                ProcessLimits::default(),
            )
            .unwrap();
        assert_eq!(running.wait().unwrap().status(), TerminalStatus::Failed);

        let noisy = root.join("noisy");
        fs::write(
            &noisy,
            "#!/bin/sh\nprintf '0123456789012345678901234567890123456789'\n",
        )
        .unwrap();
        fs::set_permissions(&noisy, fs::Permissions::from_mode(0o700)).unwrap();
        adapter.binary = noisy.clone();
        adapter.verification_digest_override = Some(digest_file(&noisy).unwrap());
        let limits = ProcessLimits::new(
            8,
            8,
            std::time::Duration::from_secs(5),
            std::time::Duration::from_millis(50),
            std::time::Duration::from_millis(5),
        )
        .unwrap();
        let mut running = adapter
            .start(Id("s".into()), Id("noisy".into()), "prompt", limits)
            .unwrap();
        assert!(matches!(running.wait(), Err(AdapterError::TruncatedOutput)));
        assert!(matches!(
            adapter.start(
                Id("s".into()),
                Id("large".into()),
                &"x".repeat(MAX_PROMPT_BYTES + 1),
                ProcessLimits::default(),
            ),
            Err(AdapterError::PromptTooLarge)
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn workspace_and_state_preflight_reject_redirection_and_excess() {
        let root = PathBuf::from(ROOT).join(format!("preflight-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let mut adapter = CodexConfig::new(
            "/bin/true",
            root.join("workspace"),
            root.join("state"),
            Url::parse("http://127.0.0.1:1/v1").unwrap(),
            "model",
            CodexArtifact::LinuxX86_64V0_153_4,
        )
        .unwrap();
        adapter.verification_digest_override = Some(digest_file(Path::new("/bin/true")).unwrap());

        let outside_state = root.join("outside-state");
        fs::create_dir_all(&outside_state).unwrap();
        std::os::unix::fs::symlink(&outside_state, &adapter.state_root).unwrap();
        assert!(matches!(
            adapter.start(
                Id("s".into()),
                Id("state".into()),
                "prompt",
                ProcessLimits::default()
            ),
            Err(AdapterError::UnsafeStateRoot)
        ));
        assert!(fs::read_dir(&outside_state).unwrap().next().is_none());
        fs::remove_file(&adapter.state_root).unwrap();

        let actual_state_parent = root.join("actual-state-parent");
        let alias_state_parent = root.join("alias-state-parent");
        fs::create_dir_all(&actual_state_parent).unwrap();
        std::os::unix::fs::symlink(&actual_state_parent, &alias_state_parent).unwrap();
        adapter.state_root = alias_state_parent.join("state");
        assert!(matches!(
            adapter.start(
                Id("s".into()),
                Id("ancestor".into()),
                "prompt",
                ProcessLimits::default()
            ),
            Err(AdapterError::UnsafeStateRoot)
        ));
        assert!(!actual_state_parent.join("state").exists());
        fs::remove_file(alias_state_parent).unwrap();
        adapter.state_root = root.join("safe-state");

        let outside_workspace = root.join("outside-workspace");
        fs::create_dir_all(&outside_workspace).unwrap();
        std::os::unix::fs::symlink(&outside_workspace, &adapter.workspace).unwrap();
        assert!(matches!(
            adapter.start(
                Id("s".into()),
                Id("workspace".into()),
                "prompt",
                ProcessLimits::default()
            ),
            Err(AdapterError::UnsafeWorkspace)
        ));
        fs::remove_file(&adapter.workspace).unwrap();

        let actual_workspace_parent = root.join("actual-workspace-parent");
        let alias_workspace_parent = root.join("alias-workspace-parent");
        fs::create_dir_all(&actual_workspace_parent).unwrap();
        std::os::unix::fs::symlink(&actual_workspace_parent, &alias_workspace_parent).unwrap();
        adapter.workspace = alias_workspace_parent.join("workspace");
        assert!(matches!(
            adapter.start(
                Id("s".into()),
                Id("workspace-ancestor".into()),
                "prompt",
                ProcessLimits::default()
            ),
            Err(AdapterError::UnsafeWorkspace)
        ));
        assert!(!actual_workspace_parent.join("workspace").exists());
        fs::remove_file(alias_workspace_parent).unwrap();

        adapter.workspace = root.join("bounded-workspace");
        fs::create_dir_all(&adapter.workspace).unwrap();
        let outside_file = root.join("outside-file");
        fs::write(&outside_file, "outside").unwrap();
        let alias = adapter.workspace.join("alias");
        std::os::unix::fs::symlink(&outside_file, &alias).unwrap();
        assert!(matches!(
            validate_workspace(&adapter.workspace),
            Err(AdapterError::UnsafeWorkspace)
        ));
        fs::remove_file(alias).unwrap();

        let oversized = adapter.workspace.join("oversized");
        fs::File::create(&oversized)
            .unwrap()
            .set_len(MAX_WORKSPACE_FILE_BYTES + 1)
            .unwrap();
        assert!(matches!(
            validate_workspace(&adapter.workspace),
            Err(AdapterError::UnsafeWorkspace)
        ));
        fs::remove_file(oversized).unwrap();

        for index in 0..=(MAX_WORKSPACE_BYTES / MAX_WORKSPACE_FILE_BYTES) {
            fs::File::create(adapter.workspace.join(format!("aggregate-{index}")))
                .unwrap()
                .set_len(MAX_WORKSPACE_FILE_BYTES)
                .unwrap();
        }
        assert!(matches!(
            validate_workspace(&adapter.workspace),
            Err(AdapterError::UnsafeWorkspace)
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn every_supported_item_and_json_scalar_is_bounded() {
        let input = br#"{"type":"thread.started","thread_id":"t","extra":[null,true,-1,2,1.5]}
{"type":"turn.started"}
{"type":"item.started","item":{"id":"f","type":"file_change"}}
{"type":"item.completed","item":{"id":"f","type":"file_change","status":"failed"}}
{"type":"item.started","item":{"id":"m","type":"mcp_tool_call"}}
{"type":"item.completed","item":{"id":"m","type":"mcp_tool_call","status":"declined"}}
{"type":"item.started","item":{"id":"w","type":"web_search"}}
{"type":"item.completed","item":{"id":"w","type":"web_search","status":"completed"}}
{"type":"item.started","item":{"id":"r","type":"reasoning"}}
{"type":"item.completed","item":{"id":"r","type":"reasoning"}}
{"type":"turn.completed","usage":{"output_tokens":2}}
"#;
        let events = map_events(
            input,
            &Id("s".into()),
            &Id("a".into()),
            TerminalStatus::Completed,
        )
        .unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event.event, Event::ToolStarted { .. }))
                .count(),
            3
        );
        assert!(matches!(parse_usage(&serde_json::Map::new()), Ok(None)));
        assert!(matches!(
            optional_u64(Some(&Value::String("bad".into()))),
            Err(AdapterError::InvalidEvent)
        ));
    }
}
