// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Fail-closed identity contract for live provider egress.
//!
//! This contract deliberately does not weaken the offline sandbox. A future
//! runtime backend must present a validated [`provider_egress::ProviderEgressHandoff`] before
//! creating a network-capable launch; callers cannot derive one from a digest
//! or endpoint string alone.

use sha2::{Digest, Sha256};
use std::fmt;
use std::io;
#[cfg(test)]
use std::io::Read;
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MAX_ENDPOINT_BYTES: usize = 512;
const MAX_HOST_BYTES: usize = 128;
const MAX_FORWARD_BYTES: usize = 16 * 1024 * 1024;

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

    #[cfg(test)]
    fn test_only(address: SocketAddr) -> Self {
        Self(address)
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
            value
                .to_ipv4()
                .is_none_or(|mapped| is_public_address(IpAddr::V4(mapped)))
                && !value.is_unspecified()
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
#[derive(Debug, Eq, PartialEq)]
pub struct ProviderEgressAuthorization {
    endpoint_sha256: String,
    generation: String,
    route_sha256: String,
    deadline_unix_ms: u64,
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
            || now_unix_ms >= handoff.deadline_unix_ms
        {
            return Err(ProviderEgressError::MissingHandoff);
        }
        Ok(Self {
            endpoint_sha256: policy.endpoint_sha256.clone(),
            generation: generation.to_owned(),
            route_sha256: route_sha256.to_owned(),
            deadline_unix_ms: handoff.deadline_unix_ms,
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
    /// Absolute handoff expiry fence.
    pub fn deadline_unix_ms(&self) -> u64 {
        self.deadline_unix_ms
    }
}

/// Runtime-owned connector for one authorized provider destination.
///
/// This is intentionally a transport primitive, not a sandbox policy. The
/// caller must already have an isolated child and a separate authenticated
/// loopback handoff. The relay connects to the exact allowlisted socket
/// address; it never performs DNS, follows redirects, or accepts a URL from
/// the child.
#[derive(Clone, Debug)]
pub struct ProviderEgressRelay {
    endpoint_sha256: String,
    allowlist: ProviderEgressAllowlist,
    generation: String,
    route_sha256: String,
    io_timeout: Duration,
    #[cfg(test)]
    max_forward_bytes: usize,
}

impl ProviderEgressRelay {
    /// Build a bounded relay bound to one validated endpoint policy.
    pub fn new(
        policy: &ProviderEgressPolicy,
        allowlist: ProviderEgressAllowlist,
        generation: impl Into<String>,
        route_sha256: impl Into<String>,
        io_timeout: Duration,
        max_forward_bytes: usize,
    ) -> Result<Self, ProviderEgressError> {
        if io_timeout.is_zero() || max_forward_bytes == 0 || max_forward_bytes > MAX_FORWARD_BYTES {
            return Err(ProviderEgressError::InvalidBounds);
        }
        Ok(Self {
            endpoint_sha256: policy.endpoint_sha256.clone(),
            allowlist,
            generation: generation.into(),
            route_sha256: route_sha256.into(),
            io_timeout,
            #[cfg(test)]
            max_forward_bytes,
        })
    }

    /// Connect to one exact allowlisted address after validating the launch
    /// authorization and absolute deadline. The returned stream is bounded by
    /// the configured socket timeout; callers must use `forward_bounded` for
    /// byte accounting.
    pub fn connect(
        &self,
        authorization: ProviderEgressAuthorization,
        target: ProviderEgressTarget,
        deadline: Instant,
    ) -> Result<TcpStream, ProviderEgressError> {
        let now_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| ProviderEgressError::DeadlineExceeded)?
            .as_millis() as u64;
        if now_unix_ms >= authorization.deadline_unix_ms
            || authorization.endpoint_sha256 != self.endpoint_sha256
            || authorization.generation != self.generation
            || authorization.route_sha256 != self.route_sha256
            || !self.allowlist.permits(target.address())
        {
            return Err(ProviderEgressError::UnauthorizedTarget);
        }
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or(ProviderEgressError::DeadlineExceeded)?;
        let handoff_remaining =
            Duration::from_millis(authorization.deadline_unix_ms.saturating_sub(now_unix_ms));
        let timeout = self.io_timeout.min(remaining).min(handoff_remaining);
        let stream = TcpStream::connect_timeout(&target.address(), timeout)
            .map_err(ProviderEgressError::Connect)?;
        stream
            .set_read_timeout(Some(timeout))
            .map_err(ProviderEgressError::Connect)?;
        stream
            .set_write_timeout(Some(timeout))
            .map_err(ProviderEgressError::Connect)?;
        Ok(stream)
    }

    /// Copy one bounded half of a runtime TCP relay stream. Socket deadlines
    /// are refreshed before every blocking operation, so the absolute
    /// deadline is enforced independently of the initial connect timeout.
    #[cfg(test)]
    fn forward_bounded<W: std::io::Write>(
        &self,
        stream: &mut TcpStream,
        writer: &mut W,
        deadline: Instant,
    ) -> Result<usize, ProviderEgressError> {
        let mut buffer = [0_u8; 16 * 1024];
        let mut total = 0;
        loop {
            let remaining_time = deadline
                .checked_duration_since(Instant::now())
                .ok_or(ProviderEgressError::DeadlineExceeded)?;
            stream
                .set_read_timeout(Some(remaining_time))
                .map_err(ProviderEgressError::Io)?;
            let remaining = self.max_forward_bytes - total;
            if remaining == 0 {
                let mut extra = [0_u8; 1];
                let count = stream.read(&mut extra).map_err(ProviderEgressError::Io)?;
                return if count == 0 {
                    Ok(total)
                } else {
                    Err(ProviderEgressError::ByteLimitExceeded)
                };
            }
            let read_size = buffer.len().min(remaining);
            let count = stream
                .read(&mut buffer[..read_size])
                .map_err(ProviderEgressError::Io)?;
            if count == 0 {
                return Ok(total);
            }
            let remaining_time = deadline
                .checked_duration_since(Instant::now())
                .ok_or(ProviderEgressError::DeadlineExceeded)?;
            stream
                .set_write_timeout(Some(remaining_time))
                .map_err(ProviderEgressError::Io)?;
            writer
                .write_all(&buffer[..count])
                .map_err(ProviderEgressError::Io)?;
            total += count;
        }
    }

    /// Copy one bounded half of an in-memory/test I/O stream. Production
    /// callers must use [`Self::forward_bounded`] so socket deadlines are
    /// refreshed for every blocking operation.
    #[cfg(test)]
    fn forward_bounded_io<R: std::io::Read, W: std::io::Write>(
        &self,
        reader: &mut R,
        writer: &mut W,
        deadline: Instant,
    ) -> Result<usize, ProviderEgressError> {
        let mut buffer = [0_u8; 16 * 1024];
        let mut total = 0;
        loop {
            if Instant::now() >= deadline {
                return Err(ProviderEgressError::DeadlineExceeded);
            }
            let remaining = self.max_forward_bytes - total;
            if remaining == 0 {
                let mut extra = [0_u8; 1];
                let count = reader.read(&mut extra).map_err(ProviderEgressError::Io)?;
                return if count == 0 {
                    Ok(total)
                } else {
                    Err(ProviderEgressError::ByteLimitExceeded)
                };
            }
            let read_size = buffer.len().min(remaining);
            let count = reader
                .read(&mut buffer[..read_size])
                .map_err(ProviderEgressError::Io)?;
            if count == 0 {
                return Ok(total);
            }
            writer
                .write_all(&buffer[..count])
                .map_err(ProviderEgressError::Io)?;
            total += count;
        }
    }
}

/// Why a live provider egress request was rejected.
#[derive(Debug)]
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
    /// Relay timeout or byte bound was zero or exceeded.
    InvalidBounds,
    /// Authorization or exact destination did not match the relay policy.
    UnauthorizedTarget,
    /// Relay deadline elapsed before the operation completed.
    DeadlineExceeded,
    /// Provider TCP connection failed.
    Connect(io::Error),
    /// Relay I/O failed.
    Io(io::Error),
    /// Forwarded bytes exceeded the per-direction relay budget.
    ByteLimitExceeded,
}

impl fmt::Display for ProviderEgressError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidIdentity => "provider egress identity is invalid",
            Self::UnsupportedScheme => "provider egress requires HTTPS",
            Self::MissingHandoff => "provider egress handoff is missing",
            Self::InvalidTarget => "provider egress target is invalid",
            Self::InvalidAllowlist => "provider egress allowlist is invalid",
            Self::InvalidBounds => "provider egress relay bounds are invalid",
            Self::UnauthorizedTarget => "provider egress target is unauthorized",
            Self::DeadlineExceeded => "provider egress relay deadline exceeded",
            Self::Connect(_) => "provider egress connection failed",
            Self::Io(_) => "provider egress relay I/O failed",
            Self::ByteLimitExceeded => "provider egress relay byte limit exceeded",
        })
    }
}

