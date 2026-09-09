// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Content-addressed provider-aware launch binding.

use crate::all_agents_provider::{EffectiveApiMode, SelectedAgent};
use crate::openai::{OpenAiAgent, OpenAiProfile};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

/// Current provider-aware launch contract.
pub const PROVIDER_LAUNCH_V1: u16 = 1;
const MAX_ID_BYTES: usize = 128;

/// Credential resolver kind represented without secret material.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialResolverKind {
    /// Explicit environment resolver.
    Environment,
    /// Already-open descriptor resolver.
    FileDescriptor,
    /// Content-pinned helper resolver.
    Helper,
}

/// Public credential resolver identity.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialResolverIdentity {
    /// Resolver implementation kind.
    pub kind: CredentialResolverKind,
    /// Digest of the logical reference, never the secret value.
    pub reference_sha256: String,
}

/// Verified runtime bundle identity.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeBundleIdentity {
    /// Content digest of the complete runtime bundle.
    pub bundle_sha256: String,
    /// Content digest of the executable selected from that bundle.
    pub executable_sha256: String,
}

/// Bounded process policy included in the launch identity.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchPolicy {
    /// Maximum captured stdout bytes.
    pub max_stdout_bytes: u64,
    /// Maximum captured stderr bytes.
    pub max_stderr_bytes: u64,
    /// Maximum process lifetime in milliseconds.
    pub timeout_ms: u64,
    /// Maximum environment entries supplied to the adapter.
    pub max_environment_entries: u16,
    /// Maximum argv entries supplied to the adapter.
    pub max_argv_entries: u16,
}

/// Closed input to one provider-aware adapter launch.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderLaunchV1 {
    /// Contract version.
    pub schema_version: u16,
    /// Provider catalog content identity.
    pub catalog_sha256: String,
    /// Complete selection content identity.
    pub selection_sha256: String,
    /// Exact provider profile settings identity.
    pub provider_profile_sha256: String,
    /// Stable selected agent identity.
    pub agent: String,
    /// Exact adapter identity.
    pub adapter: String,
    /// Exact provider API route.
    pub api_mode: EffectiveApiMode,
    /// Provider family.
    pub provider: String,
    /// Exact model spelling.
    pub model: String,
    /// Exact settings identity.
    pub settings_sha256: String,
    /// Runtime and executable identities.
    pub runtime: RuntimeBundleIdentity,
    /// Secret-free resolver identity.
    pub credential: CredentialResolverIdentity,
    /// Workload content identity.
    pub workload_sha256: String,
    /// Durable run identity.
    pub run_id: String,
    /// Durable attempt identity.
    pub attempt_id: String,
    /// Bounded process/artifact policy.
    pub policy: LaunchPolicy,
}

/// Constructor-controlled launch proof. The digest cannot be supplied independently.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderLaunchRecord {
    /// Exact launch input.
    pub input: ProviderLaunchV1,
    /// Canonical digest of `input`.
    pub launch_sha256: String,
}

/// Launch binding failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProviderLaunchError {
    /// A required digest is malformed.
    InvalidDigest(&'static str),
    /// An identity is malformed or empty.
    InvalidIdentity(&'static str),
    /// The contract version is unsupported.
    UnsupportedVersion(u16),
    /// The adapter projection does not match the requested input.
    ProjectionMismatch(&'static str),
    /// The record digest does not match its input.
    DigestMismatch,
    /// The selected adapter has no exact pinned route.
    UnsupportedAdapter,
}

impl fmt::Display for ProviderLaunchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDigest(field) => {
                write!(formatter, "invalid provider launch digest: {field}")
            }
            Self::InvalidIdentity(field) => {
                write!(formatter, "invalid provider launch identity: {field}")
            }
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported provider launch version: {version}")
            }
            Self::ProjectionMismatch(field) => {
                write!(formatter, "provider adapter projection mismatch: {field}")
            }
            Self::DigestMismatch => {
                formatter.write_str("provider launch digest does not match input")
            }
            Self::UnsupportedAdapter => {
                formatter.write_str("selected adapter has no exact pinned provider route")
            }
        }
    }
}

impl std::error::Error for ProviderLaunchError {}

