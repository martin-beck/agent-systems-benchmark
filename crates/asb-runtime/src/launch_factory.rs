// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Runtime-owned authority for handing a validated replay launch to a consumer.

use crate::live_namespace::{LiveProviderNamespaceHandoff, NamespaceIdentity};
use crate::sandbox::{LeaseClass, ResourceLease, SandboxBackend, SandboxError, SandboxLaunchInput};
use sha2::{Digest, Sha256};
use std::sync::Arc;

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
    operation_issued: bool,
}

/// Runtime-owned factory for validated replay launch authority.
pub struct ReplayLaunchFactory;

/// Runtime-owned source that materializes replay authority from a validated
/// launch boundary. Callers provide only already-validated runtime objects;
/// the source performs the sandbox attestation and binds the resulting token
/// to the exact cassette before returning opaque authority.
pub struct ReplayAuthoritySource;

/// Runtime-owned local replay authority factory.
///
/// The launch inputs are retained by the runtime boundary.  Consumers may
/// provide only the validated cassette identity to [`Self::acquire`]; they
/// cannot construct or replace the lease, relay, sandbox, or backend between
/// acquisition and attestation.
pub struct LocalReplayAuthorityFactory {
    input: SandboxLaunchInput,
    lease: ResourceLease,
    backend: SandboxBackend,
}

impl LocalReplayAuthorityFactory {
    /// Bind already-qualified runtime resources to one local replay factory.
    ///
    /// Resource provisioning and qualification belong to the runtime owner;
    /// this constructor is intentionally the only boundary that can retain
    /// them for subsequent cassette-scoped acquisition.
    pub fn new(input: SandboxLaunchInput, lease: ResourceLease, backend: SandboxBackend) -> Self {
        Self {
            input,
            lease,
            backend,
        }
    }

    /// Acquire one opaque authority for exactly one validated cassette.
    pub fn acquire(
        self,
        cassette_sha256: &str,
    ) -> Result<ReplayLaunchAuthority, ReplayAuthoritySourceError> {
        ReplayAuthoritySource::issue(
            self.input,
            self.lease,
            self.backend,
            cassette_sha256.to_owned(),
        )
    }
}

/// Failure while materializing runtime-owned replay authority.
#[derive(Debug)]
pub enum ReplayAuthoritySourceError {
    /// The runtime sandbox could not attest the launch boundary.
    Attestation(SandboxError),
    /// The attested values did not satisfy the authority contract.
    Authority(LaunchAuthorityError),
}

impl ReplayAuthoritySource {
    fn validate_cassette_digest(value: &str) -> Result<(), LaunchAuthorityError> {
        if valid_digest(value) {
            Ok(())
        } else {
            Err(LaunchAuthorityError::InvalidLaunchInput)
        }
    }

    /// Attest and materialize one authority for one exact replay cassette.
    pub fn issue(
        input: SandboxLaunchInput,
        lease: ResourceLease,
        backend: SandboxBackend,
        cassette_sha256: String,
    ) -> Result<ReplayLaunchAuthority, ReplayAuthoritySourceError> {
        Self::validate_cassette_digest(&cassette_sha256)
            .map_err(ReplayAuthoritySourceError::Authority)?;
        let token = backend
            .attest_replay_launch(&input, &lease, &cassette_sha256)
            .map_err(ReplayAuthoritySourceError::Attestation)?;
        ReplayLaunchFactory::issue_with_backend(token, input, lease, cassette_sha256, backend)
            .map_err(ReplayAuthoritySourceError::Authority)
    }
}

/// Runtime-owned authority for one explicitly admitted live-provider launch.
///
/// This type deliberately contains no endpoint, namespace, or credential
/// constructor. Those identities arrive only through the attested handoff
/// issued by the runtime egress and namespace boundaries.
#[derive(Debug)]
pub struct LiveLaunchAuthority {
    input: SandboxLaunchInput,
    lease: ResourceLease,
    backend: SandboxBackend,
    handoff: LiveProviderNamespaceHandoff,
    namespace: NamespaceIdentity,
}

/// Opaque, one-shot live launch context owned by the runtime.
#[derive(Debug)]
pub struct LiveLaunchContext {
    input: Option<SandboxLaunchInput>,
    lease: Option<ResourceLease>,
    backend: SandboxBackend,
    handoff: LiveProviderNamespaceHandoff,
    namespace: NamespaceIdentity,
}

