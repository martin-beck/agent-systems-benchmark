// SPDX-License-Identifier: MIT
//! Bounded adapter for mini-SWE-agent's noninteractive Python interface.

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
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Seek, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};
use url::Url;

/// MiniSwe release whose batch interface this module implements.
pub const SUPPORTED_VERSION: &str = "2.4.6";
/// Immutable upstream Git revision inspected for this adapter.
pub const UPSTREAM_REVISION: &str = "a83fcae82d2a08f0ee0c688f9d137b3566c097f8";
/// Immutable tree for the inspected unsigned upstream tag.
pub const UPSTREAM_TREE: &str = "665df42f5761252b83d9a30e1b82f76f1f17f828";
/// SHA-256 of the inspected `mini-swe-agent==2.4.6` universal wheel.
pub const WHEEL_SHA256: &str = "a35463c553ac825c7773b03cfa69cd44958e3af20155dcc5711fdf9e4c67cd54";
/// SHA-256 of the independently downloaded PyPI source distribution.
pub const SDIST_SHA256: &str = "0532c8193a763409fa52bb2b5a5d7ac9052dcb1c2cae43945b14b1b7f6ba869a";
/// SHA-256 of the natively exercised CPython 3.12 Linux x86_64 runtime.
pub const TESTED_PYTHON_LINUX_X86_64_SHA256: &str =
    "1643dacd9feaedc58f3cc581e4d22577dfe25c09b10282936186ccf0f2e61118";
/// Largest accepted prompt, in UTF-8 bytes.
pub const MAX_PROMPT_BYTES: usize = 4 * 1024 * 1024;
const MAX_ARTIFACT_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_ENDPOINT_BYTES: usize = 4 * 1024;
const MAX_MODEL_BYTES: usize = 256;
const MAX_TRAJECTORY_BYTES: u64 = 64 * 1024 * 1024;
const MAX_TRAJECTORY_MESSAGES: usize = 65_536;
const MAX_ACTIONS: usize = 4_096;
const MAX_WORKSPACE_FILES: usize = 4_096;
const MAX_WORKSPACE_ENTRIES: usize = 16_384;
const MAX_WORKSPACE_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_WORKSPACE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_PATH_BYTES: usize = 4 * 1024;
const CLOSED_PROXY: &str = "http://127.0.0.1:9";
const KNOWN_MINI_SWE_EGRESS: &[&str] = &[
    "mini-swe-agent.com",
    "api.github.com",
    "openrouter.ai",
    "pypi.org",
    "raw.githubusercontent.com",
    "us.i.posthog.com",
];

/// Content-pinned mini_swe artifact understood by this adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MiniSweArtifact {
    /// mini-SWE-agent 2.4.6 wheel exercised with CPython 3.12 on Linux x86_64.
    LinuxX86_64V2_4_6,
}

impl MiniSweArtifact {
    const fn digest(self) -> &'static str {
        match self {
            Self::LinuxX86_64V2_4_6 => WHEEL_SHA256,
        }
    }
}

/// Validated immutable inputs for one mini_swe installation.
#[derive(Clone, Debug)]
pub struct MiniSweConfig {
    python: PathBuf,
    wheel: PathBuf,
    workspace: PathBuf,
    state_root: PathBuf,
    endpoint: Url,
    model: String,
    artifact: MiniSweArtifact,
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
    /// The mini_swe wheel did not match its content pin.
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
    /// The saved trajectory violated the pinned bounded contract.
    InvalidTrajectory,
}

impl fmt::Display for AdapterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RelativePath(label) => write!(formatter, "{label} must be absolute"),
            Self::InvalidEndpoint => formatter.write_str("invalid provider endpoint"),
            Self::InvalidModel => formatter.write_str("invalid provider/model identifier"),
            Self::InvalidPrompt => formatter.write_str("invalid mini_swe prompt"),
            Self::PromptTooLarge => formatter.write_str("mini_swe prompt exceeds byte limit"),
            Self::Io(error) => write!(formatter, "adapter I/O failed: {error}"),
            Self::WheelMismatch => formatter.write_str("mini_swe wheel pin mismatch"),
            Self::RuntimeMismatch => formatter.write_str("mini_swe Python runtime pin mismatch"),
            Self::Process(error) => write!(formatter, "mini_swe process failed: {error}"),
            Self::UnsafeWorkspace => formatter.write_str("unsafe or excessive mini_swe workspace"),
            Self::UnsafeStateRoot => formatter.write_str("unsafe mini_swe state root"),
            Self::CleanupFailed => formatter.write_str("mini_swe private run-state cleanup failed"),
            Self::InvalidTrajectory => formatter.write_str("invalid mini-SWE-agent trajectory"),
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

