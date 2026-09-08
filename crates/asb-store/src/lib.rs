// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Crash-consistent local storage for run intent, events, and artifacts.

use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use asb_protocol::{ArtifactRef, Id};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

mod trace;
mod verification;

pub use trace::{
    ExportResult, MAX_TRACE_QUEUE_CAPACITY, OTEL_GENAI_SCHEMA_URL, OTEL_GENAI_SEMCONV_REVISION,
    OtlpAnyValue, OtlpAttribute, OtlpIntegerValue, OtlpJsonSpan, OtlpStringValue, TraceExporter,
};
pub use verification::{
    MAX_VERIFICATION_BYTES, VERIFICATION_SCHEMA_VERSION, VerificationObservation,
    VerificationOutcome,
};

/// Current immutable-manifest schema version.
pub const MANIFEST_SCHEMA_VERSION: u16 = 1;
/// Current journal-record schema version.
pub const JOURNAL_SCHEMA_VERSION: u16 = 1;
/// Absolute implementation ceiling for one manifest.
pub const MAX_MANIFEST_BYTES: u64 = 16 * 1024 * 1024;
/// Absolute implementation ceiling for one event.
pub const MAX_EVENT_BYTES: u64 = 16 * 1024 * 1024;
/// Absolute implementation ceiling for one journal.
pub const MAX_JOURNAL_BYTES: u64 = 256 * 1024 * 1024;
/// Absolute implementation ceiling for one artifact.
pub const MAX_ARTIFACT_BYTES: u64 = 16 * 1024 * 1024 * 1024;

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Explicit byte limits applied before durable writes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StoreLimits {
    /// Largest serialized manifest.
    pub max_manifest_bytes: u64,
    /// Largest serialized event before checksum framing.
    pub max_event_bytes: u64,
    /// Largest complete journal.
    pub max_journal_bytes: u64,
    /// Largest artifact.
    pub max_artifact_bytes: u64,
}

impl Default for StoreLimits {
    fn default() -> Self {
        Self {
            max_manifest_bytes: MAX_MANIFEST_BYTES,
            max_event_bytes: MAX_EVENT_BYTES,
            max_journal_bytes: MAX_JOURNAL_BYTES,
            max_artifact_bytes: MAX_ARTIFACT_BYTES,
        }
    }
}

impl StoreLimits {
    /// Reject zero limits and values above implementation ceilings.
    pub fn validate(self) -> Result<Self, StoreError> {
        for (kind, value, ceiling) in [
            ("manifest", self.max_manifest_bytes, MAX_MANIFEST_BYTES),
            ("event", self.max_event_bytes, MAX_EVENT_BYTES),
            ("journal", self.max_journal_bytes, MAX_JOURNAL_BYTES),
            ("artifact", self.max_artifact_bytes, MAX_ARTIFACT_BYTES),
        ] {
            if value == 0 || value > ceiling {
                return Err(StoreError::InvalidLimit {
                    kind,
                    value,
                    ceiling,
                });
            }
        }
        Ok(self)
    }
}

/// Immutable versioned run definition.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunManifest {
    /// Store schema generation.
    pub schema_version: u16,
    /// Filesystem-safe stable run identity.
    pub run_id: Id,
    /// Stable execution-attempt identity.
    pub attempt_id: Id,
    /// Content-pinned experiment definition from higher-level contracts.
    pub definition: Value,
}

impl RunManifest {
    fn validate(&self) -> Result<(), StoreError> {
        require_version("manifest", self.schema_version, MANIFEST_SCHEMA_VERSION)?;
        validate_component("run_id", &self.run_id.0)?;
        if self.attempt_id.0.is_empty() {
            return Err(StoreError::InvalidIdentity("attempt_id"));
        }
        Ok(())
    }
}

/// Persisted execution lifecycle states.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionState {
    /// Definition exists but preparation has not completed.
    Planned,
    /// Inputs are ready and no execution effect has started.
    Prepared,
    /// Execution intent was persisted before starting the process.
    Running,
    /// Execution stopped and terminal evidence is being collected.
    Collecting,
    /// All required evidence was committed.
    Completed,
    /// Execution or collection failed.
    Failed,
    /// Cancellation reached a terminal state.
    Cancelled,
}

impl ExecutionState {
    fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

/// One ordered durable lifecycle event.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct JournalEvent {
    /// Journal schema generation.
    pub schema_version: u16,
    /// Zero-based contiguous sequence.
    pub sequence: u64,
    /// Attempt identity copied from the manifest.
    pub attempt_id: Id,
    /// Monotonic offset from attempt creation.
    pub monotonic_offset_ns: u64,
    /// State reached after this event is committed.
    pub state: ExecutionState,
    /// Privacy-reviewed structured evidence.
    pub evidence: Value,
}

/// Conservative restart decision derived only from durable state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryDecision {
    /// No external-effect intent exists.
    StartAllowed,
    /// Preparation may continue from this recorded state.
    Resume(ExecutionState),
    /// Live process, cgroup, and artifact state must be inspected first.
    NeedsReconciliation,
    /// The attempt is durably terminal.
    Terminal(ExecutionState),
}

