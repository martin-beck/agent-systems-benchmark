// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
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
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Seek, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
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
/// SHA-256 of the bounded, path-framed installed dependency tree exercised natively.
pub const TESTED_PYTHON_ENVIRONMENT_SHA256: &str =
    "e67bf3c72123ab751ddf632d62321189b4b62191289d443e6ff02cbeed2d6aa5";
/// Largest accepted prompt, in UTF-8 bytes.
pub const MAX_PROMPT_BYTES: usize = 4 * 1024 * 1024;
const MAX_PUBLIC_ID_BYTES: usize = 4 * 1024;
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
const MAX_ENVIRONMENT_ENTRIES: usize = 32_768;
const MAX_ENVIRONMENT_FILE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_ENVIRONMENT_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_METADATA_BYTES: u64 = 1024 * 1024;
const SUBMISSION_COMMAND: &str = "echo COMPLETE_TASK_AND_SUBMIT_FINAL_OUTPUT";
const CLOSED_PROXY: &str = "http://127.0.0.1:9";
const KNOWN_MINI_SWE_EGRESS: &[&str] = &[
    "mini-swe-agent.com",
    "api.github.com",
    "openrouter.ai",
    "pypi.org",
    "raw.githubusercontent.com",
    "us.i.posthog.com",
];

const TESTED_DISTRIBUTIONS: &[(&str, &str)] = &[
    ("aiohappyeyeballs", "2.7.1"),
    ("aiohttp", "3.14.3"),
    ("aiosignal", "1.4.0"),
    ("annotated-doc", "0.0.5"),
    ("annotated-types", "0.8.0"),
    ("anyio", "4.15.1"),
    ("attrs", "26.1.0"),
    ("boto3", "1.43.89"),
    ("botocore", "1.43.89"),
    ("certifi", "2026.7.22"),
    ("charset-normalizer", "3.5.1"),
    ("click", "8.5.0"),
    ("datasets", "5.0.1"),
    ("dill", "0.4.1"),
    ("distro", "1.9.0"),
    ("fastuuid", "0.14.0"),
    ("filelock", "3.32.5"),
    ("frozenlist", "1.8.0"),
    ("fsspec", "2026.6.0"),
    ("h11", "0.16.0"),
    ("hf-xet", "1.6.0"),
    ("httpcore", "1.0.9"),
    ("httpx", "0.28.1"),
    ("huggingface-hub", "1.30.0"),
    ("idna", "3.19"),
    ("importlib-metadata", "8.9.0"),
    ("jinja2", "3.1.6"),
    ("jiter", "0.16.0"),
    ("jmespath", "1.1.0"),
    ("jsonschema", "4.26.0"),
    ("jsonschema-specifications", "2025.9.1"),
    ("linkify-it-py", "2.2.0"),
    ("litellm", "1.100.0"),
    ("markdown-it-py", "4.2.0"),
    ("markupsafe", "3.0.3"),
    ("mdit-py-plugins", "0.6.1"),
    ("mdurl", "0.1.2"),
    ("mini-swe-agent", "2.4.6"),
    ("multidict", "6.7.1"),
    ("multiprocess", "0.70.19"),
    ("numpy", "2.5.3"),
    ("openai", "2.54.0"),
    ("packaging", "26.3"),
    ("pandas", "3.0.5"),
    ("pip", "24.0"),
    ("platformdirs", "4.11.7"),
    ("prompt-toolkit", "3.0.53"),
    ("propcache", "0.5.2"),
    ("pyarrow", "25.0.1"),
    ("pydantic", "2.13.5"),
    ("pydantic-core", "2.46.5"),
    ("pydantic-settings", "2.15.0"),
    ("pygments", "2.21.0"),
    ("python-dateutil", "2.9.0.post0"),
    ("python-dotenv", "1.2.3"),
    ("pyyaml", "6.0.3"),
    ("referencing", "0.37.0"),
    ("regex", "2026.9.3"),
    ("requests", "2.34.2"),
    ("rich", "15.0.0"),
    ("rpds-py", "2026.6.3"),
    ("s3transfer", "0.19.2"),
    ("shellingham", "1.5.4"),
    ("six", "1.17.0"),
    ("sniffio", "1.3.1"),
    ("tenacity", "9.1.4"),
    ("textual", "8.2.8"),
    ("tiktoken", "0.14.0"),
    ("tokenizers", "0.23.2"),
    ("tqdm", "4.70.0"),
    ("typer", "0.27.2"),
    ("typing-extensions", "4.16.0"),
    ("typing-inspection", "0.4.4"),
    ("urllib3", "2.7.0"),
    ("wcwidth", "0.8.3"),
    ("xxhash", "4.0.1"),
    ("yarl", "1.24.5"),
    ("zipp", "4.1.0"),
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
    #[cfg(test)]
    environment_digest_override: Option<String>,
    #[cfg(test)]
    distribution_override: Option<Vec<(String, String)>>,
}

struct PreparedLaunch {
    python: PathBuf,
    wheel: PathBuf,
    environment: PathBuf,
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
    /// A public correlation identity was empty, excessive, or contained control bytes.
    InvalidIdentity,
    /// Prompt input exceeded the documented byte ceiling.
    PromptTooLarge,
    /// Local preparation or digest inspection failed.
    Io(io::Error),
    /// The mini_swe wheel did not match its content pin.
    WheelMismatch,
    /// The required Python runtime did not match its content pin.
    RuntimeMismatch,
    /// The installed Python dependency graph or content did not match the product pin.
    DependencyMismatch,
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
            Self::InvalidIdentity => formatter.write_str("invalid mini_swe correlation identity"),
            Self::PromptTooLarge => formatter.write_str("mini_swe prompt exceeds byte limit"),
            Self::Io(error) => write!(formatter, "adapter I/O failed: {error}"),
            Self::WheelMismatch => formatter.write_str("mini_swe wheel pin mismatch"),
            Self::RuntimeMismatch => formatter.write_str("mini_swe Python runtime pin mismatch"),
            Self::DependencyMismatch => {
                formatter.write_str("mini_swe Python dependency pin mismatch")
            }
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
            #[cfg(test)]
            environment_digest_override: None,
            #[cfg(test)]
            distribution_override: None,
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
            capabilities: BTreeSet::from([Capability::Cancellation, Capability::Usage]),
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

    fn environment_verification_digest(&self) -> &str {
        #[cfg(test)]
        if let Some(digest) = &self.environment_digest_override {
            return digest;
        }
        TESTED_PYTHON_ENVIRONMENT_SHA256
    }

    fn expected_distributions(&self) -> Vec<(String, String)> {
        #[cfg(test)]
        if let Some(distributions) = &self.distribution_override {
            return distributions.clone();
        }
        TESTED_DISTRIBUTIONS
            .iter()
            .map(|(name, version)| ((*name).into(), (*version).into()))
            .collect()
    }