impl ProviderLaunchV1 {
    /// Validate the closed input and return its canonical content digest.
    pub fn digest(&self) -> Result<String, ProviderLaunchError> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(|_| ProviderLaunchError::DigestMismatch)?;
        let mut digest = Sha256::new();
        digest.update(b"asb-provider-launch-v1\0");
        digest.update(bytes);
        Ok(format!("{:x}", digest.finalize()))
    }

    /// Validate all bounded and cross-field invariants.
    pub fn validate(&self) -> Result<(), ProviderLaunchError> {
        if self.schema_version != PROVIDER_LAUNCH_V1 {
            return Err(ProviderLaunchError::UnsupportedVersion(self.schema_version));
        }
        for (name, value) in [
            ("catalog_sha256", self.catalog_sha256.as_str()),
            ("selection_sha256", self.selection_sha256.as_str()),
            (
                "provider_profile_sha256",
                self.provider_profile_sha256.as_str(),
            ),
            ("settings_sha256", self.settings_sha256.as_str()),
            ("workload_sha256", self.workload_sha256.as_str()),
            ("runtime.bundle_sha256", self.runtime.bundle_sha256.as_str()),
            (
                "runtime.executable_sha256",
                self.runtime.executable_sha256.as_str(),
            ),
            (
                "credential.reference_sha256",
                self.credential.reference_sha256.as_str(),
            ),
        ] {
            if !is_sha256(value) {
                return Err(ProviderLaunchError::InvalidDigest(name));
            }
        }
        for (name, value) in [
            ("agent", self.agent.as_str()),
            ("adapter", self.adapter.as_str()),
            ("provider", self.provider.as_str()),
            ("model", self.model.as_str()),
            ("run_id", self.run_id.as_str()),
            ("attempt_id", self.attempt_id.as_str()),
        ] {
            if value.is_empty()
                || value.len() > MAX_ID_BYTES
                || !value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'/'))
            {
                return Err(ProviderLaunchError::InvalidIdentity(name));
            }
        }
        if self.policy.max_stdout_bytes == 0
            || self.policy.max_stderr_bytes == 0
            || self.policy.timeout_ms == 0
            || self.policy.max_environment_entries == 0
            || self.policy.max_argv_entries == 0
        {
            return Err(ProviderLaunchError::InvalidIdentity("policy"));
        }
        Ok(())
    }
}

impl ProviderLaunchRecord {
    /// Build a constructor-controlled record after adapter projection validation.
    pub fn bind(
        input: ProviderLaunchV1,
        projection: &ProviderLaunchProjection,
    ) -> Result<Self, ProviderLaunchError> {
        input.validate()?;
        projection.validate_against(&input)?;
        let launch_sha256 = input.digest()?;
        Ok(Self {
            input,
            launch_sha256,
        })
    }

    /// Validate the record and its canonical digest.
    pub fn validate(&self) -> Result<(), ProviderLaunchError> {
        let expected = self.input.digest()?;
        if self.launch_sha256 != expected {
            return Err(ProviderLaunchError::DigestMismatch);
        }
        Ok(())
    }
}

/// Exact adapter projection obtained from a constructor-controlled adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderLaunchProjection {
    /// Agent identity applied by the adapter.
    agent: String,
    /// Adapter identity applied by the adapter.
    adapter: String,
    /// Provider family applied by the adapter.
    provider: String,
    /// Model applied by the adapter.
    model: String,
    /// API mode applied by the adapter.
    api_mode: EffectiveApiMode,
    /// Settings identity applied by the adapter.
    settings_sha256: String,
    /// Resolver identity applied by the adapter.
    credential: CredentialResolverIdentity,
}

impl ProviderLaunchProjection {
    /// Agent identity applied by the adapter.
    pub fn agent(&self) -> &str {
        &self.agent
    }
    /// Adapter identity applied by the adapter.
    pub fn adapter(&self) -> &str {
        &self.adapter
    }
    /// Provider family applied by the adapter.
    pub fn provider(&self) -> &str {
        &self.provider
    }
    /// Model spelling applied by the adapter.
    pub fn model(&self) -> &str {
        &self.model
    }
    /// API route applied by the adapter.
    pub const fn api_mode(&self) -> EffectiveApiMode {
        self.api_mode
    }
    /// Settings identity applied by the adapter.
    pub fn settings_sha256(&self) -> &str {
        &self.settings_sha256
    }
    /// Secret-free credential resolver identity applied by the adapter.
    pub fn credential(&self) -> &CredentialResolverIdentity {
        &self.credential
    }

    /// Construct an exact projection from the pinned OpenAI adapter translation.
    pub fn openai(
        profile: &OpenAiProfile,
        agent: SelectedAgent,
    ) -> Result<Self, ProviderLaunchError> {
        let openai_agent = match agent {
            SelectedAgent::OpenCode => OpenAiAgent::OpenCode,
            SelectedAgent::OpenDesk => OpenAiAgent::OpenDesk,
            SelectedAgent::Aider => OpenAiAgent::Aider,
            SelectedAgent::Codex => OpenAiAgent::Codex,
            SelectedAgent::Gemini => OpenAiAgent::Gemini,
            SelectedAgent::QwenCode => OpenAiAgent::QwenCode,
            SelectedAgent::Goose => OpenAiAgent::Goose,
            SelectedAgent::MiniSwe => OpenAiAgent::MiniSwe,
            SelectedAgent::OpenHands => OpenAiAgent::OpenHands,
        };
        let route = profile
            .translate(openai_agent, profile.provider_profile())
            .map_err(|_| ProviderLaunchError::UnsupportedAdapter)?;
        let api_mode = match route.api_mode() {
            crate::openai::OpenAiApiMode::ChatCompletions => EffectiveApiMode::ChatCompletions,
            crate::openai::OpenAiApiMode::Responses => EffectiveApiMode::Responses,
        };
        Ok(Self {
            agent: agent_id(agent).to_owned(),
            adapter: agent_id(agent).to_owned(),
            provider: "openai".to_owned(),
            model: route.model().to_owned(),
            api_mode,
            settings_sha256: profile.provider_profile().settings_sha256.clone(),
            credential: CredentialResolverIdentity {
                kind: CredentialResolverKind::Environment,
                reference_sha256: profile
                    .provider_profile()
                    .credential
                    .reference_sha256
                    .clone()
                    .ok_or(ProviderLaunchError::InvalidDigest(
                        "credential.reference_sha256",
                    ))?,
            },
        })
    }

