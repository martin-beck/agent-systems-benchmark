// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Versioned, content-addressed measurement catalog semantics.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::Aggregation;

/// Current measurement catalog schema generation.
pub const MEASUREMENT_CATALOG_SCHEMA_V1: u16 = 1;
/// Maximum semantic groups accepted in one catalog.
pub const MAX_MEASUREMENT_GROUPS: usize = 7;
/// Maximum definitions accepted in one catalog.
pub const MAX_MEASUREMENTS: usize = 128;
/// Maximum UTF-8 bytes accepted in a public label or description.
pub const MAX_MEASUREMENT_TEXT_BYTES: usize = 256;
/// Maximum encoded bytes accepted from an untrusted catalog transport.
pub const MAX_MEASUREMENT_CATALOG_WIRE_BYTES: usize = 256 * 1024;

const CATALOG_DIGEST_DOMAIN: &[u8] = b"asb-measurement-catalog-v1\0";

/// Stable semantic group identifiers. Their serialized names are public API.
#[derive(
    Clone, Copy, Debug, Deserialize, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementGroupId {
    /// CPU, memory, storage, and network resource consumption.
    SystemResources,
    /// Scheduling pressure, throttling, and explicitly non-causal contention evidence.
    SchedulingContention,
    /// Queue, readiness, first-response, and completion latency.
    Latency,
    /// Independent correctness and reliability outcomes.
    QualityReliability,
    /// Per-agent and mixed-workload fairness evidence.
    Fairness,
    /// Provider-reported or explicitly unavailable cost evidence.
    Cost,
    /// Harness, collector, replay, and experiment provenance evidence.
    Provenance,
}

/// One stable semantic group and its bounded public explanation.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MeasurementGroup {
    /// Stable machine identity.
    pub id: MeasurementGroupId,
    /// Short public display-independent label.
    #[schemars(regex(pattern = r"^[ -?A-~]+$"), length(min = 1, max = 256))]
    pub label: String,
    /// Public semantic boundary for the group.
    #[schemars(regex(pattern = r"^[ -?A-~]+$"), length(min = 1, max = 256))]
    pub description: String,
}

/// Physical or logical quantity represented by a measurement.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementQuantity {
    /// Duration or cumulative CPU/pressure time.
    Time,
    /// Byte count or instantaneous byte quantity.
    Bytes,
    /// Discrete event count.
    Count,
    /// Dimensionless ratio.
    Ratio,
    /// Independently defined normalized score.
    Score,
    /// Monetary value with separately bound currency provenance.
    Currency,
}

/// Measurement ownership scope.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementScope {
    /// One operating-system process.
    Process,
    /// One delegated cgroup-v2 subtree.
    Cgroup,
    /// One complete benchmark attempt.
    Attempt,
    /// One complete run or comparison set.
    Run,
}

/// Closed source vocabulary; no host path or caller string enters the catalog.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementSource {
    /// Linux procfs counters read by `asb-metrics`.
    AsbMetricsProcfs,
    /// Linux cgroup-v2 counters read by `asb-metrics`.
    AsbMetricsCgroupV2,
    /// Durable ASB runner journal evidence.
    AsbRunnerJournal,
    /// Independently versioned grader output.
    IndependentGrader,
    /// Provider-reported usage with explicit absence semantics.
    ProviderUsage,
    /// Optional CSB-derived evidence that is not baseline-qualified.
    OptionalCsb,
}

/// Closed, path-free identity for an exact runtime descriptor source.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementSourceIdentity {
    /// Process counters from procfs stat.
    ProcfsProcessStat,
    /// Process counters from procfs IO.
    ProcfsProcessIo,
    /// Cgroup-v2 CPU statistics.
    CgroupV2CpuStat,
    /// Cgroup-v2 current memory.
    CgroupV2MemoryCurrent,
    /// Cgroup-v2 peak memory.
    CgroupV2MemoryPeak,
    /// Cgroup-v2 memory statistics.
    CgroupV2MemoryStat,
    /// Cgroup-v2 IO statistics.
    CgroupV2IoStat,
    /// Cgroup-v2 CPU pressure.
    CgroupV2CpuPressure,
    /// Cgroup-v2 memory pressure.
    CgroupV2MemoryPressure,
    /// Cgroup-v2 IO pressure.
    CgroupV2IoPressure,
}

impl MeasurementSourceIdentity {
    /// Exact source spelling used by runtime metric descriptors.
    #[must_use]
    pub const fn runtime_descriptor(self) -> &'static str {
        match self {
            Self::ProcfsProcessStat => "procfs:/proc/[pid]/stat",
            Self::ProcfsProcessIo => "procfs:/proc/[pid]/io",
            Self::CgroupV2CpuStat => "cgroup2:cpu.stat",
            Self::CgroupV2MemoryCurrent => "cgroup2:memory.current",
            Self::CgroupV2MemoryPeak => "cgroup2:memory.peak",
            Self::CgroupV2MemoryStat => "cgroup2:memory.stat",
            Self::CgroupV2IoStat => "cgroup2:io.stat",
            Self::CgroupV2CpuPressure => "cgroup2:cpu.pressure",
            Self::CgroupV2MemoryPressure => "cgroup2:memory.pressure",
            Self::CgroupV2IoPressure => "cgroup2:io.pressure",
        }
    }
}

/// Evidence qualification attached to a measurement source.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementQualification {
    /// Directly implemented and contract-tested by ASB.
    Implemented,
    /// Defined but not yet supported by an authoritative implementation.
    Unqualified,
}

/// Closed provenance for one definition.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MeasurementProvenance {
    /// Authoritative component/source class.
    pub source: MeasurementSource,
    /// Current evidence qualification.
    pub qualification: MeasurementQualification,
}

/// Why one execution mode cannot provide a measurement.
#[derive(
    Clone, Copy, Debug, Deserialize, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementUnavailableReason {
    /// No qualified collector implements this definition.
    NotQualified,
    /// The execution platform cannot expose the required evidence.
    PlatformUnsupported,
    /// Required permissions were not granted.
    PermissionRequired,
    /// The measurement does not have meaning in this mode.
    NotApplicable,
}

