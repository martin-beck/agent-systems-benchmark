// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Bounded adapter for the OpenDesk structured command-line interface.

use asb_protocol::{
    Capability, Event, ExtensionEvent, ExtensionKind, ExtensionManifest, Id, PROTOCOL_V1, RpcError,
    TerminalStatus, Usage,
};
use asb_runtime::{ProcessError, ProcessLimits, RunningProcess, Termination};
use serde::Deserialize;
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use url::Url;

/// OpenDesk release whose JSON event dialect this module implements.
pub const SUPPORTED_VERSION: &str = "0.3.5";
/// Immutable upstream Git revision inspected for this adapter.
pub const UPSTREAM_REVISION: &str = "f303069da72412dc90b3214d89da9c102282465f";
/// SHA-256 of the inspected `@bitclub.ai/opendesk-cli@0.3.5` npm tarball.
pub const NPM_TARBALL_SHA256: &str =
    "b55e83daf01349f278bd0e788f9c6cbbad700b2f2203309760c6e618070b930d";
/// SHA-256 of the natively exercised Linux x86_64 OpenDesk 0.3.5 executable.
pub const TESTED_LINUX_X86_64_SHA256: &str =
    "e55bd82d00612f57a3e3c7ef7bf70b5b2d988f8f548c9f18a67b95b8701e2282";
/// SHA-256 of the exercised Node.js 26.3.0 Linux x86_64 runtime.
pub const TESTED_NODE_LINUX_X86_64_SHA256: &str =
    "5325ac9da58541494afcc136f0880279a2a853609bf4dae7755e04fb682b6926";
const MAX_EXECUTABLE_BYTES: u64 = 1024 * 1024 * 1024;
/// Largest accepted prompt, in UTF-8 bytes.
pub const MAX_PROMPT_BYTES: usize = 4 * 1024 * 1024;
/// Largest accepted OpenDesk export document.
pub const MAX_EXPORT_BYTES: u64 = 32 * 1024 * 1024;
/// Largest accepted number of stored upstream messages or tool calls.
pub const MAX_UPSTREAM_ITEMS: usize = 65_536;
const MAX_ENDPOINT_BYTES: usize = 4 * 1024;
const MAX_MODEL_BYTES: usize = 256;
const OPENDESK_TELEMETRY_HOST: &str = "opendesk.matrix.openharmony.cn";

/// Content-pinned OpenDesk artifact understood by this adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpenDeskArtifact {
    /// OpenDesk 0.3.5 npm executable, exercised natively on Linux x86_64.
    LinuxX86_64V0_3_5,
}

impl OpenDeskArtifact {
    const fn digest(self) -> &'static str {
        match self {
            Self::LinuxX86_64V0_3_5 => TESTED_LINUX_X86_64_SHA256,
        }
    }
}

/// Validated immutable inputs for one OpenDesk installation.
#[derive(Clone, Debug)]
pub struct OpenDeskConfig {
    binary: PathBuf,
    node_binary: PathBuf,
    workspace: PathBuf,
    state_root: PathBuf,
    endpoint: Url,
    model: String,
    artifact: OpenDeskArtifact,
    #[cfg(test)]
    verification_digest_override: Option<String>,
    #[cfg(test)]
    runtime_digest_override: Option<String>,
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
    /// The executable did not match its content pin.
    ExecutableMismatch,
    /// The required content-pinned Node.js runtime did not match.
    RuntimeMismatch,
    /// Process execution failed.
    Process(ProcessError),
    /// Structured output was truncated.
    TruncatedOutput,
    /// OpenDesk emitted malformed, inconsistent, or unsupported structured output.
    InvalidEvent,
}

impl fmt::Display for AdapterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RelativePath(label) => write!(formatter, "{label} must be absolute"),
            Self::InvalidEndpoint => formatter.write_str("invalid provider endpoint"),
            Self::InvalidModel => formatter.write_str("invalid provider/model identifier"),
            Self::InvalidPrompt => formatter.write_str("invalid OpenDesk prompt"),
            Self::PromptTooLarge => formatter.write_str("OpenDesk prompt exceeds byte limit"),
            Self::Io(error) => write!(formatter, "adapter I/O failed: {error}"),
            Self::ExecutableMismatch => formatter.write_str("OpenDesk executable pin mismatch"),
            Self::RuntimeMismatch => formatter.write_str("OpenDesk Node.js runtime pin mismatch"),
            Self::Process(error) => write!(formatter, "OpenDesk process failed: {error}"),
            Self::TruncatedOutput => {
                formatter.write_str("OpenDesk structured output was truncated")
            }
            Self::InvalidEvent => formatter.write_str("invalid OpenDesk structured event"),
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

