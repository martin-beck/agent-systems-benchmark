// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Bounded adapters for literature workload identities.
//!
//! The adapters deliberately separate source provenance from execution.  A
//! descriptor can be selected and prepared from a deterministic local fixture
//! without making an upstream request or claiming that the official evaluator
//! is qualified.  Official scorers never run in this crate.

use crate::FIXTURE_IDS;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Component, Path, PathBuf};

const EXTERNAL_REGISTRY: &str = include_str!("../registry/v1/external-workloads.json");

/// Maximum bytes read from an explicitly acquired archive.
pub const MAX_ACQUISITION_BYTES: u64 = 64 * 1024 * 1024;
const OWNER: &str = ".asb-literature-owner";
const ANSWER: &str = "mock-answer.txt";
const PROMPT: &str = "PROMPT.md";

/// Stable identities documented by ASB.  They are provenance identities, not
/// a claim that every upstream evaluator is executable or qualified.
pub const LITERATURE_WORKLOAD_IDS: [&str; 22] = [
    "swe-bench",
    "swe-bench-lite",
    "swe-bench-verified",
    "terminal-bench",
    "aider-polyglot",
    "swe-bench-pro",
    "bigcodebench",
    "evalplus",
    "humaneval-plus",
    "mbpp-plus",
    "livecodebench",
    "swe-lancer",
    "swe-rebench",
    "swe-perf",
    "swe-fficiency",
    "core-bench",
    "agentbench",
    "tau-bench",
    "agentdojo",
    "harbor",
    "inspect-ai",
    "hal",
];

/// Public, content-addressed selection metadata shared by list/describe and
/// report consumers.  It is deliberately data-only: no acquisition or
/// evaluator is attempted while constructing the catalog.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkloadCatalogEntry {
    /// Stable selection identity.
    pub id: String,
    /// `builtin` or `literature`.
    pub source: String,
    /// Immutable source revision.
    pub source_revision: String,
    /// Source/task license expression.
    pub license: String,
    /// Evaluator identity, or `unqualified` when absent.
    pub evaluator: String,
    /// Adaptation status (never inferred as semantic parity).
    pub adaptation: String,
    /// Current platform evidence label.
    pub platform: String,
    /// Selection availability (`available`, `fixture_only`, or `unavailable`).
    pub availability: String,
    /// Evidence qualification label.
    pub evidence: String,
    /// Stable digest binding identity, scorer and source revision.
    pub identity_digest: String,
}

/// A catalog selection failure.  All externally controlled IDs fail closed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CatalogSelectionError {
    /// The ID is not in either the built-in or literature namespace.
    Unknown,
    /// The record is visible for provenance but not selectable.
    Unavailable,
    /// The requested platform has no support evidence.
    UnsupportedPlatform,
    /// No evaluator qualification exists for the record.
    EvaluatorMissing,
}

impl fmt::Display for CatalogSelectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unknown => "unknown workload ID",
            Self::Unavailable => "workload is unavailable for selection",
            Self::UnsupportedPlatform => "workload has no supported platform evidence",
            Self::EvaluatorMissing => "workload evaluator is not qualified",
        })
    }
}

