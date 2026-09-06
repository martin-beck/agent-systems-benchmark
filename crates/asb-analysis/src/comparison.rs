// SPDX-License-Identifier: MIT
//! Fail-closed comparison of validated experiment provenance.

use asb_protocol::{ExperimentManifestError, ExperimentManifestV1};
use std::error::Error;
use std::fmt;

/// Required experiment dimension that differs between two manifests.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ComparisonField {
    /// Agent implementation, revision, or executable.
    Agent,
    /// Model provider or identifier.
    Model,
    /// Inference settings.
    ModelSettings,
    /// Tool policy name, revision, or content.
    ToolPolicy,
    /// Workload identity, revision, or content.
    Workload,
    /// Scorer revision or content.
    Scorer,
    /// Execution image.
    Image,
    /// Resolved dependencies.
    Dependencies,
    /// Kernel release.
    Kernel,
    /// Distribution identity and exact version.
    Distribution,
    /// Native architecture.
    Architecture,
    /// CPU model.
    CpuModel,
    /// Available logical CPU count.
    LogicalCpuCount,
    /// Available NUMA node count.
    NumaNodeCount,
    /// CPU scaling governor.
    ScalingGovernor,
    /// Cache preparation.
    CacheState,
    /// Controlled background-load policy.
    LoadPolicy,
    /// Retry limit, strategy, or backoff.
    RetrySettings,
    /// Live/replay mode.
    ReplayMode,
    /// Replay cassette content.
    ReplayCassette,
    /// Replay pacing.
    ReplayPacing,
    /// Cancellation deadline.
    CancellationTimeout,
}

/// Whether an unqualified cross-experiment claim is permitted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComparisonQualification {
    /// All required provenance dimensions match.
    Comparable,
    /// At least one required dimension differs.
    Confounded,
}

/// Constructor-controlled comparison result.
///
/// Callers cannot forge a comparable report without validation:
///
/// ```compile_fail
/// use asb_analysis::{ComparisonQualification, ComparisonReport};
///
/// let forged = ComparisonReport {
///     qualification: ComparisonQualification::Comparable,
///     differences: Vec::new(),
/// };
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComparisonReport {
    qualification: ComparisonQualification,
    differences: Vec<ComparisonField>,
}

impl ComparisonReport {
    /// Return whether an unqualified claim is permitted.
    pub fn qualification(&self) -> ComparisonQualification {
        self.qualification
    }

    /// Return the deterministic list of required dimensions that differ.
    pub fn differences(&self) -> &[ComparisonField] {
        &self.differences
    }

    /// True only when every required dimension matches.
    pub fn permits_unqualified_claim(&self) -> bool {
        self.qualification == ComparisonQualification::Comparable
    }
}

/// Failure to validate either side before comparison.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ComparisonError {
    /// The left experiment manifest is invalid.
    InvalidLeft(ExperimentManifestError),
    /// The right experiment manifest is invalid.
    InvalidRight(ExperimentManifestError),
}

impl fmt::Display for ComparisonError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLeft(error) => write!(formatter, "invalid left experiment: {error}"),
            Self::InvalidRight(error) => write!(formatter, "invalid right experiment: {error}"),
        }
    }
}

impl Error for ComparisonError {}