    fn site_packages(&self) -> Result<PathBuf, AdapterError> {
        self.python
            .parent()
            .and_then(Path::parent)
            .map(|root| root.join("lib/python3.12/site-packages"))
            .ok_or(AdapterError::RuntimeMismatch)
    }

    fn verify_environment(&self) -> Result<(), AdapterError> {
        let site_packages = self.site_packages()?;
        if distribution_inventory(&site_packages)? != self.expected_distributions()
            || environment_digest(&site_packages)? != self.environment_verification_digest()
        {
            return Err(AdapterError::DependencyMismatch);
        }
        Ok(())
    }

    /// Start one noninteractive edit attempt using an unlinked prompt descriptor.
    pub fn start(
        &self,
        session_id: Id,
        attempt_id: Id,
        prompt: &str,
        limits: ProcessLimits,
    ) -> Result<RunningMiniSwe, AdapterError> {
        if !valid_public_id(&session_id) || !valid_public_id(&attempt_id) {
            return Err(AdapterError::InvalidIdentity);
        }
        if prompt.len() > MAX_PROMPT_BYTES {
            return Err(AdapterError::PromptTooLarge);
        }
        if prompt.contains('\0') {
            return Err(AdapterError::InvalidPrompt);
        }
        self.verify_executable()?;
        self.verify_environment()?;
        let workspace = normalized_future_directory(&self.workspace)
            .map_err(|_| AdapterError::UnsafeWorkspace)?;
        let state_root = normalized_future_directory(&self.state_root)
            .map_err(|_| AdapterError::UnsafeStateRoot)?;
        if roots_overlap(&workspace, &state_root) {
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
            let launch_root = run_root.join("launch");
            fs::DirBuilder::new().mode(0o700).create(&launch_root)?;
            let staged_python = launch_root.join("python");
            let staged_wheel = launch_root.join("mini_swe.whl");
            let staged_environment = launch_root.join("site-packages");
            copy_verified_file(
                &self.python,
                &staged_python,
                self.python_verification_digest(),
                0o500,
                AdapterError::RuntimeMismatch,
            )?;
            copy_verified_file(
                &self.wheel,
                &staged_wheel,
                self.wheel_verification_digest(),
                0o400,
                AdapterError::WheelMismatch,
            )?;
            copy_environment(&self.site_packages()?, &staged_environment)?;
            if distribution_inventory(&staged_environment)? != self.expected_distributions()
                || environment_digest(&staged_environment)?
                    != self.environment_verification_digest()
            {
                return Err(AdapterError::DependencyMismatch);
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
            Ok::<_, AdapterError>((
                prompt_file,
                PreparedLaunch {
                    python: staged_python,
                    wheel: staged_wheel,
                    environment: staged_environment,
                },
            ))
        })();
        let (prompt_file, launch) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                let _ = fs::remove_dir_all(&run_root);
                return Err(error);
            }
        };
        let trajectory_path = run_root.join("trajectory.json");
        let process =
            match self.spawn_process(&run_root, prompt_file, &trajectory_path, &launch, limits) {
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
        launch: &PreparedLaunch,
        limits: ProcessLimits,
    ) -> Result<RunningProcess, AdapterError> {
        let host = self
            .endpoint
            .host_str()
            .ok_or(AdapterError::InvalidEndpoint)?;
        const DRIVER: &str = r#"import sys, zipfile
zipfile.ZipFile(sys.argv[6]).extractall(sys.argv[7])
sys.path.insert(0,sys.argv[7])
from minisweagent.agents import get_agent
from minisweagent.config import get_config_from_spec
from minisweagent.environments import get_environment
from minisweagent.models import get_model
from minisweagent.utils.serialize import recursive_merge
base=get_config_from_spec(sys.argv[8])
override={'agent':{'mode':'yolo','confirm_exit':False,'step_limit':4096,'wall_time_limit_seconds':int(sys.argv[4]),'output_path':sys.argv[3]},'environment':{'environment_class':'local','cwd':sys.argv[1],'timeout':30},'model':{'model_class':'litellm','model_name':'openai/'+sys.argv[2],'cost_tracking':'ignore_errors','model_kwargs':{'api_base':sys.argv[5],'api_key':'asb-credential-free'}}}
cfg=recursive_merge(base,override)
agent=get_agent(get_model(config=cfg['model']),get_environment(cfg['environment'],default_type='local'),cfg['agent'],default_type='default')
agent.run(sys.stdin.read())
"#;
        let mut command = Command::new(&launch.python);
        command
            .current_dir(&self.workspace)
            .args(["-P", "-S", "-c", DRIVER])
            .arg(&self.workspace)
            .arg(&self.model)
            .arg(trajectory_path)
            .arg(limits.timeout().as_secs().max(1).to_string())
            .arg(self.endpoint.as_str())
            .arg(&launch.wheel)
            .arg(run_root.join("package"))
            .arg(run_root.join("package/minisweagent/config/mini.yaml"))
            .stdin(Stdio::from(prompt_file))
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", run_root.join("home"))
            .env("XDG_CONFIG_HOME", run_root.join("config"))
            .env("XDG_DATA_HOME", run_root.join("data"))
            .env("XDG_CACHE_HOME", run_root.join("cache"))
            .env("TMPDIR", run_root.join("tmp"))
            .env("PYTHONPATH", &launch.environment)
            .env("PYTHONHOME", "/usr")
            .env("PYTHONNOUSERSITE", "1")
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .env("PYTHON_DOTENV_DISABLED", "1")
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

fn normalized_future_directory(path: &Path) -> Result<PathBuf, AdapterError> {
    let mut existing = PathBuf::from("/");
    let mut missing = Vec::new();
    let mut saw_missing = false;
    for component in path.components() {
        match component {
            Component::RootDir | Component::CurDir => {}
            Component::Normal(name) if saw_missing => missing.push(name.to_owned()),
            Component::Normal(name) => {
                let candidate = existing.join(name);
                match fs::symlink_metadata(&candidate) {
                    Ok(metadata) => {
                        if metadata.file_type().is_symlink() || !metadata.is_dir() {
                            return Err(AdapterError::UnsafeWorkspace);
                        }
                        existing = candidate;
                    }
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        missing.push(name.to_owned());
                        saw_missing = true;
                    }
                    Err(error) => return Err(AdapterError::Io(error)),
                }
            }
            Component::ParentDir | Component::Prefix(_) => {
                return Err(AdapterError::UnsafeWorkspace);
            }
        }
    }
    let mut normalized = fs::canonicalize(existing)?;
    for component in missing {
        normalized.push(component);
    }
    Ok(normalized)
}

fn roots_overlap(left: &Path, right: &Path) -> bool {
    left == right || left.starts_with(right) || right.starts_with(left)
}