    fn validate_against(&self, input: &ProviderLaunchV1) -> Result<(), ProviderLaunchError> {
        if self.agent != input.agent {
            return Err(ProviderLaunchError::ProjectionMismatch("agent"));
        }
        if self.adapter != input.adapter {
            return Err(ProviderLaunchError::ProjectionMismatch("adapter"));
        }
        if self.provider != input.provider {
            return Err(ProviderLaunchError::ProjectionMismatch("provider"));
        }
        if self.model != input.model {
            return Err(ProviderLaunchError::ProjectionMismatch("model"));
        }
        if self.api_mode != input.api_mode {
            return Err(ProviderLaunchError::ProjectionMismatch("api_mode"));
        }
        if self.settings_sha256 != input.settings_sha256 {
            return Err(ProviderLaunchError::ProjectionMismatch("settings_sha256"));
        }
        if self.credential != input.credential {
            return Err(ProviderLaunchError::ProjectionMismatch("credential"));
        }
        Ok(())
    }
}

fn agent_id(agent: SelectedAgent) -> &'static str {
    match agent {
        SelectedAgent::OpenCode => "opencode",
        SelectedAgent::OpenDesk => "opendesk",
        SelectedAgent::Aider => "aider",
        SelectedAgent::Codex => "codex",
        SelectedAgent::Gemini => "gemini",
        SelectedAgent::QwenCode => "qwen_code",
        SelectedAgent::Goose => "goose",
        SelectedAgent::MiniSwe => "mini_swe",
        SelectedAgent::OpenHands => "openhands",
    }
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> ProviderLaunchV1 {
        ProviderLaunchV1 {
            schema_version: PROVIDER_LAUNCH_V1,
            catalog_sha256: "a".repeat(64),
            selection_sha256: "b".repeat(64),
            provider_profile_sha256: "c".repeat(64),
            agent: "codex".into(),
            adapter: "codex".into(),
            api_mode: EffectiveApiMode::Responses,
            provider: "openai".into(),
            model: "pinned-model".into(),
            settings_sha256: "d".repeat(64),
            runtime: RuntimeBundleIdentity {
                bundle_sha256: "e".repeat(64),
                executable_sha256: "f".repeat(64),
            },
            credential: CredentialResolverIdentity {
                kind: CredentialResolverKind::Environment,
                reference_sha256: "1".repeat(64),
            },
            workload_sha256: "2".repeat(64),
            run_id: "run-1".into(),
            attempt_id: "run-1-attempt".into(),
            policy: LaunchPolicy {
                max_stdout_bytes: 1,
                max_stderr_bytes: 1,
                timeout_ms: 1,
                max_environment_entries: 1,
                max_argv_entries: 1,
            },
        }
    }

    fn projection(value: &ProviderLaunchV1) -> ProviderLaunchProjection {
        ProviderLaunchProjection {
            agent: value.agent.clone(),
            adapter: value.adapter.clone(),
            provider: value.provider.clone(),
            model: value.model.clone(),
            api_mode: value.api_mode,
            settings_sha256: value.settings_sha256.clone(),
            credential: value.credential.clone(),
        }
    }

    #[test]
    fn canonical_binding_rejects_projection_drift_and_tamper() {
        let value = input();
        let record = ProviderLaunchRecord::bind(value.clone(), &projection(&value)).unwrap();
        record.validate().unwrap();
        let mut changed = value;
        changed.model = "other".into();
        assert_eq!(
            ProviderLaunchRecord {
                input: changed,
                launch_sha256: record.launch_sha256.clone()
            }
            .validate(),
            Err(ProviderLaunchError::DigestMismatch)
        );
        let mut drift = projection(&record.input);
        drift.model = "other".into();
        assert_eq!(
            ProviderLaunchRecord::bind(record.input, &drift),
            Err(ProviderLaunchError::ProjectionMismatch("model"))
        );
    }

    #[test]
    fn secrets_cannot_be_represented_in_the_launch_record() {
        let json = serde_json::to_string(
            &ProviderLaunchRecord::bind(input(), &projection(&input())).unwrap(),
        )
        .unwrap();
        assert!(!json.contains("Bearer"));
        assert!(!json.contains("secret"));
    }
}
