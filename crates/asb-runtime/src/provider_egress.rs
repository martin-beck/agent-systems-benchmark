// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Fail-closed destination policy for a future provider-egress relay.
//!
//! This module deliberately does not open sockets, resolve names, or alter a
//! sandbox network namespace.  It is the runtime-owned, immutable policy
//! value a relay may consume once an approved relay implementation exists.
//! Keeping policy separate from transport prevents a caller from turning the
//! existing replay relay into an ambient live-network escape hatch.

use std::net::{IpAddr, SocketAddr};

/// A concrete provider destination.  Names are intentionally not accepted:
/// DNS resolution belongs to the trusted relay, where rebinding can be
/// checked against this exact address before connecting.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ProviderEgressTarget(SocketAddr);

impl ProviderEgressTarget {
    /// Validate a public, concrete TCP destination.
    pub fn new(address: SocketAddr) -> Result<Self, EgressPolicyError> {
        if address.port() == 0 || !is_public_address(address.ip()) {
            return Err(EgressPolicyError::InvalidTarget);
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

    /// Construct a non-empty sorted, duplicate-free allowlist.
    pub fn new(mut targets: Vec<ProviderEgressTarget>) -> Result<Self, EgressPolicyError> {
        targets.sort_unstable();
        if targets.is_empty()
            || targets.len() > Self::MAX_TARGETS
            || targets.windows(2).any(|pair| pair[0] == pair[1])
        {
            return Err(EgressPolicyError::InvalidAllowlist);
        }
        Ok(Self { targets })
    }

    /// Exact target membership check.  No DNS, subnet, or wildcard matching
    /// is performed.
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

/// Policy validation failure; callers must reject the launch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EgressPolicyError {
    /// Target is not a public, non-zero-port concrete TCP endpoint.
    InvalidTarget,
    /// Allowlist is empty, oversized, or contains duplicates.
    InvalidAllowlist,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn target(value: &str) -> ProviderEgressTarget {
        ProviderEgressTarget::new(value.parse().unwrap()).unwrap()
    }

    #[test]
    fn rejects_non_public_and_zero_port_destinations() {
        for value in [
            "127.0.0.1:443",
            "10.0.0.1:443",
            "169.254.1.1:443",
            "0.0.0.0:443",
            "203.0.113.10:0",
            "[::1]:443",
            "[fd00::1]:443",
        ] {
            assert_eq!(
                ProviderEgressTarget::new(value.parse().unwrap()),
                Err(EgressPolicyError::InvalidTarget),
                "{value}"
            );
        }
    }

    #[test]
    fn allowlist_is_exact_sorted_and_bounded() {
        let first = target("198.51.100.10:443");
        let second = target("198.51.100.11:443");
        let policy = ProviderEgressAllowlist::new(vec![second, first]).unwrap();
        assert_eq!(policy.targets(), &[first, second]);
        assert!(policy.permits(first.address()));
        assert!(!policy.permits("198.51.100.10:8443".parse().unwrap()));
        assert_eq!(
            ProviderEgressAllowlist::new(vec![first, first]),
            Err(EgressPolicyError::InvalidAllowlist)
        );
    }

    #[test]
    fn empty_and_oversized_policies_fail_closed() {
        assert_eq!(
            ProviderEgressAllowlist::new(Vec::new()),
            Err(EgressPolicyError::InvalidAllowlist)
        );
        let targets = (1..=33)
            .map(|port| target(&format!("198.51.100.10:{port}")))
            .collect();
        assert_eq!(
            ProviderEgressAllowlist::new(targets),
            Err(EgressPolicyError::InvalidAllowlist)
        );
    }
}
