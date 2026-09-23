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
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};

const MAX_ENDPOINT_BYTES: usize = 512;
const MAX_HOST_BYTES: usize = 128;

/// A concrete provider destination; DNS names are never accepted by the
/// network-capable backend.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ProviderEgressTarget(SocketAddr);

impl ProviderEgressTarget {
    /// Validate a public, concrete TCP destination.
    pub fn new(address: SocketAddr) -> Result<Self, ProviderEgressError> {
        if address.port() == 0 || !is_public_address(address.ip()) {
            return Err(ProviderEgressError::InvalidTarget);
        }
        Ok(Self(address))
    }
    /// Exact destination address.
    pub fn address(self) -> SocketAddr {
        self.0
    }
}

/// Bounded immutable provider destination allowlist.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderEgressAllowlist {
    targets: Vec<ProviderEgressTarget>,
}

impl ProviderEgressAllowlist {
    /// Maximum destinations admitted to one relay policy.
    pub const MAX_TARGETS: usize = 32;
    /// Construct a sorted, duplicate-free allowlist.
    pub fn new(mut targets: Vec<ProviderEgressTarget>) -> Result<Self, ProviderEgressError> {
        targets.sort_unstable();
        if targets.is_empty()
            || targets.len() > Self::MAX_TARGETS
            || targets.windows(2).any(|pair| pair[0] == pair[1])
        {
            return Err(ProviderEgressError::InvalidAllowlist);
        }
        Ok(Self { targets })
    }
    /// Exact target membership; no DNS, subnet, or wildcard matching.
    pub fn permits(&self, address: SocketAddr) -> bool {
        self.targets
            .binary_search_by_key(&address, |target| target.0)
            .is_ok()
    }
    /// Validated concrete targets in canonical order.
    pub fn targets(&self) -> &[ProviderEgressTarget] {
        &self.targets
    }
}

fn is_public_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(value) => {
            !value.is_unspecified()
                && !value.is_loopback()
                && !value.is_private()
                && !value.is_link_local()
                && !value.is_multicast()
                && !value.is_broadcast()
        }
        IpAddr::V6(value) => {
            !value.is_unspecified()
                && !value.is_loopback()
                && !value.is_unique_local()
                && !value.is_unicast_link_local()
                && !value.is_multicast()
        }
    }
}

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
    generation: String,
    route_sha256: String,
    deadline_unix_ms: u64,
}

impl ProviderEgressHandoff {
    /// Issue a handoff only from a validated policy; backend authorization is
    /// intentionally a separate runtime step.
    pub fn issue(policy: &ProviderEgressPolicy) -> Self {
        Self::issue_bound(policy, "generation-1", "route-1", u64::MAX)
    }

    /// Issue a launch-bound handoff with generation, route and deadline fences.
    pub fn issue_bound(
        policy: &ProviderEgressPolicy,
        generation: impl Into<String>,
        route_sha256: impl Into<String>,
        deadline_unix_ms: u64,
    ) -> Self {
        let policy_sha256 = format!("{:x}", Sha256::digest(policy.endpoint_sha256.as_bytes()));
        Self {
            policy_sha256,
            generation: generation.into(),
            route_sha256: route_sha256.into(),
            deadline_unix_ms,
        }
    }

    /// Return the opaque policy binding for a runtime launch.
    pub fn policy_sha256(&self) -> &str {
        &self.policy_sha256
    }
    /// Per-launch generation fence.
    pub fn generation(&self) -> &str {
        &self.generation
    }
    /// Adapter route identity fence.
    pub fn route_sha256(&self) -> &str {
        &self.route_sha256
    }
    /// Absolute deadline for the relay request.
    pub fn deadline_unix_ms(&self) -> u64 {
        self.deadline_unix_ms
    }
}

/// Runtime-owned authorization result for one provider request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderEgressAuthorization {
    endpoint_sha256: String,
    generation: String,
    route_sha256: String,
}

