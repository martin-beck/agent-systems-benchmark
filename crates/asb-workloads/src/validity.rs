// SPDX-License-Identifier: MIT
//! Versioned benchmark-validity and portability registry.

use crate::{FIXTURE_IDS, OriginalWorkloads};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Maximum accepted registry document size.
pub const MAX_REGISTRY_BYTES: usize = 4 * 1024 * 1024;
/// Maximum workload revisions in one registry.
pub const MAX_REGISTRY_ENTRIES: usize = 4096;
/// Maximum entries in a bounded nested registry list.
pub const MAX_NESTED_ENTRIES: usize = 256;
/// Maximum bytes in one public textual field.
pub const MAX_PUBLIC_TEXT_BYTES: usize = 4096;

const BUILTIN_REGISTRY: &str = include_str!("../registry/v1/original-workloads.json");

/// Fail-closed registry parsing or semantic validation failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegistryError {
    /// Input exceeds the whole-document bound.
    DocumentTooLarge,
    /// JSON did not match the closed Rust representation.
    InvalidJson,
    /// Format version is unsupported.
    UnsupportedVersion,
    /// A string, digest, date, number, or collection violates a bound.
    InvalidField,
    /// Workload revisions or nested identities are duplicated or unordered.
    DuplicateOrUnordered,
    /// Related provenance, exposure, evidence, or support fields disagree.
    InconsistentEvidence,
    /// The checked-in built-in registry does not match its workload manifests.
    BuiltinMismatch,
}

impl fmt::Display for RegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::DocumentTooLarge => "benchmark-validity registry exceeds the document bound",
            Self::InvalidJson => "benchmark-validity registry JSON is invalid",
            Self::UnsupportedVersion => "benchmark-validity registry version is unsupported",
            Self::InvalidField => "benchmark-validity registry field is invalid",
            Self::DuplicateOrUnordered => {
                "benchmark-validity registry identities are duplicated or unordered"
            }
            Self::InconsistentEvidence => "benchmark-validity registry evidence is inconsistent",
            Self::BuiltinMismatch => "built-in workload validity evidence does not match manifests",
        })
    }
}

impl std::error::Error for RegistryError {}

/// Source acquisition class.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// Content is maintained in the ASB product repository.
    BuiltIn,
    /// Content is acquired from a separately pinned upstream source.
    Imported,
}

/// Public exposure and holdout state.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExposureStatus {
    /// Task content is intentionally public.
    Public,
    /// Task identity and content remain hidden from evaluated agents.
    Holdout,
}

/// Evidence-qualified platform status; ordering is not a support hierarchy.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PortabilityStatus {
    /// No build or execution evidence has been recorded.
    Planned,
    /// Compilation or package inspection passed without a native workload run.
    BuildOnly,
    /// A simulated or emulated run passed and remains non-native evidence.
    Simulated,
    /// A real run passed on the exact booted platform recorded in evidence.
    NativeTested,
    /// The workload revision is explicitly unsupported on this platform cell.
    Unsupported,
}

/// Semantic adaptation applied for one platform cell.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AdaptationKind {
    /// Exact workload bytes and grader are used.
    None,
    /// Build/runtime packaging changed without changing task semantics.
    Rebuilt,
    /// Task or oracle semantics were translated and require parity evidence.
    Translated,
    /// A disclosed subset of upstream instances is used.
    Filtered,
}

/// Pinned source and license provenance for one workload revision.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SourceProvenance {
    /// Built-in or imported acquisition boundary.
    pub kind: SourceKind,
    /// Public source locator or stable built-in namespace.
    #[schemars(length(min = 1, max = 4096))]
    pub locator: String,
    /// Immutable upstream or in-tree revision.
    #[schemars(length(min = 1, max = 4096))]
    pub revision: String,
    /// SPDX license expression for task content.
    #[schemars(length(min = 1, max = 4096))]
    pub license: String,
    /// Lowercase SHA-256 of the pinned workload content.
    #[schemars(regex(pattern = r"^[0-9a-f]{64}$"), length(equal = 64))]
    pub content_sha256: String,
    /// Calendar date on which the source facts were verified.
    #[schemars(regex(pattern = r"^[0-9]{4}-[0-9]{2}-[0-9]{2}$"), length(equal = 10))]
    pub verified_on: String,
}

