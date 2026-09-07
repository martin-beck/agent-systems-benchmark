// SPDX-License-Identifier: MIT
//! Bounded adapter for the Gemini CLI headless stream-JSON interface.

use asb_protocol::{
    Capability, Event, ExtensionEvent, ExtensionKind, ExtensionManifest, Id, PROTOCOL_V1, RpcError,
    TerminalStatus, Usage,
};
use asb_runtime::{ProcessError, ProcessLimits, RunningProcess, Termination};
use serde::Deserialize;
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::Component;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
#[cfg(test)]
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};
use url::Url;

/// Gemini CLI release whose stream-JSON dialect this module implements.
pub const SUPPORTED_VERSION: &str = "0.58.0";
/// Immutable upstream Git revision inspected for this adapter.
pub const UPSTREAM_REVISION: &str = "ac9431c9e2290d68af31a77614ff2fddb2391ca3";
/// Tree of the inspected upstream revision.
pub const UPSTREAM_TREE: &str = "5dbf74073ce1f63693abdf02d88136d7f018eeb6";
/// SHA-256 of the official `@google/gemini-cli@0.58.0` npm tarball.
pub const NPM_TARBALL_SHA256: &str =
    "8ffeb9e7edddffb054764d00749f39e8cc9804ca9b38b9093f906dd2157322ae";
/// npm registry SHA-512 integrity value for the supported package.
pub const NPM_TARBALL_INTEGRITY: &str = "sha512-++LtUYMcLE8dVxMcuwv6kIp8+h6z+std/7iVE+vSunkrwNDaWMFkWw/psv2RSySWjr2A1SsEEIGCK0xULWY2sA==";
/// SHA-256 of the exercised npm entry bundle.
pub const TESTED_ENTRY_SHA256: &str =
    "25f087a42f4484891aa73e6e36cc2790e69a3c023cc44fd244adf44a091eebd7";
/// SHA-256 of sorted sha256sum-style records for all 446 npm bundle files.
pub const TESTED_BUNDLE_TREE_SHA256: &str =
    "3e6332e8d8117f0bb41bffd67df0f216e305c745d6ab63c5e27e98769da492e1";
/// SHA-256 of the exercised Node.js 26.3.0 Linux x86_64 runtime.
pub const TESTED_NODE_LINUX_X86_64_SHA256: &str =
    "5325ac9da58541494afcc136f0880279a2a853609bf4dae7755e04fb682b6926";

const MAX_EXECUTABLE_BYTES: u64 = 1024 * 1024 * 1024;
const TESTED_BUNDLE_FILE_COUNT: usize = 446;
const O_NOFOLLOW_CLOEXEC: i32 = 0x000a_0000;
/// Largest accepted prompt, in UTF-8 bytes.
pub const MAX_PROMPT_BYTES: usize = 4 * 1024 * 1024;
/// Largest individual Gemini stream-JSON line.
pub const MAX_EVENT_LINE_BYTES: usize = 16 * 1024 * 1024;
/// Largest accepted number of upstream events.
pub const MAX_UPSTREAM_EVENTS: usize = 65_536;
/// Largest configurable tool-action budget.
pub const MAX_ACTIONS: u32 = 4_096;
/// Largest configurable Gemini session-turn budget.
pub const MAX_TURNS: u32 = 4_096;
const MAX_ENDPOINT_BYTES: usize = 4 * 1024;
const MAX_MODEL_BYTES: usize = 256;
const MAX_LABEL_BYTES: usize = 256;
const MAX_PUBLIC_ID_BYTES: usize = 4 * 1024;
const HOOK_READY_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(test)]
static RUN_NONCE: AtomicU64 = AtomicU64::new(0);

const ACTION_HOOK: &str = r#"'use strict';
const fs = require('fs');
const max = Number(process.env.ASB_MAX_ACTIONS);
const counter = process.env.ASB_ACTION_COUNTER;
const lock = counter + '.lock';
const sleep = new Int32Array(new SharedArrayBuffer(4));

function stop() {
  process.stdout.write(JSON.stringify({
    decision: 'deny',
    reason: 'ASB action budget exhausted',
    continue: false,
    stopReason: 'ASB action budget exhausted',
    suppressOutput: true
  }));
}

function exhausted() {
  try {
    fs.writeFileSync(counter + '.exhausted', 'exhausted\n', {
      encoding: 'utf8',
      mode: 0o600,
      flag: 'wx'
    });
  } catch (_) {}
  stop();
}

function claim() {
  if (!Number.isSafeInteger(max) || max < 1 || !counter) {
    stop();
    return;
  }
  const deadline = Date.now() + 2000;
  for (;;) {
    try {
      fs.mkdirSync(lock, { mode: 0o700 });
      break;
    } catch (error) {
      if (error.code !== 'EEXIST' || Date.now() >= deadline) {
        stop();
        return;
      }
      Atomics.wait(sleep, 0, 0, 10);
    }
  }
  try {
    const text = fs.readFileSync(counter, 'utf8');
    if (!/^(0|[1-9][0-9]*)\n?$/.test(text)) {
      stop();
      return;
    }
    const current = Number(text.trim());
    const next = current + 1;
    if (!Number.isSafeInteger(current) || !Number.isSafeInteger(next)) {
      stop();
      return;
    }
    if (next > max) {
      exhausted();
      return;
    }
    fs.writeFileSync(counter, String(next) + '\n', {
      encoding: 'utf8',
      mode: 0o600
    });
    process.stdout.write('{}');
  } catch (_) {
    stop();
  } finally {
    try { fs.rmdirSync(lock); } catch (_) {}
  }
}

let bytes = 0;
const chunks = [];
process.stdin.on('data', (chunk) => {
  bytes += chunk.length;
  if (bytes > 16 * 1024 * 1024) {
    stop();
    process.exit(0);
  } else {
    chunks.push(chunk);
  }
});
process.stdin.on('end', () => {
  try {
    const input = JSON.parse(Buffer.concat(chunks).toString('utf8'));
    if (input.hook_event_name === 'SessionStart') {
      fs.writeFileSync(counter + '.ready', 'ready\n', { encoding: 'utf8', mode: 0o600 });
      process.stdout.write('{}');
    } else if (input.hook_event_name === 'BeforeTool') {
      claim();
    } else {
      stop();
    }
  } catch (_) {
    stop();
  }
});
process.stdin.on('error', stop);
"#;

const TOOL_POLICY: &str = r#"[[rule]]
toolName = "*"
decision = "deny"
priority = 998
denyMessage = "ASB permits only bounded editing tools"

[[rule]]
toolName = ["write_file", "replace"]
decision = "allow"
priority = 999
"#;

/// Content-pinned Gemini artifact understood by this adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GeminiArtifact {
    /// npm Gemini CLI 0.58.0, exercised natively on Linux x86_64.
    LinuxX86_64V0_58_0,
}

impl GeminiArtifact {
    const fn digest(self) -> &'static str {
        match self {
            Self::LinuxX86_64V0_58_0 => TESTED_ENTRY_SHA256,
        }
    }
}

/// Validated immutable inputs for one Gemini CLI installation.
#[derive(Clone, Debug)]
pub struct GeminiConfig {
    binary: PathBuf,
    node_binary: PathBuf,
    workspace: PathBuf,
    state_root: PathBuf,
    endpoint: Url,
    model: String,
    max_turns: u32,
    max_actions: u32,
    artifact: GeminiArtifact,
    #[cfg(test)]
    verification_digest_override: Option<String>,
    #[cfg(test)]
    runtime_digest_override: Option<String>,
    #[cfg(test)]
    bundle_digest_override: Option<String>,
    #[cfg(test)]
    bundle_file_count_override: Option<usize>,
    #[cfg(test)]
    system_config_root_override: Option<PathBuf>,
}