/// Explicit support state for live or replay execution.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum MeasurementModeSupport {
    /// The authoritative implementation can attempt this measurement.
    Supported,
    /// No value may be inferred; the reason is closed and explicit.
    Unsupported {
        /// Why the value is unavailable.
        reason: MeasurementUnavailableReason,
    },
}

/// Supported operating-system family.
#[derive(
    Clone, Copy, Debug, Deserialize, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementOperatingSystem {
    /// Linux userspace and kernel interfaces.
    Linux,
}

/// Supported machine architecture.
#[derive(
    Clone, Copy, Debug, Deserialize, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementArchitecture {
    /// 64-bit x86.
    X86_64,
    /// 64-bit Arm.
    Aarch64,
}

/// Required platform facility.
#[derive(
    Clone, Copy, Debug, Deserialize, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementPlatformFeature {
    /// A mounted, readable Linux procfs.
    Procfs,
    /// A delegated, readable unified cgroup-v2 hierarchy.
    CgroupV2,
}

/// Closed platform constraint for a measurement definition.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MeasurementPlatform {
    /// Required operating-system family.
    pub operating_system: MeasurementOperatingSystem,
    /// Architectures with implemented portable parsing.
    #[schemars(length(min = 1, max = 8))]
    pub architectures: Vec<MeasurementArchitecture>,
    /// Required kernel/userspace interfaces.
    #[schemars(length(min = 1, max = 8))]
    pub required_features: Vec<MeasurementPlatformFeature>,
}

/// Qualitative collection overhead class with an explicit interval floor.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementOverheadClass {
    /// Direct bounded counter reads; actual elapsed overhead remains recorded per collection.
    Low,
    /// Additional sampling or processing with a material perturbation risk.
    Material,
}

/// Static collection-overhead boundary.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MeasurementOverhead {
    /// Reviewed qualitative class.
    pub class: MeasurementOverheadClass,
    /// Smallest supported interval between samples.
    #[schemars(range(min = 1))]
    pub minimum_interval_ns: u64,
    /// Whether collection requires elevated privilege.
    pub requires_privilege: bool,
}

/// Closed limits on valid interpretation.
#[derive(
    Clone, Copy, Debug, Deserialize, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementEvidenceLimit {
    /// Missing evidence remains unavailable and is never zero-filled.
    MissingIsUnavailable,
    /// A counter does not by itself establish causation.
    NotCausal,
    /// Values are comparable only under compatible experiment provenance.
    RequiresComparableExperiment,
    /// Collection elapsed time must accompany the samples.
    CollectorOverheadRecorded,
}

/// One selectable, renderer-independent measurement definition.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MeasurementDefinition {
    /// Stable dotted identifier.
    #[schemars(
        regex(pattern = r"^[a-z][a-z0-9]*(?:[._][a-z0-9]+)+$"),
        length(min = 3, max = 128)
    )]
    pub id: String,
    /// Short public name, unique under ASCII case folding.
    #[schemars(regex(pattern = r"^[ -?A-~]+$"), length(min = 1, max = 256))]
    pub name: String,
    /// Public evidence meaning, containing no runtime or host material.
    #[schemars(regex(pattern = r"^[ -?A-~]+$"), length(min = 1, max = 256))]
    pub description: String,
    /// Semantic group identity.
    pub group: MeasurementGroupId,
    /// Quantity used to validate the unit.
    pub quantity: MeasurementQuantity,
    /// Exact UCUM-style unit.
    #[schemars(length(min = 1, max = 32))]
    pub unit: String,
    /// Aggregation semantics.
    pub aggregation: Aggregation,
    /// Ownership scope.
    pub scope: MeasurementScope,
    /// Source and qualification.
    pub provenance: MeasurementProvenance,
    /// Closed identity mapping exactly to the runtime descriptor source string.
    pub source_identity: MeasurementSourceIdentity,
    /// Nominal runtime descriptor resolution in nanoseconds; zero means unspecified.
    pub resolution_ns: u64,
    /// Collection overhead contract.
    pub overhead: MeasurementOverhead,
    /// Live-provider execution support.
    pub live: MeasurementModeSupport,
    /// Recorded-response replay execution support.
    pub replay: MeasurementModeSupport,
    /// Explicit platform constraints.
    #[schemars(length(min = 1, max = 8))]
    pub platforms: Vec<MeasurementPlatform>,
    /// Explicit interpretation limits.
    #[schemars(length(min = 1, max = 8))]
    pub evidence_limits: Vec<MeasurementEvidenceLimit>,
}

/// Authoritative v1 catalog with a deterministic content address.
///
/// Direct generic deserialization is intentionally unavailable; untrusted bytes must use the
/// bounded constructors.
///
/// ```compile_fail
/// use asb_protocol::MeasurementCatalogV1;
/// let _: MeasurementCatalogV1 = serde_json::from_str("{}").unwrap();
/// ```
#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(deny_unknown_fields)]
pub struct MeasurementCatalogV1 {
    /// Closed schema generation; exactly 1.
    #[schemars(range(min = 1, max = 1))]
    pub schema_version: u16,
    /// SHA-256 of the canonical catalog contents excluding this field.
    #[schemars(regex(pattern = r"^[0-9a-f]{64}$"), length(equal = 64))]
    pub catalog_sha256: String,
    /// Stable groups in enum order.
    #[schemars(length(max = 7))]
    pub groups: Vec<MeasurementGroup>,
    /// Stable definitions in ascending ID order.
    #[schemars(length(max = 128))]
    pub measurements: Vec<MeasurementDefinition>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MeasurementCatalogWire {
    schema_version: u16,
    catalog_sha256: String,
    groups: Vec<MeasurementGroup>,
    measurements: Vec<MeasurementDefinition>,
}

#[derive(Serialize)]
struct CatalogAddress<'a> {
    schema_version: u16,
    groups: &'a [MeasurementGroup],
    measurements: &'a [MeasurementDefinition],
}

