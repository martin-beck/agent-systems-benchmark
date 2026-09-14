// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Trusted router for the independently installed optional terminal frontend.

use crate::{CliError, output_error, write_json};
use rustix::fs::{
    AtFlags, MemfdFlags, Mode, OFlags, SealFlags, fcntl_add_seals, fcntl_getfl, fcntl_setfl, fsync,
    memfd_create, mkdirat, openat, renameat, unlinkat,
};
use rustix::process::{Pid, Signal, WaitId, WaitIdOptions, kill_process_group, waitid};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
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
const CURL: &str = "/usr/bin/curl";
const SSH_KEYGEN: &str = "/usr/bin/ssh-keygen";
const MAX_REDIRECTS: usize = 3;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Operation {
    Install,
    Upgrade,
    Status,
    Doctor,
    Remove,
    Launch,
    Version,
}

impl Operation {
    const fn name(self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::Upgrade => "upgrade",
            Self::Status => "status",
            Self::Doctor => "doctor",
            Self::Remove => "remove",
            Self::Launch => "launch",
            Self::Version => "version",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Options {
    offline: bool,
    dry_run: bool,
    launch: bool,
}

#[derive(Debug)]
struct RouterError {
    code: &'static str,
    exit_code: u8,
}

impl RouterError {
    const fn policy(code: &'static str) -> Self {
        Self { code, exit_code: 3 }
    }

    const fn operation(code: &'static str) -> Self {
        Self { code, exit_code: 4 }
    }
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct RouterResponse {
    schema_version: u64,
    ok: bool,
    command: &'static str,
    operation: &'static str,
    code: &'static str,
    network: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    release: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    executable_sha256: Option<String>,
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
            release: None,
            executable_sha256: None,
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

pub(crate) fn dispatch(args: &[String], output: &mut dyn Write) -> Result<u8, CliError> {
    let (operation, options) = parse(args)?;
    if operation == Operation::Version {
        writeln!(output, "asb tui-router {}", env!("CARGO_PKG_VERSION")).map_err(output_error)?;
        return Ok(0);
    }
    let mut source = CurlSource;
    let response = match RouterPaths::environment()
        .and_then(|paths| execute(operation, options, &paths, &mut source, system_now_unix()?))
    {
        Ok(response) => response,
        Err(error) => {
            let network = if matches!(operation, Operation::Install | Operation::Upgrade)
                && !options.offline
            {
                "used"
            } else {
                "denied"
            };
            let response = RouterResponse::result(operation, false, error.code, network);
            write_json(output, &response)?;
            return Ok(error.exit_code);
        }
    };
    let exit = if response.ok { 0 } else { 3 };
    write_json(output, &response)?;
    Ok(exit)
}

fn parse(args: &[String]) -> Result<(Operation, Options), CliError> {
    match args {
        [] => Ok((Operation::Launch, Options::default())),
        [single] if single == "launch" => Ok((Operation::Launch, Options::default())),
        [single] if single == "status" => Ok((Operation::Status, Options::default())),
        [single] if single == "doctor" => Ok((Operation::Doctor, Options::default())),
        [single] if single == "remove" => Ok((Operation::Remove, Options::default())),
        [single] if single == "--version" || single == "-V" => {
            Ok((Operation::Version, Options::default()))
        }
        [operation, flags @ ..] if operation == "install" || operation == "upgrade" => {
            let mut options = Options::default();
            for flag in flags {
                let slot = match flag.as_str() {
                    "--offline" => &mut options.offline,
                    "--dry-run" => &mut options.dry_run,
                    "--launch" => &mut options.launch,
                    _ => return Err(CliError::usage("unsupported asb tui arguments")),
                };
                if *slot {
                    return Err(CliError::usage("duplicate asb tui option"));
                }
                *slot = true;
            }
            if options.dry_run && options.launch {
                return Err(CliError::usage(
                    "--dry-run and --launch cannot be used together",
                ));
            }
            Ok((
                if operation == "install" {
                    Operation::Install
                } else {
                    Operation::Upgrade
                },
                options,
            ))
        }
        _ => Err(CliError::usage("unsupported asb tui arguments")),
    }
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
) -> Result<RouterResponse, RouterError> {
    match operation {
        Operation::Install | Operation::Upgrade => {
            install_or_upgrade(operation, options, paths, source, now)
        }
        Operation::Status | Operation::Remove | Operation::Launch => {
            delegate_existing(operation, paths)
        }
        Operation::Doctor => doctor(paths),
        Operation::Version => unreachable!(),
    }
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
    let delegated = run_candidate(&executable, &request, operation != Operation::Launch)?;
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
        release: delegated.release,
        executable_sha256: delegated.executable_sha256,
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
            if bytes.len() > 64
                || !bytes
                    .iter()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, 43 | 45 | 46 | 95))
            {
                return Err(RouterError::policy("environment_terminal_invalid"));
            }
            command.env(name, value);
        }
    }
    Ok(())
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
            Operation::Launch => "frontend_exited",
            Operation::Status => "verified_installation",
            Operation::Doctor | Operation::Version => {
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
    use std::collections::VecDeque;
    use std::os::unix::fs::DirBuilderExt;
    use std::os::unix::fs::symlink;
    use std::sync::atomic::{AtomicU64, Ordering};

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
        assert!(parse(&["render".into()]).is_err());
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
