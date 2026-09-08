// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Bounded adapter for the maintained OpenHands Software Agent SDK.

use asb_protocol::{
    Capability, Event, ExtensionEvent, ExtensionKind, ExtensionManifest, Id, PROTOCOL_V1, RpcError,
    TerminalStatus, Usage,
};
use asb_runtime::{ProcessError, ProcessLimits, RunningProcess, Termination};
use serde::Deserialize;
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, VecDeque};
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Seek, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};
use url::Url;

/// Newest OpenHands SDK release whose official lock avoids proprietary runtime dependencies.
pub const SUPPORTED_VERSION: &str = "1.17.0";
/// Immutable upstream tag commit inspected for this adapter.
pub const UPSTREAM_REVISION: &str = "aabf40723d308da0d5f9063008c6793cc86df282";
/// Immutable upstream source tree for the inspected tag.
pub const UPSTREAM_TREE: &str = "850dd602d64b8d19560e63c2d9a4d44c48db82f4";
/// SHA-256 of the universal OpenHands SDK 1.17.0 wheel.
pub const SDK_WHEEL_SHA256: &str =
    "3b771e72209453871c3036a562cf33e9ad9642a54bd48edb44f89915ac54709d";
/// SHA-256 of the independently downloaded upstream source archive.
pub const UPSTREAM_ARCHIVE_SHA256: &str =
    "2434fe9ef7de2e7ab8e6ca5b771ec82e9a6737d8d091a2ded16b5d23c02da2a7";
/// SHA-256 of the natively exercised Ubuntu CPython 3.12 executable.
pub const TESTED_PYTHON_LINUX_X86_64_SHA256: &str =
    "1643dacd9feaedc58f3cc581e4d22577dfe25c09b10282936186ccf0f2e61118";
/// Content digest of the complete, sorted SDK-only Python environment.
pub const TESTED_ENVIRONMENT_SHA256: &str =
    "6372756912734f6275362a8b66c3758fd2b2adeab776eb2a0be7935f34abb9b2";
/// Largest accepted prompt in UTF-8 bytes.
pub const MAX_PROMPT_BYTES: usize = 4 * 1024 * 1024;
/// Largest accepted number of SDK tool actions.
pub const MAX_ACTIONS: u32 = 4_096;

const MAX_ENDPOINT_BYTES: usize = 4 * 1024;
const MAX_MODEL_BYTES: usize = 256;
const MAX_ID_BYTES: usize = 4 * 1024;
const MAX_RESULT_BYTES: u64 = 16 * 1024 * 1024;
const MAX_RUNTIME_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_ENVIRONMENT_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_ENVIRONMENT_ENTRIES: usize = 200_000;
const CLOSED_PROXY: &str = "http://127.0.0.1:9";
const O_NOFOLLOW_CLOEXEC: i32 = 0x000a_0000;
const FAILURE_CODE: i32 = -32_100;

/// Content-pinned OpenHands artifact understood by this adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpenHandsArtifact {
    /// OpenHands SDK 1.17.0 exercised with CPython 3.12 on Linux x86_64.
    LinuxX86_64V1_17_0,
}

impl OpenHandsArtifact {
    const fn digest(self) -> &'static str {
        match self {
            Self::LinuxX86_64V1_17_0 => SDK_WHEEL_SHA256,
        }
    }
}

/// Validated immutable inputs for one OpenHands SDK installation.
#[derive(Clone, Debug)]
pub struct OpenHandsConfig {
    python: PathBuf,
    sdk_wheel: PathBuf,
    site_packages: PathBuf,
    workspace: PathBuf,
    state_root: PathBuf,
    endpoint: Url,
    model: String,
    max_actions: u32,
    artifact: OpenHandsArtifact,
    #[cfg(test)]
    python_digest_override: Option<String>,
    #[cfg(test)]
    wheel_digest_override: Option<String>,
    #[cfg(test)]
    environment_digest_override: Option<String>,
}

