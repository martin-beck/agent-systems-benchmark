// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Runtime-owned authority for handing a validated replay launch to a consumer.

use crate::sandbox::{LeaseClass, ResourceLease, SandboxLaunchInput};

/// A launch authority that can only be issued by the runtime factory.
///
/// The fields are deliberately private. Consumers receive the authority from a
/// runtime-owned boundary and may consume it once, but cannot assemble an
/// equivalent value from paths, digests, or readiness flags.
#[derive(Debug)]
pub struct ReplayLaunchAuthority {
    input: SandboxLaunchInput,
    lease: ResourceLease,
    generation: String,
    route_digest: String,
    sidecar_digest: String,
    adapter_digest: String,
    supervisor_digest: Option<String>,
}

/// Owned launch values transferred after one-shot authority consumption.
#[derive(Debug)]
pub struct ReplayLaunchContext {
    input: SandboxLaunchInput,
    lease: ResourceLease,
    generation: String,
    route_digest: String,
    sidecar_digest: String,
    adapter_digest: String,
    supervisor_digest: Option<String>,
}

/// Runtime-owned factory for validated replay launch authority.
pub struct ReplayLaunchFactory;

/// Authority construction failures; no process is started on any failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LaunchAuthorityError {
    /// The launch is not network-denied or has no authenticated relay handoff.
    InvalidLaunchInput,
    /// The lease is not an exclusive benchmark reservation.
    InvalidLease,
    /// The relay and supervisor identities disagree.
    IdentityMismatch,
}

impl ReplayLaunchFactory {
    /// Issue opaque authority from already validated runtime launch values.
    pub fn issue(
        input: SandboxLaunchInput,
        lease: ResourceLease,
    ) -> Result<ReplayLaunchAuthority, LaunchAuthorityError> {
        if lease.class() != LeaseClass::Benchmark {
            return Err(LaunchAuthorityError::InvalidLease);
        }
        let handoff = input
            .replay_handoff()
            .ok_or(LaunchAuthorityError::InvalidLaunchInput)?;
        let plan = input
            .spec()
            .supervisor()
            .ok_or(LaunchAuthorityError::InvalidLaunchInput)?;
        if handoff.generation() != plan.generation()
            || handoff.socket_path() != plan.relay()
            || input.spec().network_policy() != crate::sandbox::NetworkPolicy::Deny
        {
            return Err(LaunchAuthorityError::IdentityMismatch);
        }
        let generation = plan.generation().to_owned();
        let route_digest = plan.route_digest().to_owned();
        let sidecar_digest = plan.sidecar().digest().to_owned();
        let adapter_digest = plan.adapter().digest().to_owned();
        let supervisor_digest = plan.supervisor().map(|command| command.digest().to_owned());
        Ok(ReplayLaunchAuthority {
            input,
            lease,
            generation,
            route_digest,
            sidecar_digest,
            adapter_digest,
            supervisor_digest,
        })
    }
}

impl ReplayLaunchAuthority {
    /// Consume this authority exactly once and transfer its validated values.
    pub fn consume(self) -> ReplayLaunchContext {
        ReplayLaunchContext {
            input: self.input,
            lease: self.lease,
            generation: self.generation,
            route_digest: self.route_digest,
            sidecar_digest: self.sidecar_digest,
            adapter_digest: self.adapter_digest,
            supervisor_digest: self.supervisor_digest,
        }
    }
}

