// SPDX-License-Identifier: MIT
//! Versioned, content-addressed experiment provenance.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Current experiment-manifest contract generation.
pub const EXPERIMENT_MANIFEST_V1: ExperimentManifestVersion = ExperimentManifestVersion::V1;
const MAX_TEXT_BYTES: usize = 1_024;

/// Version discriminator for experiment manifests.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub enum ExperimentManifestVersion {
    /// First stable comparability contract.
    #[serde(rename = "1")]
    V1,
}

/// Immutable agent implementation identity.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentIdentity {
    /// Stable adapter or agent implementation name.
    pub implementation: String,
    /// Immutable implementation revision or release.
    pub revision: String,
    /// Lowercase SHA-256 of the executable or package actually run.
    #[schemars(regex(pattern = r"^[0-9a-f]{64}$"), length(equal = 64))]
    pub binary_sha256: String,
}

/// Provider/model identity and explicit inference settings.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelIdentity {
    /// Provider implementation or endpoint family, without credentials or URLs.
    pub provider: String,
    /// Provider model identifier.
    pub model: String,
    /// Settings affecting inference, represented without credentials.
    pub settings: ModelSettings,
}

/// Common inference settings plus a pin for any provider-specific remainder.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelSettings {
    /// Temperature multiplied by 1,000.
    pub temperature_milli: Option<u16>,
    /// Top-p multiplied by 1,000,000.
    pub top_p_millionth: Option<u32>,
    /// Deterministic provider seed, when supported.
    pub seed: Option<u64>,
    /// Requested maximum output tokens.
    pub max_output_tokens: Option<u32>,
    /// Named reasoning-effort level, when supported.
    pub reasoning_effort: Option<String>,
    /// SHA-256 of canonical, credential-free provider-specific settings.
    #[schemars(inner(regex(pattern = r"^[0-9a-f]{64}$"), length(equal = 64)))]
    pub additional_settings_sha256: Option<String>,
}

/// Immutable tool-use policy.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ToolPolicyIdentity {
    /// Stable policy name.
    pub policy: String,
    /// Immutable policy revision.
    pub revision: String,
    /// Lowercase SHA-256 of canonical policy content.
    #[schemars(regex(pattern = r"^[0-9a-f]{64}$"), length(equal = 64))]
    pub content_sha256: String,
}

/// Immutable workload and independent scorer identity.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkloadIdentity {
    /// Stable workload name.
    pub workload: String,
    /// Immutable workload revision.
    pub workload_revision: String,
    /// Lowercase SHA-256 of prepared workload content.
    #[schemars(regex(pattern = r"^[0-9a-f]{64}$"), length(equal = 64))]
    pub workload_sha256: String,
    /// Immutable scorer revision.
    pub scorer_revision: String,
    /// Lowercase SHA-256 of scorer code and configuration.
    #[schemars(regex(pattern = r"^[0-9a-f]{64}$"), length(equal = 64))]
    pub scorer_sha256: String,
}

/// Content pins for the execution environment.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionIdentity {
    /// Lowercase SHA-256 of the resolved root filesystem or image manifest.
    #[schemars(regex(pattern = r"^[0-9a-f]{64}$"), length(equal = 64))]
    pub image_sha256: String,
    /// Lowercase SHA-256 of the complete resolved dependency lock.
    #[schemars(regex(pattern = r"^[0-9a-f]{64}$"), length(equal = 64))]
    pub dependencies_sha256: String,
}

/// Relevant native platform identity and topology.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlatformIdentity {
    /// Exact kernel release reported during the run.
    pub kernel_release: String,
    /// Distribution identifier.
    pub distribution: String,
    /// Exact distribution version.
    pub distribution_version: String,
    /// Native execution architecture.
    pub architecture: String,
    /// CPU model visible to the workload.
    pub cpu_model: String,
    /// Logical CPUs available to the workload.
    #[schemars(range(min = 1))]
    pub logical_cpu_count: u32,
    /// NUMA node count available to the workload.
    #[schemars(range(min = 1))]
    pub numa_node_count: u32,
    /// CPU scaling governor observed for allocated CPUs.
    pub scaling_governor: String,
}

