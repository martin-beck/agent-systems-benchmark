// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Fail-closed contracts for the optional CSB subprocess boundary.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::Read;
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};

use asb_protocol::{CallLimits, Id, PROTOCOL_V1, ProtocolVersion};
use asb_runtime::sandbox::{
    NetworkPolicy, ResourceLease, Resources, SandboxBackend, SandboxError, SandboxProcess,
    SandboxSpec,
};
use asb_runtime::{ProcessLimits, ProcessOutput, Termination};
use asb_store::{
    AtomicStore, ExecutionState, JOURNAL_SCHEMA_VERSION, JournalEvent, RecoveryDecision,
    StoreError, StoreLimits,
};
use rustix::fs::{Mode, OFlags, open, openat};
use rustix::io::{FdFlags, fcntl_setfd};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Contract generation implemented by this crate.
pub const CSB_CONTRACT_V1: ProtocolVersion = PROTOCOL_V1;
/// Maximum count of arguments in one request.
pub const MAX_ARGUMENTS: usize = 128;
/// Maximum combined UTF-8 bytes of all arguments.
pub const MAX_ARGUMENT_BYTES: usize = 32 * 1024;
/// Maximum count of declared output artifacts.
pub const MAX_ARTIFACTS: usize = 256;
/// Maximum byte length of any single identifier or relative path.
pub const MAX_COMPONENT_BYTES: usize = 1024;
/// Maximum operation identities retained by one negotiated subprocess session.
pub const MAX_SESSION_OPERATIONS: usize = 4096;
/// Exact public CSB revision admitted by generation one.
pub const CSB_SOURCE_COMMIT: &str = "d577c5249501b29e33a87524a677a101477d5579";
/// Exact Git tree admitted by generation one.
pub const CSB_SOURCE_TREE: &str = "97d08b39026f7c7d3e6f748b26a20e819a92184f";
/// Maximum bytes accepted for a pinned executable or one output artifact.
pub const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;

/// Only the independently reviewed CSB execution mode is admitted.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    /// Execute an already prepared external application without a nested scheduler.
    ExternalApplication,
}

/// Immutable public identities that must match the bytes used for execution.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CsbPin {
    /// Exact public CSB Git commit.
    pub source_commit: String,
    /// Exact Git tree for that commit.
    pub source_tree: String,
    /// SHA-256 of the executable file supplied to the sandbox.
    pub executable_sha256: String,
}

/// One bounded CSB invocation transported inside an ASB JSON-RPC request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunRequest {
    /// Contract version selected during negotiation.
    pub contract: ProtocolVersion,
    /// Stable causal identity for this effect.
    pub operation_id: String,
    /// Stable attempt identity used to fence stale retries.
    pub attempt_id: String,
    /// Immutable source and executable identities.
    pub pin: CsbPin,
    /// CSB operation admitted by this boundary.
    pub mode: ExecutionMode,
    /// Direct executable arguments; never interpreted by a shell.
    pub arguments: Vec<String>,
    /// Explicit non-secret environment passed after clearing the ambient environment.
    pub environment: BTreeMap<String, String>,
    /// Relative regular-file outputs that may be committed after verification.
    pub artifacts: Vec<String>,
}

/// Validated request whose private fields cannot be changed after admission.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedRun {
    request: RunRequest,
    request_sha256: String,
}

impl ValidatedRun {
    /// Return the immutable request.
    pub fn request(&self) -> &RunRequest {
        &self.request
    }

    /// Return the canonical JSON digest bound to durable pre-effect intent.
    pub fn request_sha256(&self) -> &str {
        &self.request_sha256
    }
}

/// Why a request was rejected before any external effect.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ValidationError {
    /// The request did not use the negotiated v1 contract.
    #[error("unsupported CSB contract version")]
    Version,
    /// A causal identity was empty, oversized, or unsafe.
    #[error("invalid {0} identity")]
    Identity(&'static str),
    /// A Git or executable digest was not lowercase hexadecimal of the exact size.
    #[error("invalid {0} digest")]
    Digest(&'static str),
    /// Argument count or encoded size exceeded the boundary.
    #[error("argument boundary exceeded")]
    Arguments,
    /// An environment name/value was not explicitly safe and bounded.
    #[error("invalid environment")]
    Environment,
    /// An output path was absolute, escaping, duplicated, or oversized.
    #[error("invalid artifact path")]
    Artifact,
    /// Canonical request serialization unexpectedly failed.
    #[error("request serialization failed")]
    Serialization,
}

/// A bounded protocol-session admission failure before any external effect.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum AdmissionError {
    /// A run was offered before the mandatory negotiation exchange.
    #[error("CSB protocol negotiation is required")]
    NegotiationRequired,
    /// A peer attempted to renegotiate a live session.
    #[error("CSB protocol is already negotiated")]
    AlreadyNegotiated,
    /// The peer offered an incompatible protocol generation.
    #[error("unsupported CSB protocol version")]
    Version,
    /// Either peer offered invalid call limits.
    #[error("invalid CSB protocol limits")]
    Limits,
    /// The in-flight or lifetime session bound is exhausted.
    #[error("CSB protocol capacity exhausted")]
    Capacity,
    /// The exact operation and attempt identity has already been admitted.
    #[error("duplicate CSB operation")]
    Duplicate,
    /// An operation identity was reused for a different attempt.
    #[error("stale CSB attempt")]
    StaleAttempt,
    /// Completion did not name an active operation.
    #[error("unknown CSB operation")]
    UnknownOperation,
    /// The request body violated the execution contract.
    #[error(transparent)]
    Validation(#[from] ValidationError),
}

/// Negotiation and causal-ID fence for one bounded subprocess session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundarySession {
    local_limits: CallLimits,
    negotiated: Option<CallLimits>,
    active: BTreeMap<String, String>,
    admitted: BTreeSet<(String, String)>,
}

impl BoundarySession {
    /// Create a session with validated local hard limits.
    pub fn new(local_limits: CallLimits) -> Result<Self, AdmissionError> {
        local_limits
            .validate()
            .map_err(|_| AdmissionError::Limits)?;
        Ok(Self {
            local_limits,
            negotiated: None,
            active: BTreeMap::new(),
            admitted: BTreeSet::new(),
        })
    }

    /// Select the common v1 protocol and the stricter peer limits exactly once.
    pub fn negotiate(
        &mut self,
        protocol: ProtocolVersion,
        peer_limits: CallLimits,
    ) -> Result<CallLimits, AdmissionError> {
        if self.negotiated.is_some() {
            return Err(AdmissionError::AlreadyNegotiated);
        }
        protocol.negotiate().map_err(|_| AdmissionError::Version)?;
        let limits = self
            .local_limits
            .intersect(peer_limits)
            .map_err(|_| AdmissionError::Limits)?;
        self.negotiated = Some(limits);
        Ok(limits)
    }