/// Filesystem-backed durable store.
#[derive(Debug)]
pub struct AtomicStore {
    root: PathBuf,
    limits: StoreLimits,
}

impl AtomicStore {
    /// Open or create a private store root.
    pub fn open(root: impl AsRef<Path>, limits: StoreLimits) -> Result<Self, StoreError> {
        let limits = limits.validate()?;
        let root = root.as_ref().to_path_buf();
        private_dir(&root)?;
        private_dir(&root.join("runs"))?;
        let lock = private_file(&root.join("store.lock"), false)?;
        lock.sync_all()?;
        sync_dir(&root)?;
        Ok(Self { root, limits })
    }

    /// Atomically create an immutable manifest and empty journal.
    pub fn create_run(&self, manifest: &RunManifest) -> Result<(), StoreError> {
        manifest.validate()?;
        let _lock = self.lock()?;
        let destination = self.run_dir(&manifest.run_id.0)?;
        if destination.exists() {
            return Err(StoreError::RunExists(manifest.run_id.0.clone()));
        }
        let bytes = encode_bounded(manifest, self.limits.max_manifest_bytes, "manifest")?;
        let stage = temporary_path(&self.root.join("runs"), &manifest.run_id.0, "create");
        private_dir(&stage)?;
        let result = (|| {
            private_dir(&stage.join("artifacts"))?;
            write_new_synced(&stage.join("manifest.json"), &bytes)?;
            write_new_synced(&stage.join("journal.ndjson"), b"")?;
            sync_dir(&stage)?;
            fs::rename(&stage, &destination)?;
            sync_dir(&self.root.join("runs"))
        })();
        if result.is_err() && stage.exists() {
            fs::remove_dir_all(&stage)?;
        }
        result
    }

    /// Load and validate an immutable manifest with a hard read bound.
    pub fn load_manifest(&self, run_id: &str) -> Result<RunManifest, StoreError> {
        let _lock = self.lock()?;
        self.load_manifest_unlocked(run_id)
    }

    /// Append an event by atomically replacing the checksummed bounded journal.
    pub fn append(&self, run_id: &str, event: &JournalEvent) -> Result<(), StoreError> {
        require_version("journal", event.schema_version, JOURNAL_SCHEMA_VERSION)?;
        let _lock = self.lock()?;
        let manifest = self.load_manifest_unlocked(run_id)?;
        if event.attempt_id != manifest.attempt_id {
            return Err(StoreError::StaleAttempt);
        }
        let mut events = self.load_journal_unlocked(run_id)?;
        let expected = u64::try_from(events.len()).map_err(|_| StoreError::SequenceOverflow)?;
        if event.sequence != expected {
            return Err(StoreError::Sequence {
                expected,
                actual: event.sequence,
            });
        }
        validate_transition(events.last().map(|item| item.state), event.state)?;
        events.push(event.clone());
        let bytes = encode_journal(
            &events,
            self.limits.max_event_bytes,
            self.limits.max_journal_bytes,
        )?;
        atomic_replace(
            &self.run_dir(run_id)?.join("journal.ndjson"),
            &bytes,
            Fault::None,
        )
    }

    /// Verify and load every journal record.
    pub fn load_journal(&self, run_id: &str) -> Result<Vec<JournalEvent>, StoreError> {
        let _lock = self.lock()?;
        self.load_journal_unlocked(run_id)
    }

    /// Refuse duplicate execution when the last durable state is uncertain.
    pub fn recovery_decision(&self, run_id: &str) -> Result<RecoveryDecision, StoreError> {
        let _lock = self.lock()?;
        self.load_manifest_unlocked(run_id)?;
        let events = self.load_journal_unlocked(run_id)?;
        Ok(match events.last().map(|event| event.state) {
            None | Some(ExecutionState::Planned) => RecoveryDecision::StartAllowed,
            Some(ExecutionState::Prepared) => RecoveryDecision::Resume(ExecutionState::Prepared),
            Some(ExecutionState::Running | ExecutionState::Collecting) => {
                RecoveryDecision::NeedsReconciliation
            }
            Some(state) if state.is_terminal() => RecoveryDecision::Terminal(state),
            Some(_) => unreachable!("all states are classified"),
        })
    }

    /// Stream a bounded artifact into content-addressed storage.
    pub fn put_artifact(
        &self,
        run_id: &str,
        name: &str,
        mut input: impl Read,
    ) -> Result<ArtifactRef, StoreError> {
        validate_artifact_name(name)?;
        let _lock = self.lock()?;
        self.load_manifest_unlocked(run_id)?;
        let directory = self.run_dir(run_id)?.join("artifacts");
        let temporary = temporary_path(&directory, "artifact", "partial");
        let mut output = private_file(&temporary, true)?;
        let mut hasher = Sha256::new();
        let mut size = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        let result = (|| {
            loop {
                let count = input.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                size = size
                    .checked_add(count as u64)
                    .ok_or(StoreError::SizeOverflow)?;
                enforce_size("artifact", size, self.limits.max_artifact_bytes)?;
                output.write_all(&buffer[..count])?;
                hasher.update(&buffer[..count]);
            }
            output.sync_all()?;
            let digest = hex_digest(&hasher.finalize());
            let destination = directory.join(&digest);
            if destination.exists() {
                verify_artifact(&destination, size, &digest)?;
                fs::remove_file(&temporary)?;
            } else {
                fs::rename(&temporary, &destination)?;
                sync_dir(&directory)?;
            }
            Ok(ArtifactRef {
                name: name.into(),
                size_bytes: size,
                sha256: digest,
            })
        })();
        drop(output);
        if result.is_err() && temporary.exists() {
            fs::remove_file(temporary)?;
        }
        result
    }