fn valid_public_id(id: &Id) -> bool {
    !id.0.is_empty()
        && id.0.len() <= MAX_PUBLIC_ID_BYTES
        && id
            .0
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn copy_verified_file(
    source: &Path,
    destination: &Path,
    expected_digest: &str,
    mode: u32,
    mismatch: AdapterError,
) -> Result<(), AdapterError> {
    let metadata = fs::metadata(source)?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_ARTIFACT_BYTES {
        return Err(mismatch);
    }
    let mut input = fs::File::open(source)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(destination)?;
    let copied = io::copy(
        &mut Read::by_ref(&mut input).take(MAX_ARTIFACT_BYTES + 1),
        &mut output,
    )?;
    if copied != metadata.len() || copied > MAX_ARTIFACT_BYTES {
        return Err(mismatch);
    }
    output.sync_all()?;
    fs::set_permissions(destination, fs::Permissions::from_mode(mode))?;
    if digest_file(destination)? != expected_digest {
        return Err(mismatch);
    }
    Ok(())
}

fn environment_files(root: &Path) -> Result<Vec<(PathBuf, u64)>, AdapterError> {
    if !is_exact_canonical_directory(root)? {
        return Err(AdapterError::DependencyMismatch);
    }
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    let mut entries = 0_usize;
    let mut total = 0_u64;
    while let Some(directory) = pending.pop() {
        let mut children = fs::read_dir(directory)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(AdapterError::Io)?;
        children.sort_by_key(|entry| entry.file_name());
        for child in children {
            entries = entries
                .checked_add(1)
                .ok_or(AdapterError::DependencyMismatch)?;
            if entries > MAX_ENVIRONMENT_ENTRIES {
                return Err(AdapterError::DependencyMismatch);
            }
            let path = child.path();
            let relative = path
                .strip_prefix(root)
                .map_err(|_| AdapterError::DependencyMismatch)?;
            if relative.as_os_str().as_encoded_bytes().len() > MAX_PATH_BYTES {
                return Err(AdapterError::DependencyMismatch);
            }
            let kind = child.file_type()?;
            if kind.is_symlink() || !(kind.is_dir() || kind.is_file()) {
                return Err(AdapterError::DependencyMismatch);
            }
            if kind.is_dir() {
                pending.push(path);
                continue;
            }
            let size = child.metadata()?.len();
            if size > MAX_ENVIRONMENT_FILE_BYTES {
                return Err(AdapterError::DependencyMismatch);
            }
            total = total
                .checked_add(size)
                .ok_or(AdapterError::DependencyMismatch)?;
            if total > MAX_ENVIRONMENT_BYTES {
                return Err(AdapterError::DependencyMismatch);
            }
            files.push((relative.to_path_buf(), size));
        }
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(files)
}

fn environment_digest(root: &Path) -> Result<String, AdapterError> {
    let mut hasher = Sha256::new();
    for (relative, expected_size) in environment_files(root)? {
        let relative = relative
            .to_str()
            .ok_or(AdapterError::DependencyMismatch)?
            .as_bytes();
        hasher.update((relative.len() as u64).to_be_bytes());
        hasher.update(relative);
        hasher.update(expected_size.to_be_bytes());
        let mut file = fs::File::open(
            root.join(std::str::from_utf8(relative).map_err(|_| AdapterError::DependencyMismatch)?),
        )?;
        let copied = io::copy(
            &mut Read::by_ref(&mut file).take(MAX_ENVIRONMENT_FILE_BYTES + 1),
            &mut DigestWriter(&mut hasher),
        )?;
        if copied != expected_size {
            return Err(AdapterError::DependencyMismatch);
        }
    }
    Ok(format!("{:x}", hasher.finalize()))
}

struct DigestWriter<'a>(&'a mut Sha256);

impl Write for DigestWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn copy_environment(source: &Path, destination: &Path) -> Result<(), AdapterError> {
    fs::DirBuilder::new().mode(0o700).create(destination)?;
    for (relative, expected_size) in environment_files(source)? {
        let target = destination.join(&relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut input = fs::File::open(source.join(&relative))?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o400)
            .open(&target)?;
        let copied = io::copy(
            &mut Read::by_ref(&mut input).take(MAX_ENVIRONMENT_FILE_BYTES + 1),
            &mut output,
        )?;
        if copied != expected_size {
            return Err(AdapterError::DependencyMismatch);
        }
        fs::set_permissions(target, fs::Permissions::from_mode(0o400))?;
    }
    Ok(())
}

fn distribution_inventory(root: &Path) -> Result<Vec<(String, String)>, AdapterError> {
    let mut distributions = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir()
            || !entry
                .file_name()
                .as_encoded_bytes()
                .ends_with(b".dist-info")
        {
            continue;
        }
        let metadata_path = entry.path().join("METADATA");
        let metadata = fs::metadata(&metadata_path)?;
        if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_METADATA_BYTES {
            return Err(AdapterError::DependencyMismatch);
        }
        let mut bytes = Vec::new();
        fs::File::open(metadata_path)?
            .take(MAX_METADATA_BYTES + 1)
            .read_to_end(&mut bytes)?;
        let text = std::str::from_utf8(&bytes).map_err(|_| AdapterError::DependencyMismatch)?;
        let mut name = None;
        let mut version = None;
        for line in text.lines().take_while(|line| !line.is_empty()) {
            if let Some(value) = line.strip_prefix("Name: ") {
                if name.replace(normalize_distribution_name(value)?).is_some() {
                    return Err(AdapterError::DependencyMismatch);
                }
            } else if let Some(value) = line.strip_prefix("Version: ")
                && (value.is_empty()
                    || value.len() > 256
                    || value.bytes().any(|byte| byte.is_ascii_control())
                    || version.replace(value.to_owned()).is_some())
            {
                return Err(AdapterError::DependencyMismatch);
            }
        }
        distributions.push((
            name.ok_or(AdapterError::DependencyMismatch)?,
            version.ok_or(AdapterError::DependencyMismatch)?,
        ));
    }
    distributions.sort();
    if distributions.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err(AdapterError::DependencyMismatch);
    }
    Ok(distributions)
}

