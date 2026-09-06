// SPDX-License-Identifier: MIT
//! Bounded adapter for the OpenCode structured command-line interface.

use asb_protocol::{
    Capability, Event, ExtensionEvent, ExtensionKind, ExtensionManifest, Id, PROTOCOL_V1, RpcError,
    TerminalStatus, Usage,
};
use asb_runtime::{ProcessError, ProcessLimits, RunningProcess, Termination};
use serde_json::{Value, json};
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

/// OpenCode release whose JSON event dialect this module implements.
pub const SUPPORTED_VERSION: &str = "1.18.29";
/// Immutable upstream Git revision inspected for this adapter.
pub const UPSTREAM_REVISION: &str = "16747470f976aca3d362ad730bcd3fe82ecc2c9a";
/// SHA-256 of the natively exercised Linux x86_64 OpenCode 1.18.29 executable.
pub const TESTED_LINUX_X86_64_SHA256: &str =
    "ca6c0e1f42be3120595bf6848937e7586ec862c87fa7aa111e89c7cc6e9a4650";
const MAX_EXECUTABLE_BYTES: u64 = 1024 * 1024 * 1024;
/// Largest accepted prompt, in UTF-8 bytes.
pub const MAX_PROMPT_BYTES: usize = 4 * 1024 * 1024;
/// Largest individual OpenCode JSON output line.
pub const MAX_EVENT_LINE_BYTES: usize = 16 * 1024 * 1024;
/// Largest number of upstream JSON events accepted per attempt.
pub const MAX_UPSTREAM_EVENTS: usize = 65_536;
const MAX_ENDPOINT_BYTES: usize = 4 * 1024;
const MAX_MODEL_BYTES: usize = 256;

/// Content-pinned OpenCode artifact understood by this adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpenCodeArtifact {
    /// OpenCode 1.18.29 native Linux x86_64 release executable.
    LinuxX86_64V1_18_29,
}

impl OpenCodeArtifact {
    const fn digest(self) -> &'static str {
        match self {
            Self::LinuxX86_64V1_18_29 => TESTED_LINUX_X86_64_SHA256,
        }
    }
}

/// Validated immutable inputs for one OpenCode installation.
#[derive(Clone, Debug)]
pub struct OpenCodeConfig {
    binary: PathBuf,
    workspace: PathBuf,
    state_root: PathBuf,
    endpoint: Url,
    model: String,
    artifact: OpenCodeArtifact,
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
    /// Process execution failed.
    Process(ProcessError),
    /// Structured output was truncated.
    TruncatedOutput,
    /// OpenCode emitted malformed, inconsistent, or unsupported structured output.
    InvalidEvent,
}

impl fmt::Display for AdapterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RelativePath(label) => write!(formatter, "{label} must be absolute"),
            Self::InvalidEndpoint => formatter.write_str("invalid provider endpoint"),
            Self::InvalidModel => formatter.write_str("invalid provider/model identifier"),
            Self::PromptTooLarge => formatter.write_str("OpenCode prompt exceeds byte limit"),
            Self::Io(error) => write!(formatter, "adapter I/O failed: {error}"),
            Self::ExecutableMismatch => formatter.write_str("OpenCode executable pin mismatch"),
            Self::Process(error) => write!(formatter, "OpenCode process failed: {error}"),
            Self::TruncatedOutput => {
                formatter.write_str("OpenCode structured output was truncated")
            }
            Self::InvalidEvent => formatter.write_str("invalid OpenCode structured event"),
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