    /// Re-read and verify a referenced artifact before accepting it as evidence.
    pub fn verify_artifact_ref(
        &self,
        run_id: &str,
        artifact: &ArtifactRef,
    ) -> Result<(), StoreError> {
        validate_artifact_name(&artifact.name)?;
        validate_digest(&artifact.sha256)?;
        let _lock = self.lock()?;
        self.load_manifest_unlocked(run_id)?;
        verify_artifact(
            &self
                .run_dir(run_id)?
                .join("artifacts")
                .join(&artifact.sha256),
            artifact.size_bytes,
            &artifact.sha256,
        )
    }

    fn lock(&self) -> Result<StoreLock, StoreError> {
        let file = private_file(&self.root.join("store.lock"), false)?;
        file.lock()?;
        Ok(StoreLock(file))
    }

    fn run_dir(&self, run_id: &str) -> Result<PathBuf, StoreError> {
        validate_component("run_id", run_id)?;
        validate_directory(&self.root)?;
        let runs = self.root.join("runs");
        validate_directory(&runs)?;
        let run = runs.join(run_id);
        if run.exists() {
            validate_directory(&run)?;
        }
        Ok(run)
    }

    fn load_manifest_unlocked(&self, run_id: &str) -> Result<RunManifest, StoreError> {
        let bytes = read_bounded(
            &self.run_dir(run_id)?.join("manifest.json"),
            self.limits.max_manifest_bytes,
            "manifest",
        )?;
        let manifest: RunManifest = serde_json::from_slice(&bytes)?;
        manifest.validate()?;
        if manifest.run_id.0 != run_id {
            return Err(StoreError::IdentityMismatch);
        }
        Ok(manifest)
    }

    fn load_journal_unlocked(&self, run_id: &str) -> Result<Vec<JournalEvent>, StoreError> {
        let bytes = read_bounded(
            &self.run_dir(run_id)?.join("journal.ndjson"),
            self.limits.max_journal_bytes,
            "journal",
        )?;
        decode_journal(&bytes, self.limits.max_event_bytes)
    }
}

struct StoreLock(File);

impl Drop for StoreLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct JournalRecord {
    event: JournalEvent,
    sha256: String,
}

struct BoundedBytes {
    bytes: Vec<u8>,
    maximum: usize,
    exceeded: bool,
}

impl BoundedBytes {
    fn new(maximum: u64) -> Result<Self, StoreError> {
        let maximum = usize::try_from(maximum).map_err(|_| StoreError::SizeOverflow)?;
        Ok(Self {
            bytes: Vec::with_capacity(maximum.min(4096)),
            maximum,
            exceeded: false,
        })
    }
}

