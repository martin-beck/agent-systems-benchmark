// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Fail-closed identity contract for live provider egress.
//!
//! This contract deliberately does not weaken the offline sandbox. A future
//! runtime backend must present a validated [`ProviderEgressHandoff`] before
//! creating a network-capable launch; callers cannot derive one from a digest
//! or endpoint string alone.

use sha2::{Digest, Sha256};
use std::fmt;

const MAX_ENDPOINT_BYTES: usize = 512;
const MAX_HOST_BYTES: usize = 128;

/// Exact endpoint identity allowed for one live provider launch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderEgressPolicy {
    endpoint: String,
    host: String,
    endpoint_sha256: String,
}

impl ProviderEgressPolicy {
    /// Validate a public HTTPS endpoint and bind it to its exact host.
    pub fn new(
        endpoint: impl Into<String>,
        allowed_host: impl Into<String>,
    ) -> Result<Self, ProviderEgressError> {
        let endpoint = endpoint.into();
        let host = allowed_host.into();
        if endpoint.len() > MAX_ENDPOINT_BYTES
            || host.is_empty()
            || host.len() > MAX_HOST_BYTES
            || endpoint.contains('?')
            || endpoint.contains('#')
        {
            return Err(ProviderEgressError::InvalidIdentity);
        }
        let rest = endpoint
            .strip_prefix("https://")
            .ok_or(ProviderEgressError::UnsupportedScheme)?;
        let authority = rest.split('/').next().unwrap_or_default();
        if authority.is_empty()
            || authority.contains('@')
            || authority.contains('?')
            || authority.contains('#')
        {
            return Err(ProviderEgressError::InvalidIdentity);
        }
        if authority != host
            || !host
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
        {
            return Err(ProviderEgressError::InvalidIdentity);
        }
        let endpoint_sha256 = format!("{:x}", Sha256::digest(endpoint.as_bytes()));
        Ok(Self {
            endpoint,
            host,
            endpoint_sha256,
        })
    }

    /// Public endpoint identity, retained only for launch binding.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
    /// Exact host permitted by the policy.
    pub fn host(&self) -> &str {
        &self.host
    }
    /// Digest bound into the provider launch record.
    pub fn endpoint_sha256(&self) -> &str {
        &self.endpoint_sha256
    }
}

/// Runtime-issued proof that the endpoint policy was accepted by a backend.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderEgressHandoff {
    policy_sha256: String,
}

impl ProviderEgressHandoff {
    /// Issue a handoff only from a validated policy; backend authorization is
    /// intentionally a separate runtime step.
    pub fn issue(policy: &ProviderEgressPolicy) -> Self {
        let policy_sha256 = format!("{:x}", Sha256::digest(policy.endpoint_sha256.as_bytes()));
        Self { policy_sha256 }
    }

    /// Return the opaque policy binding for a runtime launch.
    pub fn policy_sha256(&self) -> &str {
        &self.policy_sha256
    }
}

/// Why a live provider egress request was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderEgressError {
    /// Endpoint identity or host was malformed.
    InvalidIdentity,
    /// A non-HTTPS endpoint was supplied.
    UnsupportedScheme,
    /// The backend has not issued an authenticated handoff.
    MissingHandoff,
}

impl fmt::Display for ProviderEgressError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidIdentity => "provider egress identity is invalid",
            Self::UnsupportedScheme => "provider egress requires HTTPS",
            Self::MissingHandoff => "provider egress handoff is missing",
        })
    }
}

impl std::error::Error for ProviderEgressError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_binds_exact_https_host_without_secret_material() {
        let policy =
            ProviderEgressPolicy::new("https://openrouter.ai/api/v1", "openrouter.ai").unwrap();
        assert_eq!(policy.host(), "openrouter.ai");
        assert_eq!(policy.endpoint(), "https://openrouter.ai/api/v1");
        assert!(!policy.endpoint_sha256().contains("api_key"));
        assert_ne!(
            ProviderEgressHandoff::issue(&policy).policy_sha256(),
            policy.endpoint_sha256()
        );
    }

    #[test]
    fn policy_rejects_non_https_host_confusion_and_credentials() {
        for (endpoint, host) in [
            ("http://openrouter.ai/api/v1", "openrouter.ai"),
            ("https://evil.example/api/v1", "openrouter.ai"),
            ("https://token@openrouter.ai/api/v1", "openrouter.ai"),
            (
                "https://openrouter.ai/api/v1?api_key=secret",
                "openrouter.ai",
            ),
        ] {
            assert!(ProviderEgressPolicy::new(endpoint, host).is_err());
        }
    }
}