/// Dataset split identity and selection provenance.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SplitProvenance {
    /// Stable split name.
    #[schemars(length(min = 1, max = 4096))]
    pub name: String,
    /// Lowercase SHA-256 of the ordered selected instance identities.
    #[schemars(regex(pattern = r"^[0-9a-f]{64}$"), length(equal = 64))]
    pub selection_sha256: String,
    /// Number of instances represented by this record.
    #[schemars(range(min = 1, max = 10000000))]
    pub instance_count: u32,
}

/// Independent reference and counterexample evidence for the grader revision.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BaselineEvidence {
    /// Independently versioned scorer identity.
    #[schemars(length(min = 1, max = 4096))]
    pub scoring_version: String,
    /// SHA-256 of the reference solution artifact.
    #[schemars(regex(pattern = r"^[0-9a-f]{64}$"), length(equal = 64))]
    pub reference_artifact_sha256: String,
    /// Whether the reference solution passed the exact scorer.
    pub reference_passed: bool,
    /// Number of named failure fixtures rejected.
    #[schemars(range(max = 1000000))]
    pub counterexamples_rejected: u32,
    /// Total named failure fixtures evaluated.
    #[schemars(range(min = 1, max = 1000000))]
    pub counterexamples_total: u32,
}

/// One exact build, evaluator, dataset, or runtime dependency pin.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DependencyPin {
    /// Package ecosystem or `in-tree`.
    #[schemars(length(min = 1, max = 4096))]
    pub ecosystem: String,
    /// Exact public package/component name.
    #[schemars(length(min = 1, max = 4096))]
    pub name: String,
    /// Exact version or immutable revision.
    #[schemars(length(min = 1, max = 4096))]
    pub version: String,
    /// Lowercase artifact SHA-256.
    #[schemars(regex(pattern = r"^[0-9a-f]{64}$"), length(equal = 64))]
    pub artifact_sha256: String,
    /// SPDX license expression.
    #[schemars(length(min = 1, max = 4096))]
    pub license: String,
}

/// Explicit public/holdout policy without recording private task identities.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExposurePolicy {
    /// Public or holdout state.
    pub status: ExposureStatus,
    /// First public date; present only for public tasks.
    #[schemars(regex(pattern = r"^[0-9]{4}-[0-9]{2}-[0-9]{2}$"), length(equal = 10))]
    pub first_public_on: Option<String>,
    /// Salted opaque holdout-set identity; present only for holdouts.
    #[schemars(regex(pattern = r"^[0-9a-f]{64}$"), length(equal = 64))]
    pub holdout_set_sha256: Option<String>,
}

/// Immutable evidence for one platform cell.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlatformEvidence {
    /// Public run identity.
    #[schemars(length(min = 1, max = 4096))]
    pub run_id: String,
    /// SHA-256 of the evidence artifact.
    #[schemars(regex(pattern = r"^[0-9a-f]{64}$"), length(equal = 64))]
    pub artifact_sha256: String,
    /// Exact booted kernel release for native evidence; absent otherwise.
    #[schemars(length(min = 1, max = 4096))]
    pub kernel_release: Option<String>,
    /// Evidence collection date.
    #[schemars(regex(pattern = r"^[0-9]{4}-[0-9]{2}-[0-9]{2}$"), length(equal = 10))]
    pub tested_on: String,
}

/// Workload support and adaptation state for an exact platform and architecture.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PortabilityRecord {
    /// Platform manifest identity.
    #[schemars(length(min = 1, max = 4096))]
    pub platform_id: String,
    /// Canonical architecture (`x86_64` or `aarch64` in v1).
    #[schemars(length(min = 1, max = 4096))]
    pub architecture: String,
    /// Qualified evidence status.
    pub status: PortabilityStatus,
    /// Exact, rebuilt, translated, or filtered workload relation.
    pub adaptation: AdaptationKind,
    /// Digest of adaptation bytes; absent for an exact workload.
    #[schemars(regex(pattern = r"^[0-9a-f]{64}$"), length(equal = 64))]
    pub adaptation_sha256: Option<String>,
    /// Semantic-parity evidence digest; required for non-exact adaptations.
    #[schemars(regex(pattern = r"^[0-9a-f]{64}$"), length(equal = 64))]
    pub semantic_parity_sha256: Option<String>,
    /// Immutable qualification evidence when build/simulated/native testing is claimed.
    pub evidence: Option<PlatformEvidence>,
    /// Required explanation for planned, simulated, adapted, or unsupported cells.
    #[schemars(length(min = 1, max = 4096))]
    pub limitation: Option<String>,
}