impl Write for BoundedBytes {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.maximum.saturating_sub(self.bytes.len()) {
            self.exceeded = true;
            return Err(std::io::Error::other("serialized value exceeds bound"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum Fault {
    None,
    #[cfg(test)]
    PartialWrite,
    #[cfg(test)]
    BeforeRename,
    #[cfg(test)]
    AfterRename,
}

/// Fail-closed durable-store errors.
#[derive(Debug, Error)]
pub enum StoreError {
    /// Filesystem operation failed.
    #[error("store I/O failed: {0}")]
    Io(#[from] std::io::Error),
    /// JSON encoding or decoding failed.
    #[error("store JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    /// A configured bound is invalid.
    #[error("invalid {kind} limit {value}; ceiling is {ceiling}")]
    InvalidLimit {
        /// Object kind.
        kind: &'static str,
        /// Supplied value.
        value: u64,
        /// Hard ceiling.
        ceiling: u64,
    },
    /// Serialized or stored data exceeded its bound.
    #[error("{kind} size {actual} exceeds bound {maximum}")]
    TooLarge {
        /// Object kind.
        kind: &'static str,
        /// Observed size.
        actual: u64,
        /// Configured maximum.
        maximum: u64,
    },
    /// An unknown schema cannot be migrated safely.
    #[error("unsupported {kind} schema {actual}; supported is {supported}")]
    UnsupportedVersion {
        /// Object kind.
        kind: &'static str,
        /// Observed version.
        actual: u16,
        /// Supported version.
        supported: u16,
    },
    /// A filesystem component or stable identity is invalid.
    #[error("invalid identity: {0}")]
    InvalidIdentity(&'static str),
    /// An artifact name attempted path interpretation.
    #[error("invalid artifact name")]
    InvalidArtifactName,
    /// The immutable run already exists.
    #[error("run already exists: {0}")]
    RunExists(String),
    /// The path and manifest disagree about run identity.
    #[error("manifest run identity does not match its path")]
    IdentityMismatch,
    /// An event belongs to a different attempt.
    #[error("event has a stale attempt identity")]
    StaleAttempt,
    /// Journal sequence is not contiguous.
    #[error("journal sequence {actual} does not equal expected {expected}")]
    Sequence {
        /// Required sequence.
        expected: u64,
        /// Supplied sequence.
        actual: u64,
    },
    /// Journal length cannot be represented as a sequence.
    #[error("journal sequence overflow")]
    SequenceOverflow,
    /// Lifecycle transition is invalid.
    #[error("invalid lifecycle transition")]
    InvalidTransition,
    /// A journal record lacks newline termination.
    #[error("truncated journal record")]
    TruncatedJournal,
    /// A journal checksum does not match its event.
    #[error("journal checksum mismatch")]
    ChecksumMismatch,
    /// Artifact bytes do not match their content address.
    #[error("content-addressed artifact mismatch")]
    ArtifactMismatch,
    /// Checked byte arithmetic overflowed.
    #[error("stored size overflow")]
    SizeOverflow,
    /// Deterministic storage fault used by assurance tests.
    #[error("injected storage fault: {0}")]
    Injected(&'static str),
    /// A path is a symbolic link or not the required object type.
    #[error("unsafe store path")]
    UnsafePath,
    /// An immutable verification observation already exists.
    #[error("verification observation already exists")]
    VerificationExists,
    /// Verification evidence is invalid or inconsistent.
    #[error("verification evidence is invalid")]
    InvalidVerification,
}

fn require_version(kind: &'static str, actual: u16, supported: u16) -> Result<(), StoreError> {
    if actual != supported {
        return Err(StoreError::UnsupportedVersion {
            kind,
            actual,
            supported,
        });
    }
    Ok(())
}

fn validate_component(kind: &'static str, value: &str) -> Result<(), StoreError> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(StoreError::InvalidIdentity(kind));
    }
    Ok(())
}

fn validate_artifact_name(value: &str) -> Result<(), StoreError> {
    if value.is_empty()
        || value.contains('/')
        || value.contains('\\')
        || value == "."
        || value == ".."
    {
        return Err(StoreError::InvalidArtifactName);
    }
    Ok(())
}

fn validate_digest(value: &str) -> Result<(), StoreError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(StoreError::ArtifactMismatch);
    }
    Ok(())
}

fn enforce_size(kind: &'static str, actual: u64, maximum: u64) -> Result<(), StoreError> {
    if actual > maximum {
        return Err(StoreError::TooLarge {
            kind,
            actual,
            maximum,
        });
    }
    Ok(())
}

fn validate_transition(from: Option<ExecutionState>, to: ExecutionState) -> Result<(), StoreError> {
    let valid = matches!(
        (from, to),
        (None, ExecutionState::Planned)
            | (Some(ExecutionState::Planned), ExecutionState::Prepared)
            | (Some(ExecutionState::Prepared), ExecutionState::Running)
            | (Some(ExecutionState::Running), ExecutionState::Collecting)
            | (
                Some(ExecutionState::Running | ExecutionState::Collecting),
                ExecutionState::Failed | ExecutionState::Cancelled
            )
            | (Some(ExecutionState::Collecting), ExecutionState::Completed)
    );
    if valid {
        Ok(())
    } else {
        Err(StoreError::InvalidTransition)
    }
}

fn encode_bounded<T: Serialize>(
    value: &T,
    maximum: u64,
    kind: &'static str,
) -> Result<Vec<u8>, StoreError> {
    let mut output = BoundedBytes::new(maximum)?;
    if let Err(error) = serde_json::to_writer(&mut output, value) {
        return Err(if output.exceeded {
            StoreError::TooLarge {
                kind,
                actual: maximum.saturating_add(1),
                maximum,
            }
        } else {
            StoreError::Json(error)
        });
    }
    Ok(output.bytes)
}

fn encode_journal(
    events: &[JournalEvent],
    max_event_bytes: u64,
    max_journal_bytes: u64,
) -> Result<Vec<u8>, StoreError> {
    let mut output = BoundedBytes::new(max_journal_bytes)?;
    for event in events {
        let event_bytes = encode_bounded(event, max_event_bytes, "event")?;
        let record = JournalRecord {
            event: event.clone(),
            sha256: hex_digest(&Sha256::digest(&event_bytes)),
        };
        serde_json::to_writer(&mut output, &record).map_err(|error| {
            if output.exceeded {
                StoreError::TooLarge {
                    kind: "journal",
                    actual: max_journal_bytes.saturating_add(1),
                    maximum: max_journal_bytes,
                }
            } else {
                StoreError::Json(error)
            }
        })?;
        output.write_all(b"\n").map_err(|error| {
            if output.exceeded {
                StoreError::TooLarge {
                    kind: "journal",
                    actual: max_journal_bytes.saturating_add(1),
                    maximum: max_journal_bytes,
                }
            } else {
                StoreError::Io(error)
            }
        })?;
    }
    Ok(output.bytes)
}

fn decode_journal(bytes: &[u8], max_event_bytes: u64) -> Result<Vec<JournalEvent>, StoreError> {
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    if !bytes.ends_with(b"\n") {
        return Err(StoreError::TruncatedJournal);
    }
    let content = &bytes[..bytes.len() - 1];
    if content.is_empty() {
        return Err(StoreError::TruncatedJournal);
    }
    let mut events = Vec::new();
    for line in content.split(|byte| *byte == b'\n') {
        if line.is_empty() {
            return Err(StoreError::TruncatedJournal);
        }
        let record: JournalRecord = serde_json::from_slice(line)?;
        require_version(
            "journal",
            record.event.schema_version,
            JOURNAL_SCHEMA_VERSION,
        )?;
        let expected = u64::try_from(events.len()).map_err(|_| StoreError::SequenceOverflow)?;
        if record.event.sequence != expected {
            return Err(StoreError::Sequence {
                expected,
                actual: record.event.sequence,
            });
        }
        let encoded = encode_bounded(&record.event, max_event_bytes, "event")?;
        if record.sha256 != hex_digest(&Sha256::digest(&encoded)) {
            return Err(StoreError::ChecksumMismatch);
        }
        validate_transition(
            events.last().map(|event: &JournalEvent| event.state),
            record.event.state,
        )?;
        events.push(record.event);
    }
    Ok(events)
}

fn read_bounded(path: &Path, maximum: u64, kind: &'static str) -> Result<Vec<u8>, StoreError> {
    let file = private_read_file(path)?;
    let length = file.metadata()?.len();
    enforce_size(kind, length, maximum)?;
    let capacity = usize::try_from(length).map_err(|_| StoreError::SizeOverflow)?;
    let mut bytes = Vec::with_capacity(capacity);
    BufReader::new(file)
        .take(maximum.saturating_add(1))
        .read_to_end(&mut bytes)?;
    enforce_size(kind, bytes.len() as u64, maximum)?;
    Ok(bytes)
}

fn private_dir(path: &Path) -> Result<(), StoreError> {
    let created = match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(StoreError::UnsafePath);
            }
            false
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::DirBuilder::new().mode(0o700).create(path)?;
            true
        }
        Err(error) => return Err(error.into()),
    };
    let directory = open_directory(path)?;
    directory.set_permissions(fs::Permissions::from_mode(0o700))?;
    directory.sync_all()?;
    if created {
        let parent = containing_directory(path)?;
        sync_dir(parent)?;
    }
    Ok(())
}

fn containing_directory(path: &Path) -> Result<&Path, StoreError> {
    let parent = path.parent().ok_or(StoreError::UnsafePath)?;
    if parent.as_os_str().is_empty() {
        Ok(Path::new("."))
    } else {
        Ok(parent)
    }
}

fn private_read_file(path: &Path) -> Result<File, StoreError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err(StoreError::UnsafePath);
        }
        Ok(_) => {}
        Err(error) => return Err(error.into()),
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err(StoreError::UnsafePath);
    }
    Ok(file)
}