impl MeasurementCatalogV1 {
    /// Canonicalize, content-address, and validate a catalog.
    pub fn new(
        mut groups: Vec<MeasurementGroup>,
        mut measurements: Vec<MeasurementDefinition>,
    ) -> Result<Self, MeasurementCatalogError> {
        preflight_catalog(&groups, &measurements)?;
        groups.sort_by_key(|group| group.id);
        for measurement in &mut measurements {
            for platform in &mut measurement.platforms {
                platform.architectures.sort();
                platform.required_features.sort();
            }
            measurement.platforms.sort();
            measurement.evidence_limits.sort();
        }
        measurements.sort_by(|left, right| left.id.cmp(&right.id));
        let mut catalog = Self {
            schema_version: MEASUREMENT_CATALOG_SCHEMA_V1,
            catalog_sha256: String::new(),
            groups,
            measurements,
        };
        catalog.catalog_sha256 = catalog.computed_digest()?;
        catalog.validate()?;
        Ok(catalog)
    }

    /// Decode and validate an untrusted in-memory catalog under a hard wire-size ceiling.
    pub fn from_slice_bounded(bytes: &[u8]) -> Result<Self, MeasurementCatalogError> {
        if bytes.len() > MAX_MEASUREMENT_CATALOG_WIRE_BYTES {
            return Err(MeasurementCatalogError::WireTooLarge);
        }
        let wire: MeasurementCatalogWire =
            serde_json::from_slice(bytes).map_err(|_| MeasurementCatalogError::InvalidWire)?;
        let catalog = Self {
            schema_version: wire.schema_version,
            catalog_sha256: wire.catalog_sha256,
            groups: wire.groups,
            measurements: wire.measurements,
        };
        catalog.validate()?;
        Ok(catalog)
    }

    /// Read, decode, and validate an untrusted catalog without an unbounded read or allocation.
    pub fn from_reader_bounded(reader: impl Read) -> Result<Self, MeasurementCatalogError> {
        let limit = u64::try_from(MAX_MEASUREMENT_CATALOG_WIRE_BYTES)
            .map_err(|_| MeasurementCatalogError::WireTooLarge)?;
        let mut bytes = Vec::with_capacity(MAX_MEASUREMENT_CATALOG_WIRE_BYTES.min(8192));
        reader
            .take(limit.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|_| MeasurementCatalogError::InvalidWire)?;
        Self::from_slice_bounded(&bytes)
    }

    /// Validate every structural, semantic, privacy, and content-address invariant.
    pub fn validate(&self) -> Result<(), MeasurementCatalogError> {
        if self.schema_version != MEASUREMENT_CATALOG_SCHEMA_V1 {
            return Err(MeasurementCatalogError::UnsupportedSchemaVersion(
                self.schema_version,
            ));
        }
        if self.groups.len() > MAX_MEASUREMENT_GROUPS {
            return Err(MeasurementCatalogError::TooManyGroups);
        }
        if self.measurements.len() > MAX_MEASUREMENTS {
            return Err(MeasurementCatalogError::TooManyMeasurements);
        }

        let mut group_ids = BTreeSet::new();
        let mut group_labels = BTreeSet::new();
        let mut previous_group = None;
        for group in &self.groups {
            validate_public_text(&group.label)?;
            validate_public_text(&group.description)?;
            if !group_ids.insert(group.id) {
                return Err(MeasurementCatalogError::DuplicateGroup);
            }
            if previous_group.is_some_and(|previous| previous >= group.id) {
                return Err(MeasurementCatalogError::NonCanonicalOrder);
            }
            previous_group = Some(group.id);
            if !group_labels.insert(group.label.to_ascii_lowercase()) {
                return Err(MeasurementCatalogError::AmbiguousName);
            }
        }

        let mut definitions: BTreeMap<&str, (&str, MeasurementQuantity)> = BTreeMap::new();
        let mut names = BTreeSet::new();
        let mut previous_id: Option<&str> = None;
        for measurement in &self.measurements {
            validate_identifier(&measurement.id)?;
            validate_public_text(&measurement.name)?;
            validate_public_text(&measurement.description)?;
            validate_unit(measurement.quantity, &measurement.unit)?;
            if let Some((unit, quantity)) =
                definitions.insert(&measurement.id, (&measurement.unit, measurement.quantity))
            {
                return if unit != measurement.unit || quantity != measurement.quantity {
                    Err(MeasurementCatalogError::IncompatibleUnit)
                } else {
                    Err(MeasurementCatalogError::DuplicateMeasurement)
                };
            }
            if previous_id.is_some_and(|previous| previous >= measurement.id.as_str()) {
                return Err(MeasurementCatalogError::NonCanonicalOrder);
            }
            previous_id = Some(&measurement.id);
            if !names.insert(measurement.name.to_ascii_lowercase()) {
                return Err(MeasurementCatalogError::AmbiguousName);
            }
            if !group_ids.contains(&measurement.group) {
                return Err(MeasurementCatalogError::UnknownGroup);
            }
            validate_measurement(measurement)?;
        }

        if group_ids.iter().any(|group| {
            !self
                .measurements
                .iter()
                .any(|measurement| measurement.group == *group)
        }) {
            return Err(MeasurementCatalogError::EmptyGroup);
        }

        if self.catalog_sha256.len() != 64
            || !self
                .catalog_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || self.catalog_sha256 != self.computed_digest()?
        {
            return Err(MeasurementCatalogError::DigestMismatch);
        }
        Ok(())
    }

    fn computed_digest(&self) -> Result<String, MeasurementCatalogError> {
        let encoded = serde_json::to_vec(&CatalogAddress {
            schema_version: self.schema_version,
            groups: &self.groups,
            measurements: &self.measurements,
        })
        .map_err(|_| MeasurementCatalogError::Serialization)?;
        let mut digest = Sha256::new();
        digest.update(CATALOG_DIGEST_DOMAIN);
        digest.update(encoded);
        Ok(format!("{:x}", digest.finalize()))
    }
}

fn preflight_catalog(
    groups: &[MeasurementGroup],
    measurements: &[MeasurementDefinition],
) -> Result<(), MeasurementCatalogError> {
    if groups.len() > MAX_MEASUREMENT_GROUPS {
        return Err(MeasurementCatalogError::TooManyGroups);
    }
    if measurements.len() > MAX_MEASUREMENTS {
        return Err(MeasurementCatalogError::TooManyMeasurements);
    }
    for group in groups {
        validate_public_text(&group.label)?;
        validate_public_text(&group.description)?;
    }
    for measurement in measurements {
        validate_identifier(&measurement.id)?;
        validate_public_text(&measurement.name)?;
        validate_public_text(&measurement.description)?;
        validate_unit(measurement.quantity, &measurement.unit)?;
        if measurement.platforms.is_empty() || measurement.platforms.len() > 8 {
            return Err(MeasurementCatalogError::InvalidPlatform);
        }
        if measurement.platforms.iter().any(|platform| {
            platform.architectures.is_empty()
                || platform.architectures.len() > 8
                || platform.required_features.is_empty()
                || platform.required_features.len() > 8
        }) {
            return Err(MeasurementCatalogError::InvalidPlatform);
        }
        if measurement.evidence_limits.is_empty() || measurement.evidence_limits.len() > 8 {
            return Err(MeasurementCatalogError::InvalidEvidenceLimits);
        }
        if measurement.overhead.minimum_interval_ns == 0 {
            return Err(MeasurementCatalogError::InvalidOverhead);
        }
    }
    Ok(())
}

