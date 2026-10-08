// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Runtime-owned authority for handing a validated replay launch to a consumer.

use crate::ProcessLimits;
use crate::live_namespace::{LiveProviderNamespaceHandoff, NamespaceIdentity};
use crate::relay::ReplayRelay;
use crate::sandbox::{
    CpuSet, LeaseClass, NetworkPolicy, ResourceLease, Resources, SandboxBackend, SandboxError,
    SandboxLaunchInput, SandboxSpec, ToolPin,
};
use crate::supervisor::{PinnedCommand, SupervisorPlan};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::Duration;

static FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

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
    relay: Option<ReplayRelay>,
    cleanup: Option<TemporaryReplayRoot>,
}

/// Owns the private temporary tree used by the development replay fixture.
/// The guard is deliberately kept inside the opaque authority so every
/// successful and failed construction path has one cleanup owner.
#[derive(Debug)]
struct TemporaryReplayRoot(PathBuf);

impl TemporaryReplayRoot {
    fn disarm(mut self) -> PathBuf {
        std::mem::take(&mut self.0)
    }
}

impl Drop for TemporaryReplayRoot {
    fn drop(&mut self) {
        if !self.0.as_os_str().is_empty() {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
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
    #[allow(dead_code)]
    relay: Option<ReplayRelay>,
    cleanup: Option<TemporaryReplayRoot>,
}

/// Runtime-owned factory for validated replay launch authority.
pub struct ReplayLaunchFactory;

/// Runtime-owned source that materializes replay authority from a validated
/// launch boundary. Callers provide only already-validated runtime objects;
/// the source performs the sandbox attestation and binds the resulting token
/// to the exact cassette before returning opaque authority.
pub struct ReplayAuthoritySource;

/// Validated runtime bootstrap inputs.
///
/// This is the only configuration seam exposed to a frontend. It contains
/// executable pins and private roots, never a launch authority, lease, relay,
/// or sandbox input. The provisioner rechecks each executable identity before
/// allocating a relay or lease.
#[derive(Debug)]
pub struct LocalReplayBootstrapSpec {
    relay_root: PathBuf,
    lease_root: PathBuf,
    workspace: PathBuf,
    tools: [ToolPin; 4],
    supervisor: PinnedCommand,
    sidecar: PinnedCommand,
    adapter: PinnedCommand,
}

impl LocalReplayBootstrapSpec {
    /// Validate private roots and content-pinned replay commands.
    pub fn new(
        relay_root: &Path,
        lease_root: &Path,
        workspace: &Path,
        tools: [ToolPin; 4],
        supervisor: PinnedCommand,
        sidecar: PinnedCommand,
        adapter: PinnedCommand,
    ) -> Result<Self, ReplayAuthorityBootstrapError> {
        for root in [relay_root, lease_root, workspace] {
            validate_private_root(root).map_err(|_| ReplayAuthorityBootstrapError::InvalidRoot)?;
        }
        for command in [&supervisor, &sidecar, &adapter] {
            verify_pinned_command(command)
                .map_err(|_| ReplayAuthorityBootstrapError::InvalidCommand)?;
        }
        Ok(Self {
            relay_root: relay_root.to_owned(),
            lease_root: lease_root.to_owned(),
            workspace: workspace.to_owned(),
            tools,
            supervisor,
            sidecar,
            adapter,
        })
    }

    /// Move the validated bootstrap into a one-shot runtime provisioner.
    pub fn provisioner(self) -> LocalReplayProvisioner {
        LocalReplayProvisioner { spec: self }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// Failure while validating runtime-owned replay bootstrap inputs.
pub enum ReplayAuthorityBootstrapError {
    /// A root was not absolute, canonical, private, or an existing directory.
    InvalidRoot,
    /// A replay command changed or was not readable at the runtime boundary.
    InvalidCommand,
}

fn verify_pinned_command(command: &PinnedCommand) -> Result<(), std::io::Error> {
    let bytes = fs::read(command.executable())?;
    let observed = format!("{:x}", Sha256::digest(bytes));
    if observed == command.digest() {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "pinned replay command changed",
        ))
    }
}

fn validate_private_root(root: &Path) -> Result<(), std::io::Error> {
    if !root.is_absolute() || !root.is_dir() || fs::canonicalize(root).ok().as_deref() != Some(root)
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "replay root is not canonical",
        ));
    }
    let mode = fs::metadata(root)?.permissions().mode();
    if mode & 0o077 != 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "replay root is not private",
        ));
    }
    Ok(())
}