/// Host-local performance threshold and uncertainty calibration.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PerformanceCalibration {
    /// Platform manifest identity.
    #[schemars(length(min = 1, max = 4096))]
    pub platform_id: String,
    /// Canonical architecture.
    #[schemars(length(min = 1, max = 4096))]
    pub architecture: String,
    /// Opaque SHA-256 of the nonsecret host calibration class.
    #[schemars(regex(pattern = r"^[0-9a-f]{64}$"), length(equal = 64))]
    pub host_class_sha256: String,
    /// Exact performance metric identity.
    #[schemars(length(min = 1, max = 4096))]
    pub metric: String,
    /// Host-local decision threshold.
    pub threshold: f64,
    /// Nonnegative uncertainty on the same scale.
    pub uncertainty: f64,
    /// Number of paired calibration samples.
    #[schemars(range(min = 2, max = 1000000))]
    pub sample_count: u32,
    /// SHA-256 of immutable calibration evidence.
    #[schemars(regex(pattern = r"^[0-9a-f]{64}$"), length(equal = 64))]
    pub artifact_sha256: String,
}

/// Validity evidence for one exact workload content/scorer revision.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkloadValidity {
    /// Stable workload identity.
    #[schemars(length(min = 1, max = 4096))]
    pub workload_id: String,
    /// Exact content version.
    #[schemars(length(min = 1, max = 4096))]
    pub version: String,
    /// Pinned source and license facts.
    pub source: SourceProvenance,
    /// Exact dataset split or in-tree fixture selection.
    pub split: SplitProvenance,
    /// Protected grader baseline evidence.
    pub baseline: BaselineEvidence,
    /// Exact direct dependencies relevant to acquisition or grading.
    #[schemars(length(max = 256))]
    pub dependencies: Vec<DependencyPin>,
    /// Explicit public/holdout policy.
    pub exposure: ExposurePolicy,
    /// Platform and architecture cells, including unsupported cells.
    #[schemars(length(min = 1, max = 256))]
    pub portability: Vec<PortabilityRecord>,
    /// Host-local thresholds; empty means no performance claim.
    #[schemars(length(max = 256))]
    pub performance_calibrations: Vec<PerformanceCalibration>,
    /// Public, nonsecret limitations for this exact revision.
    #[schemars(length(min = 1, max = 256))]
    pub limitations: Vec<String>,
}

/// Closed v1 benchmark-validity registry.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkValidityRegistry {
    /// Registry format version; v1 is the only accepted value.
    #[schemars(range(min = 1, max = 1))]
    pub format_version: u32,
    /// Workload revisions sorted by `(workload_id, version)`.
    #[schemars(length(min = 1, max = 4096))]
    pub workloads: Vec<WorkloadValidity>,
}

impl BenchmarkValidityRegistry {
    /// Parse and validate a bounded closed registry document.
    pub fn from_json(bytes: &[u8]) -> Result<Self, RegistryError> {
        if bytes.len() > MAX_REGISTRY_BYTES {
            return Err(RegistryError::DocumentTooLarge);
        }
        let registry: Self =
            serde_json::from_slice(bytes).map_err(|_| RegistryError::InvalidJson)?;
        registry.validate()?;
        Ok(registry)
    }

    /// Load and cross-check the checked-in original-workload registry.
    pub fn built_in() -> Result<Self, RegistryError> {
        let registry = Self::from_json(BUILTIN_REGISTRY.as_bytes())?;
        if registry.workloads.len() != FIXTURE_IDS.len() {
            return Err(RegistryError::BuiltinMismatch);
        }
        for id in FIXTURE_IDS {
            let record = registry
                .workloads
                .iter()
                .find(|item| item.workload_id == id)
                .ok_or(RegistryError::BuiltinMismatch)?;
            let manifest =
                OriginalWorkloads::describe(id).map_err(|_| RegistryError::BuiltinMismatch)?;
            if record.version != manifest.version
                || record.source.kind != SourceKind::BuiltIn
                || record.source.revision != manifest.source_revision
                || record.source.license != manifest.license
                || record.source.content_sha256 != manifest.content_sha256
                || record.baseline.scoring_version != manifest.scoring_version
            {
                return Err(RegistryError::BuiltinMismatch);
            }
        }
        Ok(registry)
    }