fn validate_measurement(
    measurement: &MeasurementDefinition,
) -> Result<(), MeasurementCatalogError> {
    if measurement.overhead.minimum_interval_ns == 0 {
        return Err(MeasurementCatalogError::InvalidOverhead);
    }
    if measurement.platforms.is_empty() || measurement.platforms.len() > 8 {
        return Err(MeasurementCatalogError::InvalidPlatform);
    }
    for platform in &measurement.platforms {
        if platform.architectures.is_empty()
            || platform.architectures.len() > 8
            || !strictly_sorted_unique(&platform.architectures)
            || platform.required_features.is_empty()
            || platform.required_features.len() > 8
            || !strictly_sorted_unique(&platform.required_features)
        {
            return Err(MeasurementCatalogError::InvalidPlatform);
        }
    }
    if !strictly_sorted_unique(&measurement.platforms) {
        return Err(MeasurementCatalogError::InvalidPlatform);
    }
    if measurement.evidence_limits.is_empty()
        || measurement.evidence_limits.len() > 8
        || !strictly_sorted_unique(&measurement.evidence_limits)
        || !measurement
            .evidence_limits
            .contains(&MeasurementEvidenceLimit::MissingIsUnavailable)
    {
        return Err(MeasurementCatalogError::InvalidEvidenceLimits);
    }
    let source_is_implemented = matches!(
        measurement.provenance.source,
        MeasurementSource::AsbMetricsProcfs
            | MeasurementSource::AsbMetricsCgroupV2
            | MeasurementSource::AsbRunnerJournal
            | MeasurementSource::IndependentGrader
            | MeasurementSource::ProviderUsage
    );
    if source_is_implemented
        != matches!(
            measurement.provenance.qualification,
            MeasurementQualification::Implemented
        )
    {
        return Err(MeasurementCatalogError::InvalidProvenance);
    }
    if measurement.provenance.source == MeasurementSource::OptionalCsb
        && (measurement.provenance.qualification != MeasurementQualification::Unqualified
            || measurement.live
                != MeasurementModeSupport::Unsupported {
                    reason: MeasurementUnavailableReason::NotQualified,
                }
            || measurement.replay
                != MeasurementModeSupport::Unsupported {
                    reason: MeasurementUnavailableReason::NotQualified,
                })
    {
        return Err(MeasurementCatalogError::UnqualifiedExternalSupported);
    }
    let (expected_source, expected_scope, expected_feature) = match measurement.source_identity {
        MeasurementSourceIdentity::ProcfsProcessStat
        | MeasurementSourceIdentity::ProcfsProcessIo => (
            MeasurementSource::AsbMetricsProcfs,
            MeasurementScope::Process,
            MeasurementPlatformFeature::Procfs,
        ),
        MeasurementSourceIdentity::CgroupV2CpuStat
        | MeasurementSourceIdentity::CgroupV2MemoryCurrent
        | MeasurementSourceIdentity::CgroupV2MemoryPeak
        | MeasurementSourceIdentity::CgroupV2MemoryStat
        | MeasurementSourceIdentity::CgroupV2IoStat
        | MeasurementSourceIdentity::CgroupV2CpuPressure
        | MeasurementSourceIdentity::CgroupV2MemoryPressure
        | MeasurementSourceIdentity::CgroupV2IoPressure => (
            MeasurementSource::AsbMetricsCgroupV2,
            MeasurementScope::Cgroup,
            MeasurementPlatformFeature::CgroupV2,
        ),
    };
    if measurement.provenance.source != expected_source
        || measurement.scope != expected_scope
        || measurement
            .platforms
            .iter()
            .any(|platform| platform.required_features.as_slice() != [expected_feature])
    {
        return Err(MeasurementCatalogError::InvalidSourceIdentity);
    }
    Ok(())
}

fn strictly_sorted_unique<T: Ord>(values: &[T]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}

fn validate_identifier(value: &str) -> Result<(), MeasurementCatalogError> {
    let bytes = value.as_bytes();
    let valid = (3..=128).contains(&bytes.len())
        && bytes[0].is_ascii_lowercase()
        && value.contains('.')
        && bytes.iter().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'.' || *byte == b'_'
        })
        && bytes
            .last()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && bytes
            .windows(2)
            .all(|pair| !(matches!(pair[0], b'.' | b'_') && matches!(pair[1], b'.' | b'_')));
    valid
        .then_some(())
        .ok_or(MeasurementCatalogError::InvalidIdentifier)
}

fn validate_public_text(value: &str) -> Result<(), MeasurementCatalogError> {
    let lower = value.to_ascii_lowercase();
    let sensitive = [
        "/home/",
        "/srv/",
        "/tmp/",
        "file://",
        "http://",
        "https://",
        "authorization",
        "bearer ",
        "password",
        "credential",
        "api_key",
        "api-key",
        "token=",
    ];
    if value.is_empty()
        || value.len() > MAX_MEASUREMENT_TEXT_BYTES
        || !value.is_ascii()
        || value.trim() != value
        || value.chars().any(char::is_control)
        || value.contains('@')
        || sensitive.iter().any(|marker| lower.contains(marker))
    {
        return Err(MeasurementCatalogError::UnsafePublicText);
    }
    Ok(())
}

