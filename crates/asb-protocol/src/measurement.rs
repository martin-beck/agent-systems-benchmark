// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Versioned, content-addressed measurement catalog semantics.

use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::Aggregation;

/// Current measurement catalog schema generation.
pub const MEASUREMENT_CATALOG_SCHEMA_V1: u16 = 1;
/// Maximum semantic groups accepted in one catalog.
pub const MAX_MEASUREMENT_GROUPS: usize = 16;
/// Maximum definitions accepted in one catalog.
pub const MAX_MEASUREMENTS: usize = 128;
/// Maximum UTF-8 bytes accepted in a public label or description.
pub const MAX_MEASUREMENT_TEXT_BYTES: usize = 256;

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
    #[schemars(length(min = 1, max = 256))]
    pub label: String,
    /// Public semantic boundary for the group.
    #[schemars(length(min = 1, max = 256))]
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
    #[schemars(length(min = 1, max = 256))]
    pub name: String,
    /// Public evidence meaning, containing no runtime or host material.
    #[schemars(length(min = 1, max = 256))]
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
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MeasurementCatalogV1 {
    /// Closed schema generation; exactly 1.
    #[schemars(range(min = 1, max = 1))]
    pub schema_version: u16,
    /// SHA-256 of the canonical catalog contents excluding this field.
    #[schemars(regex(pattern = r"^[0-9a-f]{64}$"), length(equal = 64))]
    pub catalog_sha256: String,
    /// Stable groups in enum order.
    #[schemars(length(max = 16))]
    pub groups: Vec<MeasurementGroup>,
    /// Stable definitions in ascending ID order.
    #[schemars(length(max = 128))]
    pub measurements: Vec<MeasurementDefinition>,
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
    Ok(())
}

fn strictly_sorted_unique<T: Ord>(values: &[T]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}

fn validate_identifier(value: &str) -> Result<(), MeasurementCatalogError> {
    let valid = (3..=128).contains(&value.len())
        && value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase())
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'.' || byte == b'_'
        })
        && value.contains('.')
        && !value.contains("..")
        && !value.ends_with('.');
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
        (
            MeasurementGroupId::Latency,
            "Latency",
            "Queue, readiness, response, and completion timing evidence.",
        ),
        (
            MeasurementGroupId::QualityReliability,
            "Quality and reliability",
            "Independent correctness, failure, timeout, retry, and cancellation evidence.",
        ),
        (
            MeasurementGroupId::Fairness,
            "Fairness",
            "Per-agent and mixed-workload service distribution evidence.",
        ),
        (
            MeasurementGroupId::Cost,
            "Cost",
            "Provider-reported monetary evidence with explicit absence semantics.",
        ),
        (
            MeasurementGroupId::Provenance,
            "Provenance",
            "Harness, collector, replay, and experiment identity evidence.",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baseline_is_complete_stable_and_excludes_optional_csb() {
        let catalog = baseline_measurement_catalog();
        catalog.validate().unwrap();
        assert_eq!(catalog.groups.len(), 7);
        assert_eq!(catalog.measurements.len(), 25);
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
            MeasurementCatalogV1::new(vec![group; MAX_MEASUREMENT_GROUPS + 1], Vec::new()),
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
        item.id = "valid.metric".into();
        item.platforms.clear();
        check(item, MeasurementCatalogError::InvalidPlatform);
        let mut item = original.clone();
        item.evidence_limits.clear();
        check(item, MeasurementCatalogError::InvalidEvidenceLimits);
        let mut item = original;
        item.overhead.minimum_interval_ns = 0;
        check(item, MeasurementCatalogError::InvalidOverhead);
    }
}