impl ProviderEgressAuthorization {
    /// Validate the handoff and exact endpoint before any socket operation.
    pub fn authorize(
        policy: &ProviderEgressPolicy,
        handoff: &ProviderEgressHandoff,
        now_unix_ms: u64,
        generation: &str,
        route_sha256: &str,
    ) -> Result<Self, ProviderEgressError> {
        if handoff.policy_sha256
            != format!("{:x}", Sha256::digest(policy.endpoint_sha256.as_bytes()))
            || handoff.generation != generation
            || handoff.route_sha256 != route_sha256
            || now_unix_ms > handoff.deadline_unix_ms
        {
            return Err(ProviderEgressError::MissingHandoff);
        }
        Ok(Self {
            endpoint_sha256: policy.endpoint_sha256.clone(),
            generation: generation.to_owned(),
            route_sha256: route_sha256.to_owned(),
        })
    }

    /// Return only non-secret launch identity metadata to the backend.
    pub fn endpoint_sha256(&self) -> &str {
        &self.endpoint_sha256
    }
    /// Return the authenticated generation fence.
    pub fn generation(&self) -> &str {
        &self.generation
    }
    /// Return the authenticated route fence.
    pub fn route_sha256(&self) -> &str {
        &self.route_sha256
    }
}

/// Bounded runtime-owned provider relay primitive.
pub struct ProviderEgressRelay {
    authorization: ProviderEgressAuthorization,
    allowlist: ProviderEgressAllowlist,
    connect_timeout: Duration,
    max_request_bytes: usize,
    max_response_bytes: usize,
}

impl ProviderEgressRelay {
    /// Construct a relay only from an authenticated launch and concrete targets.
    pub fn new(
        authorization: ProviderEgressAuthorization,
        allowlist: ProviderEgressAllowlist,
        connect_timeout: Duration,
        max_request_bytes: usize,
        max_response_bytes: usize,
    ) -> Result<Self, ProviderEgressError> {
        if connect_timeout.is_zero()
            || connect_timeout > Duration::from_secs(30)
            || max_request_bytes == 0
            || max_request_bytes > 16 * 1024 * 1024
            || max_response_bytes == 0
            || max_response_bytes > 16 * 1024 * 1024
        {
            return Err(ProviderEgressError::InvalidRelayBounds);
        }
        Ok(Self {
            authorization,
            allowlist,
            connect_timeout,
            max_request_bytes,
            max_response_bytes,
        })
    }

    /// Perform one bounded byte relay to the exact allowlisted target.
    /// TLS framing belongs to the caller; this primitive never follows
    /// redirects or resolves names and never exposes credentials in errors.
    pub fn round_trip(
        &self,
        target: SocketAddr,
        request: &[u8],
        deadline: Instant,
    ) -> Result<Vec<u8>, ProviderEgressError> {
        if !self.allowlist.permits(target) || request.len() > self.max_request_bytes {
            return Err(ProviderEgressError::TargetDenied);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(ProviderEgressError::DeadlineExceeded);
        }
        let timeout = remaining.min(self.connect_timeout);
        let mut stream = std::net::TcpStream::connect_timeout(&target, timeout)
            .map_err(|_| ProviderEgressError::TransportDenied)?;
        stream
            .set_read_timeout(Some(timeout))
            .map_err(|_| ProviderEgressError::TransportDenied)?;
        stream
            .set_write_timeout(Some(timeout))
            .map_err(|_| ProviderEgressError::TransportDenied)?;
        stream
            .write_all(request)
            .map_err(|_| ProviderEgressError::TransportDenied)?;
        let mut response = Vec::new();
        stream
            .take((self.max_response_bytes + 1) as u64)
            .read_to_end(&mut response)
            .map_err(|_| ProviderEgressError::TransportDenied)?;
        if response.len() > self.max_response_bytes {
            return Err(ProviderEgressError::ResponseTooLarge);
        }
        let _ = &self.authorization;
        Ok(response)
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
    /// Concrete destination or allowlist was invalid.
    InvalidTarget,
    /// Destination allowlist was empty, oversized, or duplicated.
    InvalidAllowlist,
    /// Relay bounds were zero or exceeded hard limits.
    InvalidRelayBounds,
    /// Target was not in the authenticated allowlist or request was oversized.
    TargetDenied,
    /// Relay deadline elapsed before transport completion.
    DeadlineExceeded,
    /// Underlying transport was unavailable or timed out.
    TransportDenied,
    /// Response exceeded the authenticated byte budget.
    ResponseTooLarge,
}

impl fmt::Display for ProviderEgressError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidIdentity => "provider egress identity is invalid",
            Self::UnsupportedScheme => "provider egress requires HTTPS",
            Self::MissingHandoff => "provider egress handoff is missing",
            Self::InvalidTarget => "provider egress target is invalid",
            Self::InvalidAllowlist => "provider egress allowlist is invalid",
            Self::InvalidRelayBounds => "provider egress relay bounds are invalid",
            Self::TargetDenied => "provider egress target is denied",
            Self::DeadlineExceeded => "provider egress deadline exceeded",
            Self::TransportDenied => "provider egress transport denied",
            Self::ResponseTooLarge => "provider egress response is too large",
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