    /// Validate and reserve one causal operation before durable execution intent.
    pub fn admit(&mut self, request: RunRequest) -> Result<ValidatedRun, AdmissionError> {
        let limits = self.negotiated.ok_or(AdmissionError::NegotiationRequired)?;
        let key = (request.operation_id.clone(), request.attempt_id.clone());
        if let Some(active_attempt) = self.active.get(&request.operation_id) {
            return Err(if active_attempt == &request.attempt_id {
                AdmissionError::Duplicate
            } else {
                AdmissionError::StaleAttempt
            });
        }
        if self.admitted.contains(&key) {
            return Err(AdmissionError::Duplicate);
        }
        if self.active.len() >= usize::from(limits.max_in_flight)
            || self.admitted.len() >= MAX_SESSION_OPERATIONS
        {
            return Err(AdmissionError::Capacity);
        }
        let validated = request.validate(limits)?;
        self.active.insert(key.0.clone(), key.1.clone());
        self.admitted.insert(key);
        Ok(validated)
    }

    /// Release exactly the active causal identity after terminal evidence is durable.
    pub fn finish(&mut self, operation_id: &str, attempt_id: &str) -> Result<(), AdmissionError> {
        match self.active.get(operation_id) {
            Some(active_attempt) if active_attempt == attempt_id => {
                self.active.remove(operation_id);
                Ok(())
            }
            Some(_) => Err(AdmissionError::StaleAttempt),
            None => Err(AdmissionError::UnknownOperation),
        }
    }

    /// Return negotiated limits, when negotiation has completed.
    pub fn negotiated_limits(&self) -> Option<CallLimits> {
        self.negotiated
    }

    /// Return the count of operations awaiting durable terminal evidence.
    pub fn active_operations(&self) -> usize {
        self.active.len()
    }

    fn rollback_before_effect(&mut self, operation_id: &str, attempt_id: &str) {
        self.active.remove(operation_id);
        self.admitted
            .remove(&(operation_id.to_owned(), attempt_id.to_owned()));
    }
}

/// Failure at the durable, filesystem, or containment execution boundary.
#[derive(Debug, Error)]
pub enum ExecutionError {
    /// Protocol admission failed before execution.
    #[error("CSB request admission failed: {0}")]
    Admission(#[from] AdmissionError),
    /// Workspace, state, executable, or artifact topology was unsafe.
    #[error("CSB filesystem boundary rejected")]
    Filesystem,
    /// Executable bytes did not match the admitted digest.
    #[error("CSB executable identity mismatch")]
    ExecutableIdentity,
    /// Durable state did not permit a new execution effect.
    #[error("CSB durable state requires reconciliation")]
    DurableState,
    /// Durable journal persistence failed.
    #[error("CSB durable store failed: {0}")]
    Store(#[from] StoreError),
    /// The rootless sandbox failed or left uncertain cleanup.
    #[error("CSB sandbox failed: {0}")]
    Sandbox(#[from] SandboxError),
}

mod sealed {
    pub trait Sealed {}
}

/// Closed interface implemented only by ASB-owned contained process handles.
pub trait ContainedChild: sealed::Sealed {
    /// Request idempotent whole-scope cancellation.
    fn cancel(&mut self) -> Result<(), SandboxError>;
    /// Reap the full contained scope and return bounded terminal evidence.
    fn wait(&mut self) -> Result<ProcessOutput, SandboxError>;
}

impl sealed::Sealed for SandboxProcess {}

impl ContainedChild for SandboxProcess {
    fn cancel(&mut self) -> Result<(), SandboxError> {
        SandboxProcess::cancel(self)
    }

    fn wait(&mut self) -> Result<ProcessOutput, SandboxError> {
        SandboxProcess::wait(self)
    }
}

trait Launcher {
    type Child: ContainedChild;

    fn spawn(
        &self,
        spec: SandboxSpec,
        lease: ResourceLease,
        limits: ProcessLimits,
    ) -> Result<Self::Child, SandboxError>;
}

impl Launcher for SandboxBackend {
    type Child = SandboxProcess;

    fn spawn(
        &self,
        spec: SandboxSpec,
        lease: ResourceLease,
        limits: ProcessLimits,
    ) -> Result<Self::Child, SandboxError> {
        SandboxBackend::spawn(self, spec, lease, limits)
    }
}

/// Production CSB launcher backed only by the reviewed ASB sandbox runtime.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CsbExecutor {
    backend: SandboxBackend,
}

impl CsbExecutor {
    /// Bind execution to an exact-pinned sandbox backend.
    pub fn new(backend: SandboxBackend) -> Self {
        Self { backend }
    }

    /// Persist execution intent, then spawn one verified offline process.
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        &self,
        session: &mut BoundarySession,
        request: RunRequest,
        workspace: &Path,
        state_root: &Path,
        program: &str,
        working_directory: PathBuf,
        resources: Resources,
        lease: ResourceLease,
        process_limits: ProcessLimits,
        store_limits: StoreLimits,
        run_id: &str,
        monotonic_offset_ns: u64,
    ) -> Result<RunningCsb, ExecutionError> {
        start_with(
            &self.backend,
            session,
            request,
            workspace,
            state_root,
            program,
            working_directory,
            resources,
            lease,
            process_limits,
            store_limits,
            run_id,
            monotonic_offset_ns,
        )
    }
}

/// One contained CSB process whose terminal evidence is not yet durable.
pub struct RunningCsb<C: ContainedChild = SandboxProcess> {
    child: C,
    store: AtomicStore,
    run_id: String,
    operation_id: String,
    attempt_id: String,
    artifacts: Vec<String>,
    workspace: PathBuf,
    sequence: u64,
    monotonic_offset_ns: u64,
}

impl<C: ContainedChild> RunningCsb<C> {
    /// Request idempotent whole-scope cancellation.
    pub fn cancel(&mut self) -> Result<(), ExecutionError> {
        self.child.cancel().map_err(ExecutionError::Sandbox)
    }