impl OpenDeskConfig {
    /// Validate paths, endpoint, model, and select a content-pinned artifact.
    pub fn new(
        binary: impl Into<PathBuf>,
        node_binary: impl Into<PathBuf>,
        workspace: impl Into<PathBuf>,
        state_root: impl Into<PathBuf>,
        endpoint: Url,
        model: impl Into<String>,
        artifact: OpenDeskArtifact,
    ) -> Result<Self, AdapterError> {
        let binary = binary.into();
        let node_binary = node_binary.into();
        let workspace = workspace.into();
        let state_root = state_root.into();
        for (label, path) in [
            ("binary", binary.as_path()),
            ("Node.js runtime", node_binary.as_path()),
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
        if endpoint.host_str().is_some_and(telemetry_bypasses_proxy) {
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
            node_binary,
            workspace,
            state_root,
            endpoint,
            model,
            artifact,
            #[cfg(test)]
            verification_digest_override: None,
            #[cfg(test)]
            runtime_digest_override: None,
        })
    }

    /// Produce the explicit capabilities for this exact installation.
    #[must_use]
    pub fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            extension_id: Id("agent.opendesk".into()),
            kind: ExtensionKind::Agent,
            implementation_version: format!("opendesk-{SUPPORTED_VERSION}+asb-0.1.0"),
            protocol: PROTOCOL_V1,
            capabilities: BTreeSet::from([Capability::Cancellation]),
            executable_sha256: self.artifact.digest().into(),
        }
    }

    /// Verify the executable bytes without running untrusted configuration.
    pub fn verify_executable(&self) -> Result<(), AdapterError> {
        let digest = digest_file(&self.binary)?;
        if digest != self.verification_digest() {
            Err(AdapterError::ExecutableMismatch)
        } else {
            match digest_file(&self.node_binary) {
                Ok(runtime_digest) if runtime_digest == self.runtime_verification_digest() => {
                    Ok(())
                }
                Ok(_) | Err(AdapterError::ExecutableMismatch) => Err(AdapterError::RuntimeMismatch),
                Err(error) => Err(error),
            }
        }
    }

    fn runtime_verification_digest(&self) -> &str {
        #[cfg(test)]
        if let Some(digest) = &self.runtime_digest_override {
            return digest;
        }
        TESTED_NODE_LINUX_X86_64_SHA256
    }

    fn verification_digest(&self) -> &str {
        #[cfg(test)]
        if let Some(digest) = &self.verification_digest_override {
            return digest;
        }
        self.artifact.digest()
    }

    /// Start a noninteractive attempt.
    ///
    /// OpenDesk 0.3.5 requires the prompt in `-c` and therefore exposes it to
    /// same-UID process inspection while the process is alive. Callers must not
    /// use this adapter for secret prompts.
    pub fn start(
        &self,
        session_id: Id,
        attempt_id: Id,
        prompt: &str,
        limits: ProcessLimits,
    ) -> Result<RunningOpenDesk, AdapterError> {
        if prompt.len() > MAX_PROMPT_BYTES {
            return Err(AdapterError::PromptTooLarge);
        }
        if prompt.contains('\0') {
            return Err(AdapterError::InvalidPrompt);
        }
        self.verify_executable()?;
        fs::create_dir_all(&self.workspace)?;
        let run_root = self.route_run_root(&session_id, &attempt_id);
        fs::create_dir_all(&self.state_root)?;
        fs::DirBuilder::new().mode(0o700).create(&run_root)?;
        let prepared = (|| {
            for directory in ["home", "config", "data", "cache", "tmp"] {
                fs::DirBuilder::new()
                    .mode(0o700)
                    .create(run_root.join(directory))?;
            }
            Ok::<(), AdapterError>(())
        })();
        match prepared {
            Ok(()) => {}
            Err(error) => {
                let _ = fs::remove_dir_all(&run_root);
                return Err(error);
            }
        }

        let export_path = run_root.join("result.json");
        let result = self.spawn_process(&run_root, &export_path, prompt, limits);
        match result {
            Ok(process) => Ok(RunningOpenDesk {
                process,
                session_id,
                attempt_id,
                export_path,
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
        export_path: &Path,
        prompt: &str,
        limits: ProcessLimits,
    ) -> Result<RunningProcess, AdapterError> {
        let no_proxy = self
            .endpoint
            .host_str()
            .ok_or(AdapterError::InvalidEndpoint)?;
        let mut command = Command::new(&self.node_binary);
        command
            .arg("--use-env-proxy")
            .arg(&self.binary)
            .args(["--workspace"])
            .arg(&self.workspace)
            .args(["--headless", "--mode", "standard", "--language", "en-US"])
            .args(["--config-directory"])
            .arg(run_root.join("config"))
            .args(["--export-json"])
            .arg(export_path)
            .args(["--command", prompt])
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", run_root.join("home"))
            .env("XDG_CONFIG_HOME", run_root.join("config"))
            .env("XDG_DATA_HOME", run_root.join("data"))
            .env("XDG_CACHE_HOME", run_root.join("cache"))
            .env("TMPDIR", run_root.join("tmp"))
            .env("HTTP_PROXY", "http://127.0.0.1:9")
            .env("HTTPS_PROXY", "http://127.0.0.1:9")
            .env("NO_PROXY", no_proxy)
            .env("NODE_USE_ENV_PROXY", "1")
            .env("OPENDESK_MODEL_NAME", &self.model)
            .env("OPENDESK_PROVIDER_TYPE", "openai")
            .env("OPENDESK_PROVIDER_BASE_URL", self.endpoint.as_str())
            .env("OPENDESK_PROVIDER_API_KEY", "asb-credential-free");
        RunningProcess::spawn(command, limits).map_err(AdapterError::Process)
    }

    fn route_run_root(&self, session_id: &Id, attempt_id: &Id) -> PathBuf {
        let mut hasher = Sha256::new();
        hasher.update(b"asb-opendesk-route-v1\0");
        hasher.update(Sha256::digest(session_id.0.as_bytes()));
        hasher.update(Sha256::digest(attempt_id.0.as_bytes()));
        self.state_root
            .join(format!("attempt-{:x}", hasher.finalize()))
    }
}

const fn model_component_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')
}

fn telemetry_bypasses_proxy(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    OPENDESK_TELEMETRY_HOST == host
        || OPENDESK_TELEMETRY_HOST
            .strip_suffix(&host)
            .is_some_and(|prefix| prefix.ends_with('.'))
}

/// A cancellable OpenDesk process whose output has not yet been collected.
pub struct RunningOpenDesk {
    process: RunningProcess,
    session_id: Id,
    attempt_id: Id,
    export_path: PathBuf,
    run_root: Option<PathBuf>,
}

impl RunningOpenDesk {
    /// Native process identifier for metrics and ownership evidence.
    #[must_use]
    pub fn pid(&self) -> u32 {
        self.process.pid()
    }

    /// Request idempotent cancellation of the whole owned process group.
    pub fn cancel(&mut self) -> Result<(), AdapterError> {
        self.process.cancel().map_err(AdapterError::Process)
    }

    /// Reap the process, remove all raw state, and map bounded structural evidence.
    pub fn wait(&mut self) -> Result<OpenDeskOutcome, AdapterError> {
        let waited = self.process.wait().cloned();
        let output = match waited {
            Ok(output) => output,
            Err(error) => {
                let _ = self.remove_run_root();
                return Err(error.into());
            }
        };
        let process_status = match output.termination {
            Termination::Cancelled => TerminalStatus::Cancelled,
            Termination::TimedOut => TerminalStatus::Failed,
            Termination::Exited if output.exit_code == Some(0) => TerminalStatus::Completed,
            Termination::Exited => TerminalStatus::Failed,
        };
        let export_result = if process_status == TerminalStatus::Completed {
            read_export(&self.export_path).map(Some)
        } else {
            Ok(None)
        };
        self.remove_run_root()?;
        let export = export_result?;
        let (events, status) = map_export(
            export.as_deref(),
            &self.session_id,
            &self.attempt_id,
            process_status,
        )?;
        Ok(OpenDeskOutcome {
            events,
            status,
            exit_code: output.exit_code,
            stderr_truncated: output.stderr.truncated,
        })
    }

    fn remove_run_root(&mut self) -> Result<(), AdapterError> {
        remove_run_tree(&mut self.run_root)
    }
}

fn remove_run_tree(run_root: &mut Option<PathBuf>) -> Result<(), AdapterError> {
    let Some(path) = run_root.as_ref() else {
        return Ok(());
    };
    match fs::remove_dir_all(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    *run_root = None;
    Ok(())
}

impl Drop for RunningOpenDesk {
    fn drop(&mut self) {
        let _ = self.process.cancel();
        let terminal = self.process.wait().is_ok();
        if terminal {
            let _ = self.remove_run_root();
        }
    }
}

/// Privacy-filtered terminal evidence for one OpenDesk attempt.
#[derive(Clone, Debug, PartialEq)]
pub struct OpenDeskOutcome {
    events: Vec<ExtensionEvent>,
    status: TerminalStatus,
    exit_code: Option<i32>,
    stderr_truncated: bool,
}

impl OpenDeskOutcome {
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

fn read_export(path: &Path) -> Result<Vec<u8>, AdapterError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_EXPORT_BYTES {
        return Err(AdapterError::InvalidEvent);
    }
    let file = fs::File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(AdapterError::InvalidEvent);
    }
    let mut bytes = Vec::new();
    file.take(MAX_EXPORT_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.is_empty() || u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_EXPORT_BYTES {
        return Err(AdapterError::InvalidEvent);
    }
    Ok(bytes)
}

fn map_export(
    export: Option<&[u8]>,
    session_id: &Id,
    attempt_id: &Id,
    process_status: TerminalStatus,
) -> Result<(Vec<ExtensionEvent>, TerminalStatus), AdapterError> {
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
    if process_status == TerminalStatus::Cancelled {
        return Ok((events, TerminalStatus::Cancelled));
    }
    if process_status == TerminalStatus::Failed {
        push_failure(&mut events, &mut sequence, session_id, attempt_id)?;
        return Ok((events, TerminalStatus::Failed));
    }

    let value = serde_json::from_slice::<UniqueJson>(export.ok_or(AdapterError::InvalidEvent)?)
        .map_err(|_| AdapterError::InvalidEvent)?
        .0;
    let root = value.as_object().ok_or(AdapterError::InvalidEvent)?;
    let task_id = required_string(root, "task_id")?;
    if !safe_label(task_id, 256) || required_string(root, "task_status")? != "stopped" {
        return Err(AdapterError::InvalidEvent);
    }
    let stopped_by_user = root
        .get("stop_requested_by_user")
        .and_then(Value::as_bool)
        .ok_or(AdapterError::InvalidEvent)?;
    let benchmark = root
        .get("benchmark_usage")
        .and_then(Value::as_object)
        .ok_or(AdapterError::InvalidEvent)?;
    if benchmark.get("version").and_then(Value::as_u64) != Some(1)
        || required_string(benchmark, "task_id")? != task_id
    {
        return Err(AdapterError::InvalidEvent);
    }
    let summary = benchmark
        .get("task_summary")
        .and_then(Value::as_object)
        .ok_or(AdapterError::InvalidEvent)?;
    if required_string(summary, "task_status")? != "stopped"
        || summary
            .get("stop_requested_by_user")
            .and_then(Value::as_bool)
            != Some(stopped_by_user)
        || summary
            .get("request_count")
            .and_then(Value::as_u64)
            .is_none_or(|count| count == 0)
    {
        return Err(AdapterError::InvalidEvent);
    }
    let task_success = summary
        .get("task_success")
        .and_then(Value::as_bool)
        .ok_or(AdapterError::InvalidEvent)?;

    let messages = root
        .get("chat_context")
        .and_then(|context| context.get("messages"))
        .and_then(Value::as_array)
        .ok_or(AdapterError::InvalidEvent)?;
    if messages.len() > MAX_UPSTREAM_ITEMS {
        return Err(AdapterError::InvalidEvent);
    }
    let mut saw_assistant = false;
    let mut upstream_failed = false;
    let mut tool_count = 0_usize;
    let mut seen_tool_ids = BTreeSet::new();
    for message in messages {
        let message = message.as_object().ok_or(AdapterError::InvalidEvent)?;
        if required_string(message, "role")? != "assistant" {
            continue;
        }
        if !saw_assistant {
            push_event(
                &mut events,
                &mut sequence,
                session_id,
                attempt_id,
                Event::FirstResponse,
            )?;
            saw_assistant = true;
        }
        let content = message
            .get("content")
            .and_then(Value::as_array)
            .ok_or(AdapterError::InvalidEvent)?;
        if content.len() > MAX_UPSTREAM_ITEMS {
            return Err(AdapterError::InvalidEvent);
        }
        for part in content {
            let part = part.as_object().ok_or(AdapterError::InvalidEvent)?;
            match required_string(part, "type")? {
                "text" | "reasoning" => {
                    required_string(part, "text")?;
                }
                "error" => upstream_failed = true,
                "tool_call" => {
                    let calls = part
                        .get("toolcalls")
                        .and_then(Value::as_array)
                        .ok_or(AdapterError::InvalidEvent)?;
                    for call in calls {
                        tool_count = tool_count
                            .checked_add(1)
                            .ok_or(AdapterError::InvalidEvent)?;
                        if tool_count > MAX_UPSTREAM_ITEMS {
                            return Err(AdapterError::InvalidEvent);
                        }
                        let call = call.as_object().ok_or(AdapterError::InvalidEvent)?;
                        let id = required_string(call, "id")?;
                        let name = required_string(call, "tool_name")?;
                        if !safe_label(id, 256)
                            || !safe_label(name, 256)
                            || !seen_tool_ids.insert(id.to_owned())
                        {
                            return Err(AdapterError::InvalidEvent);
                        }
                        let success = match required_string(call, "status")? {
                            "success" => true,
                            "error" => false,
                            _ => return Err(AdapterError::InvalidEvent),
                        };
                        let tool_call_id = Id(id.into());
                        push_event(
                            &mut events,
                            &mut sequence,
                            session_id,
                            attempt_id,
                            Event::ToolStarted {
                                tool_call_id: tool_call_id.clone(),
                                name: name.into(),
                            },
                        )?;
                        push_event(
                            &mut events,
                            &mut sequence,
                            session_id,
                            attempt_id,
                            Event::ToolFinished {
                                tool_call_id,
                                success,
                            },
                        )?;
                    }
                }
                _ => return Err(AdapterError::InvalidEvent),
            }
        }
    }
    if !saw_assistant {
        return Err(AdapterError::InvalidEvent);
    }
    let input_tokens = optional_u64(summary.get("prompt_tokens"))?;
    let output_tokens = optional_u64(summary.get("completion_tokens"))?;
    if input_tokens.is_some() || output_tokens.is_some() {
        push_event(
            &mut events,
            &mut sequence,
            session_id,
            attempt_id,
            Event::Usage(Usage {
                input_tokens,
                output_tokens,
                cost_micros: None,
                currency: None,
            }),
        )?;
    }
    let status = if task_success && !stopped_by_user && !upstream_failed {
        push_event(
            &mut events,
            &mut sequence,
            session_id,
            attempt_id,
            Event::Completed,
        )?;
        TerminalStatus::Completed
    } else {
        push_failure(&mut events, &mut sequence, session_id, attempt_id)?;
        TerminalStatus::Failed
    };
    Ok((events, status))
}

fn safe_label(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && value.bytes().all(|byte| !byte.is_ascii_control())
}

fn push_failure(
    events: &mut Vec<ExtensionEvent>,
    sequence: &mut u64,
    session_id: &Id,
    attempt_id: &Id,
) -> Result<(), AdapterError> {
    push_event(
        events,
        sequence,
        session_id,
        attempt_id,
        Event::Failed(RpcError {
            code: -32_100,
            message: "OpenDesk attempt failed; raw diagnostics are not retained".into(),
            data: None,
        }),
    )
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
    use serde_json::json;
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "asb-opendesk-{name}-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn config(endpoint: &str) -> OpenDeskConfig {
        OpenDeskConfig::new(
            "/bin/true",
            "/bin/true",
            "/tmp/asb-workspace",
            "/tmp/asb-state",
            Url::parse(endpoint).unwrap(),
            "fixture-model",
            OpenDeskArtifact::LinuxX86_64V0_3_5,
        )
        .unwrap()
    }

    fn export(success: bool) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "task_id": "task_fixture",
            "task_status": "stopped",
            "stop_requested_by_user": false,
            "benchmark_usage": {
                "version": 1,
                "task_id": "task_fixture",
                "task_summary": {
                    "task_status": "stopped",
                    "task_success": success,
                    "stop_requested_by_user": false,
                    "request_count": 1,
                    "prompt_tokens": 7,
                    "completion_tokens": 2
                }
            },
            "chat_context": {"messages": [
                {"role": "user", "content": [{"type": "text", "text": "discarded input"}]},
                {"role": "assistant", "content": [
                    {"type": "reasoning", "text": "discarded reasoning"},
                    {"type": "tool_call", "toolcalls": [{
                        "id": "call-1", "tool_name": "write", "status": "success",
                        "args_string": "discarded", "result": "discarded"
                    }]},
                    {"type": "text", "text": "discarded response"}
                ]}
            ]}
        }))
        .unwrap()
    }

    #[test]
    fn configuration_and_manifest_are_bounded() {
        assert!(matches!(
            OpenDeskConfig::new(
                "relative",
                "/work",
                "/state",
                "/bin/true",
                Url::parse("https://example.invalid/v1").unwrap(),
                "model",
                OpenDeskArtifact::LinuxX86_64V0_3_5,
            ),
            Err(AdapterError::RelativePath("binary"))
        ));
        for endpoint in [
            "ftp://example.invalid/v1",
            "http://example.invalid/v1",
            "https://user@example.invalid/v1",
            "https://example.invalid/v1?secret=value",
            "https://example.invalid/v1#fragment",
            "https://opendesk.matrix.openharmony.cn/v1",
            "https://matrix.openharmony.cn/v1",
            "https://openharmony.cn/v1",
            "https://OPENHARMONY.CN./v1",
        ] {
            assert!(matches!(
                OpenDeskConfig::new(
                    "/bin/true",
                    "/bin/true",
                    "/work",
                    "/state",
                    Url::parse(endpoint).unwrap(),
                    "model",
                    OpenDeskArtifact::LinuxX86_64V0_3_5,
                ),
                Err(AdapterError::InvalidEndpoint)
            ));
        }
        for model in [
            "",
            "bad/model",
            "bad model",
            &"x".repeat(MAX_MODEL_BYTES + 1),
        ] {
            assert!(matches!(
                OpenDeskConfig::new(
                    "/bin/true",
                    "/bin/true",
                    "/work",
                    "/state",
                    Url::parse("https://example.invalid/v1").unwrap(),
                    model,
                    OpenDeskArtifact::LinuxX86_64V0_3_5,
                ),
                Err(AdapterError::InvalidModel)
            ));
        }
        let manifest = config("http://127.0.0.1:1/v1").manifest();
        assert_eq!(manifest.extension_id.0, "agent.opendesk");
        assert_eq!(
            manifest.capabilities,
            BTreeSet::from([Capability::Cancellation])
        );
        assert_eq!(manifest.executable_sha256, TESTED_LINUX_X86_64_SHA256);
        assert_eq!(NPM_TARBALL_SHA256.len(), 64);
    }

    #[test]
    fn export_maps_only_causal_redacted_evidence() {
        let (events, status) = map_export(
            Some(&export(true)),
            &Id("session".into()),
            &Id("attempt".into()),
            TerminalStatus::Completed,
        )
        .unwrap();
        assert_eq!(status, TerminalStatus::Completed);
        assert_eq!(events.len(), 7);
        assert!(events.iter().enumerate().all(|(index, event)| {
            event.sequence == u64::try_from(index).unwrap()
                && event.session_id.0 == "session"
                && event.attempt_id.0 == "attempt"
        }));
        assert!(matches!(events[2].event, Event::FirstResponse));
        assert!(
            matches!(&events[3].event, Event::ToolStarted { tool_call_id, name } if tool_call_id.0 == "call-1" && name == "write")
        );
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
        let encoded = serde_json::to_string(&events).unwrap();
        for private in [
            "discarded input",
            "discarded reasoning",
            "discarded response",
        ] {
            assert!(!encoded.contains(private));
        }
    }

    #[test]
    fn failure_cancellation_and_malformed_exports_fail_closed() {
        let (failed, status) = map_export(
            Some(&export(false)),
            &Id("s".into()),
            &Id("a".into()),
            TerminalStatus::Completed,
        )
        .unwrap();
        assert_eq!(status, TerminalStatus::Failed);
        assert!(matches!(failed.last().unwrap().event, Event::Failed(_)));
        let (cancelled, status) = map_export(
            None,
            &Id("s".into()),
            &Id("a".into()),
            TerminalStatus::Cancelled,
        )
        .unwrap();
        assert_eq!(status, TerminalStatus::Cancelled);
        assert_eq!(cancelled.len(), 2);
        let (process_failed, status) = map_export(
            None,
            &Id("s".into()),
            &Id("a".into()),
            TerminalStatus::Failed,
        )
        .unwrap();
        assert_eq!(status, TerminalStatus::Failed);
        assert!(matches!(
            process_failed.last().unwrap().event,
            Event::Failed(_)
        ));

        for malformed in [
            b"not-json".as_slice(),
            br#"{"task_id":"x","task_id":"y"}"#,
            br#"{"task_id":"x"}"#,
        ] {
            assert!(matches!(
                map_export(
                    Some(malformed),
                    &Id("s".into()),
                    &Id("a".into()),
                    TerminalStatus::Completed
                ),
                Err(AdapterError::InvalidEvent)
            ));
        }
        let mut unknown = serde_json::from_slice::<Value>(&export(true)).unwrap();
        unknown["chat_context"]["messages"][1]["content"][0]["type"] = json!("new-part");
        assert!(matches!(
            map_export(
                Some(&serde_json::to_vec(&unknown).unwrap()),
                &Id("s".into()),
                &Id("a".into()),
                TerminalStatus::Completed,
            ),
            Err(AdapterError::InvalidEvent)
        ));
    }

    #[test]
    fn export_file_bounds_and_cleanup_failures_are_explicit() {
        let scratch = Scratch::new("bounds");
        let path = scratch.0.join("result.json");
        fs::write(&path, b"").unwrap();
        assert!(matches!(
            read_export(&path),
            Err(AdapterError::InvalidEvent)
        ));
        fs::write(
            &path,
            vec![b'x'; usize::try_from(MAX_EXPORT_BYTES + 1).unwrap()],
        )
        .unwrap();
        assert!(matches!(
            read_export(&path),
            Err(AdapterError::InvalidEvent)
        ));
        let target = scratch.0.join("target.json");
        let link = scratch.0.join("linked.json");
        fs::write(&target, export(true)).unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(matches!(
            read_export(&link),
            Err(AdapterError::InvalidEvent)
        ));

        let file_root = scratch.0.join("not-directory");
        fs::write(&file_root, b"x").unwrap();
        let mut root = Some(file_root.clone());
        assert!(matches!(
            remove_run_tree(&mut root),
            Err(AdapterError::Io(_))
        ));
        assert_eq!(root.as_deref(), Some(file_root.as_path()));
        fs::remove_file(file_root).unwrap();
        remove_run_tree(&mut root).unwrap();
        assert!(root.is_none());
    }

    fn executable_adapter(script: &str) -> (Scratch, OpenDeskConfig) {
        let scratch = Scratch::new("process");
        let binary = scratch.0.join("opendesk");
        let node = scratch.0.join("node");
        fs::write(&binary, script).unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(
            &node,
            "#!/bin/sh\n[ \"$1\" = --use-env-proxy ] || exit 97\n[ \"$NODE_USE_ENV_PROXY\" = 1 ] || exit 98\nshift\nexec /bin/sh \"$@\"\n",
        )
        .unwrap();
        fs::set_permissions(&node, fs::Permissions::from_mode(0o700)).unwrap();
        let mut adapter = OpenDeskConfig::new(
            &binary,
            &node,
            scratch.0.join("work"),
            scratch.0.join("state"),
            Url::parse("http://127.0.0.1:1/v1").unwrap(),
            "fixture-model",
            OpenDeskArtifact::LinuxX86_64V0_3_5,
        )
        .unwrap();
        adapter.verification_digest_override = Some(digest_file(&binary).unwrap());
        adapter.runtime_digest_override = Some(digest_file(&node).unwrap());
        (scratch, adapter)
    }

    #[test]
    fn empty_pinned_runtime_is_a_runtime_mismatch() {
        let scratch = Scratch::new("empty-runtime");
        let binary = scratch.0.join("opendesk");
        let node = scratch.0.join("node");
        fs::write(&binary, "wrapper").unwrap();
        fs::write(&node, "").unwrap();
        let mut adapter = OpenDeskConfig::new(
            &binary,
            &node,
            scratch.0.join("work"),
            scratch.0.join("state"),
            Url::parse("http://127.0.0.1:1/v1").unwrap(),
            "fixture-model",
            OpenDeskArtifact::LinuxX86_64V0_3_5,
        )
        .unwrap();
        adapter.verification_digest_override = Some(digest_file(&binary).unwrap());
        assert!(matches!(
            adapter.verify_executable(),
            Err(AdapterError::RuntimeMismatch)
        ));
    }

    fn limits() -> ProcessLimits {
        ProcessLimits::new(
            4096,
            4096,
            Duration::from_secs(5),
            Duration::from_millis(50),
            Duration::from_millis(2),
        )
        .unwrap()
    }

    #[test]
    fn process_boundary_discards_raw_state_and_contains_telemetry() {
        let script = r#"#!/bin/sh
export_path=
workspace=
prompt=
while [ "$#" -gt 0 ]; do
  case "$1" in
    --export-json) export_path=$2; shift 2 ;;
    --workspace) workspace=$2; shift 2 ;;
    --command) prompt=$2; shift 2 ;;
    *) shift ;;
  esac