/// Configuration or production-boundary failure.
#[derive(Debug)]
pub enum AdapterError {
    /// A required path was not absolute.
    RelativePath(&'static str),
    /// A caller-owned file or directory had unsafe topology.
    UnsafePath(&'static str),
    /// The provider endpoint was malformed or unsafe.
    InvalidEndpoint,
    /// The provider model was malformed.
    InvalidModel,
    /// The action ceiling was zero or excessive.
    InvalidMaxActions,
    /// A public identity was empty or excessive.
    InvalidIdentity,
    /// The prompt exceeded its byte ceiling.
    PromptTooLarge,
    /// The SDK wheel did not match its content pin.
    WheelMismatch,
    /// The Python runtime did not match its content pin.
    RuntimeMismatch,
    /// The installed Python graph did not match its content pin.
    DependencyMismatch,
    /// Process execution failed.
    Process(ProcessError),
    /// Local preparation or cleanup failed.
    Io(io::Error),
    /// Structured evidence was absent, excessive, or malformed.
    InvalidEvidence,
    /// Captured process output was truncated or contained diagnostics.
    TruncatedOrDiagnosticOutput,
}

impl fmt::Display for AdapterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RelativePath(v) => write!(f, "{v} must be absolute"),
            Self::UnsafePath(v) => write!(f, "unsafe {v}"),
            Self::InvalidEndpoint => f.write_str("invalid provider endpoint"),
            Self::InvalidModel => f.write_str("invalid provider model"),
            Self::InvalidMaxActions => f.write_str("invalid OpenHands action ceiling"),
            Self::InvalidIdentity => f.write_str("invalid OpenHands correlation identity"),
            Self::PromptTooLarge => f.write_str("OpenHands prompt exceeds byte limit"),
            Self::WheelMismatch => f.write_str("OpenHands SDK wheel pin mismatch"),
            Self::RuntimeMismatch => f.write_str("OpenHands Python runtime pin mismatch"),
            Self::DependencyMismatch => f.write_str("OpenHands Python dependency pin mismatch"),
            Self::Process(e) => write!(f, "OpenHands process failed: {e}"),
            Self::Io(e) => write!(f, "OpenHands adapter I/O failed: {e}"),
            Self::InvalidEvidence => f.write_str("invalid OpenHands structured evidence"),
            Self::TruncatedOrDiagnosticOutput => {
                f.write_str("OpenHands output was truncated or contained diagnostics")
            }
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

impl OpenHandsConfig {
    /// Validate paths, endpoint, model, action ceiling, and artifact selection.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        python: impl Into<PathBuf>,
        sdk_wheel: impl Into<PathBuf>,
        site_packages: impl Into<PathBuf>,
        workspace: impl Into<PathBuf>,
        state_root: impl Into<PathBuf>,
        endpoint: Url,
        model: impl Into<String>,
        max_actions: u32,
        artifact: OpenHandsArtifact,
    ) -> Result<Self, AdapterError> {
        let python = python.into();
        let sdk_wheel = sdk_wheel.into();
        let site_packages = site_packages.into();
        let workspace = workspace.into();
        let state_root = state_root.into();
        for (label, path) in [
            ("Python runtime", python.as_path()),
            ("SDK wheel", sdk_wheel.as_path()),
            ("site-packages", site_packages.as_path()),
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
            || !model
                .bytes()
                .all(|v| v.is_ascii_alphanumeric() || matches!(v, b'.' | b'_' | b'-' | b'/' | b':'))
        {
            return Err(AdapterError::InvalidModel);
        }
        if max_actions == 0 || max_actions > MAX_ACTIONS {
            return Err(AdapterError::InvalidMaxActions);
        }
        Ok(Self {
            python,
            sdk_wheel,
            site_packages,
            workspace,
            state_root,
            endpoint,
            model,
            max_actions,
            artifact,
            #[cfg(test)]
            python_digest_override: None,
            #[cfg(test)]
            wheel_digest_override: None,
            #[cfg(test)]
            environment_digest_override: None,
        })
    }

    /// Describe capabilities proven for this exact SDK boundary.
    #[must_use]
    pub fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            extension_id: Id("agent.openhands".into()),
            kind: ExtensionKind::Agent,
            implementation_version: format!("openhands-sdk-{SUPPORTED_VERSION}+asb-0.1.0"),
            protocol: PROTOCOL_V1,
            capabilities: BTreeSet::from([Capability::Cancellation, Capability::Usage]),
            executable_sha256: self.artifact.digest().into(),
        }
    }

    /// Verify the runtime, SDK wheel, and complete Python environment.
    pub fn verify_installation(&self) -> Result<(), AdapterError> {
        if !exact_file(&self.python)?
            || digest_file(&self.python, MAX_RUNTIME_BYTES)? != self.python_verification_digest()
        {
            return Err(AdapterError::RuntimeMismatch);
        }
        if !exact_file(&self.sdk_wheel)?
            || digest_file(&self.sdk_wheel, MAX_RUNTIME_BYTES)? != self.wheel_verification_digest()
        {
            return Err(AdapterError::WheelMismatch);
        }
        if !exact_dir(&self.site_packages)?
            || digest_tree(&self.site_packages, None)? != self.environment_verification_digest()
        {
            return Err(AdapterError::DependencyMismatch);
        }
        Ok(())
    }