impl ReplayLaunchContext {
    /// Validated launch input for the runtime backend.
    pub fn input(&self) -> &SandboxLaunchInput {
        &self.input
    }
    /// Exclusive benchmark lease held for this launch.
    pub fn lease(&self) -> &ResourceLease {
        &self.lease
    }
    /// Authenticated launch generation.
    pub fn generation(&self) -> &str {
        &self.generation
    }
    /// Authenticated cassette route digest.
    pub fn route_digest(&self) -> &str {
        &self.route_digest
    }
    /// Pinned sidecar command digest.
    pub fn sidecar_digest(&self) -> &str {
        &self.sidecar_digest
    }
    /// Pinned adapter command digest.
    pub fn adapter_digest(&self) -> &str {
        &self.adapter_digest
    }
    /// Pinned supervisor command digest, if a supervisor is configured.
    pub fn supervisor_digest(&self) -> Option<&str> {
        self.supervisor_digest.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProcessLimits;
    use crate::relay::ReplayRelay;
    use crate::sandbox::{CpuSet, NetworkPolicy, Resources, SandboxSpec};
    use crate::supervisor::{PinnedCommand, SupervisorPlan};
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    fn fixture() -> (ReplayLaunchAuthority, fs::File, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!("asb-launch-factory-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(root.join("workspace")).unwrap();
        let relay = ReplayRelay::bind(&root, "generation-1").unwrap();
        let relay_path = relay.socket_path().to_owned();
        let sidecar =
            PinnedCommand::new(Path::new("/bin/true").to_owned(), vec![], "a".repeat(64)).unwrap();
        let adapter =
            PinnedCommand::new(Path::new("/bin/true").to_owned(), vec![], "b".repeat(64)).unwrap();
        let supervisor =
            PinnedCommand::new(Path::new("/bin/true").to_owned(), vec![], "c".repeat(64)).unwrap();
        let plan = SupervisorPlan::new(
            sidecar,
            adapter,
            relay_path.clone(),
            "generation-1".into(),
            "d".repeat(64),
            Duration::from_secs(1),
        )
        .unwrap()
        .with_supervisor(supervisor);
        let spec = SandboxSpec::new(
            &root.join("workspace"),
            PathBuf::from("."),
            "/bin/true".into(),
            vec![],
            BTreeMap::new(),
            Resources::new(1024 * 1024, 1, 100, CpuSet::new(vec![0]).unwrap()).unwrap(),
            NetworkPolicy::Deny,
        )
        .unwrap()
        .with_supervisor(plan);
        let input = SandboxLaunchInput::new(spec, ProcessLimits::default())
            .unwrap()
            .with_replay_handoff(relay.handoff("/tmp/asb-replay-relay.sock").unwrap())
            .unwrap();
        let lease_root = root.join("leases");
        fs::create_dir_all(&lease_root).unwrap();
        let lease = ResourceLease::acquire(
            &lease_root,
            LeaseClass::Benchmark,
            CpuSet::new(vec![0]).unwrap(),
        )
        .unwrap();
        let authority = ReplayLaunchFactory::issue(input, lease).unwrap();
        (authority, fs::File::open("/dev/null").unwrap(), root)
    }

    #[test]
    fn authority_consumption_preserves_attested_identities() {
        let (authority, _file, root) = fixture();
        let context = authority.consume();
        assert_eq!(context.generation(), "generation-1");
        assert_eq!(context.route_digest(), "d".repeat(64));
        assert_eq!(context.sidecar_digest(), "a".repeat(64));
        assert_eq!(context.adapter_digest(), "b".repeat(64));
        assert_eq!(context.supervisor_digest(), Some("c".repeat(64).as_str()));
        drop(context);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn issuance_rejects_missing_runtime_handoff() {
        let root = std::env::temp_dir().join(format!("asb-launch-missing-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("workspace")).unwrap();
        let spec = SandboxSpec::new(
            &root.join("workspace"),
            PathBuf::from("."),
            "/bin/true".into(),
            vec![],
            BTreeMap::new(),
            Resources::new(1024 * 1024, 1, 100, CpuSet::new(vec![0]).unwrap()).unwrap(),
            NetworkPolicy::Deny,
        )
        .unwrap();
        let input = SandboxLaunchInput::new(spec, ProcessLimits::default()).unwrap();
        let leases = root.join("leases");
        fs::create_dir_all(&leases).unwrap();
        let lease = ResourceLease::acquire(
            &leases,
            LeaseClass::Benchmark,
            CpuSet::new(vec![0]).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            ReplayLaunchFactory::issue(input, lease),
            Err(LaunchAuthorityError::InvalidLaunchInput)
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn issuance_rejects_non_benchmark_lease_before_authority() {
        let root = std::env::temp_dir().join(format!("asb-launch-ci-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("workspace")).unwrap();
        let spec = SandboxSpec::new(
            &root.join("workspace"),
            PathBuf::from("."),
            "/bin/true".into(),
            vec![],
            BTreeMap::new(),
            Resources::new(1024 * 1024, 1, 100, CpuSet::new(vec![0]).unwrap()).unwrap(),
            NetworkPolicy::Deny,
        )
        .unwrap();
        let input = SandboxLaunchInput::new(spec, ProcessLimits::default()).unwrap();
        let leases = root.join("leases");
        fs::create_dir_all(&leases).unwrap();
        let lease =
            ResourceLease::acquire(&leases, LeaseClass::Ci, CpuSet::new(vec![0]).unwrap()).unwrap();
        assert!(matches!(
            ReplayLaunchFactory::issue(input, lease),
            Err(LaunchAuthorityError::InvalidLease)
        ));
        let _ = fs::remove_dir_all(root);
    }
}