fn normalize_distribution_name(value: &str) -> Result<String, AdapterError> {
    if value.is_empty() || value.len() > 256 || !value.is_ascii() {
        return Err(AdapterError::DependencyMismatch);
    }
    let mut normalized = String::new();
    let mut separator = false;
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() {
            if separator && !normalized.is_empty() {
                normalized.push('-');
            }
            normalized.push((byte as char).to_ascii_lowercase());
            separator = false;
        } else if matches!(byte, b'-' | b'_' | b'.') {
            separator = true;
        } else {
            return Err(AdapterError::DependencyMismatch);
        }
    }
    if normalized.is_empty() || separator {
        return Err(AdapterError::DependencyMismatch);
    }
    Ok(normalized)
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
    let mut pending = BTreeMap::<String, (Id, String)>::new();
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
                        let command = item
                            .get("command")
                            .and_then(Value::as_str)
                            .ok_or(AdapterError::InvalidTrajectory)?;
                        if !safe_id(id) || pending.contains_key(id) {
                            return Err(AdapterError::InvalidTrajectory);
                        }
                        let causal_id = Id(format!("mini-swe-tool-{actions:04}"));
                        pending.insert(id.to_owned(), (causal_id.clone(), command.to_owned()));
                        events.push(event(
                            session,
                            attempt,
                            sequence,
                            Event::ToolStarted {
                                tool_call_id: causal_id,
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
                let (causal_id, _) = pending.remove(id).ok_or(AdapterError::InvalidTrajectory)?;
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
                        tool_call_id: causal_id,
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
    let submitted = match exit_status {
        "Submitted" => true,
        "LimitsExceeded" | "TimeExceeded" | "RepeatedFormatError" => false,
        _ => return Err(AdapterError::InvalidTrajectory),
    };
    if submitted {
        if pending.len() != 1 {
            return Err(AdapterError::InvalidTrajectory);
        }
        let (_, (causal_id, command)) =
            pending.pop_first().ok_or(AdapterError::InvalidTrajectory)?;
        if command != SUBMISSION_COMMAND {
            return Err(AdapterError::InvalidTrajectory);
        }
        events.push(event(
            session,
            attempt,
            sequence,
            Event::ToolFinished {
                tool_call_id: causal_id,
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
    if submitted {
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
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    use std::time::{Duration, Instant};

    const MAX_PROC_STAT_BYTES: u64 = 4096;
    const MAX_PID_EVIDENCE_BYTES: u64 = 32;
    const MAX_PID_LIST_EVIDENCE_BYTES: u64 = 128;
    const MAX_PROC_ENTRIES: usize = 65_536;
    const MAX_PROCESS_GROUP_MEMBERS: usize = 1_024;

    fn canonical_repository_root() -> io::Result<PathBuf> {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let repository = manifest
            .ancestors()
            .nth(2)
            .ok_or_else(|| io::Error::other("repository root is unavailable"))?;
        fs::canonicalize(repository)
    }

    fn paths_overlap(left: &Path, right: &Path) -> bool {
        left.starts_with(right) || right.starts_with(left)
    }

    fn open_bound_directory(path: &Path, expected: &Path) -> io::Result<fs::File> {
        let flags = rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::DIRECTORY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC;
        let descriptor = rustix::fs::open(path, flags, rustix::fs::Mode::empty())?;
        let expected_descriptor = rustix::fs::open(expected, flags, rustix::fs::Mode::empty())?;
        let opened = rustix::fs::fstat(&descriptor)?;
        let expected_opened = rustix::fs::fstat(&expected_descriptor)?;
        if opened.st_dev != expected_opened.st_dev || opened.st_ino != expected_opened.st_ino {
            return Err(io::Error::other("test-root directory binding changed"));
        }
        Ok(fs::File::from(descriptor))
    }

    fn open_bound_entry(base: &fs::File, name: &str) -> io::Result<fs::File> {
        let descriptor = rustix::fs::openat(
            base,
            name,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )?;
        let linked = rustix::fs::statat(base, name, rustix::fs::AtFlags::SYMLINK_NOFOLLOW)?;
        let opened = rustix::fs::fstat(&descriptor)?;
        if linked.st_dev != opened.st_dev || linked.st_ino != opened.st_ino {
            return Err(io::Error::other("test-root directory binding changed"));
        }
        Ok(fs::File::from(descriptor))
    }

    struct PrivateTestRoot {
        base: fs::File,
        root: fs::File,
        path: PathBuf,
        name: String,
        cleanup_attempted: bool,
    }

    impl PrivateTestRoot {
        fn new(label: &str) -> io::Result<Self> {
            let base = fs::canonicalize(std::env::temp_dir())?;
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| io::Error::other("test clock is before epoch"))?
                .as_nanos();
            let name = format!("asb-mini-swe-{label}-{}-{nonce}", std::process::id());
            Self::create_named(&base, name)
        }

        fn create_named(base_path: &Path, name: String) -> io::Result<Self> {
            if name.is_empty()
                || name.len() > 160
                || !name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
            {
                return Err(io::Error::other("invalid test-root identity"));
            }
            let before = fs::symlink_metadata(base_path)?;
            let canonical_base = fs::canonicalize(base_path)?;
            if !before.file_type().is_dir() || canonical_base != base_path {
                return Err(io::Error::other("unsafe test-root base"));
            }
            if paths_overlap(&canonical_base, &canonical_repository_root()?) {
                return Err(io::Error::other("test root overlaps repository"));
            }
            let base = open_bound_directory(base_path, &canonical_base)?;
            let opened = base.metadata()?;
            let mode = opened.mode() & 0o7777;
            let effective_uid = fs::metadata("/proc/self")?.uid();
            let private_owner = opened.uid() == effective_uid && mode & 0o022 == 0;
            let sticky_shared = mode & 0o1002 == 0o1002;
            if !private_owner && !sticky_shared {
                return Err(io::Error::other("unsafe test-root base"));
            }
            let base_anchor = PathBuf::from(format!("/proc/self/fd/{}", base.as_raw_fd()));
            let path = base_path.join(&name);
            let linked_path = base_anchor.join(&name);
            fs::DirBuilder::new().mode(0o700).create(&linked_path)?;
            let root = open_bound_entry(&base, &name)?;
            let metadata = root.metadata()?;
            if metadata.uid() != effective_uid
                || metadata.mode() & 0o7777 != 0o700
                || metadata.nlink() != 2
            {
                return Err(io::Error::other("unsafe created test root"));
            }
            Ok(Self {
                base,
                root,
                path,
                name,
                cleanup_attempted: false,
            })
        }

        fn path(&self) -> &Path {
            &self.path
        }

        fn cleanup(&mut self) -> io::Result<()> {
            self.cleanup_attempted = true;
            let root_anchor = PathBuf::from(format!("/proc/self/fd/{}", self.root.as_raw_fd()));
            for entry in fs::read_dir(&root_anchor)? {
                let entry = entry?;
                let metadata = fs::symlink_metadata(entry.path())?;
                if metadata.file_type().is_dir() {
                    fs::remove_dir_all(entry.path())?;
                } else {
                    fs::remove_file(entry.path())?;
                }
            }
            let base_anchor = PathBuf::from(format!("/proc/self/fd/{}", self.base.as_raw_fd()));
            let linked_path = base_anchor.join(&self.name);
            let linked = fs::symlink_metadata(&linked_path)?;
            if linked.file_type().is_symlink()
                || fs::canonicalize(&linked_path)? != fs::canonicalize(&root_anchor)?
            {
                return Err(io::Error::other("test root changed before cleanup"));
            }
            fs::remove_dir(linked_path)
        }
    }

    impl Drop for PrivateTestRoot {
        fn drop(&mut self) {
            if !self.cleanup_attempted {
                let _ = self.cleanup();
            }
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct ProcessIdentity {
        pid: u32,
        state: u8,
        process_group: u32,
        session: u32,
        start_time: u64,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum OriginalProcessState {
        Missing,
        Reused,
        Zombie,
        Runnable,
        ChangedOwnership,
    }

    fn parse_pid_evidence(bytes: &[u8]) -> Option<u32> {
        if bytes.is_empty() || bytes.len() > MAX_PID_EVIDENCE_BYTES as usize {
            return None;
        }
        let text = std::str::from_utf8(bytes).ok()?;
        let digits = text.strip_suffix("\n").unwrap_or(text);
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        digits.parse().ok().filter(|pid| *pid > 0)
    }

    fn parse_pid_list_evidence(bytes: &[u8]) -> Option<Vec<u32>> {
        if bytes.is_empty() || bytes.len() > MAX_PID_LIST_EVIDENCE_BYTES as usize {
            return None;
        }
        let text = std::str::from_utf8(bytes).ok()?;
        let mut pids = Vec::new();
        for line in text.lines() {
            let pid = parse_pid_evidence(line.as_bytes())?;
            if pids.contains(&pid) {
                return None;
            }
            pids.push(pid);
        }
        (!pids.is_empty()).then_some(pids)
    }

    fn parse_process_identity(expected_pid: u32, bytes: &[u8]) -> Option<ProcessIdentity> {
        if bytes.is_empty() || bytes.len() > MAX_PROC_STAT_BYTES as usize {
            return None;
        }
        let text = std::str::from_utf8(bytes).ok()?;
        let (identity, fields) = text.rsplit_once(") ")?;
        let (pid, _) = identity.split_once(" (")?;
        if pid.parse::<u32>().ok()? != expected_pid {
            return None;
        }
        let fields: Vec<&str> = fields.split_whitespace().collect();
        if fields.len() < 20
            || fields[0].len() != 1
            || !b"RSDZTtXxKWPI".contains(&fields[0].as_bytes()[0])
        {
            return None;
        }
        Some(ProcessIdentity {
            pid: expected_pid,
            state: fields[0].as_bytes()[0],
            process_group: fields[2].parse().ok()?,
            session: fields[3].parse().ok()?,
            start_time: fields[19].parse().ok()?,
        })
    }

    fn read_bounded(path: &Path, limit: u64) -> io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        fs::File::open(path)?
            .take(limit + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > limit {
            return Err(io::Error::other("bounded test evidence is oversized"));
        }
        Ok(bytes)
    }

    fn read_process_identity(pid: u32) -> io::Result<Option<ProcessIdentity>> {
        let path = PathBuf::from("/proc").join(pid.to_string()).join("stat");
        match read_bounded(&path, MAX_PROC_STAT_BYTES) {
            Ok(bytes) => parse_process_identity(pid, &bytes)
                .map(Some)
                .ok_or_else(|| io::Error::other("malformed process identity")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn process_group_members(process_group: u32, session: u32) -> io::Result<Vec<ProcessIdentity>> {
        let mut members = Vec::new();
        for (index, entry) in fs::read_dir("/proc")?.enumerate() {
            if index >= MAX_PROC_ENTRIES {
                return Err(io::Error::other("process table exceeds test bound"));
            }
            let entry = entry?;
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<u32>().ok())
            else {
                continue;
            };
            let Some(identity) = read_process_identity(pid)? else {
                continue;
            };
            if identity.process_group == process_group && identity.session == session {
                if members.len() >= MAX_PROCESS_GROUP_MEMBERS {
                    return Err(io::Error::other("process group exceeds test bound"));
                }
                members.push(identity);
            }
        }
        members.sort_by_key(|identity| identity.pid);
        Ok(members)
    }

    fn runnable_group_members(
        process_group: u32,
        session: u32,
    ) -> io::Result<Vec<ProcessIdentity>> {
        Ok(process_group_members(process_group, session)?
            .into_iter()
            .filter(|identity| identity.state != b'Z')
            .collect())
    }

    fn classify_original(
        original: ProcessIdentity,
        observed: Option<ProcessIdentity>,
    ) -> OriginalProcessState {
        let Some(observed) = observed else {
            return OriginalProcessState::Missing;
        };
        if observed.pid != original.pid || observed.start_time != original.start_time {
            return OriginalProcessState::Reused;
        }
        if observed.process_group != original.process_group || observed.session != original.session
        {
            return OriginalProcessState::ChangedOwnership;
        }
        if observed.state == b"Z"[0] {
            OriginalProcessState::Zombie
        } else {
            OriginalProcessState::Runnable
        }
    }

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
    fn submitted(cost: &str) -> String {
        valid(
            r#"[{"tool_call_id":"private-upstream-id","command":"echo COMPLETE_TASK_AND_SUBMIT_FINAL_OUTPUT"}]"#,
            "",
            "Submitted",
            1,
            cost,
        )
    }

    #[test]
    fn maps_bounded_causal_tool_and_failed_tool() {
        let input = valid(
            r#"[{"tool_call_id":"call-1","command":"false"},{"tool_call_id":"private-submit","command":"echo COMPLETE_TASK_AND_SUBMIT_FINAL_OUTPUT"}]"#,
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
        assert!(events.iter().all(|event| match &event.event {
            Event::ToolStarted { tool_call_id, .. } | Event::ToolFinished { tool_call_id, .. } => {
                !tool_call_id.0.contains("call-1") && !tool_call_id.0.contains("private-submit")
            }
            _ => true,
        }));
    }

    #[test]
    fn non_submission_is_failed_not_completed() {
        for terminal in ["LimitsExceeded", "TimeExceeded", "RepeatedFormatError"] {
            let (_, status) = parse(&valid("[]", "", terminal, 1, "0")).unwrap();
            assert_eq!(status, TerminalStatus::Failed);
        }
        for terminal in ["", "Failed", "Unknown", "UserInterruption", "submitted"] {
            assert!(matches!(
                parse(&valid("[]", "", terminal, 1, "0")),
                Err(AdapterError::InvalidTrajectory)
            ));
        }
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
            parse(&valid(
                r#"[{"tool_call_id":"submit","command":"echo COMPLETE_TASK_AND_SUBMIT_FINAL_OUTPUT"}]"#,
                "",
                "Submitted",
                0,
                "0"
            )),
            Err(AdapterError::InvalidTrajectory)
        ));
        assert!(matches!(
            parse(&submitted("-1")),
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
        let wrong_calls = valid(
            r#"[{"tool_call_id":"submit","command":"echo COMPLETE_TASK_AND_SUBMIT_FINAL_OUTPUT"}]"#,
            "",
            "Submitted",
            2,
            "0",
        );
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
        let scalar_coverage = submitted("0").replace(
            "\"trajectory_format\"",
            "\"ignored\":[true,-1,1,1.5,\"text\",null],\"trajectory_format\"",
        );
        assert!(parse(&scalar_coverage).is_ok());
        for command in ["true", "printf COMPLETE_TASK_AND_SUBMIT_FINAL_OUTPUT"] {
            let arbitrary = valid(
                &format!(r#"[{{"tool_call_id":"pending","command":"{command}"}}]"#),
                "",
                "Submitted",
                1,
                "0",
            );
            assert!(matches!(
                parse(&arbitrary),
                Err(AdapterError::InvalidTrajectory)
            ));
        }
        let path_id = valid(
            r#"[{"tool_call_id":"/private/provider/call","command":"echo COMPLETE_TASK_AND_SUBMIT_FINAL_OUTPUT"}]"#,
            "",
            "Submitted",
            1,
            "0",
        );
        let (events, _) = parse(&path_id).unwrap();
        assert!(!format!("{events:?}").contains("/private/provider/call"));
        let control_id = submitted("0").replace("private-upstream-id", "private\\nidentifier");
        assert!(matches!(
            parse(&control_id),
            Err(AdapterError::InvalidTrajectory)
        ));
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
        let (events, status) = parse(&submitted("0")).unwrap();
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
        fs::write(root.join("workspace/mini.yaml"), "hostile: true\n").unwrap();
        let python = root.join("venv/bin/python");
        let wheel = root.join("package.whl");
        fs::create_dir_all(python.parent().unwrap()).unwrap();
        fs::write(&wheel, b"fixture").unwrap();
        fs::write(&python, r#"#!/bin/sh
case "$HOME" in */attempt-*/home) ;; *) exit 91;; esac
test "$MSWEA_GLOBAL_CONFIG_DIR" = "${HOME%/home}/config" || exit 92
test ! -e "$HOME/ambient-sentinel" || exit 93
test "$PYTHON_DOTENV_DISABLED" = 1 || exit 94
test "$PYTHONNOUSERSITE" = 1 || exit 95
test "$1" = -P || exit 96
test "$2" = -S || exit 97
case "$4" in *"get_config_from_spec(sys.argv[8])"*) ;; *) exit 98;; esac
test "${12}" = "${HOME%/home}/package/minisweagent/config/mini.yaml" || exit 99
cat > "$7" <<'EOF'
{"trajectory_format":"mini-swe-agent-1.1","info":{"mini_version":"2.4.6","model_stats":{"api_calls":1,"instance_cost":0.0},"exit_status":"Submitted"},"messages":[{"role":"system"},{"role":"user"},{"role":"assistant","extra":{"actions":[{"tool_call_id":"private","command":"echo COMPLETE_TASK_AND_SUBMIT_FINAL_OUTPUT"}]}},{"role":"exit"}]}
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
        boundary_tests::install_test_environment(&mut config);
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
    fn cancellation_leaves_no_runnable_owned_descendant() {
        let mut scratch = PrivateTestRoot::new("descendant").unwrap();
        let root = scratch.path().to_path_buf();
        fs::create_dir_all(root.join("workspace")).unwrap();
        fs::create_dir_all(root.join("state")).unwrap();
        let python = root.join("venv/bin/python");
        let wheel = root.join("package.whl");
        fs::create_dir_all(python.parent().unwrap()).unwrap();
        fs::write(&wheel, b"fixture").unwrap();
        fs::write(
            &python,
            r#"#!/bin/sh
(while [ ! -p "$5/block" ]; do :; done; read ignored < "$5/block") &
first=$!
(while [ ! -p "$5/block" ]; do :; done; read ignored < "$5/block") &
second=$!
printf '%s\n%s\n' "$first" "$second" > "$5/children.pids"
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
        boundary_tests::install_test_environment(&mut config);
        let limits = ProcessLimits::new(
            1024,
            1024,
            Duration::from_secs(30),
            Duration::from_millis(20),
            Duration::from_millis(5),
        )
        .unwrap();
        let mut running = config
            .start(Id("s".into()), Id("a".into()), "prompt", limits)
            .unwrap();
        let fifo_path = root.join("workspace/block");
        rustix::fs::mkfifoat(
            rustix::fs::CWD,
            &fifo_path,
            rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
        )
        .unwrap();
        let fifo_guard = rustix::fs::open(
            &fifo_path,
            rustix::fs::OFlags::RDWR | rustix::fs::OFlags::NONBLOCK | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .unwrap();
        let pid_path = root.join("workspace/children.pids");
        let readiness_deadline = Instant::now() + Duration::from_secs(15);
        let (children, original_group) = loop {
            if let Ok(bytes) = read_bounded(&pid_path, MAX_PID_LIST_EVIDENCE_BYTES)
                && let Some(pids) = parse_pid_list_evidence(&bytes)
                && pids.len() == 2
                && let Ok(members) = process_group_members(
                    running.pid(),
                    pids.first()
                        .and_then(|pid| read_process_identity(*pid).ok().flatten())
                        .map(|identity| identity.session)
                        .unwrap_or(0),
                )
                && pids
                    .iter()
                    .all(|pid| members.iter().any(|identity| identity.pid == *pid))
            {
                break (pids, members);
            }
            assert!(
                Instant::now() < readiness_deadline,
                "helper readiness timed out"
            );
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(children.len(), 2);
        let session = original_group
            .iter()
            .find(|identity| identity.pid == children[0])
            .unwrap()
            .session;
        assert!(
            original_group
                .iter()
                .any(|identity| identity.pid == running.pid())
        );
        assert!(original_group.iter().all(|identity| {
            identity.process_group == running.pid() && identity.session == session
        }));
        running.cancel().unwrap();
        assert_eq!(running.wait().unwrap().status(), TerminalStatus::Cancelled);
        let terminal_deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if runnable_group_members(running.pid(), session)
                .unwrap()
                .is_empty()
            {
                break;
            }
            assert!(
                Instant::now() < terminal_deadline,
                "owned descendant remained runnable"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        drop(fifo_guard);
        scratch.cleanup().unwrap();
    }

    #[test]
    fn descendant_identity_evidence_fails_closed_and_distinguishes_reuse() {
        assert_eq!(parse_pid_evidence(b"17\n"), Some(17));
        for evidence in [
            b"".as_slice(),
            b"0",
            b" 17\n",
            b"17  ",
            b"x",
            &[b"1"[0]; 33],
        ] {
            assert_eq!(parse_pid_evidence(evidence), None);
        }
        let stat = b"17 (fixture helper) S 1 9 8 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 44 20";
        let original = parse_process_identity(17, stat).unwrap();
        assert_eq!(original.process_group, 9);
        assert_eq!(original.session, 8);
        assert_eq!(original.start_time, 44);
        assert_eq!(
            classify_original(original, None),
            OriginalProcessState::Missing
        );
        let mut changed = original;
        changed.start_time += 1;
        assert_eq!(
            classify_original(original, Some(changed)),
            OriginalProcessState::Reused
        );
        changed = original;
        changed.process_group += 1;
        assert_eq!(
            classify_original(original, Some(changed)),
            OriginalProcessState::ChangedOwnership
        );
        changed = original;
        changed.state = b"Z"[0];
        assert_eq!(
            classify_original(original, Some(changed)),
            OriginalProcessState::Zombie
        );
        assert_eq!(
            classify_original(original, Some(original)),
            OriginalProcessState::Runnable
        );
        assert_eq!(
            parse_process_identity(18, stat),
            None,
            "a PID mismatch cannot be credited as the original process"
        );
        assert_eq!(
            parse_process_identity(
                17,
                b"17 (fixture helper) ? 1 9 8 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 44 20"
            ),
            None
        );
        let oversized = vec![b"x"[0]; MAX_PROC_STAT_BYTES as usize + 1];
        assert_eq!(parse_process_identity(17, &oversized), None);
        assert_eq!(parse_pid_list_evidence(b"17\n18\n"), Some(vec![17, 18]));
        assert_eq!(parse_pid_list_evidence(b"17\n17\n"), None);
    }

    #[test]
    fn private_test_root_rejects_redirection_and_preserves_replacement() {
        let base = fs::canonicalize(std::env::temp_dir()).unwrap();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let fixture = base.join(format!(
            "asb-mini-swe-root-negative-{}-{nonce}",
            std::process::id()
        ));
        fs::DirBuilder::new().mode(0o700).create(&fixture).unwrap();
        let canonical_fixture = fs::canonicalize(&fixture).unwrap();
        let fixture_handle = open_bound_directory(&fixture, &canonical_fixture).unwrap();
        assert!(open_bound_directory(&fixture, &base).is_err());
        let redirected = fixture.join("redirected");
        std::os::unix::fs::symlink(&base, &redirected).unwrap();
        assert!(open_bound_directory(&redirected, &base).is_err());
        assert!(open_bound_entry(&fixture_handle, "redirected").is_err());
        assert!(PrivateTestRoot::create_named(&redirected, "child".into()).is_err());
        let leaf = fixture.join("leaf");
        std::os::unix::fs::symlink(&base, &leaf).unwrap();
        assert!(PrivateTestRoot::create_named(&fixture, "leaf".into()).is_err());
        assert!(
            fs::symlink_metadata(&leaf)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        let unsafe_base = fixture.join("unsafe-base");
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&unsafe_base)
            .unwrap();
        fs::set_permissions(&unsafe_base, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(PrivateTestRoot::create_named(&unsafe_base, "child".into()).is_err());
        let repository = canonical_repository_root().unwrap();
        assert!(PrivateTestRoot::create_named(&repository, "overlap".into()).is_err());
        assert!(
            PrivateTestRoot::create_named(&repository.join("crates"), "overlap".into()).is_err()
        );
        assert!(
            PrivateTestRoot::create_named(repository.parent().unwrap(), "ancestor-overlap".into())
                .is_err()
        );

        let name = "owned-root".to_owned();
        let mut scratch = PrivateTestRoot::create_named(&fixture, name.clone()).unwrap();
        fs::write(scratch.path().join("owned"), b"owned").unwrap();
        let displaced = fixture.join("displaced");
        fs::rename(scratch.path(), &displaced).unwrap();
        fs::DirBuilder::new()
            .mode(0o700)
            .create(fixture.join(&name))
            .unwrap();
        fs::write(fixture.join(&name).join("replacement"), b"replacement").unwrap();
        assert!(scratch.cleanup().is_err());
        assert_eq!(
            fs::read(fixture.join(&name).join("replacement")).unwrap(),
            b"replacement"
        );
        assert!(fs::read_dir(&displaced).unwrap().next().is_none());
        fs::remove_dir_all(fixture).unwrap();
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
        let python = scratch.0.join("venv/bin/python");
        let wheel = scratch.0.join("mini_swe.whl");
        fs::create_dir_all(python.parent().unwrap()).unwrap();
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
        install_test_environment(&mut config);
        (scratch, config)
    }

    pub(super) fn install_test_environment(config: &mut MiniSweConfig) {
        let site_packages = config.site_packages().unwrap();
        fs::create_dir_all(site_packages.join("fixture-1.0.dist-info")).unwrap();
        fs::write(
            site_packages.join("fixture-1.0.dist-info/METADATA"),
            "Metadata-Version: 2.1\nName: fixture\nVersion: 1.0\n\n",
        )
        .unwrap();
        fs::write(site_packages.join("fixture.py"), "VALUE = 1\n").unwrap();
        config.distribution_override = Some(vec![("fixture".into(), "1.0".into())]);
        config.environment_digest_override = Some(environment_digest(&site_packages).unwrap());
    }

    #[test]
    fn configuration_manifest_and_pins_are_bounded() {
        let (_scratch, config) = adapter("#!/bin/sh\nexit 0\n");
        let mut defaults = config.clone();
        defaults.wheel_digest_override = None;
        defaults.python_digest_override = None;
        defaults.environment_digest_override = None;
        defaults.distribution_override = None;
        assert_eq!(defaults.wheel_verification_digest(), WHEEL_SHA256);
        assert_eq!(
            defaults.python_verification_digest(),
            TESTED_PYTHON_LINUX_X86_64_SHA256
        );
        assert_eq!(
            defaults.environment_verification_digest(),
            TESTED_PYTHON_ENVIRONMENT_SHA256
        );
        let distributions = defaults.expected_distributions();
        assert_eq!(distributions.len(), 78);
        assert_eq!(
            distributions.first().unwrap(),
            &("aiohappyeyeballs".into(), "2.7.1".into())
        );
        assert_eq!(
            distributions.last().unwrap(),
            &("zipp".into(), "4.1.0".into())
        );
        assert_eq!(
            AdapterError::InvalidIdentity.to_string(),
            "invalid mini_swe correlation identity"
        );
        assert_eq!(
            AdapterError::DependencyMismatch.to_string(),
            "mini_swe Python dependency pin mismatch"
        );
        assert_eq!(config.manifest().extension_id.0, "agent.mini-swe");
        assert_eq!(
            config.manifest().capabilities,
            BTreeSet::from([Capability::Cancellation, Capability::Usage])
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
    fn public_identities_fail_before_filesystem_or_process_effects() {
        let (scratch, config) = adapter("#!/bin/sh\nexit 99\n");
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
                config.start(Id(id.into()), Id("attempt".into()), "prompt", limits()),
                Err(AdapterError::InvalidIdentity)
            ));
            assert!(matches!(
                config.start(Id("session".into()), Id(id.into()), "prompt", limits()),
                Err(AdapterError::InvalidIdentity)
            ));
        }
        assert!(matches!(
            config.start(
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
    fn isolated_interpreter_disables_automatic_site_imports() {
        let system_python = Path::new("/usr/bin/python3");
        assert!(
            system_python.is_file(),
            "Linux test image lacks /usr/bin/python3"
        );
        let (scratch, mut config) = adapter("#!/bin/sh\nexit 0\n");
        fs::copy(system_python, &config.python).unwrap();
        fs::set_permissions(&config.python, fs::Permissions::from_mode(0o700)).unwrap();
        config.python_digest_override = Some(digest_file(&config.python).unwrap());
        let site_packages = config.site_packages().unwrap();
        fs::write(
            site_packages.join("sitecustomize.py"),
            "from pathlib import Path\nPath('hostile-site-ran').write_text('unsafe')\n",
        )
        .unwrap();
        config.environment_digest_override = Some(environment_digest(&site_packages).unwrap());
        let mut running = config
            .start(
                Id("session".into()),
                Id("attempt".into()),
                "prompt",
                limits(),
            )
            .unwrap();
        assert_eq!(running.wait().unwrap().status(), TerminalStatus::Failed);
        assert!(!scratch.0.join("workspace/hostile-site-ran").exists());
    }

    #[test]
    fn dependency_tree_and_inventory_are_product_bound() {
        let (_scratch, mut config) = adapter("#!/bin/sh\nexit 0\n");
        config.verify_environment().unwrap();
        let site_packages = config.site_packages().unwrap();
        fs::write(site_packages.join("fixture.py"), "VALUE = 2\n").unwrap();
        assert!(matches!(
            config.verify_environment(),
            Err(AdapterError::DependencyMismatch)
        ));
        config.environment_digest_override = Some(environment_digest(&site_packages).unwrap());
        fs::write(
            site_packages.join("fixture-1.0.dist-info/METADATA"),
            "Metadata-Version: 2.1\nName: fixture\nVersion: 2.0\n\n",
        )
        .unwrap();
        assert!(matches!(
            config.verify_environment(),
            Err(AdapterError::DependencyMismatch)
        ));
    }

    #[test]
    fn distribution_metadata_is_canonical_unique_and_bounded() {
        let scratch = Scratch::new("distribution-metadata");
        let root = scratch.0.join("site-packages");
        let first = root.join("first.dist-info");
        fs::create_dir_all(&first).unwrap();
        let metadata = first.join("METADATA");
        fs::write(
            &metadata,
            "Metadata-Version: 2.1\nName: Fixture.Name_Test\nVersion: 1.0+local\n\n",
        )
        .unwrap();
        assert_eq!(
            distribution_inventory(&root).unwrap(),
            vec![("fixture-name-test".into(), "1.0+local".into())]
        );

        fs::write(&metadata, "Name: fixture\nName: duplicate\nVersion: 1\n\n").unwrap();
        assert!(matches!(
            distribution_inventory(&root),
            Err(AdapterError::DependencyMismatch)
        ));
        fs::write(&metadata, "Name: fixture!\nVersion: 1\n\n").unwrap();
        assert!(matches!(
            distribution_inventory(&root),
            Err(AdapterError::DependencyMismatch)
        ));
        fs::write(&metadata, "Name: fixture-\nVersion: 1\n\n").unwrap();
        assert!(matches!(
            distribution_inventory(&root),
            Err(AdapterError::DependencyMismatch)
        ));
        fs::write(&metadata, "Name: fixture\nVersion: 1\nVersion: 2\n\n").unwrap();
        assert!(matches!(
            distribution_inventory(&root),
            Err(AdapterError::DependencyMismatch)
        ));

        fs::write(&metadata, "Name: fixture\nVersion: 1\n\n").unwrap();
        let second = root.join("second.dist-info");
        fs::create_dir_all(&second).unwrap();
        fs::write(second.join("METADATA"), "Name: Fixture\nVersion: 2\n\n").unwrap();
        assert!(matches!(
            distribution_inventory(&root),
            Err(AdapterError::DependencyMismatch)
        ));

        fs::remove_dir_all(second).unwrap();
        fs::write(&metadata, vec![b'x'; MAX_METADATA_BYTES as usize + 1]).unwrap();
        assert!(matches!(
            distribution_inventory(&root),
            Err(AdapterError::DependencyMismatch)
        ));
    }

    #[test]
    fn dependency_tree_rejects_links_special_files_and_size_bounds() {
        let (_scratch, config) = adapter("#!/bin/sh\nexit 0\n");
        let site_packages = config.site_packages().unwrap();
        std::os::unix::fs::symlink(
            site_packages.join("fixture.py"),
            site_packages.join("alias.py"),
        )
        .unwrap();
        assert!(matches!(
            config.verify_environment(),
            Err(AdapterError::DependencyMismatch)
        ));
        fs::remove_file(site_packages.join("alias.py")).unwrap();
        let oversized = site_packages.join("oversized.bin");
        fs::File::create(&oversized)
            .unwrap()
            .set_len(MAX_ENVIRONMENT_FILE_BYTES + 1)
            .unwrap();
        assert!(matches!(
            config.verify_environment(),
            Err(AdapterError::DependencyMismatch)
        ));
    }

    #[test]
    fn configured_artifacts_are_replaced_only_after_private_staging() {
        let script = r#"#!/bin/sh
case "$0" in */attempt-*/launch/python) ;; *) exit 81;; esac
test "$2" = -S || exit 82
case "${10}" in */attempt-*/launch/mini_swe.whl) ;; *) exit 83;; esac
test "$(cat "${10}")" = wheel || exit 84
test "$PYTHON_DOTENV_DISABLED" = 1 || exit 85
test -z "${OPENAI_API_KEY-}" || exit 86
while test ! -e "$5/go"; do sleep 0.01; done
cat > "$7" <<'EOF'
{"trajectory_format":"mini-swe-agent-1.1","info":{"mini_version":"2.4.6","model_stats":{"api_calls":1,"instance_cost":0.0},"exit_status":"Submitted"},"messages":[{"role":"system"},{"role":"user"},{"role":"assistant","extra":{"actions":[{"tool_call_id":"private","command":"echo COMPLETE_TASK_AND_SUBMIT_FINAL_OUTPUT"}]}},{"role":"exit"}]}
EOF
"#;
        let (scratch, config) = adapter(script);
        fs::create_dir_all(scratch.0.join("workspace")).unwrap();
        fs::write(
            scratch.0.join("workspace/.env"),
            "OPENAI_API_KEY=hostile\nOPENAI_API_BASE=https://example.invalid\n",
        )
        .unwrap();
        let mut running = config
            .start(
                Id("session".into()),
                Id("attempt".into()),
                "prompt",
                limits(),
            )
            .unwrap();
        fs::write(&config.python, "#!/bin/sh\nexit 97\n").unwrap();
        fs::set_permissions(&config.python, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(&config.wheel, "replacement").unwrap();
        fs::write(scratch.0.join("workspace/go"), "").unwrap();
        assert_eq!(running.wait().unwrap().status(), TerminalStatus::Completed);
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

    #[test]
    fn overlapping_absent_roots_fail_before_mutation_in_both_directions() {
        let (scratch, mut config) = boundary_tests::adapter("#!/bin/sh\nexit 99\n");
        config.workspace = scratch.0.join("absent/workspace");
        config.state_root = config.workspace.join("private-state");
        assert!(matches!(
            config.start(
                Id("s".into()),
                Id("a".into()),
                "prompt",
                boundary_tests::limits()
            ),
            Err(AdapterError::UnsafeStateRoot)
        ));
        assert!(!scratch.0.join("absent").exists());

        config.state_root = scratch.0.join("other/state");
        config.workspace = config.state_root.join("workspace");
        assert!(matches!(
            config.start(
                Id("s".into()),
                Id("a".into()),
                "prompt",
                boundary_tests::limits()
            ),
            Err(AdapterError::UnsafeStateRoot)
        ));
        assert!(!scratch.0.join("other").exists());
    }
}
