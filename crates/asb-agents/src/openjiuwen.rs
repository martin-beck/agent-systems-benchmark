// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Provenance-bound, pre-start OpenJiuwen adapter boundary.
//!
//! This module deliberately stops before process execution.  It validates the
//! immutable package identity and translates the common provider profile without
//! claiming live execution, streaming, usage, or model support.  Those claims
//! belong to the later qualification phases.

use crate::provider::ProviderProfileAdapter;
use asb_protocol::{
    CredentialSource, EndpointClass, ExtensionKind, ExtensionManifest, Id, OptionalSettingSupport,
    PROTOCOL_V1, ProviderKind, ProviderProfileCapabilities, ProviderProfileError,
    ProviderProfileV1, ProviderProfileVersion, ProviderSettingField, ProviderTransportLimits,
    VerifiedProviderProfile,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};
use url::Url;

/// OpenJiuwen release pinned by the provenance phase.
pub const SUPPORTED_VERSION: &str = "0.1.17.post1";
/// Immutable source revision inspected by the provenance phase.
pub const UPSTREAM_REVISION: &str = "4d56dad14b67cdb8fbb3fe58a5654ac8c3e3b815";
/// SHA-256 of the immutable OpenJiuwen wheel.
pub const PACKAGE_SHA256: &str = "21e9479c6b858cda28c250d63066f862fc0915cf2039edb00f016cbec7f9abba";
/// Maximum provider model identifier size accepted at this boundary.
pub const MAX_MODEL_BYTES: usize = 256;
const MAX_ENDPOINT_BYTES: usize = 4 * 1024;
const MAX_PROVIDER_BODY_BYTES: u32 = 16 * 1024 * 1024;
const MAX_PROVIDER_TIMEOUT_MS: u64 = 60 * 60 * 1_000;
const MAX_PROVIDER_CONCURRENCY: u16 = 1_024;

/// Immutable package identity understood by this adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpenJiuwenArtifact {
    /// The provenance-pinned universal wheel; execution remains unqualified.
    LinuxX86_64V0_1_17Post1,
}

impl OpenJiuwenArtifact {
    const fn digest(self) -> &'static str {
        match self {
            Self::LinuxX86_64V0_1_17Post1 => PACKAGE_SHA256,
        }
    }
}

/// Configuration error before any provider or filesystem side effect.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AdapterError {
    /// A path required for later isolated execution was relative.
    RelativePath(&'static str),
    /// The endpoint was not a safe explicit HTTP(S) endpoint.
    InvalidEndpoint,
    /// The model identifier was empty, oversized, or contained unsafe bytes.
    InvalidModel,
}

impl fmt::Display for AdapterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RelativePath(label) => write!(f, "{label} must be absolute"),
            Self::InvalidEndpoint => f.write_str("invalid OpenJiuwen provider endpoint"),
            Self::InvalidModel => f.write_str("invalid OpenJiuwen provider model"),
        }
    }
}

impl std::error::Error for AdapterError {}

/// Exact pre-start translation produced after provider negotiation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedOpenJiuwen {
    profile: ProviderProfileV1,
}

impl PreparedOpenJiuwen {
    /// Borrow the exact profile that will be passed to a future runner.
    pub fn profile(&self) -> &ProviderProfileV1 {
        &self.profile
    }
}

/// Validated OpenJiuwen package and provider boundary.
#[derive(Clone, Debug)]
pub struct OpenJiuwenConfig {
    executable: PathBuf,
    package: PathBuf,
    workspace: PathBuf,
    state_root: PathBuf,
    endpoint: Url,
    model: String,
    artifact: OpenJiuwenArtifact,
}

impl OpenJiuwenConfig {
    /// Validate paths, endpoint, model, and the provenance-pinned artifact.
    pub fn new(
        executable: impl Into<PathBuf>,
        package: impl Into<PathBuf>,
        workspace: impl Into<PathBuf>,
        state_root: impl Into<PathBuf>,
        endpoint: Url,
        model: impl Into<String>,
        artifact: OpenJiuwenArtifact,
    ) -> Result<Self, AdapterError> {
        let executable = executable.into();
        let package = package.into();
        let workspace = workspace.into();
        let state_root = state_root.into();
        for (label, path) in [
            ("OpenJiuwen executable", executable.as_path()),
            ("OpenJiuwen package", package.as_path()),
            ("workspace", workspace.as_path()),
            ("state root", state_root.as_path()),
        ] {
            if !path.is_absolute() {
                return Err(AdapterError::RelativePath(label));
            }
        }
        if !valid_endpoint(&endpoint) {
            return Err(AdapterError::InvalidEndpoint);
        }
        let model = model.into();
        if model.is_empty()
            || model.len() > MAX_MODEL_BYTES
            || !model.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/' | b':')
            })
        {
            return Err(AdapterError::InvalidModel);
        }
        Ok(Self {
            executable,
            package,
            workspace,
            state_root,
            endpoint,
            model,
            artifact,
        })
    }

    /// Stable manifest for the pinned package; no live capability is claimed.
    #[must_use]
    pub fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            extension_id: Id("agent.openjiuwen".into()),
            kind: ExtensionKind::Agent,
            implementation_version: format!("openjiuwen-{SUPPORTED_VERSION}+asb-0.1.0"),
            protocol: PROTOCOL_V1,
            capabilities: BTreeSet::new(),
            executable_sha256: self.artifact.digest().into(),
        }
    }

    /// Package path reserved for the later execution phase.
    pub fn executable(&self) -> &Path {
        &self.executable
    }

    /// Wheel/package path reserved for the later execution phase.
    pub fn package(&self) -> &Path {
        &self.package
    }

    /// Isolated workspace reserved for the later execution phase.
    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    /// Isolated state root reserved for the later execution phase.
    pub fn state_root(&self) -> &Path {
        &self.state_root
    }

    /// Canonical endpoint identity used in provider profiles without exposing its URL.
    pub fn endpoint_identity_sha256(&self) -> String {
        endpoint_identity_sha256(&self.endpoint)
    }

    /// Exact model selected for the future runner.
    pub fn model(&self) -> &str {
        &self.model
    }
}