/// Return one deterministic catalog containing built-ins and every registry record.
#[must_use]
pub fn workload_catalog() -> Vec<WorkloadCatalogEntry> {
    let mut entries = FIXTURE_IDS
        .iter()
        .map(|id| WorkloadCatalogEntry {
            id: (*id).into(),
            source: "builtin".into(),
            source_revision: "asb-original-v1".into(),
            license: "MIT".into(),
            evaluator: "asb-original-oracle-v1".into(),
            adaptation: "none".into(),
            platform: "linux-x86_64:native-tested".into(),
            availability: "available".into(),
            evidence: "qualified".into(),
            identity_digest: digest_identity(id, "asb-original-v1", "asb-original-oracle-v1"),
        })
        .collect::<Vec<_>>();
    let document: serde_json::Value = serde_json::from_str(EXTERNAL_REGISTRY)
        .expect("checked-in external workload registry must be valid JSON");
    for record in document["workloads"].as_array().into_iter().flatten() {
        let id = record["id"].as_str().unwrap_or_default();
        let source_revision = record["source"]["commit"]
            .as_str()
            .or_else(|| record["source"]["revision"].as_str())
            .unwrap_or_else(|| record["version"].as_str().unwrap_or("unknown"));
        let evaluator = record["evaluator"]["entrypoint"]
            .as_str()
            .unwrap_or("unqualified");
        let provenance = record["evaluator"]["provenance"]["status"]
            .as_str()
            .unwrap_or("missing");
        let selection = record["selection"]
            .as_str()
            .unwrap_or("executable-candidate");
        let platform = record["platforms"]["linux-x86_64"]
            .as_str()
            .unwrap_or("unsupported");
        let unavailable = selection != "executable-candidate" || provenance != "qualified";
        entries.push(WorkloadCatalogEntry {
            id: id.into(),
            source: "literature".into(),
            source_revision: source_revision.into(),
            license: record["source"]["license"]
                .as_str()
                .unwrap_or("NOASSERTION")
                .into(),
            evaluator: evaluator.into(),
            adaptation: "fixture-only".into(),
            platform: platform.into(),
            availability: if unavailable {
                "unavailable"
            } else {
                "available"
            }
            .into(),
            evidence: provenance.into(),
            identity_digest: digest_identity(id, source_revision, evaluator),
        });
    }
    entries.sort_by(|a, b| a.id.cmp(&b.id));
    entries
}

fn digest_identity(id: &str, revision: &str, evaluator: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"asb-workload-selection-v1\0");
    hasher.update(id.as_bytes());
    hasher.update([0]);
    hasher.update(revision.as_bytes());
    hasher.update([0]);
    hasher.update(evaluator.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Select an explicitly qualified local fixture; no network or provider is used.
pub fn select_workload(
    id: &str,
    platform: &str,
) -> Result<WorkloadCatalogEntry, CatalogSelectionError> {
    let entry = workload_catalog()
        .into_iter()
        .find(|entry| entry.id == id)
        .ok_or(CatalogSelectionError::Unknown)?;
    if entry.source == "builtin" {
        return Ok(entry);
    }
    if entry.availability != "available" {
        return Err(if entry.evaluator == "unqualified" {
            CatalogSelectionError::EvaluatorMissing
        } else {
            CatalogSelectionError::Unavailable
        });
    }
    if !entry.platform.starts_with(platform) {
        return Err(CatalogSelectionError::UnsupportedPlatform);
    }
    Ok(entry)
}

/// Broad execution semantics shared by literature workload adapters.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LiteratureFamily {
    /// Repository issue or patch repair.
    RepositoryRepair,
    /// Terminal, operating-system, or system environment.
    TerminalSystem,
    /// Function or multi-language code generation.
    CodeGeneration,
    /// Stateful interaction and tool use.
    StatefulToolUse,
    /// Harness interoperability boundary, not a workload.
    HarnessBoundary,
    /// Documented candidate whose evaluator boundary is not yet supported.
    UnsupportedCandidate,
}

/// Explicit readiness of an adapter, never inferred from source metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdapterStatus {
    /// Only deterministic local fixture execution is available.
    FixtureOnly,
    /// The source is catalogued but cannot be executed by this adapter.
    Unsupported,
}

/// Provenance and execution boundary for one source revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiteratureDescriptor {
    /// Stable ASB selection identity.
    pub id: String,
    /// Workload family.
    pub family: LiteratureFamily,
    /// Immutable source revision from the registry.
    pub source_revision: String,
    /// Separate source/task license expression.
    pub license: String,
    /// Protected official evaluator identity, if known.
    pub evaluator: String,
    /// Exact content digest represented by this adapter descriptor.
    pub content_sha256: String,
    /// Explicit split or time window identity.
    pub split: String,
    /// Qualification is intentionally not implied by a fixture.
    pub status: AdapterStatus,
    /// Network is always denied for the local fixture path.
    pub allowed_network_destinations: BTreeSet<String>,
}