impl MiniSweConfig {
    /// Validate paths, endpoint, model, and select a content-pinned artifact.
    pub fn new(
        python: impl Into<PathBuf>,
        wheel: impl Into<PathBuf>,
        workspace: impl Into<PathBuf>,
        state_root: impl Into<PathBuf>,
        endpoint: Url,
        model: impl Into<String>,
        artifact: MiniSweArtifact,
    ) -> Result<Self, AdapterError> {
        let python = python.into();
        let wheel = wheel.into();
        let workspace = workspace.into();
        let state_root = state_root.into();
        for (label, path) in [
            ("Python runtime", python.as_path()),
            ("mini_swe wheel", wheel.as_path()),
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
        if KNOWN_MINI_SWE_EGRESS
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
            extension_id: Id("agent.mini-swe".into()),
            kind: ExtensionKind::Agent,
            implementation_version: format!("mini-swe-agent-{SUPPORTED_VERSION}+asb-0.1.0"),
            protocol: PROTOCOL_V1,
            capabilities: BTreeSet::from([Capability::Cancellation]),
            executable_sha256: self.artifact.digest().into(),
        }
    }

    /// Verify the exact wheel and Python runtime without importing mini_swe.
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

    /// Start one noninteractive edit attempt using an unlinked prompt descriptor.
    pub fn start(
        &self,
        session_id: Id,
        attempt_id: Id,
        prompt: &str,
        limits: ProcessLimits,
    ) -> Result<RunningMiniSwe, AdapterError> {
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
        if !has_canonical_existing_ancestor(&self.state_root)? {
            return Err(AdapterError::UnsafeStateRoot);
        }
        fs::create_dir_all(&self.workspace)?;
        collect_editable_files(&self.workspace)?;
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
            let prompt_path = run_root.join("prompt.txt");
            let mut prompt_file = OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&prompt_path)?;
            fs::remove_file(prompt_path)?;
            prompt_file.write_all(prompt.as_bytes())?;
            prompt_file.rewind()?;
            Ok::<fs::File, AdapterError>(prompt_file)
        })();
        let prompt_file = match prepared {
            Ok(path) => path,
            Err(error) => {
                let _ = fs::remove_dir_all(&run_root);
                return Err(error);
            }
        };
        let trajectory_path = run_root.join("trajectory.json");
        let process = match self.spawn_process(&run_root, prompt_file, &trajectory_path, limits) {
            Ok(process) => process,
            Err(error) => {
                let _ = fs::remove_dir_all(&run_root);
                return Err(error);
            }
        };
        Ok(RunningMiniSwe {
            process,
            session_id,
            attempt_id,
            trajectory_path,
            run_root: Some(run_root),
        })
    }

    fn spawn_process(
        &self,
        run_root: &Path,
        prompt_file: fs::File,
        trajectory_path: &Path,
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
        let python_path = site_packages;
        const DRIVER: &str = r#"import sys, zipfile
zipfile.ZipFile(sys.argv[6]).extractall(sys.argv[7])
sys.path.insert(0,sys.argv[7])
from minisweagent.agents import get_agent
from minisweagent.config import get_config_from_spec
from minisweagent.environments import get_environment
from minisweagent.models import get_model
from minisweagent.utils.serialize import recursive_merge
base=get_config_from_spec('mini.yaml')
override={'agent':{'mode':'yolo','confirm_exit':False,'step_limit':4096,'wall_time_limit_seconds':int(sys.argv[4]),'output_path':sys.argv[3]},'environment':{'environment_class':'local','cwd':sys.argv[1],'timeout':30},'model':{'model_class':'litellm','model_name':'openai/'+sys.argv[2],'cost_tracking':'ignore_errors','model_kwargs':{'api_base':sys.argv[5],'api_key':'asb-credential-free'}}}
cfg=recursive_merge(base,override)
agent=get_agent(get_model(config=cfg['model']),get_environment(cfg['environment'],default_type='local'),cfg['agent'],default_type='default')
agent.run(sys.stdin.read())
"#;
        let mut command = Command::new(&self.python);
        command
            .current_dir(&self.workspace)
            .args(["-P", "-c", DRIVER])
            .arg(&self.workspace)
            .arg(&self.model)
            .arg(trajectory_path)
            .arg(limits.timeout().as_secs().max(1).to_string())
            .arg(self.endpoint.as_str())
            .arg(&self.wheel)
            .arg(run_root.join("package"))
            .stdin(Stdio::from(prompt_file))
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", run_root.join("home"))
            .env("XDG_CONFIG_HOME", run_root.join("config"))
            .env("XDG_DATA_HOME", run_root.join("data"))
            .env("XDG_CACHE_HOME", run_root.join("cache"))
            .env("TMPDIR", run_root.join("tmp"))
            .env("PYTHONPATH", python_path)
            .env("MSWEA_GLOBAL_CONFIG_DIR", run_root.join("config"))
            .env("MSWEA_SILENT_STARTUP", "1")
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

/// A cancellable mini_swe process whose raw output is never exposed.
pub struct RunningMiniSwe {
    process: RunningProcess,
    session_id: Id,
    attempt_id: Id,
    trajectory_path: PathBuf,
    run_root: Option<PathBuf>,
}

impl RunningMiniSwe {
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
    pub fn wait(&mut self) -> Result<MiniSweOutcome, AdapterError> {
        let waited = self.process.wait().cloned();
        let output = match waited {
            Ok(output) => output,
            Err(error) => {
                let _ = self.remove_run_root();
                return Err(error.into());
            }
        };
        let status = match output.termination {
            Termination::Cancelled => TerminalStatus::Cancelled,
            Termination::TimedOut => TerminalStatus::Failed,
            Termination::Exited if output.exit_code == Some(0) => TerminalStatus::Completed,
            Termination::Exited => TerminalStatus::Failed,
        };
        let mapped = if status == TerminalStatus::Completed {
            read_trajectory(&self.trajectory_path)
                .and_then(|bytes| map_trajectory(&bytes, &self.session_id, &self.attempt_id))
        } else {
            Ok((
                terminal_events(&self.session_id, &self.attempt_id, status),
                status,
            ))
        };
        self.remove_run_root()?;
        let (events, status) = mapped?;
        Ok(MiniSweOutcome {
            events,
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

impl Drop for RunningMiniSwe {
    fn drop(&mut self) {
        let _ = self.process.cancel();
        if self.process.wait().is_ok() {
            let _ = self.remove_run_root();
        }
    }
}

/// Privacy-filtered terminal evidence for one mini_swe attempt.
#[derive(Clone, Debug, PartialEq)]
pub struct MiniSweOutcome {
    events: Vec<ExtensionEvent>,
    status: TerminalStatus,
    exit_code: Option<i32>,
    stdout_truncated: bool,
    stderr_truncated: bool,
    retry_observation: RetryObservation,
}

/// Evidence available for retries performed inside mini_swe's unstructured batch contract.
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
    /// mini-SWE-agent 2.4.6 reports retries only in content-bearing diagnostics.
    UnstructuredBatchDiagnostics,
}

impl MiniSweOutcome {
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

    /// Typed evidence boundary for retries internal to mini_swe.
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
                message: "mini_swe attempt failed; raw diagnostics are not retained".into(),
                data: None,
            }),
        )),
        TerminalStatus::Cancelled => {}
    }
    events
}