impl ProviderProfileAdapter for OpenJiuwenConfig {
    type Prepared = PreparedOpenJiuwen;

    fn provider_profile_capabilities(&self) -> ProviderProfileCapabilities {
        let support = [
            ProviderSettingField::Temperature,
            ProviderSettingField::TopP,
            ProviderSettingField::Seed,
            ProviderSettingField::MaxOutputTokens,
            ProviderSettingField::ReasoningEffort,
            ProviderSettingField::AdditionalSettings,
        ]
        .into_iter()
        .map(|field| {
            (
                field,
                OptionalSettingSupport {
                    exact_value: true,
                    explicit_omission: true,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
        ProviderProfileCapabilities {
            minimum_version: ProviderProfileVersion { major: 1, minor: 0 },
            maximum_version: ProviderProfileVersion { major: 1, minor: 0 },
            providers: BTreeSet::from([ProviderKind::OpenAiCompatible]),
            endpoint_classes: BTreeSet::from([EndpointClass::Loopback, EndpointClass::Replay]),
            credential_sources: BTreeSet::from([CredentialSource::None]),
            settings: support,
            transport_ceiling: ProviderTransportLimits {
                max_request_bytes: MAX_PROVIDER_BODY_BYTES,
                max_response_bytes: MAX_PROVIDER_BODY_BYTES,
                connect_timeout_ms: MAX_PROVIDER_TIMEOUT_MS,
                request_timeout_ms: MAX_PROVIDER_TIMEOUT_MS,
                max_concurrent_requests: MAX_PROVIDER_CONCURRENCY,
            },
        }
    }

    fn prepare_provider_profile(
        &self,
        profile: &ProviderProfileV1,
    ) -> Result<Self::Prepared, ProviderProfileError> {
        if profile.provider != ProviderKind::OpenAiCompatible
            || profile.credential.source != CredentialSource::None
            || profile.model != self.model
            || profile.endpoint.identity_sha256 != self.endpoint_identity_sha256()
        {
            return Err(ProviderProfileError::LossyTranslation);
        }
        Ok(PreparedOpenJiuwen {
            profile: profile.clone(),
        })
    }

    fn effective_provider_profile(
        &self,
        prepared: &Self::Prepared,
    ) -> Result<ProviderProfileV1, ProviderProfileError> {
        Ok(prepared.profile.clone())
    }
}

/// Bind a profile and retain the constructor-controlled verification proof.
pub fn bind_provider_profile(
    config: &OpenJiuwenConfig,
    profile: &ProviderProfileV1,
) -> Result<(PreparedOpenJiuwen, VerifiedProviderProfile), ProviderProfileError> {
    let bound = crate::provider::bind_provider_profile(config, profile)?;
    Ok(bound.into_parts())
}

fn valid_endpoint(endpoint: &Url) -> bool {
    endpoint.as_str().len() <= MAX_ENDPOINT_BYTES
        && matches!(endpoint.scheme(), "http" | "https")
        && !endpoint.cannot_be_a_base()
        && endpoint.username().is_empty()
        && endpoint.password().is_none()
        && endpoint.query().is_none()
        && endpoint.fragment().is_none()
        && (endpoint.scheme() == "https" || endpoint.host_str().is_some_and(is_loopback))
}

fn is_loopback(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "localhost" | "::1" | "[::1]")
}

fn endpoint_identity_sha256(endpoint: &Url) -> String {
    let mut hasher = Sha256::new();
    hasher.update(endpoint.scheme().as_bytes());
    hasher.update([0]);
    hasher.update(endpoint.host_str().unwrap_or_default().as_bytes());
    hasher.update([0]);
    hasher.update(
        endpoint
            .port_or_known_default()
            .unwrap_or_default()
            .to_string()
            .as_bytes(),
    );
    hasher.update([0]);
    hasher.update(endpoint.path().as_bytes());
    format!("{:x}", hasher.finalize())
}