    #[test]
    fn authorization_rejects_stale_route_generation_and_deadline() {
        let policy =
            ProviderEgressPolicy::new("https://openrouter.ai/api/v1", "openrouter.ai").unwrap();
        let handoff = ProviderEgressHandoff::issue_bound(&policy, "g-1", "route-1", 100);
        let auth = ProviderEgressAuthorization::authorize(&policy, &handoff, 99, "g-1", "route-1")
            .unwrap();
        assert_eq!(auth.endpoint_sha256(), policy.endpoint_sha256());
        assert!(
            ProviderEgressAuthorization::authorize(&policy, &handoff, 99, "g-2", "route-1")
                .is_err()
        );
        assert!(
            ProviderEgressAuthorization::authorize(&policy, &handoff, 99, "g-1", "route-2")
                .is_err()
        );
        assert!(
            ProviderEgressAuthorization::authorize(&policy, &handoff, 101, "g-1", "route-1")
                .is_err()
        );
    }

    #[test]
    fn concrete_allowlist_is_public_exact_and_bounded() {
        let first = ProviderEgressTarget::new("198.51.100.10:443".parse().unwrap()).unwrap();
        let second = ProviderEgressTarget::new("198.51.100.11:443".parse().unwrap()).unwrap();
        let policy = ProviderEgressAllowlist::new(vec![second, first]).unwrap();
        assert_eq!(policy.targets(), &[first, second]);
        assert!(policy.permits(first.address()));
        assert!(!policy.permits("198.51.100.10:8443".parse().unwrap()));
        assert!(ProviderEgressAllowlist::new(vec![first, first]).is_err());
        for address in ["127.0.0.1:443", "10.0.0.1:443", "[::1]:443"] {
            assert_eq!(
                ProviderEgressTarget::new(address.parse().unwrap()),
                Err(ProviderEgressError::InvalidTarget)
            );
        }
    }

    #[test]
    fn relay_denies_unlisted_target_expired_deadline_and_bad_bounds() {
        let policy =
            ProviderEgressPolicy::new("https://openrouter.ai/api/v1", "openrouter.ai").unwrap();
        let handoff = ProviderEgressHandoff::issue_bound(&policy, "g-1", "route-1", u64::MAX);
        let auth =
            ProviderEgressAuthorization::authorize(&policy, &handoff, 0, "g-1", "route-1").unwrap();
        let target = ProviderEgressTarget::new("198.51.100.10:443".parse().unwrap()).unwrap();
        let relay = ProviderEgressRelay::new(
            auth,
            ProviderEgressAllowlist::new(vec![target]).unwrap(),
            Duration::from_secs(1),
            1024,
            1024,
        )
        .unwrap();
        assert_eq!(
            relay.round_trip(
                "198.51.100.11:443".parse().unwrap(),
                b"x",
                Instant::now() + Duration::from_secs(1)
            ),
            Err(ProviderEgressError::TargetDenied)
        );
        assert_eq!(
            relay.round_trip(
                target.address(),
                b"x",
                Instant::now() - Duration::from_secs(1)
            ),
            Err(ProviderEgressError::DeadlineExceeded)
        );
        assert!(
            ProviderEgressRelay::new(
                ProviderEgressAuthorization::authorize(&policy, &handoff, 0, "g-1", "route-1")
                    .unwrap(),
                ProviderEgressAllowlist::new(vec![target]).unwrap(),
                Duration::ZERO,
                1024,
                1024,
            )
            .is_err()
        );
    }
}