    /// Validate all bounds and cross-field evidence invariants.
    pub fn validate(&self) -> Result<(), RegistryError> {
        if self.format_version != 1 {
            return Err(RegistryError::UnsupportedVersion);
        }
        if self.workloads.is_empty() || self.workloads.len() > MAX_REGISTRY_ENTRIES {
            return Err(RegistryError::InvalidField);
        }
        let mut previous: Option<(&str, &str)> = None;
        for workload in &self.workloads {
            workload.validate()?;
            let key = (workload.workload_id.as_str(), workload.version.as_str());
            if previous.is_some_and(|old| old >= key) {
                return Err(RegistryError::DuplicateOrUnordered);
            }
            previous = Some(key);
        }
        Ok(())
    }

    /// Find one exact workload revision.
    #[must_use]
    pub fn find(&self, workload_id: &str, version: &str) -> Option<&WorkloadValidity> {
        self.workloads
            .iter()
            .find(|item| item.workload_id == workload_id && item.version == version)
    }
}

impl WorkloadValidity {
    fn validate(&self) -> Result<(), RegistryError> {
        token(&self.workload_id)?;
        token(&self.version)?;
        self.source.validate()?;
        self.split.validate()?;
        self.baseline.validate()?;
        self.exposure.validate()?;
        if self.dependencies.len() > MAX_NESTED_ENTRIES
            || self.portability.is_empty()
            || self.portability.len() > MAX_NESTED_ENTRIES
            || self.performance_calibrations.len() > MAX_NESTED_ENTRIES
            || self.limitations.is_empty()
            || self.limitations.len() > MAX_NESTED_ENTRIES
        {
            return Err(RegistryError::InvalidField);
        }
        sorted_unique_by(&self.dependencies, |item| {
            (item.ecosystem.clone(), item.name.clone())
        })?;
        for dependency in &self.dependencies {
            dependency.validate()?;
        }
        sorted_unique_by(&self.portability, |item| {
            (item.platform_id.clone(), item.architecture.clone())
        })?;
        for cell in &self.portability {
            cell.validate()?;
        }
        sorted_unique_by(&self.performance_calibrations, |item| {
            (
                item.platform_id.clone(),
                item.architecture.clone(),
                item.metric.clone(),
            )
        })?;
        for calibration in &self.performance_calibrations {
            calibration.validate()?;
            let cell = self.portability.iter().find(|cell| {
                cell.platform_id == calibration.platform_id
                    && cell.architecture == calibration.architecture
            });
            if !cell.is_some_and(|cell| cell.status == PortabilityStatus::NativeTested) {
                return Err(RegistryError::InconsistentEvidence);
            }
        }
        for limitation in &self.limitations {
            public_text(limitation)?;
        }
        Ok(())
    }
}

impl SourceProvenance {
    fn validate(&self) -> Result<(), RegistryError> {
        public_text(&self.locator)?;
        match self.kind {
            SourceKind::BuiltIn => {
                let namespace = self
                    .locator
                    .strip_prefix("builtin:")
                    .ok_or(RegistryError::InconsistentEvidence)?;
                token(namespace)?;
            }
            SourceKind::Imported => {
                let remainder = self
                    .locator
                    .strip_prefix("https://")
                    .ok_or(RegistryError::InconsistentEvidence)?;
                let authority = remainder.split('/').next().unwrap_or_default();
                if authority.is_empty() || authority.contains('@') || self.locator.contains('#') {
                    return Err(RegistryError::InconsistentEvidence);
                }
            }
        }
        token(&self.revision)?;
        public_text(&self.license)?;
        digest(&self.content_sha256)?;
        date(&self.verified_on)
    }
}

impl SplitProvenance {
    fn validate(&self) -> Result<(), RegistryError> {
        token(&self.name)?;
        digest(&self.selection_sha256)?;
        if self.instance_count == 0 || self.instance_count > 10_000_000 {
            return Err(RegistryError::InvalidField);
        }
        Ok(())
    }
}