impl std::error::Error for ProviderEgressError {}

impl PartialEq for ProviderEgressError {
    fn eq(&self, other: &Self) -> bool {
        use ProviderEgressError::*;
        matches!(
            (self, other),
            (InvalidIdentity, InvalidIdentity)
                | (UnsupportedScheme, UnsupportedScheme)
                | (MissingHandoff, MissingHandoff)
                | (InvalidTarget, InvalidTarget)
                | (InvalidAllowlist, InvalidAllowlist)
                | (InvalidBounds, InvalidBounds)
                | (UnauthorizedTarget, UnauthorizedTarget)
                | (DeadlineExceeded, DeadlineExceeded)
                | (Connect(_), Connect(_))
                | (Io(_), Io(_))
                | (ByteLimitExceeded, ByteLimitExceeded)
        )
    }
}

impl Eq for ProviderEgressError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Read, Write};
    use std::net::TcpListener;
    use std::thread;

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
        for address in [
            "127.0.0.1:443",
            "10.0.0.1:443",
            "[::1]:443",
            "[::ffff:127.0.0.1]:443",
            "[::ffff:10.0.0.1]:443",
        ] {
            assert_eq!(
                ProviderEgressTarget::new(address.parse().unwrap()),
                Err(ProviderEgressError::InvalidTarget)
            );
        }
    }

    #[test]
    fn relay_connects_only_authorized_exact_target_and_bounds_forwarding() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let policy =
            ProviderEgressPolicy::new("https://openrouter.ai/api/v1", "openrouter.ai").unwrap();
        let handoff = ProviderEgressHandoff::issue_bound(&policy, "g-1", "route-1", u64::MAX);
        let authorization =
            ProviderEgressAuthorization::authorize(&policy, &handoff, 0, "g-1", "route-1").unwrap();
        let target = ProviderEgressTarget::test_only(address);
        let allowlist = ProviderEgressAllowlist::new(vec![target]).unwrap();
        let relay = ProviderEgressRelay::new(
            &policy,
            allowlist,
            "g-1",
            "route-1",
            Duration::from_secs(1),
            8,
        )
        .unwrap();
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 4];
            stream.read_exact(&mut request).unwrap();
            assert_eq!(&request, b"ping");
            stream.write_all(b"pong").unwrap();
        });
        let mut stream = relay
            .connect(
                authorization,
                target,
                Instant::now() + Duration::from_secs(1),
            )
            .unwrap();
        stream.write_all(b"ping").unwrap();
        let mut response = Vec::new();
        relay
            .forward_bounded(
                &mut stream,
                &mut response,
                Instant::now() + Duration::from_secs(1),
            )
            .unwrap();
        assert_eq!(response, b"pong");
        worker.join().unwrap();
        let mut exact = Cursor::new(b"12345678".to_vec());
        assert_eq!(
            relay
                .forward_bounded_io(
                    &mut exact,
                    &mut Vec::new(),
                    Instant::now() + Duration::from_secs(1)
                )
                .unwrap(),
            8
        );
        let mut oversized = Cursor::new(b"123456789".to_vec());
        assert_eq!(
            relay.forward_bounded_io(
                &mut oversized,
                &mut Vec::new(),
                Instant::now() + Duration::from_secs(1)
            ),
            Err(ProviderEgressError::ByteLimitExceeded)
        );
    }

    #[test]
    fn relay_rejects_wrong_authorization_and_expired_deadline() {
        let address = ProviderEgressTarget::test_only("127.0.0.1:1".parse().unwrap());
        let policy =
            ProviderEgressPolicy::new("https://openrouter.ai/api/v1", "openrouter.ai").unwrap();
        let other =
            ProviderEgressPolicy::new("https://api.openai.com/v1", "api.openai.com").unwrap();
        let auth = ProviderEgressAuthorization::authorize(
            &other,
            &ProviderEgressHandoff::issue(&other),
            0,
            "generation-1",
            "route-1",
        )
        .unwrap();
        let relay = ProviderEgressRelay::new(
            &policy,
            ProviderEgressAllowlist::new(vec![address]).unwrap(),
            "generation-1",
            "route-1",
            Duration::from_secs(1),
            1024,
        )
        .unwrap();
        assert!(matches!(
            relay.connect(auth, address, Instant::now() + Duration::from_secs(1)),
            Err(ProviderEgressError::UnauthorizedTarget)
        ));
        let stale_auth = ProviderEgressAuthorization::authorize(
            &policy,
            &ProviderEgressHandoff::issue_bound(&policy, "generation-2", "route-1", u64::MAX),
            0,
            "generation-2",
            "route-1",
        )
        .unwrap();
        assert!(matches!(
            relay.connect(stale_auth, address, Instant::now() + Duration::from_secs(1)),
            Err(ProviderEgressError::UnauthorizedTarget)
        ));
        let auth = ProviderEgressAuthorization::authorize(
            &policy,
            &ProviderEgressHandoff::issue(&policy),
            0,
            "generation-1",
            "route-1",
        )
        .unwrap();
        assert!(matches!(
            relay.connect(auth, address, Instant::now()),
            Err(ProviderEgressError::DeadlineExceeded)
        ));
    }
}