fn validate_unit(quantity: MeasurementQuantity, unit: &str) -> Result<(), MeasurementCatalogError> {
    let compatible = match quantity {
        MeasurementQuantity::Time => matches!(unit, "ns" | "s"),
        MeasurementQuantity::Bytes => unit == "By",
        MeasurementQuantity::Count => {
            matches!(
                unit,
                "{event}" | "{fault}" | "{request}" | "{token}" | "{task}"
            )
        }
        MeasurementQuantity::Ratio | MeasurementQuantity::Score => matches!(unit, "1" | "%"),
        MeasurementQuantity::Currency => unit == "{currency}",
    };
    compatible
        .then_some(())
        .ok_or(MeasurementCatalogError::IncompatibleUnit)
}

/// Measurement catalog validation failure.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum MeasurementCatalogError {
    /// Untrusted encoded input exceeded the fixed transport ceiling.
    #[error("measurement catalog wire input exceeds its byte limit")]
    WireTooLarge,
    /// Untrusted encoded input is not a valid closed catalog shape.
    #[error("measurement catalog wire input is invalid")]
    InvalidWire,
    /// Unknown breaking schema generation.
    #[error("unsupported measurement catalog schema version {0}")]
    UnsupportedSchemaVersion(u16),
    /// Group bound exceeded.
    #[error("too many measurement groups")]
    TooManyGroups,
    /// Definition bound exceeded.
    #[error("too many measurements")]
    TooManyMeasurements,
    /// Groups or definitions are not in canonical order.
    #[error("measurement catalog order is not canonical")]
    NonCanonicalOrder,
    /// Stable group identity was repeated.
    #[error("duplicate measurement group")]
    DuplicateGroup,
    /// Stable definition identity was repeated.
    #[error("duplicate measurement definition")]
    DuplicateMeasurement,
    /// A public name is ambiguous under ASCII case folding.
    #[error("ambiguous measurement name")]
    AmbiguousName,
    /// Definition refers to a group absent from this catalog.
    #[error("unknown measurement group")]
    UnknownGroup,
    /// A published group has no selectable definitions.
    #[error("empty measurement group")]
    EmptyGroup,
    /// Stable definition identity is malformed.
    #[error("invalid measurement identifier")]
    InvalidIdentifier,
    /// Public text is unbounded, malformed, or privacy-sensitive.
    #[error("unsafe measurement catalog text")]
    UnsafePublicText,
    /// Quantity and unit do not agree.
    #[error("incompatible measurement unit")]
    IncompatibleUnit,
    /// Collection overhead contract is invalid.
    #[error("invalid measurement overhead")]
    InvalidOverhead,
    /// Platform constraints are absent, duplicated, or unordered.
    #[error("invalid measurement platform constraints")]
    InvalidPlatform,
    /// Evidence limits are absent, duplicated, or omit missing-value semantics.
    #[error("invalid measurement evidence limits")]
    InvalidEvidenceLimits,
    /// Source and qualification disagree.
    #[error("invalid measurement provenance")]
    InvalidProvenance,
    /// Exact source identity, scope, platform feature, and source class disagree.
    #[error("invalid measurement source identity")]
    InvalidSourceIdentity,
    /// Optional external evidence was advertised before qualification.
    #[error("unqualified external measurement advertised as supported")]
    UnqualifiedExternalSupported,
    /// Declared digest is malformed or stale.
    #[error("measurement catalog digest mismatch")]
    DigestMismatch,
    /// Canonical serialization unexpectedly failed.
    #[error("measurement catalog serialization failed")]
    Serialization,
}

/// Build the authoritative baseline catalog for currently implemented process and cgroup collectors.
#[must_use]
pub fn baseline_measurement_catalog() -> MeasurementCatalogV1 {
    MeasurementCatalogV1::new(baseline_groups(), baseline_definitions())
        .expect("static baseline measurement catalog must remain valid")
}

fn baseline_groups() -> Vec<MeasurementGroup> {
    [
        (
            MeasurementGroupId::SystemResources,
            "System resources",
            "CPU, memory, faults, and physical IO attributed to a process or cgroup.",
        ),
        (
            MeasurementGroupId::SchedulingContention,
            "Scheduling and contention",
            "Pressure and throttling signals that do not by themselves establish causation.",
        ),
    ]
    .into_iter()
    .map(|(id, label, description)| MeasurementGroup {
        id,
        label: label.into(),
        description: description.into(),
    })
    .collect()
}