fn private_file(path: &Path, create_new: bool) -> Result<File, StoreError> {
    if let Ok(metadata) = fs::symlink_metadata(path)
        && (metadata.file_type().is_symlink() || !metadata.is_file())
    {
        return Err(StoreError::UnsafePath);
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(!create_new)
        .create_new(create_new)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    Ok(file)
}

fn write_new_synced(path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    let mut file = private_file(path, true)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn atomic_replace(path: &Path, bytes: &[u8], _fault: Fault) -> Result<(), StoreError> {
    let parent = path.parent().ok_or(StoreError::InvalidIdentity("path"))?;
    let temporary = temporary_path(parent, "write", "partial");
    let result = (|| {
        let mut file = private_file(&temporary, true)?;
        #[cfg(test)]
        if matches!(_fault, Fault::PartialWrite) {
            file.write_all(&bytes[..bytes.len().min(3)])?;
            file.sync_all()?;
            return Err(StoreError::Injected("disk full during write"));
        }
        file.write_all(bytes)?;
        file.sync_all()?;
        #[cfg(test)]
        if matches!(_fault, Fault::BeforeRename) {
            return Err(StoreError::Injected("crash before rename"));
        }
        fs::rename(&temporary, path)?;
        #[cfg(test)]
        if matches!(_fault, Fault::AfterRename) {
            return Err(StoreError::Injected("crash after rename"));
        }
        sync_dir(parent)
    })();
    if result.is_err() && temporary.exists() {
        fs::remove_file(temporary)?;
    }
    result
}

fn temporary_path(parent: &Path, stem: &str, suffix: &str) -> PathBuf {
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    parent.join(format!(
        ".{stem}.{}.{}.{suffix}",
        std::process::id(),
        sequence
    ))
}

fn sync_dir(path: &Path) -> Result<(), StoreError> {
    open_directory(path)?.sync_all()?;
    Ok(())
}

fn open_directory(path: &Path) -> Result<File, StoreError> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    if !file.metadata()?.is_dir() {
        return Err(StoreError::UnsafePath);
    }
    Ok(file)
}

fn validate_directory(path: &Path) -> Result<(), StoreError> {
    open_directory(path).map(|_| ())
}

fn hex_digest(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut output, "{byte:02x}").expect("String writes cannot fail");
    }
    output
}