/// Configuration or production-boundary failure.
#[derive(Debug)]
pub enum AdapterError {
    /// A required path was not absolute.
    RelativePath(&'static str),
    /// A path embedded in private CLI hook configuration was not UTF-8.
    InvalidPathEncoding,
    /// The provider endpoint was not a safe HTTP(S) base URL.
    InvalidEndpoint,
    /// The model identifier was malformed.
    InvalidModel,
    /// A session or attempt correlation identifier was malformed.
    InvalidIdentity,
    /// A zero or excessive turn/action budget was requested.
    InvalidBudget,
    /// Prompt input exceeded the byte ceiling.
    PromptTooLarge,
    /// Prompt input cannot be represented at the process boundary.
    InvalidPrompt,
    /// Workspace-owned Gemini configuration would change adapter behavior.
    AmbientConfiguration,
    /// Workspace or private state roots have unsafe or overlapping topology.
    UnsafeRootTopology,
    /// Machine-wide Gemini configuration could alter the isolated invocation.
    AmbientSystemConfiguration,
    /// The pinned CLI did not prove that its action hook was active.
    HookUnavailable,
    /// Local preparation or digest inspection failed.
    Io(io::Error),
    /// The Gemini entry bundle did not match its content pin.
    ExecutableMismatch,
    /// The required content-pinned Node.js runtime did not match.
    RuntimeMismatch,
    /// Native process execution failed.
    Process(ProcessError),
    /// Structured output exceeded a byte or line bound.
    TruncatedOutput,
    /// Gemini emitted malformed, inconsistent, or unsupported structured output.
    InvalidEvent,
}

impl fmt::Display for AdapterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RelativePath(label) => write!(formatter, "{label} must be absolute"),
            Self::InvalidPathEncoding => formatter.write_str("Gemini hook path is not UTF-8"),
            Self::InvalidEndpoint => formatter.write_str("invalid Gemini provider endpoint"),
            Self::InvalidModel => formatter.write_str("invalid Gemini model identifier"),
            Self::InvalidIdentity => formatter.write_str("invalid Gemini correlation identity"),
            Self::InvalidBudget => formatter.write_str("invalid Gemini action or turn budget"),
            Self::PromptTooLarge => formatter.write_str("Gemini prompt exceeds byte limit"),
            Self::InvalidPrompt => formatter.write_str("invalid Gemini prompt"),
            Self::AmbientConfiguration => {
                formatter.write_str("workspace Gemini configuration is not allowed")
            }
            Self::UnsafeRootTopology => {
                formatter.write_str("unsafe Gemini workspace or state-root topology")
            }
            Self::AmbientSystemConfiguration => {
                formatter.write_str("ambient system Gemini configuration is not allowed")
            }
            Self::HookUnavailable => formatter.write_str("Gemini action hook did not initialize"),
            Self::Io(error) => write!(formatter, "adapter I/O failed: {error}"),
            Self::ExecutableMismatch => formatter.write_str("Gemini executable pin mismatch"),
            Self::RuntimeMismatch => formatter.write_str("Gemini Node.js runtime pin mismatch"),
            Self::Process(error) => write!(formatter, "Gemini process failed: {error}"),
            Self::TruncatedOutput => formatter.write_str("Gemini structured output was truncated"),
            Self::InvalidEvent => formatter.write_str("invalid Gemini structured event"),
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

impl GeminiConfig {
    /// Validate paths, endpoint, model, budgets, and select a pinned artifact.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        binary: impl Into<PathBuf>,
        node_binary: impl Into<PathBuf>,
        workspace: impl Into<PathBuf>,
        state_root: impl Into<PathBuf>,
        endpoint: Url,
        model: impl Into<String>,
        max_turns: u32,
        max_actions: u32,
        artifact: GeminiArtifact,
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
            || (endpoint.scheme() == "http" && !endpoint.host_str().is_some_and(loopback_host))
        {
            return Err(AdapterError::InvalidEndpoint);
        }
        let model = model.into();
        if !safe_label(&model, MAX_MODEL_BYTES)
            || !model
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        {
            return Err(AdapterError::InvalidModel);
        }
        if max_turns == 0 || max_turns > MAX_TURNS || max_actions == 0 || max_actions > MAX_ACTIONS
        {
            return Err(AdapterError::InvalidBudget);
        }
        Ok(Self {
            binary,
            node_binary,
            workspace,
            state_root,
            endpoint,
            model,
            max_turns,
            max_actions,
            artifact,
            #[cfg(test)]
            verification_digest_override: None,
            #[cfg(test)]
            runtime_digest_override: None,
            #[cfg(test)]
            bundle_digest_override: None,
            #[cfg(test)]
            bundle_file_count_override: None,
            #[cfg(test)]
            system_config_root_override: None,
        })
    }