/// Runtime factory for validated live-provider relay launches.
pub struct LiveLaunchFactory;

/// Runtime-owned, one-attempt live-provider capability.
///
/// This is the only value a frontend needs to pass to a live attempt.  It
/// keeps the launch context and relay lifecycle together so cancellation,
/// failed spawn, and normal teardown all revoke the capability.  Callers
/// cannot construct one from an endpoint, namespace, or credential.
pub struct LiveProviderAttempt {
    context: Option<LiveLaunchContext>,
    relay: Option<crate::live_relay::LiveProviderRelay>,
}

/// Runtime-owned source of one fresh live-provider capability per scheduler
/// attempt. The callback is invoked only by the execution boundary; callers
/// cannot construct or inspect the capability it returns.
#[derive(Clone)]
pub struct LiveProviderAttemptFactory {
    acquire:
        Arc<dyn Fn(u32, bool) -> Result<LiveProviderAttempt, LaunchAuthorityError> + Send + Sync>,
}

impl std::fmt::Debug for LiveProviderAttemptFactory {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("LiveProviderAttemptFactory(..)")
    }
}

impl LiveProviderAttemptFactory {
    /// Bind a runtime acquisition callback. The callback receives the
    /// scheduler input identity and warmup marker for this attempt.
    pub fn from_fn<F>(acquire: F) -> Self
    where
        F: Fn(u32, bool) -> Result<LiveProviderAttempt, LaunchAuthorityError>
            + Send
            + Sync
            + 'static,
    {
        Self {
            acquire: Arc::new(acquire),
        }
    }

    /// Acquire one capability for one scheduler attempt.
    pub fn acquire(
        &self,
        input_id: u32,
        warmup: bool,
    ) -> Result<LiveProviderAttempt, LaunchAuthorityError> {
        (self.acquire)(input_id, warmup)
    }
}

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

impl LiveLaunchFactory {
    /// Issue authority only after runtime attestation and handoff validation.
    pub fn issue(
        token: RuntimeLaunchToken,
        input: SandboxLaunchInput,
        lease: ResourceLease,
        backend: SandboxBackend,
        namespace: NamespaceIdentity,
        now_unix_ms: u64,
    ) -> Result<LiveLaunchAuthority, LaunchAuthorityError> {
        if token.nonce == 0 || lease.class() != LeaseClass::Benchmark {
            return Err(LaunchAuthorityError::InvalidLaunchInput);
        }
        if input.spec().network_policy() != crate::sandbox::NetworkPolicy::Deny {
            return Err(LaunchAuthorityError::InvalidLaunchInput);
        }
        let (handoff, bound_namespace) = input
            .live_provider_handoff()
            .ok_or(LaunchAuthorityError::InvalidLaunchInput)?;
        let handoff = handoff.clone();
        if bound_namespace != &namespace
            || handoff.validate(&namespace, now_unix_ms).is_err()
            || token.binding_digest != live_launch_binding_digest(&input, &lease, now_unix_ms)
        {
            return Err(LaunchAuthorityError::IdentityMismatch);
        }
        Ok(LiveLaunchAuthority {
            input,
            lease,
            backend,
            handoff,
            namespace,
        })
    }

    /// Atomically acquire the one-shot context consumed by a single attempt.
    ///
    /// The authority is consumed immediately; dropping the returned attempt
    /// revokes the namespace capability and tears down the relay.
    pub fn acquire(
        token: RuntimeLaunchToken,
        input: SandboxLaunchInput,
        lease: ResourceLease,
        backend: SandboxBackend,
        namespace: NamespaceIdentity,
        now_unix_ms: u64,
        relay: crate::live_relay::LiveProviderRelay,
    ) -> Result<LiveProviderAttempt, LaunchAuthorityError> {
        let authority = Self::issue(token, input, lease, backend, namespace, now_unix_ms)?;
        Ok(LiveProviderAttempt {
            context: Some(authority.consume()),
            relay: Some(relay),
        })
    }
}

impl LiveLaunchAuthority {
    /// Consume the authority exactly once.
    pub fn consume(self) -> LiveLaunchContext {
        LiveLaunchContext {
            input: Some(self.input),
            lease: Some(self.lease),
            backend: self.backend,
            handoff: self.handoff,
            namespace: self.namespace,
        }
    }
}

