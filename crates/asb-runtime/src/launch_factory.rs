// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Runtime-owned authority for handing a validated replay launch to a consumer.

use crate::sandbox::{LeaseClass, ResourceLease, SandboxBackend, SandboxError, SandboxLaunchInput};
use sha2::{Digest, Sha256};

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
    cassette_sha256: String,
    backend: Option<SandboxBackend>,
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
    backend: Option<SandboxBackend>,
}

/// Runtime-owned factory for validated replay launch authority.
pub struct ReplayLaunchFactory;

/// Opaque proof that the runtime performed its launch-boundary attestation.
#[derive(Debug)]
pub struct RuntimeLaunchToken {
    pub(crate) nonce: u128,
    pub(crate) binding_digest: String,
}

impl RuntimeLaunchToken {
    #[cfg(test)]
    pub(crate) fn test_only(binding_digest: String) -> Self {
        Self {
            nonce: 1,
            binding_digest,
        }
    }
}

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
        token: RuntimeLaunchToken,
        input: SandboxLaunchInput,
        lease: ResourceLease,
        cassette_sha256: String,
    ) -> Result<ReplayLaunchAuthority, LaunchAuthorityError> {
        Self::issue_inner(token, input, lease, cassette_sha256, None)
    }

    /// Issue authority together with the runtime-owned sandbox backend.
    ///
    /// The backend is retained inside the opaque authority so a CLI caller
    /// cannot substitute tools or isolation settings between issuance and
    /// supervised child creation.
    pub fn issue_with_backend(
        token: RuntimeLaunchToken,
        input: SandboxLaunchInput,
        lease: ResourceLease,
        cassette_sha256: String,
        backend: SandboxBackend,
    ) -> Result<ReplayLaunchAuthority, LaunchAuthorityError> {
        Self::issue_inner(token, input, lease, cassette_sha256, Some(backend))
    }

    fn issue_inner(
        token: RuntimeLaunchToken,
        input: SandboxLaunchInput,
        lease: ResourceLease,
        cassette_sha256: String,
        backend: Option<SandboxBackend>,
    ) -> Result<ReplayLaunchAuthority, LaunchAuthorityError> {
        if token.nonce == 0 || !valid_digest(&cassette_sha256) {
            return Err(LaunchAuthorityError::InvalidLaunchInput);
        }
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
        if token.binding_digest != launch_binding_digest(&input, &lease, &cassette_sha256) {
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
            cassette_sha256,
            backend,
        })
    }
}