/// Input source accepted by acquisition. No network client exists here.
#[derive(Debug)]
pub enum Acquisition<'a> {
    /// Use the deterministic fixture shipped with ASB tests.
    LocalFixture,
    /// Verify an already acquired regular file without extracting or running it.
    PinnedArchive {
        /// Host-local archive path supplied by the caller.
        path: &'a Path,
        /// Expected lowercase SHA-256 digest.
        sha256: &'a str,
        /// Caller must explicitly accept the separately recorded license.
        license_accepted: bool,
    },
}

/// Acquisition result containing only public identity and digest evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcquisitionReceipt {
    /// Workload identity.
    pub workload_id: String,
    /// Verified bytes, if an archive was supplied.
    pub bytes: u64,
    /// Digest of the acquired bytes or deterministic fixture identity.
    pub sha256: String,
}

/// Explicitly unavailable or deterministic mock evaluation result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Evaluation {
    /// Official scorer/image/reset evidence is missing; no score is produced.
    Unavailable {
        /// Public reason that qualification is missing.
        reason: &'static str,
    },
    /// Local fixture oracle result, never an upstream benchmark score.
    LocalMock {
        /// Whether the deterministic fixture answer matched.
        passed: bool,
    },
}

/// Errors fail closed without exposing paths or submitted contents.
#[derive(Debug)]
pub enum LiteratureError {
    /// ID is not in the stable catalogue.
    UnknownWorkload,
    /// Source is a harness or unsupported candidate.
    Unsupported,
    /// Path, ownership marker, or workspace object was unsafe.
    UnsafePath,
    /// Root already exists or has caller-owned content.
    DestinationExists,
    /// Archive was too large or not a regular file.
    LimitExceeded,
    /// Archive digest or license gate failed.
    DigestMismatch,
    /// Bounded filesystem operation failed.
    Io(io::Error),
}

impl fmt::Display for LiteratureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownWorkload => f.write_str("unknown literature workload"),
            Self::Unsupported => {
                f.write_str("literature workload is not executable by this adapter")
            }
            Self::UnsafePath => f.write_str("unsafe literature workload path or ownership"),
            Self::DestinationExists => {
                f.write_str("literature workload destination already exists")
            }
            Self::LimitExceeded => {
                f.write_str("literature workload acquisition or workspace limit exceeded")
            }
            Self::DigestMismatch => {
                f.write_str("literature workload digest or license gate failed")
            }
            Self::Io(error) => write!(f, "literature workload I/O failed: {error}"),
        }
    }
}
impl std::error::Error for LiteratureError {}
impl From<io::Error> for LiteratureError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

/// A prepared, private agent workspace for one literature identity.
#[derive(Debug)]
pub struct LiteraturePrepared {
    descriptor: LiteratureDescriptor,
    root: PathBuf,
}

impl LiteraturePrepared {
    /// Agent-writable workspace; protected metadata is outside it.
    #[must_use]
    pub fn workspace(&self) -> PathBuf {
        self.root.join("workspace")
    }
    /// Public prompt for the deterministic local fixture.
    #[must_use]
    pub fn prompt(&self) -> String {
        format!(
            "Implement the bounded local fixture for {} ({:?}).\n",
            self.descriptor.id, self.descriptor.family
        )
    }
    /// Return unavailable unless an independently qualified official scorer exists.
    pub fn evaluate(&self) -> Result<Evaluation, LiteratureError> {
        verify_owner(&self.root, &self.descriptor.id)?;
        Ok(Evaluation::Unavailable {
            reason: "official evaluator, image, reset, and oracle evidence are not qualified",
        })
    }
    /// Exercise the adapter contract without representing an upstream score.
    pub fn evaluate_local_mock(&self) -> Result<Evaluation, LiteratureError> {
        verify_owner(&self.root, &self.descriptor.id)?;
        let answer = self.workspace().join(ANSWER);
        let mut text = String::new();
        if fs::metadata(&answer)
            .map(|m| m.len() <= 4096)
            .unwrap_or(false)
        {
            fs::File::open(answer)?.read_to_string(&mut text)?;
        }
        Ok(Evaluation::LocalMock {
            passed: text.trim() == "ASB-LOCAL-MOCK-OK",
        })
    }
    /// Reset the fixture without touching protected grader material.
    pub fn reset(&self) -> Result<(), LiteratureError> {
        verify_owner(&self.root, &self.descriptor.id)?;
        let workspace = self.workspace();
        if workspace.is_symlink() {
            return Err(LiteratureError::UnsafePath);
        }
        fs::remove_dir_all(&workspace)?;
        fs::DirBuilder::new().mode(0o700).create(&workspace)?;
        write_fixture(&workspace, &self.prompt())
    }
    /// Remove only the exact owned root.
    pub fn cleanup(self) -> Result<(), LiteratureError> {
        verify_owner(&self.root, &self.descriptor.id)?;
        fs::remove_dir_all(self.root)?;
        Ok(())
    }
}