impl LiveLaunchContext {
    /// Spawn through the retained runtime backend. The CLI cannot substitute
    /// an endpoint, namespace, lease, or sandbox tool after issuance.
    pub fn spawn(&mut self) -> Result<crate::sandbox::SandboxProcess, SandboxError> {
        let input = self.input.take().ok_or(SandboxError::LiveHandoff)?;
        let lease = self.lease.take().ok_or(SandboxError::LiveHandoff)?;
        self.backend.spawn_launch(input, lease)
    }

    /// Revoke the relay capability before cancellation or teardown.
    pub fn revoke(&self) {
        self.handoff.revoke();
    }

    /// Runtime-observed namespace bound to this launch.
    pub fn namespace(&self) -> &NamespaceIdentity {
        &self.namespace
    }

    /// Adapter route identity, credential-reference digest, and expiry are
    /// exposed only as opaque metadata for evidence; no secret is retained.
    pub fn route_sha256(&self) -> &str {
        self.handoff.route_sha256()
    }
    /// Adapter executable identity digest.
    pub fn adapter_sha256(&self) -> &str {
        self.handoff.adapter_sha256()
    }
    /// Credential reference digest; no credential material is retained.
    pub fn credential_ref_sha256(&self) -> &str {
        self.handoff.credential_ref_sha256()
    }
    /// Absolute expiry fence for this launch capability.
    pub fn deadline_unix_ms(&self) -> u64 {
        self.handoff.deadline_unix_ms()
    }
}

impl LiveProviderAttempt {
    /// Start the bounded authenticated relay worker for this attempt.
    pub fn start_relay(
        &mut self,
        now_unix_ms: u64,
    ) -> Result<std::thread::JoinHandle<Result<(), crate::live_relay::LiveRelayError>>, SandboxError>
    {
        let mut relay = self.relay.take().ok_or(SandboxError::LiveHandoff)?;
        Ok(std::thread::spawn(move || relay.serve_once(now_unix_ms)))
    }

    /// Consume the runtime-issued context exactly once for spawning.
    pub fn spawn(&mut self) -> Result<crate::sandbox::SandboxProcess, SandboxError> {
        self.context
            .take()
            .ok_or(SandboxError::LiveHandoff)?
            .spawn()
    }

    /// Relay owned by this attempt, for the runtime's authenticated accept
    /// loop.  It is never writable by the CLI.
    pub fn relay(&mut self) -> Option<&mut crate::live_relay::LiveProviderRelay> {
        self.relay.as_mut()
    }

    /// Revoke before cancellation or an early spawn failure.
    pub fn revoke(&self) {
        if let Some(context) = &self.context {
            context.revoke();
        }
        if let Some(relay) = &self.relay {
            relay.revoke();
        }
    }
}

impl Drop for LiveProviderAttempt {
    fn drop(&mut self) {
        self.revoke();
    }
}

/// Digest used by the runtime attestation and factory; callers cannot choose
/// individual endpoint or credential fields independently.
pub(crate) fn live_launch_binding_digest(
    input: &SandboxLaunchInput,
    lease: &ResourceLease,
    now_unix_ms: u64,
) -> String {
    let (handoff, namespace) = input
        .live_provider_handoff()
        .expect("live binding requires an attached handoff");
    let mut hasher = Sha256::new();
    for field in [
        handoff.policy_sha256(),
        handoff.generation(),
        handoff.route_sha256(),
        handoff.adapter_sha256(),
        handoff.credential_ref_sha256(),
        handoff.relay_socket().to_string_lossy().as_ref(),
        handoff.capability_sha256(),
        namespace.as_str(),
        &now_unix_ms.to_string(),
        &format!("{:?}", lease.cpus()),
        input.spec().program(),
    ] {
        hasher.update((field.len() as u64).to_le_bytes());
        hasher.update(field.as_bytes());
    }
    format!("{:x}", hasher.finalize())
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
            operation_issued: false,
        })
    }
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

impl ReplayLaunchContext {
    /// Consume the runtime-issued context through an explicitly supplied backend.
    ///
    /// The context owns the validated launch input and benchmark lease. Passing
    /// both directly to the backend prevents a caller from replacing either
    /// value between authority consumption and child creation.
    pub fn spawn(
        self,
        backend: &SandboxBackend,
    ) -> Result<crate::sandbox::SandboxProcess, crate::sandbox::SandboxError> {
        backend.spawn_launch(self.input, self.lease)
    }