/// Cache treatment before a measured attempt.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheState {
    /// Relevant caches were deliberately cold.
    Cold,
    /// Relevant caches were deliberately warmed.
    Warm,
    /// No cache preparation was performed.
    Uncontrolled,
}

/// Live or recorded execution mode.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplayMode {
    /// Requests reached the configured live provider.
    Live,
    /// Responses came from a pinned replay cassette.
    Replay,
}

/// Replay behavior affecting timing and cancellation.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReplaySettings {
    /// Live or replay execution.
    pub mode: ReplayMode,
    /// Cassette content hash; required only in replay mode.
    #[schemars(inner(regex(pattern = r"^[0-9a-f]{64}$"), length(equal = 64)))]
    pub cassette_sha256: Option<String>,
    /// Named replay pacing strategy.
    pub pacing: String,
    /// Deadline for cancellation acknowledgement.
    #[schemars(range(min = 1))]
    pub cancellation_timeout_ms: u64,
}

/// Retry behavior applied after an initial failed attempt.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RetrySettings {
    /// Maximum number of retries after the first attempt.
    pub limit: u16,
    /// Named retry strategy.
    pub strategy: String,
    /// Base backoff before a retry, in milliseconds.
    pub backoff_ms: u64,
}

/// Controls that can alter measurements independently of product behavior.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunControls {
    /// Cache preparation state.
    pub cache_state: CacheState,
    /// Stable description or revision of controlled background load.
    pub load_policy: String,
    /// Retry limit, strategy, and backoff.
    pub retry: RetrySettings,
    /// Replay, pacing, cassette, and cancellation settings.
    pub replay: ReplaySettings,
}

/// Complete v1 identity for one comparable experiment configuration.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExperimentManifestV1 {
    /// Contract generation.
    pub version: ExperimentManifestVersion,
    /// Lowercase SHA-256 of the canonical identity fields.
    #[schemars(regex(pattern = r"^[0-9a-f]{64}$"), length(equal = 64))]
    pub experiment_sha256: String,
    /// Agent implementation identity.
    pub agent: AgentIdentity,
    /// Provider/model identity.
    pub model: ModelIdentity,
    /// Tool policy identity.
    pub tool_policy: ToolPolicyIdentity,
    /// Workload and scorer identity.
    pub workload: WorkloadIdentity,
    /// Image and dependency pins.
    pub execution: ExecutionIdentity,
    /// Kernel, distribution, architecture, and CPU topology.
    pub platform: PlatformIdentity,
    /// Cache, load, retry, and replay controls.
    pub controls: RunControls,
}