fn read_trajectory(path: &Path) -> Result<Vec<u8>, AdapterError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_TRAJECTORY_BYTES {
        return Err(AdapterError::InvalidTrajectory);
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(MAX_TRAJECTORY_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_TRAJECTORY_BYTES {
        return Err(AdapterError::InvalidTrajectory);
    }
    Ok(bytes)
}

fn map_trajectory(
    bytes: &[u8],
    session: &Id,
    attempt: &Id,
) -> Result<(Vec<ExtensionEvent>, TerminalStatus), AdapterError> {
    let root = serde_json::from_slice::<UniqueJson>(bytes)
        .map_err(|_| AdapterError::InvalidTrajectory)?
        .0;
    if root.get("trajectory_format").and_then(Value::as_str) != Some("mini-swe-agent-1.1")
        || root.pointer("/info/mini_version").and_then(Value::as_str) != Some(SUPPORTED_VERSION)
    {
        return Err(AdapterError::InvalidTrajectory);
    }
    let messages = root
        .get("messages")
        .and_then(Value::as_array)
        .ok_or(AdapterError::InvalidTrajectory)?;
    if messages.len() > MAX_TRAJECTORY_MESSAGES {
        return Err(AdapterError::InvalidTrajectory);
    }
    let mut events = vec![
        event(session, attempt, 0, Event::Ready),
        event(session, attempt, 1, Event::RequestStarted),
    ];
    let mut sequence = 2_u64;
    let mut pending = BTreeSet::new();
    let mut actions = 0_usize;
    let mut responses = 0_u64;
    let mut saw_response = false;
    if messages.len() < 3
        || messages
            .first()
            .and_then(|message| message.get("role"))
            .and_then(Value::as_str)
            != Some("system")
        || messages
            .get(1)
            .and_then(|message| message.get("role"))
            .and_then(Value::as_str)
            != Some("user")
        || messages
            .last()
            .and_then(|message| message.get("role"))
            .and_then(Value::as_str)
            != Some("exit")
    {
        return Err(AdapterError::InvalidTrajectory);
    }
    for (index, message) in messages.iter().enumerate() {
        let object = message.as_object().ok_or(AdapterError::InvalidTrajectory)?;
        match object
            .get("role")
            .and_then(Value::as_str)
            .ok_or(AdapterError::InvalidTrajectory)?
        {
            "assistant" => {
                if !pending.is_empty() {
                    return Err(AdapterError::InvalidTrajectory);
                }
                responses = responses
                    .checked_add(1)
                    .ok_or(AdapterError::InvalidTrajectory)?;
                if !saw_response {
                    events.push(event(session, attempt, sequence, Event::FirstResponse));
                    sequence += 1;
                    saw_response = true;
                }
                if let Some(items) = object
                    .get("extra")
                    .and_then(|v| v.get("actions"))
                    .and_then(Value::as_array)
                {
                    for item in items {
                        actions = actions
                            .checked_add(1)
                            .ok_or(AdapterError::InvalidTrajectory)?;
                        if actions > MAX_ACTIONS {
                            return Err(AdapterError::InvalidTrajectory);
                        }
                        let id = item
                            .get("tool_call_id")
                            .and_then(Value::as_str)
                            .ok_or(AdapterError::InvalidTrajectory)?;
                        item.get("command")
                            .and_then(Value::as_str)
                            .ok_or(AdapterError::InvalidTrajectory)?;
                        if !safe_id(id) || !pending.insert(id.to_owned()) {
                            return Err(AdapterError::InvalidTrajectory);
                        }
                        events.push(event(
                            session,
                            attempt,
                            sequence,
                            Event::ToolStarted {
                                tool_call_id: Id(id.into()),
                                name: "bash".into(),
                            },
                        ));
                        sequence += 1;
                    }
                }
            }
            "tool" => {
                let id = object
                    .get("tool_call_id")
                    .and_then(Value::as_str)
                    .ok_or(AdapterError::InvalidTrajectory)?;
                if !pending.remove(id) {
                    return Err(AdapterError::InvalidTrajectory);
                }
                let code = object
                    .get("extra")
                    .and_then(|v| v.get("returncode"))
                    .and_then(Value::as_i64)
                    .ok_or(AdapterError::InvalidTrajectory)?;
                events.push(event(
                    session,
                    attempt,
                    sequence,
                    Event::ToolFinished {
                        tool_call_id: Id(id.into()),
                        success: code == 0,
                    },
                ));
                sequence += 1;
            }
            "system" if index == 0 => {}
            "user" if index == 1 => {}
            "exit" if index + 1 == messages.len() => {}
            _ => return Err(AdapterError::InvalidTrajectory),
        }
    }
    let exit_status = root
        .pointer("/info/exit_status")
        .and_then(Value::as_str)
        .ok_or(AdapterError::InvalidTrajectory)?;
    if exit_status == "Submitted" && pending.len() == 1 {
        let id = pending.pop_first().ok_or(AdapterError::InvalidTrajectory)?;
        events.push(event(
            session,
            attempt,
            sequence,
            Event::ToolFinished {
                tool_call_id: Id(id),
                success: true,
            },
        ));
        sequence += 1;
    }
    if !saw_response || !pending.is_empty() {
        return Err(AdapterError::InvalidTrajectory);
    }
    let calls = root
        .pointer("/info/model_stats/api_calls")
        .and_then(Value::as_u64)
        .ok_or(AdapterError::InvalidTrajectory)?;
    if calls == 0 || calls as usize > MAX_ACTIONS || calls != responses {
        return Err(AdapterError::InvalidTrajectory);
    }
    let cost = root
        .pointer("/info/model_stats/instance_cost")
        .and_then(Value::as_f64)
        .ok_or(AdapterError::InvalidTrajectory)?;
    if !cost.is_finite() || !(0.0..=1_000_000.0).contains(&cost) {
        return Err(AdapterError::InvalidTrajectory);
    }
    let cost_micros = Some((cost * 1_000_000.0).round() as u64);
    events.push(event(
        session,
        attempt,
        sequence,
        Event::Usage(Usage {
            input_tokens: None,
            output_tokens: None,
            cost_micros,
            currency: Some("USD".into()),
        }),
    ));
    sequence += 1;
    if exit_status == "Submitted" {
        events.push(event(session, attempt, sequence, Event::Completed));
        Ok((events, TerminalStatus::Completed))
    } else {
        events.push(event(
            session,
            attempt,
            sequence,
            Event::Failed(RpcError {
                code: -32_100,
                message: "mini-SWE-agent did not submit; raw diagnostics are not retained".into(),
                data: None,
            }),
        ));
        Ok((events, TerminalStatus::Failed))
    }
}

fn safe_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && value.bytes().all(|byte| !byte.is_ascii_control())
}