impl OpenCodeConfig {
    /// Validate paths, endpoint, model, and select a content-pinned artifact.
    pub fn new(
        binary: impl Into<PathBuf>,
        workspace: impl Into<PathBuf>,
        state_root: impl Into<PathBuf>,
        endpoint: Url,
        model: impl Into<String>,
        artifact: OpenCodeArtifact,
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
        if model.len() > MAX_MODEL_BYTES
            || model.split_once('/').is_none_or(|(provider, name)| {
                provider.is_empty()
                    || name.is_empty()
                    || name.contains('/')
                    || !provider.bytes().all(model_component_byte)
                    || !name.bytes().all(model_component_byte)
            })
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
            extension_id: Id("agent.opencode".into()),
            kind: ExtensionKind::Agent,
            implementation_version: format!("opencode-{SUPPORTED_VERSION}+asb-0.1.0"),
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
    ) -> Result<RunningOpenCode, AdapterError> {
        if prompt.len() > MAX_PROMPT_BYTES {
            return Err(AdapterError::PromptTooLarge);
        }
        self.verify_executable()?;
        fs::create_dir_all(&self.workspace)?;
        let run_root = self.unique_run_root()?;
        fs::create_dir_all(&self.state_root)?;
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

        let result = self.spawn_process(&run_root, &prompt_path, limits);
        match result {
            Ok(process) => Ok(RunningOpenCode {
                process,
                session_id,
                attempt_id,
                prompt_path: Some(prompt_path),
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
        prompt_path: &Path,
        limits: ProcessLimits,
    ) -> Result<RunningProcess, AdapterError> {
        let (provider, model_name) = self.model.split_once('/').expect("validated model");
        let mut models = serde_json::Map::new();
        models.insert(model_name.to_owned(), json!({"name": "ASB pinned model"}));
        let mut providers = serde_json::Map::new();
        providers.insert(
            provider.to_owned(),
            json!({
                "npm": "@ai-sdk/openai-compatible",
                "name": "ASB explicit endpoint",
                "options": {
                    "baseURL": self.endpoint.as_str(),
                    "apiKey": "asb-credential-free"
                },
                "models": models
            }),
        );
        let config = json!({
            "$schema": "https://opencode.ai/config.json",
            "provider": providers,
            "model": self.model,
            "share": "disabled",
            "autoupdate": false,
            "permission": {
                "edit": "allow",
                "external_directory": "deny",
                "question": "deny",
                "plan_enter": "deny",
                "plan_exit": "deny"
            }
        });
        let mut command = Command::new("/bin/sh");
        command
            .arg("-c")
            .arg(r#"exec "$1" run --format json --pure --dir "$2" --model "$3" < "$4""#)
            .arg("asb-opencode")
            .arg(&self.binary)
            .arg(&self.workspace)
            .arg(&self.model)
            .arg(prompt_path)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", run_root.join("home"))
            .env("XDG_CONFIG_HOME", run_root.join("config"))
            .env("XDG_DATA_HOME", run_root.join("data"))
            .env("XDG_CACHE_HOME", run_root.join("cache"))
            .env("OPENCODE_DISABLE_PROJECT_CONFIG", "1")
            .env("OPENCODE_CONFIG_CONTENT", config.to_string());
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

/// A cancellable OpenCode process whose output has not yet been collected.
pub struct RunningOpenCode {
    process: RunningProcess,
    session_id: Id,
    attempt_id: Id,
    prompt_path: Option<PathBuf>,
    run_root: Option<PathBuf>,
}

impl RunningOpenCode {
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
    pub fn wait(&mut self) -> Result<OpenCodeOutcome, AdapterError> {
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
        Ok(OpenCodeOutcome {
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

impl Drop for RunningOpenCode {
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

/// Privacy-filtered terminal evidence for one OpenCode attempt.
#[derive(Clone, Debug, PartialEq)]
pub struct OpenCodeOutcome {
    events: Vec<ExtensionEvent>,
    status: TerminalStatus,
    exit_code: Option<i32>,
    stderr_truncated: bool,
}

impl OpenCodeOutcome {
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
    let mut upstream_session = None;
    let mut upstream_failed = false;
    let mut in_step = false;
    let mut finished_steps = 0_usize;
    let mut seen_tool_ids = BTreeSet::new();
    let mut upstream_events = 0_usize;
    for line in text.lines().filter(|line| !line.is_empty()) {
        upstream_events = upstream_events
            .checked_add(1)
            .ok_or(AdapterError::InvalidEvent)?;
        if upstream_events > MAX_UPSTREAM_EVENTS || line.len() > MAX_EVENT_LINE_BYTES {
            return Err(AdapterError::InvalidEvent);
        }
        let value: Value = serde_json::from_str(line).map_err(|_| AdapterError::InvalidEvent)?;
        let object = value.as_object().ok_or(AdapterError::InvalidEvent)?;
        let kind = object
            .get("type")
            .and_then(Value::as_str)
            .ok_or(AdapterError::InvalidEvent)?;
        let current_session = object
            .get("sessionID")
            .and_then(Value::as_str)
            .ok_or(AdapterError::InvalidEvent)?;
        match upstream_session.as_deref() {
            None => upstream_session = Some(current_session.to_owned()),
            Some(expected) if expected != current_session => {
                return Err(AdapterError::InvalidEvent);
            }
            Some(_) => {}
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
        match kind {
            "step_start" if !in_step && !upstream_failed => in_step = true,
            "text" | "reasoning" if in_step && !upstream_failed => {}
            "step_finish" => {
                if !in_step || upstream_failed {
                    return Err(AdapterError::InvalidEvent);
                }
                if let Some(usage) = parse_usage(&value)? {
                    push_event(
                        &mut events,
                        &mut sequence,
                        session_id,
                        attempt_id,
                        Event::Usage(usage),
                    )?;
                }
                in_step = false;
                finished_steps = finished_steps
                    .checked_add(1)
                    .ok_or(AdapterError::InvalidEvent)?;
            }
            "tool_use" => {
                if !in_step || upstream_failed {
                    return Err(AdapterError::InvalidEvent);
                }
                let part = object
                    .get("part")
                    .and_then(Value::as_object)
                    .ok_or(AdapterError::InvalidEvent)?;
                let id = required_string(part, "id")?;
                let name = required_string(part, "tool")?;
                let state = part
                    .get("state")
                    .and_then(Value::as_object)
                    .ok_or(AdapterError::InvalidEvent)?;
                let state_status = required_string(state, "status")?;
                let success = match state_status {
                    "completed" => true,
                    "error" => false,
                    _ => return Err(AdapterError::InvalidEvent),
                };
                if !seen_tool_ids.insert(id.to_owned()) {
                    return Err(AdapterError::InvalidEvent);
                }
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
            "error" if !upstream_failed => upstream_failed = true,
            _ => return Err(AdapterError::InvalidEvent),
        }
    }
    match status {
        TerminalStatus::Completed
            if upstream_failed || !first || in_step || finished_steps == 0 =>
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
                message: "OpenCode attempt failed; raw diagnostics are not retained".into(),
                data: None,
            }),
        )?,
        TerminalStatus::Cancelled => {}
    }
    Ok(events)
}

fn parse_usage(value: &Value) -> Result<Option<Usage>, AdapterError> {
    let tokens = value
        .get("part")
        .and_then(|part| part.get("tokens"))
        .and_then(Value::as_object);
    let Some(tokens) = tokens else {
        return Ok(None);
    };
    let input_tokens = optional_u64(tokens.get("input"))?;
    let output_tokens = optional_u64(tokens.get("output"))?;
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
    use std::time::Duration;

    fn config(endpoint: &str) -> OpenCodeConfig {
        OpenCodeConfig::new(
            "/bin/true",
            "/tmp/asb-workspace",
            "/tmp/asb-state",
            Url::parse(endpoint).unwrap(),
            "asb/model",
            OpenCodeArtifact::LinuxX86_64V1_18_29,
        )
        .unwrap()
    }

    #[test]
    fn configuration_rejects_ambient_and_ambiguous_inputs() {
        assert!(matches!(
            OpenCodeConfig::new(
                "relative",
                "/work",
                "/state",
                Url::parse("https://example.invalid/v1").unwrap(),
                "asb/model",
                OpenCodeArtifact::LinuxX86_64V1_18_29
            ),
            Err(AdapterError::RelativePath("binary"))
        ));
        assert!(matches!(
            OpenCodeConfig::new(
                "/bin/true",
                "/work",
                "/state",
                Url::parse("http://example.invalid/v1").unwrap(),
                "asb/model",
                OpenCodeArtifact::LinuxX86_64V1_18_29
            ),
            Err(AdapterError::InvalidEndpoint)
        ));
        assert!(matches!(
            OpenCodeConfig::new(
                "/bin/true",
                "/work",
                "/state",
                Url::parse("https://user@example.invalid/v1").unwrap(),
                "asb/model",
                OpenCodeArtifact::LinuxX86_64V1_18_29
            ),
            Err(AdapterError::InvalidEndpoint)
        ));
        assert!(matches!(
            OpenCodeConfig::new(
                "/bin/true",
                "/work",
                "/state",
                Url::parse("https://example.invalid/v1").unwrap(),
                "model",
                OpenCodeArtifact::LinuxX86_64V1_18_29
            ),
            Err(AdapterError::InvalidModel)
        ));
        for endpoint in [
            "ftp://example.invalid/v1",
            "mailto:agent@example.invalid",
            "https://user:password@example.invalid/v1",
            "https://example.invalid/v1?query=secret",
            "https://example.invalid/v1#fragment",
        ] {
            assert!(matches!(
                OpenCodeConfig::new(
                    "/bin/true",
                    "/work",
                    "/state",
                    Url::parse(endpoint).unwrap(),
                    "asb/model",
                    OpenCodeArtifact::LinuxX86_64V1_18_29
                ),
                Err(AdapterError::InvalidEndpoint)
            ));
        }
        for model in [
            "asb/",
            "/model",
            "asb/mo del",
            "asb/model!",
            &format!("asb/{}", "x".repeat(MAX_MODEL_BYTES)),
        ] {
            assert!(matches!(
                OpenCodeConfig::new(
                    "/bin/true",
                    "/work",
                    "/state",
                    Url::parse("https://example.invalid/v1").unwrap(),
                    model,
                    OpenCodeArtifact::LinuxX86_64V1_18_29
                ),
                Err(AdapterError::InvalidModel)
            ));
        }
        for endpoint in [
            "http://localhost:1/v1",
            "http://[::1]:1/v1",
            "https://example.invalid/v1",
        ] {
            OpenCodeConfig::new(
                "/bin/true",
                "/work",
                "/state",
                Url::parse(endpoint).unwrap(),
                "provider_1/model-name.v1",
                OpenCodeArtifact::LinuxX86_64V1_18_29,
            )
            .unwrap();
        }
        assert!(matches!(
            OpenCodeConfig::new(
                "/bin/true",
                "/work",
                "/state",
                Url::parse("https://example.invalid/v1").unwrap(),
                "asb/model/extra",
                OpenCodeArtifact::LinuxX86_64V1_18_29
            ),
            Err(AdapterError::InvalidModel)
        ));
    }

    #[test]
    fn manifest_claims_only_cancellation() {
        let manifest = config("http://127.0.0.1:1/v1").manifest();
        assert_eq!(
            manifest.capabilities,
            BTreeSet::from([Capability::Cancellation])
        );
        assert_eq!(manifest.executable_sha256, TESTED_LINUX_X86_64_SHA256);
    }

    #[test]
    fn executable_digest_mismatch_fails_before_spawn() {
        assert!(matches!(
            config("http://127.0.0.1:1/v1").verify_executable(),
            Err(AdapterError::ExecutableMismatch)
        ));
    }

    #[test]
    fn public_errors_and_outcome_accessors_are_stable_and_redacted() {
        let io_error = AdapterError::from(io::Error::other("bounded detail"));
        let process_error = AdapterError::from(ProcessError::Spawn(io::Error::other(
            "bounded process detail",
        )));
        for (error, expected) in [
            (
                AdapterError::RelativePath("workspace"),
                "workspace must be absolute",
            ),
            (AdapterError::InvalidEndpoint, "invalid provider endpoint"),
            (
                AdapterError::InvalidModel,
                "invalid provider/model identifier",
            ),
            (
                AdapterError::PromptTooLarge,
                "OpenCode prompt exceeds byte limit",
            ),
            (io_error, "adapter I/O failed: bounded detail"),
            (
                AdapterError::ExecutableMismatch,
                "OpenCode executable pin mismatch",
            ),
            (
                process_error,
                "OpenCode process failed: failed to spawn process: bounded process detail",
            ),
            (
                AdapterError::TruncatedOutput,
                "OpenCode structured output was truncated",
            ),
            (
                AdapterError::InvalidEvent,
                "invalid OpenCode structured event",
            ),
        ] {
            assert_eq!(error.to_string(), expected);
        }

        let outcome = OpenCodeOutcome {
            events: Vec::new(),
            status: TerminalStatus::Failed,
            exit_code: Some(17),
            stderr_truncated: true,
        };
        assert!(outcome.events().is_empty());
        assert_eq!(outcome.status(), TerminalStatus::Failed);
        assert_eq!(outcome.exit_code(), Some(17));
        assert!(outcome.stderr_truncated());
    }

    #[test]
    fn empty_executable_and_unusable_state_root_fail_before_spawn() {
        let root = std::env::temp_dir().join(format!(
            "asb-opencode-preparation-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let binary = root.join("empty-opencode");
        fs::write(&binary, b"").unwrap();
        assert!(matches!(
            digest_file(&binary),
            Err(AdapterError::ExecutableMismatch)
        ));

        fs::write(&binary, b"#!/bin/sh\nexit 0\n").unwrap();
        let state_file = root.join("state-file");
        fs::write(&state_file, b"not a directory").unwrap();
        let mut adapter = OpenCodeConfig::new(
            &binary,
            root.join("work"),
            &state_file,
            Url::parse("http://127.0.0.1:1/v1").unwrap(),
            "asb/model",
            OpenCodeArtifact::LinuxX86_64V1_18_29,
        )
        .unwrap();
        adapter.verification_digest_override = Some(digest_file(&binary).unwrap());
        assert!(matches!(
            adapter.start(
                Id("s".into()),
                Id("a".into()),
                "prompt",
                ProcessLimits::default(),
            ),
            Err(AdapterError::Io(_))
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_state_cleanup_remains_retryable_and_absence_is_success() {
        let root = std::env::temp_dir().join(format!(
            "asb-opencode-cleanup-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::write(&root, b"not a directory").unwrap();
        let prompt = root.join("prompt.txt");
        let mut run_root = Some(root.clone());
        let mut prompt_path = Some(prompt.clone());
        assert!(matches!(
            remove_run_tree(&mut run_root, &mut prompt_path),
            Err(AdapterError::Io(_))
        ));
        assert_eq!(run_root.as_deref(), Some(root.as_path()));
        assert_eq!(prompt_path.as_deref(), Some(prompt.as_path()));

        fs::remove_file(&root).unwrap();
        fs::create_dir(&root).unwrap();
        fs::write(&prompt, b"private").unwrap();
        remove_run_tree(&mut run_root, &mut prompt_path).unwrap();
        assert!(run_root.is_none());
        assert!(prompt_path.is_none());
        assert!(!root.exists());

        let mut absent_root = Some(root);
        let mut absent_prompt = Some(prompt);
        remove_run_tree(&mut absent_root, &mut absent_prompt).unwrap();
        assert!(absent_root.is_none());
        assert!(absent_prompt.is_none());
    }

    #[test]
    fn oversized_prompt_fails_before_filesystem_or_process_effects() {
        assert!(matches!(
            config("http://127.0.0.1:1/v1").start(
                Id("s".into()),
                Id("a".into()),
                &"x".repeat(MAX_PROMPT_BYTES + 1),
                ProcessLimits::default(),
            ),
            Err(AdapterError::PromptTooLarge)
        ));
    }

    #[test]
    fn completion_maps_ordered_content_free_events() {
        let stdout = br#"{"type":"step_start","sessionID":"up","part":{"type":"step-start"}}
{"type":"text","sessionID":"up","part":{"text":"private"}}
{"type":"tool_use","sessionID":"up","part":{"id":"call-1","tool":"write","state":{"status":"completed","output":"private"}}}
{"type":"step_finish","sessionID":"up","part":{"tokens":{"input":7,"output":2},"cost":1}}
"#;
        let events = map_events(
            stdout,
            &Id("session".into()),
            &Id("attempt".into()),
            TerminalStatus::Completed,
        )
        .unwrap();
        assert_eq!(events.len(), 7);
        assert!(events.iter().enumerate().all(|(index, event)| {
            event.sequence == u64::try_from(index).unwrap()
                && event.session_id.0 == "session"
                && event.attempt_id.0 == "attempt"
        }));
        assert!(matches!(events[2].event, Event::FirstResponse));
        assert!(matches!(
            &events[3].event,
            Event::ToolStarted { tool_call_id, name }
                if tool_call_id.0 == "call-1" && name == "write"
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
        let serialized = serde_json::to_string(&events).unwrap();
        assert!(!serialized.contains("private"));
    }

    #[test]
    fn malformed_unknown_and_inconsistent_events_fail_closed() {
        for stdout in [
            b"not-json\n".as_slice(),
            br#"{"type":"new_event","sessionID":"one"}
"#,
            br#"{"type":"step_start","sessionID":"one"}
{"type":"text","sessionID":"two"}
"#,
            br#"{"type":"tool_use","sessionID":"one","part":{"id":"x","tool":"write","state":{"status":"running"}}}
"#,
            br#"{"type":"step_start","sessionID":"one"}
"#,
            br#"{"type":"step_finish","sessionID":"one","part":{}}
"#,
            br#"{"type":"step_start","sessionID":"one"}
{"type":"tool_use","sessionID":"one","part":{"id":"x","tool":"write","state":{"status":"completed"}}}
{"type":"tool_use","sessionID":"one","part":{"id":"x","tool":"write","state":{"status":"completed"}}}
{"type":"step_finish","sessionID":"one","part":{}}
"#,
        ] {
            assert!(matches!(
                map_events(
                    stdout,
                    &Id("s".into()),
                    &Id("a".into()),
                    TerminalStatus::Completed
                ),
                Err(AdapterError::InvalidEvent)
            ));
        }
    }

    #[test]
    fn event_line_and_count_limits_fail_closed() {
        let oversized = format!(
            "{{\"type\":\"text\",\"sessionID\":\"one\",\"padding\":\"{}\"}}\n",
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

        let mut excessive = String::new();
        for _ in 0..=MAX_UPSTREAM_EVENTS / 2 {
            excessive.push_str("{\"type\":\"step_start\",\"sessionID\":\"one\"}\n");
            excessive.push_str("{\"type\":\"step_finish\",\"sessionID\":\"one\",\"part\":{}}\n");
        }
        assert!(matches!(
            map_events(
                excessive.as_bytes(),
                &Id("s".into()),
                &Id("a".into()),
                TerminalStatus::Completed
            ),
            Err(AdapterError::InvalidEvent)
        ));
    }

    #[test]
    fn failures_are_redacted_and_cancellation_is_terminal() {
        let failed = map_events(
            br#"{"type":"error","sessionID":"up","error":{"message":"private"}}
"#,
            &Id("s".into()),
            &Id("a".into()),
            TerminalStatus::Failed,
        )
        .unwrap();
        let encoded = serde_json::to_string(&failed).unwrap();
        assert!(!encoded.contains("private"));
        assert!(matches!(failed.last().unwrap().event, Event::Failed(_)));

        let cancelled = map_events(
            b"",
            &Id("s".into()),
            &Id("a".into()),
            TerminalStatus::Cancelled,
        )
        .unwrap();
        assert_eq!(cancelled.len(), 2);

        let tool_error = map_events(
            br#"{"type":"step_start","sessionID":"up"}
{"type":"tool_use","sessionID":"up","part":{"id":"call-1","tool":"write","state":{"status":"error"}}}
{"type":"step_finish","sessionID":"up","part":{"tokens":{}}}
"#,
            &Id("s".into()),
            &Id("a".into()),
            TerminalStatus::Failed,
        )
        .unwrap();
        assert!(matches!(
            tool_error[4].event,
            Event::ToolFinished { success: false, .. }
        ));
        assert!(
            !tool_error
                .iter()
                .any(|event| matches!(event.event, Event::Usage(_)))
        );

        assert!(matches!(
            parse_usage(&json!({"part":{"tokens":{"input":"seven"}}})),
            Err(AdapterError::InvalidEvent)
        ));
        assert_eq!(
            parse_usage(&json!({"part":{"tokens":{"input":7}}}))
                .unwrap()
                .unwrap()
                .input_tokens,
            Some(7)
        );
        assert_eq!(
            parse_usage(&json!({"part":{"tokens":{"output":2}}}))
                .unwrap()
                .unwrap()
                .output_tokens,
            Some(2)
        );
    }

    #[test]
    fn real_process_boundary_cancels_and_removes_prompt() {
        let root = std::env::temp_dir().join(format!(
            "asb-opencode-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let binary = root.join("opencode");
        let workspace = root.join("work");
        let state = root.join("state");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            &binary,
            "#!/bin/sh\nprintf private > \"$HOME/session-state\"\ntrap 'exit 0' TERM\nsleep 60\n",
        )
        .unwrap();
        let mut permissions = fs::metadata(&binary).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o700);
        fs::set_permissions(&binary, permissions).unwrap();
        let digest = digest_file(&binary).unwrap();
        let mut adapter = OpenCodeConfig::new(
            &binary,
            &workspace,
            &state,
            Url::parse("http://127.0.0.1:1/v1").unwrap(),
            "asb/model",
            OpenCodeArtifact::LinuxX86_64V1_18_29,
        )
        .unwrap();
        adapter.verification_digest_override = Some(digest);
        let limits = ProcessLimits::new(
            4096,
            4096,
            Duration::from_secs(5),
            Duration::from_millis(50),
            Duration::from_millis(2),
        )
        .unwrap();
        let mut running = adapter
            .start(Id("s".into()), Id("a".into()), "private prompt", limits)
            .unwrap();
        let cmdline = fs::read(format!("/proc/{}/cmdline", running.pid())).unwrap();
        assert!(
            !cmdline
                .windows(b"private prompt".len())
                .any(|window| window == b"private prompt")
        );
        std::thread::sleep(Duration::from_millis(20));
        running.cancel().unwrap();
        let outcome = running.wait().unwrap();
        assert_eq!(outcome.status(), TerminalStatus::Cancelled);
        assert!(fs::read_dir(&state).unwrap().next().is_none());
        fs::remove_dir_all(root).unwrap();
    }
}