fn verify_artifact(path: &Path, size: u64, digest: &str) -> Result<(), StoreError> {
    let file = private_read_file(path)?;
    if file.metadata()?.len() != size {
        return Err(StoreError::ArtifactMismatch);
    }
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    if hex_digest(&hasher.finalize()) != digest {
        return Err(StoreError::ArtifactMismatch);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::os::unix::fs::symlink;
    use std::sync::{Arc, Barrier};
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(name: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let base = std::env::var_os("CARGO_TARGET_TMPDIR")
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir);
            let path = base.join(format!("asb-store-{name}-{}-{nonce}", std::process::id()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn manifest(run: &str) -> RunManifest {
        RunManifest {
            schema_version: MANIFEST_SCHEMA_VERSION,
            run_id: Id(run.into()),
            attempt_id: Id("attempt-1".into()),
            definition: serde_json::json!({"workload_sha256": "00"}),
        }
    }

    fn event(sequence: u64, state: ExecutionState) -> JournalEvent {
        JournalEvent {
            schema_version: JOURNAL_SCHEMA_VERSION,
            sequence,
            attempt_id: Id("attempt-1".into()),
            monotonic_offset_ns: sequence,
            state,
            evidence: serde_json::json!({}),
        }
    }

    #[test]
    fn lifecycle_and_recovery_are_conservative() {
        let root = TestDir::new("lifecycle");
        let store = AtomicStore::open(&root.0, StoreLimits::default()).unwrap();
        store.create_run(&manifest("run-1")).unwrap();
        assert_eq!(store.load_manifest("run-1").unwrap(), manifest("run-1"));
        assert_eq!(
            store.recovery_decision("run-1").unwrap(),
            RecoveryDecision::StartAllowed
        );
        store
            .append("run-1", &event(0, ExecutionState::Planned))
            .unwrap();
        store
            .append("run-1", &event(1, ExecutionState::Prepared))
            .unwrap();
        assert_eq!(
            store.recovery_decision("run-1").unwrap(),
            RecoveryDecision::Resume(ExecutionState::Prepared)
        );
        store
            .append("run-1", &event(2, ExecutionState::Running))
            .unwrap();
        assert_eq!(
            store.recovery_decision("run-1").unwrap(),
            RecoveryDecision::NeedsReconciliation
        );
        store
            .append("run-1", &event(3, ExecutionState::Collecting))
            .unwrap();
        store
            .append("run-1", &event(4, ExecutionState::Completed))
            .unwrap();
        assert_eq!(
            store.recovery_decision("run-1").unwrap(),
            RecoveryDecision::Terminal(ExecutionState::Completed)
        );
        assert!(matches!(
            store.append("run-1", &event(5, ExecutionState::Failed)),
            Err(StoreError::InvalidTransition)
        ));
    }

    #[test]
    fn bad_sequence_transition_and_attempt_fail_closed() {
        let root = TestDir::new("state-errors");
        let store = AtomicStore::open(&root.0, StoreLimits::default()).unwrap();
        store.create_run(&manifest("run-1")).unwrap();
        assert!(matches!(
            store.append("run-1", &event(1, ExecutionState::Planned)),
            Err(StoreError::Sequence { .. })
        ));
        assert!(matches!(
            store.append("run-1", &event(0, ExecutionState::Running)),
            Err(StoreError::InvalidTransition)
        ));
        let mut stale = event(0, ExecutionState::Planned);
        stale.attempt_id = Id("old".into());
        assert!(matches!(
            store.append("run-1", &stale),
            Err(StoreError::StaleAttempt)
        ));
    }

    #[test]
    fn truncation_checksum_and_schema_corruption_are_rejected() {
        let root = TestDir::new("corruption");
        let store = AtomicStore::open(&root.0, StoreLimits::default()).unwrap();
        store.create_run(&manifest("run-1")).unwrap();
        store
            .append("run-1", &event(0, ExecutionState::Planned))
            .unwrap();
        let journal = root.0.join("runs/run-1/journal.ndjson");
        let original = fs::read(&journal).unwrap();
        fs::write(&journal, &original[..original.len() - 1]).unwrap();
        assert!(matches!(
            store.load_journal("run-1"),
            Err(StoreError::TruncatedJournal)
        ));
        let changed = String::from_utf8(original)
            .unwrap()
            .replace("planned", "running");
        fs::write(&journal, changed).unwrap();
        assert!(matches!(
            store.load_journal("run-1"),
            Err(StoreError::ChecksumMismatch)
        ));
        let mut unsupported = manifest("run-2");
        unsupported.schema_version = 2;
        assert!(matches!(
            store.create_run(&unsupported),
            Err(StoreError::UnsupportedVersion { .. })
        ));
    }

    #[test]
    fn partial_disk_full_and_crash_boundaries_preserve_atomicity() {
        let root = TestDir::new("faults");
        let path = root.0.join("state");
        fs::write(&path, b"old").unwrap();
        assert!(matches!(
            atomic_replace(&path, b"replacement", Fault::PartialWrite),
            Err(StoreError::Injected(_))
        ));
        assert_eq!(fs::read(&path).unwrap(), b"old");
        assert!(matches!(
            atomic_replace(&path, b"replacement", Fault::BeforeRename),
            Err(StoreError::Injected(_))
        ));
        assert_eq!(fs::read(&path).unwrap(), b"old");
        assert!(matches!(
            atomic_replace(&path, b"new", Fault::AfterRename),
            Err(StoreError::Injected(_))
        ));
        assert_eq!(fs::read(&path).unwrap(), b"new");
        assert_eq!(fs::read_dir(&root.0).unwrap().count(), 1);
    }

    #[test]
    fn artifacts_are_bounded_addressed_and_path_safe() {
        let root = TestDir::new("artifacts");
        let limits = StoreLimits {
            max_artifact_bytes: 4,
            ..StoreLimits::default()
        };
        let store = AtomicStore::open(&root.0, limits).unwrap();
        store.create_run(&manifest("run-1")).unwrap();
        let artifact = store
            .put_artifact("run-1", "stdout", Cursor::new(b"data"))
            .unwrap();
        assert_eq!(artifact.size_bytes, 4);
        assert_eq!(artifact.sha256.len(), 64);
        store.verify_artifact_ref("run-1", &artifact).unwrap();
        assert_eq!(
            store
                .put_artifact("run-1", "same", Cursor::new(b"data"))
                .unwrap()
                .sha256,
            artifact.sha256
        );
        assert!(matches!(
            store.put_artifact("run-1", "large", Cursor::new(b"12345")),
            Err(StoreError::TooLarge { .. })
        ));
        assert!(matches!(
            store.put_artifact("run-1", "../escape", Cursor::new(b"x")),
            Err(StoreError::InvalidArtifactName)
        ));
        let path = root.0.join("runs/run-1/artifacts").join(&artifact.sha256);
        fs::write(path, b"evil").unwrap();
        assert!(matches!(
            store.verify_artifact_ref("run-1", &artifact),
            Err(StoreError::ArtifactMismatch)
        ));
        let mut invalid = artifact;
        invalid.sha256 = "NOT-A-DIGEST".into();
        assert!(matches!(
            store.verify_artifact_ref("run-1", &invalid),
            Err(StoreError::ArtifactMismatch)
        ));
    }

    #[test]
    fn limits_identities_and_duplicate_runs_are_rejected() {
        let root = TestDir::new("limits");
        let bad = StoreLimits {
            max_journal_bytes: 0,
            ..StoreLimits::default()
        };
        assert!(matches!(
            AtomicStore::open(&root.0, bad),
            Err(StoreError::InvalidLimit { .. })
        ));
        let store = AtomicStore::open(&root.0, StoreLimits::default()).unwrap();
        assert!(matches!(
            store.create_run(&manifest("../escape")),
            Err(StoreError::InvalidIdentity("run_id"))
        ));
        store.create_run(&manifest("run-1")).unwrap();
        assert!(matches!(
            store.create_run(&manifest("run-1")),
            Err(StoreError::RunExists(_))
        ));
        let path = root.0.join("runs/run-1/manifest.json");
        fs::write(
            &path,
            fs::read_to_string(&path).unwrap().replace("run-1", "run-2"),
        )
        .unwrap();
        assert!(matches!(
            store.load_manifest("run-1"),
            Err(StoreError::IdentityMismatch)
        ));
    }

    #[test]
    fn serialization_bounds_fail_before_committing_partial_state() {
        let root = TestDir::new("serialization-bounds");
        let manifest_limits = StoreLimits {
            max_manifest_bytes: 32,
            ..StoreLimits::default()
        };
        let store = AtomicStore::open(root.0.join("manifest"), manifest_limits).unwrap();
        assert!(matches!(
            store.create_run(&manifest("run-1")),
            Err(StoreError::TooLarge {
                kind: "manifest",
                ..
            })
        ));
        assert!(!root.0.join("manifest/runs/run-1").exists());

        let journal_limits = StoreLimits {
            max_event_bytes: 1024,
            max_journal_bytes: 16,
            ..StoreLimits::default()
        };
        let store = AtomicStore::open(root.0.join("journal"), journal_limits).unwrap();
        store.create_run(&manifest("run-1")).unwrap();
        assert!(matches!(
            store.append("run-1", &event(0, ExecutionState::Planned)),
            Err(StoreError::TooLarge {
                kind: "journal",
                ..
            })
        ));
        assert!(store.load_journal("run-1").unwrap().is_empty());

        let event_limits = StoreLimits {
            max_event_bytes: 16,
            ..StoreLimits::default()
        };
        let store = AtomicStore::open(root.0.join("event"), event_limits).unwrap();
        store.create_run(&manifest("run-1")).unwrap();
        assert!(matches!(
            store.append("run-1", &event(0, ExecutionState::Planned)),
            Err(StoreError::TooLarge { kind: "event", .. })
        ));
    }

    #[test]
    fn independent_store_writers_serialize_duplicate_sequence() {
        let root = TestDir::new("concurrency");
        let store = AtomicStore::open(&root.0, StoreLimits::default()).unwrap();
        store.create_run(&manifest("run-1")).unwrap();
        store
            .append("run-1", &event(0, ExecutionState::Planned))
            .unwrap();
        let barrier = Arc::new(Barrier::new(3));
        let mut workers = Vec::new();
        for _ in 0..2 {
            let path = root.0.clone();
            let barrier = Arc::clone(&barrier);
            workers.push(thread::spawn(move || {
                let store = AtomicStore::open(path, StoreLimits::default()).unwrap();
                barrier.wait();
                store.append("run-1", &event(1, ExecutionState::Prepared))
            }));
        }
        barrier.wait();
        let successes = workers
            .into_iter()
            .map(|worker| worker.join().unwrap().is_ok())
            .filter(|succeeded| *succeeded)
            .count();
        assert_eq!(successes, 1);
        assert_eq!(store.load_journal("run-1").unwrap().len(), 2);
    }

    #[test]
    fn private_permissions_and_unknown_journal_version_are_enforced() {
        let root = TestDir::new("permissions");
        let store = AtomicStore::open(&root.0, StoreLimits::default()).unwrap();
        store.create_run(&manifest("run-1")).unwrap();
        assert_eq!(
            fs::metadata(&root.0).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(root.0.join("store.lock"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let mut unknown = event(0, ExecutionState::Planned);
        unknown.schema_version = 2;
        assert!(matches!(
            store.append("run-1", &unknown),
            Err(StoreError::UnsupportedVersion {
                kind: "journal",
                ..
            })
        ));
    }

    #[test]
    fn symlink_roots_locks_and_runs_fail_closed() {
        let root = TestDir::new("symlinks");
        let actual = root.0.join("actual");
        fs::create_dir(&actual).unwrap();
        let linked = root.0.join("linked");
        symlink(&actual, &linked).unwrap();
        assert!(matches!(
            AtomicStore::open(&linked, StoreLimits::default()),
            Err(StoreError::UnsafePath)
        ));

        let store_root = root.0.join("store");
        let store = AtomicStore::open(&store_root, StoreLimits::default()).unwrap();
        store.create_run(&manifest("run-1")).unwrap();
        let outside = root.0.join("outside");
        fs::write(&outside, b"outside").unwrap();
        fs::remove_file(store_root.join("store.lock")).unwrap();
        symlink(&outside, store_root.join("store.lock")).unwrap();
        assert!(matches!(
            store.load_manifest("run-1"),
            Err(StoreError::UnsafePath)
        ));
        fs::remove_file(store_root.join("store.lock")).unwrap();
        fs::write(store_root.join("store.lock"), b"").unwrap();

        let manifest_path = store_root.join("runs/run-1/manifest.json");
        let saved_manifest = fs::read(&manifest_path).unwrap();
        fs::remove_file(&manifest_path).unwrap();
        symlink(&outside, &manifest_path).unwrap();
        assert!(matches!(
            store.load_manifest("run-1"),
            Err(StoreError::UnsafePath)
        ));
        fs::remove_file(&manifest_path).unwrap();
        fs::write(&manifest_path, saved_manifest).unwrap();

        let journal_path = store_root.join("runs/run-1/journal.ndjson");
        fs::remove_file(&journal_path).unwrap();
        symlink(&outside, &journal_path).unwrap();
        assert!(matches!(
            store.load_journal("run-1"),
            Err(StoreError::UnsafePath)
        ));
        fs::remove_file(&journal_path).unwrap();
        fs::write(&journal_path, b"").unwrap();

        let artifact = store
            .put_artifact("run-1", "symlink-check", Cursor::new(b"data"))
            .unwrap();
        let artifact_path = store_root
            .join("runs/run-1/artifacts")
            .join(&artifact.sha256);
        fs::remove_file(&artifact_path).unwrap();
        symlink(&outside, &artifact_path).unwrap();
        assert!(matches!(
            store.verify_artifact_ref("run-1", &artifact),
            Err(StoreError::UnsafePath)
        ));

        let run = store_root.join("runs/run-1");
        let saved = store_root.join("saved-run");
        fs::rename(&run, &saved).unwrap();
        symlink(&actual, &run).unwrap();
        assert!(store.load_manifest("run-1").is_err());
        assert_eq!(fs::read(outside).unwrap(), b"outside");
    }

    #[test]
    fn single_component_relative_roots_sync_the_current_directory() {
        assert_eq!(
            containing_directory(Path::new("relative-store")).unwrap(),
            Path::new(".")
        );
    }
}