    /// Start one isolated SDK attempt.
    pub fn start(
        &self,
        session_id: Id,
        attempt_id: Id,
        prompt: &str,
        limits: ProcessLimits,
    ) -> Result<RunningOpenHands, AdapterError> {
        if !valid_id(&session_id) || !valid_id(&attempt_id) {
            return Err(AdapterError::InvalidIdentity);
        }
        if prompt.len() > MAX_PROMPT_BYTES || prompt.contains('\0') {
            return Err(AdapterError::PromptTooLarge);
        }
        self.verify_installation()?;
        if !exact_dir(&self.workspace)? {
            return Err(AdapterError::UnsafePath("workspace"));
        }
        if !exact_dir(&self.state_root)? || roots_overlap(&self.workspace, &self.state_root)? {
            return Err(AdapterError::UnsafePath("state root"));
        }
        let run_root = self.unique_run_root()?;
        fs::DirBuilder::new().mode(0o700).create(&run_root)?;
        let prepared = self.prepare(&run_root, prompt);
        let (python, environment, prompt_file, result_path) = match prepared {
            Ok(v) => v,
            Err(e) => {
                let _ = fs::remove_dir_all(&run_root);
                return Err(e);
            }
        };
        let process = match self.spawn(
            &run_root,
            &python,
            &environment,
            prompt_file,
            &result_path,
            limits,
        ) {
            Ok(v) => v,
            Err(e) => {
                let _ = fs::remove_dir_all(&run_root);
                return Err(e);
            }
        };
        Ok(RunningOpenHands {
            process,
            session_id,
            attempt_id,
            result_path,
            max_actions: self.max_actions,
            run_root: Some(run_root),
        })
    }

    fn prepare(
        &self,
        run_root: &Path,
        prompt: &str,
    ) -> Result<(PathBuf, PathBuf, fs::File, PathBuf), AdapterError> {
        for child in ["home", "config", "cache", "data", "tmp", "launch"] {
            fs::DirBuilder::new()
                .mode(0o700)
                .create(run_root.join(child))?;
        }
        let python = run_root.join("launch/python");
        copy_verified_file(
            &self.python,
            &python,
            self.python_verification_digest(),
            AdapterError::RuntimeMismatch,
        )?;
        let environment = run_root.join("launch/site-packages");
        fs::DirBuilder::new().mode(0o700).create(&environment)?;
        if digest_tree(&self.site_packages, Some(&environment))?
            != self.environment_verification_digest()
        {
            return Err(AdapterError::DependencyMismatch);
        }
        let path = run_root.join("prompt");
        let mut prompt_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)?;
        fs::remove_file(path)?;
        prompt_file.write_all(prompt.as_bytes())?;
        prompt_file.rewind()?;
        Ok((
            python,
            environment,
            prompt_file,
            run_root.join("result.json"),
        ))
    }

    fn spawn(
        &self,
        run_root: &Path,
        python: &Path,
        environment: &Path,
        prompt: fs::File,
        result: &Path,
        limits: ProcessLimits,
    ) -> Result<RunningProcess, AdapterError> {
        let host = self
            .endpoint
            .host_str()
            .ok_or(AdapterError::InvalidEndpoint)?;
        let mut command = Command::new(python);
        command
            .current_dir(&self.workspace)
            .args(["-P", "-S", "-c", DRIVER])
            .arg(&self.workspace)
            .arg(self.endpoint.as_str())
            .arg(&self.model)
            .arg(self.max_actions.to_string())
            .arg(result)
            .stdin(Stdio::from(prompt))
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", run_root.join("home"))
            .env("XDG_CONFIG_HOME", run_root.join("config"))
            .env("XDG_CACHE_HOME", run_root.join("cache"))
            .env("XDG_DATA_HOME", run_root.join("data"))
            .env("TMPDIR", run_root.join("tmp"))
            .env("PYTHONPATH", environment)
            .env("PYTHONHOME", "/usr")
            .env("PYTHONNOUSERSITE", "1")
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .env("OPENHANDS_SUPPRESS_BANNER", "1")
            .env("LITELLM_LOCAL_MODEL_COST_MAP", "True")
            .env("HTTP_PROXY", CLOSED_PROXY)
            .env("HTTPS_PROXY", CLOSED_PROXY)
            .env("ALL_PROXY", CLOSED_PROXY)
            .env("http_proxy", CLOSED_PROXY)
            .env("https_proxy", CLOSED_PROXY)
            .env("all_proxy", CLOSED_PROXY)
            .env("NO_PROXY", host)
            .env("no_proxy", host);
        RunningProcess::spawn(command, limits).map_err(Into::into)
    }

    fn unique_run_root(&self) -> Result<PathBuf, AdapterError> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos();
        Ok(self
            .state_root
            .join(format!("attempt-{}-{nonce}", std::process::id())))
    }

    fn python_verification_digest(&self) -> &str {
        #[cfg(test)]
        if let Some(v) = &self.python_digest_override {
            return v;
        }
        TESTED_PYTHON_LINUX_X86_64_SHA256
    }
    fn wheel_verification_digest(&self) -> &str {
        #[cfg(test)]
        if let Some(v) = &self.wheel_digest_override {
            return v;
        }
        self.artifact.digest()
    }
    fn environment_verification_digest(&self) -> &str {
        #[cfg(test)]
        if let Some(v) = &self.environment_digest_override {
            return v;
        }
        TESTED_ENVIRONMENT_SHA256
    }
}