/// Hash the complete runtime launch context so an attestation cannot be
/// replayed with another command, lease, relay, supervisor, or cassette.
pub(crate) fn launch_binding_digest(
    input: &SandboxLaunchInput,
    lease: &ResourceLease,
    cassette_sha256: &str,
) -> String {
    let spec = input.spec();
    let plan = spec.supervisor();
    let mut fields = vec![
        cassette_sha256.to_owned(),
        format!("{spec:?}"),
        format!("{:?}", input.limits()),
        spec.program().to_owned(),
        spec.arguments().join("\0"),
        spec.environment()
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join("\0"),
        format!("{:?}", lease.class()),
        format!("{:?}", lease.cpus()),
    ];
    if let Some(handoff) = input.replay_handoff() {
        fields.extend([
            handoff.generation().to_owned(),
            handoff.socket_path().display().to_string(),
            handoff.child_endpoint().display().to_string(),
        ]);
    } else {
        fields.push("<no-replay-handoff>".to_owned());
    }
    if let Some(plan) = plan {
        fields.extend([
            plan.generation().to_owned(),
            plan.relay().display().to_string(),
            plan.route_digest().to_owned(),
            plan.sidecar().digest().to_owned(),
            plan.adapter().digest().to_owned(),
            plan.supervisor()
                .map(|command| command.digest().to_owned())
                .unwrap_or_else(|| "<no-supervisor>".to_owned()),
        ]);
    } else {
        fields.push("<no-supervisor-plan>".to_owned());
    }
    let mut hasher = Sha256::new();
    for field in fields {
        hasher.update((field.len() as u64).to_le_bytes());
        hasher.update(field.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

impl ReplayLaunchAuthority {
    /// Consume this authority exactly once and transfer its validated values.
    pub fn consume_for(
        self,
        cassette_sha256: &str,
    ) -> Result<ReplayLaunchContext, LaunchAuthorityError> {
        if self.cassette_sha256 != cassette_sha256 {
            return Err(LaunchAuthorityError::IdentityMismatch);
        }
        Ok(ReplayLaunchContext {
            input: self.input,
            lease: self.lease,
            generation: self.generation,
            route_digest: self.route_digest,
            sidecar_digest: self.sidecar_digest,
            adapter_digest: self.adapter_digest,
            supervisor_digest: self.supervisor_digest,
            backend: self.backend,
        })
    }
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

impl ReplayLaunchContext {
    /// Consume the runtime-issued context through the sandbox backend.
    ///
    /// The context owns the validated launch input and benchmark lease. Passing
    /// both directly to the backend prevents a caller from replacing either
    /// value between authority consumption and child creation.
    pub fn spawn(self) -> Result<crate::sandbox::SandboxProcess, crate::sandbox::SandboxError> {
        self.backend
            .ok_or(SandboxError::DelegationRejected)?
            .spawn_launch(self.input, self.lease)
    }

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
    use crate::sandbox::{CpuSet, NetworkPolicy, Resources, SandboxSpec, ToolPin};
    use crate::supervisor::{PinnedCommand, SupervisorPlan};
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    static FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn fixture() -> (ReplayLaunchAuthority, fs::File, std::path::PathBuf) {
        fixture_with_token(None, |input, lease, cassette_sha256| {
            RuntimeLaunchToken::test_only(launch_binding_digest(input, lease, cassette_sha256))
        })
        .unwrap()
    }

    fn qualified_backend() -> Option<SandboxBackend> {
        let bubblewrap =
            ToolPin::new(PathBuf::from("/usr/bin/bwrap"), "bubblewrap 0.9.0".into()).ok()?;
        let systemd_run = ToolPin::new(
            PathBuf::from("/usr/bin/systemd-run"),
            "systemd 255 (255.4-1ubuntu8.17)".into(),
        )
        .ok()?;
        let systemctl = ToolPin::new(
            PathBuf::from("/usr/bin/systemctl"),
            "systemd 255 (255.4-1ubuntu8.17)".into(),
        )
        .ok()?;
        let taskset = ToolPin::new(
            PathBuf::from("/usr/bin/taskset"),
            "taskset from util-linux 2.39.3".into(),
        )
        .ok()?;
        Some(SandboxBackend::new(
            bubblewrap,
            systemd_run,
            systemctl,
            taskset,
        ))
    }

    fn fixture_with_token(
        backend: Option<SandboxBackend>,
        make_token: impl FnOnce(&SandboxLaunchInput, &ResourceLease, &str) -> RuntimeLaunchToken,
    ) -> Result<(ReplayLaunchAuthority, fs::File, std::path::PathBuf), LaunchAuthorityError> {
        let root = std::env::temp_dir().join(format!(
            "asb-launch-factory-{}-{}",
            std::process::id(),
            FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(root.join("workspace")).unwrap();
        let relay = ReplayRelay::bind(&root, "generation-1").unwrap();
        let relay_path = relay.socket_path().to_owned();
        let sidecar =
            PinnedCommand::new(Path::new("/bin/true").to_owned(), vec![], "a".repeat(64)).unwrap();
        let adapter =
            PinnedCommand::new(Path::new("/bin/true").to_owned(), vec![], "b".repeat(64)).unwrap();
        // The supervisor receives its full argument contract below; `/bin/true`
        // rejects those arguments and exits 1, which masquerades as a sandbox
        // failure.  Use a deterministic argument-tolerant fixture instead.
        let supervisor = PinnedCommand::new(
            Path::new("/bin/sh").to_owned(),
            vec!["-c".into(), "exit 0".into()],
            "c".repeat(64),
        )
        .unwrap();
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
            // Namespace creation itself needs a realistic cgroup headroom;
            // one megabyte makes bwrap fail with EAGAIN before the child can
            // start, which obscures the delegated-capability result.
            // taskset, bwrap, the supervisor and its descendants all need a
            // process slot; TasksMax=1 makes namespace setup fail with EAGAIN.
            Resources::new(64 * 1024 * 1024, 16, 100, CpuSet::new(vec![0]).unwrap()).unwrap(),
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
        let cassette_sha256 = "e".repeat(64);
        let token = make_token(&input, &lease, &cassette_sha256);
        if backend.is_some() {
            // Keep the authenticated relay socket alive through the delegated
            // child launch; the test removes its private root after reaping.
            std::mem::forget(relay);
        }
        let authority = match backend {
            Some(backend) => ReplayLaunchFactory::issue_with_backend(
                token,
                input,
                lease,
                cassette_sha256,
                backend,
            )?,
            None => ReplayLaunchFactory::issue(token, input, lease, cassette_sha256)?,
        };
        Ok((authority, fs::File::open("/dev/null").unwrap(), root))
    }

    #[test]
    fn issuance_rejects_token_bound_to_another_launch_context() {
        let result = fixture_with_token(None, |_, _, _| {
            RuntimeLaunchToken::test_only("0".repeat(64))
        });
        assert!(matches!(
            result,
            Err(LaunchAuthorityError::IdentityMismatch)
        ));
    }

    #[test]
    fn authority_consumption_preserves_attested_identities() {
        let (authority, _file, root) = fixture();
        let context = authority.consume_for(&"e".repeat(64)).unwrap();
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
            ReplayLaunchFactory::issue(
                RuntimeLaunchToken::test_only("0".repeat(64)),
                input,
                lease,
                "e".repeat(64)
            ),
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
            ReplayLaunchFactory::issue(
                RuntimeLaunchToken::test_only("0".repeat(64)),
                input,
                lease,
                "e".repeat(64)
            ),
            Err(LaunchAuthorityError::InvalidLease)
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn consumption_rejects_wrong_cassette_identity() {
        let (authority, _file, root) = fixture();
        assert!(matches!(
            authority.consume_for(&"f".repeat(64)),
            Err(LaunchAuthorityError::IdentityMismatch)
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn context_without_runtime_backend_cannot_spawn() {
        let (authority, _file, root) = fixture();
        let context = authority.consume_for(&"e".repeat(64)).unwrap();
        assert!(matches!(
            context.spawn(),
            Err(SandboxError::DelegationRejected)
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    #[ignore = "requires delegated bwrap namespace capability"]
    fn qualified_runtime_backend_executes_and_reaps_child() {
        let Some(backend) = qualified_backend() else {
            return;
        };
        let (authority, _file, root) =
            fixture_with_token(Some(backend), |input, lease, cassette| {
                RuntimeLaunchToken::test_only(launch_binding_digest(input, lease, cassette))
            })
            .unwrap();
        let mut child = authority
            .consume_for(&"e".repeat(64))
            .unwrap()
            .spawn()
            .unwrap_or_else(|error| panic!("delegated replay child spawn failed: {error:?}"));
        let output = child.wait().unwrap();
        assert_eq!(
            output.exit_code,
            Some(0),
            "delegated replay child failed: {}",
            String::from_utf8_lossy(&output.stderr.bytes)
        );
        let _ = fs::remove_dir_all(root);
    }
}