    /// Produce explicit capabilities for this exact installation.
    #[must_use]
    pub fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            extension_id: Id("agent.gemini".into()),
            kind: ExtensionKind::Agent,
            implementation_version: format!("gemini-{SUPPORTED_VERSION}+asb-0.1.0"),
            protocol: PROTOCOL_V1,
            capabilities: BTreeSet::from([Capability::Cancellation, Capability::Usage]),
            executable_sha256: self.artifact.digest().into(),
        }
    }

    /// Verify the entry bundle and Node.js runtime without executing either.
    pub fn verify_executable(&self) -> Result<(), AdapterError> {
        if digest_file(&self.binary)? != self.verification_digest() {
            return Err(AdapterError::ExecutableMismatch);
        }
        let bundle_root = self
            .binary
            .parent()
            .ok_or(AdapterError::ExecutableMismatch)?;
        if digest_bundle_tree(bundle_root, self.bundle_file_count())?
            != self.bundle_verification_digest()
        {
            return Err(AdapterError::ExecutableMismatch);
        }
        match digest_file(&self.node_binary) {
            Ok(digest) if digest == self.runtime_verification_digest() => Ok(()),
            Ok(_) | Err(AdapterError::ExecutableMismatch) => Err(AdapterError::RuntimeMismatch),
            Err(error) => Err(error),
        }
    }

    fn verification_digest(&self) -> &str {
        #[cfg(test)]
        if let Some(digest) = &self.verification_digest_override {
            return digest;
        }
        self.artifact.digest()
    }

    fn runtime_verification_digest(&self) -> &str {
        #[cfg(test)]
        if let Some(digest) = &self.runtime_digest_override {
            return digest;
        }
        TESTED_NODE_LINUX_X86_64_SHA256
    }

    fn bundle_verification_digest(&self) -> &str {
        #[cfg(test)]
        if let Some(digest) = &self.bundle_digest_override {
            return digest;
        }
        TESTED_BUNDLE_TREE_SHA256
    }

    fn bundle_file_count(&self) -> usize {
        #[cfg(test)]
        if let Some(count) = self.bundle_file_count_override {
            return count;
        }
        TESTED_BUNDLE_FILE_COUNT
    }

    /// Start one noninteractive Gemini attempt with prompt bytes absent from argv and disk.
    pub fn start(
        &self,
        session_id: Id,
        attempt_id: Id,
        prompt: &str,
        limits: ProcessLimits,
    ) -> Result<RunningGemini, AdapterError> {
        if !valid_public_id(&session_id) || !valid_public_id(&attempt_id) {
            return Err(AdapterError::InvalidIdentity);
        }
        if prompt.len() > MAX_PROMPT_BYTES {
            return Err(AdapterError::PromptTooLarge);
        }
        if prompt.contains('\0') {
            return Err(AdapterError::InvalidPrompt);
        }
        let workspace = validate_root_topology(&self.workspace)?;
        let state_root = validate_root_topology(&self.state_root)?;
        if roots_overlap(&workspace, &state_root) {
            return Err(AdapterError::UnsafeRootTopology);
        }
        self.reject_system_configuration()?;
        self.reject_ambient_configuration()?;
        self.verify_executable()?;
        fs::create_dir_all(&self.workspace)?;
        fs::create_dir_all(&self.state_root)?;
        let run_root = self.route_run_root(&session_id, &attempt_id);
        fs::DirBuilder::new().mode(0o700).create(&run_root)?;
        let prepared = self.prepare_run_root(&run_root, prompt);
        let prompt_file = match prepared {
            Ok(file) => file,
            Err(error) => {
                let _ = fs::remove_dir_all(&run_root);
                return Err(error);
            }
        };
        let result = self.spawn_process(&run_root, prompt_file, limits);
        match result {
            Ok(mut process) => {
                if !wait_for_hook_ready(&run_root) {
                    let _ = process.cancel();
                    let _ = process.wait();
                    let _ = fs::remove_dir_all(run_root);
                    return Err(AdapterError::HookUnavailable);
                }
                Ok(RunningGemini {
                    process,
                    session_id,
                    attempt_id,
                    model: self.model.clone(),
                    max_actions: self.max_actions,
                    run_root: Some(run_root),
                })
            }
            Err(error) => {
                let _ = fs::remove_dir_all(run_root);
                Err(error)
            }
        }
    }

    fn reject_ambient_configuration(&self) -> Result<(), AdapterError> {
        for relative in [".gemini", "GEMINI.md", "gemini.md"] {
            match fs::symlink_metadata(self.workspace.join(relative)) {
                Ok(_) => return Err(AdapterError::AmbientConfiguration),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    fn reject_system_configuration(&self) -> Result<(), AdapterError> {
        #[cfg(test)]
        let root = self
            .system_config_root_override
            .as_deref()
            .unwrap_or_else(|| Path::new("/etc/gemini-cli"));
        #[cfg(not(test))]
        let root = Path::new("/etc/gemini-cli");
        for relative in ["settings.json", "system-defaults.json", "policies"] {
            match fs::symlink_metadata(root.join(relative)) {
                Ok(_) => return Err(AdapterError::AmbientSystemConfiguration),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    fn prepare_run_root(&self, run_root: &Path, prompt: &str) -> Result<fs::File, AdapterError> {
        for directory in ["home", "config", "data", "cache", "tmp"] {
            fs::DirBuilder::new()
                .mode(0o700)
                .create(run_root.join(directory))?;
        }
        let gemini_home = run_root.join("home/.gemini");
        fs::DirBuilder::new().mode(0o700).create(&gemini_home)?;
        let policies = gemini_home.join("policies");
        fs::DirBuilder::new().mode(0o700).create(&policies)?;
        let hook_path = run_root.join("action-budget.cjs");
        let counter_path = run_root.join("action-count");
        write_private(&hook_path, ACTION_HOOK.as_bytes())?;
        write_private(&counter_path, b"0\n")?;
        write_private(&policies.join("asb.toml"), TOOL_POLICY.as_bytes())?;
        write_private(&run_root.join("config/system-settings.json"), b"{}\n")?;
        write_private(&run_root.join("config/system-defaults.json"), b"{}\n")?;
        let hook_command = format!(
            "{} {}",
            shell_quote(
                self.node_binary
                    .to_str()
                    .ok_or(AdapterError::InvalidPathEncoding)?
            ),
            shell_quote(
                hook_path
                    .to_str()
                    .ok_or(AdapterError::InvalidPathEncoding)?
            )
        );
        let settings = json!({
            "privacy": {"usageStatisticsEnabled": false},
            "telemetry": {"enabled": false},
            "model": {"maxSessionTurns": self.max_turns},
            "security": {"auth": {"selectedType": "gemini-api-key"}},
            "hooksConfig": {"enabled": true, "notifications": false},
            "hooks": {
                "SessionStart": [{
                    "matcher": "startup",
                    "sequential": true,
                    "hooks": [{
                        "name": "asb-action-budget-ready",
                        "type": "command",
                        "command": hook_command,
                        "timeout": 3000
                    }]
                }],
                "BeforeTool": [{
                    "matcher": "*",
                    "sequential": true,
                    "hooks": [{
                        "name": "asb-action-budget",
                        "type": "command",
                        "command": hook_command,
                        "timeout": 3000
                    }]
                }]
            }
        });
        write_private(
            &gemini_home.join("settings.json"),
            &serde_json::to_vec(&settings).map_err(|_| AdapterError::InvalidEvent)?,
        )?;

        let prompt_path = run_root.join("prompt");
        let mut prompt_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&prompt_path)?;
        fs::remove_file(&prompt_path)?;
        prompt_file.write_all(prompt.as_bytes())?;
        prompt_file.seek(SeekFrom::Start(0))?;
        Ok(prompt_file)
    }

    fn spawn_process(
        &self,
        run_root: &Path,
        prompt_file: fs::File,
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
            .args(["--output-format", "stream-json"])
            .args(["--approval-mode", "auto_edit"])
            .arg("--skip-trust")
            .args(["--model", &self.model])
            .current_dir(&self.workspace)
            .stdin(Stdio::from(prompt_file))
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", run_root.join("home"))
            .env("GEMINI_CLI_HOME", run_root.join("home"))
            .env(
                "GEMINI_CLI_SYSTEM_SETTINGS_PATH",
                run_root.join("config/system-settings.json"),
            )
            .env(
                "GEMINI_CLI_SYSTEM_DEFAULTS_PATH",
                run_root.join("config/system-defaults.json"),
            )
            .env("XDG_CONFIG_HOME", run_root.join("config"))
            .env("XDG_DATA_HOME", run_root.join("data"))
            .env("XDG_CACHE_HOME", run_root.join("cache"))
            .env("TMPDIR", run_root.join("tmp"))
            .env("HTTP_PROXY", "http://127.0.0.1:9")
            .env("HTTPS_PROXY", "http://127.0.0.1:9")
            .env("ALL_PROXY", "http://127.0.0.1:9")
            .env("NO_PROXY", no_proxy)
            .env("NODE_USE_ENV_PROXY", "1")
            .env("GEMINI_API_KEY", "asb-credential-free")
            .env("GEMINI_CLI_TRUST_WORKSPACE", "true")
            .env("ASB_MAX_ACTIONS", self.max_actions.to_string())
            .env("ASB_ACTION_COUNTER", run_root.join("action-count"))
            .env("GOOGLE_GEMINI_BASE_URL", self.endpoint.as_str());
        RunningProcess::spawn(command, limits).map_err(AdapterError::Process)
    }

    fn route_run_root(&self, session_id: &Id, attempt_id: &Id) -> PathBuf {
        let mut hasher = Sha256::new();
        hasher.update(b"asb-gemini-route-v1\0");
        hasher.update(Sha256::digest(session_id.0.as_bytes()));
        hasher.update(Sha256::digest(attempt_id.0.as_bytes()));
        self.state_root
            .join(format!("attempt-{:x}", hasher.finalize()))
    }
}

/// A cancellable Gemini process whose output has not yet been collected.
pub struct RunningGemini {
    process: RunningProcess,
    session_id: Id,
    attempt_id: Id,
    model: String,
    max_actions: u32,
    run_root: Option<PathBuf>,
}

impl RunningGemini {
    /// Native process identifier for metrics and ownership evidence.
    #[must_use]
    pub fn pid(&self) -> u32 {
        self.process.pid()
    }

    #[cfg(test)]
    pub(crate) fn action_count_for_test(&self) -> String {
        fs::read_to_string(self.run_root.as_ref().unwrap().join("action-count")).unwrap()
    }

    /// Request idempotent cancellation of the owned process group.
    pub fn cancel(&mut self) -> Result<(), AdapterError> {
        self.process.cancel().map_err(AdapterError::Process)
    }

    /// Reap the process, remove raw state, and map bounded structural evidence.
    pub fn wait(&mut self) -> Result<GeminiOutcome, AdapterError> {
        let waited = self.process.wait().cloned();
        let output = match waited {
            Ok(output) => output,
            Err(error) => {
                let _ = self.remove_run_root();
                return Err(error.into());
            }
        };
        let budget_exhausted = self
            .run_root
            .as_ref()
            .is_some_and(|root| root.join("action-count.exhausted").is_file());
        let process_status = if budget_exhausted {
            TerminalStatus::Failed
        } else {
            match output.termination {
                Termination::Cancelled => TerminalStatus::Cancelled,
                Termination::TimedOut => TerminalStatus::Failed,
                Termination::Exited if output.exit_code == Some(0) => TerminalStatus::Completed,
                Termination::Exited => TerminalStatus::Failed,
            }
        };
        let parsed = if output.stdout.truncated {
            Err(AdapterError::TruncatedOutput)
        } else {
            map_stream(
                &output.stdout.bytes,
                &self.session_id,
                &self.attempt_id,
                &self.model,
                self.max_actions,
                process_status,
            )
        };
        self.remove_run_root()?;
        let (events, status) = parsed?;
        Ok(GeminiOutcome {
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

impl Drop for RunningGemini {
    fn drop(&mut self) {
        let _ = self.process.cancel();
        if self.process.wait().is_ok() {
            let _ = self.remove_run_root();
        }
    }
}

/// Privacy-filtered terminal evidence for one Gemini attempt.
#[derive(Clone, Debug, PartialEq)]
pub struct GeminiOutcome {
    events: Vec<ExtensionEvent>,
    status: TerminalStatus,
    exit_code: Option<i32>,
    stderr_truncated: bool,
}

impl GeminiOutcome {
    /// Ordered ASB lifecycle events without prompt, response, or tool payload content.
    #[must_use]
    pub fn events(&self) -> &[ExtensionEvent] {
        &self.events
    }
    /// Terminal status derived from both process and structured evidence.
    #[must_use]
    pub const fn status(&self) -> TerminalStatus {
        self.status
    }
    /// Native exit code when the process exited normally.
    #[must_use]
    pub const fn exit_code(&self) -> Option<i32> {
        self.exit_code
    }
    /// Whether diagnostic stderr exceeded its retention bound.
    #[must_use]
    pub const fn stderr_truncated(&self) -> bool {
        self.stderr_truncated
    }
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum WireEvent {
    Init {
        timestamp: String,
        session_id: String,
        model: String,
    },
    Message {
        timestamp: String,
        role: WireRole,
        content: String,
        #[serde(default)]
        delta: Option<bool>,
    },
    ToolUse {
        timestamp: String,
        tool_name: String,
        tool_id: String,
        parameters: Value,
    },
    ToolResult {
        timestamp: String,
        tool_id: String,
        status: WireToolStatus,
        #[serde(default)]
        output: Option<String>,
        #[serde(default)]
        error: Option<WireError>,
    },
    Error {
        timestamp: String,
        severity: WireSeverity,
        message: String,
    },
    Result {
        timestamp: String,
        status: WireResultStatus,
        #[serde(default)]
        error: Option<WireError>,
        #[serde(default)]
        stats: Option<WireStats>,
    },
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum WireRole {
    User,
    Assistant,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum WireToolStatus {
    Success,
    Error,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum WireSeverity {
    Warning,
    Error,
}

#[derive(Clone, Copy, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum WireResultStatus {
    Success,
    Error,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireError {
    #[serde(rename = "type")]
    kind: String,
    message: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireStats {
    total_tokens: u64,
    input_tokens: u64,
    output_tokens: u64,
    cached: u64,
    input: u64,
    duration_ms: u64,
    tool_calls: u64,
    models: BTreeMap<String, WireModelStats>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireModelStats {
    total_tokens: u64,
    input_tokens: u64,
    output_tokens: u64,
    cached: u64,
    input: u64,
}

#[allow(clippy::too_many_lines)]
fn map_stream(
    bytes: &[u8],
    session_id: &Id,
    attempt_id: &Id,
    expected_model: &str,
    max_actions: u32,
    process_status: TerminalStatus,
) -> Result<(Vec<ExtensionEvent>, TerminalStatus), AdapterError> {
    let mut events = Vec::new();
    let mut sequence = 0_u64;
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
    if bytes.is_empty() {
        return Err(AdapterError::InvalidEvent);
    }

    let mut initialized = false;
    let mut saw_user = false;
    let mut saw_response = false;
    let mut saw_error = false;
    let mut terminal = false;
    let mut action_count = 0_u32;
    let mut active_tools = BTreeMap::<String, String>::new();
    let mut completed_tools = BTreeSet::<String>::new();
    let mut upstream_count = 0_usize;
    for raw_line in bytes.split(|byte| *byte == b'\n') {
        let line = raw_line.strip_suffix(b"\r").unwrap_or(raw_line);
        if line.is_empty() {
            continue;
        }
        upstream_count = upstream_count
            .checked_add(1)
            .ok_or(AdapterError::InvalidEvent)?;
        if upstream_count > MAX_UPSTREAM_EVENTS || line.len() > MAX_EVENT_LINE_BYTES || terminal {
            return Err(AdapterError::InvalidEvent);
        }
        let value = serde_json::from_slice::<UniqueJson>(line)
            .map_err(|_| AdapterError::InvalidEvent)?
            .0;
        let wire: WireEvent =
            serde_json::from_value(value).map_err(|_| AdapterError::InvalidEvent)?;
        match wire {
            WireEvent::Init {
                timestamp,
                session_id: upstream_session,
                model,
            } => {
                validate_timestamp(&timestamp)?;
                if initialized
                    || upstream_count != 1
                    || !safe_label(&upstream_session, MAX_LABEL_BYTES)
                    || model != expected_model
                {
                    return Err(AdapterError::InvalidEvent);
                }
                initialized = true;
            }
            WireEvent::Message {
                timestamp,
                role,
                content,
                delta,
            } => {
                validate_timestamp(&timestamp)?;
                if !initialized || terminal || content.len() > MAX_EVENT_LINE_BYTES {
                    return Err(AdapterError::InvalidEvent);
                }
                match role {
                    WireRole::User if !saw_user && !saw_response && delta.is_none() => {
                        saw_user = true;
                    }
                    WireRole::Assistant if saw_user && delta == Some(true) => {
                        ensure_first_response(
                            &mut events,
                            &mut sequence,
                            session_id,
                            attempt_id,
                            &mut saw_response,
                        )?;
                    }
                    WireRole::User | WireRole::Assistant => {
                        return Err(AdapterError::InvalidEvent);
                    }
                }
            }
            WireEvent::ToolUse {
                timestamp,
                tool_name,
                tool_id,
                parameters,
            } => {
                validate_timestamp(&timestamp)?;
                if !initialized
                    || !saw_user
                    || !matches!(tool_name.as_str(), "write_file" | "replace")
                    || !safe_label(&tool_name, MAX_LABEL_BYTES)
                    || !safe_label(&tool_id, MAX_LABEL_BYTES)
                    || !parameters.is_object()
                    || active_tools.contains_key(&tool_id)
                    || completed_tools.contains(&tool_id)
                {
                    return Err(AdapterError::InvalidEvent);
                }
                action_count = action_count
                    .checked_add(1)
                    .ok_or(AdapterError::InvalidEvent)?;
                if action_count > max_actions {
                    return Err(AdapterError::InvalidEvent);
                }
                ensure_first_response(
                    &mut events,
                    &mut sequence,
                    session_id,
                    attempt_id,
                    &mut saw_response,
                )?;
                active_tools.insert(tool_id.clone(), tool_name.clone());
                push_event(
                    &mut events,
                    &mut sequence,
                    session_id,
                    attempt_id,
                    Event::ToolStarted {
                        tool_call_id: Id(tool_id),
                        name: tool_name,
                    },
                )?;
            }
            WireEvent::ToolResult {
                timestamp,
                tool_id,
                status,
                output,
                error,
            } => {
                validate_timestamp(&timestamp)?;
                if active_tools.remove(&tool_id).is_none()
                    || !completed_tools.insert(tool_id.clone())
                {
                    return Err(AdapterError::InvalidEvent);
                }
                let success = match status {
                    WireToolStatus::Success if error.is_none() => true,
                    WireToolStatus::Error if error.as_ref().is_some_and(valid_wire_error) => false,
                    WireToolStatus::Success | WireToolStatus::Error => {
                        return Err(AdapterError::InvalidEvent);
                    }
                };
                if output
                    .as_ref()
                    .is_some_and(|value| value.len() > MAX_EVENT_LINE_BYTES)
                {
                    return Err(AdapterError::InvalidEvent);
                }
                push_event(
                    &mut events,
                    &mut sequence,
                    session_id,
                    attempt_id,
                    Event::ToolFinished {
                        tool_call_id: Id(tool_id),
                        success,
                    },
                )?;
                saw_error |= !success;
            }
            WireEvent::Error {
                timestamp,
                severity,
                message,
            } => {
                validate_timestamp(&timestamp)?;
                if !initialized || !safe_label(&message, MAX_EVENT_LINE_BYTES) {
                    return Err(AdapterError::InvalidEvent);
                }
                let _ = severity;
                saw_error = true;
            }
            WireEvent::Result {
                timestamp,
                status,
                error,
                stats,
            } => {
                validate_timestamp(&timestamp)?;
                if !initialized || !saw_user || !active_tools.is_empty() {
                    return Err(AdapterError::InvalidEvent);
                }
                let stats = stats.ok_or(AdapterError::InvalidEvent)?;
                if stats.tool_calls != u64::from(action_count)
                    || stats.total_tokens
                        != stats
                            .input_tokens
                            .checked_add(stats.output_tokens)
                            .ok_or(AdapterError::InvalidEvent)?
                    || stats.cached.checked_add(stats.input) != Some(stats.input_tokens)
                    || stats.duration_ms == 0
                    || !model_totals_match(&stats)
                    || (status == WireResultStatus::Success
                        && (stats.input_tokens == 0 || stats.output_tokens == 0))
                    || stats.models.iter().any(|(model, values)| {
                        !safe_label(model, MAX_MODEL_BYTES)
                            || values.input_tokens.checked_add(values.output_tokens)
                                != Some(values.total_tokens)
                            || values.cached.checked_add(values.input) != Some(values.input_tokens)
                    })
                {
                    return Err(AdapterError::InvalidEvent);
                }
                if error.as_ref().is_some_and(|value| !valid_wire_error(value)) {
                    return Err(AdapterError::InvalidEvent);
                }
                push_event(
                    &mut events,
                    &mut sequence,
                    session_id,
                    attempt_id,
                    Event::Usage(Usage {
                        input_tokens: Some(stats.input_tokens),
                        output_tokens: Some(stats.output_tokens),
                        cost_micros: None,
                        currency: None,
                    }),
                )?;
                let success = status == WireResultStatus::Success
                    && error.is_none()
                    && !saw_error
                    && saw_response;
                if success {
                    push_event(
                        &mut events,
                        &mut sequence,
                        session_id,
                        attempt_id,
                        Event::Completed,
                    )?;
                } else {
                    push_failure(&mut events, &mut sequence, session_id, attempt_id)?;
                }
                terminal = true;
            }
        }
    }
    if !terminal {
        return Err(AdapterError::InvalidEvent);
    }
    let status = if matches!(
        events.last().map(|event| &event.event),
        Some(Event::Completed)
    ) {
        TerminalStatus::Completed
    } else {
        TerminalStatus::Failed
    };
    Ok((events, status))
}

fn model_totals_match(stats: &WireStats) -> bool {
    let mut total_tokens = 0_u64;
    let mut input_tokens = 0_u64;
    let mut output_tokens = 0_u64;
    let mut cached = 0_u64;
    let mut input = 0_u64;
    for model in stats.models.values() {
        let Some(next_total) = total_tokens.checked_add(model.total_tokens) else {
            return false;
        };
        let Some(next_input_tokens) = input_tokens.checked_add(model.input_tokens) else {
            return false;
        };
        let Some(next_output_tokens) = output_tokens.checked_add(model.output_tokens) else {
            return false;
        };
        let Some(next_cached) = cached.checked_add(model.cached) else {
            return false;
        };
        let Some(next_input) = input.checked_add(model.input) else {
            return false;
        };
        total_tokens = next_total;
        input_tokens = next_input_tokens;
        output_tokens = next_output_tokens;
        cached = next_cached;
        input = next_input;
    }
    total_tokens == stats.total_tokens
        && input_tokens == stats.input_tokens
        && output_tokens == stats.output_tokens
        && cached == stats.cached
        && input == stats.input
}

fn ensure_first_response(
    events: &mut Vec<ExtensionEvent>,
    sequence: &mut u64,
    session_id: &Id,
    attempt_id: &Id,
    saw_response: &mut bool,
) -> Result<(), AdapterError> {
    if !*saw_response {
        push_event(
            events,
            sequence,
            session_id,
            attempt_id,
            Event::FirstResponse,
        )?;
        *saw_response = true;
    }
    Ok(())
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
            message: "Gemini attempt failed; raw diagnostics are not retained".into(),
            data: None,
        }),
    )
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

fn valid_wire_error(error: &WireError) -> bool {
    safe_label(&error.kind, MAX_LABEL_BYTES) && safe_label(&error.message, MAX_EVENT_LINE_BYTES)
}

fn validate_timestamp(timestamp: &str) -> Result<(), AdapterError> {
    if safe_label(timestamp, 64)
        && timestamp.bytes().all(|byte| {
            byte.is_ascii_digit() || matches!(byte, b'-' | b':' | b'.' | b'+' | b'T' | b'Z')
        })
    {
        Ok(())
    } else {
        Err(AdapterError::InvalidEvent)
    }
}

fn safe_label(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && value.bytes().all(|byte| !byte.is_ascii_control())
}

fn valid_public_id(id: &Id) -> bool {
    !id.0.is_empty()
        && id.0.len() <= MAX_PUBLIC_ID_BYTES
        && id
            .0
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn loopback_host(host: &str) -> bool {
    matches!(
        host.trim_end_matches('.').to_ascii_lowercase().as_str(),
        "127.0.0.1" | "::1" | "[::1]" | "localhost"
    )
}

fn validate_root_topology(path: &Path) -> Result<PathBuf, AdapterError> {
    let mut existing = PathBuf::from("/");
    let mut missing = Vec::new();
    let mut saw_missing = false;
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::CurDir => {}
            Component::Normal(name) if saw_missing => missing.push(name.to_owned()),
            Component::Normal(name) => {
                let candidate = existing.join(name);
                match fs::symlink_metadata(&candidate) {
                    Ok(metadata) => {
                        if metadata.file_type().is_symlink() || !metadata.is_dir() {
                            return Err(AdapterError::UnsafeRootTopology);
                        }
                        existing = candidate;
                    }
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        missing.push(name.to_owned());
                        saw_missing = true;
                    }
                    Err(error) => return Err(error.into()),
                }
            }
            Component::ParentDir | Component::Prefix(_) => {
                return Err(AdapterError::UnsafeRootTopology);
            }
        }
    }
    let mut canonical = fs::canonicalize(existing)?;
    for component in missing {
        canonical.push(component);
    }
    Ok(canonical)
}

fn roots_overlap(left: &Path, right: &Path) -> bool {
    left == right || left.starts_with(right) || right.starts_with(left)
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

fn digest_bundle_tree(root: &Path, expected_count: usize) -> Result<String, AdapterError> {
    let mut pending = vec![root.to_owned()];
    let mut files = Vec::<(String, PathBuf)>::new();
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.is_dir() {
                pending.push(path);
            } else if metadata.is_file() {
                let relative = path
                    .strip_prefix(root)
                    .map_err(|_| AdapterError::ExecutableMismatch)?
                    .to_str()
                    .ok_or(AdapterError::ExecutableMismatch)?
                    .replace('\\', "/");
                files.push((relative, path));
            } else {
                return Err(AdapterError::ExecutableMismatch);
            }
        }
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    if files.len() != expected_count {
        return Err(AdapterError::ExecutableMismatch);
    }
    let mut hasher = Sha256::new();
    for (relative, path) in files {
        hasher.update(digest_file(&path)?.as_bytes());
        hasher.update(b"  ");
        hasher.update(relative.as_bytes());
        hasher.update(b"\n");
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn wait_for_hook_ready(run_root: &Path) -> bool {
    wait_for_hook_ready_until(run_root, Instant::now() + HOOK_READY_TIMEOUT)
}

fn wait_for_hook_ready_until(run_root: &Path, deadline: Instant) -> bool {
    let marker = run_root.join("action-count.ready");
    loop {
        match fs::read(&marker) {
            Ok(contents) => return contents == b"ready\n",
            Err(error) if error.kind() == io::ErrorKind::NotFound && Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(5));
            }
            Err(_) => return false,
        }
    }
}

fn write_private(path: &Path, contents: &[u8]) -> Result<(), AdapterError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(contents)?;
    file.sync_all()?;
    Ok(())
}

fn shell_quote(value: &str) -> String {
    let mut quoted = String::from("'");
    for character in value.chars() {
        if character == '\'' {
            quoted.push_str("'\\''");
        } else {
            quoted.push(character);
        }
    }
    quoted.push('\'');
    quoted
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::time::Duration;

    struct Scratch(PathBuf);
    impl Scratch {
        fn new(name: &str) -> Self {
            let nonce = RUN_NONCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("asb-gemini-{name}-{}-{nonce}", std::process::id()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn config(root: &Path) -> GeminiConfig {
        GeminiConfig::new(
            "/bin/true",
            "/bin/true",
            root.join("work"),
            root.join("state"),
            Url::parse("http://127.0.0.1:1").unwrap(),
            "fixture-model",
            4,
            4,
            GeminiArtifact::LinuxX86_64V0_58_0,
        )
        .unwrap()
    }

    fn stream(tool_status: &str, result_status: &str) -> Vec<u8> {
        format!(
            "{{\"type\":\"init\",\"timestamp\":\"2026-01-01T00:00:00Z\",\"session_id\":\"upstream\",\"model\":\"fixture-model\"}}\n\
             {{\"type\":\"message\",\"timestamp\":\"2026-01-01T00:00:00Z\",\"role\":\"user\",\"content\":\"private prompt\"}}\n\
             {{\"type\":\"tool_use\",\"timestamp\":\"2026-01-01T00:00:01Z\",\"tool_name\":\"write_file\",\"tool_id\":\"call-1\",\"parameters\":{{\"content\":\"private payload\"}}}}\n\
             {{\"type\":\"tool_result\",\"timestamp\":\"2026-01-01T00:00:02Z\",\"tool_id\":\"call-1\",\"status\":\"{tool_status}\",\"output\":\"private output\"}}\n\
             {{\"type\":\"message\",\"timestamp\":\"2026-01-01T00:00:03Z\",\"role\":\"assistant\",\"content\":\"private response\",\"delta\":true}}\n\
             {{\"type\":\"result\",\"timestamp\":\"2026-01-01T00:00:04Z\",\"status\":\"{result_status}\",\"stats\":{{\"total_tokens\":9,\"input_tokens\":7,\"output_tokens\":2,\"cached\":1,\"input\":6,\"duration_ms\":4,\"tool_calls\":1,\"models\":{{\"fixture-model\":{{\"total_tokens\":9,\"input_tokens\":7,\"output_tokens\":2,\"cached\":1,\"input\":6}}}}}}}}\n"
        )
        .into_bytes()
    }

    #[test]
    fn configuration_manifest_and_provenance_are_bounded() {
        let scratch = Scratch::new("config");
        for endpoint in [
            "ftp://example.invalid",
            "http://example.invalid",
            "https://user@example.invalid",
            "https://example.invalid?secret=x",
        ] {
            assert!(matches!(
                GeminiConfig::new(
                    "/bin/true",
                    "/bin/true",
                    &scratch.0,
                    &scratch.0,
                    Url::parse(endpoint).unwrap(),
                    "model",
                    1,
                    1,
                    GeminiArtifact::LinuxX86_64V0_58_0,
                ),
                Err(AdapterError::InvalidEndpoint)
            ));
        }
        assert!(matches!(
            GeminiConfig::new(
                "relative",
                "/bin/true",
                &scratch.0,
                &scratch.0,
                Url::parse("https://example.invalid").unwrap(),
                "model",
                1,
                1,
                GeminiArtifact::LinuxX86_64V0_58_0,
            ),
            Err(AdapterError::RelativePath("binary"))
        ));
        assert!(matches!(
            GeminiConfig::new(
                "/bin/true",
                "/bin/true",
                &scratch.0,
                &scratch.0,
                Url::parse("https://example.invalid").unwrap(),
                "bad/model",
                1,
                1,
                GeminiArtifact::LinuxX86_64V0_58_0,
            ),
            Err(AdapterError::InvalidModel)
        ));
        for (turns, actions) in [(0, 1), (1, 0), (MAX_TURNS + 1, 1), (1, MAX_ACTIONS + 1)] {
            assert!(matches!(
                GeminiConfig::new(
                    "/bin/true",
                    "/bin/true",
                    &scratch.0,
                    &scratch.0,
                    Url::parse("https://example.invalid").unwrap(),
                    "model",
                    turns,
                    actions,
                    GeminiArtifact::LinuxX86_64V0_58_0,
                ),
                Err(AdapterError::InvalidBudget)
            ));
        }
        let manifest = config(&scratch.0).manifest();
        assert_eq!(manifest.extension_id.0, "agent.gemini");
        assert_eq!(manifest.executable_sha256, TESTED_ENTRY_SHA256);
        assert!(manifest.capabilities.contains(&Capability::Usage));
        assert_eq!(NPM_TARBALL_SHA256.len(), 64);
        assert!(NPM_TARBALL_INTEGRITY.starts_with("sha512-"));
        assert_eq!(UPSTREAM_REVISION.len(), 40);
        assert_eq!(UPSTREAM_TREE.len(), 40);
        let rendered = [
            AdapterError::RelativePath("fixture").to_string(),
            AdapterError::InvalidPathEncoding.to_string(),
            AdapterError::InvalidEndpoint.to_string(),
            AdapterError::InvalidModel.to_string(),
            AdapterError::InvalidIdentity.to_string(),
            AdapterError::InvalidBudget.to_string(),
            AdapterError::PromptTooLarge.to_string(),
            AdapterError::InvalidPrompt.to_string(),
            AdapterError::AmbientConfiguration.to_string(),
            AdapterError::UnsafeRootTopology.to_string(),
            AdapterError::AmbientSystemConfiguration.to_string(),
            AdapterError::HookUnavailable.to_string(),
            AdapterError::Io(io::Error::other("fixture")).to_string(),
            AdapterError::ExecutableMismatch.to_string(),
            AdapterError::RuntimeMismatch.to_string(),
            AdapterError::TruncatedOutput.to_string(),
            AdapterError::InvalidEvent.to_string(),
        ];
        assert!(rendered.iter().all(|message| !message.is_empty()));
        assert!(matches!(
            config(&scratch.0).verify_executable(),
            Err(AdapterError::ExecutableMismatch)
        ));
    }

    #[test]
    fn stream_maps_only_structural_causal_evidence() {
        for line in stream("success", "success")
            .split(|byte| *byte == 10)
            .filter(|line| !line.is_empty())
        {
            let value = serde_json::from_slice::<UniqueJson>(line).unwrap().0;
            assert!(
                serde_json::from_value::<WireEvent>(value).is_ok(),
                "fixture event must match the inspected dialect"
            );
        }
        let (events, status) = map_stream(
            &stream("success", "success"),
            &Id("session".into()),
            &Id("attempt".into()),
            "fixture-model",
            1,
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
        assert!(matches!(events[3].event, Event::ToolStarted { .. }));
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
            "private prompt",
            "private payload",
            "private output",
            "private response",
        ] {
            assert!(!encoded.contains(private));
        }
    }

    #[test]
    fn malformed_unknown_and_inconsistent_streams_fail_closed() {
        let session = Id("s".into());
        let attempt = Id("a".into());
        for bytes in [
            b"not-json\n".as_slice(),
            br#"{"type":"init","type":"result"}"#,
            br#"{"type":"new_event","timestamp":"2026-01-01T00:00:00Z"}"#,
            br#"{"type":"init","timestamp":"bad timestamp","session_id":"x","model":"fixture-model"}"#,
        ] {
            assert!(matches!(
                map_stream(bytes, &session, &attempt, "fixture-model", 1, TerminalStatus::Completed),
                Err(AdapterError::InvalidEvent)
            ));
        }
        assert!(matches!(
            map_stream(
                &stream("success", "success"),
                &session,
                &attempt,
                "other",
                1,
                TerminalStatus::Completed
            ),
            Err(AdapterError::InvalidEvent)
        ));
        for (needle, replacement) in [
            (
                "\"tool_name\":\"write_file\"",
                "\"tool_name\":\"run_shell_command\"",
            ),
            ("\"tool_calls\":1", "\"tool_calls\":2"),
            (
                "\"total_tokens\":9,\"input_tokens\":7",
                "\"total_tokens\":8,\"input_tokens\":7",
            ),
            (
                "\"models\":{\"fixture-model\":{\"total_tokens\":9,\"input_tokens\":7,\"output_tokens\":2,\"cached\":1,\"input\":6}}",
                "\"models\":{\"fixture-model\":{\"total_tokens\":8,\"input_tokens\":6,\"output_tokens\":2,\"cached\":1,\"input\":5}}",
            ),
        ] {
            let altered = String::from_utf8(stream("success", "success"))
                .unwrap()
                .replacen(needle, replacement, 1);
            assert!(matches!(
                map_stream(
                    altered.as_bytes(),
                    &session,
                    &attempt,
                    "fixture-model",
                    1,
                    TerminalStatus::Completed
                ),
                Err(AdapterError::InvalidEvent)
            ));
        }
        let zero_usage = String::from_utf8(stream("success", "success"))
            .unwrap()
            .replace(
                "\"total_tokens\":9,\"input_tokens\":7,\"output_tokens\":2,\"cached\":1,\"input\":6,\"duration_ms\":4,\"tool_calls\":1,\"models\":{\"fixture-model\":{\"total_tokens\":9,\"input_tokens\":7,\"output_tokens\":2,\"cached\":1,\"input\":6}}",
                "\"total_tokens\":0,\"input_tokens\":0,\"output_tokens\":0,\"cached\":0,\"input\":0,\"duration_ms\":4,\"tool_calls\":1,\"models\":{\"fixture-model\":{\"total_tokens\":0,\"input_tokens\":0,\"output_tokens\":0,\"cached\":0,\"input\":0}}",
            );
        assert!(matches!(
            map_stream(
                zero_usage.as_bytes(),
                &session,
                &attempt,
                "fixture-model",
                1,
                TerminalStatus::Completed
            ),
            Err(AdapterError::InvalidEvent)
        ));
        let mut trailing = stream("success", "success");
        trailing.extend_from_slice(
            b"{\"type\":\"message\",\"timestamp\":\"2026-01-01T00:00:05Z\",\"role\":\"assistant\",\"content\":\"discard\",\"delta\":true}\n",
        );
        assert!(matches!(
            map_stream(
                &trailing,
                &session,
                &attempt,
                "fixture-model",
                1,
                TerminalStatus::Completed
            ),
            Err(AdapterError::InvalidEvent)
        ));
        assert!(matches!(
            map_stream(
                &vec![b' '; MAX_EVENT_LINE_BYTES + 1],
                &session,
                &attempt,
                "fixture-model",
                1,
                TerminalStatus::Completed
            ),
            Err(AdapterError::InvalidEvent)
        ));
        assert_eq!(shell_quote("/tmp/a'b"), "'/tmp/a'\\''b'");
        let scalars = serde_json::from_slice::<UniqueJson>(
            br#"[true,-1,1,1.5,"fixture",null,{"nested":false}]"#,
        )
        .unwrap()
        .0;
        assert!(scalars.is_array());
        assert!(matches!(
            map_stream(
                &stream("success", "success"),
                &session,
                &attempt,
                "fixture-model",
                0,
                TerminalStatus::Completed
            ),
            Err(AdapterError::InvalidEvent)
        ));
    }

    #[test]
    fn failures_cancellation_and_truncation_are_explicit() {
        let session = Id("s".into());
        let attempt = Id("a".into());
        let (cancelled, status) = map_stream(
            b"",
            &session,
            &attempt,
            "fixture-model",
            1,
            TerminalStatus::Cancelled,
        )
        .unwrap();
        assert_eq!(status, TerminalStatus::Cancelled);
        assert_eq!(cancelled.len(), 2);
        let (failed, status) = map_stream(
            b"",
            &session,
            &attempt,
            "fixture-model",
            1,
            TerminalStatus::Failed,
        )
        .unwrap();
        assert_eq!(status, TerminalStatus::Failed);
        assert!(matches!(failed.last().unwrap().event, Event::Failed(_)));

        let mut tool_failure = stream("success", "success");
        let position = tool_failure
            .windows(b"\"status\":\"success\",\"output\"".len())
            .position(|window| window == b"\"status\":\"success\",\"output\"")
            .unwrap();
        tool_failure.splice(
            position..position + b"\"status\":\"success\"".len(),
            b"\"status\":\"error\"".iter().copied(),
        );
        assert!(matches!(
            map_stream(
                &tool_failure,
                &session,
                &attempt,
                "fixture-model",
                1,
                TerminalStatus::Completed
            ),
            Err(AdapterError::InvalidEvent)
        ));
    }

    fn executable_adapter(script: &str) -> (Scratch, GeminiConfig) {
        let scratch = Scratch::new("process");
        let binary = scratch.0.join("gemini.js");
        let node = scratch.0.join("node");
        fs::write(&binary, script).unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(
            &node,
            "#!/bin/sh\n[ \"$1\" = --use-env-proxy ] || exit 97\nshift\nprintf 'ready\\n' > \"$ASB_ACTION_COUNTER.ready\"\nexec /bin/sh \"$@\"\n",
        )
        .unwrap();
        fs::set_permissions(&node, fs::Permissions::from_mode(0o700)).unwrap();
        let mut adapter = GeminiConfig::new(
            &binary,
            &node,
            scratch.0.join("work"),
            scratch.0.join("state"),
            Url::parse("http://127.0.0.1:1").unwrap(),
            "fixture-model",
            4,
            4,
            GeminiArtifact::LinuxX86_64V0_58_0,
        )
        .unwrap();
        adapter.verification_digest_override = Some(digest_file(&binary).unwrap());
        adapter.runtime_digest_override = Some(digest_file(&node).unwrap());
        adapter.bundle_file_count_override = Some(2);
        adapter.bundle_digest_override = Some(digest_bundle_tree(&scratch.0, 2).unwrap());
        (scratch, adapter)
    }

    fn limits() -> ProcessLimits {
        ProcessLimits::new(
            256 * 1024,
            64 * 1024,
            Duration::from_secs(5),
            Duration::from_millis(50),
            Duration::from_millis(2),
        )
        .unwrap()
    }

    #[test]
    fn process_boundary_hides_prompt_isolates_state_and_cleans_up() {
        let script = r#"#!/bin/sh
case " $* " in *" private-prompt "*) exit 91;; esac
[ "$HOME" != "/root" ] || exit 92
[ "$GEMINI_API_KEY" = asb-credential-free ] || exit 98
case "$GEMINI_CLI_SYSTEM_SETTINGS_PATH" in "$XDG_CONFIG_HOME"/*) ;; *) exit 99;; esac
case "$GEMINI_CLI_SYSTEM_DEFAULTS_PATH" in "$XDG_CONFIG_HOME"/*) ;; *) exit 100;; esac
[ "$(cat "$GEMINI_CLI_SYSTEM_SETTINGS_PATH")" = '{}' ] || exit 101
[ "$(cat "$GEMINI_CLI_SYSTEM_DEFAULTS_PATH")" = '{}' ] || exit 102
printf 'ready\n' > "$ASB_ACTION_COUNTER.ready"
[ "$ASB_MAX_ACTIONS" = 4 ] || exit 94
[ "$(cat "$ASB_ACTION_COUNTER")" = 0 ] || exit 95
grep -q asb-action-budget "$HOME/.gemini/settings.json" || exit 96
grep -Fq 'toolName = "*"' "$HOME/.gemini/policies/asb.toml" || exit 97
prompt=$(cat)
[ "$prompt" = private-prompt ] || exit 93
printf '%s\n' \
'{"type":"init","timestamp":"2026-01-01T00:00:00Z","session_id":"upstream","model":"fixture-model"}' \
'{"type":"message","timestamp":"2026-01-01T00:00:00Z","role":"user","content":"discard"}' \
'{"type":"message","timestamp":"2026-01-01T00:00:01Z","role":"assistant","content":"discard","delta":true}' \
'{"type":"result","timestamp":"2026-01-01T00:00:02Z","status":"success","stats":{"total_tokens":2,"input_tokens":1,"output_tokens":1,"cached":0,"input":1,"duration_ms":1,"tool_calls":0,"models":{"fixture-model":{"total_tokens":2,"input_tokens":1,"output_tokens":1,"cached":0,"input":1}}}}'
"#;
        let (scratch, adapter) = executable_adapter(script);
        let mut running = adapter
            .start(Id("s".into()), Id("a".into()), "private-prompt", limits())
            .unwrap();
        assert!(running.pid() > 0);
        assert_eq!(running.action_count_for_test(), "0\n");
        let outcome = running.wait().unwrap();
        assert_eq!(outcome.status(), TerminalStatus::Completed);
        assert_eq!(outcome.exit_code(), Some(0));
        assert!(!outcome.stderr_truncated());
        assert!(matches!(
            outcome.events().last().unwrap().event,
            Event::Completed
        ));
        assert!(
            fs::read_dir(scratch.0.join("state"))
                .unwrap()
                .next()
                .is_none()
        );
    }

    #[test]
    fn bundle_tree_additions_and_symlinks_fail_before_spawn() {
        let (scratch, adapter) = executable_adapter("#!/bin/sh\nexit 88\n");
        assert!(adapter.verify_executable().is_ok());
        let extra = scratch.0.join("unexpected");
        fs::write(&extra, b"unexpected").unwrap();
        assert!(matches!(
            adapter.verify_executable(),
            Err(AdapterError::ExecutableMismatch)
        ));
        fs::remove_file(&extra).unwrap();
        symlink("gemini.js", &extra).unwrap();
        assert!(matches!(
            adapter.verify_executable(),
            Err(AdapterError::ExecutableMismatch)
        ));
    }

    #[test]
    fn hook_readiness_is_exact_and_bounded() {
        let scratch = Scratch::new("hook-ready");
        assert!(!wait_for_hook_ready_until(&scratch.0, Instant::now()));
        fs::write(scratch.0.join("action-count.ready"), b"not-ready\n").unwrap();
        assert!(!wait_for_hook_ready_until(
            &scratch.0,
            Instant::now() + Duration::from_secs(1)
        ));
    }

    #[test]
    fn ambient_config_prompt_limits_and_cancellation_fail_closed() {
        let (scratch, adapter) = executable_adapter("#!/bin/sh\nwhile :; do sleep 1; done\n");
        fs::create_dir_all(scratch.0.join("work/.gemini")).unwrap();
        assert!(matches!(
            adapter.start(Id("s".into()), Id("a".into()), "x", limits()),
            Err(AdapterError::AmbientConfiguration)
        ));
        fs::remove_dir_all(scratch.0.join("work/.gemini")).unwrap();
        assert!(matches!(
            adapter.start(
                Id("s".into()),
                Id("a".into()),
                &"x".repeat(MAX_PROMPT_BYTES + 1),
                limits()
            ),
            Err(AdapterError::PromptTooLarge)
        ));
        let mut running = adapter
            .start(Id("s".into()), Id("a".into()), "x", limits())
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
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        );
        assert!(!name.contains(&session.0));
        assert!(!name.contains(&attempt.0));
    }

    #[test]
    fn public_identities_fail_before_filesystem_or_process_effects() {
        let scratch = Scratch::new("invalid-identity");
        let adapter = config(&scratch.0);
        for id in [
            "",
            "bad\nidentity",
            "bad identity",
            "../attempt",
            "run/attempt",
            "run:attempt",
            "run@attempt",
            "unicod\u{e9}",
        ] {
            assert!(matches!(
                adapter.start(Id(id.into()), Id("attempt".into()), "prompt", limits()),
                Err(AdapterError::InvalidIdentity)
            ));
            assert!(matches!(
                adapter.start(Id("session".into()), Id(id.into()), "prompt", limits()),
                Err(AdapterError::InvalidIdentity)
            ));
        }
        assert!(matches!(
            adapter.start(
                Id("x".repeat(MAX_PUBLIC_ID_BYTES + 1)),
                Id("attempt".into()),
                "prompt",
                limits()
            ),
            Err(AdapterError::InvalidIdentity)
        ));
        assert!(!scratch.0.join("workspace").exists());
        assert!(!scratch.0.join("state").exists());
    }

    #[test]
    fn duplicate_and_stale_route_ownership_fail_closed_then_cleanup_allows_reuse() {
        let script = "#!/bin/sh\ntrap \"exit 0\" TERM\nsleep 60\n";
        let (scratch, mut adapter) = executable_adapter(script);
        let bundle = scratch.0.join("bundle");
        fs::create_dir(&bundle).unwrap();
        let binary = bundle.join("gemini.js");
        fs::rename(&adapter.binary, &binary).unwrap();
        adapter.binary = binary;
        adapter.bundle_file_count_override = Some(1);
        adapter.bundle_digest_override = Some(digest_bundle_tree(&bundle, 1).unwrap());
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
    fn unsafe_and_overlapping_roots_fail_before_mutation() {
        let (scratch, mut adapter) = executable_adapter("#!/bin/sh\nexit 89\n");
        let outside = scratch.0.join("outside");
        fs::create_dir(&outside).unwrap();

        let workspace_link = scratch.0.join("workspace-link");
        symlink(&outside, &workspace_link).unwrap();
        adapter.workspace = workspace_link;
        adapter.state_root = scratch.0.join("state-from-workspace-link");
        assert!(matches!(
            adapter.start(Id("s".into()), Id("a".into()), "x", limits()),
            Err(AdapterError::UnsafeRootTopology)
        ));
        assert!(!adapter.state_root.exists());

        fs::remove_file(&adapter.workspace).unwrap();
        adapter.workspace = scratch.0.join("fresh-workspace");
        let state_link = scratch.0.join("state-link");
        symlink(&outside, &state_link).unwrap();
        adapter.state_root = state_link;
        assert!(matches!(
            adapter.start(Id("s".into()), Id("a".into()), "x", limits()),
            Err(AdapterError::UnsafeRootTopology)
        ));
        assert!(!adapter.workspace.exists());

        fs::remove_file(&adapter.state_root).unwrap();
        let ancestor_link = scratch.0.join("ancestor-link");
        symlink(&outside, &ancestor_link).unwrap();
        adapter.workspace = ancestor_link.join("nested-workspace");
        adapter.state_root = scratch.0.join("state-from-ancestor-link");
        assert!(matches!(
            adapter.start(Id("s".into()), Id("a".into()), "x", limits()),
            Err(AdapterError::UnsafeRootTopology)
        ));
        assert!(!outside.join("nested-workspace").exists());
        assert!(!adapter.state_root.exists());
        fs::remove_file(ancestor_link).unwrap();

        adapter.workspace = scratch.0.join("overlap");
        adapter.state_root = adapter.workspace.join("private-state");
        assert!(matches!(
            adapter.start(Id("s".into()), Id("a".into()), "x", limits()),
            Err(AdapterError::UnsafeRootTopology)
        ));
        assert!(!adapter.workspace.exists());
        assert!(!adapter.state_root.exists());
    }

    #[test]
    fn ambient_system_configuration_fails_before_process_or_state_mutation() {
        for relative in ["settings.json", "system-defaults.json", "policies"] {
            let (scratch, mut adapter) = executable_adapter("#!/bin/sh\nexit 90\n");
            let system_root = scratch.0.join("system-config");
            fs::create_dir(&system_root).unwrap();
            let ambient = system_root.join(relative);
            if relative == "policies" {
                fs::create_dir(&ambient).unwrap();
            } else {
                fs::write(&ambient, b"{}\n").unwrap();
            }
            adapter.system_config_root_override = Some(system_root);
            assert!(matches!(
                adapter.start(Id("s".into()), Id("a".into()), "x", limits()),
                Err(AdapterError::AmbientSystemConfiguration)
            ));
            assert!(!adapter.workspace.exists());
            assert!(!adapter.state_root.exists());
        }
    }
}
