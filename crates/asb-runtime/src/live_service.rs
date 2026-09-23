// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Runtime-owned, fail-closed inputs for live-provider acquisition.

use crate::provider_egress::{ProviderEgressAllowlist, ProviderEgressTarget};
use crate::sandbox::{CpuSet, LeaseClass, LeaseError, NetworkPolicy, ResourceLease};
use std::path::{Path, PathBuf};

/// Validated references accepted by the production live acquisition service.
///
/// This contract contains only public identities and paths owned by the
/// runtime. It never accepts endpoint strings, namespace identities, tokens or
/// credential bytes from the CLI.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiveProviderRuntimeConfig {
    lease_root: PathBuf,
    target: ProviderEgressTarget,
    generation: String,
    route_sha256: String,
    credential_ref_sha256: String,
    cpus: CpuSet,
}

/// Fail-closed validation failures before authority acquisition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LiveProviderRuntimeConfigError {
    /// The lease root is not an absolute existing directory.
    InvalidLeaseRoot,
    /// The selected target is not in the enrolled allowlist.
    TargetNotAllowed,
    /// The generation identity is empty or exceeds its bound.
    InvalidGeneration,
    /// The route identity is not a lowercase SHA-256 digest.
    InvalidRouteDigest,
    /// The credential reference is not a lowercase SHA-256 digest.
    InvalidCredentialReference,
    /// The requested policy is not the mandatory network-denied mode.
    NetworkPolicy,
}

impl LiveProviderRuntimeConfig {
    /// Validate runtime-owned selection references and reserve no authority.
    pub fn new(
        lease_root: &Path,
        allowlist: &ProviderEgressAllowlist,
        target: ProviderEgressTarget,
        generation: impl Into<String>,
        route_sha256: impl Into<String>,
        credential_ref_sha256: impl Into<String>,
        network_policy: NetworkPolicy,
        cpus: CpuSet,
    ) -> Result<Self, LiveProviderRuntimeConfigError> {
        if network_policy != NetworkPolicy::Deny {
            return Err(LiveProviderRuntimeConfigError::NetworkPolicy);
        }
        if !lease_root.is_absolute() || !lease_root.is_dir() {
            return Err(LiveProviderRuntimeConfigError::InvalidLeaseRoot);
        }
        if !allowlist.permits(target.address()) {
            return Err(LiveProviderRuntimeConfigError::TargetNotAllowed);
        }
        let generation = generation.into();
        if generation.is_empty() || generation.len() > 128 {
            return Err(LiveProviderRuntimeConfigError::InvalidGeneration);
        }
        let route_sha256 = route_sha256.into();
        if !valid_digest(&route_sha256) {
            return Err(LiveProviderRuntimeConfigError::InvalidRouteDigest);
        }
        let credential_ref_sha256 = credential_ref_sha256.into();
        if !valid_digest(&credential_ref_sha256) {
            return Err(LiveProviderRuntimeConfigError::InvalidCredentialReference);
        }
        Ok(Self {
            lease_root: lease_root.to_owned(),
            target,
            generation,
            route_sha256,
            credential_ref_sha256,
            cpus,
        })
    }

    /// Reserve the benchmark resources for exactly one runtime attempt.
    pub fn acquire_lease(&self) -> Result<ResourceLease, LeaseError> {
        ResourceLease::acquire(&self.lease_root, LeaseClass::Benchmark, self.cpus.clone())
    }

    /// Return the runtime-owned lease root.
    pub fn lease_root(&self) -> &Path {
        &self.lease_root
    }
    /// Return the single selected concrete target.
    pub fn target(&self) -> ProviderEgressTarget {
        self.target
    }
    /// Return the bounded generation identity.
    pub fn generation(&self) -> &str {
        &self.generation
    }
    /// Return the route identity digest.
    pub fn route_sha256(&self) -> &str {
        &self.route_sha256
    }
    /// Return the opaque credential reference digest.
    pub fn credential_ref_sha256(&self) -> &str {
        &self.credential_ref_sha256
    }
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn target() -> ProviderEgressTarget {
        ProviderEgressTarget::test_only("198.51.100.10:443".parse().unwrap())
    }

    fn root() -> PathBuf {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "asb-live-service-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::create_dir_all(&root);
        root
    }

    fn config() -> LiveProviderRuntimeConfig {
        let allowlist = ProviderEgressAllowlist::new(vec![target()]).unwrap();
        LiveProviderRuntimeConfig::new(
            &root(),
            &allowlist,
            target(),
            "generation-1",
            "a".repeat(64),
            "b".repeat(64),
            NetworkPolicy::Deny,
            CpuSet::new(vec![0]).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn validates_public_references_and_reserves_one_benchmark_lease() {
        let config = config();
        assert_eq!(
            config.target().address(),
            "198.51.100.10:443".parse::<SocketAddr>().unwrap()
        );
        let lease = config.acquire_lease().unwrap();
        assert_eq!(lease.class(), LeaseClass::Benchmark);
        assert!(matches!(
            config.acquire_lease(),
            Err(LeaseError::Conflict(0))
        ));
        drop(lease);
        let _ = std::fs::remove_dir_all(config.lease_root());
    }

    #[test]
    fn rejects_unlisted_target_network_policy_and_bad_references() {
        let root = root();
        let allowlist = ProviderEgressAllowlist::new(vec![target()]).unwrap();
        let other = ProviderEgressTarget::test_only("198.51.100.11:443".parse().unwrap());
        assert_eq!(
            LiveProviderRuntimeConfig::new(
                &root,
                &allowlist,
                other,
                "generation-1",
                "a".repeat(64),
                "b".repeat(64),
                NetworkPolicy::Deny,
                CpuSet::new(vec![0]).unwrap()
            ),
            Err(LiveProviderRuntimeConfigError::TargetNotAllowed)
        );
        assert_eq!(
            LiveProviderRuntimeConfig::new(
                &root,
                &allowlist,
                target(),
                "generation-1",
                "a".repeat(64),
                "b".repeat(64),
                NetworkPolicy::Host,
                CpuSet::new(vec![0]).unwrap()
            ),
            Err(LiveProviderRuntimeConfigError::NetworkPolicy)
        );
        assert_eq!(
            LiveProviderRuntimeConfig::new(
                &root,
                &allowlist,
                target(),
                "",
                "a".repeat(64),
                "b".repeat(64),
                NetworkPolicy::Deny,
                CpuSet::new(vec![0]).unwrap()
            ),
            Err(LiveProviderRuntimeConfigError::InvalidGeneration)
        );
        assert_eq!(
            LiveProviderRuntimeConfig::new(
                &root,
                &allowlist,
                target(),
                "generation-1",
                "A".repeat(64),
                "b".repeat(64),
                NetworkPolicy::Deny,
                CpuSet::new(vec![0]).unwrap()
            ),
            Err(LiveProviderRuntimeConfigError::InvalidRouteDigest)
        );
        assert_eq!(
            LiveProviderRuntimeConfig::new(
                &root,
                &allowlist,
                target(),
                "generation-1",
                "a".repeat(64),
                "not-a-digest",
                NetworkPolicy::Deny,
                CpuSet::new(vec![0]).unwrap()
            ),
            Err(LiveProviderRuntimeConfigError::InvalidCredentialReference)
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