/// Opaque runtime handle used by the CLI integration; all launch resources
/// remain owned by this runtime object.
pub struct LocalReplayProvisioner {
    spec: LocalReplayBootstrapSpec,
}

/// A cassette identity that has passed the runtime's canonical digest gate.
///
/// Keeping the digest private prevents callers from manufacturing a launch
/// generation from an unchecked string after the relay or lease has been
/// allocated.  The upstream cassette selector remains responsible for
/// authenticating the content; this value is the runtime handoff for that
/// already-authenticated identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedReplayCassette {
    sha256: String,
}

impl ValidatedReplayCassette {
    /// Return the canonical lower-case content digest.
    pub fn sha256(&self) -> &str {
        &self.sha256
    }
}

impl LocalReplayProvisioner {
    /// Issue a strict-replay authority for the explicit local development
    /// fixture.  This path uses only the host's pinned isolation tools and
    /// private temporary roots; it never resolves credentials, signatures, or
    /// production trust material.  Callers must opt into this fixture at an
    /// explicitly development-only command boundary.
    pub fn development_fixture(
        cassette_sha256: &str,
    ) -> Result<ReplayLaunchAuthority, ReplayAuthoritySourceError> {
        let cassette = ReplayAuthoritySource::validate_cassette_digest(cassette_sha256)
            .map_err(ReplayAuthoritySourceError::Authority)?;
        let sequence = FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("asb-dr-{sequence}"));
        fs::create_dir(&root).map_err(|_| {
            ReplayAuthoritySourceError::Authority(LaunchAuthorityError::InvalidLaunchInput)
        })?;
        let cleanup = TemporaryReplayRoot(root.clone());
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).map_err(|_| {
            ReplayAuthoritySourceError::Authority(LaunchAuthorityError::InvalidLaunchInput)
        })?;
        let relay_root = root.join("relay");
        let lease_root = root.join("lease");
        let workspace = root.join("workspace");
        for path in [&relay_root, &lease_root, &workspace] {
            fs::create_dir(path).map_err(|_| {
                ReplayAuthoritySourceError::Authority(LaunchAuthorityError::InvalidLaunchInput)
            })?;
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|_| {
                ReplayAuthoritySourceError::Authority(LaunchAuthorityError::InvalidLaunchInput)
            })?;
        }
        // This explicit development path does not depend on host bwrap or
        // systemd attestation. Production acquisition retains its strict
        // pinned-tool validation and probe.
        let tool = || {
            ToolPin::new(PathBuf::from("/bin/true"), "development-fixture".into()).map_err(|_| {
                ReplayAuthoritySourceError::Authority(LaunchAuthorityError::InvalidLaunchInput)
            })
        };
        let bwrap = tool()?;
        let systemd_run = tool()?;
        let systemctl = tool()?;
        let taskset = tool()?;
        let command = |path: &str| {
            let bytes = fs::read(path).map_err(|_| {
                ReplayAuthoritySourceError::Authority(LaunchAuthorityError::InvalidLaunchInput)
            })?;
            PinnedCommand::new_verified(
                PathBuf::from(path),
                Vec::new(),
                &format!("{:x}", Sha256::digest(bytes)),
            )
            .map_err(|_| {
                ReplayAuthoritySourceError::Authority(LaunchAuthorityError::InvalidLaunchInput)
            })
        };
        let spec = LocalReplayBootstrapSpec::new(
            &relay_root,
            &lease_root,
            &workspace,
            [bwrap, systemd_run, systemctl, taskset],
            command("/bin/true")?,
            command("/bin/true")?,
            command("/bin/true")?,
        )
        .map_err(|_| {
            ReplayAuthoritySourceError::Authority(LaunchAuthorityError::InvalidLaunchInput)
        })?;
        let authority = spec
            .provisioner()
            .acquire_with_backend(cassette, SandboxBackend::development_fixture());
        match authority {
            Ok(mut authority) => {
                authority.cleanup = Some(cleanup);
                Ok(authority)
            }
            Err(error) => {
                drop(cleanup);
                Err(error)
            }
        }
    }

    /// Acquire one opaque authority for one exact cassette identity.
    pub fn acquire(
        self,
        cassette: ValidatedReplayCassette,
    ) -> Result<ReplayLaunchAuthority, ReplayAuthoritySourceError> {
        let backend = SandboxBackend::new(
            self.spec.tools[0].clone(),
            self.spec.tools[1].clone(),
            self.spec.tools[2].clone(),
            self.spec.tools[3].clone(),
        );
        self.acquire_with_backend(cassette, backend)
    }

    fn acquire_with_backend(
        self,
        cassette: ValidatedReplayCassette,
        backend: SandboxBackend,
    ) -> Result<ReplayLaunchAuthority, ReplayAuthoritySourceError> {
        for root in [
            &self.spec.relay_root,
            &self.spec.lease_root,
            &self.spec.workspace,
        ] {
            validate_private_root(root).map_err(|_| {
                ReplayAuthoritySourceError::Authority(LaunchAuthorityError::InvalidLaunchInput)
            })?;
        }
        for command in [
            &self.spec.supervisor,
            &self.spec.sidecar,
            &self.spec.adapter,
        ] {
            verify_pinned_command(command).map_err(|_| {
                ReplayAuthoritySourceError::Authority(LaunchAuthorityError::InvalidLaunchInput)
            })?;
        }
        let cassette_sha256 = cassette.sha256;
        let generation = format!("replay-{cassette_sha256}");
        let relay = ReplayRelay::bind(&self.spec.relay_root, &generation).map_err(|_| {
            ReplayAuthoritySourceError::Authority(LaunchAuthorityError::InvalidLaunchInput)
        })?;
        let handoff = relay
            .handoff(PathBuf::from("/tmp/asb-replay-relay.sock"))
            .map_err(|_| {
                ReplayAuthoritySourceError::Authority(LaunchAuthorityError::InvalidLaunchInput)
            })?;
        let lease = ResourceLease::acquire(
            &self.spec.lease_root,
            LeaseClass::Benchmark,
            CpuSet::new(vec![0]).map_err(|_| {
                ReplayAuthoritySourceError::Authority(LaunchAuthorityError::InvalidLease)
            })?,
        )
        .map_err(|_| ReplayAuthoritySourceError::Authority(LaunchAuthorityError::InvalidLease))?;
        let plan = SupervisorPlan::new(
            self.spec.sidecar.clone(),
            self.spec.adapter.clone(),
            handoff.socket_path().to_owned(),
            generation.clone(),
            cassette_sha256.clone(),
            Duration::from_secs(30),
        )
        .map_err(|_| {
            ReplayAuthoritySourceError::Authority(LaunchAuthorityError::InvalidLaunchInput)
        })?
        .with_supervisor(self.spec.supervisor.clone());
        let spec = SandboxSpec::new(
            &self.spec.workspace,
            PathBuf::from("."),
            self.spec.supervisor.executable().display().to_string(),
            Vec::new(),
            BTreeMap::new(),
            Resources::new(64 * 1024 * 1024, 16, 100, CpuSet::new(vec![0]).unwrap()).map_err(
                |_| ReplayAuthoritySourceError::Authority(LaunchAuthorityError::InvalidLaunchInput),
            )?,
            NetworkPolicy::Deny,
        )
        .map_err(|_| {
            ReplayAuthoritySourceError::Authority(LaunchAuthorityError::InvalidLaunchInput)
        })?
        .with_supervisor(plan);
        let input = SandboxLaunchInput::new(spec, ProcessLimits::default())
            .map_err(|_| {
                ReplayAuthoritySourceError::Authority(LaunchAuthorityError::InvalidLaunchInput)
            })?
            .with_replay_handoff(handoff)
            .map_err(|_| {
                ReplayAuthoritySourceError::Authority(LaunchAuthorityError::InvalidLaunchInput)
            })?;
        ReplayAuthoritySource::issue_with_relay(input, lease, backend, cassette_sha256, relay)
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
    /// Authenticate the shape and canonical spelling of a cassette root before
    /// any relay, lease, or sandbox effect is attempted.
    pub fn validate_cassette_digest(
        value: &str,
    ) -> Result<ValidatedReplayCassette, LaunchAuthorityError> {
        if valid_digest(value) && value.bytes().all(|byte| !byte.is_ascii_uppercase()) {
            Ok(ValidatedReplayCassette {
                sha256: value.to_owned(),
            })
        } else {
            Err(LaunchAuthorityError::InvalidLaunchInput)
        }
    }

    /// Attest and materialize one authority for one exact replay cassette.
    #[allow(dead_code)]
    pub(crate) fn issue(
        input: SandboxLaunchInput,
        lease: ResourceLease,
        backend: SandboxBackend,
        cassette_sha256: String,
    ) -> Result<ReplayLaunchAuthority, ReplayAuthoritySourceError> {
        Self::validate_cassette_digest(&cassette_sha256)
            .map(|_| ())
            .map_err(ReplayAuthoritySourceError::Authority)?;
        let token = backend
            .attest_replay_launch(&input, &lease, &cassette_sha256)
            .map_err(ReplayAuthoritySourceError::Attestation)?;
        ReplayLaunchFactory::issue_with_backend(token, input, lease, cassette_sha256, backend)
            .map_err(ReplayAuthoritySourceError::Authority)
    }

    fn issue_with_relay(
        input: SandboxLaunchInput,
        lease: ResourceLease,
        backend: SandboxBackend,
        cassette_sha256: String,
        relay: ReplayRelay,
    ) -> Result<ReplayLaunchAuthority, ReplayAuthoritySourceError> {
        Self::validate_cassette_digest(&cassette_sha256)
            .map(|_| ())
            .map_err(ReplayAuthoritySourceError::Authority)?;
        let token = backend
            .attest_replay_launch(&input, &lease, &cassette_sha256)
            .map_err(ReplayAuthoritySourceError::Attestation)?;
        let mut authority =
            ReplayLaunchFactory::issue_with_backend(token, input, lease, cassette_sha256, backend)
                .map_err(ReplayAuthoritySourceError::Authority)?;
        authority.relay = Some(relay);
        Ok(authority)
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
    fence: Arc<AtomicBool>,
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
    #[allow(dead_code)]
    pub(crate) fn issue(
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
    pub(crate) fn issue_with_backend(
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
            relay: None,
            cleanup: None,
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
    #[allow(clippy::too_many_arguments)]
    pub fn acquire(
        token: RuntimeLaunchToken,
        input: SandboxLaunchInput,
        lease: ResourceLease,
        backend: SandboxBackend,
        namespace: NamespaceIdentity,
        now_unix_ms: u64,
        relay: crate::live_relay::LiveProviderRelay,
        fence: Arc<AtomicBool>,
    ) -> Result<LiveProviderAttempt, LaunchAuthorityError> {
        if fence.load(Ordering::Acquire) {
            return Err(LaunchAuthorityError::InvalidLaunchInput);
        }
        let authority = Self::issue(token, input, lease, backend, namespace, now_unix_ms)?;
        Ok(LiveProviderAttempt {
            context: Some(authority.consume()),
            relay: Some(relay),
            fence,
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
        if self.fence.load(Ordering::Acquire) {
            return Err(SandboxError::LiveHandoff);
        }
        let mut relay = self.relay.take().ok_or(SandboxError::LiveHandoff)?;
        Ok(std::thread::spawn(move || relay.serve_once(now_unix_ms)))
    }

    /// Consume the runtime-issued context exactly once for spawning.
    pub fn spawn(&mut self) -> Result<crate::sandbox::SandboxProcess, SandboxError> {
        if self.fence.load(Ordering::Acquire) {
            return Err(SandboxError::LiveHandoff);
        }
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
        self.fence.store(true, Ordering::Release);
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
            relay: self.relay,
            cleanup: self.cleanup,
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
    #[allow(dead_code)]
    pub(crate) fn spawn(
        self,
        backend: &SandboxBackend,
    ) -> Result<crate::sandbox::SandboxProcess, crate::sandbox::SandboxError> {
        let cleanup = self.cleanup;
        let process = backend.spawn_launch(self.input, self.lease)?;
        match cleanup {
            Some(guard) => Ok(process.with_cleanup_root(guard.disarm())),
            None => Ok(process),
        }
    }

    /// Consume the runtime-issued context through the backend retained by the authority.
    pub fn spawn_owned(
        self,
    ) -> Result<crate::sandbox::SandboxProcess, crate::sandbox::SandboxError> {
        let cleanup = self.cleanup;
        let process = self
            .backend
            .ok_or(SandboxError::DelegationRejected)?
            .spawn_launch(self.input, self.lease)?;
        match cleanup {
            Some(guard) => Ok(process.with_cleanup_root(guard.disarm())),
            None => Ok(process),
        }
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
    use std::sync::{Mutex, atomic::{AtomicU64, Ordering}};
    use std::time::Duration;

    static FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(0);
    static DEVELOPMENT_FIXTURE_TEST_LOCK: Mutex<()> = Mutex::new(());

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
        let identity = ReplayAuthoritySource::validate_cassette_digest(&"e".repeat(64)).unwrap();
        assert_eq!(identity.sha256(), "e".repeat(64));
        assert!(matches!(
            ReplayAuthoritySource::validate_cassette_digest("not-a-digest"),
            Err(LaunchAuthorityError::InvalidLaunchInput)
        ));
        assert!(matches!(
            ReplayAuthoritySource::validate_cassette_digest(&"A".repeat(64)),
            Err(LaunchAuthorityError::InvalidLaunchInput)
        ));
    }

    fn development_fixture_roots() -> std::collections::BTreeSet<String> {
        fs::read_dir(std::env::temp_dir())
            .unwrap()
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let name = entry.file_name().into_string().ok()?;
                name.starts_with("asb-dr-").then_some(name)
            })
            .collect()
    }

    #[test]
    fn development_fixture_cleans_root_on_success_or_failure() {
        let _guard = DEVELOPMENT_FIXTURE_TEST_LOCK.lock().unwrap();
        let before = development_fixture_roots();
        let digest = "d".repeat(64);
        if let Ok(authority) = LocalReplayProvisioner::development_fixture(&digest) {
            let context = authority.consume_for(&digest).unwrap();
            drop(context);
        }
        assert_eq!(development_fixture_roots(), before);
    }

    #[test]
    fn development_fixture_rejects_invalid_digest_before_allocating_root() {
        let _guard = DEVELOPMENT_FIXTURE_TEST_LOCK.lock().unwrap();
        let before = development_fixture_roots();
        assert!(matches!(
            LocalReplayProvisioner::development_fixture("not-a-digest"),
            Err(ReplayAuthoritySourceError::Authority(
                LaunchAuthorityError::InvalidLaunchInput
            ))
        ));
        assert_eq!(development_fixture_roots(), before);
    }

    #[test]
    fn temporary_replay_root_drop_removes_private_tree() {
        let root = std::env::temp_dir().join(format!(
            "asb-development-replay-guard-{}",
            FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::write(root.join("marker"), b"fixture").unwrap();
        drop(TemporaryReplayRoot(root.clone()));
        assert!(!root.exists());
    }

    fn bootstrap_spec(root: &Path) -> LocalReplayBootstrapSpec {
        let pin = |path: &str| ToolPin::new(PathBuf::from(path), "fixture".into()).unwrap();
        let command = || {
            let path = PathBuf::from("/bin/true");
            let digest = format!("{:x}", Sha256::digest(fs::read(&path).unwrap()));
            PinnedCommand::new_verified(path, Vec::new(), &digest).unwrap()
        };
        LocalReplayBootstrapSpec::new(
            root,
            &root.join("leases"),
            &root.join("workspace"),
            [
                pin("/bin/true"),
                pin("/bin/true"),
                pin("/bin/true"),
                pin("/bin/true"),
            ],
            command(),
            command(),
            command(),
        )
        .unwrap()
    }

    fn command_fixture() -> PinnedCommand {
        let path = PathBuf::from("/bin/true");
        let digest = format!("{:x}", Sha256::digest(fs::read(&path).unwrap()));
        PinnedCommand::new_verified(path, Vec::new(), &digest).unwrap()
    }

    #[test]
    fn local_bootstrap_rejects_untrusted_root_before_any_effect() {
        let root = std::env::temp_dir().join(format!(
            "asb-replay-bootstrap-{}-{}",
            std::process::id(),
            FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("leases")).unwrap();
        fs::create_dir_all(root.join("workspace")).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
        let error = LocalReplayBootstrapSpec::new(
            &root,
            &root.join("leases"),
            &root.join("workspace"),
            [
                ToolPin::new(PathBuf::from("/bin/true"), "fixture".into()).unwrap(),
                ToolPin::new(PathBuf::from("/bin/true"), "fixture".into()).unwrap(),
                ToolPin::new(PathBuf::from("/bin/true"), "fixture".into()).unwrap(),
                ToolPin::new(PathBuf::from("/bin/true"), "fixture".into()).unwrap(),
            ],
            PinnedCommand::new(PathBuf::from("/bin/true"), Vec::new(), "a".repeat(64)).unwrap(),
            PinnedCommand::new(PathBuf::from("/bin/true"), Vec::new(), "b".repeat(64)).unwrap(),
            PinnedCommand::new(PathBuf::from("/bin/true"), Vec::new(), "c".repeat(64)).unwrap(),
        )
        .unwrap_err();
        assert_eq!(error, ReplayAuthorityBootstrapError::InvalidRoot);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 2);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn local_bootstrap_rejects_changed_command_digest() {
        let root = std::env::temp_dir().join(format!(
            "asb-replay-bootstrap-command-{}-{}",
            std::process::id(),
            FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("leases")).unwrap();
        fs::create_dir_all(root.join("workspace")).unwrap();
        for path in [&root, &root.join("leases"), &root.join("workspace")] {
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let pin = || ToolPin::new(PathBuf::from("/bin/true"), "fixture".into()).unwrap();
        let mut commands = [command_fixture(), command_fixture(), command_fixture()];
        commands[1] =
            PinnedCommand::new(PathBuf::from("/bin/true"), Vec::new(), "0".repeat(64)).unwrap();
        assert!(matches!(
            LocalReplayBootstrapSpec::new(
                &root,
                &root.join("leases"),
                &root.join("workspace"),
                [pin(), pin(), pin(), pin()],
                commands[0].clone(),
                commands[1].clone(),
                commands[2].clone(),
            ),
            Err(ReplayAuthorityBootstrapError::InvalidCommand)
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn private_root_validation_rejects_relative_symlink_and_public_roots() {
        let root = std::env::temp_dir().join(format!(
            "asb-private-root-{}-{}",
            std::process::id(),
            FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).unwrap();
        assert!(validate_private_root(Path::new("relative-root")).is_err());
        fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(validate_private_root(&root).is_err());
        let link = root.with_extension("link");
        let _ = fs::remove_file(&link);
        std::os::unix::fs::symlink(&root, &link).unwrap();
        assert!(validate_private_root(&link).is_err());
        let _ = fs::remove_file(link);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn replay_factory_rejects_zero_nonce_and_malformed_digest() {
        let (authority, _file, root) = fixture();
        let context = authority.consume_for(&"e".repeat(64)).unwrap();
        let input = context.input;
        let lease = context.lease;
        let token = RuntimeLaunchToken {
            nonce: 0,
            binding_digest: launch_binding_digest(&input, &lease, &"e".repeat(64)),
        };
        assert!(matches!(
            ReplayLaunchFactory::issue(token, input, lease, "e".repeat(63)),
            Err(LaunchAuthorityError::InvalidLaunchInput)
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn local_bootstrap_cleans_relay_and_lease_when_attestation_fails() {
        let root = std::env::temp_dir().join(format!(
            "asb-replay-bootstrap-fail-{}-{}",
            std::process::id(),
            FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("leases")).unwrap();
        fs::create_dir_all(root.join("workspace")).unwrap();
        for path in [&root, &root.join("leases"), &root.join("workspace")] {
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let result = bootstrap_spec(&root)
            .provisioner()
            .acquire(ReplayAuthoritySource::validate_cassette_digest(&"e".repeat(64)).unwrap());
        assert!(matches!(
            result,
            Err(ReplayAuthoritySourceError::Attestation(_))
                | Err(ReplayAuthoritySourceError::Authority(_))
        ));
        assert_eq!(fs::read_dir(root.join("leases")).unwrap().count(), 0);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 2);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn local_bootstrap_positive_acquires_and_spawns_owned_authority() {
        if std::env::var_os("ASB_REQUIRE_NATIVE_SANDBOX").is_none() {
            return;
        }
        let root = std::env::temp_dir().join(format!(
            "asb-replay-bootstrap-positive-{}-{}",
            std::process::id(),
            FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("leases")).unwrap();
        fs::create_dir_all(root.join("workspace")).unwrap();
        for path in [&root, &root.join("leases"), &root.join("workspace")] {
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let pin = |path: &str, version: &str| ToolPin::new(path.into(), version.into()).unwrap();
        let bootstrap = LocalReplayBootstrapSpec::new(
            &root,
            &root.join("leases"),
            &root.join("workspace"),
            [
                pin("/usr/bin/bwrap", "bubblewrap 0.9.0"),
                pin("/usr/bin/systemd-run", "systemd 255 (255.4-1ubuntu8.17)"),
                pin("/usr/bin/systemctl", "systemd 255 (255.4-1ubuntu8.17)"),
                pin("/usr/bin/taskset", "taskset from util-linux 2.39.3"),
            ],
            command_fixture(),
            command_fixture(),
            command_fixture(),
        )
        .unwrap();
        let authority = bootstrap
            .provisioner()
            .acquire(ReplayAuthoritySource::validate_cassette_digest(&"e".repeat(64)).unwrap())
            .unwrap();
        let context = authority.consume_for(&"e".repeat(64)).unwrap();
        let mut child = context.spawn_owned().unwrap();
        let output = child.wait().unwrap();
        assert_eq!(output.exit_code, Some(0));
        assert_eq!(output.termination, crate::Termination::Exited);
        let _ = fs::remove_dir_all(root);
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