/// Adapter implementing the common literature workload lifecycle.
#[derive(Clone, Copy, Debug, Default)]
pub struct LiteratureAdapter;

impl LiteratureAdapter {
    /// Describe a stable documented source identity.
    pub fn describe(id: &str) -> Result<LiteratureDescriptor, LiteratureError> {
        if !LITERATURE_WORKLOAD_IDS.contains(&id) {
            return Err(LiteratureError::UnknownWorkload);
        }
        let family = family(id);
        let (revision, license, evaluator, split) = provenance(id);
        let mut hasher = Sha256::new();
        hasher.update(b"asb-literature-descriptor-v1\0");
        hasher.update(id.as_bytes());
        hasher.update(revision.as_bytes());
        let status = if matches!(
            family,
            LiteratureFamily::HarnessBoundary | LiteratureFamily::UnsupportedCandidate
        ) {
            AdapterStatus::Unsupported
        } else {
            AdapterStatus::FixtureOnly
        };
        Ok(LiteratureDescriptor {
            id: id.into(),
            family,
            source_revision: revision.into(),
            license: license.into(),
            evaluator: evaluator.into(),
            content_sha256: format!("{:x}", hasher.finalize()),
            split: split.into(),
            status,
            allowed_network_destinations: BTreeSet::new(),
        })
    }
    /// List every deterministic, sorted literature identity.
    #[must_use]
    pub const fn ids() -> &'static [&'static str] {
        &LITERATURE_WORKLOAD_IDS
    }
    /// Verify a local acquisition, never downloading or extracting bytes.
    pub fn acquire(
        id: &str,
        source: Acquisition<'_>,
    ) -> Result<AcquisitionReceipt, LiteratureError> {
        let descriptor = Self::describe(id)?;
        match source {
            Acquisition::LocalFixture => Ok(AcquisitionReceipt {
                workload_id: descriptor.id,
                bytes: 0,
                sha256: descriptor.content_sha256,
            }),
            Acquisition::PinnedArchive {
                path,
                sha256,
                license_accepted,
            } => {
                if !license_accepted || !is_safe_archive(path) {
                    return Err(LiteratureError::DigestMismatch);
                }
                let metadata = fs::metadata(path)?;
                if metadata.len() > MAX_ACQUISITION_BYTES {
                    return Err(LiteratureError::LimitExceeded);
                }
                let mut file = fs::File::open(path)?;
                let mut hasher = Sha256::new();
                let mut bytes = 0_u64;
                let mut chunk = [0_u8; 8192];
                loop {
                    let count = file.read(&mut chunk)?;
                    if count == 0 {
                        break;
                    }
                    bytes += count as u64;
                    hasher.update(&chunk[..count]);
                }
                if sha256 != format!("{:x}", hasher.finalize()) {
                    return Err(LiteratureError::DigestMismatch);
                }
                Ok(AcquisitionReceipt {
                    workload_id: descriptor.id,
                    bytes,
                    sha256: sha256.into(),
                })
            }
        }
    }
    /// Prepare a local fixture; external archives are verified separately.
    pub fn prepare(
        id: &str,
        root: impl Into<PathBuf>,
    ) -> Result<LiteraturePrepared, LiteratureError> {
        let descriptor = Self::describe(id)?;
        if descriptor.status == AdapterStatus::Unsupported {
            return Err(LiteratureError::Unsupported);
        }
        let root = root.into();
        if !valid_absolute(&root) {
            return Err(LiteratureError::UnsafePath);
        }
        if root.exists() {
            return Err(LiteratureError::DestinationExists);
        }
        let parent = root.parent().ok_or(LiteratureError::UnsafePath)?;
        if !parent.is_dir() || fs::symlink_metadata(parent)?.file_type().is_symlink() {
            return Err(LiteratureError::UnsafePath);
        }
        fs::DirBuilder::new().mode(0o700).create(&root)?;
        let result = (|| {
            write_owner(&root, &descriptor.id)?;
            let workspace = root.join("workspace");
            fs::DirBuilder::new().mode(0o700).create(&workspace)?;
            write_fixture(
                &workspace,
                &format!(
                    "Implement the bounded local fixture for {} ({:?}).\n",
                    descriptor.id, descriptor.family
                ),
            )
        })();
        if let Err(error) = result {
            let _ = fs::remove_dir_all(&root);
            return Err(error);
        }
        Ok(LiteraturePrepared { descriptor, root })
    }
}