/// Compare every required provenance dimension after validating both manifests.
///
/// Differences contain field identities, never raw setting values, host paths,
/// or prompts. Any required mismatch makes the report confounded.
pub fn compare_experiments(
    left: &ExperimentManifestV1,
    right: &ExperimentManifestV1,
) -> Result<ComparisonReport, ComparisonError> {
    left.validate().map_err(ComparisonError::InvalidLeft)?;
    right.validate().map_err(ComparisonError::InvalidRight)?;

    let mut differences = Vec::new();
    differing(
        &mut differences,
        ComparisonField::Agent,
        &(
            &left.agent.implementation,
            &left.agent.revision,
            &left.agent.binary_sha256,
        ),
        &(
            &right.agent.implementation,
            &right.agent.revision,
            &right.agent.binary_sha256,
        ),
    );
    differing(
        &mut differences,
        ComparisonField::Model,
        &(&left.model.provider, &left.model.model),
        &(&right.model.provider, &right.model.model),
    );
    differing(
        &mut differences,
        ComparisonField::ModelSettings,
        &left.model.settings,
        &right.model.settings,
    );
    differing(
        &mut differences,
        ComparisonField::ToolPolicy,
        &left.tool_policy,
        &right.tool_policy,
    );
    differing(
        &mut differences,
        ComparisonField::Workload,
        &(
            &left.workload.workload,
            &left.workload.workload_revision,
            &left.workload.workload_sha256,
        ),
        &(
            &right.workload.workload,
            &right.workload.workload_revision,
            &right.workload.workload_sha256,
        ),
    );
    differing(
        &mut differences,
        ComparisonField::Scorer,
        &(&left.workload.scorer_revision, &left.workload.scorer_sha256),
        &(
            &right.workload.scorer_revision,
            &right.workload.scorer_sha256,
        ),
    );
    differing(
        &mut differences,
        ComparisonField::Image,
        &left.execution.image_sha256,
        &right.execution.image_sha256,
    );
    differing(
        &mut differences,
        ComparisonField::Dependencies,
        &left.execution.dependencies_sha256,
        &right.execution.dependencies_sha256,
    );
    differing(
        &mut differences,
        ComparisonField::Kernel,
        &left.platform.kernel_release,
        &right.platform.kernel_release,
    );
    differing(
        &mut differences,
        ComparisonField::Distribution,
        &(
            &left.platform.distribution,
            &left.platform.distribution_version,
        ),
        &(
            &right.platform.distribution,
            &right.platform.distribution_version,
        ),
    );
    differing(
        &mut differences,
        ComparisonField::Architecture,
        &left.platform.architecture,
        &right.platform.architecture,
    );
    differing(
        &mut differences,
        ComparisonField::CpuModel,
        &left.platform.cpu_model,
        &right.platform.cpu_model,
    );
    differing(
        &mut differences,
        ComparisonField::LogicalCpuCount,
        &left.platform.logical_cpu_count,
        &right.platform.logical_cpu_count,
    );
    differing(
        &mut differences,
        ComparisonField::NumaNodeCount,
        &left.platform.numa_node_count,
        &right.platform.numa_node_count,
    );
    differing(
        &mut differences,
        ComparisonField::ScalingGovernor,
        &left.platform.scaling_governor,
        &right.platform.scaling_governor,
    );
    differing(
        &mut differences,
        ComparisonField::CacheState,
        &left.controls.cache_state,
        &right.controls.cache_state,
    );
    differing(
        &mut differences,
        ComparisonField::LoadPolicy,
        &left.controls.load_policy,
        &right.controls.load_policy,
    );
    differing(
        &mut differences,
        ComparisonField::RetrySettings,
        &left.controls.retry,
        &right.controls.retry,
    );
    differing(
        &mut differences,
        ComparisonField::ReplayMode,
        &left.controls.replay.mode,
        &right.controls.replay.mode,
    );
    differing(
        &mut differences,
        ComparisonField::ReplayCassette,
        &left.controls.replay.cassette_sha256,
        &right.controls.replay.cassette_sha256,
    );
    differing(
        &mut differences,
        ComparisonField::ReplayPacing,
        &left.controls.replay.pacing,
        &right.controls.replay.pacing,
    );
    differing(
        &mut differences,
        ComparisonField::CancellationTimeout,
        &left.controls.replay.cancellation_timeout_ms,
        &right.controls.replay.cancellation_timeout_ms,
    );

    let qualification = if differences.is_empty() {
        ComparisonQualification::Comparable
    } else {
        ComparisonQualification::Confounded
    };
    Ok(ComparisonReport {
        qualification,
        differences,
    })
}

