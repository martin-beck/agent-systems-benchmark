// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Runtime-owned, fail-closed inputs for live-provider acquisition.

use crate::launch_factory::{
    LaunchAuthorityError, LiveLaunchFactory, LiveProviderAttempt, RuntimeLaunchToken,
};
use crate::live_namespace::NamespaceIdentity;
use crate::live_relay::LiveProviderRelay;
use crate::provider_egress::{ProviderEgressAllowlist, ProviderEgressTarget};
use crate::sandbox::{CpuSet, LeaseClass, LeaseError, NetworkPolicy, ResourceLease};
use crate::sandbox::{SandboxBackend, SandboxLaunchInput};
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

/// Immutable provider-selection references supplied by the runtime planner.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiveProviderRuntimeSelection {
    target: ProviderEgressTarget,
    generation: String,
    route_sha256: String,
    credential_ref_sha256: String,
    network_policy: NetworkPolicy,
}

/// Runtime-attested values handed to the single composition boundary.
pub struct LiveProviderRuntimeAuthority {
    /// Runtime-issued launch token.
    pub token: RuntimeLaunchToken,
    /// Denied-network launch input bound to the attempt.
    pub input: SandboxLaunchInput,
    /// Exclusive benchmark lease.
    pub lease: ResourceLease,
    /// Pinned sandbox backend.
    pub backend: SandboxBackend,
    /// Namespace identity observed by the runtime.
    pub namespace: NamespaceIdentity,
    /// Monotonic wall-clock fence for handoff validation.
    pub now_unix_ms: u64,
    /// Runtime-bound authenticated relay.
    pub relay: LiveProviderRelay,
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
        selection: LiveProviderRuntimeSelection,
        cpus: CpuSet,
    ) -> Result<Self, LiveProviderRuntimeConfigError> {
        if selection.network_policy != NetworkPolicy::Deny {
            return Err(LiveProviderRuntimeConfigError::NetworkPolicy);
        }
        if !lease_root.is_absolute() || !lease_root.is_dir() {
            return Err(LiveProviderRuntimeConfigError::InvalidLeaseRoot);
        }
        if !allowlist.permits(selection.target.address()) {
            return Err(LiveProviderRuntimeConfigError::TargetNotAllowed);
        }
        if selection.generation.is_empty() || selection.generation.len() > 128 {
            return Err(LiveProviderRuntimeConfigError::InvalidGeneration);
        }
        if !valid_digest(&selection.route_sha256) {
            return Err(LiveProviderRuntimeConfigError::InvalidRouteDigest);
        }
        if !valid_digest(&selection.credential_ref_sha256) {
            return Err(LiveProviderRuntimeConfigError::InvalidCredentialReference);
        }
        Ok(Self {
            lease_root: lease_root.to_owned(),
            target: selection.target,
            generation: selection.generation,
            route_sha256: selection.route_sha256,
            credential_ref_sha256: selection.credential_ref_sha256,
            cpus,
        })
    }

    /// Reserve the benchmark resources for exactly one runtime attempt.
    pub fn acquire_lease(&self) -> Result<ResourceLease, LeaseError> {
        ResourceLease::acquire(&self.lease_root, LeaseClass::Benchmark, self.cpus.clone())
    }

    /// Compose one opaque attempt from authority objects produced by the
    /// runtime's attested backend/namespace/relay boundary.
    ///
    /// This is intentionally the only composition point: CLI code cannot
    /// assemble or replace any of these values.
    pub fn compose_attempt(
        &self,
        authority: LiveProviderRuntimeAuthority,
    ) -> Result<LiveProviderAttempt, LaunchAuthorityError> {
        LiveLaunchFactory::acquire(
            authority.token,
            authority.input,
            authority.lease,
            authority.backend,
            authority.namespace,
            authority.now_unix_ms,
            authority.relay,
        )
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
            selection(
                target(),
                "generation-1",
                "a".repeat(64),
                "b".repeat(64),
                NetworkPolicy::Deny,
            ),
            CpuSet::new(vec![0]).unwrap(),
        )
        .unwrap()
    }

    fn selection(
        target: ProviderEgressTarget,
        generation: &str,
        route_sha256: String,
        credential_ref_sha256: String,
        network_policy: NetworkPolicy,
    ) -> LiveProviderRuntimeSelection {
        LiveProviderRuntimeSelection {
            target,
            generation: generation.into(),
            route_sha256,
            credential_ref_sha256,
            network_policy,
        }
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
                selection(
                    other,
                    "generation-1",
                    "a".repeat(64),
                    "b".repeat(64),
                    NetworkPolicy::Deny
                ),
                CpuSet::new(vec![0]).unwrap()
            ),
            Err(LiveProviderRuntimeConfigError::TargetNotAllowed)
        );
        assert_eq!(
            LiveProviderRuntimeConfig::new(
                &root,
                &allowlist,
                selection(
                    target(),
                    "generation-1",
                    "a".repeat(64),
                    "b".repeat(64),
                    NetworkPolicy::Host
                ),
                CpuSet::new(vec![0]).unwrap()
            ),
            Err(LiveProviderRuntimeConfigError::NetworkPolicy)
        );
        assert_eq!(
            LiveProviderRuntimeConfig::new(
                &root,
                &allowlist,
                selection(
                    target(),
                    "",
                    "a".repeat(64),
                    "b".repeat(64),
                    NetworkPolicy::Deny
                ),
                CpuSet::new(vec![0]).unwrap()
            ),
            Err(LiveProviderRuntimeConfigError::InvalidGeneration)
        );
        assert_eq!(
            LiveProviderRuntimeConfig::new(
                &root,
                &allowlist,
                selection(
                    target(),
                    "generation-1",
                    "A".repeat(64),
                    "b".repeat(64),
                    NetworkPolicy::Deny
                ),
                CpuSet::new(vec![0]).unwrap()
            ),
            Err(LiveProviderRuntimeConfigError::InvalidRouteDigest)
        );
        assert_eq!(
            LiveProviderRuntimeConfig::new(
                &root,
                &allowlist,
                selection(
                    target(),
                    "generation-1",
                    "a".repeat(64),
                    "not-a-digest".into(),
                    NetworkPolicy::Deny
                ),
                CpuSet::new(vec![0]).unwrap()
            ),
            Err(LiveProviderRuntimeConfigError::InvalidCredentialReference)
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