    /// Reap the scope, commit bounded artifacts, and persist terminal evidence.
    pub fn wait(mut self, session: &mut BoundarySession) -> Result<ProcessOutput, ExecutionError> {
        finish_with(
            &mut self.child,
            &self.store,
            &self.run_id,
            &self.attempt_id,
            &self.artifacts,
            &self.workspace,
            self.sequence,
            self.monotonic_offset_ns,
        )
        .and_then(|output| {
            session.finish(&self.operation_id, &self.attempt_id)?;
            Ok(output)
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn start_with<L: Launcher>(
    launcher: &L,
    session: &mut BoundarySession,
    request: RunRequest,
    workspace: &Path,
    state_root: &Path,
    program: &str,
    working_directory: PathBuf,
    resources: Resources,
    lease: ResourceLease,
    process_limits: ProcessLimits,
    store_limits: StoreLimits,
    run_id: &str,
    monotonic_offset_ns: u64,
) -> Result<RunningCsb<L::Child>, ExecutionError> {
    let workspace = canonical_directory(workspace)?;
    let state_root = canonical_directory(state_root)?;
    if workspace.starts_with(&state_root) || state_root.starts_with(&workspace) {
        return Err(ExecutionError::Filesystem);
    }
    let store = AtomicStore::open(&state_root, store_limits)?;
    let verified_program = verify_regular_at(&workspace, program, &request.pin.executable_sha256)?;
    let manifest = store.load_manifest(run_id)?;
    if manifest.attempt_id.0 != request.attempt_id
        || store.recovery_decision(run_id)? != RecoveryDecision::Resume(ExecutionState::Prepared)
    {
        return Err(ExecutionError::DurableState);
    }

    let operation_id = request.operation_id.clone();
    let attempt_id = request.attempt_id.clone();
    let artifacts = request.artifacts.clone();
    let arguments = request.arguments.clone();
    let environment = request.environment.clone();
    // The sandbox executes the already-open inode, not a path that can be
    // replaced between identity verification and exec. The descriptor holds
    // only reviewed public executable bytes and is inheritable solely across
    // this synchronous systemd-run --scope / Bubblewrap launch chain.
    let sandbox_program = format!("/proc/self/fd/{}", verified_program.as_raw_fd());
    let spec = SandboxSpec::new(
        &workspace,
        working_directory,
        sandbox_program,
        arguments,
        environment,
        resources,
        NetworkPolicy::Deny,
    )
    .map_err(|_| ExecutionError::Filesystem)?;
    let validated = session.admit(request)?;
    let journal = store.load_journal(run_id)?;
    let sequence = u64::try_from(journal.len()).map_err(|_| ExecutionError::DurableState)?;
    let collecting_sequence = sequence
        .checked_add(1)
        .ok_or(ExecutionError::DurableState)?;
    collecting_sequence
        .checked_add(1)
        .ok_or(ExecutionError::DurableState)?;
    let running = JournalEvent {
        schema_version: JOURNAL_SCHEMA_VERSION,
        sequence,
        attempt_id: Id(attempt_id.clone()),
        monotonic_offset_ns,
        state: ExecutionState::Running,
        evidence: json!({
            "request_sha256": validated.request_sha256(),
            "executable_sha256": validated.request().pin.executable_sha256,
        }),
    };
    if let Err(error) = store.append(run_id, &running) {
        session.rollback_before_effect(&operation_id, &attempt_id);
        return Err(ExecutionError::Store(error));
    }

    fcntl_setfd(&verified_program, FdFlags::empty()).map_err(|_| ExecutionError::Filesystem)?;
    let child = launcher.spawn(spec, lease, process_limits);
    let restored = fcntl_setfd(&verified_program, FdFlags::CLOEXEC);
    if restored.is_err() {
        return Err(ExecutionError::Filesystem);
    }
    let child = child?;
    Ok(RunningCsb {
        child,
        store,
        run_id: run_id.to_owned(),
        operation_id,
        attempt_id,
        artifacts,
        workspace,
        sequence: collecting_sequence,
        monotonic_offset_ns,
    })
}

#[allow(clippy::too_many_arguments)]
fn finish_with<C: ContainedChild>(
    child: &mut C,
    store: &AtomicStore,
    run_id: &str,
    attempt_id: &str,
    artifacts: &[String],
    workspace: &Path,
    sequence: u64,
    monotonic_offset_ns: u64,
) -> Result<ProcessOutput, ExecutionError> {
    let output = child.wait()?;
    let termination = match output.termination {
        Termination::Exited => "exited",
        Termination::Cancelled => "cancelled",
        Termination::TimedOut => "timed_out",
    };
    let elapsed_ns =
        u64::try_from(output.elapsed.as_nanos()).map_err(|_| ExecutionError::DurableState)?;
    let finished_offset_ns = monotonic_offset_ns
        .checked_add(elapsed_ns)
        .ok_or(ExecutionError::DurableState)?;
    let collecting = JournalEvent {
        schema_version: JOURNAL_SCHEMA_VERSION,
        sequence,
        attempt_id: Id(attempt_id.to_owned()),
        monotonic_offset_ns: finished_offset_ns,
        state: ExecutionState::Collecting,
        evidence: json!({
            "exit_code": output.exit_code,
            "signal": output.signal,
            "termination": termination,
            "stdout_bytes": output.stdout.total_bytes,
            "stdout_truncated": output.stdout.truncated,
            "stderr_bytes": output.stderr.total_bytes,
            "stderr_truncated": output.stderr.truncated,
        }),
    };
    store.append(run_id, &collecting)?;

    let mut committed = Vec::with_capacity(artifacts.len());
    if output.termination == Termination::Exited && output.exit_code == Some(0) {
        for artifact in artifacts {
            let mut file = open_regular_at(workspace, artifact)?;
            committed.push(store.put_artifact(run_id, artifact, &mut file)?);
        }
    }
    let terminal = match output.termination {
        Termination::Cancelled => ExecutionState::Cancelled,
        Termination::Exited if output.exit_code == Some(0) => ExecutionState::Completed,
        Termination::Exited | Termination::TimedOut => ExecutionState::Failed,
    };
    store.append(
        run_id,
        &JournalEvent {
            schema_version: JOURNAL_SCHEMA_VERSION,
            sequence: sequence
                .checked_add(1)
                .ok_or(ExecutionError::DurableState)?,
            attempt_id: Id(attempt_id.to_owned()),
            monotonic_offset_ns: finished_offset_ns,
            state: terminal,
            evidence: json!({ "artifacts": committed }),
        },
    )?;
    Ok(output)
}

fn canonical_directory(path: &Path) -> Result<PathBuf, ExecutionError> {
    if !path.is_absolute() {
        return Err(ExecutionError::Filesystem);
    }
    let mut inspected = PathBuf::from("/");
    for component in path.components() {
        match component {
            Component::RootDir => continue,
            Component::Normal(name) => inspected.push(name),
            Component::CurDir | Component::ParentDir | Component::Prefix(_) => {
                return Err(ExecutionError::Filesystem);
            }
        }
        let metadata =
            std::fs::symlink_metadata(&inspected).map_err(|_| ExecutionError::Filesystem)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(ExecutionError::Filesystem);
        }
    }
    let canonical = std::fs::canonicalize(path).map_err(|_| ExecutionError::Filesystem)?;
    if canonical != inspected || !canonical.is_dir() {
        return Err(ExecutionError::Filesystem);
    }
    Ok(canonical)
}

fn verify_regular_at(
    root: &Path,
    name: &str,
    expected_sha256: &str,
) -> Result<File, ExecutionError> {
    let mut file = open_regular_at(root, name)?;
    if file
        .metadata()
        .map_err(|_| ExecutionError::Filesystem)?
        .permissions()
        .mode()
        & 0o111
        == 0
    {
        return Err(ExecutionError::Filesystem);
    }
    let mut bytes = Vec::new();
    file.by_ref()
        .take(MAX_FILE_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| ExecutionError::Filesystem)?;
    if bytes.len() as u64 > MAX_FILE_BYTES
        || format!("{:x}", Sha256::digest(&bytes)) != expected_sha256
    {
        return Err(ExecutionError::ExecutableIdentity);
    }
    Ok(file)
}

fn open_regular_at(root: &Path, name: &str) -> Result<File, ExecutionError> {
    let path = Path::new(name);
    if path.components().count() != 1
        || !matches!(path.components().next(), Some(Component::Normal(_)))
    {
        return Err(ExecutionError::Filesystem);
    }
    let directory = open(
        root,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| ExecutionError::Filesystem)?;
    let descriptor = openat(
        &directory,
        path,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| ExecutionError::Filesystem)?;
    let file = File::from(descriptor);
    let metadata = file.metadata().map_err(|_| ExecutionError::Filesystem)?;
    if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
        return Err(ExecutionError::Filesystem);
    }
    Ok(file)
}

impl RunRequest {
    /// Validate all fields before durable intent or process creation.
    pub fn validate(self, negotiated: CallLimits) -> Result<ValidatedRun, ValidationError> {
        if self.contract != CSB_CONTRACT_V1 || negotiated.validate().is_err() {
            return Err(ValidationError::Version);
        }
        validate_identity("operation", &self.operation_id)?;
        validate_identity("attempt", &self.attempt_id)?;
        if !is_hex(&self.pin.source_commit, 40) || self.pin.source_commit != CSB_SOURCE_COMMIT {
            return Err(ValidationError::Digest("source commit"));
        }
        if !is_hex(&self.pin.source_tree, 40) || self.pin.source_tree != CSB_SOURCE_TREE {
            return Err(ValidationError::Digest("source tree"));
        }
        if !is_hex(&self.pin.executable_sha256, 64) {
            return Err(ValidationError::Digest("executable"));
        }
        validate_arguments(&self.arguments)?;
        validate_environment(&self.environment)?;
        validate_artifacts(&self.artifacts)?;

        let encoded = serde_json::to_vec(&self).map_err(|_| ValidationError::Serialization)?;
        if encoded.len() > negotiated.max_frame_bytes as usize {
            return Err(ValidationError::Arguments);
        }
        let request_sha256 = format!("{:x}", Sha256::digest(&encoded));
        Ok(ValidatedRun {
            request: self,
            request_sha256,
        })
    }
}

fn validate_identity(kind: &'static str, value: &str) -> Result<(), ValidationError> {
    if value.is_empty()
        || value.len() > MAX_COMPONENT_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(ValidationError::Identity(kind));
    }
    Ok(())
}

fn is_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate_arguments(arguments: &[String]) -> Result<(), ValidationError> {
    let size = arguments.iter().try_fold(0_usize, |total, argument| {
        if argument.as_bytes().contains(&0) || argument.len() > MAX_COMPONENT_BYTES {
            None
        } else {
            total.checked_add(argument.len())
        }
    });
    if arguments.len() > MAX_ARGUMENTS || size.is_none_or(|size| size > MAX_ARGUMENT_BYTES) {
        return Err(ValidationError::Arguments);
    }
    Ok(())
}

fn validate_environment(environment: &BTreeMap<String, String>) -> Result<(), ValidationError> {
    const ALLOWED: &[&str] = &["CSB_RUN_ID", "CSB_SEED", "LANG", "LC_ALL", "TZ"];
    if environment.iter().any(|(name, value)| {
        !ALLOWED.contains(&name.as_str())
            || value.len() > MAX_COMPONENT_BYTES
            || value.as_bytes().contains(&0)
    }) {
        return Err(ValidationError::Environment);
    }
    Ok(())
}

fn validate_artifacts(artifacts: &[String]) -> Result<(), ValidationError> {
    if artifacts.len() > MAX_ARTIFACTS {
        return Err(ValidationError::Artifact);
    }
    let mut prior: Option<&str> = None;
    for artifact in artifacts {
        let path = Path::new(artifact);
        if artifact.is_empty()
            || artifact.len() > MAX_COMPONENT_BYTES
            || path.is_absolute()
            || path.components().count() != 1
            || !matches!(path.components().next(), Some(Component::Normal(_)))
            || prior.is_some_and(|value| value >= artifact.as_str())
        {
            return Err(ValidationError::Artifact);
        }
        prior = Some(artifact);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use asb_runtime::sandbox::{CpuSet, LeaseClass};
    use asb_runtime::{CapturedStream, Termination};
    use asb_store::{MANIFEST_SCHEMA_VERSION, RunManifest};
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::time::Duration;
    use tempfile::TempDir;

    struct FakeChild {
        output: ProcessOutput,
        lease: Option<ResourceLease>,
    }

    impl sealed::Sealed for FakeChild {}

    impl ContainedChild for FakeChild {
        fn cancel(&mut self) -> Result<(), SandboxError> {
            self.output.termination = Termination::Cancelled;
            self.output.exit_code = None;
            Ok(())
        }

        fn wait(&mut self) -> Result<ProcessOutput, SandboxError> {
            self.lease.take();
            Ok(self.output.clone())
        }
    }

    struct FakeLauncher {
        output: ProcessOutput,
        fail_spawn: bool,
    }

    impl Launcher for FakeLauncher {
        type Child = FakeChild;

        fn spawn(
            &self,
            _spec: SandboxSpec,
            lease: ResourceLease,
            _limits: ProcessLimits,
        ) -> Result<Self::Child, SandboxError> {
            if self.fail_spawn {
                return Err(SandboxError::DelegationRejected);
            }
            Ok(FakeChild {
                output: self.output.clone(),
                lease: Some(lease),
            })
        }
    }

    fn limits() -> CallLimits {
        CallLimits {
            max_frame_bytes: 64 * 1024,
            timeout_ms: 1_000,
            max_in_flight: 1,
        }
    }

    fn request() -> RunRequest {
        RunRequest {
            contract: CSB_CONTRACT_V1,
            operation_id: "operation-1".into(),
            attempt_id: "attempt-1".into(),
            pin: CsbPin {
                source_commit: "d577c5249501b29e33a87524a677a101477d5579".into(),
                source_tree: "97d08b39026f7c7d3e6f748b26a20e819a92184f".into(),
                executable_sha256:
                    "e1228516fe0648db35cfb7b6669b09f25dc32f62691c8fc8cca4ee1e935eee88".into(),
            },
            mode: ExecutionMode::ExternalApplication,
            arguments: vec!["--json".into()],
            environment: BTreeMap::from([("TZ".into(), "UTC".into())]),
            artifacts: vec!["result.json".into()],
        }
    }

    fn output(termination: Termination, exit_code: Option<i32>) -> ProcessOutput {
        ProcessOutput {
            exit_code,
            signal: None,
            termination,
            stdout: CapturedStream {
                bytes: b"public summary".to_vec(),
                total_bytes: 14,
                truncated: false,
            },
            stderr: CapturedStream {
                bytes: Vec::new(),
                total_bytes: 0,
                truncated: false,
            },
            elapsed: Duration::from_millis(2),
        }
    }

    fn executable(workspace: &Path, bytes: &[u8]) -> String {
        let path = workspace.join("fixture");
        fs::write(&path, bytes).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        format!("{:x}", Sha256::digest(bytes))
    }

    fn prepared_store(root: &Path, value: &RunRequest) -> AtomicStore {
        let store = AtomicStore::open(root, StoreLimits::default()).unwrap();
        store
            .create_run(&RunManifest {
                schema_version: MANIFEST_SCHEMA_VERSION,
                run_id: Id("run-1".into()),
                attempt_id: Id(value.attempt_id.clone()),
                definition: json!({"kind": "csb"}),
            })
            .unwrap();
        for (sequence, state) in [ExecutionState::Planned, ExecutionState::Prepared]
            .into_iter()
            .enumerate()
        {
            store
                .append(
                    "run-1",
                    &JournalEvent {
                        schema_version: JOURNAL_SCHEMA_VERSION,
                        sequence: sequence as u64,
                        attempt_id: Id(value.attempt_id.clone()),
                        monotonic_offset_ns: sequence as u64,
                        state,
                        evidence: json!({}),
                    },
                )
                .unwrap();
        }
        store
    }

    fn append_running(store: &AtomicStore, attempt_id: &str) {
        store
            .append(
                "run-1",
                &JournalEvent {
                    schema_version: JOURNAL_SCHEMA_VERSION,
                    sequence: 2,
                    attempt_id: Id(attempt_id.into()),
                    monotonic_offset_ns: 2,
                    state: ExecutionState::Running,
                    evidence: json!({}),
                },
            )
            .unwrap();
    }

    fn resources_and_lease(root: &Path) -> (Resources, ResourceLease) {
        let cpus = CpuSet::new(vec![0]).unwrap();
        let resources = Resources::new(64 * 1024 * 1024, 8, 100, cpus.clone()).unwrap();
        let lease = ResourceLease::acquire(root, LeaseClass::Benchmark, cpus).unwrap();
        (resources, lease)
    }

    fn process_limits() -> ProcessLimits {
        ProcessLimits::new(
            4096,
            4096,
            Duration::from_secs(2),
            Duration::from_millis(10),
            Duration::from_millis(1),
        )
        .unwrap()
    }

    #[test]
    fn accepts_bounded_pinned_external_application() {
        let validated = request().validate(limits()).unwrap();
        assert_eq!(validated.request(), &request());
        assert_eq!(validated.request_sha256().len(), 64);
    }

    #[test]
    fn rejects_unknown_contract_and_invalid_limits() {
        let mut value = request();
        value.contract.major = 2;
        assert_eq!(value.validate(limits()), Err(ValidationError::Version));
        let mut invalid = limits();
        invalid.max_frame_bytes = 0;
        assert_eq!(request().validate(invalid), Err(ValidationError::Version));
    }

    #[test]
    fn rejects_untrusted_identity_and_pin_shapes() {
        let mut value = request();
        value.operation_id = "../effect".into();
        assert_eq!(
            value.validate(limits()),
            Err(ValidationError::Identity("operation"))
        );
        let mut value = request();
        value.pin.executable_sha256 = "A".repeat(64);
        assert_eq!(
            value.validate(limits()),
            Err(ValidationError::Digest("executable"))
        );
    }

    #[test]
    fn rejects_argument_floods_and_nul() {
        let mut value = request();
        value.arguments = vec!["x".into(); MAX_ARGUMENTS + 1];
        assert_eq!(value.validate(limits()), Err(ValidationError::Arguments));
        let mut value = request();
        value.arguments = vec!["bad\0argument".into()];
        assert_eq!(value.validate(limits()), Err(ValidationError::Arguments));
    }

    #[test]
    fn rejects_credentials_and_ambient_configuration() {
        for name in ["HOME", "PATH", "AWS_ACCESS_KEY_ID", "GITHUB_TOKEN"] {
            let mut value = request();
            value.environment = BTreeMap::from([(name.into(), "public-sentinel".into())]);
            assert_eq!(value.validate(limits()), Err(ValidationError::Environment));
        }
    }

    #[test]
    fn rejects_escaping_duplicate_and_unsorted_artifacts() {
        for artifacts in [
            vec!["../secret".into()],
            vec!["/absolute".into()],
            vec!["same".into(), "same".into()],
            vec!["z".into(), "a".into()],
        ] {
            let mut value = request();
            value.artifacts = artifacts;
            assert_eq!(value.validate(limits()), Err(ValidationError::Artifact));
        }
    }

    #[test]
    fn serde_rejects_unknown_fields_and_modes() {
        let mut json = serde_json::to_value(request()).unwrap();
        json.as_object_mut()
            .unwrap()
            .insert("secret".into(), true.into());
        assert!(serde_json::from_value::<RunRequest>(json).is_err());
        let json = serde_json::to_string(&request())
            .unwrap()
            .replace("external_application", "generator");
        assert!(serde_json::from_str::<RunRequest>(&json).is_err());
    }

    #[test]
    fn negotiation_is_mandatory_single_use_and_strict() {
        let mut session = BoundarySession::new(limits()).unwrap();
        assert_eq!(
            session.admit(request()),
            Err(AdmissionError::NegotiationRequired)
        );
        let peer = CallLimits {
            max_frame_bytes: 4096,
            timeout_ms: 500,
            max_in_flight: 1,
        };
        assert_eq!(session.negotiate(CSB_CONTRACT_V1, peer), Ok(peer));
        assert_eq!(session.negotiated_limits(), Some(peer));
        assert_eq!(
            session.negotiate(CSB_CONTRACT_V1, peer),
            Err(AdmissionError::AlreadyNegotiated)
        );

        let mut wrong = BoundarySession::new(limits()).unwrap();
        assert_eq!(
            wrong.negotiate(ProtocolVersion { major: 2, minor: 0 }, peer),
            Err(AdmissionError::Version)
        );
        let mut invalid = BoundarySession::new(limits()).unwrap();
        assert_eq!(
            invalid.negotiate(
                CSB_CONTRACT_V1,
                CallLimits {
                    max_frame_bytes: 0,
                    ..peer
                }
            ),
            Err(AdmissionError::Limits)
        );
        assert_eq!(
            BoundarySession::new(CallLimits {
                max_in_flight: 0,
                ..limits()
            }),
            Err(AdmissionError::Limits)
        );
    }

    #[test]
    fn duplicate_stale_and_unknown_operations_fail_closed() {
        let mut session = BoundarySession::new(limits()).unwrap();
        session.negotiate(CSB_CONTRACT_V1, limits()).unwrap();
        session.admit(request()).unwrap();
        assert_eq!(session.active_operations(), 1);
        assert_eq!(session.admit(request()), Err(AdmissionError::Duplicate));
        let mut stale = request();
        stale.attempt_id = "attempt-2".into();
        assert_eq!(session.admit(stale), Err(AdmissionError::StaleAttempt));
        assert_eq!(
            session.finish("operation-1", "attempt-2"),
            Err(AdmissionError::StaleAttempt)
        );
        session.finish("operation-1", "attempt-1").unwrap();
        assert_eq!(session.active_operations(), 0);
        assert_eq!(session.admit(request()), Err(AdmissionError::Duplicate));
        assert_eq!(
            session.finish("missing", "attempt-1"),
            Err(AdmissionError::UnknownOperation)
        );
    }

    #[test]
    fn negotiated_in_flight_capacity_is_enforced() {
        let mut session = BoundarySession::new(limits()).unwrap();
        session.negotiate(CSB_CONTRACT_V1, limits()).unwrap();
        session.admit(request()).unwrap();
        let mut second = request();
        second.operation_id = "operation-2".into();
        second.attempt_id = "attempt-2".into();
        assert_eq!(session.admit(second.clone()), Err(AdmissionError::Capacity));
        session.finish("operation-1", "attempt-1").unwrap();
        assert!(session.admit(second).is_ok());
    }

    #[test]
    fn every_identity_and_digest_boundary_is_rejected() {
        for operation in [
            String::new(),
            "x".repeat(MAX_COMPONENT_BYTES + 1),
            "bad/id".into(),
        ] {
            let mut value = request();
            value.operation_id = operation;
            assert_eq!(
                value.validate(limits()),
                Err(ValidationError::Identity("operation"))
            );
        }
        let mut value = request();
        value.attempt_id = "bad attempt".into();
        assert_eq!(
            value.validate(limits()),
            Err(ValidationError::Identity("attempt"))
        );
        for (field, replacement) in [
            ("source commit", "a".repeat(39)),
            ("source tree", format!("{}g", "a".repeat(39))),
            ("executable", "a".repeat(63)),
        ] {
            let mut value = request();
            match field {
                "source commit" => value.pin.source_commit = replacement,
                "source tree" => value.pin.source_tree = replacement,
                "executable" => value.pin.executable_sha256 = replacement,
                _ => unreachable!(),
            }
            assert_eq!(
                value.validate(limits()),
                Err(ValidationError::Digest(field))
            );
        }
    }

    #[test]
    fn encoded_frame_and_each_argument_bound_fail_closed() {
        let mut tiny = limits();
        tiny.max_frame_bytes = 1;
        assert_eq!(request().validate(tiny), Err(ValidationError::Arguments));

        let mut value = request();
        value.arguments = vec!["x".repeat(MAX_COMPONENT_BYTES + 1)];
        assert_eq!(value.validate(limits()), Err(ValidationError::Arguments));
        let mut value = request();
        value.arguments = vec!["x".repeat(MAX_COMPONENT_BYTES); MAX_ARGUMENTS];
        assert_eq!(value.validate(limits()), Err(ValidationError::Arguments));
        let mut value = request();
        value.arguments = vec!["ok".into(); MAX_ARGUMENTS];
        assert!(value.validate(limits()).is_ok());
    }

    #[test]
    fn environment_values_are_bounded_and_nul_free() {
        for value_text in ["x".repeat(MAX_COMPONENT_BYTES + 1), "bad\0value".into()] {
            let mut value = request();
            value.environment = BTreeMap::from([("TZ".into(), value_text)]);
            assert_eq!(value.validate(limits()), Err(ValidationError::Environment));
        }
        let mut value = request();
        value.environment = BTreeMap::from([
            ("CSB_RUN_ID".into(), "run-1".into()),
            ("CSB_SEED".into(), "7".into()),
            ("LANG".into(), "C.UTF-8".into()),
            ("LC_ALL".into(), "C.UTF-8".into()),
            ("TZ".into(), "UTC".into()),
        ]);
        assert!(value.validate(limits()).is_ok());
    }

    #[test]
    fn every_artifact_shape_and_count_boundary_is_rejected() {
        for artifact in [
            String::new(),
            "x".repeat(MAX_COMPONENT_BYTES + 1),
            "./result".into(),
            "result/../secret".into(),
        ] {
            let mut value = request();
            value.artifacts = vec![artifact];
            assert_eq!(value.validate(limits()), Err(ValidationError::Artifact));
        }
        let mut value = request();
        value.artifacts = (0..=MAX_ARTIFACTS)
            .map(|index| format!("artifact-{index:03}"))
            .collect();
        assert_eq!(value.validate(limits()), Err(ValidationError::Artifact));
    }

    #[test]
    fn lifetime_operation_history_is_bounded() {
        let roomy = CallLimits {
            max_in_flight: 1,
            ..limits()
        };
        let mut session = BoundarySession::new(roomy).unwrap();
        session.negotiate(CSB_CONTRACT_V1, roomy).unwrap();
        for index in 0..MAX_SESSION_OPERATIONS {
            let mut value = request();
            value.operation_id = format!("operation-{index}");
            value.attempt_id = format!("attempt-{index}");
            session.admit(value).unwrap();
            session
                .finish(&format!("operation-{index}"), &format!("attempt-{index}"))
                .unwrap();
        }
        let mut value = request();
        value.operation_id = "operation-overflow".into();
        value.attempt_id = "attempt-overflow".into();
        assert_eq!(session.admit(value), Err(AdmissionError::Capacity));
    }

    #[test]
    fn durable_success_orders_intent_collection_and_terminal_evidence() {
        let workspace = TempDir::new().unwrap();
        let state = TempDir::new().unwrap();
        let leases = TempDir::new().unwrap();
        let mut value = request();
        value.pin.executable_sha256 = executable(workspace.path(), b"#!/bin/sh\nexit 0\n");
        fs::write(workspace.path().join("result.json"), b"{\"ok\":true}\n").unwrap();
        let store = prepared_store(state.path(), &value);
        let (resources, lease) = resources_and_lease(leases.path());
        let launcher = FakeLauncher {
            output: output(Termination::Exited, Some(0)),
            fail_spawn: false,
        };
        let mut session = BoundarySession::new(limits()).unwrap();
        session.negotiate(CSB_CONTRACT_V1, limits()).unwrap();
        let running = start_with(
            &launcher,
            &mut session,
            value,
            workspace.path(),
            state.path(),
            "fixture",
            PathBuf::from("."),
            resources,
            lease,
            process_limits(),
            StoreLimits::default(),
            "run-1",
            2,
        )
        .unwrap();
        assert_eq!(session.active_operations(), 1);
        let observed = running.wait(&mut session).unwrap();
        assert_eq!(observed.exit_code, Some(0));
        assert_eq!(session.active_operations(), 0);
        assert!(!leases.path().join("cpu-0.lease").exists());
        assert_eq!(
            store
                .load_journal("run-1")
                .unwrap()
                .iter()
                .map(|event| event.state)
                .collect::<Vec<_>>(),
            vec![
                ExecutionState::Planned,
                ExecutionState::Prepared,
                ExecutionState::Running,
                ExecutionState::Collecting,
                ExecutionState::Completed,
            ]
        );
    }

    #[test]
    fn cancellation_is_terminal_and_commits_no_artifact() {
        let workspace = TempDir::new().unwrap();
        let state = TempDir::new().unwrap();
        let leases = TempDir::new().unwrap();
        let mut value = request();
        value.pin.executable_sha256 = executable(workspace.path(), b"cancel fixture");
        let store = prepared_store(state.path(), &value);
        let (resources, lease) = resources_and_lease(leases.path());
        let launcher = FakeLauncher {
            output: output(Termination::Cancelled, None),
            fail_spawn: false,
        };
        let mut session = BoundarySession::new(limits()).unwrap();
        session.negotiate(CSB_CONTRACT_V1, limits()).unwrap();
        let mut running = start_with(
            &launcher,
            &mut session,
            value,
            workspace.path(),
            state.path(),
            "fixture",
            PathBuf::from("."),
            resources,
            lease,
            process_limits(),
            StoreLimits::default(),
            "run-1",
            2,
        )
        .unwrap();
        running.cancel().unwrap();
        running.wait(&mut session).unwrap();
        assert_eq!(
            store.load_journal("run-1").unwrap().last().unwrap().state,
            ExecutionState::Cancelled
        );
    }

    #[test]
    fn spawn_failure_preserves_uncertain_running_intent() {
        let workspace = TempDir::new().unwrap();
        let state = TempDir::new().unwrap();
        let leases = TempDir::new().unwrap();
        let mut value = request();
        value.pin.executable_sha256 = executable(workspace.path(), b"spawn fixture");
        let store = prepared_store(state.path(), &value);
        let (resources, lease) = resources_and_lease(leases.path());
        let launcher = FakeLauncher {
            output: output(Termination::Exited, Some(0)),
            fail_spawn: true,
        };
        let mut session = BoundarySession::new(limits()).unwrap();
        session.negotiate(CSB_CONTRACT_V1, limits()).unwrap();
        assert!(matches!(
            start_with(
                &launcher,
                &mut session,
                value,
                workspace.path(),
                state.path(),
                "fixture",
                PathBuf::from("."),
                resources,
                lease,
                process_limits(),
                StoreLimits::default(),
                "run-1",
                2,
            ),
            Err(ExecutionError::Sandbox(SandboxError::DelegationRejected))
        ));
        assert_eq!(
            store.recovery_decision("run-1").unwrap(),
            RecoveryDecision::NeedsReconciliation
        );
        assert_eq!(session.active_operations(), 1);
    }

    #[test]
    fn filesystem_preflight_rejects_overlap_symlink_and_wrong_digest() {
        let workspace = TempDir::new().unwrap();
        let state = TempDir::new().unwrap();
        let leases = TempDir::new().unwrap();
        let mut value = request();
        value.pin.executable_sha256 = executable(workspace.path(), b"safe fixture");
        let _store = prepared_store(state.path(), &value);
        let (resources, lease) = resources_and_lease(leases.path());
        let launcher = FakeLauncher {
            output: output(Termination::Exited, Some(0)),
            fail_spawn: false,
        };
        let mut session = BoundarySession::new(limits()).unwrap();
        session.negotiate(CSB_CONTRACT_V1, limits()).unwrap();
        assert!(matches!(
            start_with(
                &launcher,
                &mut session,
                value.clone(),
                workspace.path(),
                workspace.path(),
                "fixture",
                PathBuf::from("."),
                resources,
                lease,
                process_limits(),
                StoreLimits::default(),
                "run-1",
                2,
            ),
            Err(ExecutionError::Filesystem)
        ));

        fs::remove_file(workspace.path().join("fixture")).unwrap();
        std::os::unix::fs::symlink("target", workspace.path().join("fixture")).unwrap();
        assert!(matches!(
            verify_regular_at(workspace.path(), "fixture", &value.pin.executable_sha256),
            Err(ExecutionError::Filesystem)
        ));
        fs::remove_file(workspace.path().join("fixture")).unwrap();
        executable(workspace.path(), b"different fixture");
        assert!(matches!(
            verify_regular_at(workspace.path(), "fixture", &value.pin.executable_sha256),
            Err(ExecutionError::ExecutableIdentity)
        ));
        assert!(matches!(
            open_regular_at(workspace.path(), "../escape"),
            Err(ExecutionError::Filesystem)
        ));
    }

    #[test]
    fn verified_descriptor_survives_path_replacement() {
        use std::os::unix::fs::FileExt;

        let workspace = TempDir::new().unwrap();
        let digest = executable(workspace.path(), b"reviewed executable");
        let verified = verify_regular_at(workspace.path(), "fixture", &digest).unwrap();
        fs::remove_file(workspace.path().join("fixture")).unwrap();
        fs::write(workspace.path().join("fixture"), b"replacement").unwrap();
        let mut bytes = [0_u8; 19];
        verified.read_exact_at(&mut bytes, 0).unwrap();
        assert_eq!(&bytes, b"reviewed executable");
        assert_eq!(
            fs::read(workspace.path().join("fixture")).unwrap(),
            b"replacement"
        );
    }

    #[test]
    fn durable_precondition_rejects_attempt_mismatch_and_non_prepared_state() {
        let workspace = TempDir::new().unwrap();
        let leases = TempDir::new().unwrap();
        let mut value = request();
        value.pin.executable_sha256 = executable(workspace.path(), b"durable fixture");
        let launcher = FakeLauncher {
            output: output(Termination::Exited, Some(0)),
            fail_spawn: false,
        };

        let mismatched_state = TempDir::new().unwrap();
        let mut mismatched = value.clone();
        mismatched.attempt_id = "attempt-other".into();
        let _store = prepared_store(mismatched_state.path(), &mismatched);
        let (resources, lease) = resources_and_lease(leases.path());
        let mut session = BoundarySession::new(limits()).unwrap();
        session.negotiate(CSB_CONTRACT_V1, limits()).unwrap();
        assert!(matches!(
            start_with(
                &launcher,
                &mut session,
                value.clone(),
                workspace.path(),
                mismatched_state.path(),
                "fixture",
                PathBuf::from("."),
                resources,
                lease,
                process_limits(),
                StoreLimits::default(),
                "run-1",
                2,
            ),
            Err(ExecutionError::DurableState)
        ));

        let running_state = TempDir::new().unwrap();
        let store = prepared_store(running_state.path(), &value);
        append_running(&store, &value.attempt_id);
        let (resources, lease) = resources_and_lease(leases.path());
        let mut session = BoundarySession::new(limits()).unwrap();
        session.negotiate(CSB_CONTRACT_V1, limits()).unwrap();
        assert!(matches!(
            start_with(
                &launcher,
                &mut session,
                value,
                workspace.path(),
                running_state.path(),
                "fixture",
                PathBuf::from("."),
                resources,
                lease,
                process_limits(),
                StoreLimits::default(),
                "run-1",
                3,
            ),
            Err(ExecutionError::DurableState)
        ));
    }

    #[test]
    fn artifact_symlink_leaves_collection_needing_reconciliation() {
        let workspace = TempDir::new().unwrap();
        let state = TempDir::new().unwrap();
        let leases = TempDir::new().unwrap();
        let mut value = request();
        value.pin.executable_sha256 = executable(workspace.path(), b"artifact fixture");
        std::os::unix::fs::symlink("fixture", workspace.path().join("result.json")).unwrap();
        let store = prepared_store(state.path(), &value);
        let (resources, lease) = resources_and_lease(leases.path());
        let launcher = FakeLauncher {
            output: output(Termination::Exited, Some(0)),
            fail_spawn: false,
        };
        let mut session = BoundarySession::new(limits()).unwrap();
        session.negotiate(CSB_CONTRACT_V1, limits()).unwrap();
        let running = start_with(
            &launcher,
            &mut session,
            value,
            workspace.path(),
            state.path(),
            "fixture",
            PathBuf::from("."),
            resources,
            lease,
            process_limits(),
            StoreLimits::default(),
            "run-1",
            2,
        )
        .unwrap();
        assert!(matches!(
            running.wait(&mut session),
            Err(ExecutionError::Filesystem)
        ));
        assert_eq!(
            store.recovery_decision("run-1").unwrap(),
            RecoveryDecision::NeedsReconciliation
        );
        assert_eq!(session.active_operations(), 1);
    }

    #[test]
    fn valid_shape_but_unpinned_source_identities_are_rejected() {
        let mut value = request();
        value.pin.source_commit = "a".repeat(40);
        assert_eq!(
            value.validate(limits()),
            Err(ValidationError::Digest("source commit"))
        );
        let mut value = request();
        value.pin.source_tree = "a".repeat(40);
        assert_eq!(
            value.validate(limits()),
            Err(ValidationError::Digest("source tree"))
        );
    }

    #[test]
    fn regular_file_checks_reject_mode_kind_size_and_dot_name() {
        let root = TempDir::new().unwrap();
        let file = root.path().join("plain");
        fs::write(&file, b"plain").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
        let digest = format!("{:x}", Sha256::digest(b"plain"));
        assert!(matches!(
            verify_regular_at(root.path(), "plain", &digest),
            Err(ExecutionError::Filesystem)
        ));
        fs::create_dir(root.path().join("directory")).unwrap();
        assert!(matches!(
            open_regular_at(root.path(), "directory"),
            Err(ExecutionError::Filesystem)
        ));
        let large = fs::File::create(root.path().join("large")).unwrap();
        large.set_len(MAX_FILE_BYTES + 1).unwrap();
        assert!(matches!(
            open_regular_at(root.path(), "large"),
            Err(ExecutionError::Filesystem)
        ));
        assert!(matches!(
            open_regular_at(root.path(), "."),
            Err(ExecutionError::Filesystem)
        ));
        assert!(matches!(
            canonical_directory(&file),
            Err(ExecutionError::Filesystem)
        ));
        assert!(matches!(
            canonical_directory(Path::new("relative")),
            Err(ExecutionError::Filesystem)
        ));
        let symlink = root.path().join("symlink-root");
        std::os::unix::fs::symlink(root.path(), &symlink).unwrap();
        assert!(matches!(
            canonical_directory(&symlink),
            Err(ExecutionError::Filesystem)
        ));
    }

    #[test]
    fn failed_timeout_and_clock_overflow_remain_explicit() {
        for (index, termination, exit_code) in [
            (0, Termination::Exited, Some(9)),
            (1, Termination::TimedOut, None),
        ] {
            let state = TempDir::new().unwrap();
            let value = request();
            let store = prepared_store(state.path(), &value);
            append_running(&store, &value.attempt_id);
            let mut child = FakeChild {
                output: output(termination, exit_code),
                lease: None,
            };
            finish_with(
                &mut child,
                &store,
                "run-1",
                &value.attempt_id,
                &[],
                state.path(),
                3,
                index,
            )
            .unwrap();
            assert_eq!(
                store.load_journal("run-1").unwrap().last().unwrap().state,
                ExecutionState::Failed
            );
        }

        let state = TempDir::new().unwrap();
        let value = request();
        let store = prepared_store(state.path(), &value);
        append_running(&store, &value.attempt_id);
        let mut child = FakeChild {
            output: output(Termination::Exited, Some(0)),
            lease: None,
        };
        assert!(matches!(
            finish_with(
                &mut child,
                &store,
                "run-1",
                &value.attempt_id,
                &[],
                state.path(),
                3,
                u64::MAX,
            ),
            Err(ExecutionError::DurableState)
        ));
        assert_eq!(
            store.recovery_decision("run-1").unwrap(),
            RecoveryDecision::NeedsReconciliation
        );
    }
}
