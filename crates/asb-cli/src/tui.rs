// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Trusted router for the independently installed optional terminal frontend.

use crate::control::open_development_backend;
use crate::diagnostic;
use crate::{CliError, output_error, write_json};
use asb_control::{
    AuthenticatedGenerationProducer, BrokerConnection, BrokerState, ControlBackend, ControlLimits,
    ProvisionedControlServer,
};
use nix::sys::signal::{SigSet, SigmaskHow, Signal as NixSignal, pthread_sigmask};
use rustix::fs::{
    AtFlags, MemfdFlags, Mode, OFlags, SealFlags, fcntl_add_seals, fcntl_getfl, fcntl_setfl, fsync,
    memfd_create, mkdirat, open, openat, renameat, unlinkat,
};
use rustix::io::{FdFlags, fcntl_getfd, fcntl_setfd};
use rustix::process::{Pid, Signal, WaitId, WaitIdOptions, getpgrp, kill_process_group, waitid};
use rustix::termios::{tcgetpgrp, tcsetpgrp};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{self, IsTerminal, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const INDEX_URL: &str =
    "https://github.com/martin-beck/asb-tui/releases/download/channel-v1/stable-index.json";
const INDEX_SIGNATURE_URL: &str =
    "https://github.com/martin-beck/asb-tui/releases/download/channel-v1/stable-index.json.sig";
const CHANNEL_NAMESPACE: &str = "asb-tui-channel-v1";
const BUNDLE_NAMESPACE: &str = "asb-tui-bundle-v1";
const SIGNER_IDENTITY: &str = "martin.beck2@gmx.de";
const SIGNER_FINGERPRINT: &str = "SHA256:a36V6yPvRZyxnQ2113tiA/MlHt7mPfJEXAGByBXVkuE";
const TRUSTED_SIGNERS: &[u8] = include_bytes!("tui_allowed_signers");
const MAX_INDEX_BYTES: usize = 128 * 1024;
const MAX_MANIFEST_BYTES: usize = 128 * 1024;
const MAX_SIGNATURE_BYTES: usize = 16 * 1024;
const MAX_ARTIFACT_BYTES: u64 = 256 * 1024 * 1024;
const MAX_BUNDLE_BYTES: u64 = 512 * 1024 * 1024;
const CHUNK_BYTES: usize = 1024 * 1024;
const TRANSFER_ATTEMPTS: usize = 3;
const RESPONSE_BYTES: u64 = 16 * 1024;
const DELEGATE_TIMEOUT: Duration = Duration::from_secs(30);
const DEV_CONTROL_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(2);
const CURL: &str = "/usr/bin/curl";
const SSH_KEYGEN: &str = "/usr/bin/ssh-keygen";
const MAX_REDIRECTS: usize = 3;
const DEV_REPOSITORY_URL: &str = "https://github.com/martin-beck/asb-tui.git";
const ASB_REPOSITORY_URL: &str = "https://github.com/martin-beck/agent-systems-benchmark.git";
const DEVELOPMENT_SOURCE_REF: &str = "refs/heads/main";
const DEV_BROKER_DESCRIPTOR_ENV: &str = "ASB_TUI_DEVELOPMENT_DESCRIPTOR";
const DEV_BROKER_ASB_COMMIT_ENV: &str = "ASB_TUI_EXPECTED_ASB_SOURCE_COMMIT";
const DEV_BROKER_ASB_TREE_ENV: &str = "ASB_TUI_EXPECTED_ASB_SOURCE_TREE";
const DEV_BROKER_TUI_COMMIT_ENV: &str = "ASB_TUI_EXPECTED_TUI_SOURCE_COMMIT";
const DEV_BROKER_TUI_TREE_ENV: &str = "ASB_TUI_EXPECTED_TUI_SOURCE_TREE";
const TUI_DEV_GIT_ENV: &str = "ASB_TUI_DEV_GIT";
const TUI_DEV_CARGO_ENV: &str = "ASB_TUI_DEV_CARGO";
const TUI_DEV_RUSTC_ENV: &str = "ASB_TUI_DEV_RUSTC";
const TUI_DEV_SETSID_ENV: &str = "ASB_TUI_DEV_SETSID";
const TUI_DEV_CC_ENV: &str = "ASB_TUI_DEV_CC";
const TUI_DEV_AR_ENV: &str = "ASB_TUI_DEV_AR";
const TUI_DEV_LD_ENV: &str = "ASB_TUI_DEV_LD";
const TUI_DEV_RUSTUP_HOME_ENV: &str = "ASB_TUI_DEV_RUSTUP_HOME";
const DEV_BROKER_PROTOCOL_MINOR: u64 = 10;
const UNIX_SOCKET_PATH_LIMIT: usize = 108;
const DEV_GIT: &str = "/usr/bin/git";
const DEV_SETSID: &str = "/usr/bin/setsid";
// Development-only override; stable release paths never consult this variable.
const DEV_CARGO_OVERRIDE: &str = "ASB_DEV_CARGO";
const DEV_GIT_OVERRIDE: &str = "ASB_DEV_GIT";
const DEV_SETSID_OVERRIDE: &str = "ASB_DEV_SETSID";
const DEV_CC_OVERRIDE: &str = "ASB_DEV_CC";
const DEV_AR_OVERRIDE: &str = "ASB_DEV_AR";
const DEV_LD_OVERRIDE: &str = "ASB_DEV_LD";
const DEV_CARGO_HOME: &str = "CARGO_HOME";
const DEV_RUSTUP_HOME: &str = "ASB_DEV_RUSTUP_HOME";
const DEV_BUNDLE_OVERRIDE: &str = "ASB_TUI_DEV_BUNDLE";
const GROUP_WRITABLE_RUSTUP_PATH_WARNING: &str =
    "development_rustup_permission_or_ownership_findings_allowed";
mod build_identity {
    include!(concat!(env!("OUT_DIR"), "/asb_source_identity.rs"));
}
const ASB_SOURCE_COMMIT: &str = build_identity::COMMIT;
const ASB_SOURCE_TREE: &str = build_identity::TREE;
const DEV_COMMAND_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const MAX_DEV_COMMAND_OUTPUT: usize = 128 * 1024;
const MAX_DEV_COMMAND_DRAIN_READS: usize = 16;
const MAX_DEV_WORKSPACE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Operation {
    Preflight,
    Install,
    Upgrade,
    Status,
    Doctor,
    Remove,
    Launch,
    LiveProvider,
    DynamicCatalog,
    Help,
    Version,
}

impl Operation {
    const fn name(self) -> &'static str {
        match self {
            Self::Preflight => "preflight",
            Self::Install => "install",
            Self::Upgrade => "upgrade",
            Self::Status => "status",
            Self::Doctor => "doctor",
            Self::Remove => "remove",
            Self::Launch => "launch",
            Self::LiveProvider => "live_provider",
            Self::DynamicCatalog => "dynamic_catalog",
            Self::Help => "help",
            Self::Version => "version",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Options {
    channel: Channel,
    channel_explicit: bool,
    offline: bool,
    dry_run: bool,
    launch: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum Channel {
    #[default]
    Dev,
    Stable,
    Nightly,
    Experimental,
}

impl Channel {
    const fn name(self) -> &'static str {
        match self {
            Self::Dev => "dev",
            Self::Stable => "stable",
            Self::Nightly => "nightly",
            Self::Experimental => "experimental",
        }
    }

    fn parse(value: &str) -> Result<Self, CliError> {
        match value {
            "dev" => Ok(Self::Dev),
            "stable" => Ok(Self::Stable),
            "nightly" => Ok(Self::Nightly),
            "experimental" => Ok(Self::Experimental),
            _ => Err(CliError::legacy_usage("unsupported asb tui channel")),
        }
    }
}

#[derive(Debug)]
struct RouterError {
    code: &'static str,
    exit_code: u8,
}

#[derive(Debug)]
struct DevelopmentCargo {
    path: PathBuf,
    bound_file: Option<File>,
    bound_rustup_bin: Option<File>,
    group_writable_rustup_paths: bool,
}

#[derive(Debug)]
struct DevelopmentTool {
    path: PathBuf,
    bound_file: Option<File>,
}

#[derive(Debug)]
struct DevelopmentLinkerPrefix {
    directory: File,
    linker: File,
}

#[derive(Debug)]
struct DevelopmentLaunchTools {
    git: PathBuf,
    setsid: PathBuf,
    cc: PathBuf,
    ar: PathBuf,
    ld: PathBuf,
    rustup_home: Option<PathBuf>,
    cargo: DevelopmentCargo,
    rustc: Option<DevelopmentTool>,
}

impl DevelopmentLaunchTools {
    fn resolve() -> Result<Self, RouterError> {
        let git = resolve_development_tool(DEV_GIT_OVERRIDE, DEV_GIT)?;
        let setsid = resolve_development_tool(DEV_SETSID_OVERRIDE, DEV_SETSID)?;
        let cc = resolve_development_tool_candidates(
            DEV_CC_OVERRIDE,
            &["/usr/bin/cc", "/usr/local/bin/cc"],
        )?;
        let ar = resolve_development_tool_candidates(
            DEV_AR_OVERRIDE,
            &["/usr/bin/ar", "/usr/local/bin/ar"],
        )?;
        let ld = resolve_development_tool_candidates(
            DEV_LD_OVERRIDE,
            &["/usr/bin/ld", "/usr/local/bin/ld"],
        )?;
        let rustup_home = resolve_development_rustup_home()?;
        let cargo = resolve_development_cargo_with_rustup(rustup_home.as_deref())?;
        let rustc = resolve_development_rustc_bound(
            &cargo,
            rustup_home.as_deref(),
            std::env::var_os(DEV_CARGO_OVERRIDE).is_some(),
        )?;
        Ok(Self {
            git,
            setsid,
            cc,
            ar,
            ld,
            rustup_home,
            cargo,
            rustc,
        })
    }

    fn apply(&self, command: &mut Command) {
        command
            .env_remove("PATH")
            .env("LANG", "C.UTF-8")
            .env(TUI_DEV_GIT_ENV, &self.git)
            .env(TUI_DEV_CARGO_ENV, self.cargo.execution_path())
            .env(TUI_DEV_SETSID_ENV, &self.setsid)
            .env(TUI_DEV_CC_ENV, &self.cc)
            .env(TUI_DEV_AR_ENV, &self.ar)
            .env(TUI_DEV_LD_ENV, &self.ld);
        if let Some(rustc) = &self.rustc {
            command.env(TUI_DEV_RUSTC_ENV, rustc.execution_path());
        }
        if let Some(rustup_home) = &self.rustup_home {
            command.env(TUI_DEV_RUSTUP_HOME_ENV, rustup_home);
        }
    }

    fn make_descriptors_inheritable(
        &self,
    ) -> Result<(Option<FdFlags>, Option<FdFlags>), RouterError> {
        make_toolchain_descriptors_inheritable(
            self.cargo.bound_file.as_ref(),
            self.rustc
                .as_ref()
                .and_then(|tool| tool.bound_file.as_ref()),
        )
    }

    fn restore_descriptor_flags(
        &self,
        flags: (Option<FdFlags>, Option<FdFlags>),
    ) -> Result<(), RouterError> {
        restore_toolchain_descriptor_flags(
            self.cargo.bound_file.as_ref(),
            self.rustc
                .as_ref()
                .and_then(|tool| tool.bound_file.as_ref()),
            flags,
        )
    }
}

impl DevelopmentTool {
    fn execution_path(&self) -> PathBuf {
        self.bound_file
            .as_ref()
            .map(descriptor_path)
            .unwrap_or_else(|| self.path.clone())
    }
}

impl DevelopmentLinkerPrefix {
    fn search_root(&self) -> PathBuf {
        descriptor_path(&self.directory)
    }

    fn linker_path(&self) -> PathBuf {
        self.search_root().join("ld")
    }

    fn validate(&self) -> Result<(), RouterError> {
        let uid = rustix::process::geteuid().as_raw();
        let directory = self
            .directory
            .metadata()
            .map_err(|_| RouterError::operation("trusted_tool_unavailable"))?;
        if !directory.is_dir() || directory.uid() != uid || directory.mode() & 0o777 != 0o500 {
            return Err(RouterError::policy("trusted_tool_invalid"));
        }
        let mut entries = fs::read_dir(self.search_root())
            .map_err(|_| RouterError::operation("trusted_tool_unavailable"))?;
        let entry = entries
            .next()
            .transpose()
            .map_err(|_| RouterError::operation("trusted_tool_unavailable"))?
            .ok_or_else(|| RouterError::policy("trusted_tool_invalid"))?;
        if entry.file_name() != std::ffi::OsStr::new("ld")
            || !entry
                .file_type()
                .map_err(|_| RouterError::operation("trusted_tool_unavailable"))?
                .is_file()
            || entries.next().is_some()
        {
            return Err(RouterError::policy("trusted_tool_invalid"));
        }
        let opened = openat(
            &self.directory,
            "ld",
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map(File::from)
        .map_err(|_| RouterError::operation("trusted_tool_unavailable"))?;
        let opened = opened
            .metadata()
            .map_err(|_| RouterError::operation("trusted_tool_unavailable"))?;
        let retained = self
            .linker
            .metadata()
            .map_err(|_| RouterError::operation("trusted_tool_unavailable"))?;
        if !opened.is_file()
            || opened.uid() != uid
            || opened.mode() & 0o777 != 0o500
            || opened.len() == 0
            || opened.len() > MAX_ARTIFACT_BYTES
            || opened.dev() != retained.dev()
            || opened.ino() != retained.ino()
        {
            return Err(RouterError::policy("trusted_tool_invalid"));
        }
        Ok(())
    }
}

impl Drop for DevelopmentLinkerPrefix {
    fn drop(&mut self) {
        // The prefix is non-writable for the entire child lifetime. Restore
        // owner write only after the retained descriptors leave that boundary
        // so the enclosing private build root can be removed deterministically.
        let _ = self
            .directory
            .set_permissions(fs::Permissions::from_mode(0o700));
    }
}

impl DevelopmentCargo {
    fn execution_path(&self) -> PathBuf {
        self.bound_file
            .as_ref()
            .map(descriptor_path)
            .unwrap_or_else(|| self.path.clone())
    }
}

impl RouterError {
    const fn policy(code: &'static str) -> Self {
        Self { code, exit_code: 3 }
    }

    const fn operation(code: &'static str) -> Self {
        Self { code, exit_code: 4 }
    }

    /// Resolve routed lifecycle failures through the same closed catalog as
    /// ordinary CLI failures. The existing JSON response intentionally keeps
    /// its stable `classification` and `remediation` fields; this typed value
    /// is the authority used by human/detail presentation and contract tests.
    fn diagnostic(&self) -> diagnostic::Diagnostic {
        let severity = if self.exit_code == 3 {
            diagnostic::Severity::Error
        } else {
            diagnostic::Severity::Failure
        };
        diagnostic::Diagnostic::for_code(self.code, self.code, severity)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RouterResponse {
    schema_version: u64,
    ok: bool,
    command: &'static str,
    operation: &'static str,
    code: &'static str,
    network: &'static str,
    channel: &'static str,
    development_only: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    classification: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    remediation: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    release: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    executable_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_commit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_tree: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    asb_source_commit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    asb_source_tree: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    channel_manifest_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    warnings: Option<Vec<&'static str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    verified: Option<bool>,
}

impl RouterResponse {
    fn result(operation: Operation, ok: bool, code: &'static str, network: &'static str) -> Self {
        Self {
            schema_version: 1,
            ok,
            command: "tui",
            operation: operation.name(),
            code,
            network,
            channel: "dev",
            development_only: true,
            classification: None,
            remediation: None,
            release: None,
            executable_sha256: None,
            source_commit: None,
            source_tree: None,
            asb_source_commit: None,
            asb_source_tree: None,
            channel_manifest_sha256: None,
            warnings: None,
            verified: None,
        }
    }
}

#[derive(Clone, Debug)]
struct RouterPaths {
    install_root: PathBuf,
    state_root: PathBuf,
    cache_root: PathBuf,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChannelIndex {
    schema_version: u64,
    issued_unix: u64,
    expires_unix: u64,
    releases: Vec<ChannelRelease>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChannelRelease {
    release: String,
    target: String,
    asb_version: String,
    protocol_version: u64,
    manifest_url: String,
    manifest_signature_url: String,
    manifest_sha256: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BundleManifest {
    schema_version: u64,
    release: String,
    source_commit: String,
    source_tree: String,
    issued_unix: u64,
    expires_unix: u64,
    compatibility: BundleCompatibility,
    components: Vec<BundleComponent>,
    artifacts: Vec<BundleArtifact>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BundleCompatibility {
    bundle: String,
    architecture: String,
    asb_version: String,
    protocol_version: u64,
    coordinator_version: String,
    coordinator_commit: String,
    quality_version: String,
    quality_commit: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BundleComponent {
    name: String,
    version: String,
    commit: String,
    tree: String,
    artifact_sha256: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BundleArtifact {
    name: String,
    kind: String,
    url: String,
    size: u64,
    sha256: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ActiveInstallation {
    schema_version: u64,
    release: String,
    executable_sha256: String,
    source_commit: String,
    source_tree: String,
    target: String,
    bundle: String,
    asb_version: String,
    protocol_version: u64,
    coordinator_version: String,
    coordinator_commit: String,
    quality_version: String,
    quality_commit: String,
    classification: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct DevelopmentInstallation {
    schema_version: u64,
    channel: String,
    development_only: bool,
    source_repository: String,
    source_commit: String,
    source_tree: String,
    asb_source_commit: String,
    asb_source_tree: String,
    executable_sha256: String,
    installed_unix: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DevelopmentBundleManifest {
    schema_version: u64,
    channel: String,
    development_only: bool,
    source_repository: String,
    source_ref: String,
    source_commit: String,
    source_tree: String,
    asb_source_commit: String,
    asb_source_tree: String,
    target: String,
    executable_sha256: String,
    executable_size: u64,
    built_unix: u64,
    warnings: Vec<String>,
}

/// The development-channel handoff manifest is diagnostic provenance, not a
/// production trust assertion.  Its content-addressed digest lets the TUI
/// distinguish the exact materialized pair from a stale or edited pointer.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct DevelopmentChannelManifest {
    schema_version: u64,
    channel: String,
    development_only: bool,
    asb_repository: String,
    asb_ref: String,
    asb_source_commit: String,
    asb_source_tree: String,
    tui_repository: String,
    tui_ref: String,
    tui_source_commit: String,
    tui_source_tree: String,
    executable_sha256: String,
    executable_size: u64,
    built_unix: u64,
    warnings: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DelegatedResponse {
    schema_version: u64,
    classification: String,
    ok: bool,
    code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    installed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    verified: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    release: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    executable_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bundle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    asb_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    protocol_version: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_commit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_tree: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct AcceptedRelease {
    schema_version: u64,
    release: String,
    manifest_sha256: String,
}

trait Source {
    fn document(&mut self, url: &str, maximum: usize) -> Result<Vec<u8>, RouterError>;
    fn range(&mut self, url: &str, offset: u64, maximum: usize) -> Result<Vec<u8>, RouterError>;
}

struct CurlSource;

impl CurlSource {
    fn command(url: &str) -> Result<Command, RouterError> {
        validate_tool(CURL)?;
        let mut command = Command::new(CURL);
        command.env_clear().env("LANG", "C.UTF-8");
        command.args([
            "--disable",
            "--fail",
            "--silent",
            "--show-error",
            "--proto",
            "=https",
            "--tlsv1.2",
            "--max-redirs",
            "0",
            "--connect-timeout",
            "5",
            "--max-time",
            "30",
        ]);
        command.arg(url);
        Ok(command)
    }

    fn bounded_output(mut command: Command, maximum: usize) -> Result<Vec<u8>, RouterError> {
        command.stdout(Stdio::piped()).stderr(Stdio::null());
        let mut child = command
            .spawn()
            .map_err(|_| RouterError::operation("transfer_unavailable"))?;
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| RouterError::operation("transfer_unavailable"))?;
        let mut bytes = Vec::new();
        stdout
            .by_ref()
            .take(maximum as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| RouterError::operation("transfer_failed"))?;
        if bytes.len() > maximum {
            let _ = child.kill();
            let _ = child.wait();
            return Err(RouterError::policy("transfer_too_large"));
        }
        let status = child
            .wait()
            .map_err(|_| RouterError::operation("transfer_failed"))?;
        status
            .success()
            .then_some(bytes)
            .ok_or_else(|| RouterError::operation("transfer_failed"))
    }
}

impl Source for CurlSource {
    fn document(&mut self, url: &str, maximum: usize) -> Result<Vec<u8>, RouterError> {
        let resolved = resolve_redirects(url)?;
        Self::bounded_output(Self::command(&resolved)?, maximum)
    }

    fn range(&mut self, url: &str, offset: u64, maximum: usize) -> Result<Vec<u8>, RouterError> {
        let end = offset.saturating_add(maximum as u64).saturating_sub(1);
        let resolved = resolve_redirects(url)?;
        let mut command = Self::command(&resolved)?;
        command.args(["--range", &format!("{offset}-{end}")]);
        Self::bounded_output(command, maximum)
    }
}

pub(crate) fn dispatch(
    args: &[String],
    output: &mut dyn Write,
    progress: &mut dyn Write,
    human: bool,
) -> Result<u8, CliError> {
    let (operation, options) = parse(args)?;
    if operation == Operation::Help {
        writeln!(
            output,
            "Usage: asb tui [preflight|launch|live-provider|dynamic-catalog|status|doctor|remove|install|upgrade] [--channel dev|stable|nightly|experimental]\nFresh installs default to the development channel; existing installations preserve their active channel."
        )
        .map_err(output_error)?;
        return Ok(0);
    }
    if operation == Operation::Version {
        writeln!(output, "asb tui-router {}", env!("CARGO_PKG_VERSION")).map_err(output_error)?;
        return Ok(0);
    }
    let mut source = CurlSource;
    // Resolve the active channel before executing the operation so that an
    // error response carries the same channel as the attempted operation.
    // In particular, omitted operations must preserve an active development
    // installation instead of being re-annotated as stable after launch
    // fails during terminal handoff.
    let mut resolved_channel = options.channel;
    let response = match RouterPaths::environment().and_then(|paths| {
        resolved_channel = if matches!(operation, Operation::Install | Operation::Upgrade) {
            options.channel
        } else {
            existing_channel(options, &paths).unwrap_or(options.channel)
        };
        execute(
            operation,
            options,
            &paths,
            &mut source,
            system_now_unix()?,
            progress,
            human,
        )
    }) {
        Ok(response) => response,
        Err(error) => {
            let typed_diagnostic = error.diagnostic();
            debug_assert_eq!(typed_diagnostic.code, error.code);
            let network = if matches!(operation, Operation::Install | Operation::Upgrade)
                && !options.offline
            {
                "used"
            } else {
                "denied"
            };
            let mut response = RouterResponse::result(operation, false, error.code, network);
            response.classification = Some(error.classification());
            response.remediation = error.remediation();
            annotate_channel(&mut response, resolved_channel);
            write_json(output, &response)?;
            return Ok(error.exit_code);
        }
    };
    let mut response = response;
    annotate_channel(&mut response, resolved_channel);
    let exit = if response.ok { 0 } else { 3 };
    write_json(output, &response)?;
    Ok(exit)
}

impl RouterError {
    fn classification(&self) -> &'static str {
        match self.code {
            "trusted_tool_invalid" | "trusted_tool_unavailable" => "host_limitation",
            _ => "product_failure",
        }
    }

    fn remediation(&self) -> Option<&'static str> {
        match self.code {
            "trusted_tool_invalid" => {
                Some("repair_toolchain_permissions_or_set_private_ASB_DEV_RUSTUP_HOME")
            }
            "trusted_tool_unavailable" => {
                Some("install_a_supported_rust_toolchain_or_set_ASB_DEV_CARGO")
            }
            _ => None,
        }
    }
}

fn annotate_channel(response: &mut RouterResponse, resolved: Channel) {
    response.channel = resolved.name();
    response.development_only = resolved == Channel::Dev;
}

fn parse(args: &[String]) -> Result<(Operation, Options), CliError> {
    let (operation, flags) = match args.first().map(String::as_str) {
        None => (Operation::Launch, &args[0..]),
        Some("preflight") => (Operation::Preflight, &args[1..]),
        Some("launch") => (Operation::Launch, &args[1..]),
        Some("live-provider") => (Operation::LiveProvider, &args[1..]),
        Some("dynamic-catalog") => (Operation::DynamicCatalog, &args[1..]),
        Some("status") => (Operation::Status, &args[1..]),
        Some("doctor") => (Operation::Doctor, &args[1..]),
        Some("remove") => (Operation::Remove, &args[1..]),
        Some("install") => (Operation::Install, &args[1..]),
        Some("upgrade") => (Operation::Upgrade, &args[1..]),
        Some("--help") | Some("-h") | Some("help") => (Operation::Help, &args[1..]),
        Some("--version") | Some("-V") => (Operation::Version, &args[1..]),
        _ => return Err(CliError::legacy_usage("unsupported asb tui arguments")),
    };
    let mut options = Options::default();
    let mut channel_seen = false;
    let mut index = 0;
    while index < flags.len() {
        let flag = &flags[index];
        if flag == "--channel" {
            if channel_seen {
                return Err(CliError::legacy_usage("duplicate asb tui option"));
            }
            channel_seen = true;
            options.channel_explicit = true;
            index += 1;
            let value = flags
                .get(index)
                .ok_or_else(|| CliError::legacy_usage("--channel requires a value"))?;
            options.channel = Channel::parse(value)?;
            index += 1;
            continue;
        }
        let slot = match flag.as_str() {
            "--offline" if matches!(operation, Operation::Install | Operation::Upgrade) => {
                &mut options.offline
            }
            "--dry-run" if matches!(operation, Operation::Install | Operation::Upgrade) => {
                &mut options.dry_run
            }
            "--launch" if matches!(operation, Operation::Install | Operation::Upgrade) => {
                &mut options.launch
            }
            _ => return Err(CliError::legacy_usage("unsupported asb tui arguments")),
        };
        if *slot {
            return Err(CliError::legacy_usage("duplicate asb tui option"));
        }
        *slot = true;
        index += 1;
    }
    if options.dry_run && options.launch {
        return Err(CliError::legacy_usage(
            "--dry-run and --launch cannot be used together",
        ));
    }
    Ok((operation, options))
}

impl RouterPaths {
    fn environment() -> Result<Self, RouterError> {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let root = |name: &str, fallback: &str| match std::env::var_os(name) {
            Some(value) if !value.is_empty() => Ok(PathBuf::from(value)),
            _ => home
                .as_ref()
                .map(|value| value.join(fallback))
                .ok_or_else(|| RouterError::policy("xdg_root_invalid")),
        };
        Self::from_roots(
            &root("XDG_DATA_HOME", ".local/share")?,
            &root("XDG_STATE_HOME", ".local/state")?,
            &root("XDG_CACHE_HOME", ".cache")?,
        )
    }

    fn from_roots(data: &Path, state: &Path, cache: &Path) -> Result<Self, RouterError> {
        if !safe_absolute(data) || !safe_absolute(state) || !safe_absolute(cache) {
            return Err(RouterError::policy("xdg_root_invalid"));
        }
        Ok(Self {
            install_root: data.join("asb/extensions/asb-tui"),
            state_root: state.join("asb/extensions/asb-tui"),
            cache_root: cache.join("asb/asb-tui"),
        })
    }
}

fn execute(
    operation: Operation,
    options: Options,
    paths: &RouterPaths,
    source: &mut impl Source,
    now: u64,
    progress: &mut dyn Write,
    human: bool,
) -> Result<RouterResponse, RouterError> {
    match operation {
        Operation::Install | Operation::Upgrade => {
            if options.channel == Channel::Dev {
                prepare_private_directory_with_notice(
                    &paths.state_root,
                    "ASB TUI lifecycle state",
                    progress,
                    human,
                )?;
                let lock = open_lock(&paths.state_root.join("router.lock"))?;
                lock.try_lock()
                    .map_err(|_| RouterError::operation("lifecycle_busy"))?;
                return materialize_development(operation, options, paths, now);
            }
            if options.channel != Channel::Stable {
                return Err(RouterError::policy("channel_unavailable"));
            }
            prepare_private_directory_with_notice(
                &paths.cache_root,
                "ASB TUI release cache",
                progress,
                human,
            )?;
            prepare_private_directory_with_notice(
                &paths.state_root,
                "ASB TUI lifecycle state",
                progress,
                human,
            )?;
            install_or_upgrade(operation, options, paths, source, now)
        }
        Operation::Preflight => preflight_development(),
        Operation::Status
        | Operation::Remove
        | Operation::Launch
        | Operation::LiveProvider
        | Operation::DynamicCatalog => {
            if existing_channel(options, paths)? == Channel::Dev {
                if operation == Operation::Remove {
                    // Keep an absent default-dev installation read-only; a
                    // no-op remove must not create XDG state directories.
                    if development_active(paths)?.is_some() {
                        prepare_private_directory_with_notice(
                            &paths.state_root,
                            "ASB TUI lifecycle state",
                            progress,
                            human,
                        )?;
                        let lock = open_lock(&paths.state_root.join("router.lock"))?;
                        lock.try_lock()
                            .map_err(|_| RouterError::operation("lifecycle_busy"))?;
                    }
                }
                execute_development_existing(operation, paths)
            } else {
                delegate_existing(operation, paths)
            }
        }
        Operation::Doctor => {
            if existing_channel(options, paths)? == Channel::Dev {
                match development_active(paths)? {
                    Some((active, _)) => Ok(development_response(
                        Operation::Doctor,
                        "development_verified",
                        &active,
                    )),
                    None => Ok(RouterResponse::result(
                        Operation::Doctor,
                        true,
                        "extension_not_installed",
                        "denied",
                    )),
                }
            } else {
                doctor(paths)
            }
        }
        Operation::Help | Operation::Version => unreachable!(),
    }
}

/// Resolve an existing installation's channel. An omitted channel preserves
/// the active installation; an explicit unavailable channel never falls back
/// to another installation or silently changes provenance.
fn existing_channel(options: Options, paths: &RouterPaths) -> Result<Channel, RouterError> {
    let has_development = development_active(paths)?.is_some();
    if options.channel_explicit {
        return match options.channel {
            Channel::Dev if has_development => Ok(Channel::Dev),
            Channel::Stable if !has_development => Ok(Channel::Stable),
            _ => Err(RouterError::policy("channel_unavailable")),
        };
    }
    // Omission means dev for a fresh checkout. Once a verified stable marker
    // exists, preserve that active channel for status/launch/remove.
    if has_development {
        Ok(Channel::Dev)
    } else if paths.install_root.join("active.json").is_file() {
        Ok(Channel::Stable)
    } else {
        Ok(Channel::Dev)
    }
}

fn development_source_identity() -> Result<(&'static str, &'static str), RouterError> {
    development_source_identity_from(ASB_SOURCE_COMMIT, ASB_SOURCE_TREE)
}

fn development_source_identity_from(
    commit: &'static str,
    tree: &'static str,
) -> Result<(&'static str, &'static str), RouterError> {
    if valid_hex(commit, 40) && valid_hex(tree, 40) {
        Ok((commit, tree))
    } else {
        Err(RouterError::policy("dev_source_identity_unknown"))
    }
}

/// Resolve an immutable repository main head through the bounded development
/// toolchain.  This is deliberately used only by the development channel;
/// stable lifecycle verification retains its signed release-index policy.
fn resolve_development_main_head(
    git: &Path,
    setsid: &Path,
    repository: &str,
    root: &Path,
) -> Result<String, RouterError> {
    let mut command = Command::new(setsid);
    command
        .env_clear()
        .env("LANG", "C.UTF-8")
        .current_dir(root)
        .args(["--wait"])
        .arg(git)
        .args(["ls-remote", repository, DEVELOPMENT_SOURCE_REF]);
    let output = run_development_command(command, root)?;
    let text = String::from_utf8(output)
        .map_err(|_| RouterError::policy("dev_source_identity_invalid"))?;
    let mut fields = text.split_whitespace();
    let commit = fields
        .next()
        .filter(|value| valid_hex(value, 40))
        .ok_or_else(|| RouterError::policy("dev_source_identity_invalid"))?;
    if fields.next() != Some(DEVELOPMENT_SOURCE_REF) || fields.next().is_some() {
        return Err(RouterError::policy("dev_source_identity_invalid"));
    }
    Ok(commit.to_owned())
}

fn development_channel_manifest(
    asb_source_commit: &str,
    asb_source_tree: &str,
    tui_source_commit: &str,
    tui_source_tree: &str,
    executable_sha256: &str,
    executable_size: u64,
    built_unix: u64,
) -> DevelopmentChannelManifest {
    DevelopmentChannelManifest {
        schema_version: 1,
        channel: "dev".to_owned(),
        development_only: true,
        asb_repository: ASB_REPOSITORY_URL.to_owned(),
        asb_ref: DEVELOPMENT_SOURCE_REF.to_owned(),
        asb_source_commit: asb_source_commit.to_owned(),
        asb_source_tree: asb_source_tree.to_owned(),
        tui_repository: DEV_REPOSITORY_URL.to_owned(),
        tui_ref: DEVELOPMENT_SOURCE_REF.to_owned(),
        tui_source_commit: tui_source_commit.to_owned(),
        tui_source_tree: tui_source_tree.to_owned(),
        executable_sha256: executable_sha256.to_owned(),
        executable_size,
        built_unix,
        warnings: development_warnings()
            .into_iter()
            .map(str::to_owned)
            .collect(),
    }
}

fn encode_channel_manifest(
    manifest: &DevelopmentChannelManifest,
) -> Result<(Vec<u8>, String), RouterError> {
    let bytes =
        serde_json::to_vec(manifest).map_err(|_| RouterError::operation("dev_metadata_failed"))?;
    Ok((bytes.clone(), digest(&bytes)))
}

fn publish_development_channel_manifest(
    install_root: &Path,
    executable_sha256: &str,
    manifest: &DevelopmentChannelManifest,
) -> Result<String, RouterError> {
    let (bytes, manifest_sha256) = encode_channel_manifest(manifest)?;
    let version = install_root.join("dev-versions").join(executable_sha256);
    atomic_private(&version.join("channel-manifest.json"), &bytes, 0o600)?;
    atomic_private(&install_root.join("active-channel.json"), &bytes, 0o600)?;
    Ok(manifest_sha256)
}

/// Keep compiler paths independent from the per-install random staging roots.
/// Cargo/Rust otherwise embeds those absolute paths in release debug metadata.
/// The validated linker root shares this one effective Cargo-to-rustc channel;
/// setting both global and target-specific flags makes Cargo ignore the latter.
fn development_encoded_rustflags(
    root: &Path,
    target: &Path,
    cargo_home: &Path,
    linker_prefix: &DevelopmentLinkerPrefix,
) -> Result<std::ffi::OsString, RouterError> {
    linker_prefix.validate()?;
    let search_root = linker_prefix.search_root();
    let search_root = search_root
        .to_str()
        .ok_or_else(|| RouterError::policy("trusted_tool_invalid"))?;
    let arguments = [
        development_remap_argument(root, "/asb-dev/workspace")?,
        development_remap_argument(target, "/asb-dev/target")?,
        development_remap_argument(cargo_home, "/asb-dev/cargo-home")?,
        "-C".to_owned(),
        format!("link-arg=-B{search_root}"),
    ];
    let mut encoded = Vec::new();
    for (index, argument) in arguments.iter().enumerate() {
        if argument.as_bytes().contains(&0) || argument.as_bytes().contains(&0x1f) {
            return Err(RouterError::policy("trusted_tool_invalid"));
        }
        if index != 0 {
            encoded.push(0x1f);
        }
        encoded.extend_from_slice(argument.as_bytes());
    }
    Ok(std::ffi::OsString::from_vec(encoded))
}

/// Copy exactly one validated linker into a fresh private GCC program prefix.
/// GCC gives `-B` broad helper and library semantics, so the validated linker
/// source directory must never be supplied to the child. The directory fd
/// also keeps later lookup on the opened directory if its visible path moves.
fn materialize_development_linker_prefix(
    root: &Path,
    ld: &Path,
) -> Result<DevelopmentLinkerPrefix, RouterError> {
    let validated =
        validate_development_tool(ld).map_err(|_| RouterError::policy("trusted_tool_invalid"))?;
    if validated != ld || validated.to_str().is_none() {
        return Err(RouterError::policy("trusted_tool_invalid"));
    }
    let path_metadata = fs::symlink_metadata(&validated)
        .map_err(|_| RouterError::operation("trusted_tool_unavailable"))?;
    let mut source = open(
        &validated,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(|_| RouterError::operation("trusted_tool_unavailable"))?;
    let source_metadata = source
        .metadata()
        .map_err(|_| RouterError::operation("trusted_tool_unavailable"))?;
    let uid = rustix::process::geteuid().as_raw();
    if !source_metadata.is_file()
        || (source_metadata.uid() != 0 && source_metadata.uid() != uid)
        || source_metadata.mode() & 0o022 != 0
        || source_metadata.mode() & 0o111 == 0
        || source_metadata.len() == 0
        || source_metadata.len() > MAX_ARTIFACT_BYTES
        || source_metadata.dev() != path_metadata.dev()
        || source_metadata.ino() != path_metadata.ino()
    {
        return Err(RouterError::policy("trusted_tool_invalid"));
    }

    let prefix_path = root.join("linker-prefix");
    prepare_private_directory(&prefix_path)?;
    let directory = open_private_directory(&prefix_path, false)?;
    let mut output = openat(
        &directory,
        "ld",
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o500),
    )
    .map(File::from)
    .map_err(|_| RouterError::operation("trusted_tool_unavailable"))?;
    let copied = io::copy(&mut source, &mut output)
        .map_err(|_| RouterError::operation("trusted_tool_unavailable"))?;
    if copied != source_metadata.len() || output.sync_all().is_err() {
        return Err(RouterError::operation("trusted_tool_unavailable"));
    }
    output
        .set_permissions(fs::Permissions::from_mode(0o500))
        .map_err(|_| RouterError::operation("trusted_tool_unavailable"))?;
    drop(output);
    directory
        .set_permissions(fs::Permissions::from_mode(0o500))
        .map_err(|_| RouterError::operation("trusted_tool_unavailable"))?;
    fsync(&directory).map_err(|_| RouterError::operation("trusted_tool_unavailable"))?;
    let linker = openat(
        &directory,
        "ld",
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(|_| RouterError::operation("trusted_tool_unavailable"))?;
    let prefix = DevelopmentLinkerPrefix { directory, linker };
    prefix.validate()?;
    Ok(prefix)
}

fn validate_development_gcc_driver(
    cc: &Path,
    linker_prefix: &DevelopmentLinkerPrefix,
    root: &Path,
) -> Result<(), RouterError> {
    validate_development_gcc_driver_with_timeout(cc, linker_prefix, root, Duration::from_secs(5))
}

fn validate_development_gcc_driver_with_timeout(
    cc: &Path,
    linker_prefix: &DevelopmentLinkerPrefix,
    root: &Path,
    timeout: Duration,
) -> Result<(), RouterError> {
    linker_prefix.validate()?;
    let inherited = make_descriptor_inheritable(&linker_prefix.directory)?;
    let search_argument = format!("-B{}", linker_prefix.search_root().display());
    let mut command = Command::new(cc);
    command
        .env_clear()
        .env("LANG", "C.UTF-8")
        .args([search_argument.as_str(), "-print-prog-name=collect2"])
        .process_group(0);
    let probe =
        run_development_command_with_limits(command, root, timeout, MAX_DEV_WORKSPACE_BYTES);
    let restore = restore_descriptor_flags(&linker_prefix.directory, inherited);
    restore?;
    let output = probe.map_err(|_| RouterError::policy("development_compiler_unsupported"))?;
    let output = std::str::from_utf8(&output)
        .map_err(|_| RouterError::policy("development_compiler_unsupported"))?
        .trim();
    if output.is_empty() || output.split_whitespace().count() != 1 {
        return Err(RouterError::policy("development_compiler_unsupported"));
    }
    let collect2 = Path::new(output);
    if !safe_absolute(collect2) || collect2.starts_with(linker_prefix.search_root()) {
        return Err(RouterError::policy("development_compiler_unsupported"));
    }
    validate_development_tool(collect2)
        .map(|_| ())
        .map_err(|_| RouterError::policy("development_compiler_unsupported"))
}

fn development_remap_argument(path: &Path, destination: &str) -> Result<String, RouterError> {
    if !safe_absolute(path) {
        return Err(RouterError::policy("trusted_tool_invalid"));
    }
    let source = path
        .to_str()
        .ok_or_else(|| RouterError::policy("trusted_tool_invalid"))?;
    Ok(format!("--remap-path-prefix={source}={destination}"))
}

/// Check the bounded development host before starting a clone or build.  This
/// is deliberately read-only: it reports host limitations with remediation
/// while retaining the trust policy used by the materializer.
fn preflight_development() -> Result<RouterResponse, RouterError> {
    let paths = RouterPaths::environment()?;
    if !safe_absolute(&paths.cache_root)
        || !safe_absolute(&paths.install_root)
        || !safe_absolute(&paths.state_root)
    {
        return Err(RouterError::policy("development_filesystem_invalid"));
    }
    development_source_identity()?;
    resolve_development_tool(DEV_GIT_OVERRIDE, DEV_GIT)?;
    resolve_development_tool(DEV_SETSID_OVERRIDE, DEV_SETSID)?;
    resolve_development_tool_candidates(DEV_CC_OVERRIDE, &["/usr/bin/cc", "/usr/local/bin/cc"])?;
    resolve_development_tool_candidates(DEV_AR_OVERRIDE, &["/usr/bin/ar", "/usr/local/bin/ar"])?;
    resolve_development_tool_candidates(DEV_LD_OVERRIDE, &["/usr/bin/ld", "/usr/local/bin/ld"])?;
    let rustup_home = resolve_development_rustup_home()?;
    let cargo = resolve_development_cargo_with_rustup(rustup_home.as_deref())?;
    let _ = resolve_development_rustc_bound(
        &cargo,
        rustup_home.as_deref(),
        std::env::var_os(DEV_CARGO_OVERRIDE).is_some(),
    )?;
    let mut response = RouterResponse::result(
        Operation::Preflight,
        true,
        "development_host_ready",
        "denied",
    );
    response.classification = Some("host_ready");
    response.warnings = Some(development_warnings_with_toolchain(
        cargo.group_writable_rustup_paths,
    ));
    Ok(response)
}

fn install_or_upgrade(
    operation: Operation,
    options: Options,
    paths: &RouterPaths,
    source: &mut impl Source,
    now: u64,
) -> Result<RouterResponse, RouterError> {
    prepare_private_directory(&paths.cache_root)?;
    prepare_private_directory(&paths.state_root)?;
    let lock = open_lock(&paths.state_root.join("router.lock"))?;
    lock.try_lock()
        .map_err(|_| RouterError::operation("lifecycle_busy"))?;
    let index_bytes = acquire_document(
        source,
        &paths.cache_root.join("channel.json"),
        INDEX_URL,
        MAX_INDEX_BYTES,
        options.offline,
    )?;
    let signature = acquire_document(
        source,
        &paths.cache_root.join("channel.json.sig"),
        INDEX_SIGNATURE_URL,
        MAX_SIGNATURE_BYTES,
        options.offline,
    )?;
    verify_signature(&index_bytes, &signature, CHANNEL_NAMESPACE)?;
    if !options.offline {
        atomic_private(&paths.cache_root.join("channel.json"), &index_bytes, 0o600)?;
        atomic_private(
            &paths.cache_root.join("channel.json.sig"),
            &signature,
            0o600,
        )?;
    }
    let selected = select_release(&index_bytes, now, target())?;
    enforce_rollback(&paths.state_root, &selected)?;
    let release_cache = paths
        .cache_root
        .join("releases")
        .join(&selected.manifest_sha256);
    prepare_private_directory(&release_cache)?;
    let manifest_bytes = acquire_document(
        source,
        &release_cache.join("manifest.json"),
        &selected.manifest_url,
        MAX_MANIFEST_BYTES,
        options.offline,
    )?;
    if digest(&manifest_bytes) != selected.manifest_sha256 {
        return Err(RouterError::policy("manifest_digest_mismatch"));
    }
    let manifest_signature = acquire_document(
        source,
        &release_cache.join("manifest.json.sig"),
        &selected.manifest_signature_url,
        MAX_SIGNATURE_BYTES,
        options.offline,
    )?;
    verify_signature(&manifest_bytes, &manifest_signature, BUNDLE_NAMESPACE)?;
    let manifest = validate_manifest(&manifest_bytes, &selected, now)?;
    if !options.offline {
        atomic_private(&release_cache.join("manifest.json"), &manifest_bytes, 0o600)?;
        atomic_private(
            &release_cache.join("manifest.json.sig"),
            &manifest_signature,
            0o600,
        )?;
    }
    let artifact_root = release_cache.join("artifacts");
    prepare_private_directory(&artifact_root)?;
    let verified = obtain_artifacts(&manifest, source, &artifact_root, options.offline)?;
    validate_documents(&manifest, &verified)?;
    let executable = verified
        .get("asb-tui")
        .ok_or_else(|| RouterError::policy("artifact_set_incomplete"))?;
    if options.dry_run {
        let network = if options.offline { "denied" } else { "used" };
        let mut response = RouterResponse::result(operation, true, "candidate_verified", network);
        response.release = Some(manifest.release);
        response.executable_sha256 = Some(digest(executable));
        response.verified = Some(true);
        return Ok(response);
    }
    persist_verified_promotion(
        &paths.state_root,
        &selected,
        &index_bytes,
        &signature,
        &manifest_bytes,
        &manifest_signature,
    )?;
    // A verified pending floor closes the crash window between candidate
    // activation and accepted-state commit. Failed/malformed candidates cannot
    // lower this signed, parent-verified monotonic floor.
    record_floor(&paths.state_root, "pending.json", &selected)?;
    prepare_private_directory(&paths.install_root)?;
    let request = serde_json::json!({
        "operation": operation.name(),
        "schema_version": 1,
        "install_root": paths.install_root,
        "manifest": release_cache.join("manifest.json"),
        "signature": release_cache.join("manifest.json.sig"),
        "artifacts": artifact_root,
        "target": target(),
        "asb_version": env!("CARGO_PKG_VERSION"),
        "protocol_version": 1,
        "expected_release": manifest.release,
        "expected_source_commit": manifest.source_commit,
        "expected_source_tree": manifest.source_tree,
        "expected_executable_sha256": digest(executable),
    });
    let network = if options.offline { "denied" } else { "used" };
    let delegated = run_candidate(executable, &request, true)?;
    let delegated_response = response_from_delegated(operation, delegated, network)?;
    if !delegated_response.ok {
        return Err(RouterError::policy("candidate_rejected_lifecycle"));
    }
    // Advance durable rollback state only after the candidate emitted the exact,
    // operation-bound success response. The response itself deliberately has no
    // identity fields, so bind the result to the parent-verified manifest bytes.
    record_accepted(&paths.state_root, &selected)?;
    remove_private_file(&paths.state_root.join("pending.json"))?;
    let mut response = RouterResponse::result(
        operation,
        true,
        if operation == Operation::Install {
            "extension_installed"
        } else {
            "extension_upgraded"
        },
        network,
    );
    response.release = Some(manifest.release.clone());
    response.executable_sha256 = Some(digest(executable));
    response.verified = Some(true);
    if options.launch {
        let launched = delegate_existing(Operation::Launch, paths)?;
        if !launched.ok {
            return Err(RouterError::policy("candidate_rejected_lifecycle"));
        }
        response.code = if operation == Operation::Install {
            "extension_installed_and_frontend_exited"
        } else {
            "extension_upgraded_and_frontend_exited"
        };
    }
    Ok(response)
}

fn materialize_development(
    operation: Operation,
    options: Options,
    paths: &RouterPaths,
    now: u64,
) -> Result<RouterResponse, RouterError> {
    if options.offline {
        return Err(RouterError::policy(
            "development_source_unavailable_offline",
        ));
    }
    let (asb_source_commit, asb_source_tree) = development_source_identity()?;
    if let Some(bundle) = std::env::var_os(DEV_BUNDLE_OVERRIDE) {
        return consume_development_bundle(
            operation,
            options,
            paths,
            now,
            asb_source_commit,
            asb_source_tree,
            Path::new(&bundle),
        );
    }
    let git = resolve_development_tool(DEV_GIT_OVERRIDE, DEV_GIT)?;
    let setsid = resolve_development_tool(DEV_SETSID_OVERRIDE, DEV_SETSID)?;
    let cc = resolve_development_tool_candidates(
        DEV_CC_OVERRIDE,
        &["/usr/bin/cc", "/usr/local/bin/cc"],
    )?;
    let ar = resolve_development_tool_candidates(
        DEV_AR_OVERRIDE,
        &["/usr/bin/ar", "/usr/local/bin/ar"],
    )?;
    let ld = resolve_development_tool_candidates(
        DEV_LD_OVERRIDE,
        &["/usr/bin/ld", "/usr/local/bin/ld"],
    )?;
    let rustup_home = resolve_development_rustup_home()?;
    let cargo = resolve_development_cargo_with_rustup(rustup_home.as_deref())?;
    let rustc = resolve_development_rustc_bound(
        &cargo,
        rustup_home.as_deref(),
        std::env::var_os(DEV_CARGO_OVERRIDE).is_some(),
    )?;
    prepare_private_directory(&paths.cache_root)?;
    let root = paths.cache_root.join(format!(
        "dev-build-{}-{}",
        std::process::id(),
        TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let target = paths.cache_root.join(format!(
        "dev-target-{}-{}",
        std::process::id(),
        TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    prepare_private_directory(&root)?;
    let result = (|| {
        prepare_private_directory(&target)?;
        let current_asb_head = resolve_development_main_head(
            Path::new(&git),
            Path::new(&setsid),
            ASB_REPOSITORY_URL,
            &root,
        )?;
        if current_asb_head != asb_source_commit {
            return Err(RouterError::policy("dev_source_identity_stale"));
        }
        let mut head = Command::new(&setsid);
        head.env_clear()
            .env("LANG", "C.UTF-8")
            .args(["--wait"])
            .arg(&git)
            .args(["ls-remote", DEV_REPOSITORY_URL, DEVELOPMENT_SOURCE_REF]);
        let output = run_development_command(head, &root)?;
        let commit = String::from_utf8(output)
            .map_err(|_| RouterError::policy("dev_source_identity_invalid"))?
            .split_whitespace()
            .next()
            .map(str::to_owned)
            .filter(|value| valid_hex(value, 40))
            .ok_or_else(|| RouterError::policy("dev_source_identity_invalid"))?;
        let source = root.join("source");
        let mut clone = Command::new(&setsid);
        clone
            .env_clear()
            .env("LANG", "C.UTF-8")
            .args(["--wait"])
            .arg(&git)
            .args([
                "clone",
                "--depth",
                "1",
                "--filter=blob:none",
                "--no-checkout",
                "--single-branch",
                "--branch",
                "main",
                "--",
                DEV_REPOSITORY_URL,
            ])
            .arg(&source);
        run_development_command(clone, &root)?;
        enforce_workspace_quota(&root)?;
        let mut checkout = Command::new(&setsid);
        checkout
            .env_clear()
            .env("LANG", "C.UTF-8")
            .current_dir(&source);
        checkout
            .args(["--wait"])
            .arg(&git)
            .args(["checkout", "--detach", &commit]);
        run_development_command(checkout, &root)?;
        enforce_workspace_quota(&root)?;
        let mut checked_commit_command = Command::new(&setsid);
        checked_commit_command
            .env_clear()
            .env("LANG", "C.UTF-8")
            .current_dir(&source)
            .args(["--wait"])
            .arg(&git)
            .args(["rev-parse", "HEAD"]);
        let checked_commit =
            String::from_utf8(run_development_command(checked_commit_command, &root)?)
                .map_err(|_| RouterError::policy("dev_source_identity_invalid"))?
                .trim()
                .to_owned();
        validate_development_source_commit(&commit, &checked_commit)?;
        let cargo_home = root.join("cargo-home");
        prepare_private_directory(&cargo_home)?;
        let linker_prefix = materialize_development_linker_prefix(&root, &ld)?;
        validate_development_gcc_driver(&cc, &linker_prefix, &root)?;
        let cargo_program = cargo.execution_path();
        let rustc_program = rustc.as_ref().map(DevelopmentTool::execution_path);
        let linker_program = linker_prefix.linker_path();
        let mut build = Command::new(&setsid);
        build
            .env_clear()
            .env("LANG", "C.UTF-8")
            .env("HOME", &root)
            .env("CARGO_HOME", &cargo_home)
            .env("CARGO_TARGET_DIR", &target)
            .env("CARGO_INCREMENTAL", "0")
            .env("SOURCE_DATE_EPOCH", "0")
            .env(
                "CARGO_ENCODED_RUSTFLAGS",
                development_encoded_rustflags(&root, &target, &cargo_home, &linker_prefix)?,
            )
            .env_remove("RUSTFLAGS")
            .current_dir(&source)
            .args(["--wait"])
            .arg(&cargo_program)
            .args(["build", "--locked", "--release", "--bin", "asb-tui"]);
        apply_development_toolchain_environment(
            &mut build,
            rustup_home.as_deref(),
            rustc_program.as_deref(),
            Some(&cc),
            Some(&ar),
            Some(&linker_program),
        );
        linker_prefix.validate()?;
        let linker_directory_flags = make_descriptor_inheritable(&linker_prefix.directory)?;
        let inherited_flags = make_toolchain_descriptors_inheritable(
            cargo.bound_file.as_ref(),
            rustc.as_ref().and_then(|tool| tool.bound_file.as_ref()),
        )
        .inspect_err(|_| {
            let _ = restore_descriptor_flags(&linker_prefix.directory, linker_directory_flags);
        })?;
        let build_result = run_development_command_with_limits_and_roots(
            build,
            &root,
            &[root.as_path(), target.as_path()],
            DEV_COMMAND_TIMEOUT,
            MAX_DEV_WORKSPACE_BYTES,
        );
        let restore_result = restore_toolchain_descriptor_flags(
            cargo.bound_file.as_ref(),
            rustc.as_ref().and_then(|tool| tool.bound_file.as_ref()),
            inherited_flags,
        );
        let linker_restore_result =
            restore_descriptor_flags(&linker_prefix.directory, linker_directory_flags);
        linker_restore_result?;
        restore_result?;
        build_result?;
        enforce_workspace_quota_for_roots(
            &[root.as_path(), target.as_path()],
            MAX_DEV_WORKSPACE_BYTES,
        )?;
        let executable = target.join("release/asb-tui");
        harden_private_development_tree(&target, &executable)?;
        let mut source_tree_command = Command::new(&setsid);
        source_tree_command
            .env_clear()
            .env("LANG", "C.UTF-8")
            .current_dir(&source)
            .args(["--wait"])
            .arg(&git)
            .args(["rev-parse", "HEAD^{tree}"]);
        let source_tree = String::from_utf8(run_development_command(source_tree_command, &root)?)
            .map_err(|_| RouterError::policy("dev_source_identity_invalid"))?
            .trim()
            .to_owned();
        if !valid_hex(&source_tree, 40) {
            return Err(RouterError::policy("dev_source_identity_invalid"));
        }
        let bytes = read_bounded(&executable, MAX_ARTIFACT_BYTES as usize)?;
        let executable_sha256 = digest(&bytes);
        prepare_private_directory(&paths.install_root)?;
        let metadata = serde_json::to_vec(&DevelopmentInstallation {
            schema_version: 1,
            channel: "dev".to_owned(),
            development_only: true,
            source_repository: DEV_REPOSITORY_URL.to_owned(),
            source_commit: commit.clone(),
            source_tree: source_tree.clone(),
            asb_source_commit: asb_source_commit.to_owned(),
            asb_source_tree: asb_source_tree.to_owned(),
            executable_sha256: executable_sha256.clone(),
            installed_unix: now,
        })
        .map_err(|_| RouterError::operation("dev_metadata_failed"))?;
        let channel_manifest = development_channel_manifest(
            asb_source_commit,
            asb_source_tree,
            &commit,
            &source_tree,
            &executable_sha256,
            bytes.len() as u64,
            now,
        );
        let (_, channel_manifest_sha256) = encode_channel_manifest(&channel_manifest)?;
        if options.dry_run {
            let mut response =
                RouterResponse::result(operation, true, "development_dry_run", "used");
            response.channel = "dev";
            response.development_only = true;
            response.executable_sha256 = Some(executable_sha256);
            response.source_commit = Some(commit);
            response.source_tree = Some(source_tree);
            response.asb_source_commit = Some(asb_source_commit.to_owned());
            response.asb_source_tree = Some(asb_source_tree.to_owned());
            response.channel_manifest_sha256 = Some(channel_manifest_sha256);
            response.warnings = Some(development_warnings_with_toolchain(
                cargo.group_writable_rustup_paths,
            ));
            return Ok(response);
        }
        publish_development_version(&paths.install_root, &executable_sha256, &bytes, &metadata)?;
        let channel_manifest_sha256 = publish_development_channel_manifest(
            &paths.install_root,
            &executable_sha256,
            &channel_manifest,
        )?;
        let mut response = RouterResponse::result(operation, true, "development_built", "used");
        response.channel = "dev";
        response.development_only = true;
        response.executable_sha256 = Some(executable_sha256);
        response.source_commit = Some(commit);
        response.source_tree = Some(source_tree);
        response.asb_source_commit = Some(asb_source_commit.to_owned());
        response.asb_source_tree = Some(asb_source_tree.to_owned());
        response.channel_manifest_sha256 = Some(channel_manifest_sha256);
        response.warnings = Some(development_warnings_with_toolchain(
            cargo.group_writable_rustup_paths,
        ));
        if options.launch {
            let launched = execute_development_existing(Operation::Launch, paths)?;
            if !launched.ok {
                return Err(RouterError::operation("development_launch_failed"));
            }
            response.code = "development_built_and_launched";
        }
        Ok(response)
    })();
    let cleanup = match fs::remove_dir_all(&root) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    };
    let target_cleanup = match fs::remove_dir_all(&target) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    };
    match (result, cleanup, target_cleanup) {
        (Ok(response), Ok(()), Ok(())) => Ok(response),
        (Err(error), Ok(()), Ok(())) => Err(error),
        _ => Err(RouterError::operation("dev_cleanup_failed")),
    }
}

fn consume_development_bundle(
    operation: Operation,
    options: Options,
    paths: &RouterPaths,
    now: u64,
    asb_source_commit: &str,
    asb_source_tree: &str,
    bundle: &Path,
) -> Result<RouterResponse, RouterError> {
    if options.offline {
        return Err(RouterError::policy(
            "development_source_unavailable_offline",
        ));
    }
    let manifest_bytes = read_bounded(&bundle.join("manifest.json"), MAX_MANIFEST_BYTES)?;
    let manifest: DevelopmentBundleManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|_| RouterError::policy("development_bundle_invalid"))?;
    if manifest.schema_version != 1
        || manifest.channel != "dev"
        || !manifest.development_only
        || manifest.source_repository != DEV_REPOSITORY_URL
        || manifest.source_ref != "refs/heads/main"
        || manifest.asb_source_commit != asb_source_commit
        || manifest.asb_source_tree != asb_source_tree
        || manifest.target != format!("{}-unknown-linux-gnu", std::env::consts::ARCH)
        || manifest.warnings.as_slice()
            != [
                "development_missing_authentication_allowed",
                "development_missing_signatures_allowed",
                "development_missing_key_management_allowed",
            ]
        || !valid_hex(&manifest.source_commit, 40)
        || !valid_hex(&manifest.source_tree, 40)
        || !valid_hex(&manifest.asb_source_commit, 40)
        || !valid_hex(&manifest.asb_source_tree, 40)
        || !valid_hex(&manifest.executable_sha256, 64)
        || manifest.executable_size == 0
        || manifest.built_unix == 0
    {
        return Err(RouterError::policy("development_bundle_invalid"));
    }
    let executable = bundle.join("asb-tui");
    let bytes = read_bounded(&executable, MAX_ARTIFACT_BYTES as usize)?;
    if bytes.len() as u64 != manifest.executable_size
        || digest(&bytes) != manifest.executable_sha256
    {
        return Err(RouterError::policy("development_bundle_invalid"));
    }
    prepare_private_directory(&paths.install_root)?;
    let metadata = serde_json::to_vec(&DevelopmentInstallation {
        schema_version: 1,
        channel: "dev".to_owned(),
        development_only: true,
        source_repository: manifest.source_repository,
        source_commit: manifest.source_commit.clone(),
        source_tree: manifest.source_tree.clone(),
        asb_source_commit: asb_source_commit.to_owned(),
        asb_source_tree: asb_source_tree.to_owned(),
        executable_sha256: manifest.executable_sha256.clone(),
        installed_unix: now,
    })
    .map_err(|_| RouterError::operation("dev_metadata_failed"))?;
    let channel_manifest = development_channel_manifest(
        asb_source_commit,
        asb_source_tree,
        &manifest.source_commit,
        &manifest.source_tree,
        &manifest.executable_sha256,
        manifest.executable_size,
        manifest.built_unix,
    );
    let (_, channel_manifest_sha256) = encode_channel_manifest(&channel_manifest)?;
    if options.dry_run {
        let mut response =
            RouterResponse::result(operation, true, "development_bundle_verified", "used");
        response.channel = "dev";
        response.development_only = true;
        response.executable_sha256 = Some(manifest.executable_sha256);
        response.source_commit = Some(manifest.source_commit);
        response.source_tree = Some(manifest.source_tree);
        response.asb_source_commit = Some(asb_source_commit.to_owned());
        response.asb_source_tree = Some(asb_source_tree.to_owned());
        response.channel_manifest_sha256 = Some(channel_manifest_sha256);
        response.warnings = Some(development_warnings());
        return Ok(response);
    }
    publish_development_version(
        &paths.install_root,
        &manifest.executable_sha256,
        &bytes,
        &metadata,
    )?;
    let channel_manifest_sha256 = publish_development_channel_manifest(
        &paths.install_root,
        &manifest.executable_sha256,
        &channel_manifest,
    )?;
    let mut response =
        RouterResponse::result(operation, true, "development_bundle_consumed", "used");
    response.channel = "dev";
    response.development_only = true;
    response.executable_sha256 = Some(manifest.executable_sha256);
    response.source_commit = Some(manifest.source_commit);
    response.source_tree = Some(manifest.source_tree);
    response.asb_source_commit = Some(asb_source_commit.to_owned());
    response.asb_source_tree = Some(asb_source_tree.to_owned());
    response.channel_manifest_sha256 = Some(channel_manifest_sha256);
    response.warnings = Some(development_warnings());
    if options.launch {
        let launched = execute_development_existing(Operation::Launch, paths)?;
        if !launched.ok {
            return Err(RouterError::operation("development_launch_failed"));
        }
        response.code = "development_bundle_consumed_and_launched";
    }
    Ok(response)
}

fn validate_development_source_commit(expected: &str, observed: &str) -> Result<(), RouterError> {
    if valid_hex(expected, 40) && observed == expected {
        Ok(())
    } else {
        Err(RouterError::policy("dev_source_identity_mismatch"))
    }
}

fn publish_development_version(
    install_root: &Path,
    executable_sha256: &str,
    bytes: &[u8],
    metadata: &[u8],
) -> Result<(), RouterError> {
    let version = install_root.join("dev-versions").join(executable_sha256);
    let version_existed = version.exists();
    prepare_private_directory(&version)?;
    let publication = (|| {
        atomic_private(&version.join("asb-tui"), bytes, 0o700)?;
        // Keep the exact provenance envelope beside the content-addressed
        // executable.  The active pointer is only a selector; status and
        // launch re-read this immutable per-version manifest before use.
        atomic_private(&version.join("manifest.json"), metadata, 0o600)?;
        atomic_private(&install_root.join("active-dev.json"), metadata, 0o600)
    })();
    if let Err(error) = publication {
        if !version_existed {
            let _ = fs::remove_dir_all(&version);
        }
        return Err(error);
    }
    Ok(())
}

fn development_response(
    operation: Operation,
    code: &'static str,
    active: &DevelopmentInstallation,
) -> RouterResponse {
    let mut response = RouterResponse::result(operation, true, code, "denied");
    response.channel = "dev";
    response.development_only = true;
    response.executable_sha256 = Some(active.executable_sha256.clone());
    response.source_commit = Some(active.source_commit.clone());
    response.source_tree = Some(active.source_tree.clone());
    response.asb_source_commit = Some(active.asb_source_commit.clone());
    response.asb_source_tree = Some(active.asb_source_tree.clone());
    response.warnings = Some(development_warnings());
    response.verified = Some(false);
    response
}

fn development_warnings() -> Vec<&'static str> {
    vec![
        "development_missing_authentication_allowed",
        "development_missing_signatures_allowed",
        "development_missing_key_management_allowed",
    ]
}

fn development_warnings_with_toolchain(group_writable_rustup_paths: bool) -> Vec<&'static str> {
    let mut warnings = development_warnings();
    if group_writable_rustup_paths {
        warnings.push(GROUP_WRITABLE_RUSTUP_PATH_WARNING);
    }
    warnings
}

fn read_development_channel_manifest(
    paths: &RouterPaths,
    active: &DevelopmentInstallation,
) -> Result<Option<(DevelopmentChannelManifest, String)>, RouterError> {
    let root_manifest = paths.install_root.join("active-channel.json");
    if !root_manifest.exists() {
        // Older development installations predate AR-1690.  They remain
        // readable for status/removal, while every new materialization emits
        // the manifest below.
        return Ok(None);
    }
    let bytes = read_bounded(&root_manifest, MAX_MANIFEST_BYTES)
        .map_err(|_| RouterError::policy("development_installation_invalid"))?;
    let manifest: DevelopmentChannelManifest = serde_json::from_slice(&bytes)
        .map_err(|_| RouterError::policy("development_installation_invalid"))?;
    if manifest.schema_version != 1
        || manifest.channel != "dev"
        || !manifest.development_only
        || manifest.asb_repository != ASB_REPOSITORY_URL
        || manifest.asb_ref != DEVELOPMENT_SOURCE_REF
        || manifest.asb_source_commit != active.asb_source_commit
        || manifest.asb_source_tree != active.asb_source_tree
        || manifest.tui_repository != active.source_repository
        || manifest.tui_ref != DEVELOPMENT_SOURCE_REF
        || manifest.tui_source_commit != active.source_commit
        || manifest.tui_source_tree != active.source_tree
        || manifest.executable_sha256 != active.executable_sha256
        || manifest.executable_size == 0
        || manifest.built_unix == 0
        || manifest.warnings
            != development_warnings()
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>()
    {
        return Err(RouterError::policy("development_installation_invalid"));
    }
    let version_manifest = paths
        .install_root
        .join("dev-versions")
        .join(&active.executable_sha256)
        .join("channel-manifest.json");
    let version_bytes = read_bounded(&version_manifest, MAX_MANIFEST_BYTES)
        .map_err(|_| RouterError::policy("development_installation_invalid"))?;
    if version_bytes != bytes {
        return Err(RouterError::policy("development_installation_invalid"));
    }
    Ok(Some((manifest, digest(&bytes))))
}

fn development_active(
    paths: &RouterPaths,
) -> Result<Option<(DevelopmentInstallation, PathBuf)>, RouterError> {
    let marker = paths.install_root.join("active-dev.json");
    if !marker.exists() {
        return Ok(None);
    }
    let (asb_source_commit, asb_source_tree) = development_source_identity()?;
    validate_private_directory(&paths.install_root)?;
    let bytes = read_bounded(&marker, 64 * 1024)
        .map_err(|_| RouterError::policy("development_installation_invalid"))?;
    let active: DevelopmentInstallation = serde_json::from_slice(&bytes)
        .map_err(|_| RouterError::policy("development_installation_invalid"))?;
    if active.schema_version != 1
        || !active.development_only
        || active.channel != "dev"
        || active.source_repository != DEV_REPOSITORY_URL
        || !valid_hex(&active.source_commit, 40)
        || !valid_hex(&active.source_tree, 40)
        || active.asb_source_commit != asb_source_commit
        || active.asb_source_tree != asb_source_tree
        || !valid_hex(&active.executable_sha256, 64)
    {
        return Err(RouterError::policy("development_installation_invalid"));
    }
    let executable = paths
        .install_root
        .join("dev-versions")
        .join(&active.executable_sha256)
        .join("asb-tui");
    read_bounded_digest(&executable, MAX_ARTIFACT_BYTES, &active.executable_sha256)?;
    let manifest = paths
        .install_root
        .join("dev-versions")
        .join(&active.executable_sha256)
        .join("manifest.json");
    let manifest_bytes = read_bounded(&manifest, 64 * 1024)
        .map_err(|_| RouterError::policy("development_installation_invalid"))?;
    let recorded: DevelopmentInstallation = serde_json::from_slice(&manifest_bytes)
        .map_err(|_| RouterError::policy("development_installation_invalid"))?;
    if recorded != active {
        return Err(RouterError::policy("development_installation_invalid"));
    }
    if let Some((manifest, _)) = read_development_channel_manifest(paths, &active)?
        && manifest.executable_size
            != fs::metadata(&executable)
                .map_err(|_| RouterError::policy("development_installation_invalid"))?
                .len()
    {
        return Err(RouterError::policy("development_installation_invalid"));
    }
    Ok(Some((active, executable)))
}

fn execute_development_existing(
    operation: Operation,
    paths: &RouterPaths,
) -> Result<RouterResponse, RouterError> {
    let Some((active, executable)) = development_active(paths)? else {
        return Err(RouterError::policy("extension_not_installed"));
    };
    let mut response = match operation {
        Operation::Status => Ok(development_response(
            operation,
            "development_installed",
            &active,
        )),
        Operation::Remove => {
            remove_development_installation(paths, &active, false)?;
            Ok(development_response(
                operation,
                "development_removed",
                &active,
            ))
        }
        Operation::Launch | Operation::LiveProvider | Operation::DynamicCatalog => {
            let descriptor = development_broker_descriptor(&active)?;
            let mut command = Command::new(&executable);
            command
                .env_clear()
                .env(DEV_BROKER_DESCRIPTOR_ENV, descriptor)
                .env(DEV_BROKER_ASB_COMMIT_ENV, ASB_SOURCE_COMMIT)
                .env(DEV_BROKER_ASB_TREE_ENV, ASB_SOURCE_TREE)
                .env(DEV_BROKER_TUI_COMMIT_ENV, &active.source_commit)
                .env(DEV_BROKER_TUI_TREE_ENV, &active.source_tree)
                .args(["run", "--broker", "--development"]);
            if let Some(route) = frontend_route_flag(operation) {
                command.arg(route);
            }
            add_candidate_environment(&mut command)?;
            add_development_terminal_environment(&mut command)?;
            let tools = DevelopmentLaunchTools::resolve()?;
            tools.apply(&mut command);
            let inherited = tools.make_descriptors_inheritable()?;
            let launch_result = launch_development_broker(command, &paths.state_root);
            let restore_result = tools.restore_descriptor_flags(inherited);
            restore_result?;
            let status = launch_result?;
            if !status.success() {
                return Err(RouterError::operation("development_launch_failed"));
            }
            Ok(development_response(
                operation,
                "development_launched",
                &active,
            ))
        }
        _ => Err(RouterError::policy("development_operation_invalid")),
    }?;
    if let Some((_, manifest_sha256)) = read_development_channel_manifest(paths, &active)? {
        response.channel_manifest_sha256 = Some(manifest_sha256);
    }
    Ok(response)
}

fn launch_development_broker(
    command: Command,
    state_root: &Path,
) -> Result<std::process::ExitStatus, RouterError> {
    let backend = open_development_backend(state_root.to_path_buf())
        .map_err(|_| RouterError::operation("development_control_unavailable"))?;
    launch_development_broker_with_backend(command, state_root, backend)
}

fn launch_development_broker_with_backend<B: ControlBackend + Send + Sync + 'static>(
    mut command: Command,
    state_root: &Path,
    backend: B,
) -> Result<std::process::ExitStatus, RouterError> {
    let mut foreground_terminal = DevelopmentForegroundTerminal::capture()?;
    let broker_root = development_broker_root(state_root);
    let control_dir = broker_root.join("control");
    let provisioning_dir = broker_root.join("provisioning");
    prepare_private_directory(&control_dir)?;
    prepare_private_directory(&provisioning_dir)?;
    let control_socket = control_dir.join("control.sock");
    let provisioning_socket = provisioning_dir.join("provisioning.sock");
    let server = ProvisionedControlServer::bind(
        &control_socket,
        &provisioning_socket,
        ControlLimits::default(),
        backend,
    )
    .map_err(|_| RouterError::operation("development_control_unavailable"))?;
    let mut broker_state = BrokerState::fresh()
        .map_err(|_| RouterError::operation("development_channel_unavailable"))?;
    let pending = broker_state
        .begin_initial(DEV_CONTROL_HANDSHAKE_TIMEOUT)
        .map_err(|_| RouterError::operation("development_channel_unavailable"))?;
    let (router, frontend) = match BrokerConnection::pair() {
        Ok(pair) => pair,
        Err(_) => {
            let _ = fs::remove_dir_all(&broker_root);
            return Err(RouterError::operation("development_channel_unavailable"));
        }
    };
    command.stdin(Stdio::from(frontend));
    // The broker fd replaces stdin, so the development child can use a
    // private process group without affecting the stable terminal launcher.
    command.process_group(0);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(_) => {
            let _ = fs::remove_dir_all(&broker_root);
            return Err(RouterError::operation("development_launch_failed"));
        }
    };
    if let Some(terminal) = foreground_terminal.as_mut()
        && terminal.assign(Pid::from_child(&child)).is_err()
    {
        terminate_development_child(&mut child);
        let _ = fs::remove_dir_all(&broker_root);
        return Err(RouterError::operation("development_terminal_unavailable"));
    }
    let result = (|| {
        // A parent-first handoff is only valid while the frontend is alive.  Do
        // not reserve or publish a generation to a child which already exited:
        // this also keeps the ordinary cleanup path bounded for short-lived
        // commands such as `/bin/true`.
        if child.try_wait().ok().flatten().is_some() {
            terminate_development_child(&mut child);
            let _ = fs::remove_dir_all(&broker_root);
            return Err(RouterError::operation("development_launch_failed"));
        }
        // The deadline only bounds admission of the two handshake endpoints. Once
        // both endpoints are admitted, `serve_connections_until` joins their
        // workers and remains alive for the interactive child lifetime.
        let mut server_worker = Some(thread::spawn(move || {
            server.serve_connections_until(1, 1, Some(DEV_CONTROL_HANDSHAKE_TIMEOUT))
        }));
        let producer = match AuthenticatedGenerationProducer::new(
            control_socket,
            provisioning_socket,
            ControlLimits::default(),
        ) {
            Ok(producer) => producer,
            Err(_) => {
                terminate_development_child(&mut child);
                let _ = server_worker.take().expect("server worker").join();
                let _ = fs::remove_dir_all(&broker_root);
                return Err(RouterError::operation("development_control_unavailable"));
            }
        };
        let authenticated = match producer.acquire(&pending) {
            Ok(authenticated) => authenticated,
            Err(_) => {
                terminate_development_child(&mut child);
                let _ = server_worker.take().expect("server worker").join();
                let _ = fs::remove_dir_all(&broker_root);
                return Err(RouterError::operation("development_channel_rejected"));
            }
        };
        // The child can exit while the parent is admitting the two control
        // endpoints. Never commit a generation for an already-dead frontend.
        if child.try_wait().ok().flatten().is_some() {
            terminate_development_child(&mut child);
            drop(authenticated);
            drop(router);
            let _ = server_worker.take().expect("server worker").join();
            let _ = fs::remove_dir_all(&broker_root);
            return Err(RouterError::operation("development_launch_failed"));
        }
        if broker_state
            .commit_success(pending, &router, authenticated)
            .is_err()
        {
            terminate_development_child(&mut child);
            let _ = server_worker.take().expect("server worker").join();
            let _ = fs::remove_dir_all(&broker_root);
            return Err(RouterError::operation("development_channel_rejected"));
        }
        let child_deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if Instant::now() >= child_deadline {
                terminate_development_child(&mut child);
                drop(router);
                if let Some(worker) = server_worker.take() {
                    let _ = worker.join();
                }
                let _ = fs::remove_dir_all(&broker_root);
                return Err(RouterError::operation("development_launch_timeout"));
            }
            let child_status = match child.try_wait() {
                Ok(status) => status,
                Err(_) => {
                    terminate_development_child(&mut child);
                    if let Some(worker) = server_worker.take() {
                        let _ = worker.join();
                    }
                    let _ = fs::remove_dir_all(&broker_root);
                    return Err(RouterError::operation("development_launch_failed"));
                }
            };
            if let Some(status) = child_status {
                let server_ok = server_worker
                    .take()
                    .expect("server worker")
                    .join()
                    .is_ok_and(|result| result.is_ok());
                let _ = fs::remove_dir_all(&broker_root);
                return server_ok
                    .then_some(status)
                    .ok_or_else(|| RouterError::operation("development_control_failed"));
            }
            if server_worker
                .as_ref()
                .is_some_and(std::thread::JoinHandle::is_finished)
            {
                let server_ok = server_worker
                    .take()
                    .expect("server worker")
                    .join()
                    .is_ok_and(|result| result.is_ok());
                if !server_ok {
                    terminate_development_child(&mut child);
                    let _ = fs::remove_dir_all(&broker_root);
                    return Err(RouterError::operation("development_control_failed"));
                }
            }
            thread::sleep(Duration::from_millis(10));
        }
    })();
    if foreground_terminal
        .as_mut()
        .is_some_and(|terminal| terminal.restore().is_err())
    {
        return Err(RouterError::operation("development_terminal_unavailable"));
    }
    result
}

/// Scoped foreground ownership for the development frontend's private process
/// group. The ASB launcher must already be the foreground group before it can
/// delegate the terminal, and the exact prior group is restored on every exit.
struct DevelopmentForegroundTerminal {
    terminal: File,
    original_group: Pid,
    assigned_group: Option<Pid>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DevelopmentTerminalError {
    ForegroundChanged,
    SignalMask,
    Terminal,
}

struct SigttouBlock {
    previous: SigSet,
    restored: bool,
}

impl SigttouBlock {
    fn acquire() -> Result<Self, DevelopmentTerminalError> {
        let mut blocked = SigSet::empty();
        blocked.add(NixSignal::SIGTTOU);
        let mut previous = SigSet::empty();
        pthread_sigmask(SigmaskHow::SIG_BLOCK, Some(&blocked), Some(&mut previous))
            .map_err(|_| DevelopmentTerminalError::SignalMask)?;
        Ok(Self {
            previous,
            restored: false,
        })
    }

    fn restore(mut self) -> Result<(), DevelopmentTerminalError> {
        pthread_sigmask(SigmaskHow::SIG_SETMASK, Some(&self.previous), None)
            .map_err(|_| DevelopmentTerminalError::SignalMask)?;
        self.restored = true;
        Ok(())
    }
}

impl Drop for SigttouBlock {
    fn drop(&mut self) {
        if !self.restored {
            let _ = pthread_sigmask(SigmaskHow::SIG_SETMASK, Some(&self.previous), None);
        }
    }
}

#[cfg(test)]
type DevelopmentForegroundHook = Box<dyn FnOnce(&File, Pid)>;

#[cfg(test)]
thread_local! {
    static DEVELOPMENT_FOREGROUND_BEFORE_ASSIGN: std::cell::RefCell<
        Option<DevelopmentForegroundHook>,
    > = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn run_development_foreground_before_assign(terminal: &File, child_group: Pid) {
    DEVELOPMENT_FOREGROUND_BEFORE_ASSIGN.with(|hook| {
        if let Some(hook) = hook.borrow_mut().take() {
            hook(terminal, child_group);
        }
    });
}

impl DevelopmentForegroundTerminal {
    fn capture() -> Result<Option<Self>, RouterError> {
        if !io::stdin().is_terminal() {
            return Ok(None);
        }
        let terminal = File::open("/proc/self/fd/0")
            .map_err(|_| RouterError::operation("development_terminal_unavailable"))?;
        if !terminal.is_terminal() {
            return Err(RouterError::operation("development_terminal_unavailable"));
        }
        let original_group = tcgetpgrp(&terminal)
            .map_err(|_| RouterError::operation("development_terminal_unavailable"))?;
        if original_group != getpgrp() {
            return Err(RouterError::operation("development_terminal_unavailable"));
        }
        Ok(Some(Self {
            terminal,
            original_group,
            assigned_group: None,
        }))
    }

    fn assign(&mut self, child_group: Pid) -> Result<(), DevelopmentTerminalError> {
        let blocked = SigttouBlock::acquire()?;
        #[cfg(test)]
        run_development_foreground_before_assign(&self.terminal, child_group);
        let assigned = (|| {
            if tcgetpgrp(&self.terminal).map_err(|_| DevelopmentTerminalError::Terminal)?
                != self.original_group
            {
                return Err(DevelopmentTerminalError::ForegroundChanged);
            }
            // Keep this handoff adjacent to the ownership revalidation. The
            // scoped SIGTTOU block makes a changed owner a typed failure
            // instead of stopping the launcher in a background process group.
            tcsetpgrp(&self.terminal, child_group)
                .map_err(|_| DevelopmentTerminalError::Terminal)?;
            self.assigned_group = Some(child_group);
            // The child may reach terminal setup in the short interval between
            // spawn and tcsetpgrp and be stopped by SIGTTIN/SIGTTOU. Continuing
            // the now-foreground group is harmless when it never stopped.
            kill_process_group(child_group, Signal::CONT)
                .map_err(|_| DevelopmentTerminalError::Terminal)
        })();
        let mask_restored = blocked.restore();
        assigned.and(mask_restored)
    }

    fn restore(&mut self) -> Result<(), DevelopmentTerminalError> {
        let Some(assigned_group) = self.assigned_group else {
            return Ok(());
        };
        let blocked = SigttouBlock::acquire()?;
        let current_group = match tcgetpgrp(&self.terminal) {
            Ok(group) => group,
            Err(_) => {
                self.assigned_group = None;
                blocked.restore()?;
                return Err(DevelopmentTerminalError::Terminal);
            }
        };
        if current_group == self.original_group {
            self.assigned_group = None;
            blocked.restore()?;
            return Ok(());
        }
        if current_group != assigned_group {
            // Another foreground owner won after assignment. Relinquish the
            // lease without overwriting that legitimate owner.
            self.assigned_group = None;
            blocked.restore()?;
            return Err(DevelopmentTerminalError::ForegroundChanged);
        }
        let restored = tcsetpgrp(&self.terminal, self.original_group)
            .map_err(|_| DevelopmentTerminalError::Terminal);
        if restored.is_ok() {
            self.assigned_group = None;
        }
        let mask_restored = blocked.restore();
        restored.and(mask_restored)
    }
}

impl Drop for DevelopmentForegroundTerminal {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

/// The broker consumes fd 0, so the development TUI must receive a separately
/// validated path to the parent's terminal before the handoff.  This is
/// development-only metadata; the child revalidates ownership and character
/// device shape before redirecting stdin.
fn add_development_terminal_environment(command: &mut Command) -> Result<(), RouterError> {
    if !io::stdin().is_terminal() {
        return Ok(());
    }
    let path = fs::read_link("/proc/self/fd/0")
        .map_err(|_| RouterError::operation("development_terminal_unavailable"))?;
    let text = path
        .to_str()
        .ok_or_else(|| RouterError::operation("development_terminal_unavailable"))?;
    let Some(number) = text.strip_prefix("/dev/pts/") else {
        return Err(RouterError::operation("development_terminal_unavailable"));
    };
    if number.is_empty()
        || number.len() > 16
        || !number.bytes().all(|byte| byte.is_ascii_digit())
        || text.len() > 64
    {
        return Err(RouterError::operation("development_terminal_unavailable"));
    }
    let metadata = fs::symlink_metadata(&path)
        .map_err(|_| RouterError::operation("development_terminal_unavailable"))?;
    if !metadata.file_type().is_char_device()
        || metadata.uid() != rustix::process::getuid().as_raw()
    {
        return Err(RouterError::operation("development_terminal_unavailable"));
    }
    command.env("ASB_TUI_DEVELOPMENT_TERMINAL_PATH", path);
    Ok(())
}

/// Keep both development Unix endpoints below Linux's sockaddr_un path bound
/// even when XDG state roots are deeply nested.  The fallback remains a
/// private, process-unique temporary directory and is removed on every normal
/// launch exit path.
fn development_broker_root(state_root: &Path) -> PathBuf {
    let suffix = format!(
        "{}-{}",
        std::process::id(),
        TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let state_root_candidate = state_root.join(format!(".dev-broker-{suffix}"));
    let control_socket = state_root_candidate.join("control/control.sock");
    let provisioning_socket = state_root_candidate.join("provisioning/provisioning.sock");
    if control_socket.as_os_str().len() < UNIX_SOCKET_PATH_LIMIT
        && provisioning_socket.as_os_str().len() < UNIX_SOCKET_PATH_LIMIT
    {
        state_root_candidate
    } else {
        std::env::temp_dir().join(format!("asb-broker-{suffix}"))
    }
}

fn terminate_development_child(child: &mut Child) {
    let pid = Pid::from_child(child);
    let _ = kill_process_group(pid, Signal::KILL);
    let _ = child.kill();
    let _ = child.wait();
}

fn development_broker_descriptor(active: &DevelopmentInstallation) -> Result<String, RouterError> {
    if !valid_hex(ASB_SOURCE_COMMIT, 40)
        || !valid_hex(ASB_SOURCE_TREE, 40)
        || !valid_hex(&active.source_commit, 40)
        || !valid_hex(&active.source_tree, 40)
    {
        return Err(RouterError::policy("dev_source_identity_unknown"));
    }
    let descriptor = serde_json::json!({
        "schema_version": 1,
        "profile": "development",
        "development_only": true,
        "operation": "launch",
        "protocol_minor": DEV_BROKER_PROTOCOL_MINOR,
        "asb_source_commit": ASB_SOURCE_COMMIT,
        "asb_source_tree": ASB_SOURCE_TREE,
        "tui_source_commit": active.source_commit,
        "tui_source_tree": active.source_tree,
    });
    let encoded = serde_json::to_string(&descriptor)
        .map_err(|_| RouterError::operation("development_descriptor_failed"))?;
    if encoded.len() > 16 * 1024 {
        return Err(RouterError::policy("development_descriptor_oversized"));
    }
    Ok(encoded)
}

fn remove_development_installation(
    paths: &RouterPaths,
    active: &DevelopmentInstallation,
    fail_final_delete: bool,
) -> Result<(), RouterError> {
    let marker = paths.install_root.join("active-dev.json");
    let version = paths
        .install_root
        .join("dev-versions")
        .join(&active.executable_sha256);
    let trash = paths.install_root.join(format!(
        ".dev-remove-{}-{}",
        std::process::id(),
        TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    prepare_private_directory(&trash)?;
    let trash_marker = trash.join("active-dev.json");
    let channel_manifest = paths.install_root.join("active-channel.json");
    let trash_channel_manifest = trash.join("active-channel.json");
    let trash_version = trash.join("version");
    if fs::rename(&marker, &trash_marker).is_err() {
        let _ = fs::remove_dir_all(&trash);
        return Err(RouterError::operation("development_remove_failed"));
    }
    if fs::rename(&version, &trash_version).is_err() {
        let _ = fs::rename(&trash_marker, &marker);
        let _ = fs::remove_dir_all(&trash);
        return Err(RouterError::operation("development_remove_failed"));
    }
    let moved_channel_manifest = if channel_manifest.exists() {
        if fs::rename(&channel_manifest, &trash_channel_manifest).is_err() {
            let _ = fs::rename(&trash_version, &version);
            let _ = fs::rename(&trash_marker, &marker);
            let _ = fs::remove_dir_all(&trash);
            return Err(RouterError::operation("development_remove_failed"));
        }
        true
    } else {
        false
    };
    let final_delete = if fail_final_delete {
        Err(())
    } else {
        fs::remove_dir_all(&trash).map_err(|_| ())
    };
    if final_delete.is_ok() {
        return Ok(());
    }
    let _ = fs::rename(&trash_version, &version);
    let _ = fs::rename(&trash_marker, &marker);
    if moved_channel_manifest {
        let _ = fs::rename(&trash_channel_manifest, &channel_manifest);
    }
    let _ = fs::remove_dir_all(&trash);
    Err(RouterError::operation("development_remove_failed"))
}

fn run_development_command(command: Command, root: &Path) -> Result<Vec<u8>, RouterError> {
    run_development_command_with_limits(command, root, DEV_COMMAND_TIMEOUT, MAX_DEV_WORKSPACE_BYTES)
}

fn run_development_command_with_limits(
    command: Command,
    root: &Path,
    timeout: Duration,
    quota: u64,
) -> Result<Vec<u8>, RouterError> {
    run_development_command_with_limits_and_roots(command, root, &[root], timeout, quota)
}

fn run_development_command_with_limits_and_roots(
    mut command: Command,
    cleanup_root: &Path,
    quota_roots: &[&Path],
    timeout: Duration,
    quota: u64,
) -> Result<Vec<u8>, RouterError> {
    command.stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child = command
        .spawn()
        .map_err(|_| RouterError::operation("dev_command_unavailable"))?;
    let pid = Pid::from_child(&child);
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| RouterError::operation("dev_command_unavailable"))?;
    let stdout_flags = fcntl_getfl(&stdout).map_err(|_| {
        terminate_bounded_command(&mut child, pid, &mut stdout, cleanup_root);
        RouterError::operation("dev_command_failed")
    })?;
    fcntl_setfl(&stdout, stdout_flags | OFlags::NONBLOCK).map_err(|_| {
        terminate_bounded_command(&mut child, pid, &mut stdout, cleanup_root);
        RouterError::operation("dev_command_failed")
    })?;
    let mut bytes = Vec::new();
    let mut oversized = false;
    let deadline = Instant::now() + timeout;
    loop {
        if drain_development_command_output(&mut stdout, &mut bytes, &mut oversized).is_err() {
            terminate_bounded_command(&mut child, pid, &mut stdout, cleanup_root);
            return Err(RouterError::operation("dev_command_failed"));
        }
        let exited = match waitid(
            WaitId::Pid(pid),
            WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
        ) {
            Ok(status) => status.is_some(),
            Err(_) => {
                terminate_bounded_command(&mut child, pid, &mut stdout, cleanup_root);
                return Err(RouterError::operation("dev_command_failed"));
            }
        };
        if exited {
            // Keep the leader unreaped while killing its owned process group.
            // This fences PID reuse and closes stdout/linker descriptors held
            // by descendants before bounded output drain and caller cleanup.
            let status = finish_bounded_command(&mut child, pid);
            let drain = drain_development_command_output_until_closed(
                &mut stdout,
                &mut bytes,
                &mut oversized,
            );
            let status = status?;
            drain?;
            if bounded_directory_size_for_roots(quota_roots, quota)? > quota {
                let _ = fs::remove_dir_all(cleanup_root);
                return Err(RouterError::policy("dev_workspace_quota_exceeded"));
            }
            if oversized || !status.success() {
                return Err(RouterError::operation("dev_command_failed"));
            }
            return Ok(bytes);
        }
        let quota_size = match bounded_directory_size_for_roots(quota_roots, quota) {
            Ok(size) => size,
            Err(error) => {
                terminate_bounded_command(&mut child, pid, &mut stdout, cleanup_root);
                return Err(error);
            }
        };
        if quota_size > quota {
            terminate_bounded_command(&mut child, pid, &mut stdout, cleanup_root);
            return Err(RouterError::policy("dev_workspace_quota_exceeded"));
        }
        if Instant::now() >= deadline {
            terminate_bounded_command(&mut child, pid, &mut stdout, cleanup_root);
            return Err(RouterError::operation("dev_command_timeout"));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn drain_development_command_output(
    stdout: &mut impl Read,
    bytes: &mut Vec<u8>,
    oversized: &mut bool,
) -> Result<bool, ()> {
    let mut buffer = [0_u8; 8192];
    // stdout is nonblocking, but a hostile descendant can keep it continuously
    // readable. Bound each drain pass so the caller always regains control to
    // enforce its process-group, quota, and wall-clock deadlines.
    for _ in 0..MAX_DEV_COMMAND_DRAIN_READS {
        match stdout.read(&mut buffer) {
            Ok(0) => return Ok(true),
            Ok(count) => {
                if bytes.len() < MAX_DEV_COMMAND_OUTPUT {
                    let retained = count.min(MAX_DEV_COMMAND_OUTPUT - bytes.len());
                    bytes.extend_from_slice(&buffer[..retained]);
                    *oversized |= retained != count;
                } else {
                    *oversized = true;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(false),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => return Err(()),
        }
    }
    Ok(false)
}

fn drain_development_command_output_until_closed(
    stdout: &mut impl Read,
    bytes: &mut Vec<u8>,
    oversized: &mut bool,
) -> Result<(), RouterError> {
    let deadline = Instant::now() + Duration::from_millis(250);
    loop {
        if drain_development_command_output(stdout, bytes, oversized)
            .map_err(|_| RouterError::operation("dev_command_failed"))?
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(RouterError::operation("dev_command_failed"));
        }
        thread::sleep(Duration::from_millis(5));
    }
}

fn finish_bounded_command(
    child: &mut Child,
    pid: Pid,
) -> Result<std::process::ExitStatus, RouterError> {
    let group = match kill_process_group(pid, Signal::KILL) {
        Ok(()) | Err(rustix::io::Errno::SRCH) => Ok(()),
        Err(_) => Err(RouterError::operation("dev_command_failed")),
    };
    let status = child
        .wait()
        .map_err(|_| RouterError::operation("dev_command_failed"));
    group?;
    status
}

fn terminate_bounded_command(
    child: &mut Child,
    pid: Pid,
    stdout: &mut impl Read,
    cleanup_root: &Path,
) {
    let _ = kill_process_group(pid, Signal::KILL);
    let _ = child.kill();
    let _ = child.wait();
    let mut discarded = Vec::new();
    let mut oversized = false;
    let _ = drain_development_command_output_until_closed(stdout, &mut discarded, &mut oversized);
    let _ = fs::remove_dir_all(cleanup_root);
}

fn enforce_workspace_quota(root: &Path) -> Result<(), RouterError> {
    enforce_workspace_quota_with_limit(root, MAX_DEV_WORKSPACE_BYTES)
}

fn enforce_workspace_quota_with_limit(root: &Path, limit: u64) -> Result<(), RouterError> {
    if bounded_directory_size(root, limit)? > limit {
        return Err(RouterError::policy("dev_workspace_quota_exceeded"));
    }
    Ok(())
}

fn enforce_workspace_quota_for_roots(roots: &[&Path], limit: u64) -> Result<(), RouterError> {
    if bounded_directory_size_for_roots(roots, limit)? > limit {
        return Err(RouterError::policy("dev_workspace_quota_exceeded"));
    }
    Ok(())
}

fn harden_private_development_tree(root: &Path, executable: &Path) -> Result<(), RouterError> {
    let metadata =
        fs::symlink_metadata(root).map_err(|_| RouterError::policy("dev_artifact_invalid"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(RouterError::policy("dev_artifact_invalid"));
    }
    fs::set_permissions(root, fs::Permissions::from_mode(0o700))
        .map_err(|_| RouterError::policy("dev_artifact_invalid"))?;
    for entry in fs::read_dir(root).map_err(|_| RouterError::policy("dev_artifact_invalid"))? {
        let entry = entry.map_err(|_| RouterError::policy("dev_artifact_invalid"))?;
        let path = entry.path();
        let metadata =
            fs::symlink_metadata(&path).map_err(|_| RouterError::policy("dev_artifact_invalid"))?;
        if metadata.file_type().is_symlink() {
            return Err(RouterError::policy("dev_artifact_invalid"));
        }
        if metadata.is_dir() {
            harden_private_development_tree(&path, executable)?;
        } else if metadata.is_file() {
            let mode = if path == executable { 0o700 } else { 0o600 };
            fs::set_permissions(&path, fs::Permissions::from_mode(mode))
                .map_err(|_| RouterError::policy("dev_artifact_invalid"))?;
        } else {
            return Err(RouterError::policy("dev_artifact_invalid"));
        }
    }
    Ok(())
}

fn bounded_directory_size_for_roots(roots: &[&Path], limit: u64) -> Result<u64, RouterError> {
    let mut total = 0_u64;
    for root in roots {
        let size = bounded_directory_size(root, limit.saturating_sub(total))?;
        total = total.saturating_add(size);
        if total > limit {
            return Ok(total);
        }
    }
    Ok(total)
}

fn bounded_directory_size(root: &Path, limit: u64) -> Result<u64, RouterError> {
    bounded_directory_size_inner(root, limit, false)?
        .ok_or_else(|| RouterError::operation("dev_workspace_unavailable"))
}

fn bounded_directory_size_inner(
    root: &Path,
    limit: u64,
    allow_missing: bool,
) -> Result<Option<u64>, RouterError> {
    let metadata = match fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if allow_missing && error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(_) => return Err(RouterError::operation("dev_workspace_unavailable")),
    };
    if metadata.file_type().is_symlink() {
        return Err(RouterError::policy("dev_workspace_unsafe"));
    }
    if metadata.is_file() {
        return Ok(Some(metadata.len()));
    }
    let mut total = 0_u64;
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if allow_missing && error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(_) => return Err(RouterError::operation("dev_workspace_unavailable")),
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) if allow_missing && error.kind() == std::io::ErrorKind::NotFound => {
                continue;
            }
            Err(_) => return Err(RouterError::operation("dev_workspace_unavailable")),
        };
        let Some(size) =
            bounded_directory_size_inner(&entry.path(), limit.saturating_sub(total), true)?
        else {
            continue;
        };
        total = total.saturating_add(size);
        if total > limit {
            return Ok(Some(total));
        }
    }
    Ok(Some(total))
}

fn doctor(paths: &RouterPaths) -> Result<RouterResponse, RouterError> {
    if !paths.install_root.exists() {
        return Ok(RouterResponse::result(
            Operation::Doctor,
            true,
            "extension_not_installed",
            "denied",
        ));
    }
    let (active, _) = verified_active(paths)?;
    let mut response =
        RouterResponse::result(Operation::Doctor, true, "extension_verified", "denied");
    response.release = Some(active.release);
    response.executable_sha256 = Some(active.executable_sha256);
    response.verified = Some(true);
    Ok(response)
}

fn delegate_existing(
    operation: Operation,
    paths: &RouterPaths,
) -> Result<RouterResponse, RouterError> {
    let (active, executable) = verified_active(paths)?;
    let request = serde_json::json!({
        "operation": operation.name(),
        "schema_version": 1,
        "install_root": paths.install_root,
    });
    let delegated = run_candidate(
        &executable,
        &request,
        !matches!(
            operation,
            Operation::Launch | Operation::LiveProvider | Operation::DynamicCatalog
        ),
    )?;
    if operation == Operation::Status
        && delegated.release.is_some()
        && !delegated_status_matches_active(&delegated, &active)
    {
        return Err(RouterError::policy("candidate_response_invalid"));
    }
    response_from_delegated(operation, delegated, "denied")
}

fn delegated_status_matches_active(
    response: &DelegatedResponse,
    active: &ActiveInstallation,
) -> bool {
    response.release.as_deref() == Some(active.release.as_str())
        && response.executable_sha256.as_deref() == Some(active.executable_sha256.as_str())
        && response.target.as_deref() == Some(active.target.as_str())
        && response.bundle.as_deref() == Some(active.bundle.as_str())
        && response.asb_version.as_deref() == Some(active.asb_version.as_str())
        && response.protocol_version == Some(active.protocol_version)
        && response.source_commit.as_deref() == Some(active.source_commit.as_str())
        && response.source_tree.as_deref() == Some(active.source_tree.as_str())
}

fn response_from_delegated(
    operation: Operation,
    delegated: DelegatedResponse,
    network: &'static str,
) -> Result<RouterResponse, RouterError> {
    validate_delegated(operation, &delegated)?;
    let code = match delegated.code.as_str() {
        "extension_installed" => "extension_installed",
        "extension_upgraded" => "extension_upgraded",
        "extension_removed" => "extension_removed",
        "frontend_exited" => "frontend_exited",
        "verified_installation" => "verified_installation",
        "extension_not_installed" => "extension_not_installed",
        _ if delegated.ok => return Err(RouterError::policy("candidate_response_invalid")),
        _ => "candidate_rejected_lifecycle",
    };
    Ok(RouterResponse {
        schema_version: 1,
        ok: delegated.ok,
        command: "tui",
        operation: operation.name(),
        code,
        network,
        channel: "stable",
        development_only: false,
        classification: None,
        remediation: None,
        release: delegated.release,
        executable_sha256: delegated.executable_sha256,
        source_commit: None,
        source_tree: None,
        asb_source_commit: None,
        asb_source_tree: None,
        channel_manifest_sha256: None,
        warnings: None,
        verified: delegated.verified,
    })
}

fn acquire_document(
    source: &mut impl Source,
    cached: &Path,
    url: &str,
    maximum: usize,
    offline: bool,
) -> Result<Vec<u8>, RouterError> {
    if offline {
        return read_bounded(cached, maximum);
    }
    source.document(url, maximum)
}

fn select_release(
    bytes: &[u8],
    now: u64,
    expected_target: &str,
) -> Result<ChannelRelease, RouterError> {
    select_release_contract(bytes, Some(now), expected_target)
}

fn select_stored_release(
    bytes: &[u8],
    expected_target: &str,
) -> Result<ChannelRelease, RouterError> {
    select_release_contract(bytes, None, expected_target)
}

fn select_release_contract(
    bytes: &[u8],
    install_time: Option<u64>,
    expected_target: &str,
) -> Result<ChannelRelease, RouterError> {
    let index: ChannelIndex =
        serde_json::from_slice(bytes).map_err(|_| RouterError::policy("channel_invalid"))?;
    let invalid_time = match install_time {
        Some(now) => index.issued_unix > now || index.expires_unix <= now,
        None => index.expires_unix <= index.issued_unix,
    };
    if index.schema_version != 1
        || invalid_time
        || index.expires_unix.saturating_sub(index.issued_unix) > 31 * 24 * 60 * 60
        || index.releases.is_empty()
        || index.releases.len() > 16
    {
        return Err(RouterError::policy("channel_invalid"));
    }
    let mut seen = BTreeSet::new();
    let mut eligible = Vec::new();
    for release in index.releases {
        if !valid_release(&release.release)
            || !valid_hex(&release.manifest_sha256, 64)
            || !matches!(
                release.target.as_str(),
                "x86_64-unknown-linux-gnu" | "aarch64-unknown-linux-gnu"
            )
            || release.asb_version != env!("CARGO_PKG_VERSION")
            || release.protocol_version != 1
            || !valid_manifest_urls(&release)
            || !seen.insert((release.release.clone(), release.target.clone()))
        {
            return Err(RouterError::policy("channel_invalid"));
        }
        if release.target == expected_target {
            eligible.push(release);
        }
    }
    eligible
        .into_iter()
        .max_by_key(|entry| version_parts(&entry.release).unwrap_or((0, 0, 0)))
        .ok_or_else(|| RouterError::policy("compatible_release_unavailable"))
}

fn valid_manifest_urls(release: &ChannelRelease) -> bool {
    let prefix = format!(
        "https://github.com/martin-beck/asb-tui/releases/download/{}/",
        release.release
    );
    release.manifest_url == format!("{prefix}manifest.json")
        && release.manifest_signature_url == format!("{prefix}manifest.json.sig")
}

fn validate_manifest(
    bytes: &[u8],
    selected: &ChannelRelease,
    now: u64,
) -> Result<BundleManifest, RouterError> {
    validate_manifest_contract(bytes, selected, Some(now))
}

fn validate_installed_manifest(
    bytes: &[u8],
    selected: &ChannelRelease,
) -> Result<BundleManifest, RouterError> {
    validate_manifest_contract(bytes, selected, None)
}

fn validate_manifest_contract(
    bytes: &[u8],
    selected: &ChannelRelease,
    install_time: Option<u64>,
) -> Result<BundleManifest, RouterError> {
    let manifest: BundleManifest =
        serde_json::from_slice(bytes).map_err(|_| RouterError::policy("manifest_invalid"))?;
    let expected_bundle = if target().starts_with("x86_64") {
        "asb-tui-v1-linux-x86_64"
    } else {
        "asb-tui-v1-linux-aarch64"
    };
    let expected_arch = if target().starts_with("x86_64") {
        "x86_64"
    } else {
        "aarch64"
    };
    let invalid_time = match install_time {
        Some(now) => manifest.issued_unix > now || manifest.expires_unix <= now,
        None => manifest.expires_unix <= manifest.issued_unix,
    };
    if manifest.schema_version != 1
        || manifest.release != selected.release
        || !valid_hex(&manifest.source_commit, 40)
        || !valid_hex(&manifest.source_tree, 40)
        || invalid_time
        || manifest.expires_unix.saturating_sub(manifest.issued_unix) > 366 * 24 * 60 * 60
        || manifest.compatibility.bundle != expected_bundle
        || manifest.compatibility.architecture != expected_arch
        || manifest.compatibility.asb_version != env!("CARGO_PKG_VERSION")
        || manifest.compatibility.protocol_version != 1
        || manifest.compatibility.coordinator_version != "v0.3.5"
        || manifest.compatibility.coordinator_commit != "510817b93feb80dde13e5a6c61d657954fae2346"
        || manifest.compatibility.quality_version != "v0.23.0"
        || manifest.compatibility.quality_commit != "8a9f056b7fc7926b9465a0f7a09225d4da1c572a"
    {
        return Err(RouterError::policy("manifest_invalid"));
    }
    validate_components(&manifest)?;
    validate_artifact_inventory(&manifest)?;
    Ok(manifest)
}

fn validate_components(manifest: &BundleManifest) -> Result<(), RouterError> {
    let executable_digest = manifest
        .artifacts
        .iter()
        .find(|artifact| artifact.name == "asb-tui")
        .map(|artifact| artifact.sha256.as_str())
        .ok_or_else(|| RouterError::policy("component_invalid"))?;
    let expected = BTreeMap::from([
        (
            "asb-tui",
            (
                manifest.release.as_str(),
                manifest.source_commit.as_str(),
                manifest.source_tree.as_str(),
                executable_digest,
            ),
        ),
        (
            "agent-workflow-coordinator",
            (
                "v0.3.5",
                "510817b93feb80dde13e5a6c61d657954fae2346",
                "41d08ed42333cb47b07c2c401a9167b56c7cfb81",
                "a51e58ed71dd93979acc55560fc8208db7131d8e6d0a6b7b805fa383fef25b34",
            ),
        ),
        (
            "agent-workflow-quality",
            (
                "v0.23.0",
                "8a9f056b7fc7926b9465a0f7a09225d4da1c572a",
                "ca77db478f0737142690d37683d826710cd953b0",
                "9d480cabe955a5faf88f6f1c8dfd2bfe63045445173c86f93d200a00c97fff89",
            ),
        ),
    ]);
    let mut observed = BTreeMap::new();
    for component in &manifest.components {
        if !valid_release(&component.version)
            || !valid_hex(&component.commit, 40)
            || !valid_hex(&component.tree, 40)
            || !valid_hex(&component.artifact_sha256, 64)
            || observed
                .insert(
                    component.name.as_str(),
                    (
                        component.version.as_str(),
                        component.commit.as_str(),
                        component.tree.as_str(),
                        component.artifact_sha256.as_str(),
                    ),
                )
                .is_some()
        {
            return Err(RouterError::policy("component_invalid"));
        }
    }
    if observed != expected {
        return Err(RouterError::policy("component_invalid"));
    }
    Ok(())
}

fn validate_artifact_inventory(manifest: &BundleManifest) -> Result<(), RouterError> {
    let expected = BTreeSet::from([
        ("asb-tui", "executable"),
        ("licenses", "license_report"),
        ("provenance", "provenance"),
        ("sbom", "sbom"),
        ("source", "source"),
    ]);
    let mut observed = BTreeSet::new();
    let mut total = 0_u64;
    let prefix = format!(
        "https://github.com/martin-beck/asb-tui/releases/download/{}/",
        manifest.release
    );
    for artifact in &manifest.artifacts {
        total = total
            .checked_add(artifact.size)
            .ok_or_else(|| RouterError::policy("artifact_quota_exceeded"))?;
        if artifact.size == 0
            || artifact.size > MAX_ARTIFACT_BYTES
            || !valid_hex(&artifact.sha256, 64)
            || !artifact.url.starts_with(&prefix)
            || artifact.url[prefix.len()..].contains('/')
            || artifact.url.contains(['?', '#', '%'])
            || !observed.insert((artifact.name.as_str(), artifact.kind.as_str()))
        {
            return Err(RouterError::policy("artifact_invalid"));
        }
    }
    if observed != expected || total > MAX_BUNDLE_BYTES {
        return Err(RouterError::policy("artifact_set_incomplete"));
    }
    Ok(())
}

fn obtain_artifacts(
    manifest: &BundleManifest,
    source: &mut impl Source,
    root: &Path,
    offline: bool,
) -> Result<BTreeMap<String, Vec<u8>>, RouterError> {
    let mut result = BTreeMap::new();
    for artifact in &manifest.artifacts {
        let path = root.join(&artifact.name);
        let bytes = if let Ok(bytes) = read_exact_digest(&path, artifact.size, &artifact.sha256) {
            bytes
        } else if offline {
            return Err(RouterError::policy("offline_artifact_unavailable"));
        } else {
            let bytes = download_resumable(artifact, source, &path)?;
            atomic_private(
                &path,
                &bytes,
                if artifact.kind == "executable" {
                    0o700
                } else {
                    0o600
                },
            )?;
            bytes
        };
        result.insert(artifact.name.clone(), bytes);
    }
    Ok(result)
}

fn download_resumable(
    artifact: &BundleArtifact,
    source: &mut impl Source,
    final_path: &Path,
) -> Result<Vec<u8>, RouterError> {
    let partial = final_path.with_extension("partial");
    let mut bytes = if partial.exists() {
        read_bounded(&partial, artifact.size as usize).unwrap_or_default()
    } else {
        Vec::new()
    };
    if bytes.len() as u64 >= artifact.size {
        bytes.clear();
    }
    while (bytes.len() as u64) < artifact.size {
        let remaining = usize::try_from(artifact.size - bytes.len() as u64)
            .map_err(|_| RouterError::policy("artifact_quota_exceeded"))?;
        let request = remaining.min(CHUNK_BYTES);
        let mut chunk = None;
        for _ in 0..TRANSFER_ATTEMPTS {
            if let Ok(value) = source.range(&artifact.url, bytes.len() as u64, request)
                && !value.is_empty()
                && value.len() <= request
            {
                chunk = Some(value);
                break;
            }
        }
        let value = chunk.ok_or_else(|| RouterError::operation("artifact_transfer_failed"))?;
        bytes.extend_from_slice(&value);
        if bytes.len() as u64 > artifact.size {
            let _ = remove_private_file(&partial);
            return Err(RouterError::policy("artifact_size_mismatch"));
        }
        atomic_private(&partial, &bytes, 0o600)?;
    }
    if digest(&bytes) != artifact.sha256 {
        let _ = remove_private_file(&partial);
        return Err(RouterError::policy("artifact_digest_mismatch"));
    }
    remove_private_file(&partial)?;
    Ok(bytes)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LicenseReport {
    schema_version: u64,
    release: String,
    packages: Vec<LicensePackage>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LicensePackage {
    name: String,
    version: String,
    license: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Provenance {
    schema_version: u64,
    release: String,
    source_commit: String,
    source_tree: String,
    builder: String,
    reproducible: bool,
}

fn validate_documents(
    manifest: &BundleManifest,
    artifacts: &BTreeMap<String, Vec<u8>>,
) -> Result<(), RouterError> {
    let licenses: LicenseReport = serde_json::from_slice(
        artifacts
            .get("licenses")
            .ok_or_else(|| RouterError::policy("license_report_missing"))?,
    )
    .map_err(|_| RouterError::policy("license_report_invalid"))?;
    if licenses.schema_version != 1 || licenses.release != manifest.release {
        return Err(RouterError::policy("license_report_invalid"));
    }
    let mut license_identities = BTreeSet::new();
    for package in licenses.packages {
        if package.name.is_empty()
            || package.version.is_empty()
            || !(matches!(
                package.license.as_str(),
                "MIT"
                    | "Apache-2.0"
                    | "MIT OR Apache-2.0"
                    | "Apache-2.0 OR BSL-1.0"
                    | "Unlicense OR MIT"
                    | "(MIT OR Apache-2.0) AND Unicode-3.0"
            ) || package.name == "foldhash"
                && package.version == "0.2.0"
                && package.license == "Zlib")
            || !license_identities.insert((package.name, package.version, package.license))
        {
            return Err(RouterError::policy("license_policy_rejected"));
        }
    }
    if ![
        "asb-tui",
        "agent-workflow-coordinator",
        "agent-workflow-quality",
    ]
    .iter()
    .all(|name| {
        license_identities
            .iter()
            .any(|identity| identity.0 == *name)
    }) {
        return Err(RouterError::policy("license_report_incomplete"));
    }
    let sbom: serde_json::Value = serde_json::from_slice(
        artifacts
            .get("sbom")
            .ok_or_else(|| RouterError::policy("sbom_missing"))?,
    )
    .map_err(|_| RouterError::policy("sbom_invalid"))?;
    let sbom_packages = sbom
        .get("packages")
        .and_then(|value| value.as_array())
        .ok_or_else(|| RouterError::policy("sbom_invalid"))?;
    let mut sbom_identities = BTreeSet::new();
    for package in sbom_packages {
        let identity = (
            package.get("name").and_then(|value| value.as_str()),
            package.get("versionInfo").and_then(|value| value.as_str()),
            package
                .get("licenseDeclared")
                .and_then(|value| value.as_str()),
        );
        let (Some(name), Some(version), Some(license)) = identity else {
            return Err(RouterError::policy("sbom_invalid"));
        };
        if name.is_empty()
            || version.is_empty()
            || license.is_empty()
            || !sbom_identities.insert((name.to_owned(), version.to_owned(), license.to_owned()))
        {
            return Err(RouterError::policy("sbom_invalid"));
        }
    }
    if sbom.get("spdxVersion").and_then(|value| value.as_str()) != Some("SPDX-2.3")
        || sbom.get("name").and_then(|value| value.as_str())
            != Some(format!("asb-tui-{}", manifest.release).as_str())
        || license_identities != sbom_identities
    {
        return Err(RouterError::policy("sbom_invalid"));
    }
    let provenance: Provenance = serde_json::from_slice(
        artifacts
            .get("provenance")
            .ok_or_else(|| RouterError::policy("provenance_missing"))?,
    )
    .map_err(|_| RouterError::policy("provenance_invalid"))?;
    if provenance.schema_version != 1
        || provenance.release != manifest.release
        || provenance.source_commit != manifest.source_commit
        || provenance.source_tree != manifest.source_tree
        || provenance.builder != "github-actions"
        || !provenance.reproducible
    {
        return Err(RouterError::policy("provenance_invalid"));
    }
    Ok(())
}

fn verified_active(paths: &RouterPaths) -> Result<(ActiveInstallation, Vec<u8>), RouterError> {
    validate_private_directory(&paths.install_root)?;
    let bytes = read_bounded(&paths.install_root.join("active.json"), 64 * 1024)
        .map_err(|_| RouterError::policy("extension_not_installed"))?;
    let active: ActiveInstallation = serde_json::from_slice(&bytes)
        .map_err(|_| RouterError::policy("installation_verification_failed"))?;
    let (selected, manifest) = authenticated_manifest_for_active(&paths.state_root, &active)?;
    if !valid_active(&active, &manifest) || selected.release != active.release {
        return Err(RouterError::policy("installation_verification_failed"));
    }
    let executable = read_bounded_digest(
        &paths
            .install_root
            .join("versions")
            .join(&active.executable_sha256)
            .join("asb-tui"),
        MAX_ARTIFACT_BYTES,
        &active.executable_sha256,
    )?;
    Ok((active, executable))
}

fn valid_active(active: &ActiveInstallation, manifest: &BundleManifest) -> bool {
    let executable_sha256 = manifest
        .artifacts
        .iter()
        .find(|artifact| artifact.name == "asb-tui" && artifact.kind == "executable")
        .map(|artifact| artifact.sha256.as_str());
    active.schema_version == 1
        && active.classification == "verified_extension"
        && active.release == manifest.release
        && active.executable_sha256.as_str() == executable_sha256.unwrap_or_default()
        && active.source_commit == manifest.source_commit
        && active.source_tree == manifest.source_tree
        && active.target == target()
        && active.bundle == manifest.compatibility.bundle
        && active.asb_version == manifest.compatibility.asb_version
        && active.protocol_version == manifest.compatibility.protocol_version
        && active.coordinator_version == manifest.compatibility.coordinator_version
        && active.coordinator_commit == manifest.compatibility.coordinator_commit
        && active.quality_version == manifest.compatibility.quality_version
        && active.quality_commit == manifest.compatibility.quality_commit
}

fn persist_verified_promotion(
    state_root: &Path,
    selected: &ChannelRelease,
    channel: &[u8],
    channel_signature: &[u8],
    manifest: &[u8],
    signature: &[u8],
) -> Result<(), RouterError> {
    if digest(manifest) != selected.manifest_sha256 {
        return Err(RouterError::policy("manifest_digest_mismatch"));
    }
    verify_signature(channel, channel_signature, CHANNEL_NAMESPACE)?;
    let stored = select_stored_release(channel, target())?;
    if stored.release != selected.release || stored.manifest_sha256 != selected.manifest_sha256 {
        return Err(RouterError::policy("channel_invalid"));
    }
    verify_signature(manifest, signature, BUNDLE_NAMESPACE)?;
    let root = state_root.join("manifests").join(&selected.manifest_sha256);
    prepare_private_directory(&root)?;
    atomic_private(&root.join("channel.json"), channel, 0o600)?;
    atomic_private(&root.join("channel.json.sig"), channel_signature, 0o600)?;
    atomic_private(&root.join("manifest.json"), manifest, 0o600)?;
    atomic_private(&root.join("manifest.json.sig"), signature, 0o600)
}

fn authenticated_manifest_for_active(
    state_root: &Path,
    active: &ActiveInstallation,
) -> Result<(ChannelRelease, BundleManifest), RouterError> {
    for floor_name in ["accepted.json", "pending.json"] {
        let floor_bytes = match read_bounded(&state_root.join(floor_name), 4096) {
            Ok(bytes) => bytes,
            Err(error) if error.code == "cached_input_unavailable" => continue,
            Err(_) => return Err(RouterError::policy("installation_verification_failed")),
        };
        let floor: AcceptedRelease = serde_json::from_slice(&floor_bytes)
            .map_err(|_| RouterError::policy("installation_verification_failed"))?;
        if floor.schema_version != 1
            || !valid_release(&floor.release)
            || !valid_hex(&floor.manifest_sha256, 64)
        {
            return Err(RouterError::policy("installation_verification_failed"));
        }
        if floor.release != active.release {
            continue;
        }
        let root = state_root.join("manifests").join(&floor.manifest_sha256);
        let channel = read_bounded(&root.join("channel.json"), MAX_INDEX_BYTES)
            .map_err(|_| RouterError::policy("installation_verification_failed"))?;
        let channel_signature =
            read_bounded(&root.join("channel.json.sig"), MAX_SIGNATURE_BYTES)
                .map_err(|_| RouterError::policy("installation_verification_failed"))?;
        let manifest_bytes = read_bounded(&root.join("manifest.json"), MAX_MANIFEST_BYTES)
            .map_err(|_| RouterError::policy("installation_verification_failed"))?;
        let signature = read_bounded(&root.join("manifest.json.sig"), MAX_SIGNATURE_BYTES)
            .map_err(|_| RouterError::policy("installation_verification_failed"))?;
        if digest(&manifest_bytes) != floor.manifest_sha256 {
            return Err(RouterError::policy("installation_verification_failed"));
        }
        verify_signature(&channel, &channel_signature, CHANNEL_NAMESPACE)
            .map_err(|_| RouterError::policy("installation_verification_failed"))?;
        let promoted = select_stored_release(&channel, target())
            .map_err(|_| RouterError::policy("installation_verification_failed"))?;
        if promoted.release != floor.release || promoted.manifest_sha256 != floor.manifest_sha256 {
            return Err(RouterError::policy("installation_verification_failed"));
        }
        verify_signature(&manifest_bytes, &signature, BUNDLE_NAMESPACE)
            .map_err(|_| RouterError::policy("installation_verification_failed"))?;
        let selected = promoted;
        let manifest = validate_installed_manifest(&manifest_bytes, &selected)
            .map_err(|_| RouterError::policy("installation_verification_failed"))?;
        return Ok((selected, manifest));
    }
    Err(RouterError::policy("installation_verification_failed"))
}

fn run_candidate(
    executable: &[u8],
    request: &serde_json::Value,
    bounded: bool,
) -> Result<DelegatedResponse, RouterError> {
    let descriptor = memfd_create("asb-tui-candidate", MemfdFlags::ALLOW_SEALING)
        .map_err(|_| RouterError::operation("candidate_execution_failed"))?;
    let mut file: File = descriptor.into();
    file.write_all(executable)
        .and_then(|()| file.sync_all())
        .and_then(|()| file.set_permissions(fs::Permissions::from_mode(0o700)))
        .map_err(|_| RouterError::operation("candidate_execution_failed"))?;
    fcntl_add_seals(
        &file,
        SealFlags::WRITE | SealFlags::GROW | SealFlags::SHRINK | SealFlags::SEAL,
    )
    .map_err(|_| RouterError::operation("candidate_execution_failed"))?;
    let program = format!("/proc/self/fd/{}", file.as_raw_fd());
    let mut command = Command::new(program);
    command
        .args(["lifecycle", "--format", "json"])
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("LANG", "C.UTF-8");
    // A launched frontend owns the controlling terminal for interactive input;
    // placing it in a background process group would make a read from its
    // controlling tty receive SIGTTIN. Bounded lifecycle probes retain their
    // isolated process group for descendant cleanup.
    if bounded {
        command.process_group(0);
    }
    add_candidate_environment(&mut command)?;
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| RouterError::operation("candidate_execution_failed"))?;
    let pid = Pid::from_child(&child);
    let encoded = serde_json::to_vec(request)
        .map_err(|_| RouterError::operation("candidate_request_failed"))?;
    child
        .stdin
        .take()
        .ok_or_else(|| RouterError::operation("candidate_request_failed"))?
        .write_all(&encoded)
        .map_err(|_| RouterError::operation("candidate_request_failed"))?;
    let mut output = child
        .stdout
        .take()
        .ok_or_else(|| RouterError::operation("candidate_response_invalid"))?;
    // A candidate may accidentally leave a descendant holding stdout open.
    // Make the pipe nonblocking and drain it after the leader exits, so the
    // router never waits for an unowned descendant or reader thread.
    let flags =
        fcntl_getfl(&output).map_err(|_| RouterError::operation("candidate_response_invalid"))?;
    fcntl_setfl(&output, flags | OFlags::NONBLOCK)
        .map_err(|_| RouterError::operation("candidate_response_invalid"))?;
    let status = wait_child(&mut child, pid, bounded)?;
    let bytes = drain_candidate_output(&mut output, status.success())?;
    if bytes.len() as u64 > RESPONSE_BYTES || !matches!(status.code(), Some(0 | 3)) {
        return Err(RouterError::policy("candidate_response_invalid"));
    }
    let response: DelegatedResponse = serde_json::from_slice(&bytes)
        .map_err(|_| RouterError::policy("candidate_response_invalid"))?;
    if response.ok != status.success() {
        return Err(RouterError::policy("candidate_response_invalid"));
    }
    Ok(response)
}

fn drain_candidate_output(
    output: &mut impl Read,
    leader_succeeded: bool,
) -> Result<Vec<u8>, RouterError> {
    let deadline = Instant::now() + Duration::from_millis(250);
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        match output.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                bytes.extend_from_slice(&buffer[..count]);
                if bytes.len() as u64 > RESPONSE_BYTES {
                    return Err(RouterError::policy("candidate_response_invalid"));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    break;
                }
                thread::sleep(Duration::from_millis(5));
            }
            Err(_) => return Err(RouterError::operation("candidate_response_invalid")),
        }
    }
    if bytes.is_empty() && leader_succeeded {
        return Err(RouterError::policy("candidate_response_invalid"));
    }
    Ok(bytes)
}

fn add_candidate_environment(command: &mut Command) -> Result<(), RouterError> {
    for name in [
        "HOME",
        "XDG_CONFIG_HOME",
        "XDG_CACHE_HOME",
        "XDG_DATA_HOME",
        "XDG_STATE_HOME",
        "XDG_RUNTIME_DIR",
    ] {
        if let Some(value) = std::env::var_os(name) {
            if value.is_empty() {
                continue;
            }
            if !environment_path_safe(Path::new(&value)) {
                return Err(RouterError::policy("environment_path_invalid"));
            }
            command.env(name, value);
        }
    }
    for name in ["TERM", "COLORTERM"] {
        if let Some(value) = std::env::var_os(name) {
            let bytes = value.as_encoded_bytes();
            if bytes.is_empty() {
                continue;
            }
            if !valid_terminal_value(bytes) {
                return Err(RouterError::policy("environment_terminal_invalid"));
            }
            command.env(name, value);
        }
    }
    Ok(())
}

fn valid_terminal_value(bytes: &[u8]) -> bool {
    bytes.len() <= 64
        && !bytes.is_empty()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, 43 | 45 | 46 | 95))
}

fn environment_path_safe(path: &Path) -> bool {
    if !safe_absolute(path) {
        return false;
    }
    let Ok(mut current) = File::open("/") else {
        return false;
    };
    for component in path.components() {
        let Component::Normal(part) = component else {
            continue;
        };
        match openat(
            &current,
            part,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(next) => current = File::from(next),
            Err(rustix::io::Errno::NOENT) => {
                let Ok(metadata) = current.metadata() else {
                    return false;
                };
                return metadata.uid() == rustix::process::geteuid().as_raw()
                    && metadata.mode() & 0o022 == 0;
            }
            Err(_) => return false,
        }
    }
    current.metadata().is_ok_and(|metadata| {
        metadata.uid() == rustix::process::geteuid().as_raw() && metadata.mode() & 0o022 == 0
    })
}

fn wait_child(
    child: &mut Child,
    pid: Pid,
    bounded: bool,
) -> Result<std::process::ExitStatus, RouterError> {
    let started = Instant::now();
    loop {
        if waitid(
            WaitId::Pid(pid),
            WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
        )
        .map_err(|_| RouterError::operation("candidate_execution_failed"))?
        .is_some()
        {
            // Keep the exited leader unreaped while terminating its owned
            // process group. This fences PID reuse and closes response pipes
            // retained by a misbehaving descendant before the reader joins.
            match kill_process_group(pid, Signal::KILL) {
                Ok(()) | Err(rustix::io::Errno::SRCH) => {}
                Err(_) => return Err(RouterError::operation("candidate_execution_failed")),
            }
            return child
                .wait()
                .map_err(|_| RouterError::operation("candidate_execution_failed"));
        }
        if bounded && started.elapsed() >= DELEGATE_TIMEOUT {
            let _ = kill_process_group(pid, Signal::KILL);
            let _ = child.wait();
            return Err(RouterError::operation("candidate_timeout"));
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn validate_delegated(
    operation: Operation,
    response: &DelegatedResponse,
) -> Result<(), RouterError> {
    if response.schema_version != 1
        || !matches!(
            response.classification.as_str(),
            "source_only_unverified" | "verified_extension"
        )
        || response.code.is_empty()
        || response.code.len() > 64
        || !response.code.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || (index > 0 && byte == b'_')
        })
        || response.verified == Some(true) && response.classification != "verified_extension"
    {
        return Err(RouterError::policy("candidate_response_invalid"));
    }
    let no_identity = response.installed.is_none()
        && response.verified.is_none()
        && response.release.is_none()
        && response.executable_sha256.is_none()
        && response.target.is_none()
        && response.bundle.is_none()
        && response.asb_version.is_none()
        && response.protocol_version.is_none()
        && response.source_commit.is_none()
        && response.source_tree.is_none();
    if response.ok {
        if response.classification != "verified_extension" {
            return Err(RouterError::policy("candidate_response_invalid"));
        }
        let expected = match operation {
            Operation::Install => "extension_installed",
            Operation::Upgrade => "extension_upgraded",
            Operation::Remove => "extension_removed",
            Operation::Launch | Operation::LiveProvider | Operation::DynamicCatalog => {
                "frontend_exited"
            }
            Operation::Status => "verified_installation",
            Operation::Preflight | Operation::Doctor | Operation::Help | Operation::Version => {
                return Err(RouterError::policy("candidate_response_invalid"));
            }
        };
        if response.code != expected {
            return Err(RouterError::policy("candidate_response_invalid"));
        }
        if operation == Operation::Status {
            if !valid_verified_status(response) {
                return Err(RouterError::policy("candidate_response_invalid"));
            }
        } else if !no_identity {
            return Err(RouterError::policy("candidate_response_invalid"));
        }
    } else if operation == Operation::Status {
        let absent = response.code == "extension_not_installed"
            && response.installed == Some(false)
            && response.verified == Some(false)
            && response.release.is_none()
            && response.executable_sha256.is_none()
            && response.target.is_none()
            && response.bundle.is_none()
            && response.asb_version.is_none()
            && response.protocol_version.is_none()
            && response.source_commit.is_none()
            && response.source_tree.is_none();
        let invalid = response.code == "installation_verification_failed"
            && response.installed == Some(true)
            && response.verified == Some(false)
            && valid_status_identity(response);
        if !absent && !invalid {
            return Err(RouterError::policy("candidate_response_invalid"));
        }
    } else if !no_identity {
        return Err(RouterError::policy("candidate_response_invalid"));
    }
    Ok(())
}

/// The lifecycle operation is carried in the authenticated JSON request for
/// installed bundles; the development executable receives the same route
/// explicitly because it is launched directly after the broker handoff.
fn frontend_route_flag(operation: Operation) -> Option<&'static str> {
    match operation {
        Operation::LiveProvider => Some("--live-provider"),
        Operation::DynamicCatalog => Some("--dynamic-catalog"),
        _ => None,
    }
}

fn valid_verified_status(response: &DelegatedResponse) -> bool {
    response.classification == "verified_extension"
        && response.installed == Some(true)
        && response.verified == Some(true)
        && valid_status_identity(response)
}

fn valid_status_identity(response: &DelegatedResponse) -> bool {
    let expected_bundle = if target().starts_with("x86_64") {
        "asb-tui-v1-linux-x86_64"
    } else {
        "asb-tui-v1-linux-aarch64"
    };
    response.release.as_deref().is_some_and(valid_release)
        && response
            .executable_sha256
            .as_deref()
            .is_some_and(|value| valid_hex(value, 64))
        && response.target.as_deref() == Some(target())
        && response.bundle.as_deref() == Some(expected_bundle)
        && response.asb_version.as_deref() == Some(env!("CARGO_PKG_VERSION"))
        && response.protocol_version == Some(1)
        && response
            .source_commit
            .as_deref()
            .is_some_and(|value| valid_hex(value, 40))
        && response
            .source_tree
            .as_deref()
            .is_some_and(|value| valid_hex(value, 40))
}

fn verify_signature(bytes: &[u8], signature: &[u8], namespace: &str) -> Result<(), RouterError> {
    verify_signature_with(
        bytes,
        signature,
        namespace,
        TRUSTED_SIGNERS,
        SIGNER_FINGERPRINT,
    )
}

fn verify_signature_with(
    bytes: &[u8],
    signature: &[u8],
    namespace: &str,
    allowed_signers: &[u8],
    fingerprint: &str,
) -> Result<(), RouterError> {
    let mut anchor: File = memfd_create("asb-tui-router-signers", MemfdFlags::ALLOW_SEALING)
        .map_err(|_| RouterError::operation("signature_verifier_unavailable"))?
        .into();
    anchor
        .write_all(allowed_signers)
        .and_then(|()| anchor.sync_all())
        .map_err(|_| RouterError::operation("signature_verifier_unavailable"))?;
    fcntl_add_seals(
        &anchor,
        SealFlags::WRITE | SealFlags::GROW | SealFlags::SHRINK | SealFlags::SEAL,
    )
    .map_err(|_| RouterError::operation("signature_verifier_unavailable"))?;
    let mut signature_file: File =
        memfd_create("asb-tui-router-signature", MemfdFlags::ALLOW_SEALING)
            .map_err(|_| RouterError::operation("signature_verifier_unavailable"))?
            .into();
    signature_file
        .write_all(signature)
        .and_then(|()| signature_file.sync_all())
        .map_err(|_| RouterError::operation("signature_verifier_unavailable"))?;
    fcntl_add_seals(
        &signature_file,
        SealFlags::WRITE | SealFlags::GROW | SealFlags::SHRINK | SealFlags::SEAL,
    )
    .map_err(|_| RouterError::operation("signature_verifier_unavailable"))?;
    let anchor_path = format!("/proc/self/fd/{}", anchor.as_raw_fd());
    let signature_path = format!("/proc/self/fd/{}", signature_file.as_raw_fd());
    validate_tool(SSH_KEYGEN)?;
    let fingerprints = Command::new(SSH_KEYGEN)
        .env_clear()
        .env("LANG", "C.UTF-8")
        .args(["-lf", &anchor_path])
        .output()
        .map_err(|_| RouterError::operation("signature_verifier_unavailable"))?;
    let rendered_fingerprints = String::from_utf8_lossy(&fingerprints.stdout);
    let observed: Vec<_> = rendered_fingerprints
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .collect();
    if !fingerprints.status.success() || observed != [fingerprint] {
        return Err(RouterError::policy("trust_anchor_invalid"));
    }
    let mut child = Command::new(SSH_KEYGEN)
        .env_clear()
        .env("LANG", "C.UTF-8")
        .args([
            "-Y",
            "verify",
            "-f",
            &anchor_path,
            "-I",
            SIGNER_IDENTITY,
            "-n",
            namespace,
            "-s",
            &signature_path,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| RouterError::operation("signature_verifier_unavailable"))?;
    child
        .stdin
        .take()
        .ok_or_else(|| RouterError::operation("signature_verifier_unavailable"))?
        .write_all(bytes)
        .map_err(|_| RouterError::operation("signature_verifier_unavailable"))?;
    child
        .wait()
        .map_err(|_| RouterError::operation("signature_verifier_unavailable"))?
        .success()
        .then_some(())
        .ok_or_else(|| RouterError::policy("signature_invalid"))
}

fn enforce_rollback(root: &Path, selected: &ChannelRelease) -> Result<(), RouterError> {
    for name in ["accepted.json", "pending.json"] {
        let path = root.join(name);
        let accepted = match read_bounded(&path, 4096) {
            Ok(bytes) => serde_json::from_slice::<AcceptedRelease>(&bytes)
                .map_err(|_| RouterError::policy("rollback_state_invalid"))?,
            Err(error) if error.code == "cached_input_unavailable" => continue,
            Err(_) => return Err(RouterError::policy("rollback_state_invalid")),
        };
        if accepted.schema_version != 1
            || !valid_release(&accepted.release)
            || !valid_hex(&accepted.manifest_sha256, 64)
            || version_parts(&selected.release)? < version_parts(&accepted.release)?
            || selected.release == accepted.release
                && selected.manifest_sha256 != accepted.manifest_sha256
        {
            return Err(RouterError::policy("rollback_rejected"));
        }
    }
    Ok(())
}

fn record_accepted(root: &Path, selected: &ChannelRelease) -> Result<(), RouterError> {
    record_floor(root, "accepted.json", selected)
}

fn record_floor(root: &Path, name: &str, selected: &ChannelRelease) -> Result<(), RouterError> {
    let bytes = serde_json::to_vec(&AcceptedRelease {
        schema_version: 1,
        release: selected.release.clone(),
        manifest_sha256: selected.manifest_sha256.clone(),
    })
    .map_err(|_| RouterError::operation("rollback_state_failed"))?;
    atomic_private(&root.join(name), &bytes, 0o600)
}

fn prepare_private_directory(path: &Path) -> Result<(), RouterError> {
    if !safe_absolute(path) {
        return Err(RouterError::policy("state_path_invalid"));
    }
    open_private_directory(path, true).map(|_| ())
}

fn prepare_private_directory_with_notice(
    path: &Path,
    purpose: &str,
    progress: &mut dyn Write,
    human: bool,
) -> Result<(), RouterError> {
    let missing = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata.file_type().is_symlink() || !metadata.is_dir(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => true,
        Err(_) => false,
    };
    if human && missing {
        writeln!(
            progress,
            "[WAIT] ASB will create directory {} for {}.",
            path.display(),
            purpose
        )
        .map_err(|_| RouterError::operation("output_unavailable"))?;
        progress
            .flush()
            .map_err(|_| RouterError::operation("output_unavailable"))?;
    }
    prepare_private_directory(path)
}

fn validate_private_directory(path: &Path) -> Result<(), RouterError> {
    open_private_directory(path, false)
        .map(|_| ())
        .map_err(|error| {
            if error.code == "state_unavailable" {
                RouterError::policy("extension_not_installed")
            } else {
                error
            }
        })
}

fn open_private_directory(path: &Path, create: bool) -> Result<File, RouterError> {
    if !safe_absolute(path) {
        return Err(RouterError::policy("state_path_invalid"));
    }
    let mut current = File::open("/").map_err(|_| RouterError::operation("state_unavailable"))?;
    for component in path.components() {
        if let Component::Normal(part) = component {
            if create {
                match mkdirat(&current, part, Mode::from_raw_mode(0o700)) {
                    Ok(()) | Err(rustix::io::Errno::EXIST) => {}
                    Err(_) => return Err(RouterError::operation("state_unavailable")),
                }
            }
            let next = openat(
                &current,
                part,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|_| RouterError::operation("state_unavailable"))?;
            current = File::from(next);
        }
    }
    let metadata = current
        .metadata()
        .map_err(|_| RouterError::operation("state_unavailable"))?;
    if !metadata.is_dir()
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.mode() & 0o077 != 0
    {
        return Err(RouterError::policy("state_path_invalid"));
    }
    Ok(current)
}

fn open_lock(path: &Path) -> Result<File, RouterError> {
    let (directory, name) = open_parent(path, true)?;
    let file = openat(
        &directory,
        name,
        OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
    )
    .map(File::from)
    .map_err(|_| RouterError::operation("lifecycle_busy"))?;
    let metadata = file
        .metadata()
        .map_err(|_| RouterError::operation("lifecycle_busy"))?;
    if !metadata.is_file()
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.mode() & 0o077 != 0
    {
        return Err(RouterError::policy("state_path_invalid"));
    }
    Ok(file)
}

fn atomic_private(path: &Path, bytes: &[u8], mode: u32) -> Result<(), RouterError> {
    let (directory, name) = open_parent(path, true)?;
    let temporary = format!(
        ".tmp-{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| RouterError::operation("state_write_failed"))?
            .as_nanos(),
        TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let mut file = openat(
        &directory,
        &temporary,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_raw_mode(mode),
    )
    .map(File::from)
    .map_err(|_| RouterError::operation("state_write_failed"))?;
    if file.write_all(bytes).is_err() || file.sync_all().is_err() {
        let _ = unlinkat(&directory, &temporary, AtFlags::empty());
        return Err(RouterError::operation("state_write_failed"));
    }
    renameat(&directory, &temporary, &directory, name).map_err(|_| {
        let _ = unlinkat(&directory, &temporary, AtFlags::empty());
        RouterError::operation("state_write_failed")
    })?;
    fsync(&directory).map_err(|_| RouterError::operation("state_write_failed"))
}

fn read_bounded(path: &Path, maximum: usize) -> Result<Vec<u8>, RouterError> {
    let (directory, name) =
        open_parent(path, false).map_err(|_| RouterError::policy("cached_input_unavailable"))?;
    let file = openat(
        &directory,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(|_| RouterError::policy("cached_input_unavailable"))?;
    let opened = file
        .metadata()
        .map_err(|_| RouterError::policy("cached_input_invalid"))?;
    if !opened.is_file() || opened.len() > maximum as u64 {
        return Err(RouterError::policy("cached_input_invalid"));
    }
    let mut bytes = Vec::new();
    file.take(maximum as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| RouterError::policy("cached_input_invalid"))?;
    if bytes.len() > maximum {
        return Err(RouterError::policy("cached_input_invalid"));
    }
    Ok(bytes)
}

fn open_parent(path: &Path, create: bool) -> Result<(File, &std::ffi::OsStr), RouterError> {
    let parent = path
        .parent()
        .ok_or_else(|| RouterError::policy("state_path_invalid"))?;
    let name = path
        .file_name()
        .ok_or_else(|| RouterError::policy("state_path_invalid"))?;
    Ok((open_private_directory(parent, create)?, name))
}

fn remove_private_file(path: &Path) -> Result<(), RouterError> {
    let (directory, name) = open_parent(path, false)?;
    match unlinkat(&directory, name, AtFlags::empty()) {
        Ok(()) | Err(rustix::io::Errno::NOENT) => {
            fsync(&directory).map_err(|_| RouterError::operation("state_write_failed"))
        }
        Err(_) => Err(RouterError::operation("state_write_failed")),
    }
}

fn read_exact_digest(path: &Path, size: u64, expected: &str) -> Result<Vec<u8>, RouterError> {
    let maximum = usize::try_from(size).map_err(|_| RouterError::policy("artifact_too_large"))?;
    let bytes = read_bounded(path, maximum)?;
    if bytes.len() as u64 != size || digest(&bytes) != expected {
        return Err(RouterError::policy("artifact_digest_mismatch"));
    }
    Ok(bytes)
}

fn read_bounded_digest(path: &Path, maximum: u64, expected: &str) -> Result<Vec<u8>, RouterError> {
    let maximum =
        usize::try_from(maximum).map_err(|_| RouterError::policy("artifact_too_large"))?;
    let bytes = read_bounded(path, maximum)?;
    if bytes.is_empty() || digest(&bytes) != expected {
        return Err(RouterError::policy("artifact_digest_mismatch"));
    }
    Ok(bytes)
}

fn system_now_unix() -> Result<u64, RouterError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| RouterError::policy("system_clock_invalid"))?
        .as_secs();
    (1_704_067_200..=4_102_444_800)
        .contains(&now)
        .then_some(now)
        .ok_or_else(|| RouterError::policy("system_clock_invalid"))
}

fn target() -> &'static str {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    return "x86_64-unknown-linux-gnu";
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    return "aarch64-unknown-linux-gnu";
    #[allow(unreachable_code)]
    "unsupported"
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn safe_absolute(path: &Path) -> bool {
    path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::RootDir | Component::Normal(_)))
}

fn validate_tool(path: &str) -> Result<(), RouterError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| RouterError::operation("trusted_tool_unavailable"))?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.uid() != 0
        || metadata.mode() & 0o022 != 0
    {
        return Err(RouterError::policy("trusted_tool_invalid"));
    }
    Ok(())
}

fn resolve_development_tool(override_name: &str, fallback: &str) -> Result<PathBuf, RouterError> {
    resolve_development_tool_from(std::env::var_os(override_name).as_deref(), fallback)
}

fn resolve_development_tool_candidates(
    override_name: &str,
    fallbacks: &[&str],
) -> Result<PathBuf, RouterError> {
    resolve_development_tool_candidates_from(std::env::var_os(override_name).as_deref(), fallbacks)
}

fn resolve_development_tool_candidates_from(
    override_path: Option<&std::ffi::OsStr>,
    fallbacks: &[&str],
) -> Result<PathBuf, RouterError> {
    if let Some(value) = override_path {
        return validate_development_tool(&PathBuf::from(value))
            .map_err(|_| RouterError::policy("trusted_tool_invalid"));
    }
    for fallback in fallbacks {
        match validate_development_tool(Path::new(fallback)) {
            Ok(path) => return Ok(path),
            Err(error) if error.code == "trusted_tool_invalid" => return Err(error),
            Err(_) => {}
        }
    }
    Err(RouterError::operation("trusted_tool_unavailable"))
}

fn resolve_development_tool_from(
    override_path: Option<&std::ffi::OsStr>,
    fallback: &str,
) -> Result<PathBuf, RouterError> {
    if let Some(value) = override_path {
        return validate_development_tool(&PathBuf::from(value))
            .map_err(|_| RouterError::policy("trusted_tool_invalid"));
    }
    match validate_development_tool(Path::new(fallback)) {
        Ok(path) => Ok(path),
        Err(error) if error.code == "trusted_tool_invalid" => Err(error),
        Err(_) => Err(RouterError::operation("trusted_tool_unavailable")),
    }
}

fn resolve_development_cargo_with_rustup(
    rustup_home: Option<&Path>,
) -> Result<DevelopmentCargo, RouterError> {
    let override_path = std::env::var_os(DEV_CARGO_OVERRIDE);
    let home = std::env::var_os("HOME");
    let cargo_home = std::env::var_os(DEV_CARGO_HOME);
    resolve_development_cargo_from_with_rustup(
        override_path.as_deref(),
        home.as_deref(),
        cargo_home.as_deref(),
        rustup_home,
    )
}

fn resolve_development_rustup_home() -> Result<Option<PathBuf>, RouterError> {
    resolve_development_rustup_home_from_sources(
        std::env::var_os(DEV_RUSTUP_HOME).as_deref(),
        std::env::var_os("RUSTUP_HOME").as_deref(),
        std::env::var_os("HOME").as_deref(),
    )
}

fn resolve_development_rustup_home_from_sources(
    explicit: Option<&std::ffi::OsStr>,
    ambient: Option<&std::ffi::OsStr>,
    home: Option<&std::ffi::OsStr>,
) -> Result<Option<PathBuf>, RouterError> {
    if let Some(value) = explicit {
        return resolve_development_rustup_home_from(Some(value));
    }
    if let Some(value) = ambient {
        return resolve_development_rustup_home_from(Some(value));
    }
    let Some(home) = home else {
        return Ok(None);
    };
    let candidate = PathBuf::from(home).join(".rustup");
    match fs::symlink_metadata(&candidate) {
        Ok(_) => resolve_development_rustup_home_from(Some(candidate.as_os_str())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(RouterError::operation("trusted_tool_unavailable")),
    }
}

fn resolve_development_rustup_home_from(
    value: Option<&std::ffi::OsStr>,
) -> Result<Option<PathBuf>, RouterError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let path = PathBuf::from(value);
    if !safe_absolute(&path) {
        return Err(RouterError::policy("trusted_tool_invalid"));
    }
    validate_development_rustup_parent_chain(&path)?;
    let metadata = fs::symlink_metadata(&path)
        .map_err(|_| RouterError::operation("trusted_tool_unavailable"))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(RouterError::policy("trusted_tool_invalid"));
    }
    Ok(Some(path))
}

fn resolve_development_cargo_from_with_rustup(
    override_path: Option<&std::ffi::OsStr>,
    home: Option<&std::ffi::OsStr>,
    cargo_home: Option<&std::ffi::OsStr>,
    rustup_home: Option<&Path>,
) -> Result<DevelopmentCargo, RouterError> {
    if let Some(value) = override_path {
        let candidate = PathBuf::from(value);
        let resolved = match validate_development_tool(&candidate) {
            Ok(path) => path,
            Err(_) => validate_group_writable_rustup_shim(&candidate)?
                .ok_or_else(|| RouterError::policy("trusted_tool_invalid"))?,
        };
        let (path, bound_file, bound_rustup_bin, group_writable_rustup_paths) =
            resolve_rustup_proxy_cargo(&resolved, rustup_home)?;
        let rustup_home_warning = rustup_home
            .map(|path| development_permission_warning(path, rustix::process::geteuid().as_raw()))
            .transpose()?
            .unwrap_or(false);
        return Ok(DevelopmentCargo {
            path,
            bound_file,
            bound_rustup_bin,
            group_writable_rustup_paths: group_writable_rustup_paths || rustup_home_warning,
        });
    }

    let mut candidates = Vec::new();
    if let Some(value) = cargo_home.map(PathBuf::from) {
        let conventional = home
            .map(PathBuf::from)
            .is_some_and(|home| value == home.join(".cargo"));
        candidates.push((value.join("bin/cargo"), conventional));
    }
    if let Some(value) = home.map(PathBuf::from) {
        candidates.push((value.join(".cargo/bin/cargo"), true));
    }
    candidates.extend([
        (PathBuf::from("/usr/bin/cargo"), false),
        (PathBuf::from("/usr/local/bin/cargo"), false),
    ]);
    for (candidate, allow_group_writable_shim) in candidates {
        let strict_error = match validate_development_tool(&candidate) {
            Ok(resolved) => {
                let (path, bound_file, bound_rustup_bin, group_writable_rustup_paths) =
                    resolve_rustup_proxy_cargo(&resolved, rustup_home)?;
                let rustup_home_warning = rustup_home
                    .map(|path| {
                        development_permission_warning(path, rustix::process::geteuid().as_raw())
                    })
                    .transpose()?
                    .unwrap_or(false);
                return Ok(DevelopmentCargo {
                    path,
                    bound_file,
                    bound_rustup_bin,
                    group_writable_rustup_paths: group_writable_rustup_paths || rustup_home_warning,
                });
            }
            Err(error) if error.code == "trusted_tool_unavailable" => continue,
            Err(error) => error,
        };
        if !allow_group_writable_shim {
            return Err(strict_error);
        }
        match validate_group_writable_rustup_shim(&candidate) {
            Ok(Some(resolved)) => {
                let (path, bound_file, bound_rustup_bin, _) =
                    resolve_rustup_proxy_cargo(&resolved, rustup_home)?;
                return Ok(DevelopmentCargo {
                    path,
                    bound_file,
                    bound_rustup_bin,
                    group_writable_rustup_paths: true,
                });
            }
            Ok(None) => return Err(strict_error),
            Err(error) if error.code == "trusted_tool_unavailable" => continue,
            Err(error) => return Err(error),
        }
    }
    Err(RouterError::operation("trusted_tool_unavailable"))
}

/// Accept the conventional `~/.cargo/bin/cargo -> rustup` shim only when the
/// sole strict-policy violation is group write on its current-user-owned
/// `.cargo` and/or `bin` ancestors. The resolved rustup binary remains a
/// current-user-owned, non-symlink, non-writable executable at the exact
/// sibling path.
fn validate_group_writable_rustup_shim(path: &Path) -> Result<Option<PathBuf>, RouterError> {
    if !safe_absolute(path)
        || path.file_name() != Some(std::ffi::OsStr::new("cargo"))
        || path.parent().and_then(Path::file_name) != Some(std::ffi::OsStr::new("bin"))
        || path
            .parent()
            .and_then(Path::parent)
            .and_then(Path::file_name)
            != Some(std::ffi::OsStr::new(".cargo"))
    {
        return Ok(None);
    }
    let uid = rustix::process::geteuid().as_raw();
    validate_rustup_shim_parent_chain(path, uid)?;
    let link_metadata = fs::symlink_metadata(path)
        .map_err(|_| RouterError::operation("trusted_tool_unavailable"))?;
    if !link_metadata.file_type().is_symlink() {
        return Err(RouterError::policy("trusted_tool_invalid"));
    }
    let resolved =
        fs::canonicalize(path).map_err(|_| RouterError::operation("trusted_tool_unavailable"))?;
    let expected_rustup = path
        .parent()
        .ok_or_else(|| RouterError::policy("trusted_tool_invalid"))?
        .join("rustup");
    if resolved != expected_rustup {
        return Err(RouterError::policy("trusted_tool_invalid"));
    }
    let metadata = fs::symlink_metadata(&resolved)
        .map_err(|_| RouterError::operation("trusted_tool_unavailable"))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(RouterError::policy("trusted_tool_invalid"));
    }
    Ok(Some(resolved))
}

fn validate_rustup_shim_parent_chain(path: &Path, _uid: u32) -> Result<(), RouterError> {
    let mut current = PathBuf::from("/");
    let parent = path
        .parent()
        .ok_or_else(|| RouterError::policy("trusted_tool_invalid"))?;
    for component in parent.components() {
        if let Component::Normal(name) = component {
            current.push(name);
            let metadata = fs::symlink_metadata(&current)
                .map_err(|_| RouterError::operation("trusted_tool_unavailable"))?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err(RouterError::policy("trusted_tool_invalid"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
fn trusted_owner_and_mode(owner: u32, mode: u32, uid: u32, allow_group_write: bool) -> bool {
    (owner == 0 || owner == uid)
        && mode & 0o002 == 0
        && (mode & 0o020 == 0 || (allow_group_write && owner == uid))
}

fn development_permission_warning(path: &Path, uid: u32) -> Result<bool, RouterError> {
    let mut warning = false;
    let mut current = PathBuf::from("/");
    for component in path.components() {
        if let Component::Normal(name) = component {
            current.push(name);
            let metadata = fs::symlink_metadata(&current)
                .map_err(|_| RouterError::operation("trusted_tool_unavailable"))?;
            if metadata.file_type().is_symlink() {
                return Err(RouterError::policy("trusted_tool_invalid"));
            }
            warning |= metadata.uid() != 0 && metadata.uid() != uid;
            warning |= metadata.mode() & 0o022 != 0;
        }
    }
    Ok(warning)
}

fn resolve_rustup_proxy_cargo(
    resolved: &Path,
    rustup_home: Option<&Path>,
) -> Result<(PathBuf, Option<File>, Option<File>, bool), RouterError> {
    if resolved.file_name() != Some(std::ffi::OsStr::new("rustup")) {
        return Ok((resolved.to_path_buf(), None, None, false));
    }
    let Some(rustup_home) = rustup_home else {
        return Err(RouterError::operation("trusted_tool_unavailable"));
    };
    let (settings, settings_group_writable) = read_development_rustup_settings(rustup_home)
        .map_err(|_| RouterError::policy("trusted_tool_invalid"))?;
    let settings =
        std::str::from_utf8(&settings).map_err(|_| RouterError::policy("trusted_tool_invalid"))?;
    let toolchain = settings
        .lines()
        .find_map(|line| line.trim().strip_prefix("default_toolchain = "))
        .map(str::trim)
        .and_then(|value| {
            value
                .strip_prefix('"')
                .and_then(|value| value.strip_suffix('"'))
        })
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 256
                && Path::new(value)
                    .components()
                    .all(|component| matches!(component, Component::Normal(_)))
        })
        .ok_or_else(|| RouterError::policy("trusted_tool_invalid"))?;
    let cargo = rustup_home
        .join("toolchains")
        .join(toolchain)
        .join("bin/cargo");
    let (cargo, file, bin, toolchain_group_writable) =
        validate_development_rustup_tool(&cargo, rustup_home)?;
    Ok((
        cargo,
        Some(file),
        Some(bin),
        settings_group_writable || toolchain_group_writable,
    ))
}

fn read_development_rustup_settings(rustup_home: &Path) -> Result<(Vec<u8>, bool), RouterError> {
    let uid = rustix::process::geteuid().as_raw();
    let directory_metadata = fs::symlink_metadata(rustup_home)
        .map_err(|_| RouterError::operation("trusted_tool_unavailable"))?;
    let directory =
        File::open(rustup_home).map_err(|_| RouterError::operation("trusted_tool_unavailable"))?;
    let opened_directory = directory
        .metadata()
        .map_err(|_| RouterError::operation("trusted_tool_unavailable"))?;
    if !opened_directory.is_dir()
        || directory_metadata.dev() != opened_directory.dev()
        || directory_metadata.ino() != opened_directory.ino()
    {
        return Err(RouterError::policy("trusted_tool_invalid"));
    }
    let file = openat(
        &directory,
        "settings.toml",
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(|_| RouterError::operation("trusted_tool_unavailable"))?;
    let metadata = file
        .metadata()
        .map_err(|_| RouterError::operation("trusted_tool_unavailable"))?;
    if !metadata.is_file() || metadata.len() > 64 * 1024 {
        return Err(RouterError::policy("trusted_tool_invalid"));
    }
    let group_writable = metadata.uid() != uid
        || metadata.mode() & 0o022 != 0
        || development_permission_warning(rustup_home, uid)?;
    let mut bytes = Vec::new();
    file.take(64 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| RouterError::policy("trusted_tool_invalid"))?;
    if bytes.len() > 64 * 1024 {
        return Err(RouterError::policy("trusted_tool_invalid"));
    }
    Ok((bytes, group_writable))
}

fn validate_development_rustup_tool(
    path: &Path,
    rustup_home: &Path,
) -> Result<(PathBuf, File, File, bool), RouterError> {
    if !safe_absolute(path) || !safe_absolute(rustup_home) {
        return Err(RouterError::policy("trusted_tool_invalid"));
    }
    let rustup_root = fs::canonicalize(rustup_home)
        .map_err(|_| RouterError::operation("trusted_tool_unavailable"))?;
    let resolved = path.to_path_buf();
    if resolved != path || !resolved.starts_with(rustup_root.join("toolchains")) {
        return Err(RouterError::policy("trusted_tool_invalid"));
    }
    let relative = resolved
        .strip_prefix(&rustup_root)
        .map_err(|_| RouterError::policy("trusted_tool_invalid"))?;
    let components = relative.components().collect::<Vec<_>>();
    if components.len() != 4
        || components[0].as_os_str() != "toolchains"
        || !matches!(components[1], Component::Normal(_))
        || components[2].as_os_str() != "bin"
        || !matches!(components[3].as_os_str().to_str(), Some("cargo" | "rustc"))
    {
        return Err(RouterError::policy("trusted_tool_invalid"));
    }
    let uid = rustix::process::geteuid().as_raw();
    let root_link_metadata = fs::symlink_metadata(&rustup_root)
        .map_err(|_| RouterError::operation("trusted_tool_unavailable"))?;
    let mut directory =
        File::open(&rustup_root).map_err(|_| RouterError::operation("trusted_tool_unavailable"))?;
    let root_metadata = directory
        .metadata()
        .map_err(|_| RouterError::operation("trusted_tool_unavailable"))?;
    if !root_metadata.is_dir()
        || root_link_metadata.file_type().is_symlink()
        || root_link_metadata.dev() != root_metadata.dev()
        || root_link_metadata.ino() != root_metadata.ino()
    {
        return Err(RouterError::policy("trusted_tool_invalid"));
    }
    let mut group_writable = development_permission_warning(&rustup_root, uid)?;
    for component in [&components[0], &components[1], &components[2]] {
        let next = openat(
            &directory,
            component.as_os_str(),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map(File::from)
        .map_err(classify_trusted_tool_open_error)?;
        let metadata = next
            .metadata()
            .map_err(|_| RouterError::operation("trusted_tool_unavailable"))?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(RouterError::policy("trusted_tool_invalid"));
        }
        group_writable |= metadata.uid() != 0 && metadata.uid() != uid;
        group_writable |= metadata.mode() & 0o022 != 0;
        directory = next;
    }
    let file = openat(
        &directory,
        components[3].as_os_str(),
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(classify_trusted_tool_open_error)?;
    let metadata = file
        .metadata()
        .map_err(|_| RouterError::operation("trusted_tool_unavailable"))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(RouterError::policy("trusted_tool_invalid"));
    }
    group_writable |= metadata.uid() != uid || metadata.mode() & 0o022 != 0;
    Ok((resolved, file, directory, group_writable))
}

fn classify_trusted_tool_open_error(error: rustix::io::Errno) -> RouterError {
    if error == rustix::io::Errno::NOENT {
        RouterError::operation("trusted_tool_unavailable")
    } else {
        RouterError::policy("trusted_tool_invalid")
    }
}

fn apply_development_rustup_home(command: &mut Command, rustup_home: Option<&Path>) {
    if let Some(rustup_home) = rustup_home {
        command.env("RUSTUP_HOME", rustup_home);
    }
}

#[cfg(test)]
fn resolve_development_rustc(
    cargo: &Path,
    rustup_home: Option<&Path>,
    direct_override: bool,
) -> Result<Option<PathBuf>, RouterError> {
    let cargo = DevelopmentCargo {
        path: cargo.to_path_buf(),
        bound_file: None,
        bound_rustup_bin: None,
        group_writable_rustup_paths: false,
    };
    Ok(
        resolve_development_rustc_bound(&cargo, rustup_home, direct_override)?
            .map(|tool| tool.path),
    )
}

fn resolve_development_rustc_bound(
    cargo: &DevelopmentCargo,
    rustup_home: Option<&Path>,
    direct_override: bool,
) -> Result<Option<DevelopmentTool>, RouterError> {
    if rustup_home.is_none() && !direct_override {
        return Ok(None);
    }
    if let Some(bin) = cargo.bound_rustup_bin.as_ref() {
        if cargo.bound_file.is_none()
            || cargo.path.file_name() != Some(std::ffi::OsStr::new("cargo"))
            || cargo.path.parent().and_then(Path::file_name) != Some(std::ffi::OsStr::new("bin"))
        {
            return Err(RouterError::policy("trusted_tool_invalid"));
        }
        let path = cargo
            .path
            .parent()
            .ok_or_else(|| RouterError::policy("trusted_tool_invalid"))?
            .join("rustc");
        let file = open_validated_development_rustup_tool_at(bin, "rustc")?;
        return Ok(Some(DevelopmentTool {
            path,
            bound_file: Some(file),
        }));
    }
    let resolved = fs::canonicalize(&cargo.path)
        .map_err(|_| RouterError::operation("trusted_tool_unavailable"))?;
    if resolved.file_name() != Some(std::ffi::OsStr::new("cargo")) {
        return Ok(None);
    }
    if let Some(rustup_home) = rustup_home {
        let rustup_root = fs::canonicalize(rustup_home)
            .map_err(|_| RouterError::operation("trusted_tool_unavailable"))?;
        if !resolved.starts_with(rustup_root.join("toolchains")) {
            return Err(RouterError::policy("trusted_tool_invalid"));
        }
    }
    let bin = resolved
        .parent()
        .filter(|path| path.file_name() == Some(std::ffi::OsStr::new("bin")))
        .ok_or_else(|| RouterError::policy("trusted_tool_invalid"))?;
    let rustc = bin.join("rustc");
    if let Some(rustup_home) = rustup_home {
        let (path, file, _, _) = validate_development_rustup_tool(&rustc, rustup_home)?;
        Ok(Some(DevelopmentTool {
            path,
            bound_file: Some(file),
        }))
    } else {
        let path = validate_development_tool(&rustc)?;
        Ok(Some(DevelopmentTool {
            path,
            bound_file: None,
        }))
    }
}

fn open_validated_development_rustup_tool_at(bin: &File, name: &str) -> Result<File, RouterError> {
    let file = openat(
        bin,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(classify_trusted_tool_open_error)?;
    let metadata = file
        .metadata()
        .map_err(|_| RouterError::operation("trusted_tool_unavailable"))?;
    let uid = rustix::process::geteuid().as_raw();
    if !metadata.is_file() || metadata.uid() != uid || metadata.mode() & 0o022 != 0 {
        return Err(RouterError::policy("trusted_tool_invalid"));
    }
    Ok(file)
}

fn descriptor_path(file: &File) -> PathBuf {
    PathBuf::from(format!("/proc/self/fd/{}", file.as_raw_fd()))
}

fn make_descriptor_inheritable(file: &File) -> Result<FdFlags, RouterError> {
    let flags =
        fcntl_getfd(file).map_err(|_| RouterError::operation("trusted_tool_unavailable"))?;
    fcntl_setfd(file, flags - FdFlags::CLOEXEC)
        .map_err(|_| RouterError::operation("trusted_tool_unavailable"))?;
    Ok(flags)
}

fn restore_descriptor_flags(file: &File, flags: FdFlags) -> Result<(), RouterError> {
    fcntl_setfd(file, flags).map_err(|_| RouterError::operation("trusted_tool_unavailable"))
}

fn make_toolchain_descriptors_inheritable(
    cargo: Option<&File>,
    rustc: Option<&File>,
) -> Result<(Option<FdFlags>, Option<FdFlags>), RouterError> {
    let cargo_flags = cargo.map(make_descriptor_inheritable).transpose()?;
    match rustc.map(make_descriptor_inheritable).transpose() {
        Ok(rustc_flags) => Ok((cargo_flags, rustc_flags)),
        Err(error) => {
            if let (Some(file), Some(flags)) = (cargo, cargo_flags) {
                let _ = restore_descriptor_flags(file, flags);
            }
            Err(error)
        }
    }
}

fn restore_toolchain_descriptor_flags(
    cargo: Option<&File>,
    rustc: Option<&File>,
    flags: (Option<FdFlags>, Option<FdFlags>),
) -> Result<(), RouterError> {
    let rustc_result = match (rustc, flags.1) {
        (Some(file), Some(flags)) => restore_descriptor_flags(file, flags),
        _ => Ok(()),
    };
    let cargo_result = match (cargo, flags.0) {
        (Some(file), Some(flags)) => restore_descriptor_flags(file, flags),
        _ => Ok(()),
    };
    rustc_result.and(cargo_result)
}

fn apply_development_toolchain_environment(
    command: &mut Command,
    rustup_home: Option<&Path>,
    rustc: Option<&Path>,
    cc: Option<&Path>,
    ar: Option<&Path>,
    ld: Option<&Path>,
) {
    apply_development_rustup_home(command, rustup_home);
    if let Some(rustc) = rustc {
        command.env("RUSTC", rustc);
    }
    if let Some(cc) = cc {
        command.env("CC", cc);
        command.env("RUSTC_LINKER", cc);
        command.env(
            format!(
                "CARGO_TARGET_{}_LINKER",
                target().to_ascii_uppercase().replace('-', "_")
            ),
            cc,
        );
    }
    if let Some(ar) = ar {
        command.env("AR", ar);
    }
    if let Some(ld) = ld {
        command.env("LD", ld);
    }
}

fn validate_development_tool(path: &Path) -> Result<PathBuf, RouterError> {
    if !safe_absolute(path) {
        return Err(RouterError::policy("trusted_tool_invalid"));
    }
    let uid = rustix::process::geteuid().as_raw();
    validate_development_parent_chain(path, uid)?;
    let link_metadata = fs::symlink_metadata(path)
        .map_err(|_| RouterError::operation("trusted_tool_unavailable"))?;
    if link_metadata.file_type().is_symlink()
        && link_metadata.uid() != 0
        && link_metadata.uid() != uid
    {
        return Err(RouterError::policy("trusted_tool_invalid"));
    }
    let resolved =
        fs::canonicalize(path).map_err(|_| RouterError::operation("trusted_tool_unavailable"))?;
    let metadata = fs::symlink_metadata(&resolved)
        .map_err(|_| RouterError::operation("trusted_tool_unavailable"))?;
    if !metadata.is_file()
        || (metadata.uid() != 0 && metadata.uid() != uid)
        || metadata.mode() & 0o022 != 0
    {
        return Err(RouterError::policy("trusted_tool_invalid"));
    }
    let mut parent = resolved.parent();
    while let Some(directory) = parent {
        let metadata = fs::symlink_metadata(directory)
            .map_err(|_| RouterError::operation("trusted_tool_unavailable"))?;
        if !metadata.is_dir()
            || (metadata.uid() != 0 && metadata.uid() != uid)
            || metadata.mode() & 0o022 != 0
        {
            return Err(RouterError::policy("trusted_tool_invalid"));
        }
        if directory == Path::new("/") {
            break;
        }
        parent = directory.parent();
    }
    Ok(resolved)
}

fn validate_development_parent_chain(path: &Path, uid: u32) -> Result<(), RouterError> {
    let mut current = PathBuf::from("/");
    let parent = path
        .parent()
        .ok_or_else(|| RouterError::policy("trusted_tool_invalid"))?;
    for component in parent.components() {
        if let Component::Normal(name) = component {
            current.push(name);
            let metadata = fs::symlink_metadata(&current)
                .map_err(|_| RouterError::operation("trusted_tool_unavailable"))?;
            if !metadata.is_dir()
                || metadata.file_type().is_symlink()
                || (metadata.uid() != 0 && metadata.uid() != uid)
                || metadata.mode() & 0o022 != 0
            {
                return Err(RouterError::policy("trusted_tool_invalid"));
            }
        }
    }
    Ok(())
}

fn validate_development_rustup_parent_chain(path: &Path) -> Result<(), RouterError> {
    let mut current = PathBuf::from("/");
    let parent = path
        .parent()
        .ok_or_else(|| RouterError::policy("trusted_tool_invalid"))?;
    for component in parent.components() {
        if let Component::Normal(name) = component {
            current.push(name);
            let metadata = fs::symlink_metadata(&current)
                .map_err(|_| RouterError::operation("trusted_tool_unavailable"))?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err(RouterError::policy("trusted_tool_invalid"));
            }
        }
    }
    Ok(())
}

fn resolve_redirects(initial: &str) -> Result<String, RouterError> {
    let mut current = initial.to_owned();
    for _ in 0..=MAX_REDIRECTS {
        if !allowed_https_url(&current) {
            return Err(RouterError::policy("redirect_rejected"));
        }
        validate_tool(CURL)?;
        let output = Command::new(CURL)
            .env_clear()
            .env("LANG", "C.UTF-8")
            .args([
                "--disable",
                "--silent",
                "--show-error",
                "--proto",
                "=https",
                "--tlsv1.2",
                "--max-redirs",
                "0",
                "--connect-timeout",
                "5",
                "--max-time",
                "15",
                "--head",
                "--dump-header",
                "-",
                "--output",
                "/dev/null",
                &current,
            ])
            .output()
            .map_err(|_| RouterError::operation("transfer_unavailable"))?;
        if output.stdout.len() > 32 * 1024 {
            return Err(RouterError::policy("redirect_rejected"));
        }
        let headers = std::str::from_utf8(&output.stdout)
            .map_err(|_| RouterError::policy("redirect_rejected"))?;
        match parse_redirect(headers)? {
            Some(next) => current = next,
            None if output.status.success() => return Ok(current),
            None => return Err(RouterError::operation("transfer_failed")),
        }
    }
    Err(RouterError::policy("redirect_rejected"))
}

fn parse_redirect(headers: &str) -> Result<Option<String>, RouterError> {
    let block = headers
        .split("\r\n\r\n")
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .pop()
        .ok_or_else(|| RouterError::policy("redirect_rejected"))?;
    let mut lines = block.split("\r\n");
    let status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| RouterError::policy("redirect_rejected"))?;
    let locations: Vec<_> = lines
        .filter_map(|line| line.split_once(':'))
        .filter(|(name, _)| name.eq_ignore_ascii_case("location"))
        .map(|(_, value)| value.trim().to_owned())
        .collect();
    if (300..400).contains(&status) {
        if locations.len() != 1 || !allowed_https_url(&locations[0]) {
            return Err(RouterError::policy("redirect_rejected"));
        }
        Ok(locations.into_iter().next())
    } else if (200..300).contains(&status) && locations.is_empty() {
        Ok(None)
    } else {
        Err(RouterError::policy("redirect_rejected"))
    }
}

fn allowed_https_url(value: &str) -> bool {
    let without_scheme = match value.strip_prefix("https://") {
        Some(value) => value,
        None => return false,
    };
    let host = without_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
    matches!(host, "github.com" | "release-assets.githubusercontent.com")
        && !without_scheme.contains('@')
}

fn valid_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn valid_release(value: &str) -> bool {
    value.len() <= 32 && version_parts(value).is_ok()
}

fn version_parts(value: &str) -> Result<(u64, u64, u64), RouterError> {
    let mut parts = value
        .strip_prefix('v')
        .ok_or_else(|| RouterError::policy("release_invalid"))?
        .split('.');
    let version = (
        parts.next().and_then(|part| part.parse().ok()),
        parts.next().and_then(|part| part.parse().ok()),
        parts.next().and_then(|part| part.parse().ok()),
    );
    let no_leading_zero = |part: &str| part == "0" || !part.starts_with('0');
    let textual: Vec<_> = value.trim_start_matches('v').split('.').collect();
    match version {
        (Some(major), Some(minor), Some(patch))
            if parts.next().is_none()
                && textual.len() == 3
                && textual.iter().all(|part| no_leading_zero(part)) =>
        {
            Ok((major, minor, patch))
        }
        _ => Err(RouterError::policy("release_invalid")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preflight_classifies_toolchain_rejection_with_recovery() {
        let error = RouterError::policy("trusted_tool_invalid");
        assert_eq!(error.classification(), "host_limitation");
        assert_eq!(
            error.remediation(),
            Some("repair_toolchain_permissions_or_set_private_ASB_DEV_RUSTUP_HOME")
        );
        let failure = RouterError::operation("candidate_execution_failed");
        assert_eq!(failure.classification(), "product_failure");
        assert_eq!(failure.remediation(), None);
    }

    #[test]
    fn tui_directory_preparation_notices_only_missing_human_destinations() {
        let scratch = Scratch::new("tui-directory-notice");
        let destination = scratch.0.join("state");
        let mut progress = Vec::new();
        prepare_private_directory_with_notice(
            &destination,
            "ASB TUI lifecycle state",
            &mut progress,
            true,
        )
        .expect("TUI state directory");
        assert_eq!(
            String::from_utf8(progress).unwrap(),
            format!(
                "[WAIT] ASB will create directory {} for ASB TUI lifecycle state.\n",
                destination.display()
            )
        );
        let mut reused_progress = Vec::new();
        prepare_private_directory_with_notice(
            &destination,
            "ASB TUI lifecycle state",
            &mut reused_progress,
            true,
        )
        .expect("existing TUI state directory");
        assert!(reused_progress.is_empty());
    }
    use asb_control::{
        BROKER_PACKET_BYTES, BrokerPacket, CONTROL_DYNAMIC_PROVIDER_CATALOG_V1, CONTROL_FANOUT_V1,
        ControlBackend, ControlCall, ControlClient, ControlResult, HandoffStatus, RequestDeadline,
        SUPPORTED_CONTROL_VERSIONS_LEGACY,
    };
    use rustix::net::{RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, ReturnFlags, recvmsg};
    #[cfg(feature = "cross-repo-qualification")]
    use rustix::pty::{OpenptFlags, grantpt, ioctl_tiocgptpeer, openpt, ptsname, unlockpt};
    #[cfg(feature = "cross-repo-qualification")]
    use rustix::termios::{Winsize, tcsetwinsize};
    use std::collections::VecDeque;
    use std::io::IoSliceMut;
    use std::mem::MaybeUninit;
    use std::os::unix::fs::DirBuilderExt;
    use std::os::unix::fs::symlink;
    use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    static NONCE: AtomicU64 = AtomicU64::new(0);

    struct RangeSource {
        values: VecDeque<Option<Vec<u8>>>,
        offsets: Vec<u64>,
    }

    impl Source for RangeSource {
        fn document(&mut self, _: &str, _: usize) -> Result<Vec<u8>, RouterError> {
            Err(RouterError::operation("unexpected_document"))
        }

        fn range(&mut self, _: &str, offset: u64, _: usize) -> Result<Vec<u8>, RouterError> {
            self.offsets.push(offset);
            self.values
                .pop_front()
                .flatten()
                .ok_or_else(|| RouterError::operation("synthetic_interruption"))
        }
    }

    struct Scratch(PathBuf);

    struct RecordingBackend<B> {
        inner: B,
        calls: Arc<Mutex<Vec<ControlCall>>>,
        results: Arc<Mutex<Vec<asb_control::BoundControlResult>>>,
    }
    impl<B: ControlBackend> ControlBackend for RecordingBackend<B> {
        fn runner_instance_id(&self) -> &str {
            self.inner.runner_instance_id()
        }

        fn oldest_revision(&self) -> asb_control::Revision {
            self.inner.oldest_revision()
        }

        fn latest_revision(&self) -> asb_control::Revision {
            self.inner.latest_revision()
        }

        fn execute(
            &self,
            call: &ControlCall,
            deadline: RequestDeadline,
        ) -> Result<asb_control::BoundControlResult, asb_control::BackendFailure> {
            self.calls
                .lock()
                .expect("bootstrap recorder lock")
                .push(call.clone());
            let result = self.inner.execute(call, deadline)?;
            self.results
                .lock()
                .expect("result recorder lock")
                .push(result.clone());
            Ok(result)
        }
    }

    impl Scratch {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "asb-tui-router-{name}-{}-{}",
                std::process::id(),
                NONCE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&path)
                .unwrap();
            Self(fs::canonicalize(path).unwrap())
        }
    }

    #[test]
    fn development_backend_exposes_typed_read_only_capabilities() {
        let scratch = Scratch::new("development-backend");
        let backend = open_development_backend(scratch.0.clone()).expect("development backend");
        let call = ControlCall::Capabilities;
        let result = backend
            .execute(&call, RequestDeadline::start(100).unwrap())
            .expect("development capabilities");
        assert!(matches!(
            result.result,
            ControlResult::Capabilities(asb_control::Capabilities {
                run_control: false,
                repeat: false,
                events: false,
                ..
            })
        ));
        assert!(
            backend
                .execute(
                    &ControlCall::ConfigurationStatus(asb_control::ConfigurationStatusRequest {
                        runner_instance_id: backend.runner_instance_id().into(),
                    },),
                    RequestDeadline::start(100).unwrap(),
                )
                .is_ok()
        );
        assert!(matches!(
            backend
                .execute(
                    &ControlCall::MeasurementCatalog,
                    RequestDeadline::start(100).unwrap(),
                )
                .expect("development measurement catalog")
                .result,
            ControlResult::MeasurementCatalog(_)
        ));
        assert!(matches!(
            backend
                .execute(
                    &ControlCall::History(asb_control::PageParams {
                        after: None,
                        limit: 10,
                    }),
                    RequestDeadline::start(100).unwrap(),
                )
                .expect("development history")
                .result,
            ControlResult::History(_)
        ));
        assert!(matches!(
            backend.execute(
                &ControlCall::AuthEnroll(asb_control::AuthEnrollParams {
                    provider: "development".into(),
                    endpoint_identity_sha256: "identity".into(),
                    credential_locator_sha256: "locator".into(),
                    idempotency_key: "mutation".into(),
                }),
                RequestDeadline::start(100).unwrap(),
            ),
            Err(asb_control::BackendFailure::CapabilityUnavailable)
        ));
        assert!(matches!(
            backend.execute(
                &ControlCall::ProviderProfileUpsert(asb_control::ProviderProfileUpsertParams {
                    idempotency_key: "mutation".into(),
                    expected_generation: asb_control::Revision(0),
                    runner_instance_id: backend.runner_instance_id().into(),
                    entry: asb_control::ProviderCatalogEntry {
                        provider_id: "development".into(),
                        display_name: "development".into(),
                        availability: asb_control::ProviderAvailability::Unavailable(
                            "development".into(),
                        ),
                        models: Vec::new(),
                        auth_methods: Vec::new(),
                    },
                    credential_reference_sha256: None,
                }),
                RequestDeadline::start(100).unwrap(),
            ),
            Err(asb_control::BackendFailure::CapabilityUnavailable)
        ));
        assert!(matches!(
            backend.execute(
                &ControlCall::ConfigurationApply(asb_control::ConfigurationApplyParams {
                    idempotency_key: "mutation".into(),
                    expected_generation: asb_control::Revision(0),
                    selection: asb_control::ConfigurationSelection {
                        agent_ids: Vec::new(),
                        provider_id: "development".into(),
                        model_id: "development".into(),
                        auth_method: asb_control::ProviderAuthMethod::None,
                        credential_reference_sha256: None,
                    },
                }),
                RequestDeadline::start(100).unwrap(),
            ),
            Err(asb_control::BackendFailure::CapabilityUnavailable)
        ));
        assert!(matches!(
            backend.execute(
                &ControlCall::RecordingCampaignExecute(
                    asb_control::RecordingCampaignExecuteParams {
                        idempotency_key: "mutation".into(),
                        expected_generation: asb_control::Revision(0),
                        runner_instance_id: backend.runner_instance_id().into(),
                        campaign_id: "development".into(),
                    },
                ),
                RequestDeadline::start(100).unwrap(),
            ),
            Err(asb_control::BackendFailure::CapabilityUnavailable)
        ));
        assert!(matches!(
            backend.execute(
                &ControlCall::Analyze {
                    run_ids: Vec::new()
                },
                RequestDeadline::start(100).unwrap(),
            ),
            Err(asb_control::BackendFailure::CapabilityUnavailable)
        ));
        let identity = backend.runner_instance_id().to_owned();
        assert!(
            backend
                .execute(
                    &ControlCall::AgentCatalog(asb_control::AgentCatalogRequest {
                        action: asb_control::AgentCatalogAction::Status,
                        runner_instance_id: identity.clone(),
                        known_generation: None,
                    }),
                    RequestDeadline::start(100).unwrap(),
                )
                .is_ok()
        );
        assert!(matches!(
            backend.execute(
                &ControlCall::AgentCatalog(asb_control::AgentCatalogRequest {
                    action: asb_control::AgentCatalogAction::Refresh,
                    runner_instance_id: identity.clone(),
                    known_generation: None,
                }),
                RequestDeadline::start(100).unwrap(),
            ),
            Err(asb_control::BackendFailure::CapabilityUnavailable)
        ));
        let refreshed = backend
            .execute(
                &ControlCall::ProviderCatalog(asb_control::ProviderCatalogRequest {
                    action: asb_control::ProviderCatalogAction::Refresh,
                    runner_instance_id: identity.clone(),
                    known_generation: None,
                }),
                RequestDeadline::start(100).unwrap(),
            )
            .expect("development provider refresh remains non-blocking");
        assert!(matches!(
            refreshed.result,
            ControlResult::ProviderCatalog(ref catalog) if catalog.refreshed
        ));
        assert!(
            backend
                .execute(
                    &ControlCall::ProviderCatalog(asb_control::ProviderCatalogRequest {
                        action: asb_control::ProviderCatalogAction::Status,
                        runner_instance_id: identity.clone(),
                        known_generation: None,
                    }),
                    RequestDeadline::start(100).unwrap(),
                )
                .is_ok()
        );
        assert!(
            backend
                .execute(
                    &ControlCall::RecordingCampaignStatus(
                        asb_control::RecordingCampaignStatusRequest {
                            runner_instance_id: identity.clone(),
                        },
                    ),
                    RequestDeadline::start(100).unwrap(),
                )
                .is_ok()
        );
        assert!(
            backend
                .execute(
                    &ControlCall::RecordingCampaignProgress(
                        asb_control::RecordingCampaignProgressRequest {
                            runner_instance_id: identity,
                            campaign_id: "development".into(),
                        },
                    ),
                    RequestDeadline::start(100).unwrap(),
                )
                .is_err()
        );
    }

    #[test]
    fn development_backend_preserves_negotiated_catalog_and_mutation_denials() {
        let scratch = Scratch::new("development-versioned-backend");
        let backend = open_development_backend(scratch.0.clone()).expect("development backend");
        let identity = backend.runner_instance_id().to_owned();
        let catalog = |action, runner_instance_id, known_generation| {
            ControlCall::ProviderCatalog(asb_control::ProviderCatalogRequest {
                action,
                runner_instance_id,
                known_generation,
            })
        };

        for action in [
            asb_control::ProviderCatalogAction::Status,
            asb_control::ProviderCatalogAction::Refresh,
        ] {
            let call = catalog(action, identity.clone(), None);
            let legacy = backend
                .execute_versioned(
                    &call,
                    RequestDeadline::start(100).unwrap(),
                    CONTROL_FANOUT_V1,
                )
                .expect("legacy development catalog");
            assert!(matches!(legacy.result, ControlResult::ProviderCatalog(_)));
            legacy
                .validate_for_call_and_version(
                    &call,
                    asb_control::ControlLimits::default(),
                    CONTROL_FANOUT_V1,
                )
                .unwrap();

            let dynamic = backend
                .execute_versioned(
                    &call,
                    RequestDeadline::start(100).unwrap(),
                    CONTROL_DYNAMIC_PROVIDER_CATALOG_V1,
                )
                .expect("v1.15 development catalog");
            assert!(matches!(
                dynamic.result,
                ControlResult::DynamicProviderCatalog(_)
            ));
            dynamic
                .validate_for_call_and_version(
                    &call,
                    asb_control::ControlLimits::default(),
                    CONTROL_DYNAMIC_PROVIDER_CATALOG_V1,
                )
                .unwrap();
        }

        for rejected in [
            catalog(
                asb_control::ProviderCatalogAction::Status,
                "stale-development-runner".into(),
                None,
            ),
            catalog(
                asb_control::ProviderCatalogAction::Status,
                identity.clone(),
                Some(asb_control::Revision(u64::MAX)),
            ),
        ] {
            assert!(matches!(
                backend.execute_versioned(
                    &rejected,
                    RequestDeadline::start(100).unwrap(),
                    CONTROL_DYNAMIC_PROVIDER_CATALOG_V1,
                ),
                Err(asb_control::BackendFailure::StaleIdentity)
            ));
        }

        let mutations = [
            ControlCall::AuthEnroll(asb_control::AuthEnrollParams {
                provider: "development".into(),
                endpoint_identity_sha256: "identity".into(),
                credential_locator_sha256: "locator".into(),
                idempotency_key: "versioned-auth".into(),
            }),
            ControlCall::ConfigurationApply(asb_control::ConfigurationApplyParams {
                idempotency_key: "versioned-configuration".into(),
                expected_generation: asb_control::Revision(0),
                selection: asb_control::ConfigurationSelection {
                    agent_ids: Vec::new(),
                    provider_id: "development".into(),
                    model_id: "development".into(),
                    auth_method: asb_control::ProviderAuthMethod::None,
                    credential_reference_sha256: None,
                },
            }),
            ControlCall::ProviderProfileUpsert(asb_control::ProviderProfileUpsertParams {
                idempotency_key: "versioned-profile".into(),
                expected_generation: asb_control::Revision(0),
                runner_instance_id: identity.clone(),
                entry: asb_control::ProviderCatalogEntry {
                    provider_id: "development-profile".into(),
                    display_name: "Development profile".into(),
                    availability: asb_control::ProviderAvailability::Unavailable(
                        "development".into(),
                    ),
                    models: Vec::new(),
                    auth_methods: Vec::new(),
                },
                credential_reference_sha256: None,
            }),
            ControlCall::Launch(asb_control::LaunchParams {
                idempotency_key: "versioned-launch".into(),
                plan_id: "development".into(),
            }),
            ControlCall::RecordingCampaignExecute(asb_control::RecordingCampaignExecuteParams {
                idempotency_key: "versioned-recording".into(),
                expected_generation: asb_control::Revision(0),
                runner_instance_id: identity,
                campaign_id: "development".into(),
            }),
        ];
        for mutation in mutations {
            for version in [CONTROL_FANOUT_V1, CONTROL_DYNAMIC_PROVIDER_CATALOG_V1] {
                assert!(matches!(
                    backend.execute_versioned(
                        &mutation,
                        RequestDeadline::start(100).unwrap(),
                        version,
                    ),
                    Err(asb_control::BackendFailure::CapabilityUnavailable)
                ));
            }
        }
    }

    #[test]
    fn development_bridge_serves_v115_dynamic_catalog_over_private_socket() {
        let scratch = Scratch::new("development-v115-bridge");
        let control_dir = scratch.0.join("control");
        let provisioning_dir = scratch.0.join("provisioning");
        prepare_private_directory(&control_dir).unwrap();
        prepare_private_directory(&provisioning_dir).unwrap();
        let control_path = control_dir.join("control.sock");
        let provisioning_path = provisioning_dir.join("provision.sock");
        let backend = open_development_backend(scratch.0.clone()).unwrap();
        let server = ProvisionedControlServer::bind(
            &control_path,
            &provisioning_path,
            ControlLimits::default(),
            backend,
        )
        .unwrap();
        let worker = thread::spawn(move || server.serve_connections(1, 0));
        let mut client = ControlClient::connect_with_versions(
            &control_path,
            ControlLimits::default(),
            [CONTROL_DYNAMIC_PROVIDER_CATALOG_V1],
        )
        .unwrap();
        let response = client
            .call(
                ControlCall::ProviderCatalog(asb_control::ProviderCatalogRequest {
                    action: asb_control::ProviderCatalogAction::Refresh,
                    runner_instance_id: client.negotiated().runner_instance_id.clone(),
                    known_generation: None,
                }),
                client.negotiated().limits.max_timeout_ms,
            )
            .unwrap()
            .into_result()
            .unwrap();
        assert!(matches!(
            response,
            asb_control::ControlSuccess::Operation(operation)
                if matches!(operation.result, ControlResult::DynamicProviderCatalog(_))
        ));
        drop(client);
        worker.join().unwrap().unwrap();
        assert!(!control_path.exists());
        assert!(!provisioning_path.exists());
    }

    #[test]
    fn development_bridge_serves_real_backend_over_private_control_socket() {
        let scratch = Scratch::new("development-bridge");
        let control_dir = scratch.0.join("control");
        let provisioning_dir = scratch.0.join("provisioning");
        prepare_private_directory(&control_dir).unwrap();
        prepare_private_directory(&provisioning_dir).unwrap();
        let control_path = control_dir.join("control.sock");
        let provisioning_path = provisioning_dir.join("provision.sock");
        let backend = open_development_backend(scratch.0.clone()).unwrap();
        let expected_runner = backend.runner_instance_id().to_owned();
        let server = ProvisionedControlServer::bind(
            &control_path,
            &provisioning_path,
            ControlLimits::default(),
            backend,
        )
        .unwrap();
        let (worker_done, worker_result) = std::sync::mpsc::channel();
        let worker = thread::spawn(move || {
            let result = server.serve_connections(1, 0);
            let _ = worker_done.send(result);
        });

        let mut client = ControlClient::connect_with_versions(
            &control_path,
            ControlLimits::default(),
            SUPPORTED_CONTROL_VERSIONS_LEGACY,
        )
        .unwrap();
        assert_eq!(
            client.negotiated().version,
            asb_control::CONTROL_RUNTIME_BOOTSTRAP_V1
        );
        assert_eq!(client.negotiated().runner_instance_id, expected_runner);
        let timeout = client.negotiated().limits.max_timeout_ms;

        let calls = [
            ControlCall::Capabilities,
            ControlCall::MeasurementCatalog,
            ControlCall::History(asb_control::PageParams {
                after: None,
                limit: client.negotiated().limits.max_page_items,
            }),
            ControlCall::ConfigurationStatus(asb_control::ConfigurationStatusRequest {
                runner_instance_id: client.negotiated().runner_instance_id.clone(),
            }),
            ControlCall::ProviderCatalog(asb_control::ProviderCatalogRequest {
                action: asb_control::ProviderCatalogAction::Status,
                runner_instance_id: client.negotiated().runner_instance_id.clone(),
                known_generation: None,
            }),
            ControlCall::RecordingCampaignStatus(asb_control::RecordingCampaignStatusRequest {
                runner_instance_id: client.negotiated().runner_instance_id.clone(),
            }),
            ControlCall::AuthStatus(asb_control::AuthStatusParams {
                provider: "development".into(),
            }),
        ];
        let auth_index = calls.len() - 1;
        for (index, call) in calls.into_iter().enumerate() {
            let response = client.call(call, timeout).unwrap();
            if index == auth_index {
                assert_eq!(response.error().map(|error| error.code), Some(-33_003));
            } else {
                assert!(
                    response.error().is_none(),
                    "unexpected bridge error: {response:?}"
                );
            }
        }
        // A client that presents a stale generation must not receive a
        // current catalog as if its cursor were authoritative.
        let runner_instance_id = client.negotiated().runner_instance_id.clone();
        let stale = client
            .call(
                ControlCall::ProviderCatalog(asb_control::ProviderCatalogRequest {
                    action: asb_control::ProviderCatalogAction::Status,
                    runner_instance_id,
                    known_generation: Some(asb_control::Revision(u64::MAX)),
                }),
                timeout,
            )
            .unwrap();
        assert!(stale.error().is_some(), "stale generation was accepted");
        drop(client);
        assert!(
            worker_result
                .recv_timeout(Duration::from_secs(2))
                .expect("bridge worker shutdown timed out")
                .is_ok()
        );
        worker.join().unwrap();
        assert!(!control_path.exists());
        assert!(!provisioning_path.exists());
    }

    /// Cross-project qualification is deliberately opt-in: CI supplies the
    /// exact asb-tui artifact and its digest rather than allowing this crate
    /// to compile an unpinned sibling checkout. The socket server remains the
    /// real ASB backend, not a protocol fixture.
    #[test]
    fn pinned_asb_tui_binary_fails_closed_without_stable_auth() {
        let (binary, expected, pinned) = match (
            std::env::var("ASB_TUI_BINARY"),
            std::env::var("ASB_TUI_EXPECTED_SHA256"),
        ) {
            (Ok(binary), Ok(expected)) => (PathBuf::from(binary), expected, true),
            (Err(_), Err(_)) => {
                // The ordinary workspace gate has no sibling checkout. Use a
                // deterministic failing executable and a malformed socket
                // client to cover the same stable fail-closed boundary; the
                // verification workflow replaces it with the pinned binary.
                let binary = PathBuf::from("/bin/false");
                let expected = format!("{:x}", Sha256::digest(fs::read(&binary).unwrap()));
                (binary, expected, false)
            }
            _ => panic!("pinned TUI binary and digest must be supplied together"),
        };
        assert!(binary.is_absolute());
        assert!(asb_control::validate_digest(&expected).is_ok());
        let bytes = fs::read(&binary).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected);
        let metadata = fs::symlink_metadata(&binary).unwrap();
        assert!(metadata.file_type().is_file() && metadata.mode() & 0o022 == 0);

        let scratch = Scratch::new("pinned-tui-bridge");
        let control_dir = scratch.0.join("control");
        let provisioning_dir = scratch.0.join("provisioning");
        prepare_private_directory(&control_dir).unwrap();
        prepare_private_directory(&provisioning_dir).unwrap();
        let control_path = control_dir.join("control.sock");
        let provisioning_path = provisioning_dir.join("provision.sock");
        let calls = Arc::new(Mutex::new(Vec::new()));
        let expected_calls = Arc::clone(&calls);
        let backend = open_development_backend(scratch.0.clone()).unwrap();
        let server = ProvisionedControlServer::bind(
            &control_path,
            &provisioning_path,
            ControlLimits::default(),
            RecordingBackend {
                inner: backend,
                calls,
                results: Arc::new(Mutex::new(Vec::new())),
            },
        )
        .unwrap();
        let (worker_done, worker_result) = std::sync::mpsc::channel();
        let worker = thread::spawn(move || {
            let result = server.serve_connections(1, 0);
            let _ = worker_done.send(result);
        });
        if !pinned {
            let mut malformed = std::os::unix::net::UnixStream::connect(&control_path).unwrap();
            malformed.write_all(b"not-a-control-envelope").unwrap();
        }
        let command = format!(
            "{} run --socket {}",
            shell_quote(&binary),
            shell_quote(&control_path)
        );
        let mut child = Command::new("/usr/bin/script")
            .args(["-qefc", &command, "/dev/null"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let feeder = thread::spawn(move || {
            thread::sleep(Duration::from_millis(500));
            let mut input = input;
            input.write_all(b"q")
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("pinned asb-tui did not exit");
            }
            thread::sleep(Duration::from_millis(20));
        };
        feeder.join().unwrap().ok();
        assert!(
            !status.success(),
            "stable socket mode bypassed auth: {status}"
        );
        assert!(
            worker_result
                .recv_timeout(Duration::from_secs(2))
                .expect("pinned bridge worker shutdown timed out")
                .is_ok()
        );
        worker.join().unwrap();
        let calls = expected_calls.lock().unwrap();
        // Stable mode must fail closed at authentication before any
        // bootstrap/status request is admitted by the backend.
        assert!(
            calls.is_empty(),
            "stable route reached bootstrap: {calls:?}"
        );
        assert!(!control_path.exists());
        assert!(!provisioning_path.exists());
    }

    fn shell_quote(path: &Path) -> String {
        let value = path.to_str().expect("UTF-8 test path");
        assert!(value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'_' | b'-')
        }));
        format!("'{value}'")
    }

    #[test]
    fn development_launch_cleans_channel_when_child_exits_before_request() {
        let scratch = Scratch::new("broker-child-exit");
        let error = launch_development_broker(Command::new("/bin/true"), &scratch.0)
            .expect_err("an exited frontend must not receive a handoff");
        assert_eq!(error.code, "development_launch_failed");
        let entries = fs::read_dir(&scratch.0)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(entries.iter().all(|entry| {
            !entry
                .file_name()
                .to_string_lossy()
                .starts_with(".dev-broker-")
        }));
    }

    #[test]
    fn development_launch_rejects_spawn_failure_and_cleans_up() {
        let scratch = Scratch::new("broker-spawn-failure");
        let error = launch_development_broker(
            Command::new("/definitely/missing/asb-development-frontend"),
            &scratch.0,
        )
        .expect_err("a missing frontend must fail before any handoff");
        assert_eq!(error.code, "development_launch_failed");
        let entries = fs::read_dir(&scratch.0)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(entries.iter().all(|entry| {
            !entry
                .file_name()
                .to_string_lossy()
                .starts_with(".dev-broker-")
        }));
    }

    #[test]
    fn development_broker_root_falls_back_for_deep_state_roots() {
        let deep = PathBuf::from(format!("/{}", "nested/".repeat(20)));
        let root = development_broker_root(&deep);
        assert!(root.starts_with(std::env::temp_dir()));
        assert!(root.join("control/control.sock").as_os_str().len() < UNIX_SOCKET_PATH_LIMIT);
        assert!(
            root.join("provisioning/provisioning.sock")
                .as_os_str()
                .len()
                < UNIX_SOCKET_PATH_LIMIT
        );
    }

    #[test]
    fn development_broker_root_stays_under_state_root_when_paths_fit() {
        let state = PathBuf::from("/tmp/asb-state");
        let root = development_broker_root(&state);
        assert!(root.starts_with(state));
    }

    #[test]
    fn qualified_cassette_fixture_reopens_with_catalog_and_offline_campaign() {
        let scratch = Scratch::new("qualified-cassette-fixture");
        let (backend, campaign_id) =
            crate::control::open_qualified_cassette_backend(scratch.0.clone())
                .expect("qualified cassette fixture");
        let request =
            ControlCall::RecordingCampaignStatus(asb_control::RecordingCampaignStatusRequest {
                runner_instance_id: backend.runner_instance_id().to_owned(),
            });
        let result = backend
            .execute(&request, RequestDeadline::start(5_000).unwrap())
            .expect("reopened campaign status");
        assert!(!campaign_id.is_empty());
        let _ = result;
    }

    #[cfg(all(unix, feature = "cross-repo-qualification"))]
    #[ignore = "requires the explicitly pinned asb-tui qualification workflow"]
    #[test]
    fn pinned_tui_inherited_fd_development_launch_uses_pty_and_cleans_up() {
        assert_eq!(std::env::var("ASB_TUI_QUALIFICATION").as_deref(), Ok("1"));
        let binary = PathBuf::from(std::env::var("ASB_TUI_BINARY").unwrap());
        let expected = std::env::var("ASB_TUI_EXPECTED_SHA256").unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(fs::read(&binary).unwrap())),
            expected
        );
        let master =
            openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY | OpenptFlags::CLOEXEC).unwrap();
        grantpt(&master).unwrap();
        unlockpt(&master).unwrap();
        let terminal_path = PathBuf::from(ptsname(&master, Vec::new()).unwrap().to_str().unwrap());
        let slave = ioctl_tiocgptpeer(
            &master,
            OpenptFlags::RDWR | OpenptFlags::NOCTTY | OpenptFlags::CLOEXEC,
        )
        .unwrap();
        tcsetwinsize(
            &master,
            Winsize {
                ws_row: 24,
                ws_col: 80,
                ws_xpixel: 0,
                ws_ypixel: 0,
            },
        )
        .unwrap();
        let scratch = Scratch::new("pinned-inherited-fd");
        let calls = Arc::new(Mutex::new(Vec::new()));
        let results = Arc::new(Mutex::new(Vec::new()));
        let (qualified_backend, campaign_id) =
            crate::control::open_qualified_cassette_backend(scratch.0.clone())
                .expect("qualified cassette backend");
        let backend = RecordingBackend {
            inner: qualified_backend,
            calls: Arc::clone(&calls),
            results: Arc::clone(&results),
        };
        let tui_commit = std::env::var("ASB_TUI_SOURCE_COMMIT").unwrap();
        let tui_tree = std::env::var("ASB_TUI_SOURCE_TREE").unwrap();
        let descriptor = serde_json::json!({
            "schema_version": 1,
            "profile": "development",
            "development_only": true,
            "operation": "launch",
            "protocol_minor": DEV_BROKER_PROTOCOL_MINOR,
            "asb_source_commit": ASB_SOURCE_COMMIT,
            "asb_source_tree": ASB_SOURCE_TREE,
            "tui_source_commit": tui_commit,
            "tui_source_tree": tui_tree,
        });
        let mut command = Command::new(binary);
        command
            .env_clear()
            .args(["run", "--broker", "--development"])
            .env(
                DEV_BROKER_DESCRIPTOR_ENV,
                serde_json::to_string(&descriptor).unwrap(),
            )
            .env(DEV_BROKER_ASB_COMMIT_ENV, ASB_SOURCE_COMMIT)
            .env(DEV_BROKER_ASB_TREE_ENV, ASB_SOURCE_TREE)
            .env(
                DEV_BROKER_TUI_COMMIT_ENV,
                descriptor["tui_source_commit"].as_str().unwrap(),
            )
            .env(
                DEV_BROKER_TUI_TREE_ENV,
                descriptor["tui_source_tree"].as_str().unwrap(),
            )
            .env("TERM", "xterm-256color")
            .env("ASB_TUI_DEVELOPMENT_TERMINAL_PATH", &terminal_path)
            .stdout(Stdio::from(slave.try_clone().unwrap()))
            .stderr(Stdio::inherit());
        let tools = DevelopmentLaunchTools::resolve().unwrap();
        tools.apply(&mut command);
        let inherited = tools.make_descriptors_inheritable().unwrap();
        let root = scratch.0.clone();
        let (launch_done, launch_result) = std::sync::mpsc::channel();
        thread::spawn(move || {
            let result = launch_development_broker_with_backend(command, &root, backend);
            let _ = launch_done.send(result);
        });
        let mut master = File::from(master);
        let deadline = Instant::now() + Duration::from_secs(8);
        while Instant::now() < deadline
            && !calls
                .lock()
                .expect("bootstrap recorder lock")
                .iter()
                .any(|call| matches!(call, ControlCall::RecordingCampaignStatus(_)))
        {
            thread::sleep(Duration::from_millis(20));
        }
        thread::sleep(Duration::from_millis(250));
        let observed = calls.lock().expect("bootstrap recorder lock").clone();
        assert!(
            observed.len() >= 4,
            "typed bootstrap did not complete before deadline: {observed:?}"
        );
        assert!(matches!(observed.first(), Some(ControlCall::Capabilities)));
        let measurement = observed
            .iter()
            .position(|call| matches!(call, ControlCall::MeasurementCatalog))
            .expect("measurement catalog bootstrap");
        let history = observed
            .iter()
            .position(|call| matches!(call, ControlCall::History(_)))
            .expect("history bootstrap");
        assert!(measurement < history, "bootstrap order: {observed:?}");
        // Exercise the real TUI action path: enter Run Control, activate the
        // seeded offline campaign, select its digest-only cassette, dispatch
        // strict replay, then quit. The ASB recorder sees the typed calls.
        let feeder_calls = Arc::clone(&calls);
        let feeder = thread::spawn(move || {
            // The benchmark picker starts empty.  Select all advertised
            // measures explicitly before entering Run Control; otherwise the
            // frontend correctly rejects an offline campaign with no scope.
            thread::sleep(Duration::from_millis(500));
            master.write_all(b"2").unwrap();
            thread::sleep(Duration::from_millis(500));
            master.write_all(b"a").unwrap();
            // Bootstrap already proved the control route is alive.  Allow the
            // renderer to settle after the explicit selection before changing
            // routes; subsequent transitions are gated on typed calls.
            thread::sleep(Duration::from_millis(750));
            master.write_all(b"s").unwrap();
            thread::sleep(Duration::from_millis(750));
            let mut offline_default_requested = false;
            for _ in 0..8 {
                master.write_all(b"o").unwrap();
                let deadline = Instant::now() + Duration::from_secs(2);
                while Instant::now() < deadline
                    && !feeder_calls
                        .lock()
                        .expect("feeder recorder lock")
                        .iter()
                        .any(|call| matches!(call, ControlCall::RecordingCampaignOfflineDefault(_)))
                {
                    thread::sleep(Duration::from_millis(20));
                }
                if feeder_calls
                    .lock()
                    .expect("feeder recorder lock")
                    .iter()
                    .any(|call| matches!(call, ControlCall::RecordingCampaignOfflineDefault(_)))
                {
                    offline_default_requested = true;
                    break;
                }
                thread::sleep(Duration::from_millis(250));
            }
            assert!(
                offline_default_requested,
                "Run Control did not dispatch offline default after readiness"
            );
            // The recording-cassette route is rendered asynchronously after
            // the offline default response; gate the route key on its typed
            // catalog request rather than sending ahead of the response.
            let mut catalog_requested = false;
            for _ in 0..6 {
                master.write_all(b"]").unwrap();
                let deadline = Instant::now() + Duration::from_secs(2);
                while Instant::now() < deadline
                    && !feeder_calls
                        .lock()
                        .expect("feeder recorder lock")
                        .iter()
                        .any(|call| matches!(call, ControlCall::RecordingCassetteCatalog(_)))
                {
                    thread::sleep(Duration::from_millis(20));
                }
                if feeder_calls
                    .lock()
                    .expect("feeder recorder lock")
                    .iter()
                    .any(|call| matches!(call, ControlCall::RecordingCassetteCatalog(_)))
                {
                    catalog_requested = true;
                    break;
                }
                thread::sleep(Duration::from_millis(250));
            }
            assert!(
                catalog_requested,
                "offline cassette catalog was not requested after activation"
            );
            master.write_all(b"J").unwrap();
            // Replay is a bounded control round trip; repeat quit input so a
            // render/control transition cannot swallow the single byte.
            for _ in 0..5 {
                thread::sleep(Duration::from_millis(300));
                master.write_all(b"q").unwrap();
            }
        });
        let result = match launch_result.recv_timeout(Duration::from_secs(20)) {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => {
                let calls = calls.lock().expect("call recorder lock");
                panic!("development launch: {error:?}; calls={calls:?}");
            }
            Err(error) => {
                let calls = calls.lock().expect("call recorder lock");
                panic!("development launch completion timed out: {error}; calls={calls:?}");
            }
        };
        tools.restore_descriptor_flags(inherited).unwrap();
        feeder.join().unwrap();
        assert!(result.code().is_some(), "development child did not exit");
        let observed = calls.lock().expect("cassette recorder lock").clone();
        assert!(
            observed.iter().any(|call| {
                matches!(call, ControlCall::RecordingCassetteCatalog(request)
                if request.campaign_id == campaign_id)
            }),
            "TUI did not fetch the seeded cassette catalog: {observed:?}"
        );
        let replay = observed
            .iter()
            .find_map(|call| match call {
                ControlCall::RecordingReplayDispatch(params) => Some(params),
                _ => None,
            })
            .expect("TUI did not dispatch selected replay");
        assert_eq!(replay.campaign_id, campaign_id);
        assert!(asb_control::validate_digest(&replay.cassette_sha256).is_ok());
        assert!(
            !observed
                .iter()
                .any(|call| matches!(call, ControlCall::RecordingCampaignExecute(_))),
            "offline replay attempted a provider capture"
        );
        let replay_result = results
            .lock()
            .expect("cassette result recorder lock")
            .iter()
            .find_map(|bound| match &bound.result {
                ControlResult::RecordingReplayDispatch(value)
                    if value.campaign_id == campaign_id =>
                {
                    Some(value.clone())
                }
                _ => None,
            })
            .expect("TUI replay did not receive a typed result");
        assert!(
            replay_result.offline_only,
            "replay must deny provider egress"
        );
        assert!(fs::read_dir(&scratch.0).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".dev-broker-")
        }));
    }

    #[test]
    fn development_launch_kills_descendants_holding_inherited_channel() {
        let scratch = Scratch::new("broker-descendant");
        let pid_file = scratch.0.join("descendant.pid");
        let script = format!(
            "sleep 60 & echo $! > {}; exit 0",
            pid_file.to_string_lossy()
        );
        let mut command = Command::new("/bin/sh");
        command.args(["-c", &script]);
        let error = launch_development_broker(command, &scratch.0)
            .expect_err("an exited frontend must not receive a handoff");
        assert_eq!(error.code, "development_launch_failed");
        let pid: i32 = fs::read_to_string(pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline && Path::new(&format!("/proc/{pid}")).exists() {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(!Path::new(&format!("/proc/{pid}")).exists());
    }

    #[test]
    fn development_broker_foregrounds_interactive_child_and_restores_terminal() {
        const ROLE: &str = "ASB_AR1727_FOREGROUND_TEST_ROLE";
        const MARKER: &str = "ASB_AR1727_FOREGROUND_TEST_MARKER";
        const TEST_NAME: &str =
            "tui::tests::development_broker_foregrounds_interactive_child_and_restores_terminal";

        match std::env::var(ROLE).as_deref() {
            Ok("frontend") => {
                let terminal = File::open("/dev/tty").expect("frontend controlling terminal");
                let group = getpgrp();
                let deadline = Instant::now() + Duration::from_secs(2);
                while tcgetpgrp(&terminal).ok() != Some(group) {
                    assert!(
                        Instant::now() < deadline,
                        "development frontend never became the terminal foreground group"
                    );
                    thread::sleep(Duration::from_millis(10));
                }
                let broker = rustix::io::dup(io::stdin()).expect("duplicate inherited broker");
                let mut payload = [0_u8; BROKER_PACKET_BYTES];
                let mut iov = [IoSliceMut::new(&mut payload)];
                let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
                let mut ancillary = RecvAncillaryBuffer::new(&mut space);
                let received = recvmsg(
                    &broker,
                    &mut iov,
                    &mut ancillary,
                    RecvFlags::CMSG_CLOEXEC | RecvFlags::TRUNC,
                )
                .expect("receive parent-first broker generation");
                assert_eq!(received.bytes, BROKER_PACKET_BYTES);
                assert!(
                    !received
                        .flags
                        .intersects(ReturnFlags::TRUNC | ReturnFlags::CTRUNC)
                );
                let mut descriptors = Vec::new();
                for message in ancillary.drain() {
                    match message {
                        RecvAncillaryMessage::ScmRights(rights) => descriptors.extend(rights),
                        _ => panic!("unexpected broker ancillary message"),
                    }
                }
                assert_eq!(
                    descriptors.len(),
                    1,
                    "broker must transfer one control stream"
                );
                let packet = BrokerPacket::decode(&payload).expect("valid broker packet");
                assert_eq!(packet.status, HandoffStatus::Success);
                fs::write(
                    std::env::var_os(MARKER).expect("foreground marker path"),
                    format!("{}\n", group.as_raw_nonzero()),
                )
                .expect("record foreground ownership");
                // The parent-first producer completed admission before this
                // point. Close the deterministic control stream after proving
                // the transfer; the server must finish and the launcher must
                // still restore terminal ownership.
                drop(descriptors);
                return;
            }
            Ok("launcher") => {
                assert!(
                    io::stdin().is_terminal(),
                    "launcher needs a controlling PTY"
                );
                let terminal = File::open("/dev/tty").expect("launcher controlling terminal");
                let original_group = tcgetpgrp(&terminal).expect("original foreground group");
                let original_mask = SigSet::thread_get_mask().expect("original signal mask");
                assert_eq!(original_group, getpgrp(), "launcher must start foreground");

                let scratch = Scratch::new("broker-foreground");
                let marker = scratch.0.join("frontend-foreground");
                let mut command = Command::new(std::env::current_exe().expect("test executable"));
                command
                    .args(["--exact", TEST_NAME, "--nocapture"])
                    .env(ROLE, "frontend")
                    .env(MARKER, &marker)
                    .stdout(Stdio::inherit())
                    .stderr(Stdio::inherit());
                let backend =
                    open_development_backend(scratch.0.clone()).expect("development backend");
                let status = launch_development_broker_with_backend(command, &scratch.0, backend)
                    .expect("development broker launch");
                assert!(status.success(), "development frontend failed: {status}");
                assert!(
                    marker.exists(),
                    "frontend did not observe foreground ownership"
                );
                assert_eq!(
                    tcgetpgrp(&terminal).expect("restored foreground group"),
                    original_group,
                    "development broker did not restore the caller foreground group"
                );
                assert_eq!(
                    SigSet::thread_get_mask().expect("restored signal mask"),
                    original_mask,
                    "development broker did not restore the caller signal mask"
                );
                return;
            }
            Ok(other) => panic!("unexpected foreground test role: {other}"),
            Err(_) => {}
        }

        let executable = std::env::current_exe().expect("test executable");
        let command = format!(
            "{} --exact {TEST_NAME} --nocapture",
            shell_quote(&executable)
        );
        let status = Command::new("/usr/bin/script")
            .args(["-qefc", &command, "/dev/null"])
            .env(ROLE, "launcher")
            .status()
            .expect("controlling PTY harness");
        assert!(
            status.success(),
            "controlling PTY regression failed: {status}"
        );
    }

    #[test]
    fn development_broker_rejects_changed_foreground_owner_and_reaps_child() {
        const ROLE: &str = "ASB_AR1727_CHANGED_FOREGROUND_TEST_ROLE";
        const TEST_NAME: &str =
            "tui::tests::development_broker_rejects_changed_foreground_owner_and_reaps_child";

        match std::env::var(ROLE).as_deref() {
            Ok("launcher") => {
                assert!(
                    io::stdin().is_terminal(),
                    "launcher needs a controlling PTY"
                );
                let terminal = File::open("/dev/tty").expect("launcher controlling terminal");
                let original_group = tcgetpgrp(&terminal).expect("original foreground group");
                let original_mask = SigSet::thread_get_mask().expect("original signal mask");
                assert_eq!(original_group, getpgrp(), "launcher must start foreground");

                let mut contender = Command::new("/bin/sleep");
                contender
                    .arg("30")
                    .process_group(0)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null());
                let mut contender = contender.spawn().expect("foreground contender");
                let contender_group = Pid::from_child(&contender);
                let launched_group = Arc::new(AtomicI32::new(0));
                let observed_group = Arc::clone(&launched_group);
                DEVELOPMENT_FOREGROUND_BEFORE_ASSIGN.with(|hook| {
                    assert!(hook.borrow().is_none(), "foreground hook already installed");
                    *hook.borrow_mut() = Some(Box::new(move |terminal, child_group| {
                        observed_group.store(child_group.as_raw_nonzero().get(), Ordering::SeqCst);
                        tcsetpgrp(terminal, contender_group)
                            .expect("transfer foreground to intervening owner");
                    }));
                });

                let scratch = Scratch::new("broker-changed-foreground");
                let backend =
                    open_development_backend(scratch.0.clone()).expect("development backend");
                let mut command = Command::new("/bin/sleep");
                command.arg("30");
                let failure = launch_development_broker_with_backend(command, &scratch.0, backend)
                    .expect_err("changed foreground owner must reject launch");
                assert_eq!(failure.code, "development_terminal_unavailable");
                assert_eq!(
                    tcgetpgrp(&terminal).expect("intervening foreground owner"),
                    contender_group,
                    "failed assignment overwrote the intervening foreground owner"
                );

                let launched_pid = launched_group.load(Ordering::SeqCst);
                assert!(launched_pid > 0, "launch hook did not observe child group");
                let deadline = Instant::now() + Duration::from_secs(2);
                while Instant::now() < deadline
                    && Path::new(&format!("/proc/{launched_pid}")).exists()
                {
                    thread::sleep(Duration::from_millis(10));
                }
                assert!(
                    !Path::new(&format!("/proc/{launched_pid}")).exists(),
                    "rejected development child was not reaped"
                );
                assert!(
                    !development_broker_root(&scratch.0).exists(),
                    "rejected broker root was not removed"
                );

                // Test-harness cleanup happens only after proving the launcher
                // preserved the intervening owner. Restore the harness group
                // under the same scoped signal discipline used in production.
                let blocked = SigttouBlock::acquire().expect("block SIGTTOU for harness cleanup");
                tcsetpgrp(&terminal, original_group).expect("restore harness foreground group");
                blocked.restore().expect("restore harness signal mask");
                let _ = kill_process_group(contender_group, Signal::KILL);
                contender.wait().expect("reap foreground contender");
                assert_eq!(
                    tcgetpgrp(&terminal).expect("restored harness foreground group"),
                    original_group
                );
                assert_eq!(
                    SigSet::thread_get_mask().expect("restored signal mask"),
                    original_mask,
                    "rejected handoff did not restore the caller signal mask"
                );
                return;
            }
            Ok(other) => panic!("unexpected changed-foreground test role: {other}"),
            Err(_) => {}
        }

        let executable = std::env::current_exe().expect("test executable");
        let command = format!(
            "{} --exact {TEST_NAME} --nocapture",
            shell_quote(&executable)
        );
        let status = Command::new("/usr/bin/script")
            .args(["-qefc", &command, "/dev/null"])
            .env(ROLE, "launcher")
            .status()
            .expect("controlling PTY harness");
        assert!(
            status.success(),
            "changed-foreground PTY regression failed: {status}"
        );
    }

    #[test]
    fn development_terminal_restore_preserves_intervening_foreground_owner() {
        const ROLE: &str = "ASB_AR1727_RESTORE_OWNER_TEST_ROLE";
        const TEST_NAME: &str =
            "tui::tests::development_terminal_restore_preserves_intervening_foreground_owner";

        match std::env::var(ROLE).as_deref() {
            Ok("launcher") => {
                let terminal = File::open("/dev/tty").expect("launcher controlling terminal");
                let original_group = tcgetpgrp(&terminal).expect("original foreground group");
                let original_mask = SigSet::thread_get_mask().expect("original signal mask");
                assert_eq!(original_group, getpgrp(), "launcher must start foreground");
                let mut assigned = Command::new("/bin/sleep")
                    .arg("30")
                    .process_group(0)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .expect("assigned child");
                let assigned_group = Pid::from_child(&assigned);
                let mut contender = Command::new("/bin/sleep")
                    .arg("30")
                    .process_group(0)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .expect("foreground contender");
                let contender_group = Pid::from_child(&contender);

                let mut ownership = DevelopmentForegroundTerminal::capture()
                    .expect("capture foreground terminal")
                    .expect("controlling terminal");
                ownership
                    .assign(assigned_group)
                    .expect("assign child foreground group");
                assert_eq!(
                    tcgetpgrp(&terminal).expect("assigned foreground group"),
                    assigned_group
                );
                let blocked = SigttouBlock::acquire().expect("block SIGTTOU for owner change");
                tcsetpgrp(&terminal, contender_group).expect("install intervening owner");
                blocked
                    .restore()
                    .expect("restore signal mask after owner change");
                assert_eq!(
                    ownership.restore(),
                    Err(DevelopmentTerminalError::ForegroundChanged)
                );
                assert_eq!(
                    tcgetpgrp(&terminal).expect("preserved intervening owner"),
                    contender_group,
                    "restore overwrote the intervening foreground owner"
                );
                assert_eq!(
                    SigSet::thread_get_mask().expect("signal mask after rejected restore"),
                    original_mask
                );

                let blocked = SigttouBlock::acquire().expect("block SIGTTOU for harness cleanup");
                tcsetpgrp(&terminal, original_group).expect("restore harness foreground group");
                blocked.restore().expect("restore harness signal mask");
                let _ = kill_process_group(assigned_group, Signal::KILL);
                let _ = kill_process_group(contender_group, Signal::KILL);
                assigned.wait().expect("reap assigned child");
                contender.wait().expect("reap foreground contender");
                assert_eq!(
                    tcgetpgrp(&terminal).expect("restored harness foreground group"),
                    original_group
                );
                return;
            }
            Ok(other) => panic!("unexpected restore-owner test role: {other}"),
            Err(_) => {}
        }

        let executable = std::env::current_exe().expect("test executable");
        let command = format!(
            "{} --exact {TEST_NAME} --nocapture",
            shell_quote(&executable)
        );
        let status = Command::new("/usr/bin/script")
            .args(["-qefc", &command, "/dev/null"])
            .env(ROLE, "launcher")
            .status()
            .expect("controlling PTY harness");
        assert!(
            status.success(),
            "restore-owner PTY regression failed: {status}"
        );
    }

    #[test]
    fn development_long_session_process_group_cleanup_is_bounded() {
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", "sleep 60"])
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child = command.spawn().expect("long-session child");
        thread::sleep(Duration::from_millis(25));
        let started = Instant::now();
        terminate_development_child(&mut child);
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(child.try_wait().expect("reaped child").is_some());
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn selected(release: &str, manifest_sha256: &str) -> ChannelRelease {
        ChannelRelease {
            release: release.into(),
            target: target().into(),
            asb_version: "0.1.0".into(),
            protocol_version: 1,
            manifest_url: format!(
                "https://github.com/martin-beck/asb-tui/releases/download/{release}/manifest.json"
            ),
            manifest_signature_url: format!(
                "https://github.com/martin-beck/asb-tui/releases/download/{release}/manifest.json.sig"
            ),
            manifest_sha256: manifest_sha256.into(),
        }
    }

    fn policy_manifest() -> BundleManifest {
        BundleManifest {
            schema_version: 1,
            release: "v1.0.0".into(),
            source_commit: "a".repeat(40),
            source_tree: "b".repeat(40),
            issued_unix: 1,
            expires_unix: 2,
            compatibility: BundleCompatibility {
                bundle: "asb-tui-v1-linux-x86_64".into(),
                architecture: "x86_64".into(),
                asb_version: "0.1.0".into(),
                protocol_version: 1,
                coordinator_version: "v0.3.5".into(),
                coordinator_commit: "510817b93feb80dde13e5a6c61d657954fae2346".into(),
                quality_version: "v0.23.0".into(),
                quality_commit: "8a9f056b7fc7926b9465a0f7a09225d4da1c572a".into(),
            },
            components: Vec::new(),
            artifacts: Vec::new(),
        }
    }

    fn policy_documents(package: serde_json::Value) -> BTreeMap<String, Vec<u8>> {
        let dependency = serde_json::json!({
            "name":package["name"],"versionInfo":package["version"],
            "licenseDeclared":package["license"]
        });
        let packages = serde_json::json!([
            {"name":"asb-tui","version":"1","license":"MIT"},
            {"name":"agent-workflow-coordinator","version":"1","license":"MIT"},
            {"name":"agent-workflow-quality","version":"1","license":"MIT"},
            package
        ]);
        BTreeMap::from([
            (
                "licenses".into(),
                serde_json::to_vec(&serde_json::json!({
                    "schema_version":1,"release":"v1.0.0","packages":packages
                }))
                .unwrap(),
            ),
            (
                "sbom".into(),
                serde_json::to_vec(&serde_json::json!({
                    "spdxVersion":"SPDX-2.3","name":"asb-tui-v1.0.0",
                    "packages":[
                        {"name":"asb-tui","versionInfo":"1","licenseDeclared":"MIT"},
                        {"name":"agent-workflow-coordinator","versionInfo":"1","licenseDeclared":"MIT"},
                        {"name":"agent-workflow-quality","versionInfo":"1","licenseDeclared":"MIT"},
                        dependency
                    ]
                }))
                .unwrap(),
            ),
            (
                "provenance".into(),
                serde_json::to_vec(&serde_json::json!({
                    "schema_version":1,"release":"v1.0.0","source_commit":"a".repeat(40),
                    "source_tree":"b".repeat(40),"builder":"github-actions","reproducible":true
                }))
                .unwrap(),
            ),
        ])
    }

    #[test]
    fn parser_is_closed_and_typed() {
        assert_eq!(parse(&[]).unwrap().0, Operation::Launch);
        assert_eq!(parse(&["status".into()]).unwrap().0, Operation::Status);
        assert_eq!(
            parse(&["live-provider".into()]).unwrap().0,
            Operation::LiveProvider
        );
        assert_eq!(
            parse(&["dynamic-catalog".into()]).unwrap().0,
            Operation::DynamicCatalog
        );
        assert_eq!(parse(&["install".into()]).unwrap().1.channel, Channel::Dev);
        for name in ["dev", "stable", "nightly", "experimental"] {
            let parsed = parse(&["install".into(), "--channel".into(), name.into()]);
            assert_eq!(parsed.unwrap().1.channel.name(), name);
        }
        for operation in ["launch", "status", "doctor", "remove", "help"] {
            let parsed = parse(&[operation.into(), "--channel".into(), "stable".into()]).unwrap();
            assert_eq!(parsed.1.channel, Channel::Stable);
            assert!(parsed.1.channel_explicit);
        }
        assert_eq!(parse(&["--help".into()]).unwrap().0, Operation::Help);
        assert!(parse(&["install".into(), "--channel".into()]).is_err());
        assert!(
            parse(&[
                "install".into(),
                "--channel".into(),
                "stable".into(),
                "--channel".into(),
                "dev".into()
            ])
            .is_err()
        );
        assert!(
            parse(&[
                "install".into(),
                "--offline".into(),
                "--dry-run".into(),
                "--launch".into(),
            ])
            .is_err()
        );
        assert!(parse(&["install".into(), "--offline".into(), "--offline".into()]).is_err());
        for flag in ["--offline", "--dry-run", "--launch"] {
            let (_, options) = parse(&["install".into(), flag.into()]).unwrap();
            assert_eq!(
                (options.offline, options.dry_run, options.launch),
                match flag {
                    "--offline" => (true, false, false),
                    "--dry-run" => (false, true, false),
                    _ => (false, false, true),
                }
            );
        }
        assert!(parse(&["render".into()]).is_err());
    }

    #[test]
    fn development_identity_unknown_is_typed_and_fail_closed() {
        let error = development_source_identity_from("", "").unwrap_err();
        assert_eq!(error.code, "dev_source_identity_unknown");
        assert!(
            development_source_identity_from(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            )
            .is_ok()
        );
    }

    #[cfg(unix)]
    #[test]
    fn development_cargo_discovery_accepts_private_user_toolchain() {
        let root = PathBuf::from(std::env::var_os("HOME").unwrap())
            .join(".cache")
            .join(format!("asb-ar1567-cargo-{}", std::process::id()));
        prepare_private_directory(&root).unwrap();
        let cargo = root.join("toolchain/bin/cargo");
        prepare_private_directory(cargo.parent().unwrap()).unwrap();
        fs::write(&cargo, b"fixture cargo").unwrap();
        fs::set_permissions(&cargo, fs::Permissions::from_mode(0o700)).unwrap();
        let resolved = resolve_development_cargo_from_with_rustup(
            None,
            Some(root.join("home").as_os_str()),
            Some(cargo.parent().unwrap().parent().unwrap().as_os_str()),
            None,
        )
        .unwrap();
        assert_eq!(resolved.path, fs::canonicalize(cargo).unwrap());
        assert!(!resolved.group_writable_rustup_paths);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn development_cargo_discovery_rejects_override_and_path_widening() {
        let root = PathBuf::from(std::env::var_os("HOME").unwrap())
            .join(".cache")
            .join(format!("asb-ar1567-cargo-reject-{}", std::process::id()));
        prepare_private_directory(&root).unwrap();
        let cargo = root.join("cargo");
        fs::write(&cargo, b"fixture cargo").unwrap();
        fs::set_permissions(&cargo, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            resolve_development_cargo_from_with_rustup(
                Some(std::ffi::OsStr::new("relative/cargo")),
                None,
                None,
                None,
            )
            .unwrap_err()
            .code,
            "trusted_tool_invalid"
        );
        fs::set_permissions(&cargo, fs::Permissions::from_mode(0o777)).unwrap();
        assert_eq!(
            validate_development_tool(&cargo).unwrap_err().code,
            "trusted_tool_invalid"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn development_rustup_home_requires_private_absolute_root() {
        let root = PathBuf::from(std::env::var_os("HOME").unwrap())
            .join(".cache")
            .join(format!("asb-ar1634-rustup-{}", std::process::id()));
        prepare_private_directory(&root).unwrap();
        assert_eq!(
            resolve_development_rustup_home_from(Some(root.as_os_str())).unwrap(),
            Some(root.clone())
        );
        assert_eq!(
            resolve_development_rustup_home_from(Some(std::ffi::OsStr::new("relative")))
                .unwrap_err()
                .code,
            "trusted_tool_invalid"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn development_rustup_home_uses_ordered_validated_sources() {
        let root = PathBuf::from(std::env::var_os("HOME").unwrap())
            .join(".cache")
            .join(format!("asb-ar1634-rustup-sources-{}", std::process::id()));
        prepare_private_directory(&root).unwrap();
        let fallback_home = root.join("home");
        prepare_private_directory(&fallback_home).unwrap();
        let fallback = fallback_home.join(".rustup");
        prepare_private_directory(&fallback).unwrap();
        let ambient = root.join("ambient-rustup");
        prepare_private_directory(&ambient).unwrap();
        let explicit = root.join("explicit-rustup");
        prepare_private_directory(&explicit).unwrap();

        assert_eq!(
            resolve_development_rustup_home_from_sources(
                None,
                None,
                Some(fallback_home.as_os_str()),
            )
            .unwrap(),
            Some(fallback.clone())
        );
        assert_eq!(
            resolve_development_rustup_home_from_sources(
                None,
                Some(ambient.as_os_str()),
                Some(fallback_home.as_os_str()),
            )
            .unwrap(),
            Some(ambient.clone())
        );
        assert_eq!(
            resolve_development_rustup_home_from_sources(
                Some(explicit.as_os_str()),
                Some(ambient.as_os_str()),
                Some(fallback_home.as_os_str()),
            )
            .unwrap(),
            Some(explicit.clone())
        );
        assert_eq!(
            resolve_development_rustup_home_from_sources(
                None,
                None,
                Some(root.join("missing-home").as_os_str()),
            )
            .unwrap(),
            None
        );
        assert_eq!(
            resolve_development_rustup_home_from_sources(
                None,
                Some(root.join("missing-ambient").as_os_str()),
                Some(fallback_home.as_os_str()),
            )
            .unwrap_err()
            .code,
            "trusted_tool_unavailable"
        );

        fs::set_permissions(&fallback, fs::Permissions::from_mode(0o777)).unwrap();
        assert_eq!(
            resolve_development_rustup_home_from_sources(
                None,
                None,
                Some(fallback_home.as_os_str()),
            )
            .unwrap(),
            Some(fallback)
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn development_rustup_proxy_resolves_selected_toolchain_cargo() {
        let root = PathBuf::from(std::env::var_os("HOME").unwrap())
            .join(".cache")
            .join(format!("asb-ar1634-rustup-proxy-{}", std::process::id()));
        prepare_private_directory(&root).unwrap();
        fs::write(
            root.join("settings.toml"),
            b"version = \"12\"\ndefault_toolchain = \"fixture-toolchain\"\n",
        )
        .unwrap();
        fs::set_permissions(
            root.join("settings.toml"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        let proxy = root.join("bin/rustup");
        prepare_private_directory(proxy.parent().unwrap()).unwrap();
        fs::write(&proxy, b"rustup proxy").unwrap();
        fs::set_permissions(&proxy, fs::Permissions::from_mode(0o700)).unwrap();
        let cargo = root.join("toolchains/fixture-toolchain/bin/cargo");
        prepare_private_directory(cargo.parent().unwrap()).unwrap();
        fs::write(&cargo, b"toolchain cargo").unwrap();
        fs::set_permissions(&cargo, fs::Permissions::from_mode(0o700)).unwrap();
        let rustc = cargo.parent().unwrap().join("rustc");
        fs::write(&rustc, b"toolchain rustc").unwrap();
        fs::set_permissions(&rustc, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            resolve_development_cargo_from_with_rustup(Some(proxy.as_os_str()), None, None, None,)
                .unwrap_err()
                .code,
            "trusted_tool_unavailable"
        );
        let resolved = resolve_development_cargo_from_with_rustup(
            Some(proxy.as_os_str()),
            None,
            None,
            Some(&root),
        )
        .unwrap();
        assert_eq!(resolved.path, cargo);
        assert!(!resolved.group_writable_rustup_paths);
        assert_eq!(
            resolve_development_rustc(&resolved.path, Some(&root), false).unwrap(),
            Some(cargo.parent().unwrap().join("rustc"))
        );
        assert_eq!(
            resolve_development_rustc(&resolved.path, None, false).unwrap(),
            None
        );
        assert_eq!(
            resolve_development_rustc(&resolved.path, None, true).unwrap(),
            Some(cargo.parent().unwrap().join("rustc"))
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn development_cargo_accepts_user_owned_group_writable_rustup_shim_with_warning() {
        let root = PathBuf::from(std::env::var_os("HOME").unwrap())
            .join(".cache")
            .join(format!("asb-ar1726-rustup-shim-{}", std::process::id()));
        prepare_private_directory(&root).unwrap();
        let home = root.join("home");
        let cargo_root = home.join(".cargo");
        let shim_bin = cargo_root.join("bin");
        prepare_private_directory(&home).unwrap();
        prepare_private_directory(&cargo_root).unwrap();
        prepare_private_directory(&shim_bin).unwrap();
        fs::set_permissions(&cargo_root, fs::Permissions::from_mode(0o775)).unwrap();
        fs::set_permissions(&shim_bin, fs::Permissions::from_mode(0o775)).unwrap();

        let proxy = shim_bin.join("rustup");
        fs::write(&proxy, b"rustup proxy").unwrap();
        fs::set_permissions(&proxy, fs::Permissions::from_mode(0o700)).unwrap();
        std::os::unix::fs::symlink(&proxy, shim_bin.join("cargo")).unwrap();

        let rustup_home = root.join("rustup");
        prepare_private_directory(&rustup_home).unwrap();
        fs::write(
            rustup_home.join("settings.toml"),
            b"version = \"12\"\ndefault_toolchain = \"fixture-toolchain\"\n",
        )
        .unwrap();
        fs::set_permissions(
            rustup_home.join("settings.toml"),
            fs::Permissions::from_mode(0o664),
        )
        .unwrap();
        let toolchain_bin = rustup_home.join("toolchains/fixture-toolchain/bin");
        prepare_private_directory(&toolchain_bin).unwrap();
        fs::set_permissions(
            rustup_home.join("toolchains/fixture-toolchain"),
            fs::Permissions::from_mode(0o775),
        )
        .unwrap();
        fs::set_permissions(&toolchain_bin, fs::Permissions::from_mode(0o775)).unwrap();
        let cargo = toolchain_bin.join("cargo");
        let rustc = toolchain_bin.join("rustc");
        fs::write(&cargo, b"toolchain cargo").unwrap();
        fs::write(&rustc, b"toolchain rustc").unwrap();
        fs::set_permissions(&cargo, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&rustc, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            validate_development_tool(&cargo).unwrap_err().code,
            "trusted_tool_invalid"
        );
        assert!(
            resolve_development_cargo_from_with_rustup(
                Some(shim_bin.join("cargo").as_os_str()),
                None,
                None,
                Some(&rustup_home),
            )
            .is_ok()
        );
        let cargo_home_resolved = resolve_development_cargo_from_with_rustup(
            None,
            Some(home.as_os_str()),
            Some(cargo_root.as_os_str()),
            Some(&rustup_home),
        )
        .unwrap();
        assert_eq!(cargo_home_resolved.path, cargo);
        assert!(cargo_home_resolved.group_writable_rustup_paths);

        let resolved = resolve_development_cargo_from_with_rustup(
            None,
            Some(home.as_os_str()),
            None,
            Some(&rustup_home),
        )
        .unwrap();
        assert_eq!(resolved.path, cargo);
        assert!(resolved.group_writable_rustup_paths);
        assert_eq!(
            resolve_development_rustc(&resolved.path, Some(&rustup_home), false).unwrap(),
            Some(rustc)
        );

        let mut response = RouterResponse::result(
            Operation::Preflight,
            true,
            "development_host_ready",
            "denied",
        );
        response.warnings = Some(development_warnings_with_toolchain(
            resolved.group_writable_rustup_paths,
        ));
        let json = serde_json::to_vec(&response).unwrap();
        assert!(String::from_utf8_lossy(&json).contains(GROUP_WRITABLE_RUSTUP_PATH_WARNING));
        let mut human = Vec::new();
        crate::render_human(&["tui".into(), "preflight".into()], &json, &mut human).unwrap();
        let human = String::from_utf8(human).unwrap();
        let compact = human
            .replace("[WARN] ", "")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            compact.contains("group-writable or differently owned paths"),
            "{human}"
        );
        assert!(!human.contains(GROUP_WRITABLE_RUSTUP_PATH_WARNING));
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn development_rustup_tools_execute_validated_objects_after_path_replacement() {
        let root = PathBuf::from(std::env::var_os("HOME").unwrap())
            .join(".cache")
            .join(format!("asb-ar1726-object-binding-{}", std::process::id()));
        prepare_private_directory(&root).unwrap();
        let bin = root.join("toolchains/fixture-toolchain/bin");
        prepare_private_directory(&bin).unwrap();
        fs::set_permissions(
            root.join("toolchains/fixture-toolchain"),
            fs::Permissions::from_mode(0o775),
        )
        .unwrap();
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o775)).unwrap();
        let cargo_path = bin.join("cargo");
        let rustc_path = bin.join("rustc");
        fs::write(&cargo_path, b"#!/bin/sh\nexec \"$RUSTC\"\n").unwrap();
        fs::write(&rustc_path, b"#!/bin/sh\nprintf validated-rustc\n").unwrap();
        fs::set_permissions(&cargo_path, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&rustc_path, fs::Permissions::from_mode(0o700)).unwrap();

        let (cargo_display, cargo_file, _, cargo_warning) =
            validate_development_rustup_tool(&cargo_path, &root).unwrap();
        let (rustc_display, rustc_file, _, rustc_warning) =
            validate_development_rustup_tool(&rustc_path, &root).unwrap();
        assert_eq!(cargo_display, cargo_path);
        assert_eq!(rustc_display, rustc_path);
        assert!(cargo_warning && rustc_warning);
        let cargo_descriptor = descriptor_path(&cargo_file);
        let rustc_descriptor = descriptor_path(&rustc_file);

        fs::rename(&cargo_path, bin.join("validated-cargo")).unwrap();
        fs::rename(&rustc_path, bin.join("validated-rustc")).unwrap();
        fs::write(&cargo_path, b"#!/bin/sh\nprintf substituted-cargo\n").unwrap();
        fs::write(&rustc_path, b"#!/bin/sh\nprintf substituted-rustc\n").unwrap();
        fs::set_permissions(&cargo_path, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&rustc_path, fs::Permissions::from_mode(0o700)).unwrap();

        let inherited =
            make_toolchain_descriptors_inheritable(Some(&cargo_file), Some(&rustc_file)).unwrap();
        let output = Command::new(DEV_SETSID)
            .env_clear()
            .env("RUSTC", &rustc_descriptor)
            .args(["--wait"])
            .arg(&cargo_descriptor)
            .output()
            .unwrap();
        restore_toolchain_descriptor_flags(Some(&cargo_file), Some(&rustc_file), inherited)
            .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"validated-rustc");
        assert!(fcntl_getfd(&cargo_file).unwrap().contains(FdFlags::CLOEXEC));
        assert!(fcntl_getfd(&rustc_file).unwrap().contains(FdFlags::CLOEXEC));

        drop(cargo_file);
        drop(rustc_file);
        assert!(!cargo_descriptor.exists());
        assert!(!rustc_descriptor.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn development_launch_propagates_only_bound_tools_across_hostile_path_replacement() {
        let root = PathBuf::from(std::env::var_os("HOME").unwrap())
            .join(".cache")
            .join(format!("asb-ar1734-launch-tools-{}", std::process::id()));
        prepare_private_directory(&root).unwrap();
        let bin = root.join("toolchains/fixture-toolchain/bin");
        prepare_private_directory(&bin).unwrap();
        fs::set_permissions(
            root.join("toolchains/fixture-toolchain"),
            fs::Permissions::from_mode(0o775),
        )
        .unwrap();
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o775)).unwrap();
        let cargo_path = bin.join("cargo");
        let rustc_path = bin.join("rustc");
        fs::write(&cargo_path, b"#!/bin/sh\nexec \"$ASB_TUI_DEV_RUSTC\"\n").unwrap();
        fs::write(&rustc_path, b"#!/bin/sh\nprintf validated-toolchain\n").unwrap();
        fs::set_permissions(&cargo_path, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&rustc_path, fs::Permissions::from_mode(0o700)).unwrap();
        let (cargo_path, cargo_file, cargo_bin, _) =
            validate_development_rustup_tool(&cargo_path, &root).unwrap();
        let cargo = DevelopmentCargo {
            path: cargo_path,
            bound_file: Some(cargo_file),
            bound_rustup_bin: Some(cargo_bin),
            group_writable_rustup_paths: true,
        };
        let rustc = resolve_development_rustc_bound(&cargo, Some(&root), false)
            .unwrap()
            .unwrap();
        let cargo_descriptor = cargo.execution_path();
        let rustc_descriptor = rustc.execution_path();
        let tools = DevelopmentLaunchTools {
            git: PathBuf::from(DEV_GIT),
            setsid: PathBuf::from(DEV_SETSID),
            cc: PathBuf::from("/usr/bin/cc"),
            ar: PathBuf::from("/usr/bin/ar"),
            ld: PathBuf::from("/usr/bin/ld"),
            rustup_home: Some(root.clone()),
            cargo,
            rustc: Some(rustc),
        };
        let hostile = root.join("hostile-bin");
        prepare_private_directory(&hostile).unwrap();
        for name in ["cargo", "rustc", "git", "setsid", "cc", "ar", "ld"] {
            fs::write(hostile.join(name), b"#!/bin/sh\nprintf hostile\n").unwrap();
            fs::set_permissions(hostile.join(name), fs::Permissions::from_mode(0o700)).unwrap();
        }
        let mut command = Command::new("/bin/sh");
        command
            .env_clear()
            .env("PATH", &hostile)
            .args([
                "-c",
                "test \"$ASB_TUI_DEV_GIT\" = /usr/bin/git && test \"$ASB_TUI_DEV_SETSID\" = /usr/bin/setsid && test \"$ASB_TUI_DEV_CC\" = /usr/bin/cc && test \"$ASB_TUI_DEV_AR\" = /usr/bin/ar && test \"$ASB_TUI_DEV_LD\" = /usr/bin/ld && test \"$ASB_TUI_DEV_RUSTUP_HOME\" != /hostile && exec \"$ASB_TUI_DEV_CARGO\"",
            ]);
        tools.apply(&mut command);
        assert!(
            !command
                .get_envs()
                .any(|(name, value)| name == std::ffi::OsStr::new("PATH") && value.is_some())
        );

        fs::rename(&bin, root.join("retained-bin")).unwrap();
        prepare_private_directory(&bin).unwrap();
        fs::write(bin.join("cargo"), b"#!/bin/sh\nprintf substituted-cargo\n").unwrap();
        fs::write(bin.join("rustc"), b"#!/bin/sh\nprintf substituted-rustc\n").unwrap();
        fs::set_permissions(bin.join("cargo"), fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(bin.join("rustc"), fs::Permissions::from_mode(0o700)).unwrap();

        let inherited = tools.make_descriptors_inheritable().unwrap();
        let output = command.output().unwrap();
        tools.restore_descriptor_flags(inherited).unwrap();
        assert!(
            output.status.success(),
            "status={:?} stdout={} stderr={}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"validated-toolchain");
        assert!(cargo_descriptor.exists());
        assert!(rustc_descriptor.exists());
        assert!(
            fcntl_getfd(tools.cargo.bound_file.as_ref().unwrap())
                .unwrap()
                .contains(FdFlags::CLOEXEC)
        );
        assert!(
            fcntl_getfd(
                tools
                    .rustc
                    .as_ref()
                    .and_then(|tool| tool.bound_file.as_ref())
                    .unwrap()
            )
            .unwrap()
            .contains(FdFlags::CLOEXEC)
        );
        drop(tools);
        assert!(!cargo_descriptor.exists());
        assert!(!rustc_descriptor.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn development_launch_tool_failure_has_stable_human_and_json_contract() {
        let error = RouterError::operation("trusted_tool_unavailable");
        let mut response = RouterResponse::result(Operation::Launch, false, error.code, "denied");
        response.classification = Some(error.classification());
        response.remediation = error.remediation();
        annotate_channel(&mut response, Channel::Dev);
        assert_eq!(
            resolve_development_tool_candidates(
                "ASB_AR1734_MISSING_TOOL_OVERRIDE",
                &["/definitely/missing/asb-ar1734-tool"],
            )
            .unwrap_err()
            .code,
            "trusted_tool_unavailable"
        );
        let json = serde_json::to_vec(&response).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&json).unwrap();
        assert_eq!(value["code"], "trusted_tool_unavailable");
        assert_eq!(value["classification"], "host_limitation");
        assert_eq!(
            value["remediation"],
            "install_a_supported_rust_toolchain_or_set_ASB_DEV_CARGO"
        );
        assert_eq!(value["channel"], "dev");
        assert_eq!(value["development_only"], true);
        assert_eq!(value["network"], "denied");
        let mut human = Vec::new();
        crate::render_human(&["tui".into()], &json, &mut human).unwrap();
        let human = String::from_utf8(human).unwrap();
        let compact = human
            .replace("[WARN] ", "")
            .replace("[ERR ] ", "")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        assert!(compact.contains("trusted tool unavailable"), "{human}");
        assert!(human.contains("Next: asb tui doctor"));
        assert!(!human.contains('/'));
    }

    #[cfg(unix)]
    #[test]
    fn development_rustup_cargo_and_rustc_remain_bound_to_one_toolchain() {
        let root = PathBuf::from(std::env::var_os("HOME").unwrap())
            .join(".cache")
            .join(format!(
                "asb-ar1726-toolchain-pairing-{}",
                std::process::id()
            ));
        prepare_private_directory(&root).unwrap();
        let toolchains = root.join("toolchains");
        let selected = toolchains.join("selected-toolchain");
        let retained = toolchains.join("retained-toolchain");
        let alternate = toolchains.join("alternate-toolchain");
        let selected_bin = selected.join("bin");
        let alternate_bin = alternate.join("bin");
        prepare_private_directory(&selected_bin).unwrap();
        prepare_private_directory(&alternate_bin).unwrap();
        for directory in [
            &toolchains,
            &selected,
            &selected_bin,
            &alternate,
            &alternate_bin,
        ] {
            fs::set_permissions(directory, fs::Permissions::from_mode(0o775)).unwrap();
        }
        let selected_cargo = selected_bin.join("cargo");
        fs::write(&selected_cargo, b"#!/bin/sh\nexec \"$RUSTC\"\n").unwrap();
        fs::set_permissions(&selected_cargo, fs::Permissions::from_mode(0o700)).unwrap();
        let alternate_cargo = alternate_bin.join("cargo");
        let alternate_rustc = alternate_bin.join("rustc");
        fs::write(&alternate_cargo, b"#!/bin/sh\nprintf alternate-cargo\n").unwrap();
        fs::write(&alternate_rustc, b"#!/bin/sh\nprintf alternate-rustc\n").unwrap();
        fs::set_permissions(&alternate_cargo, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&alternate_rustc, fs::Permissions::from_mode(0o700)).unwrap();

        let (cargo_path, cargo_file, cargo_bin, cargo_warning) =
            validate_development_rustup_tool(&selected_cargo, &root).unwrap();
        assert!(cargo_warning);
        let cargo = DevelopmentCargo {
            path: cargo_path,
            bound_file: Some(cargo_file),
            bound_rustup_bin: Some(cargo_bin),
            group_writable_rustup_paths: true,
        };
        let cargo_descriptor = cargo.execution_path();
        assert!(
            fcntl_getfd(cargo.bound_file.as_ref().unwrap())
                .unwrap()
                .contains(FdFlags::CLOEXEC)
        );

        fs::rename(&selected, &retained).unwrap();
        std::os::unix::fs::symlink(&alternate, &selected).unwrap();
        assert_eq!(
            resolve_development_rustc_bound(&cargo, Some(&root), false)
                .unwrap_err()
                .code,
            "trusted_tool_unavailable"
        );
        assert!(cargo_descriptor.exists());
        assert!(
            fcntl_getfd(cargo.bound_file.as_ref().unwrap())
                .unwrap()
                .contains(FdFlags::CLOEXEC)
        );

        let retained_rustc = retained.join("bin/rustc");
        std::os::unix::fs::symlink(&alternate_rustc, &retained_rustc).unwrap();
        assert_eq!(
            resolve_development_rustc_bound(&cargo, Some(&root), false)
                .unwrap_err()
                .code,
            "trusted_tool_invalid"
        );
        fs::remove_file(&retained_rustc).unwrap();
        fs::write(&retained_rustc, b"#!/bin/sh\nprintf selected-rustc\n").unwrap();
        fs::set_permissions(&retained_rustc, fs::Permissions::from_mode(0o700)).unwrap();

        let rustc = resolve_development_rustc_bound(&cargo, Some(&root), false)
            .unwrap()
            .unwrap();
        let rustc_descriptor = rustc.execution_path();
        let inherited = make_toolchain_descriptors_inheritable(
            cargo.bound_file.as_ref(),
            rustc.bound_file.as_ref(),
        )
        .unwrap();
        let output = Command::new(DEV_SETSID)
            .env_clear()
            .env("RUSTC", &rustc_descriptor)
            .args(["--wait"])
            .arg(&cargo_descriptor)
            .output()
            .unwrap();
        restore_toolchain_descriptor_flags(
            cargo.bound_file.as_ref(),
            rustc.bound_file.as_ref(),
            inherited,
        )
        .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"selected-rustc");
        assert!(
            fcntl_getfd(cargo.bound_file.as_ref().unwrap())
                .unwrap()
                .contains(FdFlags::CLOEXEC)
        );
        assert!(
            fcntl_getfd(rustc.bound_file.as_ref().unwrap())
                .unwrap()
                .contains(FdFlags::CLOEXEC)
        );

        let cargo_identity = cargo.bound_file.as_ref().unwrap().metadata().unwrap();
        let rustc_identity = rustc.bound_file.as_ref().unwrap().metadata().unwrap();
        drop(rustc);
        drop(cargo);
        for (descriptor, identity) in [
            (&cargo_descriptor, cargo_identity),
            (&rustc_descriptor, rustc_identity),
        ] {
            if let Ok(reused) = fs::metadata(descriptor) {
                assert_ne!(
                    (reused.dev(), reused.ino()),
                    (identity.dev(), identity.ino())
                );
            }
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn development_group_writable_rustup_shim_keeps_hostile_paths_fail_closed() {
        let root = PathBuf::from(std::env::var_os("HOME").unwrap())
            .join(".cache")
            .join(format!("asb-ar1726-hostile-shim-{}", std::process::id()));
        prepare_private_directory(&root).unwrap();
        let cargo_root = root.join("home/.cargo");
        let shim_bin = cargo_root.join("bin");
        prepare_private_directory(&shim_bin).unwrap();
        fs::set_permissions(&cargo_root, fs::Permissions::from_mode(0o775)).unwrap();
        fs::set_permissions(&shim_bin, fs::Permissions::from_mode(0o775)).unwrap();
        let proxy = shim_bin.join("rustup");
        fs::write(&proxy, b"rustup").unwrap();
        fs::set_permissions(&proxy, fs::Permissions::from_mode(0o700)).unwrap();
        let shim = shim_bin.join("cargo");
        std::os::unix::fs::symlink(&proxy, &shim).unwrap();

        fs::set_permissions(&cargo_root, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(validate_group_writable_rustup_shim(&shim).is_ok());
        fs::set_permissions(&cargo_root, fs::Permissions::from_mode(0o775)).unwrap();

        fs::remove_file(&shim).unwrap();
        let escaped = root.join("escaped-rustup");
        fs::write(&escaped, b"rustup").unwrap();
        fs::set_permissions(&escaped, fs::Permissions::from_mode(0o700)).unwrap();
        std::os::unix::fs::symlink(&escaped, &shim).unwrap();
        assert_eq!(
            validate_group_writable_rustup_shim(&shim).unwrap_err().code,
            "trusted_tool_invalid"
        );

        fs::remove_file(&shim).unwrap();
        fs::write(&shim, b"substituted cargo").unwrap();
        fs::set_permissions(&shim, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            validate_group_writable_rustup_shim(&shim).unwrap_err().code,
            "trusted_tool_invalid"
        );

        assert!(!trusted_owner_and_mode(1234, 0o755, 5678, true));
        assert!(!trusted_owner_and_mode(5678, 0o757, 5678, true));
        assert!(trusted_owner_and_mode(5678, 0o775, 5678, true));
        assert!(!trusted_owner_and_mode(5678, 0o775, 5678, false));
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn development_group_writable_rustup_shim_rejects_symlinked_parent() {
        let root = PathBuf::from(std::env::var_os("HOME").unwrap())
            .join(".cache")
            .join(format!(
                "asb-ar1726-parent-substitution-{}",
                std::process::id()
            ));
        prepare_private_directory(&root).unwrap();
        let home = root.join("home");
        let real_cargo = root.join("real-cargo");
        prepare_private_directory(&home).unwrap();
        prepare_private_directory(&real_cargo.join("bin")).unwrap();
        fs::set_permissions(&real_cargo, fs::Permissions::from_mode(0o775)).unwrap();
        fs::set_permissions(real_cargo.join("bin"), fs::Permissions::from_mode(0o775)).unwrap();
        std::os::unix::fs::symlink(&real_cargo, home.join(".cargo")).unwrap();
        let proxy = root.join("rustup");
        fs::write(&proxy, b"rustup").unwrap();
        fs::set_permissions(&proxy, fs::Permissions::from_mode(0o700)).unwrap();
        let shim = home.join(".cargo/bin/cargo");
        std::os::unix::fs::symlink(&proxy, &shim).unwrap();
        assert_eq!(
            validate_group_writable_rustup_shim(&shim).unwrap_err().code,
            "trusted_tool_invalid"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn development_rustup_proxy_rejects_missing_or_oversized_settings() {
        let root = PathBuf::from(std::env::var_os("HOME").unwrap())
            .join(".cache")
            .join(format!("asb-ar1634-rustup-settings-{}", std::process::id()));
        prepare_private_directory(&root).unwrap();
        let proxy = root.join("bin/rustup");
        prepare_private_directory(proxy.parent().unwrap()).unwrap();
        fs::write(&proxy, b"rustup proxy").unwrap();
        fs::set_permissions(&proxy, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            resolve_development_cargo_from_with_rustup(
                Some(proxy.as_os_str()),
                None,
                None,
                Some(&root),
            )
            .unwrap_err()
            .code,
            "trusted_tool_invalid"
        );
        fs::write(root.join("settings.toml"), vec![b'x'; 64 * 1024 + 1]).unwrap();
        assert_eq!(
            resolve_development_cargo_from_with_rustup(
                Some(proxy.as_os_str()),
                None,
                None,
                Some(&root),
            )
            .unwrap_err()
            .code,
            "trusted_tool_invalid"
        );
        fs::write(
            root.join("settings.toml"),
            b"version = \"12\"\ndefault_toolchain = \"fixture-toolchain\"\n",
        )
        .unwrap();
        fs::set_permissions(
            root.join("settings.toml"),
            fs::Permissions::from_mode(0o666),
        )
        .unwrap();
        assert_eq!(
            resolve_development_cargo_from_with_rustup(
                Some(proxy.as_os_str()),
                None,
                None,
                Some(&root),
            )
            .unwrap_err()
            .code,
            "trusted_tool_unavailable"
        );
        fs::write(
            root.join("settings.toml"),
            b"version = \"12\"\ndefault_toolchain = \"../escape\"\n",
        )
        .unwrap();
        assert_eq!(
            resolve_development_cargo_from_with_rustup(
                Some(proxy.as_os_str()),
                None,
                None,
                Some(&root),
            )
            .unwrap_err()
            .code,
            "trusted_tool_invalid"
        );
        fs::remove_file(root.join("settings.toml")).unwrap();
        let substituted_settings = root.join("substituted-settings.toml");
        fs::write(
            &substituted_settings,
            b"version = \"12\"\ndefault_toolchain = \"fixture-toolchain\"\n",
        )
        .unwrap();
        std::os::unix::fs::symlink(&substituted_settings, root.join("settings.toml")).unwrap();
        assert_eq!(
            resolve_development_cargo_from_with_rustup(
                Some(proxy.as_os_str()),
                None,
                None,
                Some(&root),
            )
            .unwrap_err()
            .code,
            "trusted_tool_invalid"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn development_build_propagates_only_validated_rustup_home() {
        let root = PathBuf::from(std::env::var_os("HOME").unwrap())
            .join(".cache")
            .join(format!("asb-ar1634-rustup-env-{}", std::process::id()));
        prepare_private_directory(&root).unwrap();
        let mut command = Command::new("/usr/bin/env");
        command.env_clear();
        apply_development_toolchain_environment(&mut command, Some(&root), None, None, None, None);
        let output = command.output().unwrap();
        assert!(
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .any(|line| line == format!("RUSTUP_HOME={}", root.display()))
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn development_build_propagates_validated_toolchain_path() {
        let root = PathBuf::from(std::env::var_os("HOME").unwrap())
            .join(".cache")
            .join(format!("asb-ar1636-toolchain-env-{}", std::process::id()));
        prepare_private_directory(&root).unwrap();
        let bin = root.join("toolchains/fixture-toolchain/bin");
        prepare_private_directory(&bin).unwrap();
        let cargo = bin.join("cargo");
        let rustc = bin.join("rustc");
        let cc = bin.join("cc");
        let ar = bin.join("ar");
        let ld = bin.join("ld");
        fs::write(&cargo, b"cargo").unwrap();
        fs::write(&rustc, b"rustc").unwrap();
        fs::write(&cc, b"cc").unwrap();
        fs::write(&ar, b"ar").unwrap();
        fs::write(&ld, b"ld").unwrap();
        for tool in [&cargo, &rustc, &cc, &ar, &ld] {
            fs::set_permissions(tool, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let mut command = Command::new("/usr/bin/env");
        command.env_clear();
        let rustc = resolve_development_rustc(&cargo, Some(&root), false)
            .unwrap()
            .unwrap();
        let cc = validate_development_tool(&cc).unwrap();
        let ar = validate_development_tool(&ar).unwrap();
        let ld = validate_development_tool(&ld).unwrap();
        let linker_prefix = materialize_development_linker_prefix(&root, &ld).unwrap();
        let linker_path = linker_prefix.linker_path();
        apply_development_toolchain_environment(
            &mut command,
            Some(&root),
            Some(&rustc),
            Some(&cc),
            Some(&ar),
            Some(&linker_path),
        );
        let output = command.output().unwrap();
        let variables = String::from_utf8_lossy(&output.stdout);
        assert!(
            variables
                .lines()
                .any(|line| line == format!("RUSTUP_HOME={}", root.display()))
        );
        assert!(
            variables
                .lines()
                .any(|line| line == format!("RUSTC={}", rustc.display()))
        );
        assert!(
            variables
                .lines()
                .any(|line| line == format!("CC={}", cc.display()))
        );
        assert!(
            variables
                .lines()
                .any(|line| line == format!("RUSTC_LINKER={}", cc.display()))
        );
        assert!(variables.lines().any(|line| {
            line == format!(
                "CARGO_TARGET_{}_LINKER={}",
                target().to_ascii_uppercase().replace('-', "_"),
                cc.display()
            )
        }));
        assert!(
            variables
                .lines()
                .any(|line| line == format!("AR={}", ar.display()))
        );
        assert!(
            variables
                .lines()
                .any(|line| line == format!("LD={}", linker_path.display()))
        );
        assert!(!variables.lines().any(|line| line == "RUSTFLAGS="));
        assert!(
            !variables
                .lines()
                .any(|line| line.starts_with("CARGO_TARGET_") && line.contains("_RUSTFLAGS="))
        );
        assert!(!variables.lines().any(|line| line.starts_with("PATH=")));

        let target_dir = root.join("target");
        let cargo_home = root.join("cargo-home");
        let encoded =
            development_encoded_rustflags(&root, &target_dir, &cargo_home, &linker_prefix)
                .unwrap()
                .into_vec();
        let arguments: Vec<&[u8]> = encoded.split(|byte| *byte == 0x1f).collect();
        assert_eq!(arguments.len(), 5);
        assert_eq!(
            arguments[0],
            format!("--remap-path-prefix={}=/asb-dev/workspace", root.display()).as_bytes()
        );
        assert_eq!(
            arguments[1],
            format!(
                "--remap-path-prefix={}=/asb-dev/target",
                target_dir.display()
            )
            .as_bytes()
        );
        assert_eq!(
            arguments[2],
            format!(
                "--remap-path-prefix={}=/asb-dev/cargo-home",
                cargo_home.display()
            )
            .as_bytes()
        );
        assert_eq!(arguments[3], b"-C");
        assert_eq!(
            arguments[4],
            format!("link-arg=-B{}", linker_prefix.search_root().display()).as_bytes()
        );
        drop(linker_prefix);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn development_env_cleared_cargo_links_with_validated_ld_root() {
        let scratch = Scratch::new("development-real-link");
        let source = scratch.0.join("source");
        let target_dir = scratch.0.join("target");
        let cargo_home = scratch.0.join("cargo-home");
        prepare_private_directory(&source.join("src")).unwrap();
        prepare_private_directory(&target_dir).unwrap();
        prepare_private_directory(&cargo_home).unwrap();
        fs::write(
            source.join("Cargo.toml"),
            b"[package]\nname = \"asb-development-link-fixture\"\nversion = \"0.0.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        fs::write(
            source.join("src/main.rs"),
            b"fn main() { println!(\"validated linker fixture\"); }\n",
        )
        .unwrap();

        let rustup_home = resolve_development_rustup_home().unwrap();
        let cargo = resolve_development_cargo_with_rustup(rustup_home.as_deref()).unwrap();
        let rustc = resolve_development_rustc_bound(
            &cargo,
            rustup_home.as_deref(),
            std::env::var_os(DEV_CARGO_OVERRIDE).is_some(),
        )
        .unwrap();
        let cc = resolve_development_tool_candidates(
            DEV_CC_OVERRIDE,
            &["/usr/bin/cc", "/usr/local/bin/cc"],
        )
        .unwrap();
        let ar = resolve_development_tool_candidates(
            DEV_AR_OVERRIDE,
            &["/usr/bin/ar", "/usr/local/bin/ar"],
        )
        .unwrap();
        let ld = resolve_development_tool_candidates(
            DEV_LD_OVERRIDE,
            &["/usr/bin/ld", "/usr/local/bin/ld"],
        )
        .unwrap();
        let linker_prefix = materialize_development_linker_prefix(&scratch.0, &ld).unwrap();
        let linker_path = linker_prefix.linker_path();
        let mut command = Command::new(cargo.execution_path());
        command
            .env_clear()
            .env("LANG", "C.UTF-8")
            .env("HOME", &scratch.0)
            .env("CARGO_HOME", &cargo_home)
            .env("CARGO_TARGET_DIR", &target_dir)
            .env("CARGO_INCREMENTAL", "0")
            .env("SOURCE_DATE_EPOCH", "0")
            .env(
                "CARGO_ENCODED_RUSTFLAGS",
                development_encoded_rustflags(&scratch.0, &target_dir, &cargo_home, &linker_prefix)
                    .unwrap(),
            )
            .current_dir(&source)
            .args(["build", "--offline", "--release"]);
        apply_development_toolchain_environment(
            &mut command,
            rustup_home.as_deref(),
            rustc.as_ref().map(|tool| tool.execution_path()).as_deref(),
            Some(&cc),
            Some(&ar),
            Some(&linker_path),
        );
        linker_prefix.validate().unwrap();
        let linker_flags = make_descriptor_inheritable(&linker_prefix.directory).unwrap();
        let inherited = make_toolchain_descriptors_inheritable(
            cargo.bound_file.as_ref(),
            rustc.as_ref().and_then(|tool| tool.bound_file.as_ref()),
        )
        .unwrap();
        let output = command.output().unwrap();
        restore_descriptor_flags(&linker_prefix.directory, linker_flags).unwrap();
        restore_toolchain_descriptor_flags(
            cargo.bound_file.as_ref(),
            rustc.as_ref().and_then(|tool| tool.bound_file.as_ref()),
            inherited,
        )
        .unwrap();
        assert!(
            output.status.success(),
            "env-cleared Cargo link failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            target_dir
                .join("release/asb-development-link-fixture")
                .is_file()
        );
    }

    #[cfg(unix)]
    #[test]
    fn development_linker_prefix_excludes_hostile_siblings_and_path_replacement() {
        let scratch = Scratch::new("development-hostile-linker-siblings");
        let hostile_root = PathBuf::from(std::env::var_os("HOME").unwrap())
            .join(".cache")
            .join(format!(
                "asb-ar1751-hostile-linker-{}-{}",
                std::process::id(),
                TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
        let build_root = scratch.0.join("build");
        prepare_private_directory(&hostile_root).unwrap();
        prepare_private_directory(&build_root).unwrap();
        let system_ld = resolve_development_tool_candidates(
            DEV_LD_OVERRIDE,
            &["/usr/bin/ld", "/usr/local/bin/ld"],
        )
        .unwrap();
        let source_ld = hostile_root.join("ld");
        fs::copy(&system_ld, &source_ld).unwrap();
        fs::set_permissions(&source_ld, fs::Permissions::from_mode(0o700)).unwrap();
        let marker = scratch.0.join("hostile-helper-ran");
        fs::write(
            hostile_root.join("collect2"),
            format!(
                "#!/bin/sh\nprintf hostile > {}\nexit 91\n",
                marker.display()
            ),
        )
        .unwrap();
        fs::set_permissions(
            hostile_root.join("collect2"),
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        fs::write(hostile_root.join("crtbegin.o"), b"hostile startup object").unwrap();
        fs::write(hostile_root.join("libgcc.a"), b"hostile library").unwrap();

        let linker_prefix = materialize_development_linker_prefix(&build_root, &source_ld).unwrap();
        let retained_prefix = build_root.join("retained-linker-prefix");
        fs::rename(build_root.join("linker-prefix"), &retained_prefix).unwrap();
        prepare_private_directory(&build_root.join("linker-prefix")).unwrap();
        fs::copy("/bin/false", build_root.join("linker-prefix/ld")).unwrap();
        fs::copy("/bin/false", build_root.join("linker-prefix/collect2")).unwrap();
        fs::rename(&source_ld, hostile_root.join("validated-ld-replaced")).unwrap();
        fs::copy("/bin/false", &source_ld).unwrap();
        fs::set_permissions(&source_ld, fs::Permissions::from_mode(0o700)).unwrap();
        linker_prefix.validate().unwrap();

        let before = fcntl_getfd(&linker_prefix.directory).unwrap();
        assert!(before.contains(FdFlags::CLOEXEC));
        let inherited = make_descriptor_inheritable(&linker_prefix.directory).unwrap();
        assert!(
            !fcntl_getfd(&linker_prefix.directory)
                .unwrap()
                .contains(FdFlags::CLOEXEC)
        );
        let cc = resolve_development_tool_candidates(
            DEV_CC_OVERRIDE,
            &["/usr/bin/cc", "/usr/local/bin/cc"],
        )
        .unwrap();
        let search_argument = format!("-B{}", linker_prefix.search_root().display());
        let trace = Command::new(&cc)
            .env_clear()
            .env("LANG", "C.UTF-8")
            .args([
                "-###",
                &search_argument,
                "-x",
                "c",
                "/dev/null",
                "-o",
                "/dev/null",
            ])
            .output()
            .unwrap();
        assert!(trace.status.success());
        let trace = String::from_utf8_lossy(&trace.stderr);
        assert!(trace.contains(&format!("-L{}", linker_prefix.search_root().display())));
        assert!(!trace.contains(&hostile_root.to_string_lossy().into_owned()));
        assert!(
            !trace.contains(
                &build_root
                    .join("linker-prefix")
                    .to_string_lossy()
                    .into_owned()
            )
        );
        assert!(!trace.contains(&format!(
            "{}/collect2",
            linker_prefix.search_root().display()
        )));

        let source = scratch.0.join("main.c");
        let executable = scratch.0.join("fixture");
        fs::write(&source, b"int main(void) { return 0; }\n").unwrap();
        let output = Command::new(&cc)
            .env_clear()
            .env("LANG", "C.UTF-8")
            .arg(&search_argument)
            .arg(&source)
            .arg("-o")
            .arg(&executable)
            .output()
            .unwrap();
        restore_descriptor_flags(&linker_prefix.directory, inherited).unwrap();
        assert!(
            fcntl_getfd(&linker_prefix.directory)
                .unwrap()
                .contains(FdFlags::CLOEXEC)
        );
        assert!(
            output.status.success(),
            "confined GCC link failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(executable.is_file());
        assert!(!marker.exists());
        fs::remove_dir_all(hostile_root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn development_linker_prefix_rejects_extra_entry_and_inode_substitution() {
        let scratch = Scratch::new("development-linker-prefix-substitution");
        let ld = resolve_development_tool_candidates(
            DEV_LD_OVERRIDE,
            &["/usr/bin/ld", "/usr/local/bin/ld"],
        )
        .unwrap();
        let prefix = materialize_development_linker_prefix(&scratch.0, &ld).unwrap();
        prefix
            .directory
            .set_permissions(fs::Permissions::from_mode(0o700))
            .unwrap();
        fs::write(prefix.search_root().join("collect2"), b"hostile helper").unwrap();
        prefix
            .directory
            .set_permissions(fs::Permissions::from_mode(0o500))
            .unwrap();
        assert_eq!(prefix.validate().unwrap_err().code, "trusted_tool_invalid");

        prefix
            .directory
            .set_permissions(fs::Permissions::from_mode(0o700))
            .unwrap();
        fs::remove_file(prefix.search_root().join("collect2")).unwrap();
        fs::remove_file(prefix.linker_path()).unwrap();
        fs::copy("/bin/false", prefix.linker_path()).unwrap();
        fs::set_permissions(prefix.linker_path(), fs::Permissions::from_mode(0o500)).unwrap();
        prefix
            .directory
            .set_permissions(fs::Permissions::from_mode(0o500))
            .unwrap();
        assert_eq!(prefix.validate().unwrap_err().code, "trusted_tool_invalid");
    }

    #[cfg(unix)]
    #[test]
    fn development_linker_prefix_requires_gcc_collect2_semantics() {
        let scratch = Scratch::new("development-linker-driver");
        let ld = resolve_development_tool_candidates(
            DEV_LD_OVERRIDE,
            &["/usr/bin/ld", "/usr/local/bin/ld"],
        )
        .unwrap();
        let prefix = materialize_development_linker_prefix(&scratch.0, &ld).unwrap();
        let cc = resolve_development_tool_candidates(
            DEV_CC_OVERRIDE,
            &["/usr/bin/cc", "/usr/local/bin/cc"],
        )
        .unwrap();
        validate_development_gcc_driver(&cc, &prefix, &scratch.0).unwrap();
        assert!(
            fcntl_getfd(&prefix.directory)
                .unwrap()
                .contains(FdFlags::CLOEXEC)
        );
        assert_eq!(
            validate_development_gcc_driver(Path::new("/bin/false"), &prefix, &scratch.0)
                .unwrap_err()
                .code,
            "development_compiler_unsupported"
        );
        assert!(
            fcntl_getfd(&prefix.directory)
                .unwrap()
                .contains(FdFlags::CLOEXEC)
        );

        let relative = scratch.0.join("relative-driver");
        fs::write(&relative, b"#!/bin/sh\nprintf relative-collect2\n").unwrap();
        fs::set_permissions(&relative, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            validate_development_gcc_driver(&relative, &prefix, &scratch.0)
                .unwrap_err()
                .code,
            "development_compiler_unsupported"
        );
        assert!(
            fcntl_getfd(&prefix.directory)
                .unwrap()
                .contains(FdFlags::CLOEXEC)
        );

        let missing = scratch.0.join("missing-driver");
        fs::write(
            &missing,
            b"#!/bin/sh\nprintf /definitely/missing/collect2\n",
        )
        .unwrap();
        fs::set_permissions(&missing, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            validate_development_gcc_driver(&missing, &prefix, &scratch.0)
                .unwrap_err()
                .code,
            "development_compiler_unsupported"
        );
        assert!(
            fcntl_getfd(&prefix.directory)
                .unwrap()
                .contains(FdFlags::CLOEXEC)
        );

        let unavailable = scratch.0.join("unavailable-driver");
        assert_eq!(
            validate_development_gcc_driver(&unavailable, &prefix, &scratch.0)
                .unwrap_err()
                .code,
            "development_compiler_unsupported"
        );
        assert!(
            fcntl_getfd(&prefix.directory)
                .unwrap()
                .contains(FdFlags::CLOEXEC)
        );
    }

    #[cfg(unix)]
    #[test]
    fn development_gcc_probe_kills_stdout_descendant_and_restores_cloexec() {
        let scratch = Scratch::new("development-linker-probe-descendant");
        let ld = resolve_development_tool_candidates(
            DEV_LD_OVERRIDE,
            &["/usr/bin/ld", "/usr/local/bin/ld"],
        )
        .unwrap();
        let prefix = materialize_development_linker_prefix(&scratch.0, &ld).unwrap();
        let pid_file = scratch.0.join("descendant.pid");
        let inherited_marker = scratch.0.join("descendant-inherited-fd");
        let escaped_marker = scratch.0.join("descendant-escaped");
        let driver = scratch.0.join("hostile-driver");
        let script = format!(
            "#!/bin/sh\n{{\n  if test -e /proc/self/fd/{fd}; then printf retained > {inherited}; fi\n  /bin/sleep 30\n  printf escaped > {escaped}\n}} &\nchild=$!\nprintf '%s\\n' \"$child\" > {pid}\nwhile ! test -s {inherited}; do :; done\nprintf '/definitely/missing/collect2\\n'\nexit 0\n",
            fd = prefix.directory.as_raw_fd(),
            inherited = shell_quote(&inherited_marker),
            escaped = shell_quote(&escaped_marker),
            pid = shell_quote(&pid_file),
        );
        fs::write(&driver, script).unwrap();
        fs::set_permissions(&driver, fs::Permissions::from_mode(0o700)).unwrap();
        let probe_root = scratch.0.join("probe-root");
        prepare_private_directory(&probe_root).unwrap();

        let started = Instant::now();
        assert_eq!(
            validate_development_gcc_driver_with_timeout(
                &driver,
                &prefix,
                &probe_root,
                Duration::from_secs(2),
            )
            .unwrap_err()
            .code,
            "development_compiler_unsupported"
        );
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(inherited_marker.is_file());
        assert!(
            fcntl_getfd(&prefix.directory)
                .unwrap()
                .contains(FdFlags::CLOEXEC)
        );
        let descendant: i32 = fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline && Path::new(&format!("/proc/{descendant}")).exists() {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(!Path::new(&format!("/proc/{descendant}")).exists());
        assert!(!escaped_marker.exists());

        let timeout_driver = scratch.0.join("timeout-driver");
        fs::write(&timeout_driver, b"#!/bin/sh\n/bin/sleep 30\n").unwrap();
        fs::set_permissions(&timeout_driver, fs::Permissions::from_mode(0o700)).unwrap();
        let timeout_root = scratch.0.join("timeout-root");
        prepare_private_directory(&timeout_root).unwrap();
        let started = Instant::now();
        assert_eq!(
            validate_development_gcc_driver_with_timeout(
                &timeout_driver,
                &prefix,
                &timeout_root,
                Duration::from_millis(100),
            )
            .unwrap_err()
            .code,
            "development_compiler_unsupported"
        );
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(
            fcntl_getfd(&prefix.directory)
                .unwrap()
                .contains(FdFlags::CLOEXEC)
        );

        let writer_pid_file = scratch.0.join("writer-descendant.pid");
        let writer_pid_staging = scratch.0.join("writer-descendant.pid.tmp");
        let writer_inherited_marker = scratch.0.join("writer-inherited-fd");
        let writer_start_marker = scratch.0.join("writer-start");
        let writer_driver = scratch.0.join("continuous-writer-driver");
        let script = format!(
            "#!/bin/sh\n{{\n  if test -e /proc/self/fd/{fd}; then printf retained > {inherited}; fi\n  while ! test -e {start}; do :; done\n  exec /bin/cat /dev/zero\n}} &\nchild=$!\nprintf \"%s\\n\" \"$child\" > {pid_staging}\n/bin/mv {pid_staging} {pid}\nwhile ! test -s {inherited}; do :; done\n: > {start}\n/bin/sleep 30\n",
            fd = prefix.directory.as_raw_fd(),
            inherited = shell_quote(&writer_inherited_marker),
            start = shell_quote(&writer_start_marker),
            pid_staging = shell_quote(&writer_pid_staging),
            pid = shell_quote(&writer_pid_file),
        );
        fs::write(&writer_driver, script).unwrap();
        fs::set_permissions(&writer_driver, fs::Permissions::from_mode(0o700)).unwrap();
        let writer_root = scratch.0.join("continuous-writer-root");
        prepare_private_directory(&writer_root).unwrap();
        let started = Instant::now();
        assert_eq!(
            validate_development_gcc_driver_with_timeout(
                &writer_driver,
                &prefix,
                &writer_root,
                Duration::from_millis(500),
            )
            .unwrap_err()
            .code,
            "development_compiler_unsupported"
        );
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(writer_inherited_marker.is_file());
        assert!(
            fcntl_getfd(&prefix.directory)
                .unwrap()
                .contains(FdFlags::CLOEXEC)
        );
        let writer_descendant: i32 = fs::read_to_string(&writer_pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline && Path::new(&format!("/proc/{writer_descendant}")).exists()
        {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(!Path::new(&format!("/proc/{writer_descendant}")).exists());
    }

    #[cfg(unix)]
    #[test]
    fn development_linker_prefix_rejects_untrusted_linker_paths() {
        let scratch = Scratch::new("development-linker-rejection");
        let target_dir = scratch.0.join("target");
        let cargo_home = scratch.0.join("cargo-home");
        let missing = scratch.0.join("missing-ld");
        assert_eq!(
            materialize_development_linker_prefix(&scratch.0, &missing)
                .unwrap_err()
                .code,
            "trusted_tool_invalid"
        );
        assert_eq!(
            materialize_development_linker_prefix(&scratch.0, Path::new("relative-ld"))
                .unwrap_err()
                .code,
            "trusted_tool_invalid"
        );

        let writable = scratch.0.join("writable-ld");
        fs::write(&writable, b"not executable").unwrap();
        fs::set_permissions(&writable, fs::Permissions::from_mode(0o722)).unwrap();
        assert_eq!(
            materialize_development_linker_prefix(&scratch.0, &writable)
                .unwrap_err()
                .code,
            "trusted_tool_invalid"
        );

        let trusted = scratch.0.join("trusted-ld");
        let substituted = scratch.0.join("substituted-ld");
        fs::write(&trusted, b"fixture").unwrap();
        fs::set_permissions(&trusted, fs::Permissions::from_mode(0o700)).unwrap();
        symlink(&trusted, &substituted).unwrap();
        assert_eq!(
            materialize_development_linker_prefix(&scratch.0, &substituted)
                .unwrap_err()
                .code,
            "trusted_tool_invalid"
        );

        let encoded_separator = scratch.0.join("encoded\u{1f}separator");
        let ld = resolve_development_tool_candidates(
            DEV_LD_OVERRIDE,
            &["/usr/bin/ld", "/usr/local/bin/ld"],
        )
        .unwrap();
        let encoded_prefix_root = scratch.0.join("encoded-prefix");
        prepare_private_directory(&encoded_prefix_root).unwrap();
        let linker_prefix =
            materialize_development_linker_prefix(&encoded_prefix_root, &ld).unwrap();
        assert_eq!(
            development_encoded_rustflags(
                &encoded_separator,
                &target_dir,
                &cargo_home,
                &linker_prefix,
            )
            .unwrap_err()
            .code,
            "trusted_tool_invalid"
        );

        let private_root = PathBuf::from(std::env::var_os("HOME").unwrap())
            .join(".cache")
            .join(format!("asb-ar1737-non-utf8-linker-{}", std::process::id()));
        let non_utf8_root =
            private_root.join(std::ffi::OsString::from_vec(vec![b'l', b'd', b'-', 0xff]));
        prepare_private_directory(&non_utf8_root).unwrap();
        let non_utf8_ld = non_utf8_root.join("ld");
        fs::write(&non_utf8_ld, b"fixture").unwrap();
        fs::set_permissions(&non_utf8_ld, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            materialize_development_linker_prefix(&scratch.0, &non_utf8_ld)
                .unwrap_err()
                .code,
            "trusted_tool_invalid"
        );
        fs::remove_dir_all(private_root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn development_rustc_sibling_rejects_missing_unsafe_and_symlink_targets() {
        let root = PathBuf::from(std::env::var_os("HOME").unwrap())
            .join(".cache")
            .join(format!("asb-ar1636-rustc-reject-{}", std::process::id()));
        prepare_private_directory(&root).unwrap();
        let bin = root.join("toolchains/fixture-toolchain/bin");
        prepare_private_directory(&bin).unwrap();
        let cargo = bin.join("cargo");
        let rustc = bin.join("rustc");
        fs::write(&cargo, b"cargo").unwrap();
        fs::set_permissions(&cargo, fs::Permissions::from_mode(0o700)).unwrap();
        let foreign_root = root.join("foreign-rustup");
        prepare_private_directory(&foreign_root).unwrap();
        fs::write(&rustc, b"rustc").unwrap();
        fs::set_permissions(&rustc, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            resolve_development_rustc(&cargo, Some(&foreign_root), false)
                .unwrap_err()
                .code,
            "trusted_tool_invalid"
        );
        fs::remove_file(&rustc).unwrap();
        assert_eq!(
            resolve_development_rustc(&cargo, Some(&root), false)
                .unwrap_err()
                .code,
            "trusted_tool_unavailable"
        );
        fs::write(&rustc, b"rustc").unwrap();
        fs::set_permissions(&rustc, fs::Permissions::from_mode(0o777)).unwrap();
        assert_eq!(
            resolve_development_rustc(&cargo, Some(&root), false).unwrap(),
            Some(rustc.clone())
        );
        fs::remove_file(&rustc).unwrap();
        let unsafe_target = root.join("unsafe-rustc");
        fs::write(&unsafe_target, b"rustc").unwrap();
        fs::set_permissions(&unsafe_target, fs::Permissions::from_mode(0o777)).unwrap();
        std::os::unix::fs::symlink(&unsafe_target, &rustc).unwrap();
        assert_eq!(
            resolve_development_rustc(&cargo, Some(&root), false)
                .unwrap_err()
                .code,
            "trusted_tool_invalid"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn development_tool_overrides_accept_private_immutable_paths_only() {
        let root = PathBuf::from(std::env::var_os("HOME").unwrap())
            .join(".cache")
            .join(format!("asb-ar1572-tools-{}", std::process::id()));
        prepare_private_directory(&root).unwrap();
        let tool = root.join("bin/tool");
        prepare_private_directory(tool.parent().unwrap()).unwrap();
        fs::write(&tool, b"tool").unwrap();
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            resolve_development_tool_from(Some(tool.as_os_str()), "/missing/tool").unwrap(),
            fs::canonicalize(&tool).unwrap()
        );
        assert_eq!(
            resolve_development_tool_from(
                Some(std::ffi::OsStr::new("relative/tool")),
                "/missing/tool"
            )
            .unwrap_err()
            .code,
            "trusted_tool_invalid"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn development_linker_candidates_reject_unsafe_overrides() {
        let root = PathBuf::from(std::env::var_os("HOME").unwrap())
            .join(".cache")
            .join(format!("asb-ar1638-linker-{}", std::process::id()));
        prepare_private_directory(&root).unwrap();
        let tool = root.join("bin/cc");
        prepare_private_directory(tool.parent().unwrap()).unwrap();
        fs::write(&tool, b"cc").unwrap();
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            resolve_development_tool_candidates_from(
                Some(std::ffi::OsStr::new("relative/cc")),
                &[],
            )
            .unwrap_err()
            .code,
            "trusted_tool_invalid"
        );
        assert_eq!(
            resolve_development_tool_candidates_from(Some(root.join("missing").as_os_str()), &[],)
                .unwrap_err()
                .code,
            "trusted_tool_invalid"
        );
        assert_eq!(
            resolve_development_tool_candidates_from(Some(tool.as_os_str()), &[]).unwrap(),
            fs::canonicalize(&tool).unwrap()
        );
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o777)).unwrap();
        assert_eq!(
            resolve_development_tool_candidates_from(Some(tool.as_os_str()), &[])
                .unwrap_err()
                .code,
            "trusted_tool_invalid"
        );
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o700)).unwrap();
        let target = root.join("unsafe");
        fs::write(&target, b"cc").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o777)).unwrap();
        fs::remove_file(&tool).unwrap();
        std::os::unix::fs::symlink(&target, &tool).unwrap();
        assert_eq!(
            resolve_development_tool_candidates_from(Some(tool.as_os_str()), &[])
                .unwrap_err()
                .code,
            "trusted_tool_invalid"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn development_commands_are_bounded_and_timeout_cleanup_is_private() {
        let scratch = Scratch::new("dev-command");
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "printf bounded"]);
        assert_eq!(
            run_development_command_with_limits(
                command,
                &scratch.0,
                Duration::from_secs(1),
                MAX_DEV_WORKSPACE_BYTES,
            )
            .unwrap(),
            b"bounded"
        );
        let timeout_scratch = Scratch::new("dev-timeout");
        let mut timeout = Command::new(DEV_SETSID);
        timeout.args(["--wait", "/bin/sh", "-c", "sleep 2"]);
        let error = run_development_command_with_limits(
            timeout,
            &timeout_scratch.0,
            Duration::from_millis(10),
            MAX_DEV_WORKSPACE_BYTES,
        )
        .unwrap_err();
        assert_eq!(error.code, "dev_command_timeout");
        assert!(!timeout_scratch.0.exists());
        let noisy_scratch = Scratch::new("dev-noisy");
        let mut noisy = Command::new(DEV_SETSID);
        noisy.args(["--wait", "/bin/sh", "-c", "head -c 200000 /dev/zero"]);
        assert_eq!(
            run_development_command_with_limits(
                noisy,
                &noisy_scratch.0,
                Duration::from_secs(1),
                MAX_DEV_WORKSPACE_BYTES,
            )
            .unwrap_err()
            .code,
            "dev_command_failed"
        );
    }

    #[test]
    fn development_workspace_quota_rejects_excess_and_symlinks() {
        let scratch = Scratch::new("dev-quota");
        fs::write(scratch.0.join("payload"), b"12345").unwrap();
        assert!(bounded_directory_size(&scratch.0, 4).unwrap() > 4);
        symlink(scratch.0.join("payload"), scratch.0.join("link")).unwrap();
        assert_eq!(
            bounded_directory_size(&scratch.0, MAX_DEV_WORKSPACE_BYTES)
                .unwrap_err()
                .code,
            "dev_workspace_unsafe"
        );
        let producer_scratch = Scratch::new("dev-quota-producer");
        let mut producer = Command::new(DEV_SETSID);
        producer.args([
            "--wait",
            "/bin/sh",
            "-c",
            &format!(
                "head -c 4096 /dev/zero > {}/descendant",
                producer_scratch.0.display()
            ),
        ]);
        assert_eq!(
            run_development_command_with_limits(
                producer,
                &producer_scratch.0,
                Duration::from_secs(2),
                64,
            )
            .unwrap_err()
            .code,
            "dev_workspace_quota_exceeded"
        );
        assert!(!producer_scratch.0.exists());
    }

    #[test]
    fn development_target_staging_has_independent_bounded_cleanup() {
        let source = Scratch::new("dev-target-source");
        let target = Scratch::new("dev-target-artifacts");
        fs::write(source.0.join("source-artifact"), [0_u8; 40]).unwrap();
        let mut producer = Command::new(DEV_SETSID);
        producer.args([
            "--wait",
            "/bin/sh",
            "-c",
            &format!("head -c 4096 /dev/zero > {}/artifact", target.0.display()),
        ]);
        let error = run_development_command_with_limits_and_roots(
            producer,
            &source.0,
            &[source.0.as_path(), target.0.as_path()],
            Duration::from_secs(2),
            64,
        )
        .unwrap_err();
        assert_eq!(error.code, "dev_workspace_quota_exceeded");
        assert!(!source.0.exists());
        assert!(target.0.exists());
        fs::remove_dir_all(target.0.clone()).unwrap();
    }

    #[test]
    fn development_aggregate_quota_accepts_bounded_private_roots() {
        let source = Scratch::new("dev-quota-source-ok");
        let target = Scratch::new("dev-quota-target-ok");
        fs::write(source.0.join("source"), [0_u8; 32]).unwrap();
        fs::write(target.0.join("target"), [0_u8; 32]).unwrap();
        assert_eq!(
            bounded_directory_size_for_roots(&[source.0.as_path(), target.0.as_path()], 128)
                .unwrap(),
            64
        );
        enforce_workspace_quota_for_roots(&[source.0.as_path(), target.0.as_path()], 128).unwrap();
        enforce_workspace_quota(&source.0).unwrap();
        enforce_workspace_quota_with_limit(&source.0, 128).unwrap();
    }

    #[test]
    fn development_aggregate_quota_rejects_overflow_and_reports_limit() {
        let source = Scratch::new("dev-quota-source-overflow");
        let target = Scratch::new("dev-quota-target-overflow");
        fs::write(source.0.join("source"), [0_u8; 80]).unwrap();
        fs::write(target.0.join("target"), [0_u8; 80]).unwrap();
        assert!(
            bounded_directory_size_for_roots(&[source.0.as_path(), target.0.as_path()], 128)
                .unwrap()
                > 128
        );
        assert_eq!(
            enforce_workspace_quota_for_roots(&[source.0.as_path(), target.0.as_path()], 128)
                .unwrap_err()
                .code,
            "dev_workspace_quota_exceeded"
        );
        assert_eq!(
            enforce_workspace_quota_with_limit(&source.0, 1)
                .unwrap_err()
                .code,
            "dev_workspace_quota_exceeded"
        );
    }

    #[test]
    fn development_artifact_tree_is_private_under_normal_umask() {
        let target = Scratch::new("dev-artifact-modes");
        let release = target.0.join("release");
        fs::create_dir(&release).unwrap();
        let executable = release.join("asb-tui");
        let sidecar = release.join("build-metadata");
        let nested = release.join("nested");
        fs::write(&executable, b"binary").unwrap();
        fs::write(&sidecar, b"metadata").unwrap();
        fs::create_dir(&nested).unwrap();
        fs::write(nested.join("debug"), b"debug").unwrap();
        fs::set_permissions(&release, fs::Permissions::from_mode(0o775)).unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o775)).unwrap();
        fs::set_permissions(&sidecar, fs::Permissions::from_mode(0o775)).unwrap();
        fs::set_permissions(&nested, fs::Permissions::from_mode(0o775)).unwrap();
        fs::set_permissions(nested.join("debug"), fs::Permissions::from_mode(0o775)).unwrap();
        harden_private_development_tree(&target.0, &executable).unwrap();
        assert_eq!(target.0.metadata().unwrap().mode() & 0o777, 0o700);
        assert_eq!(release.metadata().unwrap().mode() & 0o777, 0o700);
        assert_eq!(executable.metadata().unwrap().mode() & 0o777, 0o700);
        assert_eq!(sidecar.metadata().unwrap().mode() & 0o777, 0o600);
        assert_eq!(nested.metadata().unwrap().mode() & 0o777, 0o700);
        assert_eq!(
            nested.join("debug").metadata().unwrap().mode() & 0o777,
            0o600
        );
        assert_eq!(read_bounded(&executable, 64).unwrap(), b"binary");
    }

    #[test]
    fn development_artifact_tree_rejects_missing_and_symlinked_nodes() {
        let scratch = Scratch::new("dev-artifact-invalid");
        let missing = scratch.0.join("missing");
        assert_eq!(
            harden_private_development_tree(&missing, &missing)
                .unwrap_err()
                .code,
            "dev_artifact_invalid"
        );
        let outside = Scratch::new("dev-artifact-link");
        let root_link = scratch.0.join("root-link");
        symlink(&outside.0, &root_link).unwrap();
        assert_eq!(
            harden_private_development_tree(&root_link, &root_link)
                .unwrap_err()
                .code,
            "dev_artifact_invalid"
        );
        let file_root = scratch.0.join("file-root");
        fs::write(&file_root, b"not-a-directory").unwrap();
        assert_eq!(
            harden_private_development_tree(&file_root, &file_root)
                .unwrap_err()
                .code,
            "dev_artifact_invalid"
        );
        let child_root = scratch.0.join("child-root");
        fs::create_dir(&child_root).unwrap();
        let target = Scratch::new("dev-artifact-outside");
        symlink(&target.0, child_root.join("link")).unwrap();
        assert_eq!(
            harden_private_development_tree(&child_root, &child_root)
                .unwrap_err()
                .code,
            "dev_artifact_invalid"
        );
    }

    #[test]
    fn bounded_directory_size_tolerates_vanished_child_entries() {
        let scratch = Scratch::new("dev-quota-race");
        let missing = scratch.0.join("vanished");
        assert_eq!(
            bounded_directory_size_inner(&missing, MAX_DEV_WORKSPACE_BYTES, true).unwrap(),
            None
        );
        fs::write(scratch.0.join("stable"), [0_u8; 8]).unwrap();
        assert_eq!(
            bounded_directory_size(&scratch.0, MAX_DEV_WORKSPACE_BYTES).unwrap(),
            8
        );
    }

    #[test]
    fn quota_scan_error_terminates_child_and_cleans_root() {
        let scratch = Scratch::new("dev-quota-scan-error");
        let invalid_root = scratch.0.join("invalid-root");
        let target = Scratch::new("dev-quota-scan-target");
        symlink(&target.0, &invalid_root).unwrap();
        let marker = scratch.0.join("escaped");
        let mut command = Command::new(DEV_SETSID);
        command.args([
            "--wait",
            "/bin/sh",
            "-c",
            &format!("sleep 0.3; echo escaped > {}", marker.display()),
        ]);
        let error = run_development_command_with_limits_and_roots(
            command,
            &scratch.0,
            &[scratch.0.as_path(), invalid_root.as_path()],
            Duration::from_secs(2),
            MAX_DEV_WORKSPACE_BYTES,
        )
        .unwrap_err();
        assert_eq!(error.code, "dev_workspace_unsafe");
        assert!(!scratch.0.exists());
        thread::sleep(Duration::from_millis(400));
        assert!(!marker.exists());
    }

    #[cfg(unix)]
    #[test]
    fn development_metadata_is_explicitly_non_production() {
        let metadata = DevelopmentInstallation {
            schema_version: 1,
            channel: "dev".to_owned(),
            development_only: true,
            source_repository: DEV_REPOSITORY_URL.to_owned(),
            source_commit: "a".repeat(40),
            source_tree: "c".repeat(40),
            asb_source_commit: ASB_SOURCE_COMMIT.to_owned(),
            asb_source_tree: ASB_SOURCE_TREE.to_owned(),
            executable_sha256: "b".repeat(64),
            installed_unix: 1,
        };
        let encoded = serde_json::to_value(metadata).unwrap();
        assert_eq!(encoded["channel"], "dev");
        assert_eq!(encoded["development_only"], true);
        assert_eq!(encoded["source_commit"].as_str().unwrap().len(), 40);
    }

    #[test]
    fn development_broker_descriptor_binds_current_identities() {
        let active = DevelopmentInstallation {
            schema_version: 1,
            channel: "dev".to_owned(),
            development_only: true,
            source_repository: DEV_REPOSITORY_URL.to_owned(),
            source_commit: "a".repeat(40),
            source_tree: "b".repeat(40),
            asb_source_commit: ASB_SOURCE_COMMIT.to_owned(),
            asb_source_tree: ASB_SOURCE_TREE.to_owned(),
            executable_sha256: "c".repeat(64),
            installed_unix: 1,
        };
        let descriptor: serde_json::Value =
            serde_json::from_str(&development_broker_descriptor(&active).unwrap()).unwrap();
        assert_eq!(descriptor["operation"], "launch");
        assert_eq!(descriptor["protocol_minor"], DEV_BROKER_PROTOCOL_MINOR);
        assert_eq!(descriptor["asb_source_commit"], ASB_SOURCE_COMMIT);
        assert_eq!(descriptor["tui_source_commit"], "a".repeat(40));

        let mut stale = active;
        stale.source_commit = "not-an-identity".to_owned();
        assert_eq!(
            development_broker_descriptor(&stale).unwrap_err().code,
            "dev_source_identity_unknown"
        );
    }

    #[test]
    fn development_broker_preserves_valid_terminal_capability_values() {
        assert!(valid_terminal_value(b"xterm-256color"));
        assert!(valid_terminal_value(b"screen"));
        assert!(!valid_terminal_value(b"xterm\n256color"));
        assert!(!valid_terminal_value(&[b'a'; 65]));
    }

    #[test]
    fn development_source_commit_must_match_checked_out_main() {
        let commit = "a".repeat(40);
        assert!(validate_development_source_commit(&commit, &commit).is_ok());
        assert_eq!(
            validate_development_source_commit(&commit, &"b".repeat(40))
                .unwrap_err()
                .code,
            "dev_source_identity_mismatch"
        );
    }

    #[test]
    fn development_main_head_resolver_reads_exact_local_main_ref() {
        let scratch = Scratch::new("dev-main-head");
        let repository = scratch.0.join("repository");
        prepare_private_directory(&repository).unwrap();
        assert!(
            Command::new(DEV_GIT)
                .args(["init", "--quiet"])
                .current_dir(&repository)
                .status()
                .unwrap()
                .success()
        );
        fs::write(repository.join("README"), b"fixture").unwrap();
        assert!(
            Command::new(DEV_GIT)
                .args(["add", "README"])
                .current_dir(&repository)
                .status()
                .unwrap()
                .success()
        );
        assert!(
            Command::new(DEV_GIT)
                .args([
                    "-c",
                    "user.name=ASB test",
                    "-c",
                    "user.email=asb-test@example.invalid",
                    "commit",
                    "--quiet",
                    "-m",
                    "fixture",
                ])
                .current_dir(&repository)
                .status()
                .unwrap()
                .success()
        );
        assert!(
            Command::new(DEV_GIT)
                .args(["branch", "-M", "main"])
                .current_dir(&repository)
                .status()
                .unwrap()
                .success()
        );
        let expected = String::from_utf8(
            Command::new(DEV_GIT)
                .args(["rev-parse", "HEAD"])
                .current_dir(&repository)
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .to_owned();
        let resolved = resolve_development_main_head(
            Path::new(DEV_GIT),
            Path::new(DEV_SETSID),
            repository.to_str().unwrap(),
            &scratch.0,
        )
        .unwrap();
        assert_eq!(resolved, expected);
        let invalid = resolve_development_main_head(
            Path::new(DEV_GIT),
            Path::new(DEV_SETSID),
            &scratch.0.join("missing").to_string_lossy(),
            &scratch.0,
        )
        .unwrap_err();
        assert_eq!(invalid.code, "dev_command_failed");
    }

    #[test]
    fn development_repeated_builds_have_identical_path_independent_bytes() {
        let scratch = Scratch::new("dev-reproducible-build");
        let mut digests = Vec::new();
        for name in ["first-random-root", "second-random-root"] {
            let root = scratch.0.join(name);
            let source = root.join("source");
            let target = root.join("target");
            let cargo_home = root.join("cargo-home");
            prepare_private_directory(&source).unwrap();
            prepare_private_directory(&target).unwrap();
            prepare_private_directory(&cargo_home).unwrap();
            fs::write(
                source.join("main.rs"),
                b"fn main() { println!(\"development fixture\"); }\n",
            )
            .unwrap();
            let executable = target.join("fixture");
            let ld = resolve_development_tool_candidates(
                DEV_LD_OVERRIDE,
                &["/usr/bin/ld", "/usr/local/bin/ld"],
            )
            .unwrap();
            let linker_prefix = materialize_development_linker_prefix(&root, &ld).unwrap();
            let flags = development_encoded_rustflags(&root, &target, &cargo_home, &linker_prefix)
                .unwrap()
                .into_vec();
            let flags: Vec<String> = flags
                .split(|byte| *byte == 0x1f)
                .map(|argument| String::from_utf8(argument.to_vec()).unwrap())
                .collect();
            let mut command = Command::new("rustc");
            command
                .env_clear()
                .env("LANG", "C.UTF-8")
                .env("PATH", std::env::var_os("PATH").unwrap_or_default())
                .env("SOURCE_DATE_EPOCH", "0")
                .current_dir(&source)
                .args([
                    "--edition=2021",
                    "-C",
                    "debuginfo=2",
                    "-C",
                    "incremental=no",
                ])
                .args(flags)
                .args(["main.rs", "-o"])
                .arg(&executable);
            let linker_flags = make_descriptor_inheritable(&linker_prefix.directory).unwrap();
            assert!(command.status().unwrap().success());
            restore_descriptor_flags(&linker_prefix.directory, linker_flags).unwrap();
            digests.push(fs::read(executable).map(|bytes| digest(&bytes)).unwrap());
        }
        assert_eq!(digests[0], digests[1]);
    }

    #[test]
    fn development_publication_rolls_back_new_version_on_marker_failure() {
        let scratch = Scratch::new("dev-publication-rollback");
        let install_root = scratch.0.join("install");
        prepare_private_directory(&install_root).unwrap();
        fs::create_dir(install_root.join("active-dev.json")).unwrap();
        let bytes = b"new development executable";
        let executable_sha256 = digest(bytes);
        let error = publish_development_version(
            &install_root,
            &executable_sha256,
            bytes,
            br#"{"schema_version":1}"#,
        )
        .unwrap_err();
        assert_eq!(error.code, "state_write_failed");
        assert!(
            !install_root
                .join("dev-versions")
                .join(executable_sha256)
                .exists()
        );
        assert!(install_root.join("active-dev.json").is_dir());
    }

    #[test]
    fn development_bundle_consumer_validates_size_before_publish() {
        let scratch = Scratch::new("development-bundle-consumer");
        let paths = RouterPaths {
            install_root: scratch.0.join("install"),
            state_root: scratch.0.join("state"),
            cache_root: scratch.0.join("cache"),
        };
        let bundle = scratch.0.join("bundle");
        fs::create_dir_all(&bundle).unwrap();
        let bytes = b"development executable";
        fs::write(bundle.join("asb-tui"), bytes).unwrap();
        fs::set_permissions(&bundle, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(bundle.join("asb-tui"), fs::Permissions::from_mode(0o700)).unwrap();
        let manifest = DevelopmentBundleManifest {
            schema_version: 1,
            channel: "dev".into(),
            development_only: true,
            source_repository: DEV_REPOSITORY_URL.into(),
            source_ref: "refs/heads/main".into(),
            source_commit: "a".repeat(40),
            source_tree: "b".repeat(40),
            asb_source_commit: ASB_SOURCE_COMMIT.into(),
            asb_source_tree: ASB_SOURCE_TREE.into(),
            target: format!("{}-unknown-linux-gnu", std::env::consts::ARCH),
            executable_sha256: digest(bytes),
            executable_size: bytes.len() as u64,
            built_unix: 1,
            warnings: development_warnings()
                .into_iter()
                .map(str::to_owned)
                .collect(),
        };
        fs::write(
            bundle.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let verified = consume_development_bundle(
            Operation::Install,
            Options {
                dry_run: true,
                ..Options::default()
            },
            &paths,
            2,
            ASB_SOURCE_COMMIT,
            ASB_SOURCE_TREE,
            &bundle,
        )
        .unwrap();
        assert_eq!(verified.code, "development_bundle_verified");
        let consumed = consume_development_bundle(
            Operation::Install,
            Options::default(),
            &paths,
            2,
            ASB_SOURCE_COMMIT,
            ASB_SOURCE_TREE,
            &bundle,
        )
        .unwrap();
        assert_eq!(consumed.code, "development_bundle_consumed");

        let mut zero_size = manifest;
        zero_size.executable_size = 0;
        fs::write(
            bundle.join("manifest.json"),
            serde_json::to_vec(&zero_size).unwrap(),
        )
        .unwrap();
        let rejected = consume_development_bundle(
            Operation::Install,
            Options {
                dry_run: true,
                ..Options::default()
            },
            &paths,
            2,
            ASB_SOURCE_COMMIT,
            ASB_SOURCE_TREE,
            &bundle,
        )
        .unwrap_err();
        assert_eq!(rejected.code, "development_bundle_invalid");
    }

    #[test]
    fn development_lifecycle_status_and_remove_are_atomic_and_typed() {
        let scratch = Scratch::new("dev-lifecycle");
        let paths = RouterPaths {
            install_root: scratch.0.join("install"),
            state_root: scratch.0.join("state"),
            cache_root: scratch.0.join("cache"),
        };
        prepare_private_directory(&paths.install_root).unwrap();
        let bytes = b"development executable";
        let executable_sha256 = digest(bytes);
        let version = paths
            .install_root
            .join("dev-versions")
            .join(&executable_sha256);
        prepare_private_directory(&version).unwrap();
        atomic_private(&version.join("asb-tui"), bytes, 0o700).unwrap();
        let metadata = DevelopmentInstallation {
            schema_version: 1,
            channel: "dev".to_owned(),
            development_only: true,
            source_repository: DEV_REPOSITORY_URL.to_owned(),
            source_commit: "a".repeat(40),
            source_tree: "b".repeat(40),
            asb_source_commit: ASB_SOURCE_COMMIT.to_owned(),
            asb_source_tree: ASB_SOURCE_TREE.to_owned(),
            executable_sha256,
            installed_unix: 1,
        };
        atomic_private(
            &paths.install_root.join("active-dev.json"),
            &serde_json::to_vec(&metadata).unwrap(),
            0o600,
        )
        .unwrap();
        atomic_private(
            &version.join("manifest.json"),
            &serde_json::to_vec(&metadata).unwrap(),
            0o600,
        )
        .unwrap();
        let channel_manifest = development_channel_manifest(
            &metadata.asb_source_commit,
            &metadata.asb_source_tree,
            &metadata.source_commit,
            &metadata.source_tree,
            &metadata.executable_sha256,
            bytes.len() as u64,
            metadata.installed_unix,
        );
        let channel_digest = publish_development_channel_manifest(
            &paths.install_root,
            &metadata.executable_sha256,
            &channel_manifest,
        )
        .unwrap();
        let root_channel_manifest = paths.install_root.join("active-channel.json");
        let version_channel_manifest = version.join("channel-manifest.json");
        let channel_bytes = fs::read(&root_channel_manifest).unwrap();
        let status = execute_development_existing(Operation::Status, &paths).unwrap();
        assert_eq!(status.code, "development_installed");
        assert!(status.development_only);
        assert_eq!(status.channel, "dev");
        assert_eq!(status.source_commit, Some("a".repeat(40)));
        assert_eq!(status.source_tree, Some("b".repeat(40)));
        assert_eq!(status.channel_manifest_sha256, Some(channel_digest));
        fs::write(&root_channel_manifest, b"{}").unwrap();
        assert_eq!(
            development_active(&paths).unwrap_err().code,
            "development_installation_invalid"
        );
        fs::write(&root_channel_manifest, &channel_bytes).unwrap();
        fs::write(&version_channel_manifest, b"{}").unwrap();
        assert_eq!(
            development_active(&paths).unwrap_err().code,
            "development_installation_invalid"
        );
        fs::write(&version_channel_manifest, &channel_bytes).unwrap();
        assert_eq!(
            status.warnings,
            Some(vec![
                "development_missing_authentication_allowed",
                "development_missing_signatures_allowed",
                "development_missing_key_management_allowed",
            ])
        );
        let active_before_remove = development_active(&paths).unwrap().unwrap().0;
        assert_eq!(
            remove_development_installation(&paths, &active_before_remove, true)
                .unwrap_err()
                .code,
            "development_remove_failed"
        );
        assert!(paths.install_root.join("active-dev.json").exists());
        assert!(version.exists());
        execute_development_existing(Operation::Remove, &paths).unwrap();
        assert!(!paths.install_root.join("active-dev.json").exists());
        assert!(!version.exists());
    }

    #[test]
    fn stale_development_identity_fails_closed_with_typed_diagnostic() {
        let scratch = Scratch::new("dev-identity");
        let paths = RouterPaths {
            install_root: scratch.0.join("install"),
            state_root: scratch.0.join("state"),
            cache_root: scratch.0.join("cache"),
        };
        prepare_private_directory(&paths.install_root).unwrap();
        let bytes = b"development executable";
        let executable_sha256 = digest(bytes);
        let version = paths
            .install_root
            .join("dev-versions")
            .join(&executable_sha256);
        prepare_private_directory(&version).unwrap();
        atomic_private(&version.join("asb-tui"), bytes, 0o700).unwrap();
        let metadata = DevelopmentInstallation {
            schema_version: 1,
            channel: "dev".to_owned(),
            development_only: true,
            source_repository: DEV_REPOSITORY_URL.to_owned(),
            source_commit: "a".repeat(40),
            source_tree: "b".repeat(40),
            asb_source_commit: ASB_SOURCE_COMMIT.to_owned(),
            asb_source_tree: ASB_SOURCE_TREE.to_owned(),
            executable_sha256,
            installed_unix: 1,
        };
        let mut tampered = serde_json::to_value(&metadata).unwrap();
        tampered["asb_source_commit"] = serde_json::Value::String("0".repeat(40));
        atomic_private(
            &version.join("manifest.json"),
            &serde_json::to_vec(&metadata).unwrap(),
            0o600,
        )
        .unwrap();
        atomic_private(
            &paths.install_root.join("active-dev.json"),
            &serde_json::to_vec(&tampered).unwrap(),
            0o600,
        )
        .unwrap();
        assert_eq!(
            development_active(&paths).unwrap_err().code,
            "development_installation_invalid"
        );
    }

    #[test]
    fn non_install_operations_preserve_resolved_channel() {
        let mut response = RouterResponse::result(Operation::Status, true, "ok", "denied");
        annotate_channel(&mut response, Channel::Dev);
        assert_eq!(response.channel, "dev");
        assert!(response.development_only);
        let mut absent = RouterResponse::result(
            Operation::Status,
            false,
            "extension_not_installed",
            "denied",
        );
        annotate_channel(&mut absent, Channel::Dev);
        assert_eq!(absent.channel, "dev");
        assert!(absent.development_only);
        annotate_channel(&mut response, Channel::Experimental);
        assert_eq!(response.channel, "experimental");
        assert!(!response.development_only);
    }

    #[test]
    fn launch_error_preserves_resolved_development_channel() {
        let mut response = RouterResponse::result(
            Operation::Launch,
            false,
            "development_launch_failed",
            "denied",
        );
        annotate_channel(&mut response, Channel::Dev);
        assert_eq!(response.channel, "dev");
        assert!(response.development_only);
    }

    struct NoopSource;

    impl Source for NoopSource {
        fn document(&mut self, _: &str, _: usize) -> Result<Vec<u8>, RouterError> {
            Err(RouterError::operation("unexpected_document"))
        }

        fn range(&mut self, _: &str, _: u64, _: usize) -> Result<Vec<u8>, RouterError> {
            Err(RouterError::operation("unexpected_range"))
        }
    }

    #[test]
    fn explicit_channels_are_typed_and_never_fallback() {
        let scratch = Scratch::new("channel-routing");
        let paths = RouterPaths::from_roots(
            &scratch.0.join("data"),
            &scratch.0.join("state"),
            &scratch.0.join("cache"),
        )
        .unwrap();
        let mut source = NoopSource;
        for channel in [Channel::Nightly, Channel::Experimental] {
            let options = Options {
                channel,
                channel_explicit: true,
                ..Options::default()
            };
            let error = execute(
                Operation::Install,
                options,
                &paths,
                &mut source,
                1,
                &mut Vec::new(),
                false,
            )
            .unwrap_err();
            assert_eq!(error.code, "channel_unavailable");
        }
        let options = Options {
            channel: Channel::Dev,
            channel_explicit: true,
            ..Options::default()
        };
        let error = execute(
            Operation::Status,
            options,
            &paths,
            &mut source,
            1,
            &mut Vec::new(),
            false,
        )
        .unwrap_err();
        assert_eq!(error.code, "channel_unavailable");
    }

    #[test]
    fn omitted_channel_resolves_dev_until_stable_is_installed() {
        let scratch = Scratch::new("channel-default");
        let paths = RouterPaths::from_roots(
            &scratch.0.join("data"),
            &scratch.0.join("state"),
            &scratch.0.join("cache"),
        )
        .unwrap();
        assert_eq!(
            existing_channel(Options::default(), &paths).unwrap(),
            Channel::Dev
        );
        fs::create_dir_all(&paths.install_root).unwrap();
        fs::write(
            paths.install_root.join("active.json"),
            b"not validated here",
        )
        .unwrap();
        assert_eq!(
            existing_channel(Options::default(), &paths).unwrap(),
            Channel::Stable
        );
        let explicit_dev = Options {
            channel: Channel::Dev,
            channel_explicit: true,
            ..Options::default()
        };
        assert_eq!(
            existing_channel(explicit_dev, &paths).unwrap_err().code,
            "channel_unavailable"
        );
    }

    #[test]
    fn xdg_paths_are_rootless_separate_and_normalized() {
        let paths = RouterPaths::from_roots(
            Path::new("/tmp/asb-test-data"),
            Path::new("/tmp/asb-test-state"),
            Path::new("/tmp/asb-test-cache"),
        )
        .unwrap();
        assert!(paths.install_root.ends_with("asb/extensions/asb-tui"));
        assert!(paths.state_root.ends_with("asb/extensions/asb-tui"));
        assert!(paths.cache_root.ends_with("asb/asb-tui"));
        assert_ne!(paths.install_root, paths.cache_root);
        assert!(
            RouterPaths::from_roots(
                Path::new("relative"),
                Path::new("/tmp/s"),
                Path::new("/tmp/c")
            )
            .is_err()
        );
        assert!(
            RouterPaths::from_roots(
                Path::new("/tmp/../escape"),
                Path::new("/tmp/s"),
                Path::new("/tmp/c")
            )
            .is_err()
        );
    }

    #[test]
    fn channel_selection_rejects_expiry_ambiguity_and_mutable_urls() {
        let entry = serde_json::json!({
            "release":"v1.2.3","target":target(),"asb_version":"0.1.0",
            "protocol_version":1,
            "manifest_url":"https://github.com/martin-beck/asb-tui/releases/download/v1.2.3/manifest.json",
            "manifest_signature_url":"https://github.com/martin-beck/asb-tui/releases/download/v1.2.3/manifest.json.sig",
            "manifest_sha256":"a".repeat(64)
        });
        let index = serde_json::json!({
            "schema_version":1,"issued_unix":1_799_999_000u64,
            "expires_unix":1_800_001_000u64,"releases":[entry]
        });
        assert_eq!(
            select_release(
                &serde_json::to_vec(&index).unwrap(),
                1_800_000_000,
                target()
            )
            .unwrap()
            .release,
            "v1.2.3"
        );
        let mut expired = index.clone();
        expired["expires_unix"] = 1_800_000_000u64.into();
        assert!(
            select_release(
                &serde_json::to_vec(&expired).unwrap(),
                1_800_000_000,
                target()
            )
            .is_err()
        );
        let mut duplicate = index.clone();
        duplicate["releases"]
            .as_array_mut()
            .unwrap()
            .push(index["releases"][0].clone());
        assert!(
            select_release(
                &serde_json::to_vec(&duplicate).unwrap(),
                1_800_000_000,
                target()
            )
            .is_err()
        );
        let mut mutable = index;
        mutable["releases"][0]["manifest_url"] =
            "https://github.com/martin-beck/asb-tui/releases/latest/manifest.json".into();
        assert!(
            select_release(
                &serde_json::to_vec(&mutable).unwrap(),
                1_800_000_000,
                target()
            )
            .is_err()
        );
    }

    #[test]
    fn release_ordering_is_numeric_and_lowercase_identities_are_exact() {
        assert!(version_parts("v1.10.0").unwrap() > version_parts("v1.9.9").unwrap());
        assert!(version_parts("v1.0").is_err());
        assert!(version_parts("v01.0.0").is_err());
        assert!(valid_hex(&"a".repeat(40), 40));
        assert!(!valid_hex(&"A".repeat(40), 40));
    }

    #[test]
    fn delegated_response_is_closed_and_classification_consistent() {
        let valid = DelegatedResponse {
            schema_version: 1,
            classification: "verified_extension".into(),
            ok: true,
            code: "verified_installation".into(),
            installed: Some(true),
            verified: Some(true),
            release: Some("v1.0.0".into()),
            executable_sha256: Some("a".repeat(64)),
            target: Some(target().into()),
            bundle: Some(
                if target().starts_with("x86_64") {
                    "asb-tui-v1-linux-x86_64"
                } else {
                    "asb-tui-v1-linux-aarch64"
                }
                .into(),
            ),
            asb_version: Some("0.1.0".into()),
            protocol_version: Some(1),
            source_commit: Some("b".repeat(40)),
            source_tree: Some("c".repeat(40)),
        };
        assert!(validate_delegated(Operation::Status, &valid).is_ok());
        let active = ActiveInstallation {
            schema_version: 1,
            release: valid.release.clone().unwrap(),
            executable_sha256: valid.executable_sha256.clone().unwrap(),
            source_commit: valid.source_commit.clone().unwrap(),
            source_tree: valid.source_tree.clone().unwrap(),
            target: valid.target.clone().unwrap(),
            bundle: valid.bundle.clone().unwrap(),
            asb_version: valid.asb_version.clone().unwrap(),
            protocol_version: 1,
            coordinator_version: "v0.3.5".into(),
            coordinator_commit: "510817b93feb80dde13e5a6c61d657954fae2346".into(),
            quality_version: "v0.23.0".into(),
            quality_commit: "8a9f056b7fc7926b9465a0f7a09225d4da1c572a".into(),
            classification: "verified_extension".into(),
        };
        assert!(delegated_status_matches_active(&valid, &active));
        let mut mismatched = valid.clone();
        mismatched.release = Some("v1.0.1".into());
        assert!(!delegated_status_matches_active(&mismatched, &active));
        let mut invalid = valid;
        invalid.classification = "source_only_unverified".into();
        assert!(validate_delegated(Operation::Status, &invalid).is_err());

        let minimal = DelegatedResponse {
            schema_version: 1,
            classification: "verified_extension".into(),
            ok: true,
            code: "extension_installed".into(),
            installed: None,
            verified: None,
            release: None,
            executable_sha256: None,
            target: None,
            bundle: None,
            asb_version: None,
            protocol_version: None,
            source_commit: None,
            source_tree: None,
        };
        assert!(validate_delegated(Operation::Install, &minimal).is_ok());
        assert!(validate_delegated(Operation::Upgrade, &minimal).is_err());
        let mut source_only = minimal;
        source_only.classification = "source_only_unverified".into();
        assert!(validate_delegated(Operation::Install, &source_only).is_err());
    }

    #[test]
    fn github_release_redirect_is_https_and_host_bounded() {
        let github = "HTTP/1.1 200 Connection established\r\n\r\nHTTP/2 302\r\nlocation: https://release-assets.githubusercontent.com/github-production-release-asset/1/file?sig=abc\r\n\r\n";
        assert_eq!(
            parse_redirect(github).unwrap().unwrap(),
            "https://release-assets.githubusercontent.com/github-production-release-asset/1/file?sig=abc"
        );
        assert!(
            parse_redirect("HTTP/2 302\r\nlocation: https://attacker.invalid/file\r\n\r\n")
                .is_err()
        );
        assert!(
            parse_redirect(
                "HTTP/2 302\r\nlocation: http://release-assets.githubusercontent.com/file\r\n\r\n"
            )
            .is_err()
        );
        assert!(parse_redirect("HTTP/2 302\r\nlocation: https://github.com/a\r\nlocation: https://github.com/b\r\n\r\n").is_err());
        assert!(
            parse_redirect("HTTP/2 200\r\ncontent-length: 1\r\n\r\n")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn durable_rollback_survives_download_cache_removal() {
        let scratch = Scratch::new("rollback");
        let state = scratch.0.join("state");
        let cache = scratch.0.join("cache");
        prepare_private_directory(&state).unwrap();
        prepare_private_directory(&cache).unwrap();
        record_accepted(&state, &selected("v2.0.0", &"a".repeat(64))).unwrap();
        fs::remove_dir_all(&cache).unwrap();
        assert!(enforce_rollback(&state, &selected("v1.9.9", &"b".repeat(64))).is_err());
        assert!(enforce_rollback(&state, &selected("v2.0.0", &"b".repeat(64))).is_err());
        assert!(enforce_rollback(&state, &selected("v2.0.1", &"b".repeat(64))).is_ok());
    }

    #[test]
    fn pending_floor_closes_activation_crash_window() {
        let scratch = Scratch::new("pending-floor");
        let state = scratch.0.join("state");
        prepare_private_directory(&state).unwrap();
        record_accepted(&state, &selected("v2.0.0", &"a".repeat(64))).unwrap();
        record_floor(&state, "pending.json", &selected("v3.0.0", &"b".repeat(64))).unwrap();
        assert!(enforce_rollback(&state, &selected("v2.5.0", &"c".repeat(64))).is_err());
        assert!(enforce_rollback(&state, &selected("v3.0.0", &"b".repeat(64))).is_ok());
    }

    #[test]
    fn retained_directory_operations_reject_symlinks_and_ignore_stale_temps() {
        let scratch = Scratch::new("dirfd");
        let state = scratch.0.join("state");
        prepare_private_directory(&state).unwrap();
        fs::write(state.join(".tmp-stale"), b"stale").unwrap();
        atomic_private(&state.join("accepted.json"), b"{}", 0o600).unwrap();
        assert_eq!(fs::read(state.join("accepted.json")).unwrap(), b"{}");

        let outside = scratch.0.join("outside");
        fs::create_dir(&outside).unwrap();
        symlink(&outside, scratch.0.join("linked")).unwrap();
        assert!(prepare_private_directory(&scratch.0.join("linked/child")).is_err());
        assert!(!environment_path_safe(&scratch.0.join("linked")));
        assert!(!environment_path_safe(Path::new("relative")));
        fs::set_permissions(&outside, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(!environment_path_safe(&outside));
    }

    #[test]
    fn environment_paths_require_private_existing_roots() {
        let scratch = Scratch::new("environment-path");
        let root = scratch.0.join("xdg");
        prepare_private_directory(&root).unwrap();
        assert!(environment_path_safe(&root));
        fs::set_permissions(&root, fs::Permissions::from_mode(0o775)).unwrap();
        assert!(!environment_path_safe(&root));
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let missing = scratch.0.join("missing");
        assert!(environment_path_safe(&missing));
    }

    #[test]
    fn resumable_transfer_retries_from_authenticated_partial_and_rejects_truncation() {
        let scratch = Scratch::new("resume");
        let path = scratch.0.join("artifact");
        atomic_private(&path.with_extension("partial"), b"abc", 0o600).unwrap();
        let artifact = BundleArtifact {
            name: "asb-tui".into(),
            kind: "executable".into(),
            url: "https://github.com/martin-beck/asb-tui/releases/download/v1.0.0/asb-tui".into(),
            size: 6,
            sha256: digest(b"abcdef"),
        };
        let mut resumed = RangeSource {
            values: VecDeque::from([None, Some(b"def".to_vec())]),
            offsets: Vec::new(),
        };
        assert_eq!(
            download_resumable(&artifact, &mut resumed, &path).unwrap(),
            b"abcdef"
        );
        assert_eq!(resumed.offsets, [3, 3]);
        assert!(!path.with_extension("partial").exists());

        let mut truncated = RangeSource {
            values: VecDeque::from([Some(Vec::new()), Some(Vec::new()), Some(Vec::new())]),
            offsets: Vec::new(),
        };
        assert!(download_resumable(&artifact, &mut truncated, &path).is_err());
        assert_eq!(truncated.offsets.len(), TRANSFER_ATTEMPTS);
    }

    #[test]
    fn cached_artifact_at_size_limit_still_requires_exact_length() {
        let scratch = Scratch::new("cached-size-limit");
        let path = scratch.0.join("artifact");
        atomic_private(&path, b"short", 0o600).unwrap();
        assert!(read_exact_digest(&path, MAX_ARTIFACT_BYTES, &digest(b"short")).is_err());
    }

    #[test]
    fn lifecycle_lock_and_unwritable_state_fail_closed() {
        let scratch = Scratch::new("lock-permission");
        let state = scratch.0.join("state");
        prepare_private_directory(&state).unwrap();
        let first = open_lock(&state.join("router.lock")).unwrap();
        first.try_lock().unwrap();
        let second = open_lock(&state.join("router.lock")).unwrap();
        assert!(second.try_lock().is_err());
        drop(second);
        drop(first);

        fs::set_permissions(&state, fs::Permissions::from_mode(0o500)).unwrap();
        assert!(atomic_private(&state.join("unwritable"), b"bytes", 0o600).is_err());
        fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[test]
    fn zlib_is_allowed_only_for_exact_foldhash_release() {
        let valid = policy_documents(
            serde_json::json!({"name":"foldhash","version":"0.2.0","license":"Zlib"}),
        );
        assert!(validate_documents(&policy_manifest(), &valid).is_ok());
        for invalid in [
            serde_json::json!({"name":"other","version":"0.2.0","license":"Zlib"}),
            serde_json::json!({"name":"foldhash","version":"0.3.0","license":"Zlib"}),
            serde_json::json!({"name":"foldhash","version":"0.2.0","license":"GPL-3.0"}),
        ] {
            assert!(validate_documents(&policy_manifest(), &policy_documents(invalid)).is_err());
        }
        let mut omitted = valid.clone();
        let mut sbom: serde_json::Value = serde_json::from_slice(&omitted["sbom"]).unwrap();
        sbom["packages"].as_array_mut().unwrap().pop();
        omitted.insert("sbom".into(), serde_json::to_vec(&sbom).unwrap());
        assert!(validate_documents(&policy_manifest(), &omitted).is_err());

        let mut mismatched = valid;
        let mut sbom: serde_json::Value = serde_json::from_slice(&mismatched["sbom"]).unwrap();
        sbom["packages"][3]["versionInfo"] = "0.2.1".into();
        mismatched.insert("sbom".into(), serde_json::to_vec(&sbom).unwrap());
        assert!(validate_documents(&policy_manifest(), &mismatched).is_err());
    }

    #[test]
    fn component_identity_is_exact_and_binds_tui_executable() {
        let mut manifest = policy_manifest();
        manifest.artifacts.push(BundleArtifact {
            name: "asb-tui".into(),
            kind: "executable".into(),
            url: "https://github.com/martin-beck/asb-tui/releases/download/v1.0.0/asb-tui".into(),
            size: 1,
            sha256: "c".repeat(64),
        });
        manifest.components = vec![
            BundleComponent {
                name: "asb-tui".into(),
                version: "v1.0.0".into(),
                commit: "a".repeat(40),
                tree: "b".repeat(40),
                artifact_sha256: "c".repeat(64),
            },
            BundleComponent {
                name: "agent-workflow-coordinator".into(),
                version: "v0.3.5".into(),
                commit: "510817b93feb80dde13e5a6c61d657954fae2346".into(),
                tree: "41d08ed42333cb47b07c2c401a9167b56c7cfb81".into(),
                artifact_sha256: "a51e58ed71dd93979acc55560fc8208db7131d8e6d0a6b7b805fa383fef25b34"
                    .into(),
            },
            BundleComponent {
                name: "agent-workflow-quality".into(),
                version: "v0.23.0".into(),
                commit: "8a9f056b7fc7926b9465a0f7a09225d4da1c572a".into(),
                tree: "ca77db478f0737142690d37683d826710cd953b0".into(),
                artifact_sha256: "9d480cabe955a5faf88f6f1c8dfd2bfe63045445173c86f93d200a00c97fff89"
                    .into(),
            },
        ];
        assert!(validate_components(&manifest).is_ok());
        manifest.components[0].artifact_sha256 = "d".repeat(64);
        assert!(validate_components(&manifest).is_err());
    }

    #[test]
    fn exact_upstream_bundle_signature_and_schema_bytes_are_pinned() {
        let manifest = include_bytes!("../fixtures/tui/manifest.json");
        let signature = include_bytes!("../fixtures/tui/manifest.json.sig");
        verify_signature(manifest, signature, BUNDLE_NAMESPACE).unwrap();
        assert!(verify_signature(b"tampered", signature, BUNDLE_NAMESPACE).is_err());
        assert_eq!(
            digest(include_bytes!(
                "../schema/tui/v1/lifecycle-request.schema.json"
            )),
            "233a8bae0349cd4e12742005fdefc1c5da876a7826bb742418f0417e6c597730"
        );
        assert_eq!(
            digest(include_bytes!(
                "../schema/tui/v1/lifecycle-response.schema.json"
            )),
            "52f7d909dd085afdf0f74623e2a2c9e8ca7de1764c04e44ebc7d4929e7bae97a"
        );
        assert_eq!(
            digest(include_bytes!(
                "../schema/tui/v1/bundle-manifest.schema.json"
            )),
            "c259ff4fd77f0653d2a34d4aab2cd3e454cceb3e98d4db1ee78403dd59fbf02b"
        );
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn expired_signed_manifest_is_rejected_for_install_but_valid_for_installed_reauthentication() {
        let bytes = include_bytes!("../fixtures/tui/authenticated-active-manifest.json");
        let selected = selected("v0.0.1", &digest(bytes));
        assert!(validate_manifest(bytes, &selected, 1_800_000_000).is_err());
        assert!(validate_installed_manifest(bytes, &selected).is_ok());
    }

    #[test]
    fn synthetic_channel_signature_namespace_and_selection_are_closed() {
        let scratch = Scratch::new("signed-channel");
        let key = scratch.0.join("signer");
        assert!(
            Command::new(SSH_KEYGEN)
                .args(["-q", "-t", "ed25519", "-N", "", "-f"])
                .arg(&key)
                .status()
                .unwrap()
                .success()
        );
        let entry = serde_json::json!({
            "release":"v1.2.3","target":target(),"asb_version":"0.1.0",
            "protocol_version":1,
            "manifest_url":"https://github.com/martin-beck/asb-tui/releases/download/v1.2.3/manifest.json",
            "manifest_signature_url":"https://github.com/martin-beck/asb-tui/releases/download/v1.2.3/manifest.json.sig",
            "manifest_sha256":"a".repeat(64)
        });
        let channel = serde_json::to_vec(&serde_json::json!({
            "schema_version":1,"issued_unix":1_799_999_000u64,
            "expires_unix":1_800_001_000u64,"releases":[entry]
        }))
        .unwrap();
        let channel_path = scratch.0.join("channel.json");
        fs::write(&channel_path, &channel).unwrap();
        assert!(
            Command::new(SSH_KEYGEN)
                .args(["-Y", "sign", "-f"])
                .arg(&key)
                .args(["-n", CHANNEL_NAMESPACE])
                .arg(&channel_path)
                .stdout(Stdio::null())
                .status()
                .unwrap()
                .success()
        );
        let public = fs::read_to_string(key.with_extension("pub")).unwrap();
        let mut fields = public.split_whitespace();
        let allowed = format!(
            "{} {} {}\n",
            SIGNER_IDENTITY,
            fields.next().unwrap(),
            fields.next().unwrap()
        );
        let rendered = Command::new(SSH_KEYGEN)
            .args(["-lf"])
            .arg(key.with_extension("pub"))
            .output()
            .unwrap();
        assert!(rendered.status.success());
        let output = String::from_utf8(rendered.stdout).unwrap();
        let fingerprint = output.split_whitespace().nth(1).unwrap();
        let signature = fs::read(channel_path.with_extension("json.sig")).unwrap();
        verify_signature_with(
            &channel,
            &signature,
            CHANNEL_NAMESPACE,
            allowed.as_bytes(),
            fingerprint,
        )
        .unwrap();
        assert!(
            verify_signature_with(
                &channel,
                &signature,
                BUNDLE_NAMESPACE,
                allowed.as_bytes(),
                fingerprint,
            )
            .is_err()
        );
        assert_eq!(
            select_release(&channel, 1_800_000_000, target())
                .unwrap()
                .release,
            "v1.2.3"
        );
    }

    #[test]
    fn exact_candidate_bytes_emit_one_closed_lifecycle_response() {
        let response = serde_json::json!({
            "schema_version":1,"classification":"verified_extension","ok":true,
            "code":"extension_installed"
        });
        let script = format!("#!/bin/sh\ncat >/dev/null\nprintf '%s\\n' '{}'\n", response);
        let request = serde_json::json!({
            "operation":"install","schema_version":1,"install_root":"/tmp/unused"
        });
        let observed = run_candidate(script.as_bytes(), &request, true).unwrap();
        assert!(observed.ok);
        assert_eq!(observed.code, "extension_installed");
        let hostile = format!("#!/bin/sh\nprintf '%s\\n' '{} trailing'\n", response);
        assert!(run_candidate(hostile.as_bytes(), &request, true).is_err());
    }

    #[test]
    fn candidate_descendant_cannot_retain_the_response_pipe() {
        let mut runner = Command::new("/bin/sleep")
            .arg("30")
            .process_group(0)
            .spawn()
            .unwrap();
        let runner_group = Pid::from_child(&runner);
        let response = serde_json::json!({
            "schema_version":1,"classification":"verified_extension","ok":true,
            "code":"extension_installed"
        });
        let script = format!(
            "#!/bin/sh\n(sleep 30) &\ncat >/dev/null\nprintf '%s\\n' '{}'\n",
            response
        );
        let request = serde_json::json!({
            "operation":"install","schema_version":1,"install_root":"/tmp/unused"
        });
        let started = Instant::now();
        let result = run_candidate(script.as_bytes(), &request, false);
        let runner_survived = runner.try_wait().unwrap().is_none();
        let _ = kill_process_group(runner_group, Signal::KILL);
        let _ = runner.wait();
        let observed = result.unwrap();
        assert!(observed.ok);
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(runner_survived);
    }

    #[test]
    fn delegated_parser_rejects_unknown_duplicate_and_oversized_output() {
        let valid = r#"{"schema_version":1,"classification":"source_only_unverified","ok":false,"code":"extension_not_installed"}"#;
        assert!(serde_json::from_str::<DelegatedResponse>(valid).is_ok());
        for invalid in [
            valid.replace("}", ",\"host\":\"private\"}"),
            valid.replace("\"ok\":false", "\"ok\":false,\"ok\":true"),
        ] {
            assert!(serde_json::from_str::<DelegatedResponse>(&invalid).is_err());
        }
        let script = format!("#!/bin/sh\nprintf '%*s' {} x\n", RESPONSE_BYTES + 1);
        assert!(run_candidate(script.as_bytes(), &serde_json::json!({}), true).is_err());
    }
}