impl ExperimentManifestV1 {
    /// Compute the content address after validating every identity component.
    pub fn compute_content_sha256(&self) -> Result<String, ExperimentManifestError> {
        self.validate_components()?;
        let mut encoder = HashEncoder::new();
        encoder.text(&self.agent.implementation);
        encoder.text(&self.agent.revision);
        encoder.text(&self.agent.binary_sha256);
        encoder.text(&self.model.provider);
        encoder.text(&self.model.model);
        encoder.optional_u16(self.model.settings.temperature_milli);
        encoder.optional_u32(self.model.settings.top_p_millionth);
        encoder.optional_u64(self.model.settings.seed);
        encoder.optional_u32(self.model.settings.max_output_tokens);
        encoder.optional_text(self.model.settings.reasoning_effort.as_deref());
        encoder.optional_text(self.model.settings.additional_settings_sha256.as_deref());
        encoder.text(&self.tool_policy.policy);
        encoder.text(&self.tool_policy.revision);
        encoder.text(&self.tool_policy.content_sha256);
        encoder.text(&self.workload.workload);
        encoder.text(&self.workload.workload_revision);
        encoder.text(&self.workload.workload_sha256);
        encoder.text(&self.workload.scorer_revision);
        encoder.text(&self.workload.scorer_sha256);
        encoder.text(&self.execution.image_sha256);
        encoder.text(&self.execution.dependencies_sha256);
        encoder.text(&self.platform.kernel_release);
        encoder.text(&self.platform.distribution);
        encoder.text(&self.platform.distribution_version);
        encoder.text(&self.platform.architecture);
        encoder.text(&self.platform.cpu_model);
        encoder.u32(self.platform.logical_cpu_count);
        encoder.u32(self.platform.numa_node_count);
        encoder.text(&self.platform.scaling_governor);
        encoder.byte(match self.controls.cache_state {
            CacheState::Cold => 0,
            CacheState::Warm => 1,
            CacheState::Uncontrolled => 2,
        });
        encoder.text(&self.controls.load_policy);
        encoder.u16(self.controls.retry.limit);
        encoder.text(&self.controls.retry.strategy);
        encoder.u64(self.controls.retry.backoff_ms);
        encoder.byte(match self.controls.replay.mode {
            ReplayMode::Live => 0,
            ReplayMode::Replay => 1,
        });
        encoder.optional_text(self.controls.replay.cassette_sha256.as_deref());
        encoder.text(&self.controls.replay.pacing);
        encoder.u64(self.controls.replay.cancellation_timeout_ms);
        Ok(encoder.finish())
    }

    /// Replace the content address with the canonical digest of this manifest.
    pub fn refresh_content_address(&mut self) -> Result<(), ExperimentManifestError> {
        self.experiment_sha256 = self.compute_content_sha256()?;
        Ok(())
    }

    /// Validate bounds, replay consistency, hashes, and the content address.
    pub fn validate(&self) -> Result<(), ExperimentManifestError> {
        let expected = self.compute_content_sha256()?;
        valid_sha256(&self.experiment_sha256, "experiment_sha256")?;
        if self.experiment_sha256 != expected {
            return Err(ExperimentManifestError::ContentAddressMismatch);
        }
        Ok(())
    }

    fn validate_components(&self) -> Result<(), ExperimentManifestError> {
        text(&self.agent.implementation, "agent.implementation")?;
        text(&self.agent.revision, "agent.revision")?;
        valid_sha256(&self.agent.binary_sha256, "agent.binary_sha256")?;
        text(&self.model.provider, "model.provider")?;
        text(&self.model.model, "model.model")?;
        self.model.settings.validate()?;
        text(&self.tool_policy.policy, "tool_policy.policy")?;
        text(&self.tool_policy.revision, "tool_policy.revision")?;
        valid_sha256(
            &self.tool_policy.content_sha256,
            "tool_policy.content_sha256",
        )?;
        text(&self.workload.workload, "workload.workload")?;
        text(
            &self.workload.workload_revision,
            "workload.workload_revision",
        )?;
        valid_sha256(&self.workload.workload_sha256, "workload.workload_sha256")?;
        text(&self.workload.scorer_revision, "workload.scorer_revision")?;
        valid_sha256(&self.workload.scorer_sha256, "workload.scorer_sha256")?;
        valid_sha256(&self.execution.image_sha256, "execution.image_sha256")?;
        valid_sha256(
            &self.execution.dependencies_sha256,
            "execution.dependencies_sha256",
        )?;
        text(&self.platform.kernel_release, "platform.kernel_release")?;
        text(&self.platform.distribution, "platform.distribution")?;
        text(
            &self.platform.distribution_version,
            "platform.distribution_version",
        )?;
        text(&self.platform.architecture, "platform.architecture")?;
        text(&self.platform.cpu_model, "platform.cpu_model")?;
        positive(
            self.platform.logical_cpu_count,
            "platform.logical_cpu_count",
        )?;
        positive(self.platform.numa_node_count, "platform.numa_node_count")?;
        text(&self.platform.scaling_governor, "platform.scaling_governor")?;
        text(&self.controls.load_policy, "controls.load_policy")?;
        text(&self.controls.retry.strategy, "controls.retry.strategy")?;
        text(&self.controls.replay.pacing, "controls.replay.pacing")?;
        if self.controls.replay.cancellation_timeout_ms == 0 {
            return Err(ExperimentManifestError::ZeroCount(
                "controls.replay.cancellation_timeout_ms",
            ));
        }
        match (
            self.controls.replay.mode,
            self.controls.replay.cassette_sha256.as_deref(),
        ) {
            (ReplayMode::Live, None) => {}
            (ReplayMode::Replay, Some(digest)) => {
                valid_sha256(digest, "controls.replay.cassette_sha256")?;
            }
            _ => return Err(ExperimentManifestError::InconsistentReplay),
        }
        Ok(())
    }
}