done
printf '%s\n%s\n' "$HTTPS_PROXY" "$NO_PROXY" > "$workspace/network-policy"
printf '%s' '{"task_id":"task_fixture","task_status":"stopped","stop_requested_by_user":false,"benchmark_usage":{"version":1,"task_id":"task_fixture","task_summary":{"task_status":"stopped","task_success":true,"stop_requested_by_user":false,"request_count":1,"prompt_tokens":7,"completion_tokens":2}},"chat_context":{"messages":[{"role":"assistant","content":[{"type":"text","text":"raw response"}]}]}}' > "$export_path"
printf '%s' "$prompt"
"#;
        let (_scratch, adapter) = executable_adapter(script);
        let mut running = adapter
            .start(Id("s".into()), Id("a".into()), "synthetic-prompt", limits())
            .unwrap();
        let outcome = running.wait().unwrap();
        assert_eq!(outcome.status(), TerminalStatus::Completed);
        assert!(fs::read_dir(&adapter.state_root).unwrap().next().is_none());
        let policy = fs::read_to_string(adapter.workspace.join("network-policy")).unwrap();
        assert_eq!(policy, "http://127.0.0.1:9\n127.0.0.1\n");
        assert!(
            !serde_json::to_string(outcome.events())
                .unwrap()
                .contains("raw response")
        );
    }

    #[test]
    fn process_boundary_cancels_and_removes_state() {
        let script = "#!/bin/sh\ntrap 'exit 0' TERM\nsleep 60\n";
        let (_scratch, adapter) = executable_adapter(script);
        let mut running = adapter
            .start(Id("s".into()), Id("a".into()), "synthetic", limits())
            .unwrap();
        std::thread::sleep(Duration::from_millis(20));
        running.cancel().unwrap();
        let outcome = running.wait().unwrap();
        assert_eq!(outcome.status(), TerminalStatus::Cancelled);
        assert!(fs::read_dir(&adapter.state_root).unwrap().next().is_none());
    }

    #[test]
    fn route_run_roots_are_stable_opaque_and_distinct() {
        let (_scratch, adapter) = executable_adapter("#!/bin/sh\nexit 0\n");
        let session = Id("public-session".into());
        let attempt = Id("public-attempt".into());
        let first = adapter.route_run_root(&session, &attempt);
        assert_eq!(first, adapter.route_run_root(&session, &attempt));
        assert_ne!(
            first,
            adapter.route_run_root(&session, &Id("other-attempt".into()))
        );
        let name = first.file_name().unwrap().to_str().unwrap();
        assert_eq!(name.len(), "attempt-".len() + 64);
        assert!(
            name.strip_prefix("attempt-")
                .unwrap()
                .bytes()
                .all(|byte| { byte.is_ascii_digit() || matches!(byte, b'a'..=b'f') })
        );
        assert!(!name.contains(&session.0));
        assert!(!name.contains(&attempt.0));
    }

    #[test]
    fn duplicate_and_stale_route_ownership_fail_closed_then_cleanup_allows_reuse() {
        let script = "#!/bin/sh\ntrap 'exit 0' TERM\nsleep 60\n";
        let (_scratch, adapter) = executable_adapter(script);
        let session = Id("same-session".into());
        let attempt = Id("same-attempt".into());
        let stale = adapter.route_run_root(&session, &attempt);
        fs::create_dir_all(&stale).unwrap();
        assert!(matches!(
            adapter.start(session.clone(), attempt.clone(), "synthetic", limits()),
            Err(AdapterError::Io(error)) if error.kind() == io::ErrorKind::AlreadyExists
        ));
        fs::remove_dir_all(&stale).unwrap();

        let mut running = adapter
            .start(session.clone(), attempt.clone(), "synthetic", limits())
            .unwrap();
        assert!(matches!(
            adapter.start(session.clone(), attempt.clone(), "synthetic", limits()),
            Err(AdapterError::Io(error)) if error.kind() == io::ErrorKind::AlreadyExists
        ));
        running.cancel().unwrap();
        assert_eq!(running.wait().unwrap().status(), TerminalStatus::Cancelled);
        assert!(!stale.exists());

        let mut reused = adapter
            .start(session, attempt, "synthetic", limits())
            .unwrap();
        reused.cancel().unwrap();
        assert_eq!(reused.wait().unwrap().status(), TerminalStatus::Cancelled);
    }

    #[test]
    fn prompt_and_required_usage_are_fail_closed() {
        let (_scratch, adapter) = executable_adapter("#!/bin/sh\nexit 0\n");
        assert!(matches!(
            adapter.start(Id("s".into()), Id("a".into()), "bad\0prompt", limits()),
            Err(AdapterError::InvalidPrompt)
        ));
        let mut missing_count = serde_json::from_slice::<Value>(&export(true)).unwrap();
        missing_count["benchmark_usage"]["task_summary"]
            .as_object_mut()
            .unwrap()
            .remove("request_count");
        assert!(matches!(
            map_export(
                Some(&serde_json::to_vec(&missing_count).unwrap()),
                &Id("s".into()),
                &Id("a".into()),
                TerminalStatus::Completed,
            ),
            Err(AdapterError::InvalidEvent)
        ));
    }
}