impl BaselineEvidence {
    fn validate(&self) -> Result<(), RegistryError> {
        token(&self.scoring_version)?;
        digest(&self.reference_artifact_sha256)?;
        if !self.reference_passed
            || self.counterexamples_total == 0
            || self.counterexamples_total > 1_000_000
            || self.counterexamples_rejected != self.counterexamples_total
        {
            return Err(RegistryError::InconsistentEvidence);
        }
        Ok(())
    }
}

impl DependencyPin {
    fn validate(&self) -> Result<(), RegistryError> {
        token(&self.ecosystem)?;
        token(&self.name)?;
        token(&self.version)?;
        digest(&self.artifact_sha256)?;
        public_text(&self.license)
    }
}

impl ExposurePolicy {
    fn validate(&self) -> Result<(), RegistryError> {
        match self.status {
            ExposureStatus::Public => {
                date(
                    self.first_public_on
                        .as_deref()
                        .ok_or(RegistryError::InconsistentEvidence)?,
                )?;
                if self.holdout_set_sha256.is_some() {
                    return Err(RegistryError::InconsistentEvidence);
                }
            }
            ExposureStatus::Holdout => {
                if self.first_public_on.is_some() {
                    return Err(RegistryError::InconsistentEvidence);
                }
                digest(
                    self.holdout_set_sha256
                        .as_deref()
                        .ok_or(RegistryError::InconsistentEvidence)?,
                )?;
            }
        }
        Ok(())
    }
}

impl PortabilityRecord {
    fn validate(&self) -> Result<(), RegistryError> {
        token(&self.platform_id)?;
        if !matches!(self.architecture.as_str(), "x86_64" | "aarch64") {
            return Err(RegistryError::InvalidField);
        }
        match self.adaptation {
            AdaptationKind::None => {
                if self.adaptation_sha256.is_some() || self.semantic_parity_sha256.is_some() {
                    return Err(RegistryError::InconsistentEvidence);
                }
            }
            AdaptationKind::Rebuilt | AdaptationKind::Translated | AdaptationKind::Filtered => {
                digest(
                    self.adaptation_sha256
                        .as_deref()
                        .ok_or(RegistryError::InconsistentEvidence)?,
                )?;
                digest(
                    self.semantic_parity_sha256
                        .as_deref()
                        .ok_or(RegistryError::InconsistentEvidence)?,
                )?;
            }
        }
        match self.status {
            PortabilityStatus::Planned | PortabilityStatus::Unsupported => {
                if self.evidence.is_some() {
                    return Err(RegistryError::InconsistentEvidence);
                }
            }
            PortabilityStatus::BuildOnly | PortabilityStatus::Simulated => {
                let evidence = self
                    .evidence
                    .as_ref()
                    .ok_or(RegistryError::InconsistentEvidence)?;
                evidence.validate(false)?;
            }
            PortabilityStatus::NativeTested => {
                let evidence = self
                    .evidence
                    .as_ref()
                    .ok_or(RegistryError::InconsistentEvidence)?;
                evidence.validate(true)?;
            }
        }
        if matches!(
            self.status,
            PortabilityStatus::Planned
                | PortabilityStatus::Simulated
                | PortabilityStatus::Unsupported
        ) || self.adaptation != AdaptationKind::None
        {
            public_text(
                self.limitation
                    .as_deref()
                    .ok_or(RegistryError::InconsistentEvidence)?,
            )?;
        } else if let Some(limitation) = &self.limitation {
            public_text(limitation)?;
        }
        Ok(())
    }
}

impl PlatformEvidence {
    fn validate(&self, native: bool) -> Result<(), RegistryError> {
        token(&self.run_id)?;
        digest(&self.artifact_sha256)?;
        date(&self.tested_on)?;
        match (&self.kernel_release, native) {
            (Some(release), true) => token(release),
            (None, false) => Ok(()),
            _ => Err(RegistryError::InconsistentEvidence),
        }
    }
}

impl PerformanceCalibration {
    fn validate(&self) -> Result<(), RegistryError> {
        token(&self.platform_id)?;
        if !matches!(self.architecture.as_str(), "x86_64" | "aarch64") {
            return Err(RegistryError::InvalidField);
        }
        digest(&self.host_class_sha256)?;
        token(&self.metric)?;
        if !self.threshold.is_finite()
            || self.threshold <= 0.0
            || !self.uncertainty.is_finite()
            || self.uncertainty < 0.0
            || self.sample_count < 2
            || self.sample_count > 1_000_000
        {
            return Err(RegistryError::InvalidField);
        }
        digest(&self.artifact_sha256)
    }
}