fn family(id: &str) -> LiteratureFamily {
    match id {
        "swe-bench" | "swe-bench-lite" | "swe-bench-verified" | "swe-bench-pro" | "swe-rebench"
        | "swe-lancer" | "swe-perf" | "swe-fficiency" => LiteratureFamily::RepositoryRepair,
        "terminal-bench" | "agentbench" => LiteratureFamily::TerminalSystem,
        "aider-polyglot" | "bigcodebench" | "evalplus" | "humaneval-plus" | "mbpp-plus"
        | "livecodebench" => LiteratureFamily::CodeGeneration,
        "tau-bench" | "agentdojo" | "harbor" | "inspect-ai" => LiteratureFamily::StatefulToolUse,
        "hal" => LiteratureFamily::HarnessBoundary,
        _ => LiteratureFamily::UnsupportedCandidate,
    }
}

fn provenance(id: &str) -> (&'static str, &'static str, &'static str, &'static str) {
    match id {
        "swe-bench" => (
            "02e7a74ffd0b707aab73d203fe87bdc7c76afc8e",
            "MIT",
            "swebench.harness@02e7a74",
            "verified",
        ),
        "swe-bench-lite" => (
            "02e7a74ffd0b707aab73d203fe87bdc7c76afc8e",
            "MIT",
            "swebench.harness@02e7a74",
            "lite",
        ),
        "swe-bench-verified" => (
            "02e7a74ffd0b707aab73d203fe87bdc7c76afc8e",
            "MIT",
            "swebench.harness@02e7a74",
            "verified",
        ),
        "terminal-bench" => (
            "452bf305c6daa62fc59061d22133a7cbc7c1572e",
            "Apache-2.0",
            "harbor.run@4407eb5",
            "terminal-bench-v4",
        ),
        "aider-polyglot" => (
            "5dc9490bb35f9729ef2c95d00a19ccd30c26339c",
            "Apache-2.0-and-per-exercise",
            "aider.polyglot@7e0611e",
            "public",
        ),
        "swe-bench-pro" => (
            "ca10a60a5fcae51e6948ffe1485d4153d421e6c5",
            "MIT",
            "swe_bench_pro_eval@ca10a60",
            "test",
        ),
        "bigcodebench" => (
            "9059fb84d1188c02edeac4995361656a2fdecbef",
            "Apache-2.0",
            "bigcodebench@v0.2.4-9059fb84",
            "complete",
        ),
        "evalplus" | "humaneval-plus" | "mbpp-plus" => (
            "e5d0ed0bab96280b60b637ec7f15b5e4841b0cb2",
            "MIT",
            "evalplus@v0.3.1-e5d0ed0b",
            "plus",
        ),
        "livecodebench" => (
            "28fef95ea8c9f7a547c8329f2cd3d32b92c1fa24",
            "MIT",
            "livecodebench@release_v6",
            "release-v6",
        ),
        "swe-lancer" => (
            "51052cede8cc608f95bb00346635e03759013e5a",
            "NOASSERTION",
            "frontier-evals@51052ced",
            "archived",
        ),
        "swe-rebench" => (
            "e4907b7a90eafaa1f0a6428fd04fe31cdd8b4284",
            "CC-BY-4.0",
            "swe-bench-fork@e4907b7a",
            "window-89cdfbab",
        ),
        "swe-perf" => (
            "9e8fed285ed640b51baaf66b13773706dc95aa45",
            "NOASSERTION",
            "run_evaluation@9e8fed285ed640b51baaf66b13773706dc95aa45",
            "test",
        ),
        "swe-fficiency" => (
            "12d32a2d6800824a7d84bdb6797b5708e7b7957f",
            "Apache-2.0",
            "swefficiency@12d32a2d6800824a7d84bdb6797b5708e7b7957f",
            "test",
        ),
        "core-bench" => (
            "e32a2980e72fe6eb04ee04eb749458f570625663",
            "MIT",
            "hal-eval@16bb03ebc11577fb5ea6dc8bb6c968387085e6aa",
            "test",
        ),
        "agentbench" => (
            "2308.03688",
            "NOASSERTION",
            "agentbench-official",
            "published",
        ),
        "tau-bench" => (
            "tau-bench-pinned",
            "MIT",
            "tau-bench@pinned",
            "retail-airline",
        ),
        "agentdojo" => (
            "agentdojo-pinned",
            "Apache-2.0",
            "agentdojo@pinned",
            "tasks",
        ),
        "harbor" => ("harbor-pinned", "Apache-2.0", "harbor@pinned", "tasks"),
        "inspect-ai" => ("inspect-ai-pinned", "MIT", "inspect-ai@pinned", "scenarios"),
        "hal" => ("2510.11977", "NOASSERTION", "hal-harness", "harness-only"),
        _ => ("unsupported", "NOASSERTION", "unqualified", "unqualified"),
    }
}