    /// Consume the runtime-issued context through the backend retained by the authority.
    pub fn spawn_owned(
        self,
    ) -> Result<crate::sandbox::SandboxProcess, crate::sandbox::SandboxError> {
        self.backend
            .ok_or(SandboxError::DelegationRejected)?
            .spawn_launch(self.input, self.lease)
    }

    /// Issue the one-shot authenticated operation used by the primary replay path.
    pub fn issue_operation(
        &mut self,
    ) -> Result<crate::ReplayOperation, crate::ReplayOperationError> {
        if self.operation_issued {
            return Err(crate::ReplayOperationError::AlreadyIssued);
        }
        self.operation_issued = true;
        crate::ReplayOperation::issue(self.generation.clone())
            .map_err(crate::ReplayOperationError::Transport)
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
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    static FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn test_backend() -> SandboxBackend {
        let pin = |path: &str| ToolPin::new(PathBuf::from(path), "test".into()).unwrap();
        SandboxBackend::new(
            pin("/bin/true"),
            pin("/bin/true"),
            pin("/bin/true"),
            pin("/bin/true"),
        )
    }

    fn live_fixture() -> (
        SandboxLaunchInput,
        ResourceLease,
        NamespaceIdentity,
        PathBuf,
    ) {
        let root = std::env::temp_dir().join(format!(
            "asb-live-factory-{}-{}",
            std::process::id(),
            FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let relay_path = root.join("relay.sock");
        let listener = std::os::unix::net::UnixListener::bind(&relay_path).unwrap();
        fs::set_permissions(&relay_path, fs::Permissions::from_mode(0o600)).unwrap();
        std::mem::forget(listener);
        let policy = crate::provider_egress::ProviderEgressPolicy::new(
            "https://openrouter.ai/api/v1",
            "openrouter.ai",
        )
        .unwrap();
        let namespace = NamespaceIdentity::new("net:[123]").unwrap();
        let handoff = crate::provider_egress::ProviderEgressHandoff::issue_bound(
            &policy,
            "generation-1",
            "a".repeat(64),
            2_000,
        );
        let live = LiveProviderNamespaceHandoff::issue(
            &policy,
            &handoff,
            namespace.clone(),
            "generation-1",
            "a".repeat(64),
            "b".repeat(64),
            "c".repeat(64),
            relay_path,
            PathBuf::from("/tmp/provider-relay.sock"),
            2_000,
            100,
        )
        .unwrap();
        let resources =
            Resources::new(64 * 1024 * 1024, 16, 100, CpuSet::new(vec![0]).unwrap()).unwrap();
        let spec = SandboxSpec::new(
            &workspace,
            PathBuf::from("."),
            "/bin/true".into(),
            vec![],
            BTreeMap::new(),
            resources,
            NetworkPolicy::Deny,
        )
        .unwrap();
        let input = SandboxLaunchInput::new(spec, ProcessLimits::default())
            .unwrap()
            .with_live_provider_handoff(live, namespace.clone(), 100)
            .unwrap();
        let leases = root.join("leases");
        fs::create_dir_all(&leases).unwrap();
        let lease = ResourceLease::acquire(
            &leases,
            LeaseClass::Benchmark,
            CpuSet::new(vec![0]).unwrap(),
        )
        .unwrap();
        (input, lease, namespace, root)
    }

    fn fixture() -> (ReplayLaunchAuthority, fs::File, std::path::PathBuf) {
        fixture_with_token(None, |input, lease, cassette_sha256| {
            RuntimeLaunchToken::test_only(launch_binding_digest(input, lease, cassette_sha256))
        })
        .unwrap()
    }

    #[test]
    fn live_factory_issues_opaque_context_from_attested_handoff() {
        let (input, lease, namespace, root) = live_fixture();
        let token = RuntimeLaunchToken::test_only(live_launch_binding_digest(&input, &lease, 100));
        let authority =
            LiveLaunchFactory::issue(token, input, lease, test_backend(), namespace, 100).unwrap();
        let context = authority.consume();
        assert_eq!(context.route_sha256(), "a".repeat(64));
        assert_eq!(context.adapter_sha256(), "b".repeat(64));
        assert_eq!(context.credential_ref_sha256(), "c".repeat(64));
        assert_eq!(context.namespace().as_str(), "net:[123]");
        assert_eq!(context.deadline_unix_ms(), 2_000);
        assert_eq!(context.route_sha256(), "a".repeat(64));
        context.revoke();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn live_context_consumes_spawn_inputs_once_and_rejects_reuse() {
        let (input, lease, namespace, root) = live_fixture();
        let token = RuntimeLaunchToken::test_only(live_launch_binding_digest(&input, &lease, 100));
        let authority =
            LiveLaunchFactory::issue(token, input, lease, test_backend(), namespace, 100).unwrap();
        let mut context = authority.consume();
        assert!(context.spawn().is_err());
        assert!(context.spawn().is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn live_factory_rejects_expired_or_copied_attestation() {
        let (input, lease, namespace, root) = live_fixture();
        let token = RuntimeLaunchToken::test_only(live_launch_binding_digest(&input, &lease, 100));
        assert!(matches!(
            LiveLaunchFactory::issue(token, input, lease, test_backend(), namespace, 2_000),
            Err(LaunchAuthorityError::IdentityMismatch)
        ));
        let _ = fs::remove_dir_all(root);
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
            vec![
                "-c".into(),
                "printf '%s\\n' \"$0\" \"$@\" > /workspace/supervisor-args".into(),
            ],
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
    fn authority_source_digest_gate_is_closed() {
        assert!(ReplayAuthoritySource::validate_cassette_digest(&"e".repeat(64)).is_ok());
        assert!(matches!(
            ReplayAuthoritySource::validate_cassette_digest("not-a-digest"),
            Err(LaunchAuthorityError::InvalidLaunchInput)
        ));
    }

    #[test]
    fn authority_consumption_preserves_attested_identities() {
        let (authority, _file, root) = fixture();
        let mut context = authority.consume_for(&"e".repeat(64)).unwrap();
        assert_eq!(context.generation(), "generation-1");
        assert_eq!(context.route_digest(), "d".repeat(64));
        assert_eq!(context.sidecar_digest(), "a".repeat(64));
        assert_eq!(context.adapter_digest(), "b".repeat(64));
        assert_eq!(context.supervisor_digest(), Some("c".repeat(64).as_str()));
        assert_eq!(context.input().spec().network_policy(), NetworkPolicy::Deny);
        assert_eq!(context.lease().class(), LeaseClass::Benchmark);
        assert!(context.issue_operation().is_ok());
        assert!(matches!(
            context.issue_operation(),
            Err(crate::ReplayOperationError::AlreadyIssued)
        ));
        drop(context);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn replay_factory_retains_runtime_backend_and_factory_debug_is_opaque() {
        let (authority, _file, root) =
            fixture_with_token(Some(test_backend()), |input, lease, cassette| {
                RuntimeLaunchToken::test_only(launch_binding_digest(input, lease, cassette))
            })
            .unwrap();
        let context = authority.consume_for(&"e".repeat(64)).unwrap();
        assert!(context.backend.is_some());
        let factory = LiveProviderAttemptFactory::from_fn(|_, _| {
            Err(LaunchAuthorityError::InvalidLaunchInput)
        });
        assert_eq!(format!("{factory:?}"), "LiveProviderAttemptFactory(..)");
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
            context.spawn_owned(),
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
            .spawn_owned()
            .unwrap_or_else(|error| panic!("delegated replay child spawn failed: {error:?}"));
        let output = child.wait().unwrap();
        assert_eq!(
            output.exit_code,
            Some(0),
            "delegated replay child failed: {}",
            String::from_utf8_lossy(&output.stderr.bytes)
        );
        let args = fs::read_to_string(root.join("workspace/supervisor-args")).unwrap();
        assert!(args.lines().any(|arg| arg == "--unshare-net"));
        assert!(args.lines().any(|arg| arg == "--relay"));
        assert!(args.lines().any(|arg| arg == "generation-1"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn consumed_context_issues_only_one_operation() {
        let (authority, _file, root) = fixture();
        let mut context = authority.consume_for(&"e".repeat(64)).unwrap();
        let _operation = context.issue_operation().unwrap();
        assert!(matches!(
            context.issue_operation(),
            Err(crate::ReplayOperationError::AlreadyIssued)
        ));
        let _ = fs::remove_dir_all(root);
    }
}