fn sorted_unique_by<T, K: Ord>(items: &[T], key: impl Fn(&T) -> K) -> Result<(), RegistryError> {
    if items.windows(2).any(|pair| key(&pair[0]) >= key(&pair[1])) {
        return Err(RegistryError::DuplicateOrUnordered);
    }
    Ok(())
}

fn token(value: &str) -> Result<(), RegistryError> {
    public_text(value)?;
    if value.chars().any(char::is_whitespace) {
        return Err(RegistryError::InvalidField);
    }
    Ok(())
}

fn public_text(value: &str) -> Result<(), RegistryError> {
    let folded = value.to_ascii_lowercase();
    let contains_sensitive_assignment = [
        "authorization:",
        "api_key=",
        "apikey=",
        "password=",
        "token=",
    ]
    .iter()
    .any(|marker| folded.contains(marker));
    let contains_absolute_path = value.split_whitespace().any(|word| {
        let word = word.trim_start_matches(['(', '[', '{', '\"', '\'']);
        word.starts_with('/')
            || word.starts_with("~/")
            || (word.len() >= 3
                && word.as_bytes()[1] == b':'
                && matches!(word.as_bytes()[2], b'/' | b'\\'))
    });
    if value.is_empty()
        || value.len() > MAX_PUBLIC_TEXT_BYTES
        || value.trim() != value
        || value.chars().any(char::is_control)
        || value.starts_with('/')
        || value.starts_with("~/")
        || value.starts_with("file:")
        || contains_sensitive_assignment
        || contains_absolute_path
    {
        return Err(RegistryError::InvalidField);
    }
    Ok(())
}

fn digest(value: &str) -> Result<(), RegistryError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(RegistryError::InvalidField);
    }
    Ok(())
}