fn valid_absolute(path: &Path) -> bool {
    path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::RootDir | Component::Normal(_)))
}
fn is_safe_archive(path: &Path) -> bool {
    path.is_absolute()
        && fs::symlink_metadata(path)
            .map(|m| m.is_file())
            .unwrap_or(false)
}
fn write_owner(root: &Path, id: &str) -> Result<(), LiteratureError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(root.join(OWNER))?;
    file.write_all(id.as_bytes())?;
    file.sync_all()?;
    Ok(())
}
fn verify_owner(root: &Path, id: &str) -> Result<(), LiteratureError> {
    if !root.is_dir() || fs::symlink_metadata(root)?.file_type().is_symlink() {
        return Err(LiteratureError::UnsafePath);
    }
    let owner = fs::read_to_string(root.join(OWNER)).map_err(|_| LiteratureError::UnsafePath)?;
    if owner != id {
        return Err(LiteratureError::UnsafePath);
    }
    Ok(())
}
fn write_fixture(workspace: &Path, prompt: &str) -> Result<(), LiteratureError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(workspace.join(PROMPT))?;
    file.write_all(prompt.as_bytes())?;
    file.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};
    fn root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "asb-literature-{label}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }
    #[test]
    fn every_documented_family_has_a_local_fixture_and_official_score_is_unavailable() {
        for id in LITERATURE_WORKLOAD_IDS {
            let descriptor = LiteratureAdapter::describe(id).unwrap();
            if descriptor.status == AdapterStatus::FixtureOnly {
                let root = root(id);
                let prepared = LiteratureAdapter::prepare(id, &root).unwrap();
                assert_eq!(
                    prepared.evaluate().unwrap(),
                    Evaluation::Unavailable {
                        reason: "official evaluator, image, reset, and oracle evidence are not qualified"
                    }
                );
                assert_eq!(
                    prepared.evaluate_local_mock().unwrap(),
                    Evaluation::LocalMock { passed: false }
                );
                prepared.cleanup().unwrap();
            } else {
                assert_eq!(descriptor.status, AdapterStatus::Unsupported);
            }
        }
    }
    #[test]
    fn malformed_or_unaccepted_archive_fails_closed_without_network() {
        let path = root("archive");
        fs::write(&path, b"fixture").unwrap();
        let digest = format!("{:x}", Sha256::digest(b"fixture"));
        let receipt = LiteratureAdapter::acquire(
            "swe-bench",
            Acquisition::PinnedArchive {
                path: &path,
                sha256: &digest,
                license_accepted: true,
            },
        )
        .unwrap();
        assert_eq!(receipt.bytes, 7);
        assert_eq!(receipt.sha256, digest);
        assert_eq!(
            LiteratureAdapter::acquire(
                "swe-bench",
                Acquisition::PinnedArchive {
                    path: &path,
                    sha256: "0".repeat(64).as_str(),
                    license_accepted: false
                }
            )
            .unwrap_err()
            .to_string(),
            "literature workload digest or license gate failed"
        );
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn catalog_contains_builtins_and_every_registry_record_with_stable_identity() {
        let first = workload_catalog();
        let second = workload_catalog();
        assert_eq!(first, second);
        assert!(
            first
                .iter()
                .any(|entry| entry.id == "original.bug-fix" && entry.source == "builtin")
        );
        let literature = first
            .iter()
            .filter(|entry| entry.source == "literature")
            .collect::<Vec<_>>();
        assert_eq!(literature.len(), 22);
        let literature_count = literature.len();
        for entry in literature {
            assert!(!entry.source_revision.is_empty());
            assert!(!entry.identity_digest.is_empty());
        }
        assert_eq!(first.len(), FIXTURE_IDS.len() + literature_count);
    }

    #[test]
    fn selection_is_explicit_and_fails_closed_for_unqualified_records() {
        assert!(select_workload("original.bug-fix", "linux-x86_64").is_ok());
        assert_eq!(
            select_workload("not-a-workload", "linux-x86_64"),
            Err(CatalogSelectionError::Unknown)
        );
        assert_eq!(
            select_workload("agentops", "linux-x86_64"),
            Err(CatalogSelectionError::Unavailable)
        );
        assert_eq!(
            select_workload("swe-bench", "linux-x86_64"),
            Err(CatalogSelectionError::Unavailable)
        );
    }

    #[test]
    fn reset_and_mock_oracle_are_owned_and_agent_writable_only() {
        let root = root("reset");
        let prepared = LiteratureAdapter::prepare("terminal-bench", &root).unwrap();
        fs::write(prepared.workspace().join(ANSWER), "ASB-LOCAL-MOCK-OK").unwrap();
        assert_eq!(
            prepared.evaluate_local_mock().unwrap(),
            Evaluation::LocalMock { passed: true }
        );
        prepared.reset().unwrap();
        assert!(!prepared.workspace().join(ANSWER).exists());
        assert_eq!(
            prepared.evaluate().unwrap(),
            Evaluation::Unavailable {
                reason: "official evaluator, image, reset, and oracle evidence are not qualified"
            }
        );
        prepared.cleanup().unwrap();
    }

    #[test]
    fn relative_and_symlink_archives_are_rejected() {
        assert!(matches!(
            LiteratureAdapter::acquire(
                "aider-polyglot",
                Acquisition::PinnedArchive {
                    path: Path::new("relative.archive"),
                    sha256: "0",
                    license_accepted: true,
                }
            ),
            Err(LiteratureError::DigestMismatch)
        ));
        let target = root("target");
        let link = root("link");
        fs::write(&target, b"fixture").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(matches!(
            LiteratureAdapter::acquire(
                "aider-polyglot",
                Acquisition::PinnedArchive {
                    path: &link,
                    sha256: "0",
                    license_accepted: true,
                }
            ),
            Err(LiteratureError::DigestMismatch)
        ));
        fs::remove_file(&link).unwrap();
        fs::remove_file(target).unwrap();
    }
    #[test]
    fn unsupported_harness_and_unknown_ids_are_not_executable() {
        assert_eq!(
            LiteratureAdapter::describe("not-a-workload")
                .unwrap_err()
                .to_string(),
            "unknown literature workload"
        );
        assert_eq!(
            LiteratureAdapter::prepare("hal", root("hal"))
                .unwrap_err()
                .to_string(),
            "literature workload is not executable by this adapter"
        );
    }
}