struct HashEncoder(Sha256);

impl HashEncoder {
    fn new() -> Self {
        let mut digest = Sha256::new();
        digest.update(b"asb-experiment-v1\0");
        Self(digest)
    }

    fn byte(&mut self, value: u8) {
        self.0.update([value]);
    }

    fn u16(&mut self, value: u16) {
        self.0.update(value.to_be_bytes());
    }

    fn u32(&mut self, value: u32) {
        self.0.update(value.to_be_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.0.update(value.to_be_bytes());
    }

    fn text(&mut self, value: &str) {
        self.u64(value.len() as u64);
        self.0.update(value.as_bytes());
    }

    fn optional_text(&mut self, value: Option<&str>) {
        self.byte(u8::from(value.is_some()));
        if let Some(value) = value {
            self.text(value);
        }
    }

    fn optional_u16(&mut self, value: Option<u16>) {
        self.byte(u8::from(value.is_some()));
        if let Some(value) = value {
            self.u16(value);
        }
    }

    fn optional_u32(&mut self, value: Option<u32>) {
        self.byte(u8::from(value.is_some()));
        if let Some(value) = value {
            self.u32(value);
        }
    }

    fn optional_u64(&mut self, value: Option<u64>) {
        self.byte(u8::from(value.is_some()));
        if let Some(value) = value {
            self.u64(value);
        }
    }

    fn finish(self) -> String {
        format!("{:x}", self.0.finalize())
    }
}

/// Invalid or self-inconsistent experiment provenance.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ExperimentManifestError {
    /// A required text field is empty, oversized, or contains control characters.
    #[error("invalid text field {0}")]
    InvalidText(&'static str),
    /// A digest is not exactly 64 lowercase hexadecimal characters.
    #[error("invalid SHA-256 field {0}")]
    InvalidSha256(&'static str),
    /// A model setting is outside its declared domain.
    #[error("invalid model settings")]
    InvalidSettings,
    /// A topology or timeout count that must be positive is zero.
    #[error("zero is not valid for {0}")]
    ZeroCount(&'static str),
    /// Live/replay mode and cassette presence disagree.
    #[error("replay mode and cassette presence disagree")]
    InconsistentReplay,
    /// The declared content address does not match the identity fields.
    #[error("experiment content address mismatch")]
    ContentAddressMismatch,
}

fn text(value: &str, field: &'static str) -> Result<(), ExperimentManifestError> {
    if value.is_empty()
        || value.len() > MAX_TEXT_BYTES
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return Err(ExperimentManifestError::InvalidText(field));
    }
    Ok(())
}

fn valid_sha256(value: &str, field: &'static str) -> Result<(), ExperimentManifestError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(ExperimentManifestError::InvalidSha256(field));
    }
    Ok(())
}

fn positive(value: u32, field: &'static str) -> Result<(), ExperimentManifestError> {
    if value == 0 {
        return Err(ExperimentManifestError::ZeroCount(field));
    }
    Ok(())
}

impl ModelSettings {
    fn validate(&self) -> Result<(), ExperimentManifestError> {
        if self.temperature_milli.is_some_and(|value| value > 2_000)
            || self.top_p_millionth.is_some_and(|value| value > 1_000_000)
            || self.max_output_tokens == Some(0)
        {
            return Err(ExperimentManifestError::InvalidSettings);
        }
        if let Some(effort) = &self.reasoning_effort {
            text(effort, "model.settings.reasoning_effort")?;
        }
        if let Some(digest) = &self.additional_settings_sha256 {
            valid_sha256(digest, "model.settings.additional_settings_sha256")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> ExperimentManifestV1 {
        let digest = "a".repeat(64);
        let mut value = ExperimentManifestV1 {
            version: EXPERIMENT_MANIFEST_V1,
            experiment_sha256: String::new(),
            agent: AgentIdentity {
                implementation: "asb-example".into(),
                revision: "1.0.0".into(),
                binary_sha256: digest.clone(),
            },
            model: ModelIdentity {
                provider: "example".into(),
                model: "model-v1".into(),
                settings: ModelSettings {
                    temperature_milli: Some(0),
                    top_p_millionth: Some(1_000_000),
                    seed: Some(7),
                    max_output_tokens: Some(4_096),
                    reasoning_effort: Some("medium".into()),
                    additional_settings_sha256: Some(digest.clone()),
                },
            },
            tool_policy: ToolPolicyIdentity {
                policy: "standard".into(),
                revision: "1".into(),
                content_sha256: digest.clone(),
            },
            workload: WorkloadIdentity {
                workload: "patch".into(),
                workload_revision: "abc123".into(),
                workload_sha256: digest.clone(),
                scorer_revision: "def456".into(),
                scorer_sha256: digest.clone(),
            },
            execution: ExecutionIdentity {
                image_sha256: digest.clone(),
                dependencies_sha256: digest,
            },
            platform: PlatformIdentity {
                kernel_release: "6.12.0".into(),
                distribution: "debian".into(),
                distribution_version: "13.6".into(),
                architecture: "x86_64".into(),
                cpu_model: "example-cpu".into(),
                logical_cpu_count: 8,
                numa_node_count: 1,
                scaling_governor: "performance".into(),
            },
            controls: RunControls {
                cache_state: CacheState::Cold,
                load_policy: "isolated".into(),
                retry: RetrySettings {
                    limit: 0,
                    strategy: "none".into(),
                    backoff_ms: 0,
                },
                replay: ReplaySettings {
                    mode: ReplayMode::Live,
                    cassette_sha256: None,
                    pacing: "provider".into(),
                    cancellation_timeout_ms: 10_000,
                },
            },
        };
        value.refresh_content_address().unwrap();
        value
    }

    #[test]
    fn address_is_deterministic_and_covers_controls() {
        let original = manifest();
        assert_eq!(original.validate(), Ok(()));
        assert_eq!(
            original.compute_content_sha256().unwrap(),
            original.experiment_sha256
        );
        let mut changed = original.clone();
        changed.controls.retry.limit = 1;
        assert_eq!(
            changed.validate(),
            Err(ExperimentManifestError::ContentAddressMismatch)
        );
        changed.refresh_content_address().unwrap();
        assert_ne!(changed.experiment_sha256, original.experiment_sha256);
    }

    #[test]
    fn replay_and_malformed_inputs_fail_closed() {
        let mut value = manifest();
        value.controls.replay.cassette_sha256 = Some("b".repeat(64));
        assert_eq!(
            value.compute_content_sha256(),
            Err(ExperimentManifestError::InconsistentReplay)
        );
        let mut value = manifest();
        value.agent.binary_sha256 = "A".repeat(64);
        assert_eq!(
            value.compute_content_sha256(),
            Err(ExperimentManifestError::InvalidSha256(
                "agent.binary_sha256"
            ))
        );
        let mut value = manifest();
        value.platform.logical_cpu_count = 0;
        assert_eq!(
            value.compute_content_sha256(),
            Err(ExperimentManifestError::ZeroCount(
                "platform.logical_cpu_count"
            ))
        );
    }

    #[test]
    fn canonical_address_is_independent_of_declared_address() {
        let mut value = manifest();
        let computed = value.compute_content_sha256().unwrap();
        value.experiment_sha256 = "f".repeat(64);
        assert_eq!(value.compute_content_sha256().unwrap(), computed);
        assert_eq!(
            value.validate(),
            Err(ExperimentManifestError::ContentAddressMismatch)
        );
    }

    #[test]
    fn every_optional_and_validation_boundary_is_exercised() {
        let mut value = manifest();
        value.controls.cache_state = CacheState::Uncontrolled;
        value.model.settings.temperature_milli = None;
        value.model.settings.top_p_millionth = None;
        value.model.settings.seed = None;
        value.model.settings.max_output_tokens = None;
        value.model.settings.reasoning_effort = None;
        value.model.settings.additional_settings_sha256 = None;
        value.refresh_content_address().unwrap();
        assert_eq!(value.validate(), Ok(()));

        let mut invalid = manifest();
        invalid.agent.implementation.clear();
        assert_eq!(
            invalid.compute_content_sha256(),
            Err(ExperimentManifestError::InvalidText("agent.implementation"))
        );
        let mut invalid = manifest();
        invalid.model.settings.temperature_milli = Some(2_001);
        assert_eq!(
            invalid.compute_content_sha256(),
            Err(ExperimentManifestError::InvalidSettings)
        );
        let mut invalid = manifest();
        invalid.controls.replay.cancellation_timeout_ms = 0;
        assert_eq!(
            invalid.compute_content_sha256(),
            Err(ExperimentManifestError::ZeroCount(
                "controls.replay.cancellation_timeout_ms"
            ))
        );
        let mut invalid = manifest();
        invalid.model.settings.additional_settings_sha256 = Some("short".into());
        assert_eq!(
            invalid.compute_content_sha256(),
            Err(ExperimentManifestError::InvalidSha256(
                "model.settings.additional_settings_sha256"
            ))
        );

        for bad in [" padded", "control\n", &"x".repeat(MAX_TEXT_BYTES + 1)] {
            let mut invalid = manifest();
            invalid.agent.implementation = bad.into();
            assert!(matches!(
                invalid.compute_content_sha256(),
                Err(ExperimentManifestError::InvalidText(_))
            ));
        }
        let mut invalid = manifest();
        invalid.agent.binary_sha256 = "short".into();
        assert!(matches!(
            invalid.compute_content_sha256(),
            Err(ExperimentManifestError::InvalidSha256(_))
        ));
        let mut invalid = manifest();
        invalid.model.settings.top_p_millionth = Some(1_000_001);
        assert_eq!(
            invalid.compute_content_sha256(),
            Err(ExperimentManifestError::InvalidSettings)
        );
        let mut invalid = manifest();
        invalid.model.settings.max_output_tokens = Some(0);
        assert_eq!(
            invalid.compute_content_sha256(),
            Err(ExperimentManifestError::InvalidSettings)
        );
        let mut invalid = manifest();
        invalid.platform.numa_node_count = 0;
        assert!(matches!(
            invalid.compute_content_sha256(),
            Err(ExperimentManifestError::ZeroCount(_))
        ));
        let mut invalid = manifest();
        invalid.controls.replay.mode = ReplayMode::Replay;
        invalid.controls.replay.cassette_sha256 = None;
        assert_eq!(
            invalid.compute_content_sha256(),
            Err(ExperimentManifestError::InconsistentReplay)
        );
        let mut invalid = manifest();
        invalid.controls.replay.mode = ReplayMode::Replay;
        invalid.controls.replay.cassette_sha256 = Some("bad".into());
        assert!(matches!(
            invalid.compute_content_sha256(),
            Err(ExperimentManifestError::InvalidSha256(_))
        ));
    }
}