struct UniqueJson(Value);
impl<'de> Deserialize<'de> for UniqueJson {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
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
        let mut out = Vec::new();
        while let Some(value) = values.next_element::<UniqueJson>()? {
            out.push(value.0);
        }
        Ok(UniqueJson(Value::Array(out)))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut values: A) -> Result<Self::Value, A::Error> {
        let mut out = serde_json::Map::new();
        while let Some(key) = values.next_key::<String>()? {
            if out.contains_key(&key) {
                return Err(de::Error::custom("duplicate JSON object member"));
            }
            out.insert(key, values.next_value::<UniqueJson>()?.0);
        }
        Ok(UniqueJson(Value::Object(out)))
    }
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

    fn ids() -> (Id, Id) {
        (Id("session".into()), Id("attempt".into()))
    }
    fn parse(input: &str) -> Result<(Vec<ExtensionEvent>, TerminalStatus), AdapterError> {
        let (session, attempt) = ids();
        map_trajectory(input.as_bytes(), &session, &attempt)
    }
    fn valid(actions: &str, tools: &str, exit: &str, calls: u64, cost: &str) -> String {
        format!(
            r#"{{"trajectory_format":"mini-swe-agent-1.1","info":{{"mini_version":"2.4.6","model_stats":{{"api_calls":{calls},"instance_cost":{cost}}},"exit_status":"{exit}"}},"messages":[{{"role":"system"}},{{"role":"user"}},{{"role":"assistant","extra":{{"actions":{actions}}}}}{tools},{{"role":"exit"}}]}}"#
        )
    }

    #[test]
    fn maps_bounded_causal_tool_and_failed_tool() {
        let input = valid(
            r#"[{"tool_call_id":"call-1","command":"false"}]"#,
            r#",{"role":"tool","tool_call_id":"call-1","extra":{"returncode":7}}"#,
            "Submitted",
            1,
            "0.25",
        );
        let (events, status) = parse(&input).unwrap();
        assert_eq!(status, TerminalStatus::Completed);
        assert!(
            events
                .iter()
                .any(|e| matches!(&e.event, Event::ToolFinished { success: false, .. }))
        );
        assert!(events.iter().any(|e| matches!(
            &e.event,
            Event::Usage(Usage {
                cost_micros: Some(250_000),
                ..
            })
        )));
    }

    #[test]
    fn non_submission_is_failed_not_completed() {
        let (_, status) = parse(&valid("[]", "", "LimitsExceeded", 1, "0")).unwrap();
        assert_eq!(status, TerminalStatus::Failed);
    }

    #[test]
    fn rejects_duplicate_members_ids_unmatched_tools_and_unknown_roles() {
        let duplicate_member = r#"{"trajectory_format":"mini-swe-agent-1.1","trajectory_format":"mini-swe-agent-1.1"}"#;
        assert!(matches!(
            parse(duplicate_member),
            Err(AdapterError::InvalidTrajectory)
        ));
        let duplicate_id = valid(
            r#"[{"tool_call_id":"x","command":"a"},{"tool_call_id":"x","command":"b"}]"#,
            "",
            "Submitted",
            1,
            "0",
        );
        assert!(matches!(
            parse(&duplicate_id),
            Err(AdapterError::InvalidTrajectory)
        ));
        let unmatched = valid(
            "[]",
            r#",{"role":"tool","tool_call_id":"x","extra":{"returncode":0}}"#,
            "Submitted",
            1,
            "0",
        );
        assert!(matches!(
            parse(&unmatched),
            Err(AdapterError::InvalidTrajectory)
        ));
        let unknown = valid("[]", r#",{"role":"developer"}"#, "Submitted", 1, "0");
        assert!(matches!(
            parse(&unknown),
            Err(AdapterError::InvalidTrajectory)
        ));
    }

    #[test]
    fn rejects_invalid_usage_and_unfinished_actions() {
        assert!(matches!(
            parse(&valid("[]", "", "Submitted", 0, "0")),
            Err(AdapterError::InvalidTrajectory)
        ));
        assert!(matches!(
            parse(&valid("[]", "", "Submitted", 1, "-1")),
            Err(AdapterError::InvalidTrajectory)
        ));
        assert!(matches!(
            parse(&valid(
                r#"[{"tool_call_id":"x","command":"a"}]"#,
                "",
                "LimitsExceeded",
                1,
                "0"
            )),
            Err(AdapterError::InvalidTrajectory)
        ));
        let wrong_calls = valid("[]", "", "Submitted", 2, "0");
        assert!(matches!(
            parse(&wrong_calls),
            Err(AdapterError::InvalidTrajectory)
        ));
        let missing_command = valid(r#"[{"tool_call_id":"x"}]"#, "", "Submitted", 1, "0");
        assert!(matches!(
            parse(&missing_command),
            Err(AdapterError::InvalidTrajectory)
        ));
        let reordered = valid("[]", r#",{"role":"user"}"#, "Submitted", 1, "0");
        assert!(matches!(
            parse(&reordered),
            Err(AdapterError::InvalidTrajectory)
        ));
        let scalar_coverage = valid("[]", "", "Submitted", 1, "0").replace(
            "\"trajectory_format\"",
            "\"ignored\":[true,-1,1,1.5,\"text\",null],\"trajectory_format\"",
        );
        assert!(parse(&scalar_coverage).is_ok());
    }

    #[test]
    fn public_errors_and_outcome_evidence_are_redacted_and_stable() {
        let errors = [
            AdapterError::RelativePath("fixture"),
            AdapterError::InvalidEndpoint,
            AdapterError::InvalidModel,
            AdapterError::InvalidPrompt,
            AdapterError::PromptTooLarge,
            AdapterError::Io(io::Error::other("io")),
            AdapterError::WheelMismatch,
            AdapterError::RuntimeMismatch,
            AdapterError::UnsafeWorkspace,
            AdapterError::UnsafeStateRoot,
            AdapterError::CleanupFailed,
            AdapterError::InvalidTrajectory,
            AdapterError::Process(ProcessError::Spawn(io::Error::other("spawn"))),
        ];
        for error in errors {
            let text = error.to_string();
            assert!(!text.is_empty());
            assert!(!text.contains("private prompt"));
        }
        let (events, status) = parse(&valid("[]", "", "Submitted", 1, "0")).unwrap();
        let outcome = MiniSweOutcome {
            events,
            status,
            exit_code: Some(0),
            stdout_truncated: true,
            stderr_truncated: false,
            retry_observation: RetryObservation::Unavailable {
                reason: RetryUnavailableReason::UnstructuredBatchDiagnostics,
            },
        };
        assert_eq!(outcome.exit_code(), Some(0));
        assert!(outcome.stdout_truncated());
        assert!(!outcome.stderr_truncated());
        assert!(!outcome.events().is_empty());
        assert_eq!(
            outcome.retry_observation(),
            RetryObservation::Unavailable {
                reason: RetryUnavailableReason::UnstructuredBatchDiagnostics
            }
        );
        let _: AdapterError = io::Error::other("io conversion").into();
        let _: AdapterError = ProcessError::Spawn(io::Error::other("process conversion")).into();
        assert!(matches!(
            terminal_events(&Id("s".into()), &Id("a".into()), TerminalStatus::Completed)
                .last()
                .unwrap()
                .event,
            Event::Completed
        ));
    }

    #[test]
    fn trajectory_file_and_spawn_failures_are_bounded_and_cleaned() {
        let base = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let root = base.join(format!("asb-mini-swe-files-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let empty = root.join("empty");
        fs::write(&empty, b"").unwrap();
        assert!(matches!(
            read_trajectory(&empty),
            Err(AdapterError::InvalidTrajectory)
        ));
        let huge = root.join("huge");
        fs::File::create(&huge)
            .unwrap()
            .set_len(MAX_TRAJECTORY_BYTES + 1)
            .unwrap();
        assert!(matches!(
            read_trajectory(&huge),
            Err(AdapterError::InvalidTrajectory)
        ));
        let regular = root.join("regular");
        fs::write(&regular, b"{}").unwrap();
        assert_eq!(read_trajectory(&regular).unwrap(), b"{}");
        let (scratch, mut config) = boundary_tests::adapter("not an executable format");
        assert!(matches!(
            config.start(
                Id("s".into()),
                Id("a".into()),
                "prompt",
                boundary_tests::limits()
            ),
            Err(AdapterError::Process(_))
        ));
        assert!(
            fs::read_dir(scratch.0.join("state"))
                .unwrap()
                .next()
                .is_none()
        );
        config.python_digest_override = Some("00".repeat(32));
        assert!(matches!(
            config.verify_executable(),
            Err(AdapterError::RuntimeMismatch)
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn child_import_boundary_uses_private_config_before_driver_runs() {
        let base = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let root = base.join(format!("asb-mini-swe-isolation-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("workspace")).unwrap();
        fs::create_dir_all(root.join("state")).unwrap();
        let python = root.join("python");
        let wheel = root.join("package.whl");
        fs::write(&wheel, b"fixture").unwrap();
        fs::write(&python, r#"#!/bin/sh
case "$HOME" in */attempt-*/home) ;; *) exit 91;; esac
test "$MSWEA_GLOBAL_CONFIG_DIR" = "${HOME%/home}/config" || exit 92
test ! -e "$HOME/ambient-sentinel" || exit 93
cat > "$6" <<'EOF'
{"trajectory_format":"mini-swe-agent-1.1","info":{"mini_version":"2.4.6","model_stats":{"api_calls":1,"instance_cost":0.0},"exit_status":"Submitted"},"messages":[{"role":"system"},{"role":"user"},{"role":"assistant","extra":{"actions":[]}},{"role":"exit"}]}
EOF
"#).unwrap();
        let mut mode = fs::metadata(&python).unwrap().permissions();
        mode.set_mode(0o700);
        fs::set_permissions(&python, mode).unwrap();
        let endpoint = Url::parse("http://127.0.0.1:1/v1/").unwrap();
        let mut config = MiniSweConfig::new(
            &python,
            &wheel,
            root.join("workspace"),
            root.join("state"),
            endpoint,
            "fixture",
            MiniSweArtifact::LinuxX86_64V2_4_6,
        )
        .unwrap();
        config.wheel_digest_override = Some(digest_file(&wheel).unwrap());
        config.python_digest_override = Some(digest_file(&python).unwrap());
        let limits = ProcessLimits::new(
            1024 * 1024,
            1024 * 1024,
            Duration::from_secs(5),
            Duration::from_millis(50),
            Duration::from_millis(5),
        )
        .unwrap();
        let mut running = config
            .start(Id("s".into()), Id("a".into()), "private prompt", limits)
            .unwrap();
        assert_eq!(running.wait().unwrap().status(), TerminalStatus::Completed);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cancellation_reaps_owned_descendant_group() {
        let base = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let root = base.join(format!("asb-mini-swe-descendant-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("workspace")).unwrap();
        fs::create_dir_all(root.join("state")).unwrap();
        let python = root.join("python");
        let wheel = root.join("package.whl");
        fs::write(&wheel, b"fixture").unwrap();
        fs::write(
            &python,
            r#"#!/bin/sh
sleep 30 &
echo $! > "$4/child.pid"
wait
"#,
        )
        .unwrap();
        let mut mode = fs::metadata(&python).unwrap().permissions();
        mode.set_mode(0o700);
        fs::set_permissions(&python, mode).unwrap();
        let mut config = MiniSweConfig::new(
            &python,
            &wheel,
            root.join("workspace"),
            root.join("state"),
            Url::parse("http://127.0.0.1:1/v1/").unwrap(),
            "fixture",
            MiniSweArtifact::LinuxX86_64V2_4_6,
        )
        .unwrap();
        config.wheel_digest_override = Some(digest_file(&wheel).unwrap());
        config.python_digest_override = Some(digest_file(&python).unwrap());
        let limits = ProcessLimits::new(
            1024,
            1024,
            Duration::from_secs(10),
            Duration::from_millis(20),
            Duration::from_millis(5),
        )
        .unwrap();
        let mut running = config
            .start(Id("s".into()), Id("a".into()), "prompt", limits)
            .unwrap();
        let pid_path = root.join("workspace/child.pid");
        for _ in 0..200 {
            if pid_path.exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let child = fs::read_to_string(&pid_path).unwrap();
        running.cancel().unwrap();
        assert_eq!(running.wait().unwrap().status(), TerminalStatus::Cancelled);
        let proc_path = PathBuf::from("/proc").join(child.trim());
        for _ in 0..200 {
            if !proc_path.exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!proc_path.exists());
        fs::remove_dir_all(root).unwrap();
    }
}
#[cfg(test)]
mod boundary_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::time::Duration;

    pub(super) struct Scratch(pub(super) PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "asb-mini_swe-{label}-{}-{nonce}",
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

    pub(super) fn limits() -> ProcessLimits {
        ProcessLimits::new(
            64 * 1024,
            64 * 1024,
            Duration::from_secs(2),
            Duration::from_millis(200),
            Duration::from_millis(5),
        )
        .unwrap()
    }

    pub(super) fn adapter(script: &str) -> (Scratch, MiniSweConfig) {
        let scratch = Scratch::new("process");
        let python = scratch.0.join("python");
        let wheel = scratch.0.join("mini_swe.whl");
        fs::write(&python, script).unwrap();
        fs::set_permissions(&python, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(&wheel, "wheel").unwrap();
        let mut config = MiniSweConfig::new(
            &python,
            &wheel,
            scratch.0.join("workspace"),
            scratch.0.join("state"),
            Url::parse("http://127.0.0.1:1/v1").unwrap(),
            "fixture-model",
            MiniSweArtifact::LinuxX86_64V2_4_6,
        )
        .unwrap();
        config.python_digest_override = Some(digest_file(&python).unwrap());
        config.wheel_digest_override = Some(digest_file(&wheel).unwrap());
        (scratch, config)
    }

    #[test]
    fn configuration_manifest_and_pins_are_bounded() {
        let (_scratch, config) = adapter("#!/bin/sh\nexit 0\n");
        let mut defaults = config.clone();
        defaults.wheel_digest_override = None;
        defaults.python_digest_override = None;
        assert_eq!(defaults.wheel_verification_digest(), WHEEL_SHA256);
        assert_eq!(
            defaults.python_verification_digest(),
            TESTED_PYTHON_LINUX_X86_64_SHA256
        );
        assert_eq!(config.manifest().extension_id.0, "agent.mini-swe");
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
                MiniSweConfig::new(
                    "/python",
                    "/mini_swe.whl",
                    "/workspace",
                    "/state",
                    Url::parse(endpoint).unwrap(),
                    "model",
                    MiniSweArtifact::LinuxX86_64V2_4_6,
                ),
                Err(AdapterError::InvalidEndpoint)
            ));
        }
        assert!(matches!(
            MiniSweConfig::new(
                "python",
                "/mini_swe.whl",
                "/workspace",
                "/state",
                Url::parse("https://example.invalid/v1").unwrap(),
                "model",
                MiniSweArtifact::LinuxX86_64V2_4_6,
            ),
            Err(AdapterError::RelativePath("Python runtime"))
        ));
        assert!(matches!(
            MiniSweConfig::new(
                "/python",
                "/mini_swe.whl",
                "/workspace",
                "/state",
                Url::parse("https://example.invalid/v1").unwrap(),
                "bad model\n",
                MiniSweArtifact::LinuxX86_64V2_4_6,
            ),
            Err(AdapterError::InvalidModel)
        ));
        assert!(matches!(
            MiniSweConfig::new(
                "/python",
                "/wheel",
                "/workspace",
                "/state",
                Url::parse("https://example.invalid/v1").unwrap(),
                "x".repeat(MAX_MODEL_BYTES + 1),
                MiniSweArtifact::LinuxX86_64V2_4_6
            ),
            Err(AdapterError::InvalidModel)
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