fn differing<T: PartialEq>(
    differences: &mut Vec<ComparisonField>,
    field: ComparisonField,
    left: &T,
    right: &T,
) {
    if left != right {
        differences.push(field);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checked_fixture(contents: &str) -> ExperimentManifestV1 {
        let value: ExperimentManifestV1 = serde_json::from_str(contents).unwrap();
        value.validate().unwrap();
        value
    }

    fn manifest() -> ExperimentManifestV1 {
        let digest = "a".repeat(64);
        let fixture = serde_json::json!({
            "version": "1",
            "experiment_sha256": digest,
            "agent": {
                "implementation": "asb-example",
                "revision": "1.0.0",
                "binary_sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            },
            "model": {
                "provider": "example",
                "model": "model-v1",
                "settings": {
                    "temperature_milli": 0,
                    "top_p_millionth": 1000000,
                    "seed": 7,
                    "max_output_tokens": 4096,
                    "reasoning_effort": "medium",
                    "additional_settings_sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                }
            },
            "tool_policy": {
                "policy": "standard",
                "revision": "1",
                "content_sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            },
            "workload": {
                "workload": "patch",
                "workload_revision": "abc123",
                "workload_sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "scorer_revision": "def456",
                "scorer_sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            },
            "execution": {
                "image_sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "dependencies_sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            },
            "platform": {
                "kernel_release": "6.12.0",
                "distribution": "debian",
                "distribution_version": "13.6",
                "architecture": "x86_64",
                "cpu_model": "example-cpu",
                "logical_cpu_count": 8,
                "numa_node_count": 1,
                "scaling_governor": "performance"
            },
            "controls": {
                "cache_state": "cold",
                "load_policy": "isolated",
                "retry": {"limit": 0, "strategy": "none", "backoff_ms": 0},
                "replay": {
                    "mode": "replay",
                    "cassette_sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "pacing": "recorded",
                    "cancellation_timeout_ms": 10000
                }
            }
        });
        let mut value: ExperimentManifestV1 = serde_json::from_value(fixture).unwrap();
        value.refresh_content_address().unwrap();
        value
    }

    #[test]
    fn exact_match_is_the_only_unqualified_result() {
        let value = checked_fixture(include_str!(
            "../../asb-protocol/fixtures/v1/experiment-manifest.json"
        ));
        let report = compare_experiments(&value, &value).unwrap();
        assert_eq!(report.qualification(), ComparisonQualification::Comparable);
        assert!(report.differences().is_empty());
        assert!(report.permits_unqualified_claim());
    }

    #[test]
    fn every_required_group_is_reported_in_stable_order() {
        let left = checked_fixture(include_str!(
            "../../asb-protocol/fixtures/v1/experiment-manifest.json"
        ));
        let right = checked_fixture(include_str!(
            "../../asb-protocol/fixtures/v1/experiment-manifest-confounded.json"
        ));

        let report = compare_experiments(&left, &right).unwrap();
        assert_eq!(
            report.differences(),
            &[
                ComparisonField::Agent,
                ComparisonField::Model,
                ComparisonField::ModelSettings,
                ComparisonField::ToolPolicy,
                ComparisonField::Workload,
                ComparisonField::Scorer,
                ComparisonField::Image,
                ComparisonField::Dependencies,
                ComparisonField::Kernel,
                ComparisonField::Distribution,
                ComparisonField::Architecture,
                ComparisonField::CpuModel,
                ComparisonField::LogicalCpuCount,
                ComparisonField::NumaNodeCount,
                ComparisonField::ScalingGovernor,
                ComparisonField::CacheState,
                ComparisonField::LoadPolicy,
                ComparisonField::RetrySettings,
                ComparisonField::ReplayMode,
                ComparisonField::ReplayCassette,
                ComparisonField::ReplayPacing,
                ComparisonField::CancellationTimeout,
            ]
        );
        assert_eq!(report.qualification(), ComparisonQualification::Confounded);
        assert!(!report.permits_unqualified_claim());
        assert_eq!(
            compare_experiments(&right, &left).unwrap().differences(),
            report.differences()
        );
    }

    #[test]
    fn stale_addresses_fail_closed_on_either_side() {
        let valid = manifest();
        let mut invalid = valid.clone();
        invalid.platform.kernel_release = "other".into();
        assert_eq!(
            compare_experiments(&invalid, &valid),
            Err(ComparisonError::InvalidLeft(
                ExperimentManifestError::ContentAddressMismatch
            ))
        );
        assert_eq!(
            compare_experiments(&valid, &invalid),
            Err(ComparisonError::InvalidRight(
                ExperimentManifestError::ContentAddressMismatch
            ))
        );
        assert_eq!(
            ComparisonError::InvalidLeft(ExperimentManifestError::ContentAddressMismatch)
                .to_string(),
            "invalid left experiment: experiment content address mismatch"
        );
        assert_eq!(
            ComparisonError::InvalidRight(ExperimentManifestError::ContentAddressMismatch)
                .to_string(),
            "invalid right experiment: experiment content address mismatch"
        );
    }
}