fn baseline_definitions() -> Vec<MeasurementDefinition> {
    let mut definitions = Vec::new();
    for (id, name, quantity, unit, aggregation) in [
        (
            "process.cpu.user_time",
            "Process user CPU time",
            MeasurementQuantity::Time,
            "ns",
            Aggregation::Counter,
        ),
        (
            "process.cpu.system_time",
            "Process system CPU time",
            MeasurementQuantity::Time,
            "ns",
            Aggregation::Counter,
        ),
        (
            "process.memory.resident",
            "Process resident memory",
            MeasurementQuantity::Bytes,
            "By",
            Aggregation::Gauge,
        ),
        (
            "process.faults.minor",
            "Process minor faults",
            MeasurementQuantity::Count,
            "{fault}",
            Aggregation::Counter,
        ),
        (
            "process.faults.major",
            "Process major faults",
            MeasurementQuantity::Count,
            "{fault}",
            Aggregation::Counter,
        ),
        (
            "process.io.read",
            "Process physical read bytes",
            MeasurementQuantity::Bytes,
            "By",
            Aggregation::Counter,
        ),
        (
            "process.io.write",
            "Process physical write bytes",
            MeasurementQuantity::Bytes,
            "By",
            Aggregation::Counter,
        ),
    ] {
        definitions.push(kernel_definition(
            id,
            name,
            MeasurementGroupId::SystemResources,
            quantity,
            unit,
            aggregation,
            MeasurementScope::Process,
            MeasurementSource::AsbMetricsProcfs,
            MeasurementPlatformFeature::Procfs,
            false,
        ));
    }
    for (id, name, quantity, unit, aggregation, contention) in [
        (
            "cgroup.cpu.usage",
            "Cgroup CPU time",
            MeasurementQuantity::Time,
            "ns",
            Aggregation::Counter,
            false,
        ),
        (
            "cgroup.cpu.user",
            "Cgroup user CPU time",
            MeasurementQuantity::Time,
            "ns",
            Aggregation::Counter,
            false,
        ),
        (
            "cgroup.cpu.system",
            "Cgroup system CPU time",
            MeasurementQuantity::Time,
            "ns",
            Aggregation::Counter,
            false,
        ),
        (
            "cgroup.cpu.throttled_time",
            "Cgroup throttled CPU time",
            MeasurementQuantity::Time,
            "ns",
            Aggregation::Counter,
            true,
        ),
        (
            "cgroup.cpu.periods",
            "Cgroup CPU periods",
            MeasurementQuantity::Count,
            "{event}",
            Aggregation::Counter,
            false,
        ),
        (
            "cgroup.cpu.throttled_periods",
            "Cgroup throttled CPU periods",
            MeasurementQuantity::Count,
            "{event}",
            Aggregation::Counter,
            true,
        ),
        (
            "cgroup.memory.current",
            "Cgroup current memory",
            MeasurementQuantity::Bytes,
            "By",
            Aggregation::Gauge,
            false,
        ),
        (
            "cgroup.memory.peak",
            "Cgroup peak memory",
            MeasurementQuantity::Bytes,
            "By",
            Aggregation::Gauge,
            false,
        ),
        (
            "cgroup.faults.minor",
            "Cgroup minor faults",
            MeasurementQuantity::Count,
            "{fault}",
            Aggregation::Counter,
            false,
        ),
        (
            "cgroup.faults.major",
            "Cgroup major faults",
            MeasurementQuantity::Count,
            "{fault}",
            Aggregation::Counter,
            false,
        ),
        (
            "cgroup.io.read",
            "Cgroup physical read bytes",
            MeasurementQuantity::Bytes,
            "By",
            Aggregation::Counter,
            false,
        ),
        (
            "cgroup.io.write",
            "Cgroup physical write bytes",
            MeasurementQuantity::Bytes,
            "By",
            Aggregation::Counter,
            false,
        ),
        (
            "cgroup.pressure.cpu.some",
            "Cgroup CPU some pressure time",
            MeasurementQuantity::Time,
            "ns",
            Aggregation::Counter,
            true,
        ),
        (
            "cgroup.pressure.cpu.full",
            "Cgroup CPU full pressure time",
            MeasurementQuantity::Time,
            "ns",
            Aggregation::Counter,
            true,
        ),
        (
            "cgroup.pressure.memory.some",
            "Cgroup memory some pressure time",
            MeasurementQuantity::Time,
            "ns",
            Aggregation::Counter,
            true,
        ),
        (
            "cgroup.pressure.memory.full",
            "Cgroup memory full pressure time",
            MeasurementQuantity::Time,
            "ns",
            Aggregation::Counter,
            true,
        ),
        (
            "cgroup.pressure.io.some",
            "Cgroup IO some pressure time",
            MeasurementQuantity::Time,
            "ns",
            Aggregation::Counter,
            true,
        ),
        (
            "cgroup.pressure.io.full",
            "Cgroup IO full pressure time",
            MeasurementQuantity::Time,
            "ns",
            Aggregation::Counter,
            true,
        ),
    ] {
        definitions.push(kernel_definition(
            id,
            name,
            if contention {
                MeasurementGroupId::SchedulingContention
            } else {
                MeasurementGroupId::SystemResources
            },
            quantity,
            unit,
            aggregation,
            MeasurementScope::Cgroup,
            MeasurementSource::AsbMetricsCgroupV2,
            MeasurementPlatformFeature::CgroupV2,
            contention,
        ));
    }
    definitions
}

#[allow(clippy::too_many_arguments)]
fn kernel_definition(
    id: &str,
    name: &str,
    group: MeasurementGroupId,
    quantity: MeasurementQuantity,
    unit: &str,
    aggregation: Aggregation,
    scope: MeasurementScope,
    source: MeasurementSource,
    feature: MeasurementPlatformFeature,
    contention: bool,
) -> MeasurementDefinition {
    let (source_identity, resolution_ns) = runtime_descriptor_semantics(id);
    let mut limits = vec![
        MeasurementEvidenceLimit::MissingIsUnavailable,
        MeasurementEvidenceLimit::RequiresComparableExperiment,
        MeasurementEvidenceLimit::CollectorOverheadRecorded,
    ];
    if contention {
        limits.push(MeasurementEvidenceLimit::NotCausal);
    }
    limits.sort();
    MeasurementDefinition {
        id: id.into(),
        name: name.into(),
        description: if contention {
            "A cumulative kernel signal that must not be interpreted as causal contention alone."
        } else {
            "A direct bounded Linux kernel counter collected by ASB."
        }
        .into(),
        group,
        quantity,
        unit: unit.into(),
        aggregation,
        scope,
        provenance: MeasurementProvenance {
            source,
            qualification: MeasurementQualification::Implemented,
        },
        source_identity,
        resolution_ns,
        overhead: MeasurementOverhead {
            class: MeasurementOverheadClass::Low,
            minimum_interval_ns: 1_000_000,
            requires_privilege: false,
        },
        live: MeasurementModeSupport::Supported,
        replay: MeasurementModeSupport::Supported,
        platforms: vec![MeasurementPlatform {
            operating_system: MeasurementOperatingSystem::Linux,
            architectures: vec![
                MeasurementArchitecture::X86_64,
                MeasurementArchitecture::Aarch64,
            ],
            required_features: vec![feature],
        }],
        evidence_limits: limits,
    }
}