fn date(value: &str) -> Result<(), RegistryError> {
    let bytes = value.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes
            .iter()
            .enumerate()
            .any(|(index, byte)| index != 4 && index != 7 && !byte.is_ascii_digit())
    {
        return Err(RegistryError::InvalidField);
    }
    let year: u32 = value[0..4]
        .parse()
        .map_err(|_| RegistryError::InvalidField)?;
    let month: u32 = value[5..7]
        .parse()
        .map_err(|_| RegistryError::InvalidField)?;
    let day: u32 = value[8..10]
        .parse()
        .map_err(|_| RegistryError::InvalidField)?;
    if year < 1970 {
        return Err(RegistryError::InvalidField);
    }
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let maximum = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return Err(RegistryError::InvalidField),
    };
    if day == 0 || day > maximum {
        return Err(RegistryError::InvalidField);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> BenchmarkValidityRegistry {
        BenchmarkValidityRegistry::from_json(BUILTIN_REGISTRY.as_bytes()).unwrap()
    }

    fn zero_digest() -> String {
        "0".repeat(64)
    }

    #[test]
    fn every_public_error_is_content_free_and_parse_bounds_fail_closed() {
        let cases = [
            (RegistryError::DocumentTooLarge, "exceeds"),
            (RegistryError::InvalidJson, "JSON"),
            (RegistryError::UnsupportedVersion, "version"),
            (RegistryError::InvalidField, "field"),
            (RegistryError::DuplicateOrUnordered, "duplicated"),
            (RegistryError::InconsistentEvidence, "inconsistent"),
            (RegistryError::BuiltinMismatch, "built-in"),
        ];
        for (error, expected) in cases {
            assert!(error.to_string().contains(expected));
        }
        assert_eq!(
            BenchmarkValidityRegistry::from_json(b"not-json"),
            Err(RegistryError::InvalidJson)
        );
        let mut invalid = registry();
        invalid.format_version = 2;
        assert_eq!(invalid.validate(), Err(RegistryError::UnsupportedVersion));
        invalid.format_version = 1;
        invalid.workloads.clear();
        assert_eq!(invalid.validate(), Err(RegistryError::InvalidField));
        invalid.workloads = vec![registry().workloads[0].clone(); MAX_REGISTRY_ENTRIES + 1];
        assert_eq!(invalid.validate(), Err(RegistryError::InvalidField));
    }

    #[test]
    fn source_dependency_and_collection_bounds_are_strict() {
        let mut valid = registry();
        let record = &mut valid.workloads[0];
        record.source.kind = SourceKind::Imported;
        record.source.locator = "https://example.invalid/dataset".into();
        record.dependencies = vec![DependencyPin {
            ecosystem: "cargo".into(),
            name: "example".into(),
            version: "1.0.0".into(),
            artifact_sha256: zero_digest(),
            license: "MIT OR Apache-2.0".into(),
        }];
        valid.validate().unwrap();

        for locator in [
            "http://example.invalid/source",
            "https://user@example.invalid/source",
            "https://example.invalid/source#mutable",
            "https://",
        ] {
            let mut bad = valid.clone();
            bad.workloads[0].source.locator = locator.into();
            assert_eq!(bad.validate(), Err(RegistryError::InconsistentEvidence));
        }
        let mut bad_builtin = registry();
        bad_builtin.workloads[0].source.locator = "https://example.invalid".into();
        assert_eq!(
            bad_builtin.validate(),
            Err(RegistryError::InconsistentEvidence)
        );
        let mut bad_dependency = valid.clone();
        bad_dependency.workloads[0].dependencies[0].artifact_sha256 = "bad".into();
        assert_eq!(bad_dependency.validate(), Err(RegistryError::InvalidField));
        let mut duplicate = valid.clone();
        let repeated = duplicate.workloads[0].dependencies[0].clone();
        duplicate.workloads[0].dependencies.push(repeated);
        assert_eq!(
            duplicate.validate(),
            Err(RegistryError::DuplicateOrUnordered)
        );
        let mut excessive = valid;
        excessive.workloads[0].dependencies =
            vec![excessive.workloads[0].dependencies[0].clone(); MAX_NESTED_ENTRIES + 1];
        assert_eq!(excessive.validate(), Err(RegistryError::InvalidField));
    }

    #[test]
    fn exposure_evidence_and_portability_variants_are_exhaustive() {
        let mut public_missing_date = registry();
        public_missing_date.workloads[0].exposure.first_public_on = None;
        assert_eq!(
            public_missing_date.validate(),
            Err(RegistryError::InconsistentEvidence)
        );
        let mut holdout = registry();
        holdout.workloads[0].exposure = ExposurePolicy {
            status: ExposureStatus::Holdout,
            first_public_on: None,
            holdout_set_sha256: Some(zero_digest()),
        };
        holdout.validate().unwrap();
        holdout.workloads[0].exposure.holdout_set_sha256 = None;
        assert_eq!(holdout.validate(), Err(RegistryError::InconsistentEvidence));

        let mut qualified = registry();
        let cell = &mut qualified.workloads[0].portability[0];
        cell.status = PortabilityStatus::BuildOnly;
        cell.evidence = Some(PlatformEvidence {
            run_id: "build-run".into(),
            artifact_sha256: zero_digest(),
            kernel_release: None,
            tested_on: "2026-09-08".into(),
        });
        cell.limitation = None;
        qualified.validate().unwrap();
        qualified.workloads[0].portability[0].status = PortabilityStatus::Simulated;
        qualified.workloads[0].portability[0].limitation = Some("Emulated only.".into());
        qualified.validate().unwrap();
        qualified.workloads[0].portability[0].status = PortabilityStatus::Unsupported;
        assert_eq!(
            qualified.validate(),
            Err(RegistryError::InconsistentEvidence)
        );

        for adaptation in [
            AdaptationKind::Rebuilt,
            AdaptationKind::Translated,
            AdaptationKind::Filtered,
        ] {
            let mut adapted = registry();
            let cell = &mut adapted.workloads[0].portability[0];
            cell.adaptation = adaptation;
            cell.adaptation_sha256 = Some("1".repeat(64));
            cell.semantic_parity_sha256 = Some("2".repeat(64));
            adapted.validate().unwrap();
        }
    }

    #[test]
    fn primitive_privacy_date_and_calibration_boundaries_are_rejected() {
        for value in [
            "",
            " leading",
            "trailing ",
            "line\nbreak",
            "/private/path",
            "at /private/path",
            "C:\\private\\path",
            "api_key=not-public",
            "Authorization: not-public",
        ] {
            assert_eq!(public_text(value), Err(RegistryError::InvalidField));
        }
        assert_eq!(
            public_text(&"x".repeat(MAX_PUBLIC_TEXT_BYTES + 1)),
            Err(RegistryError::InvalidField)
        );
        assert_eq!(token("two words"), Err(RegistryError::InvalidField));
        assert_eq!(digest("ABC"), Err(RegistryError::InvalidField));
        for invalid in [
            "1969-12-31",
            "2024/02/29",
            "2024-13-01",
            "2024-02-30",
            "2024-00-01",
            "2024-01-00",
        ] {
            assert_eq!(date(invalid), Err(RegistryError::InvalidField));
        }
        date("2024-02-29").unwrap();
        date("2026-01-31").unwrap();

        let mut calibration = PerformanceCalibration {
            platform_id: "ubuntu-24.04".into(),
            architecture: "riscv64".into(),
            host_class_sha256: zero_digest(),
            metric: "speedup".into(),
            threshold: 1.0,
            uncertainty: 0.1,
            sample_count: 2,
            artifact_sha256: zero_digest(),
        };
        assert_eq!(calibration.validate(), Err(RegistryError::InvalidField));
        calibration.architecture = "x86_64".into();
        for (threshold, uncertainty, samples) in [
            (0.0, 0.1, 2),
            (1.0, -0.1, 2),
            (1.0, 0.1, 1),
            (1.0, 0.1, 1_000_001),
        ] {
            calibration.threshold = threshold;
            calibration.uncertainty = uncertainty;
            calibration.sample_count = samples;
            assert_eq!(calibration.validate(), Err(RegistryError::InvalidField));
        }
    }

    #[test]
    fn duplicate_calibrations_and_partial_adaptations_cannot_create_support() {
        let mut duplicate = registry();
        let cell = &mut duplicate.workloads[0].portability[1];
        cell.status = PortabilityStatus::NativeTested;
        cell.evidence = Some(PlatformEvidence {
            run_id: "native-run".into(),
            artifact_sha256: zero_digest(),
            kernel_release: Some("6.8.0-generic".into()),
            tested_on: "2026-09-08".into(),
        });
        let calibration = PerformanceCalibration {
            platform_id: "ubuntu-24.04".into(),
            architecture: "x86_64".into(),
            host_class_sha256: "1".repeat(64),
            metric: "paired-speedup".into(),
            threshold: 1.1,
            uncertainty: 0.05,
            sample_count: 8,
            artifact_sha256: "2".repeat(64),
        };
        duplicate.workloads[0].performance_calibrations = vec![calibration.clone(), calibration];
        assert_eq!(
            duplicate.validate(),
            Err(RegistryError::DuplicateOrUnordered)
        );

        let changes: [fn(&mut PortabilityRecord); 6] = [
            |cell: &mut PortabilityRecord| cell.architecture = "riscv64".into(),
            |cell: &mut PortabilityRecord| cell.adaptation_sha256 = Some(zero_digest()),
            |cell: &mut PortabilityRecord| {
                cell.adaptation = AdaptationKind::Rebuilt;
                cell.adaptation_sha256 = None;
                cell.semantic_parity_sha256 = Some(zero_digest());
            },
            |cell: &mut PortabilityRecord| {
                cell.adaptation = AdaptationKind::Rebuilt;
                cell.adaptation_sha256 = Some(zero_digest());
                cell.semantic_parity_sha256 = None;
            },
            |cell: &mut PortabilityRecord| cell.limitation = None,
            |cell: &mut PortabilityRecord| cell.limitation = Some("/private/path".into()),
        ];
        for change in changes {
            let mut invalid = registry();
            change(&mut invalid.workloads[0].portability[0]);
            assert!(invalid.validate().is_err());
        }

        let mut contradictory_holdout = registry();
        contradictory_holdout.workloads[0].exposure = ExposurePolicy {
            status: ExposureStatus::Holdout,
            first_public_on: Some("2026-09-08".into()),
            holdout_set_sha256: Some(zero_digest()),
        };
        assert_eq!(
            contradictory_holdout.validate(),
            Err(RegistryError::InconsistentEvidence)
        );
    }
}