/// A cancellable OpenHands SDK attempt.
pub struct RunningOpenHands {
    process: RunningProcess,
    session_id: Id,
    attempt_id: Id,
    result_path: PathBuf,
    max_actions: u32,
    run_root: Option<PathBuf>,
}

impl RunningOpenHands {
    /// Native process identifier.
    #[must_use]
    pub fn pid(&self) -> u32 {
        self.process.pid()
    }
    /// Request idempotent process-group cancellation.
    pub fn cancel(&mut self) -> Result<(), AdapterError> {
        self.process.cancel().map_err(Into::into)
    }
    /// Reap, map bounded evidence, and clean all private state.
    pub fn wait(&mut self) -> Result<OpenHandsOutcome, AdapterError> {
        let output = self.process.wait()?.clone();
        let evidence = if output.termination == Termination::Exited && output.exit_code == Some(0) {
            Some(read_evidence(&self.result_path)?)
        } else {
            None
        };
        self.cleanup()?;
        if output.stdout.truncated || output.stderr.truncated {
            return Err(AdapterError::TruncatedOrDiagnosticOutput);
        }
        if !output.stdout.bytes.is_empty() || !output.stderr.bytes.is_empty() {
            return Err(AdapterError::TruncatedOrDiagnosticOutput);
        }
        let status = match output.termination {
            Termination::Cancelled => TerminalStatus::Cancelled,
            Termination::TimedOut => TerminalStatus::Failed,
            Termination::Exited if output.exit_code == Some(0) => {
                if evidence.as_ref().is_some_and(|v| v.status == "completed") {
                    TerminalStatus::Completed
                } else {
                    TerminalStatus::Failed
                }
            }
            Termination::Exited => TerminalStatus::Failed,
        };
        let events = map_evidence(
            evidence.as_ref(),
            &self.session_id,
            &self.attempt_id,
            status,
            self.max_actions,
        )?;
        Ok(OpenHandsOutcome {
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

impl Drop for RunningOpenHands {
    fn drop(&mut self) {
        let _ = self.process.cancel();
        if self.process.wait().is_ok() {
            let _ = self.cleanup();
        }
    }
}

/// Privacy-filtered terminal evidence for one OpenHands attempt.
#[derive(Clone, Debug, PartialEq)]
pub struct OpenHandsOutcome {
    events: Vec<ExtensionEvent>,
    status: TerminalStatus,
    exit_code: Option<i32>,
}

impl OpenHandsOutcome {
    /// Ordered lifecycle, tool, usage, and terminal evidence.
    #[must_use]
    pub fn events(&self) -> &[ExtensionEvent] {
        &self.events
    }
    /// Terminal status derived from process and evidence boundaries.
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DriverEvidence {
    version: u32,
    status: String,
    actions: Vec<DriverAction>,
    input_tokens: u64,
    output_tokens: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DriverAction {
    id: String,
    name: String,
    success: bool,
}

fn read_evidence(path: &Path) -> Result<DriverEvidence, AdapterError> {
    if !exact_file(path)? {
        return Err(AdapterError::InvalidEvidence);
    }
    let bytes = fs::read(path)?;
    if bytes.is_empty() || bytes.len() as u64 > MAX_RESULT_BYTES {
        return Err(AdapterError::InvalidEvidence);
    }
    let unique =
        serde_json::from_slice::<UniqueJson>(&bytes).map_err(|_| AdapterError::InvalidEvidence)?;
    serde_json::from_value(unique.0).map_err(|_| AdapterError::InvalidEvidence)
}

struct UniqueJson(Value);

impl<'de> Deserialize<'de> for UniqueJson {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct UniqueVisitor;
        impl<'de> Visitor<'de> for UniqueVisitor {
            type Value = Value;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("JSON without duplicate object keys")
            }
            fn visit_bool<E>(self, value: bool) -> Result<Value, E> {
                Ok(Value::Bool(value))
            }
            fn visit_i64<E>(self, value: i64) -> Result<Value, E> {
                Ok(Value::Number(value.into()))
            }
            fn visit_u64<E>(self, value: u64) -> Result<Value, E> {
                Ok(Value::Number(value.into()))
            }
            fn visit_f64<E: de::Error>(self, value: f64) -> Result<Value, E> {
                serde_json::Number::from_f64(value)
                    .map(Value::Number)
                    .ok_or_else(|| E::custom("non-finite number"))
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Value, E> {
                Ok(Value::String(value.into()))
            }
            fn visit_string<E>(self, value: String) -> Result<Value, E> {
                Ok(Value::String(value))
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
        deserializer.deserialize_any(UniqueVisitor).map(Self)
    }
}

fn map_evidence(
    evidence: Option<&DriverEvidence>,
    session: &Id,
    attempt: &Id,
    status: TerminalStatus,
    max_actions: u32,
) -> Result<Vec<ExtensionEvent>, AdapterError> {
    let mut events = Vec::new();
    let mut sequence = 0;
    push(&mut events, &mut sequence, session, attempt, Event::Ready);
    push(
        &mut events,
        &mut sequence,
        session,
        attempt,
        Event::RequestStarted,
    );
    if let Some(value) = evidence {
        if value.version != 1
            || value.actions.len() > max_actions as usize
            || !matches!(value.status.as_str(), "completed" | "failed")
        {
            return Err(AdapterError::InvalidEvidence);
        }
        push(
            &mut events,
            &mut sequence,
            session,
            attempt,
            Event::FirstResponse,
        );
        let mut ids = BTreeSet::new();
        for action in &value.actions {
            if action.name != "write"
                || !valid_remote_id(&action.id)
                || !ids.insert(action.id.clone())
            {
                return Err(AdapterError::InvalidEvidence);
            }
            push(
                &mut events,
                &mut sequence,
                session,
                attempt,
                Event::ToolStarted {
                    tool_call_id: Id(action.id.clone()),
                    name: action.name.clone(),
                },
            );
            push(
                &mut events,
                &mut sequence,
                session,
                attempt,
                Event::ToolFinished {
                    tool_call_id: Id(action.id.clone()),
                    success: action.success,
                },
            );
        }
        if value.input_tokens > 0 && value.output_tokens > 0 {
            push(
                &mut events,
                &mut sequence,
                session,
                attempt,
                Event::Usage(Usage {
                    input_tokens: Some(value.input_tokens),
                    output_tokens: Some(value.output_tokens),
                    cost_micros: None,
                    currency: None,
                }),
            );
        } else if value.status == "completed" {
            return Err(AdapterError::InvalidEvidence);
        }
    }
    match status {
        TerminalStatus::Completed
            if evidence.is_some_and(|v| {
                v.status == "completed" && v.actions.iter().all(|a| a.success)
            }) =>
        {
            push(
                &mut events,
                &mut sequence,
                session,
                attempt,
                Event::Completed,
            );
        }
        TerminalStatus::Completed => return Err(AdapterError::InvalidEvidence),
        TerminalStatus::Failed => push(
            &mut events,
            &mut sequence,
            session,
            attempt,
            Event::Failed(RpcError {
                code: FAILURE_CODE,
                message: "OpenHands attempt failed; raw diagnostics are not retained".into(),
                data: None,
            }),
        ),
        TerminalStatus::Cancelled => {}
    }
    Ok(events)
}

fn push(
    events: &mut Vec<ExtensionEvent>,
    sequence: &mut u64,
    session: &Id,
    attempt: &Id,
    event: Event,
) {
    let current = *sequence;
    *sequence = sequence.saturating_add(1);
    events.push(ExtensionEvent {
        session_id: session.clone(),
        attempt_id: attempt.clone(),
        sequence: current,
        event,
    });
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
    endpoint.scheme() != "http" || matches!(host, "127.0.0.1" | "::1" | "[::1]" | "localhost")
}

fn valid_id(value: &Id) -> bool {
    !value.0.is_empty() && value.0.len() <= MAX_ID_BYTES
}

fn valid_remote_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ID_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':'))
}

fn exact_file(path: &Path) -> Result<bool, AdapterError> {
    Ok(fs::symlink_metadata(path)?.file_type().is_file() && fs::canonicalize(path)? == path)
}

fn exact_dir(path: &Path) -> Result<bool, AdapterError> {
    Ok(fs::symlink_metadata(path)?.file_type().is_dir() && fs::canonicalize(path)? == path)
}

fn roots_overlap(left: &Path, right: &Path) -> Result<bool, AdapterError> {
    let left = fs::canonicalize(left)?;
    let right = fs::canonicalize(right)?;
    Ok(left.starts_with(&right) || right.starts_with(&left))
}

fn digest_file(path: &Path, maximum: u64) -> Result<String, AdapterError> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(O_NOFOLLOW_CLOEXEC)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > maximum {
        return Err(AdapterError::UnsafePath("file"));
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

fn copy_verified_file(
    source: &Path,
    target: &Path,
    expected: &str,
    mismatch: AdapterError,
) -> Result<(), AdapterError> {
    if !exact_file(source)? {
        return Err(mismatch);
    }
    let mut input = OpenOptions::new()
        .read(true)
        .custom_flags(O_NOFOLLOW_CLOEXEC)
        .open(source)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o500)
        .open(target)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
        output.write_all(&buffer[..count])?;
    }
    output.sync_all()?;
    if format!("{:x}", digest.finalize()) != expected {
        return Err(mismatch);
    }
    Ok(())
}

fn digest_tree(source: &Path, target: Option<&Path>) -> Result<String, AdapterError> {
    let mut queue = VecDeque::from([(source.to_path_buf(), PathBuf::new())]);
    let mut entries = 0_usize;
    let mut bytes = 0_u64;
    let mut digest = Sha256::new();
    while let Some((directory, relative)) = queue.pop_front() {
        let mut children = fs::read_dir(&directory)?.collect::<Result<Vec<_>, _>>()?;
        children.sort_by_key(fs::DirEntry::file_name);
        for child in children {
            if child.file_name() == "__pycache__"
                || child.path().extension().is_some_and(|value| value == "pyc")
            {
                continue;
            }
            entries = entries
                .checked_add(1)
                .ok_or(AdapterError::DependencyMismatch)?;
            if entries > MAX_ENVIRONMENT_ENTRIES {
                return Err(AdapterError::DependencyMismatch);
            }
            let name = child.file_name();
            let name = name.to_str().ok_or(AdapterError::DependencyMismatch)?;
            let child_relative = relative.join(name);
            let encoded = child_relative
                .to_str()
                .ok_or(AdapterError::DependencyMismatch)?
                .as_bytes();
            let metadata = fs::symlink_metadata(child.path())?;
            if metadata.file_type().is_symlink() {
                return Err(AdapterError::DependencyMismatch);
            }
            digest.update((encoded.len() as u64).to_be_bytes());
            digest.update(encoded);
            if metadata.is_dir() {
                digest.update(b"D");
                if let Some(root) = target {
                    fs::DirBuilder::new()
                        .mode(0o700)
                        .create(root.join(&child_relative))?;
                }
                queue.push_back((child.path(), child_relative));
            } else if metadata.is_file() {
                digest.update(b"F");
                digest.update(metadata.len().to_be_bytes());
                bytes = bytes
                    .checked_add(metadata.len())
                    .filter(|v| *v <= MAX_ENVIRONMENT_BYTES)
                    .ok_or(AdapterError::DependencyMismatch)?;
                let mut input = OpenOptions::new()
                    .read(true)
                    .custom_flags(O_NOFOLLOW_CLOEXEC)
                    .open(child.path())?;
                let mut output = if let Some(root) = target {
                    Some(
                        OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .mode(if metadata.permissions().mode() & 0o111 == 0 {
                                0o400
                            } else {
                                0o500
                            })
                            .open(root.join(&child_relative))?,
                    )
                } else {
                    None
                };
                let mut buffer = [0_u8; 64 * 1024];
                loop {
                    let count = input.read(&mut buffer)?;
                    if count == 0 {
                        break;
                    }
                    digest.update(&buffer[..count]);
                    if let Some(file) = &mut output {
                        file.write_all(&buffer[..count])?;
                    }
                }
                if let Some(file) = &mut output {
                    file.sync_all()?;
                }
            } else {
                return Err(AdapterError::DependencyMismatch);
            }
        }
    }
    Ok(format!("{:x}", digest.finalize()))
}

const DRIVER: &str = r#"
import json, logging, os, sys, warnings
os.environ['LOG_LEVEL']='CRITICAL'
warnings.filterwarnings('ignore')
logging.disable(logging.CRITICAL)
from collections.abc import Sequence
from pathlib import Path
from pydantic import Field, SecretStr
from openhands.sdk import LLM, Action, Agent, Conversation, Observation, ToolDefinition
from openhands.sdk.conversation.state import ConversationExecutionStatus, ConversationState
from openhands.sdk.event import ActionEvent, ObservationEvent
from openhands.sdk.security.confirmation_policy import AlwaysConfirm
from openhands.sdk.tool import Tool, ToolExecutor, register_tool
class WriteAction(Action):
    path: str = Field(description='Workspace-relative output path')
    content: str = Field(description='UTF-8 output content')
class WriteObservation(Observation):
    path: str
    bytes_written: int
class WriteExecutor(ToolExecutor[WriteAction, WriteObservation]):
    def __init__(self, workspace): self.workspace=workspace
    def __call__(self, action, conversation=None):
        if action.path != 'result.txt' or '\x00' in action.content or len(action.content.encode()) > 16777216:
            raise ValueError('rejected bounded write')
        root=os.open(self.workspace, os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW|os.O_CLOEXEC)
        try:
            fd=os.open(action.path, os.O_WRONLY|os.O_CREAT|os.O_TRUNC|os.O_NOFOLLOW|os.O_CLOEXEC, 0o600, dir_fd=root)
            try:
                with os.fdopen(fd, 'w', encoding='utf-8', closefd=False) as output:
                    output.write(action.content)
                    output.flush()
                    os.fsync(fd)
            finally:
                os.close(fd)
        finally:
            os.close(root)
        return WriteObservation(path=action.path, bytes_written=len(action.content.encode()))
class WriteTool(ToolDefinition[WriteAction, WriteObservation]):
    @classmethod
    def create(cls, conv_state, **params) -> Sequence[ToolDefinition]:
        workspace=Path(conv_state.workspace.working_dir)
        return [cls(action_type=WriteAction, observation_type=WriteObservation, description='Write exact UTF-8 content to result.txt.', executor=WriteExecutor(workspace))]
register_tool(WriteTool.name, WriteTool)
workspace=Path(sys.argv[1])
result=Path(sys.argv[5])
actions={}
def on_event(event):
    if isinstance(event, ActionEvent) and event.tool_name == 'write':
        actions[event.tool_call_id]={'id':event.tool_call_id,'name':'write','success':False}
    elif isinstance(event, ObservationEvent) and event.tool_name == 'write':
        if event.tool_call_id in actions: actions[event.tool_call_id]['success']=True
try:
    llm=LLM(usage_id='agent', model='openai/'+sys.argv[3], api_key=SecretStr('asb-credential-free'), base_url=sys.argv[2])
    agent=Agent(llm=llm, tools=[Tool(name=WriteTool.name)])
    conversation=Conversation(agent=agent, workspace=workspace, callbacks=[on_event], persistence_dir=None, visualizer=None, max_iteration_per_run=int(sys.argv[4])+1)
    conversation.set_confirmation_policy(AlwaysConfirm())
    conversation.send_message(sys.stdin.read())
    approvals=0
    while conversation.state.execution_status != ConversationExecutionStatus.FINISHED:
        conversation.run()
        if conversation.state.execution_status == ConversationExecutionStatus.WAITING_FOR_CONFIRMATION:
            pending=ConversationState.get_unmatched_actions(conversation.state.events)
            if not pending or any(a.tool_name != 'write' for a in pending) or approvals+len(pending)>int(sys.argv[4]):
                raise RuntimeError('unexpected pending action')
            approvals += len(pending)
        elif conversation.state.execution_status != ConversationExecutionStatus.FINISHED:
            raise RuntimeError('unexpected terminal status')
    usage=conversation.conversation_stats.get_combined_metrics().accumulated_token_usage
    payload={'version':1,'status':'completed','actions':list(actions.values()),'input_tokens':usage.prompt_tokens if usage else 0,'output_tokens':usage.completion_tokens if usage else 0}
    conversation.close()
except Exception:
    payload={'version':1,'status':'failed','actions':list(actions.values()),'input_tokens':0,'output_tokens':0}
temporary=result.with_suffix('.tmp')
temporary.write_text(json.dumps(payload, separators=(',',':')), encoding='utf-8')
os.chmod(temporary, 0o600)
os.replace(temporary, result)
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::time::Duration;

    struct RemoveDirectory(PathBuf);

    impl Drop for RemoveDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn evidence_requires_positive_usage_and_bounded_unique_actions() {
        let session = Id("s".into());
        let attempt = Id("a".into());
        let valid = DriverEvidence {
            version: 1,
            status: "completed".into(),
            actions: vec![DriverAction {
                id: "call-1".into(),
                name: "write".into(),
                success: true,
            }],
            input_tokens: 10,
            output_tokens: 4,
        };
        let events = map_evidence(
            Some(&valid),
            &session,
            &attempt,
            TerminalStatus::Completed,
            1,
        )
        .unwrap();
        assert!(matches!(events.last().unwrap().event, Event::Completed));
        let missing_usage = DriverEvidence {
            input_tokens: 0,
            ..valid
        };
        assert!(
            map_evidence(
                Some(&missing_usage),
                &session,
                &attempt,
                TerminalStatus::Completed,
                1
            )
            .is_err()
        );
    }

    #[test]
    fn endpoint_and_identity_validation_fail_closed() {
        assert!(!valid_endpoint(
            &Url::parse("http://example.com/v1").unwrap()
        ));
        assert!(valid_endpoint(
            &Url::parse("http://127.0.0.1:1/v1").unwrap()
        ));
        assert!(!valid_id(&Id(String::new())));
    }

    #[test]
    fn evidence_rejects_excess_duplicate_unknown_and_failed_actions() {
        let session = Id("s".into());
        let attempt = Id("a".into());
        let make = |id: &str, name: &str, success| DriverAction {
            id: id.into(),
            name: name.into(),
            success,
        };
        for actions in [
            vec![make("one", "write", true), make("two", "write", true)],
            vec![make("one", "write", true), make("one", "write", true)],
            vec![make("one", "shell", true)],
            vec![make("one", "write", false)],
            vec![make("private value", "write", true)],
        ] {
            let evidence = DriverEvidence {
                version: 1,
                status: "completed".into(),
                actions,
                input_tokens: 1,
                output_tokens: 1,
            };
            assert!(
                map_evidence(
                    Some(&evidence),
                    &session,
                    &attempt,
                    TerminalStatus::Completed,
                    1
                )
                .is_err()
            );
        }
    }

    #[test]
    fn evidence_parser_rejects_duplicate_and_unknown_fields() {
        let root = std::env::temp_dir().join(format!(
            "asb-openhands-evidence-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let _remove = RemoveDirectory(root.clone());
        let path = root.join("result.json");
        fs::write(
            &path,
            br#"{"version":1,"version":1,"status":"completed","actions":[],"input_tokens":1,"output_tokens":1}"#,
        )
        .unwrap();
        assert!(matches!(
            read_evidence(&path),
            Err(AdapterError::InvalidEvidence)
        ));
        fs::write(
            &path,
            br#"{"version":1,"status":"completed","actions":[],"input_tokens":1,"output_tokens":1,"raw":"secret"}"#,
        )
        .unwrap();
        assert!(matches!(
            read_evidence(&path),
            Err(AdapterError::InvalidEvidence)
        ));
    }

    #[test]
    fn environment_digest_rejects_symlinks_and_copies_regular_content() {
        let root = std::env::temp_dir().join(format!(
            "asb-openhands-tree-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let source = root.join("source");
        let target = root.join("target");
        let _remove = RemoveDirectory(root.clone());
        fs::create_dir_all(source.join("package")).unwrap();
        fs::create_dir_all(&target).unwrap();
        fs::write(source.join("package/module.py"), "value = 1\n").unwrap();
        let source_digest = digest_tree(&source, Some(&target)).unwrap();
        assert_eq!(source_digest, digest_tree(&target, None).unwrap());
        symlink("/etc/passwd", source.join("package/escape")).unwrap();
        assert!(matches!(
            digest_tree(&source, None),
            Err(AdapterError::DependencyMismatch)
        ));
    }

    #[test]
    fn cancellation_reaps_and_cleans_private_state() {
        let root = std::env::temp_dir().join(format!(
            "asb-openhands-cancel-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let python = root.join("python");
        let wheel = root.join("sdk.whl");
        let environment = root.join("site-packages");
        let workspace = root.join("workspace");
        let state = root.join("state");
        let _remove = RemoveDirectory(root.clone());
        fs::create_dir_all(&environment).unwrap();
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&state).unwrap();
        fs::write(&python, "#!/bin/sh\nsleep 30\n").unwrap();
        fs::set_permissions(&python, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(&wheel, "fixture wheel").unwrap();
        fs::write(environment.join("module.py"), "fixture = True\n").unwrap();
        let mut config = OpenHandsConfig::new(
            &python,
            &wheel,
            &environment,
            &workspace,
            &state,
            Url::parse("http://127.0.0.1:1/v1").unwrap(),
            "fixture",
            1,
            OpenHandsArtifact::LinuxX86_64V1_17_0,
        )
        .unwrap();
        config.python_digest_override = Some(digest_file(&python, MAX_RUNTIME_BYTES).unwrap());
        config.wheel_digest_override = Some(digest_file(&wheel, MAX_RUNTIME_BYTES).unwrap());
        config.environment_digest_override = Some(digest_tree(&environment, None).unwrap());
        let limits = ProcessLimits::new(
            1024,
            1024,
            Duration::from_secs(10),
            Duration::from_millis(100),
            Duration::from_millis(5),
        )
        .unwrap();
        let mut running = config
            .start(
                Id("session".into()),
                Id("attempt".into()),
                "fixture",
                limits,
            )
            .unwrap();
        running.cancel().unwrap();
        let outcome = running.wait().unwrap();
        assert_eq!(outcome.status(), TerminalStatus::Cancelled);
        assert!(fs::read_dir(&state).unwrap().next().is_none());
    }
}