fn runtime_descriptor_semantics(id: &str) -> (MeasurementSourceIdentity, u64) {
    match id {
        "process.cpu.user_time" | "process.cpu.system_time" => {
            (MeasurementSourceIdentity::ProcfsProcessStat, 10_000_000)
        }
        "process.memory.resident" | "process.faults.minor" | "process.faults.major" => {
            (MeasurementSourceIdentity::ProcfsProcessStat, 0)
        }
        "process.io.read" | "process.io.write" => (MeasurementSourceIdentity::ProcfsProcessIo, 0),
        "cgroup.cpu.usage"
        | "cgroup.cpu.user"
        | "cgroup.cpu.system"
        | "cgroup.cpu.throttled_time" => (MeasurementSourceIdentity::CgroupV2CpuStat, 1_000),
        "cgroup.cpu.periods" | "cgroup.cpu.throttled_periods" => {
            (MeasurementSourceIdentity::CgroupV2CpuStat, 0)
        }
        "cgroup.memory.current" => (MeasurementSourceIdentity::CgroupV2MemoryCurrent, 0),
        "cgroup.memory.peak" => (MeasurementSourceIdentity::CgroupV2MemoryPeak, 0),
        "cgroup.faults.minor" | "cgroup.faults.major" => {
            (MeasurementSourceIdentity::CgroupV2MemoryStat, 0)
        }
        "cgroup.io.read" | "cgroup.io.write" => (MeasurementSourceIdentity::CgroupV2IoStat, 0),
        "cgroup.pressure.cpu.some" | "cgroup.pressure.cpu.full" => {
            (MeasurementSourceIdentity::CgroupV2CpuPressure, 1_000)
        }
        "cgroup.pressure.memory.some" | "cgroup.pressure.memory.full" => {
            (MeasurementSourceIdentity::CgroupV2MemoryPressure, 1_000)
        }
        "cgroup.pressure.io.some" | "cgroup.pressure.io.full" => {
            (MeasurementSourceIdentity::CgroupV2IoPressure, 1_000)
        }
        _ => unreachable!("baseline metric identity is statically closed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    #[test]
    fn baseline_is_complete_stable_and_excludes_optional_csb() {
        let catalog = baseline_measurement_catalog();
        catalog.validate().unwrap();
        assert_eq!(catalog.groups.len(), 2);
        assert_eq!(catalog.measurements.len(), 25);
        assert!(catalog.groups.iter().all(|group| {
            catalog
                .measurements
                .iter()
                .any(|measurement| measurement.group == group.id)
        }));
        assert!(catalog.measurements.iter().all(|measurement| {
            measurement.provenance.source != MeasurementSource::OptionalCsb
        }));
        assert!(
            catalog
                .measurements
                .windows(2)
                .all(|pair| pair[0].id < pair[1].id)
        );
        assert_eq!(catalog.catalog_sha256, catalog.computed_digest().unwrap());
    }

    #[test]
    fn constructor_canonicalizes_input() {
        let baseline = baseline_measurement_catalog();
        let mut groups = baseline.groups.clone();
        let mut measurements = baseline.measurements.clone();
        groups.reverse();
        measurements.reverse();
        assert_eq!(
            MeasurementCatalogV1::new(groups, measurements).unwrap(),
            baseline
        );
    }

    #[test]
    fn bounds_are_enforced() {
        let group = baseline_groups()[0].clone();
        let definition = baseline_definitions()[0].clone();
        assert_eq!(
            MeasurementCatalogV1::new(vec![group.clone(); MAX_MEASUREMENT_GROUPS + 1], Vec::new()),
            Err(MeasurementCatalogError::TooManyGroups)
        );
        let mut definitions = Vec::new();
        for index in 0..=MAX_MEASUREMENTS {
            let mut item = definition.clone();
            item.id = format!("synthetic.metric_{index}");
            item.name = format!("Synthetic metric {index}");
            definitions.push(item);
        }
        assert_eq!(
            MeasurementCatalogV1::new(vec![baseline_groups()[0].clone()], definitions),
            Err(MeasurementCatalogError::TooManyMeasurements)
        );

        let mut definitions = Vec::new();
        for index in 0..MAX_MEASUREMENTS {
            let mut item = definition.clone();
            item.id = format!("synthetic.metric_{index}");
            item.name = format!("Synthetic metric {index}");
            definitions.push(item);
        }
        let maximal = MeasurementCatalogV1::new(vec![group], definitions).unwrap();
        assert_eq!(maximal.measurements.len(), MAX_MEASUREMENTS);
    }

    #[test]
    fn preflight_rejects_attacker_work_before_canonicalization() {
        let baseline = baseline_measurement_catalog();
        let mut oversized = vec![baseline.measurements[0].clone(); MAX_MEASUREMENTS + 1];
        oversized[0].id = "Bad ID".into();
        assert_eq!(
            MeasurementCatalogV1::new(baseline.groups.clone(), oversized),
            Err(MeasurementCatalogError::TooManyMeasurements)
        );

        let mut nested = baseline.measurements[0].clone();
        nested.name = "z".repeat(MAX_MEASUREMENT_TEXT_BYTES + 1);
        nested.platforms.reverse();
        assert_eq!(
            MeasurementCatalogV1::new(baseline.groups.clone(), vec![nested]),
            Err(MeasurementCatalogError::UnsafePublicText)
        );

        let mut nested = baseline.measurements[0].clone();
        nested.platforms = vec![nested.platforms[0].clone(); 9];
        assert_eq!(
            MeasurementCatalogV1::new(baseline.groups, vec![nested]),
            Err(MeasurementCatalogError::InvalidPlatform)
        );
    }

    #[test]
    fn untrusted_wire_reads_and_allocations_are_bounded() {
        let fixture = include_bytes!("../fixtures/v1/measurement-catalog.json");
        assert_eq!(
            MeasurementCatalogV1::from_slice_bounded(fixture).unwrap(),
            baseline_measurement_catalog()
        );
        assert_eq!(
            MeasurementCatalogV1::from_slice_bounded(&vec![
                b' ';
                MAX_MEASUREMENT_CATALOG_WIRE_BYTES + 1
            ]),
            Err(MeasurementCatalogError::WireTooLarge)
        );
        assert_eq!(
            MeasurementCatalogV1::from_slice_bounded(b"{"),
            Err(MeasurementCatalogError::InvalidWire)
        );

        struct Endless {
            bytes_read: usize,
        }
        impl io::Read for &mut Endless {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                buffer.fill(b' ');
                self.bytes_read += buffer.len();
                Ok(buffer.len())
            }
        }
        let mut endless = Endless { bytes_read: 0 };
        assert_eq!(
            MeasurementCatalogV1::from_reader_bounded(&mut endless),
            Err(MeasurementCatalogError::WireTooLarge)
        );
        assert_eq!(endless.bytes_read, MAX_MEASUREMENT_CATALOG_WIRE_BYTES + 1);

        struct Failing;
        impl io::Read for Failing {
            fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::other("synthetic read failure"))
            }
        }
        assert_eq!(
            MeasurementCatalogV1::from_reader_bounded(Failing),
            Err(MeasurementCatalogError::InvalidWire)
        );
    }

    #[test]
    fn advertised_groups_must_have_selectable_definitions() {
        let baseline = baseline_measurement_catalog();
        let empty = MeasurementGroup {
            id: MeasurementGroupId::Latency,
            label: "Latency".into(),
            description: "Completion timing evidence.".into(),
        };
        assert_eq!(
            MeasurementCatalogV1::new(
                [baseline.groups.clone(), vec![empty]].concat(),
                baseline.measurements
            ),
            Err(MeasurementCatalogError::EmptyGroup)
        );
        assert!(MeasurementCatalogV1::new(Vec::new(), Vec::new()).is_ok());
    }

    #[test]
    fn duplicate_ambiguous_unit_group_and_order_fail_closed() {
        let baseline = baseline_measurement_catalog();
        let first = baseline.measurements[0].clone();
        let mut duplicate = first.clone();
        duplicate.name = "Another name".into();
        assert_eq!(
            MeasurementCatalogV1::new(baseline.groups.clone(), vec![first.clone(), duplicate]),
            Err(MeasurementCatalogError::DuplicateMeasurement)
        );

        let mut incompatible = first.clone();
        incompatible.unit = "By".into();
        incompatible.quantity = MeasurementQuantity::Bytes;
        assert_eq!(
            MeasurementCatalogV1::new(baseline.groups.clone(), vec![first.clone(), incompatible]),
            Err(MeasurementCatalogError::IncompatibleUnit)
        );

        let mut ambiguous = baseline.measurements[1].clone();
        ambiguous.name = first.name.to_ascii_uppercase();
        assert_eq!(
            MeasurementCatalogV1::new(baseline.groups.clone(), vec![first.clone(), ambiguous]),
            Err(MeasurementCatalogError::AmbiguousName)
        );

        let mut unknown = first;
        unknown.group = MeasurementGroupId::Cost;
        let groups = vec![baseline_groups()[0].clone()];
        assert_eq!(
            MeasurementCatalogV1::new(groups, vec![unknown]),
            Err(MeasurementCatalogError::UnknownGroup)
        );

        let mut stale = baseline;
        stale.measurements.swap(0, 1);
        assert_eq!(
            stale.validate(),
            Err(MeasurementCatalogError::NonCanonicalOrder)
        );
    }

    #[test]
    fn privacy_provenance_support_and_digest_fail_closed() {
        let baseline = baseline_measurement_catalog();
        let mut private = baseline.measurements[0].clone();
        private.description = "/home/operator/token=secret".into();
        assert_eq!(
            MeasurementCatalogV1::new(baseline.groups.clone(), vec![private]),
            Err(MeasurementCatalogError::UnsafePublicText)
        );

        let mut csb = baseline.measurements[0].clone();
        csb.provenance = MeasurementProvenance {
            source: MeasurementSource::OptionalCsb,
            qualification: MeasurementQualification::Unqualified,
        };
        assert_eq!(
            MeasurementCatalogV1::new(baseline.groups.clone(), vec![csb]),
            Err(MeasurementCatalogError::UnqualifiedExternalSupported)
        );

        let mut unsupported = baseline.measurements[0].clone();
        unsupported.provenance.qualification = MeasurementQualification::Unqualified;
        assert_eq!(
            MeasurementCatalogV1::new(baseline.groups.clone(), vec![unsupported]),
            Err(MeasurementCatalogError::InvalidProvenance)
        );

        let mut mismatched = baseline.measurements[0].clone();
        mismatched.source_identity = MeasurementSourceIdentity::ProcfsProcessIo;
        assert_eq!(
            MeasurementCatalogV1::new(baseline.groups.clone(), vec![mismatched]),
            Err(MeasurementCatalogError::InvalidSourceIdentity)
        );

        let mut stale = baseline;
        stale.catalog_sha256 = "0".repeat(64);
        assert_eq!(
            stale.validate(),
            Err(MeasurementCatalogError::DigestMismatch)
        );
    }

    #[test]
    fn unit_platform_evidence_overhead_and_identifier_fail_closed() {
        let baseline = baseline_measurement_catalog();
        let check = |item: MeasurementDefinition, expected| {
            let result = MeasurementCatalogV1::new(baseline.groups.clone(), vec![item.clone()]);
            assert_eq!(result, Err(expected));
        };
        let original = baseline.measurements[0].clone();
        let mut item = original.clone();
        item.unit = "By".into();
        check(item, MeasurementCatalogError::IncompatibleUnit);
        let mut item = original.clone();
        item.id = "Bad ID".into();
        check(item, MeasurementCatalogError::InvalidIdentifier);
        let mut item = original.clone();
        item.id = "bad.__id".into();
        check(item, MeasurementCatalogError::InvalidIdentifier);
        let mut item = original.clone();
        item.id = "valid.metric".into();
        item.platforms.clear();
        check(item, MeasurementCatalogError::InvalidPlatform);
        let mut noncanonical = baseline.clone();
        noncanonical.measurements[0].platforms[0]
            .architectures
            .reverse();
        assert_eq!(
            noncanonical.validate(),
            Err(MeasurementCatalogError::InvalidPlatform)
        );
        let mut item = original.clone();
        item.platforms.push(item.platforms[0].clone());
        check(item, MeasurementCatalogError::InvalidPlatform);
        let mut item = original.clone();
        item.evidence_limits.clear();
        check(item, MeasurementCatalogError::InvalidEvidenceLimits);
        let mut item = original;
        item.overhead.minimum_interval_ns = 0;
        check(item, MeasurementCatalogError::InvalidOverhead);
    }

    #[test]
    fn public_text_is_ascii_and_duplicate_errors_are_specific() {
        let baseline = baseline_measurement_catalog();
        let first = baseline.measurements[0].clone();
        let mut unicode = first.clone();
        unicode.name = "Synthetic metric alpha".replace("alpha", "\u{03b1}");
        assert_eq!(
            MeasurementCatalogV1::new(baseline.groups.clone(), vec![unicode]),
            Err(MeasurementCatalogError::UnsafePublicText)
        );
        assert_eq!(
            MeasurementCatalogV1::new(
                vec![baseline.groups[0].clone(), baseline.groups[0].clone()],
                vec![first.clone()]
            ),
            Err(MeasurementCatalogError::DuplicateGroup)
        );
        assert_eq!(
            MeasurementCatalogV1::new(baseline.groups, vec![first.clone(), first]),
            Err(MeasurementCatalogError::DuplicateMeasurement)
        );
    }
}
