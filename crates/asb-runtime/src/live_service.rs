// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Runtime-owned, fail-closed inputs for live-provider acquisition.

use crate::ProcessLimits;
use crate::credential_injection::{CredentialInjection, CredentialInjectionError};
use crate::launch_factory::{
    LaunchAuthorityError, LiveLaunchFactory, LiveProviderAttempt, LiveProviderAttemptFactory,
};
use crate::live_namespace::NamespaceIdentity;
use crate::live_relay::LiveProviderRelay;
use crate::provider_egress::{
    ProviderEgressAllowlist, ProviderEgressHandoff, ProviderEgressPolicy, ProviderEgressTarget,
};
use crate::sandbox::{
    CpuSet, LeaseClass, LeaseError, NetworkPolicy, ResourceLease, SandboxBackend,
    SandboxLaunchInput, ToolPin,
};
use asb_control::{
    ControlCall, ControlClient, ControlResult, ControlSuccess, IssuedCertificateChainV1,
    RuntimeEnrollmentReceiptV1, RuntimeReceiptRequestV1, RuntimeReceiptResponseV1,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};
use std::time::{SystemTime, UNIX_EPOCH};

/// Runtime-owned store for one authenticated certificate chain.
///
/// The chain type is opaque and can only be produced by the validated
/// certificate-authority APIs in `asb-control`; this store never accepts
/// certificate bytes, identities, or trust anchors from a CLI caller.
#[derive(Clone, Debug, Default)]
pub struct RuntimeCertificateChainStore {
    chain: Arc<Mutex<Option<IssuedCertificateChainV1>>>,
}

/// Failure while installing or retrieving the runtime-owned chain.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeCertificateChainStoreError {
    /// No chain has been enrolled in this runtime instance.
    Unavailable,
    /// A lower or equal generation cannot replace the active chain.
    StaleGeneration,
    /// The store lock was poisoned after an unexpected runtime failure.
    StateUnavailable,
}

/// Fail-closed errors from the runtime-owned control receipt adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LiveProviderControlAdapterError {
    /// The authenticated control transport rejected or could not complete the call.
    Transport,
    /// The control response did not contain the requested typed receipt.
    InvalidResponse,
    /// No authenticated chain is enrolled in the runtime store.
    ChainUnavailable,
    /// The receipt failed runtime bridge validation.
    AttestationMismatch,
}

impl RuntimeCertificateChainStore {
    /// Create an empty store. Empty stores fail closed on acquisition.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Install a validated opaque chain from the authenticated control/runtime
    /// boundary. Raw certificate material never enters this API.
    pub fn install(
        &self,
        chain: IssuedCertificateChainV1,
    ) -> Result<(), RuntimeCertificateChainStoreError> {
        let mut current = self
            .chain
            .lock()
            .map_err(|_| RuntimeCertificateChainStoreError::StateUnavailable)?;
        if current
            .as_ref()
            .is_some_and(|existing| existing.identity().generation >= chain.identity().generation)
        {
            return Err(RuntimeCertificateChainStoreError::StaleGeneration);
        }
        *current = Some(chain);
        Ok(())
    }

    /// Return the currently enrolled opaque chain for runtime-owned bridge
    /// validation. No certificate bytes or private authority escape.
    pub fn chain(&self) -> Result<IssuedCertificateChainV1, RuntimeCertificateChainStoreError> {
        self.chain
            .lock()
            .map_err(|_| RuntimeCertificateChainStoreError::StateUnavailable)?
            .clone()
            .ok_or(RuntimeCertificateChainStoreError::Unavailable)
    }
}

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

/// Failure while an adapter resolves the final opaque credential capability.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LiveProviderResolveError {
    /// The selected credential reference is unavailable or stale.
    CredentialUnavailable,
    /// The adapter could not create a safe final-boundary capability.
    CredentialInjection(CredentialInjectionError),
}

/// Cross-crate resolver seam. It accepts only public selection metadata and
/// returns an opaque final-boundary capability; it cannot provide authority.
pub trait LiveProviderResolver: Send {
    /// Resolve one credential capability for one runtime-selected attempt.
    fn resolve_credential(
        &mut self,
        selection: &LiveProviderRuntimeSelection,
    ) -> Result<Box<dyn CredentialInjection>, LiveProviderResolveError>;
}

/// Runtime entrypoint for resolver composition before authority acquisition.
pub struct LiveProviderRuntimeService;

static LOCAL_PROVIDER_GENERATION: AtomicU64 = AtomicU64::new(0);
static LOCAL_PROVIDER_ACTIVE_GENERATION: AtomicU64 = AtomicU64::new(0);
const LOCAL_PROVIDER_CREDENTIAL_SHA256: &str =
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const LOCAL_PROVIDER_TOOL_SHA256: &str =
    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const LOCAL_PROVIDER_MODEL: &str = "local-deterministic-mock-v1";
const LOCAL_PROVIDER_MAX_REQUEST_BYTES: usize = 16 * 1024 * 1024;

/// Runtime-owned deterministic loopback authority for offline development and
/// CI. It contains only private in-memory capability markers and ephemeral
/// roots; no caller-supplied endpoint, credential, policy, or tool is accepted.
pub struct LocalProviderAuthority {
    generation: u64,
    lease_root: PathBuf,
    relay_root: PathBuf,
    credential: LocalProviderCredentialCapability,
    teardown: Arc<AtomicU64>,
}

impl std::fmt::Debug for LocalProviderAuthority {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("LocalProviderAuthority(..)")
    }
}

/// Opaque in-memory credential capability for the deterministic local mock.
#[derive(Debug)]
pub struct LocalProviderCredentialCapability {
    reference_sha256: String,
}

/// Public, content-minimized result from the deterministic local provider.
///
/// This is deliberately not a [`LiveProviderAttempt`]. The local fixture is a
/// credential-free protocol double used to qualify request binding and
/// teardown; it cannot mint production launch authority or authorize egress.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalProviderMockResponse {
    attempt_id: u32,
    response_sha256: String,
}

impl LocalProviderMockResponse {
    /// Scheduler identity assigned to the mock request.
    #[must_use]
    pub const fn attempt_id(&self) -> u32 {
        self.attempt_id
    }

    /// Digest of the deterministic response; no response body is retained.
    #[must_use]
    pub fn response_sha256(&self) -> &str {
        &self.response_sha256
    }
}

/// Fail-closed local mock request validation failures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalProviderMockError {
    /// The runtime-owned authority has been revoked or superseded.
    Inactive,
    /// Zero is not a valid scheduler attempt identity.
    InvalidAttempt,
    /// The request used a different pinned local model.
    ModelMismatch,
    /// The credential reference did not match the enrolled opaque marker.
    CredentialMismatch,
    /// The bounded request body exceeded the local fixture budget.
    RequestTooLarge,
    /// The scheduler cancelled this mock attempt before dispatch.
    Cancelled,
}

/// Runtime-owned local mock backend. It is deliberately separate from
/// [`LiveProviderAttempt`] so a deterministic fixture cannot be mistaken for
/// production provider authority.
pub struct LocalProviderMockBackend {
    authority: LocalProviderAuthority,
}

impl std::fmt::Debug for LocalProviderMockBackend {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("LocalProviderMockBackend(..)")
    }
}

impl LocalProviderMockBackend {
    /// Provision one ephemeral runtime-owned local mock backend.
    pub fn provision() -> Result<Self, LocalProviderAuthorityError> {
        Ok(Self {
            authority: LocalProviderAuthorityProvisioner::provision()?,
        })
    }

    /// Admit one scheduler attempt under the backend's generation fence.
    pub fn issue_attempt(
        &self,
        attempt_id: u32,
    ) -> Result<LocalProviderMockAttempt<'_>, LocalProviderMockError> {
        if attempt_id == 0 || !self.authority.is_active() {
            return Err(if attempt_id == 0 {
                LocalProviderMockError::InvalidAttempt
            } else {
                LocalProviderMockError::Inactive
            });
        }
        Ok(LocalProviderMockAttempt {
            authority: &self.authority,
            attempt_id,
            cancelled: false,
        })
    }

    /// Revoke all outstanding attempts and tear down the mock authority.
    pub fn revoke(&self) {
        self.authority.revoke();
    }
}

/// One bounded local mock scheduler attempt. This type has no conversion to
/// or constructor for the production [`LiveProviderAttempt`] capability.
pub struct LocalProviderMockAttempt<'a> {
    authority: &'a LocalProviderAuthority,
    attempt_id: u32,
    cancelled: bool,
}

impl std::fmt::Debug for LocalProviderMockAttempt<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LocalProviderMockAttempt")
            .field("attempt_id", &self.attempt_id)
            .field("cancelled", &self.cancelled)
            .finish()
    }
}

impl LocalProviderMockAttempt<'_> {
    /// Scheduler identity assigned at admission.
    #[must_use]
    pub const fn attempt_id(&self) -> u32 {
        self.attempt_id
    }

    /// Cancel this attempt before a request is dispatched.
    pub fn cancel(&mut self) {
        self.cancelled = true;
    }

    /// Dispatch one deterministic bounded request through the local mock.
    pub fn execute(
        &self,
        model: &str,
        credential_reference_sha256: &str,
        request: &[u8],
    ) -> Result<LocalProviderMockResponse, LocalProviderMockError> {
        if self.cancelled {
            return Err(LocalProviderMockError::Cancelled);
        }
        self.authority.execute_mock_request(
            self.attempt_id,
            model,
            credential_reference_sha256,
            request,
        )
    }
}

/// Fixed runtime-owned local authority provisioner. Its zero-argument API is
/// deliberate: CLI/config/environment callers cannot provide authority.
pub struct LocalProviderAuthorityProvisioner;

/// Fail-closed errors while provisioning the local authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalProviderAuthorityError {
    /// The runtime could not create or secure ephemeral private roots.
    Unavailable,
}

impl LocalProviderAuthorityProvisioner {
    /// Provision one deterministic loopback authority without external I/O.
    pub fn provision() -> Result<LocalProviderAuthority, LocalProviderAuthorityError> {
        let generation = LOCAL_PROVIDER_GENERATION
            .fetch_add(1, Ordering::SeqCst)
            .checked_add(1)
            .ok_or(LocalProviderAuthorityError::Unavailable)?;
        let base = std::env::temp_dir().join(format!(
            "asb-local-provider-{}-{generation}",
            std::process::id()
        ));
        let lease_root = base.join("leases");
        let relay_root = base.join("relay");
        std::fs::create_dir(&base)
            .and_then(|_| std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o700)))
            .and_then(|_| std::fs::create_dir(&lease_root))
            .and_then(|_| std::fs::create_dir(&relay_root))
            .and_then(|_| {
                std::fs::set_permissions(&lease_root, std::fs::Permissions::from_mode(0o700))
            })
            .and_then(|_| {
                std::fs::set_permissions(&relay_root, std::fs::Permissions::from_mode(0o700))
            })
            .map_err(|_| LocalProviderAuthorityError::Unavailable)?;
        LOCAL_PROVIDER_ACTIVE_GENERATION.store(generation, Ordering::SeqCst);
        Ok(LocalProviderAuthority {
            generation,
            lease_root,
            relay_root,
            credential: LocalProviderCredentialCapability {
                reference_sha256: LOCAL_PROVIDER_CREDENTIAL_SHA256.to_owned(),
            },
            teardown: Arc::new(AtomicU64::new(generation)),
        })
    }
}

impl LocalProviderAuthority {
    /// Stable local provider identity; no endpoint or path is exposed.
    #[must_use]
    pub fn provider(&self) -> &'static str {
        "local-deterministic-mock"
    }

    /// Runtime generation fencing identity.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Loopback-only target identity for the local mock.
    #[must_use]
    pub fn target(&self) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], 4000))
    }

    /// Digest reference for the opaque in-memory credential marker.
    #[must_use]
    pub fn credential_reference_sha256(&self) -> &str {
        &self.credential.reference_sha256
    }

    /// Fixed digest identity for the pinned deterministic mock tool bundle.
    #[must_use]
    pub fn tool_bundle_sha256(&self) -> &'static str {
        LOCAL_PROVIDER_TOOL_SHA256
    }

    /// Execute one deterministic loopback protocol-double request.
    ///
    /// The method accepts only the opaque credential reference and a bounded
    /// request body. It never accepts an endpoint, credential bytes, policy,
    /// lease, namespace, or tool and therefore cannot be used to mint a live
    /// launch attempt. The response retains only a digest for evidence.
    pub fn execute_mock_request(
        &self,
        attempt_id: u32,
        model: &str,
        credential_reference_sha256: &str,
        request: &[u8],
    ) -> Result<LocalProviderMockResponse, LocalProviderMockError> {
        if !self.is_active() {
            return Err(LocalProviderMockError::Inactive);
        }
        if attempt_id == 0 {
            return Err(LocalProviderMockError::InvalidAttempt);
        }
        if model != LOCAL_PROVIDER_MODEL {
            return Err(LocalProviderMockError::ModelMismatch);
        }
        if credential_reference_sha256 != self.credential_reference_sha256() {
            return Err(LocalProviderMockError::CredentialMismatch);
        }
        if request.len() > LOCAL_PROVIDER_MAX_REQUEST_BYTES {
            return Err(LocalProviderMockError::RequestTooLarge);
        }
        let mut digest = Sha256::new();
        digest.update(LOCAL_PROVIDER_MODEL.as_bytes());
        digest.update(attempt_id.to_le_bytes());
        digest.update(request);
        Ok(LocalProviderMockResponse {
            attempt_id,
            response_sha256: format!("{:x}", digest.finalize()),
        })
    }

    /// Return whether this authority generation remains usable.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.teardown.load(Ordering::Acquire) == self.generation
            && LOCAL_PROVIDER_ACTIVE_GENERATION.load(Ordering::Acquire) == self.generation
    }

    /// Fence this authority before cancellation or teardown.
    pub fn revoke(&self) {
        self.teardown.store(0, Ordering::Release);
    }
}

impl Drop for LocalProviderAuthority {
    fn drop(&mut self) {
        self.revoke();
        let _ = std::fs::remove_dir_all(self.lease_root.parent().unwrap_or(&self.lease_root));
        let _ = std::fs::remove_dir_all(&self.relay_root);
    }
}

/// Runtime-owned scheduler composition for live provider attempts.
///
/// The scheduler owns the opaque provisioner handle and the validated launch
/// input. A frontend receives only the resulting factory, so it cannot supply
/// a provider, lease, relay, namespace, credential, or backend authority per
/// attempt. Each factory invocation creates a fresh attempt and binds its
/// scheduler identity at the runtime boundary.
pub struct LiveProviderRuntimeScheduler {
    factory: LiveProviderAttemptFactory,
}

/// Opaque runtime-owned source consumed by production run and sweep dispatch.
///
/// The source is minted only from an already authenticated runtime handle and
/// retains the scheduler factory privately. Frontends can move it into the
/// CLI boundary, but cannot provide or inspect any authority inputs.
pub struct LiveProviderRuntimeDispatchSource {
    scheduler: LiveProviderRuntimeScheduler,
}

impl std::fmt::Debug for LiveProviderRuntimeDispatchSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("LiveProviderRuntimeDispatchSource(..)")
    }
}

impl std::fmt::Debug for LiveProviderRuntimeScheduler {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("LiveProviderRuntimeScheduler(..)")
    }
}

/// Secret-free claims authenticated by the control certificate chain.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiveProviderControlClaims {
    provider: String,
    endpoint_identity_sha256: String,
    credential_ref_sha256: String,
    generation: u64,
    target: SocketAddr,
    tool_bundle_sha256: String,
    lease_root_sha256: String,
    relay_root_sha256: String,
}

/// Runtime-only result of authenticated control enrollment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiveProviderControlAttestation {
    chain_sha256: String,
    claims: LiveProviderControlClaims,
}

const MAX_ENROLLMENT_RECORD_BYTES: usize = 16 * 1024;
#[allow(dead_code)] // Used by the runtime/control bridge and validation.
const ENROLLMENT_RECORD_SCHEMA_VERSION: u16 = 1;

/// Bounded, secret-free enrollment transport emitted by the authenticated
/// control/runtime bridge. Private roots are represented by digests only.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LiveProviderEnrollmentRecordV1 {
    schema_version: u16,
    chain_sha256: String,
    provider: String,
    endpoint_identity_sha256: String,
    credential_ref_sha256: String,
    generation: u64,
    target: SocketAddr,
    tool_bundle_sha256: String,
    lease_root_sha256: String,
    relay_root_sha256: String,
    issued_at_unix_ms: u64,
    expires_at_unix_ms: u64,
    nonce_sha256: String,
}

/// Errors from bounded enrollment-record transport and freshness validation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LiveProviderEnrollmentRecordError {
    /// The record exceeds the bounded transport size.
    TooLarge,
    /// The JSON record is malformed or contains unknown fields.
    Malformed,
    /// The record schema version is unsupported.
    UnsupportedSchema,
    /// The record is expired, not yet valid, or has an invalid interval.
    Freshness,
    /// The record does not match the authenticated control attestation.
    AttestationMismatch,
    /// A digest or identity field is invalid.
    InvalidIdentity,
    /// The record was already consumed by this runtime ledger.
    Replay,
}

impl LiveProviderEnrollmentRecordV1 {
    /// Encode the record with a hard byte ceiling and no secret material.
    pub fn encode(&self) -> Result<Vec<u8>, LiveProviderEnrollmentRecordError> {
        let encoded =
            serde_json::to_vec(self).map_err(|_| LiveProviderEnrollmentRecordError::Malformed)?;
        if encoded.len() > MAX_ENROLLMENT_RECORD_BYTES {
            return Err(LiveProviderEnrollmentRecordError::TooLarge);
        }
        Ok(encoded)
    }

    /// Decode a bounded record, rejecting oversized or unknown input.
    pub fn decode(bytes: &[u8]) -> Result<Self, LiveProviderEnrollmentRecordError> {
        if bytes.len() > MAX_ENROLLMENT_RECORD_BYTES {
            return Err(LiveProviderEnrollmentRecordError::TooLarge);
        }
        serde_json::from_slice(bytes).map_err(|_| LiveProviderEnrollmentRecordError::Malformed)
    }

    /// Validate this record against an authenticated control attestation.
    #[allow(dead_code)] // Called by the runtime/control bridge and ledger.
    pub(crate) fn validate_against(
        &self,
        attestation: &LiveProviderControlAttestation,
        now_unix_ms: u64,
    ) -> Result<(), LiveProviderEnrollmentRecordError> {
        if self.schema_version != ENROLLMENT_RECORD_SCHEMA_VERSION {
            return Err(LiveProviderEnrollmentRecordError::UnsupportedSchema);
        }
        if self.issued_at_unix_ms > self.expires_at_unix_ms
            || now_unix_ms < self.issued_at_unix_ms
            || now_unix_ms > self.expires_at_unix_ms
            || self.expires_at_unix_ms - self.issued_at_unix_ms > 15 * 60 * 1000
        {
            return Err(LiveProviderEnrollmentRecordError::Freshness);
        }
        let claims = attestation.claims();
        if self.chain_sha256 != attestation.chain_sha256()
            || self.provider != claims.provider
            || self.endpoint_identity_sha256 != claims.endpoint_identity_sha256
            || self.credential_ref_sha256 != claims.credential_ref_sha256
            || self.generation != claims.generation
            || self.target != claims.target
            || self.tool_bundle_sha256 != claims.tool_bundle_sha256
            || self.lease_root_sha256 != claims.lease_root_sha256
            || self.relay_root_sha256 != claims.relay_root_sha256
        {
            return Err(LiveProviderEnrollmentRecordError::AttestationMismatch);
        }
        if !valid_digest(&self.chain_sha256)
            || !valid_digest(&self.endpoint_identity_sha256)
            || !valid_digest(&self.credential_ref_sha256)
            || !valid_digest(&self.tool_bundle_sha256)
            || !valid_digest(&self.lease_root_sha256)
            || !valid_digest(&self.relay_root_sha256)
            || !valid_digest(&self.nonce_sha256)
            || self.provider.is_empty()
            || self.generation == 0
        {
            return Err(LiveProviderEnrollmentRecordError::InvalidIdentity);
        }
        if self.nonce_sha256 != expected_nonce(attestation) {
            return Err(LiveProviderEnrollmentRecordError::AttestationMismatch);
        }
        Ok(())
    }

    #[allow(dead_code)] // Called by the runtime replay ledger.
    fn id_sha256(&self) -> Result<String, LiveProviderEnrollmentRecordError> {
        let encoded = self.encode()?;
        Ok(format!("{:x}", Sha256::digest(encoded)))
    }

    pub(crate) fn from_attestation(
        attestation: &LiveProviderControlAttestation,
        issued_at_unix_ms: u64,
        expires_at_unix_ms: u64,
    ) -> Self {
        let claims = attestation.claims();
        Self {
            schema_version: ENROLLMENT_RECORD_SCHEMA_VERSION,
            chain_sha256: attestation.chain_sha256().to_owned(),
            provider: claims.provider.clone(),
            endpoint_identity_sha256: claims.endpoint_identity_sha256.clone(),
            credential_ref_sha256: claims.credential_ref_sha256.clone(),
            generation: claims.generation,
            target: claims.target,
            tool_bundle_sha256: claims.tool_bundle_sha256.clone(),
            lease_root_sha256: claims.lease_root_sha256.clone(),
            relay_root_sha256: claims.relay_root_sha256.clone(),
            issued_at_unix_ms,
            expires_at_unix_ms,
            nonce_sha256: expected_nonce(attestation),
        }
    }
}

/// Runtime-owned bridge from an authenticated control receipt to a bounded
/// enrollment record. The CLI receives no certificate authority or private
/// bootstrap inputs through this boundary.
#[derive(Debug, Default)]
pub struct LiveProviderRuntimeBridge {
    ledger: LiveProviderEnrollmentLedger,
}

/// Opaque runtime-owned profile produced only after authenticated receipt
/// validation and one-shot replay consumption. The profile retains no raw
/// credentials or private paths and cannot be constructed by a frontend.
#[derive(Clone)]
pub struct LiveProviderRuntimeAuthorityProfile {
    record: LiveProviderEnrollmentRecordV1,
    attestation: LiveProviderControlAttestation,
}

impl std::fmt::Debug for LiveProviderRuntimeAuthorityProfile {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("LiveProviderRuntimeAuthorityProfile(..)")
    }
}

impl LiveProviderRuntimeAuthorityProfile {
    /// Authenticated provider identity bound to this profile.
    #[must_use]
    pub fn provider(&self) -> &str {
        &self.record.provider
    }

    /// Authenticated certificate generation bound to this profile.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.attestation.claims.generation
    }

    /// Consume runtime-owned enrolled material and mint the opaque live
    /// handle. This boundary is crate-private so a CLI cannot provide policy,
    /// roots, tools, or backend authority.
    #[allow(clippy::too_many_arguments)]
    #[allow(dead_code)] // Consumed by the authenticated runtime/control caller.
    pub(crate) fn materialize_handle(
        self,
        config: LiveProviderRuntimeConfig,
        policy: ProviderEgressPolicy,
        allowlist: ProviderEgressAllowlist,
        relay_root: &Path,
        bubblewrap: ToolPin,
        systemd_run: ToolPin,
        systemctl: ToolPin,
        taskset: ToolPin,
        live_launch_gate: ToolPin,
    ) -> Result<LiveProviderRuntimeHandle, LiveProviderProvisionError> {
        let claims = self.attestation.claims();
        if self.record.target != config.target().address()
            || self.record.generation != claims.generation
            || self.record.credential_ref_sha256 != config.credential_ref_sha256()
            || self.record.provider.is_empty()
        {
            return Err(LiveProviderProvisionError::InvalidConfiguration);
        }
        LiveProviderBootstrapSpec::from_enrollment(
            config,
            policy,
            allowlist,
            relay_root,
            bubblewrap,
            systemd_run,
            systemctl,
            taskset,
            live_launch_gate,
        )
        .map_err(|_| LiveProviderProvisionError::InvalidConfiguration)?
        .provisioner()
        .map_err(|_| LiveProviderProvisionError::InvalidConfiguration)
    }
}

impl LiveProviderRuntimeBridge {
    /// Create an empty bridge with one-shot replay protection.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Materialize one opaque authority profile from authenticated runtime
    /// state. Caller-supplied records, stale attestations, and replayed
    /// records fail closed before any live launch authority is available.
    pub fn materialize_profile(
        &self,
        record: &LiveProviderEnrollmentRecordV1,
        attestation: &LiveProviderControlAttestation,
        now_unix_ms: u64,
    ) -> Result<LiveProviderRuntimeAuthorityProfile, LiveProviderEnrollmentRecordError> {
        self.ledger.consume(record, attestation, now_unix_ms)?;
        Ok(LiveProviderRuntimeAuthorityProfile {
            record: record.clone(),
            attestation: attestation.clone(),
        })
    }

    /// Validate a control-issued receipt against its opaque authenticated chain
    /// and consume it exactly once in the runtime ledger.
    pub fn ingest_control_receipt(
        &self,
        receipt: &RuntimeEnrollmentReceiptV1,
        chain: &IssuedCertificateChainV1,
        now_unix_ms: u64,
    ) -> Result<LiveProviderEnrollmentRecordV1, LiveProviderEnrollmentRecordError> {
        if receipt.schema_version != ENROLLMENT_RECORD_SCHEMA_VERSION
            || receipt.chain_sha256 != chain.chain_sha256()
        {
            return Err(LiveProviderEnrollmentRecordError::AttestationMismatch);
        }
        let target = receipt
            .target
            .parse()
            .map_err(|_| LiveProviderEnrollmentRecordError::InvalidIdentity)?;
        let identity = chain.identity();
        if receipt.endpoint_identity_sha256 != identity.endpoint_identity_sha256
            || receipt.generation != identity.generation
        {
            return Err(LiveProviderEnrollmentRecordError::AttestationMismatch);
        }
        let claims = LiveProviderControlClaims::new(
            receipt.provider.clone(),
            receipt.endpoint_identity_sha256.clone(),
            receipt.credential_ref_sha256.clone(),
            receipt.generation,
            target,
            receipt.tool_bundle_sha256.clone(),
            receipt.lease_root_sha256.clone(),
            receipt.relay_root_sha256.clone(),
        )
        .map_err(|_| LiveProviderEnrollmentRecordError::InvalidIdentity)?;
        let attestation = LiveProviderControlAttestation::from_control(chain, claims)
            .map_err(|_| LiveProviderEnrollmentRecordError::AttestationMismatch)?;
        let record = LiveProviderEnrollmentRecordV1::from_attestation(
            &attestation,
            receipt.issued_at_unix_ms,
            receipt.expires_at_unix_ms,
        );
        if record.nonce_sha256 != receipt.nonce_sha256 {
            return Err(LiveProviderEnrollmentRecordError::AttestationMismatch);
        }
        self.ledger.consume(&record, &attestation, now_unix_ms)?;
        Ok(record)
    }

    /// Materialize an opaque authority profile directly from one authenticated
    /// control receipt and its already validated certificate chain.  This is
    /// the runtime-owned receipt-to-bootstrap boundary: callers cannot inject
    /// a record, endpoint, credential, policy, root, or tool and the ledger
    /// consumes the receipt before a profile is returned.
    pub fn materialize_control_receipt_profile(
        &self,
        receipt: &RuntimeEnrollmentReceiptV1,
        chain: &IssuedCertificateChainV1,
        now_unix_ms: u64,
    ) -> Result<LiveProviderRuntimeAuthorityProfile, LiveProviderEnrollmentRecordError> {
        let record = self.ingest_control_receipt(receipt, chain, now_unix_ms)?;
        let claims = LiveProviderControlClaims::new(
            record.provider.clone(),
            record.endpoint_identity_sha256.clone(),
            record.credential_ref_sha256.clone(),
            record.generation,
            record.target,
            record.tool_bundle_sha256.clone(),
            record.lease_root_sha256.clone(),
            record.relay_root_sha256.clone(),
        )
        .map_err(|_| LiveProviderEnrollmentRecordError::InvalidIdentity)?;
        let attestation = LiveProviderControlAttestation::from_control(chain, claims)
            .map_err(|_| LiveProviderEnrollmentRecordError::AttestationMismatch)?;
        Ok(LiveProviderRuntimeAuthorityProfile {
            record,
            attestation,
        })
    }

    /// Consume one authenticated control response for the runtime dispatch
    /// path. The request binding and certificate chain are supplied by the
    /// control/runtime boundary; callers cannot turn a fabricated response
    /// into a live enrollment record.
    pub fn ingest_control_response(
        &self,
        request: &RuntimeReceiptRequestV1,
        response: &RuntimeReceiptResponseV1,
        chain: &IssuedCertificateChainV1,
        now_unix_ms: u64,
    ) -> Result<LiveProviderEnrollmentRecordV1, LiveProviderEnrollmentRecordError> {
        response
            .validate_for(request)
            .map_err(|_| LiveProviderEnrollmentRecordError::AttestationMismatch)?;
        self.ingest_control_receipt(&response.receipt, chain, now_unix_ms)
    }

    /// Request and validate one receipt through the authenticated control
    /// client, using only the opaque chain retained by the runtime store.
    pub fn request_control_receipt(
        &self,
        client: &mut ControlClient,
        chains: &RuntimeCertificateChainStore,
        request: RuntimeReceiptRequestV1,
        now_unix_ms: u64,
    ) -> Result<LiveProviderEnrollmentRecordV1, LiveProviderControlAdapterError> {
        let response = client
            .call(ControlCall::RuntimeReceipt(request.clone()), 60_000)
            .map_err(|_| LiveProviderControlAdapterError::Transport)?;
        let result = response
            .result()
            .and_then(|success| match success {
                ControlSuccess::Operation(bound) => Some(&bound.result),
                ControlSuccess::Negotiated(_) => None,
            })
            .and_then(|result| match result {
                ControlResult::RuntimeReceipt(receipt) => Some(receipt),
                _ => None,
            })
            .ok_or(LiveProviderControlAdapterError::InvalidResponse)?;
        let chain = chains
            .chain()
            .map_err(|_| LiveProviderControlAdapterError::ChainUnavailable)?;
        self.ingest_control_response(&request, result, &chain, now_unix_ms)
            .map_err(|_| LiveProviderControlAdapterError::AttestationMismatch)
    }
}

#[allow(dead_code)] // Used by control-issued record construction.
fn expected_nonce(attestation: &LiveProviderControlAttestation) -> String {
    let claims = attestation.claims();
    let mut digest = Sha256::new();
    digest.update(attestation.chain_sha256().as_bytes());
    digest.update(claims.provider.as_bytes());
    digest.update(claims.credential_ref_sha256.as_bytes());
    digest.update(claims.generation.to_le_bytes());
    digest.update(claims.target.to_string().as_bytes());
    digest.update(claims.tool_bundle_sha256.as_bytes());
    digest.update(claims.lease_root_sha256.as_bytes());
    digest.update(claims.relay_root_sha256.as_bytes());
    format!("{:x}", digest.finalize())
}

/// Runtime-owned replay ledger for one-way enrollment record consumption.
#[derive(Debug, Default)]
pub struct LiveProviderEnrollmentLedger {
    consumed: Mutex<BTreeSet<String>>,
}

impl LiveProviderEnrollmentLedger {
    /// Create an empty bounded enrollment ledger.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Validate and consume one record exactly once for this runtime instance.
    #[allow(dead_code)] // Called by the runtime-owned record ingestion path.
    pub(crate) fn consume(
        &self,
        record: &LiveProviderEnrollmentRecordV1,
        attestation: &LiveProviderControlAttestation,
        now_unix_ms: u64,
    ) -> Result<(), LiveProviderEnrollmentRecordError> {
        record.validate_against(attestation, now_unix_ms)?;
        let id = record.id_sha256()?;
        let mut consumed = self
            .consumed
            .lock()
            .map_err(|_| LiveProviderEnrollmentRecordError::Replay)?;
        if !consumed.insert(id) {
            return Err(LiveProviderEnrollmentRecordError::Replay);
        }
        Ok(())
    }
}

impl LiveProviderControlClaims {
    /// Construct bounded, secret-free claims for the runtime handoff.
    #[allow(clippy::too_many_arguments)]
    #[allow(dead_code)] // Consumed by the authenticated control bridge in AR-1355.
    pub(crate) fn new(
        provider: String,
        endpoint_identity_sha256: String,
        credential_ref_sha256: String,
        generation: u64,
        target: SocketAddr,
        tool_bundle_sha256: String,
        lease_root_sha256: String,
        relay_root_sha256: String,
    ) -> Result<Self, LiveProviderEnrollmentError> {
        if provider.is_empty()
            || generation == 0
            || !valid_digest(&endpoint_identity_sha256)
            || !valid_digest(&credential_ref_sha256)
            || !valid_digest(&tool_bundle_sha256)
            || !valid_digest(&lease_root_sha256)
            || !valid_digest(&relay_root_sha256)
        {
            return Err(LiveProviderEnrollmentError::Unavailable);
        }
        ProviderEgressTarget::new(target).map_err(|_| LiveProviderEnrollmentError::Unavailable)?;
        Ok(Self {
            provider,
            endpoint_identity_sha256,
            credential_ref_sha256,
            generation,
            target,
            tool_bundle_sha256,
            lease_root_sha256,
            relay_root_sha256,
        })
    }
}

impl LiveProviderControlAttestation {
    /// Bind claims to a certificate chain already authenticated by control.
    #[allow(dead_code)] // Consumed by the authenticated control bridge in AR-1355.
    pub(crate) fn from_control(
        chain: &IssuedCertificateChainV1,
        claims: LiveProviderControlClaims,
    ) -> Result<Self, LiveProviderEnrollmentError> {
        let identity = chain.identity();
        if identity.endpoint_identity_sha256 != claims.endpoint_identity_sha256
            || identity.generation != claims.generation
            || !matches!(identity.role.as_str(), "operator" | "administrator")
        {
            return Err(LiveProviderEnrollmentError::Unavailable);
        }
        Ok(Self {
            chain_sha256: chain.chain_sha256().to_owned(),
            claims,
        })
    }

    /// Return the authenticated chain identity without secret material.
    #[must_use]
    #[allow(dead_code)] // Consumed by the authenticated control bridge in AR-1355.
    pub(crate) fn chain_sha256(&self) -> &str {
        &self.chain_sha256
    }

    /// Return the bounded claims for runtime-owned provisioning.
    #[must_use]
    #[allow(dead_code)] // Consumed by the authenticated control bridge in AR-1355.
    pub(crate) fn claims(&self) -> &LiveProviderControlClaims {
        &self.claims
    }
}

/// Cross-crate enrollment source. Implementations may be supplied by the
/// runtime/control layer, but can return only an opaque runtime-minted handle.
pub trait LiveProviderEnrollment: Send + Sync {
    /// Return one enrolled runtime handle or a bounded failure.
    fn enroll(&self) -> Result<LiveProviderRuntimeHandle, LiveProviderEnrollmentError>;
}

/// Enrollment failures intentionally contain no paths, output, or secrets.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LiveProviderEnrollmentError {
    /// The installed enrollment is absent, stale, or invalid.
    Unavailable,
}

/// Opaque runtime-owned handle minted only by the private bootstrap path.
/// Callers can pass it back to the service but cannot construct or inspect
/// policy, backend, relay-root, namespace, lease, or credential authority.
pub struct LiveProviderRuntimeHandle {
    provisioner: LiveProviderProvisioner,
}

impl std::fmt::Debug for LiveProviderRuntimeHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("LiveProviderRuntimeHandle(..)")
    }
}

impl LiveProviderRuntimeService {
    /// Acquire through a runtime-owned enrollment source and opaque handle.
    pub fn acquire_from_enrollment(
        &self,
        enrollment: &dyn LiveProviderEnrollment,
        attempt_id: u32,
        input: SandboxLaunchInput,
        limits: ProcessLimits,
        adapter_sha256: &str,
        now_unix_ms: u64,
    ) -> Result<LiveProviderAttempt, LiveProviderProvisionError> {
        let handle = enrollment
            .enroll()
            .map_err(|_| LiveProviderProvisionError::InvalidConfiguration)?;
        self.acquire(
            &handle,
            attempt_id,
            input,
            limits,
            adapter_sha256,
            now_unix_ms,
        )
    }

    /// Consume the opaque runtime handle for one scheduler attempt.
    pub fn acquire(
        &self,
        handle: &LiveProviderRuntimeHandle,
        attempt_id: u32,
        input: SandboxLaunchInput,
        limits: ProcessLimits,
        adapter_sha256: &str,
        now_unix_ms: u64,
    ) -> Result<LiveProviderAttempt, LiveProviderProvisionError> {
        handle
            .provisioner
            .acquire(attempt_id, input, limits, adapter_sha256, now_unix_ms)
    }

    /// Consume an authenticated record inside the runtime-owned bridge and
    /// mint the opaque handle only after replay and freshness checks pass.
    /// The bootstrap specification is crate-private so CLI callers cannot
    /// provide policy, tools, roots, or backend authority.
    #[allow(clippy::too_many_arguments)]
    #[allow(dead_code)] // Consumed by the authenticated run/sweep bridge.
    pub(crate) fn acquire_from_record(
        &self,
        ledger: &LiveProviderEnrollmentLedger,
        record: &LiveProviderEnrollmentRecordV1,
        attestation: &LiveProviderControlAttestation,
        bootstrap: LiveProviderBootstrapSpec,
        attempt_id: u32,
        input: SandboxLaunchInput,
        limits: ProcessLimits,
        adapter_sha256: &str,
        now_unix_ms: u64,
    ) -> Result<LiveProviderAttempt, LiveProviderProvisionError> {
        ledger
            .consume(record, attestation, now_unix_ms)
            .map_err(|_| LiveProviderProvisionError::InvalidConfiguration)?;
        let handle = bootstrap
            .provisioner()
            .map_err(|_| LiveProviderProvisionError::InvalidConfiguration)?;
        self.acquire(
            &handle,
            attempt_id,
            input,
            limits,
            adapter_sha256,
            now_unix_ms,
        )
    }

    /// Resolve the adapter-owned credential without exposing bytes or authority.
    pub fn resolve_credential(
        &self,
        resolver: &mut dyn LiveProviderResolver,
        selection: &LiveProviderRuntimeSelection,
    ) -> Result<Box<dyn CredentialInjection>, LiveProviderResolveError> {
        resolver.resolve_credential(selection)
    }
}

impl LiveProviderRuntimeScheduler {
    /// Compose a scheduler factory from a runtime-issued opaque handle.
    ///
    /// `input`, `limits`, and `adapter_sha256` are validated runtime values;
    /// this constructor rejects malformed adapter identities and non-denied
    /// network policy before exposing a factory to a frontend.
    pub fn new(
        handle: LiveProviderRuntimeHandle,
        input: SandboxLaunchInput,
        limits: ProcessLimits,
        adapter_sha256: &str,
    ) -> Result<Self, LiveProviderProvisionError> {
        if input.spec().network_policy() != NetworkPolicy::Deny
            || input.limits() != limits
            || !valid_digest(adapter_sha256)
        {
            return Err(LiveProviderProvisionError::InvalidConfiguration);
        }
        let adapter_sha256 = adapter_sha256.to_owned();
        let service = LiveProviderRuntimeService;
        let factory = LiveProviderAttemptFactory::from_fn(move |attempt_id, _warmup| {
            let now_unix_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| LaunchAuthorityError::InvalidLaunchInput)?
                .as_millis()
                .try_into()
                .map_err(|_| LaunchAuthorityError::InvalidLaunchInput)?;
            service
                .acquire(
                    &handle,
                    attempt_id,
                    input.clone(),
                    limits,
                    &adapter_sha256,
                    now_unix_ms,
                )
                .map_err(|_| LaunchAuthorityError::InvalidLaunchInput)
        });
        Ok(Self { factory })
    }

    /// Return the opaque per-attempt factory for a run or sweep scheduler.
    #[must_use]
    pub fn into_factory(self) -> LiveProviderAttemptFactory {
        self.factory
    }
}

impl LiveProviderRuntimeDispatchSource {
    /// Mint a dispatch source from an opaque runtime handle. All launch
    /// inputs remain runtime-owned and network-denied validation is retained.
    pub fn from_handle(
        handle: LiveProviderRuntimeHandle,
        input: SandboxLaunchInput,
        limits: ProcessLimits,
        adapter_sha256: &str,
    ) -> Result<Self, LiveProviderProvisionError> {
        Ok(Self {
            scheduler: LiveProviderRuntimeScheduler::new(handle, input, limits, adapter_sha256)?,
        })
    }

    /// Consume the opaque source at the CLI run/sweep boundary.
    #[must_use]
    pub fn into_scheduler(self) -> LiveProviderRuntimeScheduler {
        self.scheduler
    }
}

/// Runtime-owned enrollment used to construct the live provisioner.
///
/// This type is crate-private on purpose: CLI callers cannot provide a policy,
/// allowlist, tool pin, or filesystem root. The control/runtime layer creates
/// it only after loading the enrolled installation record.
#[derive(Debug)]
#[allow(dead_code)] // AR-1349 consumes the bootstrap handle when CLI wiring lands.
pub(crate) struct LiveProviderBootstrapSpec {
    config: LiveProviderRuntimeConfig,
    policy: ProviderEgressPolicy,
    allowlist: ProviderEgressAllowlist,
    relay_root: PathBuf,
    tools: [ToolPin; 5],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)] // Bootstrap failures remain private until the runtime caller is wired.
pub(crate) enum LiveProviderBootstrapError {
    Enrollment,
    RelayRoot,
    Backend,
}

#[allow(dead_code)]
impl LiveProviderBootstrapSpec {
    /// Build an enrolled bootstrap record from already validated runtime data.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_enrollment(
        config: LiveProviderRuntimeConfig,
        policy: ProviderEgressPolicy,
        allowlist: ProviderEgressAllowlist,
        relay_root: &Path,
        bubblewrap: ToolPin,
        systemd_run: ToolPin,
        systemctl: ToolPin,
        taskset: ToolPin,
        live_launch_gate: ToolPin,
    ) -> Result<Self, LiveProviderBootstrapError> {
        let canonical =
            std::fs::canonicalize(relay_root).map_err(|_| LiveProviderBootstrapError::RelayRoot)?;
        let metadata =
            std::fs::metadata(&canonical).map_err(|_| LiveProviderBootstrapError::RelayRoot)?;
        if !metadata.is_dir() || canonical != relay_root {
            return Err(LiveProviderBootstrapError::RelayRoot);
        }
        if policy.endpoint().is_empty()
            || !allowlist.permits(config.target().address())
            || config.generation().is_empty()
        {
            return Err(LiveProviderBootstrapError::Enrollment);
        }
        Ok(Self {
            config,
            policy,
            allowlist,
            relay_root: canonical,
            tools: [
                bubblewrap,
                systemd_run,
                systemctl,
                taskset,
                live_launch_gate,
            ],
        })
    }

    /// Consume the enrollment and return only the opaque provisioner handle.
    pub(crate) fn provisioner(
        self,
    ) -> Result<LiveProviderRuntimeHandle, LiveProviderBootstrapError> {
        let [
            bubblewrap,
            systemd_run,
            systemctl,
            taskset,
            live_launch_gate,
        ] = self.tools;
        let backend = SandboxBackend::new(bubblewrap, systemd_run, systemctl, taskset)
            .with_live_launch_gate(live_launch_gate);
        LiveProviderProvisioner::new(
            self.config,
            self.policy,
            self.allowlist,
            backend,
            &self.relay_root,
        )
        .map(|provisioner| LiveProviderRuntimeHandle { provisioner })
        .map_err(|_| LiveProviderBootstrapError::Backend)
    }
}

/// Runtime-owned composition boundary for one live-provider attempt.
///
/// The service owns the policy, concrete target allowlist, lease namespace,
/// sandbox backend and relay root. Callers provide only an already validated
/// launch input and an adapter identity; they cannot provide a namespace,
/// handoff, lease, token, relay or credential bytes.
pub struct LiveProviderProvisioner {
    config: LiveProviderRuntimeConfig,
    policy: ProviderEgressPolicy,
    allowlist: ProviderEgressAllowlist,
    backend: SandboxBackend,
    relay_root: PathBuf,
}

/// Errors intentionally omit paths, subprocess output and credential data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LiveProviderProvisionError {
    /// The runtime configuration or attempt identity was invalid.
    InvalidConfiguration,
    /// Runtime namespace observation or clock access failed.
    RuntimeObservation,
    /// Lease acquisition failed or the lease did not match the launch.
    LeaseUnavailable,
    /// Authenticated relay construction failed closed.
    RelayUnavailable,
    /// The pinned backend could not attest the launch boundary.
    BackendUnavailable,
    /// Runtime launch authority could not be issued.
    AuthorityRejected,
}

impl LiveProviderProvisioner {
    /// Construct a service from runtime-owned policy and pinned backend data.
    #[allow(dead_code)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        config: LiveProviderRuntimeConfig,
        policy: ProviderEgressPolicy,
        allowlist: ProviderEgressAllowlist,
        backend: SandboxBackend,
        relay_root: &Path,
    ) -> Result<Self, LiveProviderProvisionError> {
        if !relay_root.is_absolute()
            || !relay_root.is_dir()
            || !allowlist.permits(config.target().address())
            || policy.endpoint().is_empty()
        {
            return Err(LiveProviderProvisionError::InvalidConfiguration);
        }
        Ok(Self {
            config,
            policy,
            allowlist,
            backend,
            relay_root: relay_root.to_owned(),
        })
    }

    /// Acquire one opaque attempt, binding every authority to the runtime's
    /// observed namespace and to the scheduler attempt identity.
    #[allow(dead_code)]
    pub(crate) fn acquire(
        &self,
        attempt_id: u32,
        input: SandboxLaunchInput,
        limits: ProcessLimits,
        adapter_sha256: &str,
        now_unix_ms: u64,
    ) -> Result<LiveProviderAttempt, LiveProviderProvisionError> {
        if attempt_id == 0
            || !valid_digest(adapter_sha256)
            || input.spec().network_policy() != NetworkPolicy::Deny
            || input.limits() != limits
        {
            return Err(LiveProviderProvisionError::InvalidConfiguration);
        }
        let namespace = NamespaceIdentity::current()
            .map_err(|_| LiveProviderProvisionError::RuntimeObservation)?;
        let generation = format!("{}-{attempt_id}", self.config.generation());
        if generation.len() > 128 {
            return Err(LiveProviderProvisionError::InvalidConfiguration);
        }
        let deadline = now_unix_ms
            .checked_add(60_000)
            .ok_or(LiveProviderProvisionError::InvalidConfiguration)?;
        let route = self.config.route_sha256().to_owned();
        let egress =
            ProviderEgressHandoff::issue_bound(&self.policy, &generation, &route, deadline);
        let child = PathBuf::from(format!("/tmp/asb-live-provider-{attempt_id}.sock"));
        let lease = self
            .config
            .acquire_lease()
            .map_err(|_| LiveProviderProvisionError::LeaseUnavailable)?;
        let (relay, handoff) = LiveProviderRelay::bind_runtime(
            &self.relay_root,
            &self.policy,
            &egress,
            namespace.clone(),
            self.allowlist.clone(),
            self.config.target(),
            generation,
            self.config.route_sha256().to_owned(),
            adapter_sha256.to_owned(),
            self.config.credential_ref_sha256().to_owned(),
            child,
            limits.timeout(),
            16 * 1024 * 1024,
            now_unix_ms,
        )
        .map_err(|_| LiveProviderProvisionError::RelayUnavailable)?;
        let input = input
            .with_live_provider_handoff(handoff, namespace.clone(), now_unix_ms)
            .map_err(|_| LiveProviderProvisionError::RuntimeObservation)?;
        let token = self
            .backend
            .attest_live_launch(&input, &lease, now_unix_ms)
            .map_err(|_| LiveProviderProvisionError::BackendUnavailable)?;
        LiveLaunchFactory::acquire(
            token,
            input,
            lease,
            self.backend.clone(),
            namespace,
            now_unix_ms,
            relay,
        )
        .map_err(|error| match error {
            LaunchAuthorityError::InvalidLaunchInput
            | LaunchAuthorityError::InvalidLease
            | LaunchAuthorityError::IdentityMismatch => {
                LiveProviderProvisionError::AuthorityRejected
            }
        })
    }
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
    use crate::sandbox::{Resources, SandboxSpec, ToolPin};
    use asb_control::{CertificateAuthorityV1, CertificateIdentityV1};
    use sha2::{Digest, Sha256};
    use std::collections::BTreeMap;
    use std::net::SocketAddr;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

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

    fn provisioner() -> (LiveProviderProvisioner, PathBuf) {
        let lease_root = root();
        let relay_root = root();
        let selected = target();
        let allowlist = ProviderEgressAllowlist::new(vec![selected]).unwrap();
        let config = LiveProviderRuntimeConfig::new(
            &lease_root,
            &allowlist,
            selection(
                selected,
                "generation-1",
                "a".repeat(64),
                "b".repeat(64),
                NetworkPolicy::Deny,
            ),
            CpuSet::new(vec![0]).unwrap(),
        )
        .unwrap();
        let policy =
            ProviderEgressPolicy::new("https://openrouter.ai/api/v1", "openrouter.ai").unwrap();
        let pin = |path: &str| ToolPin::new(path.into(), "test".into()).unwrap();
        let backend = SandboxBackend::new(
            pin("/bin/true"),
            pin("/bin/true"),
            pin("/bin/true"),
            pin("/bin/true"),
        );
        (
            LiveProviderProvisioner::new(config, policy, allowlist, backend, &relay_root).unwrap(),
            relay_root,
        )
    }

    fn bootstrap_spec() -> (LiveProviderBootstrapSpec, PathBuf) {
        let lease_root = root();
        let relay_root = root();
        let selected = target();
        let allowlist = ProviderEgressAllowlist::new(vec![selected]).unwrap();
        let config = LiveProviderRuntimeConfig::new(
            &lease_root,
            &allowlist,
            selection(
                selected,
                "generation-1",
                "a".repeat(64),
                "b".repeat(64),
                NetworkPolicy::Deny,
            ),
            CpuSet::new(vec![0]).unwrap(),
        )
        .unwrap();
        let policy =
            ProviderEgressPolicy::new("https://openrouter.ai/api/v1", "openrouter.ai").unwrap();
        let pin = |path: &str| ToolPin::new(path.into(), "test".into()).unwrap();
        let spec = LiveProviderBootstrapSpec::from_enrollment(
            config,
            policy,
            allowlist,
            &relay_root,
            pin("/bin/true"),
            pin("/bin/true"),
            pin("/bin/true"),
            pin("/bin/true"),
            pin("/bin/true"),
        )
        .unwrap();
        (spec, relay_root)
    }

    fn launch_input(root: &Path) -> SandboxLaunchInput {
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let resources =
            Resources::new(64 * 1024 * 1024, 16, 100, CpuSet::new(vec![0]).unwrap()).unwrap();
        let spec = SandboxSpec::new(
            &workspace,
            PathBuf::from("."),
            "/bin/true".into(),
            Vec::new(),
            BTreeMap::new(),
            resources,
            NetworkPolicy::Deny,
        )
        .unwrap();
        SandboxLaunchInput::new(
            spec,
            ProcessLimits::new(
                4096,
                4096,
                Duration::from_secs(1),
                Duration::from_millis(100),
                Duration::from_millis(5),
            )
            .unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn bootstrap_returns_only_runtime_owned_provisioner() {
        let (spec, relay_root) = bootstrap_spec();
        let provisioner = spec.provisioner().unwrap();
        assert_eq!(provisioner.provisioner.config.generation(), "generation-1");
        assert_eq!(provisioner.provisioner.policy.host(), "openrouter.ai");
        assert_eq!(provisioner.provisioner.relay_root, relay_root);
        let _ = std::fs::remove_dir_all(relay_root);
    }

    #[test]
    fn scheduler_composition_rejects_unbound_or_mismatched_inputs() {
        let (spec, relay_root) = bootstrap_spec();
        let handle = spec.provisioner().unwrap();
        let input = launch_input(&root());
        let limits = input.limits();
        assert_eq!(
            LiveProviderRuntimeScheduler::new(handle, input.clone(), limits, "not-a-digest",)
                .unwrap_err(),
            LiveProviderProvisionError::InvalidConfiguration
        );

        let (spec, _) = bootstrap_spec();
        let handle = spec.provisioner().unwrap();
        let different_limits = ProcessLimits::new(
            4096,
            4096,
            Duration::from_secs(2),
            Duration::from_millis(100),
            Duration::from_millis(5),
        )
        .unwrap();
        assert_eq!(
            LiveProviderRuntimeScheduler::new(handle, input, different_limits, &"c".repeat(64),)
                .unwrap_err(),
            LiveProviderProvisionError::InvalidConfiguration
        );
        let _ = std::fs::remove_dir_all(relay_root);
    }

    #[test]
    fn dispatch_source_is_runtime_owned_and_consumable_once() {
        let (spec, relay_root) = bootstrap_spec();
        let handle = spec.provisioner().unwrap();
        let input = launch_input(&root());
        let source = LiveProviderRuntimeDispatchSource::from_handle(
            handle,
            input,
            ProcessLimits::new(
                4096,
                4096,
                Duration::from_secs(1),
                Duration::from_millis(100),
                Duration::from_millis(5),
            )
            .unwrap(),
            &"d".repeat(64),
        )
        .unwrap();
        assert_eq!(
            format!("{source:?}"),
            "LiveProviderRuntimeDispatchSource(..)"
        );
        let scheduler = source.into_scheduler();
        assert_eq!(format!("{scheduler:?}"), "LiveProviderRuntimeScheduler(..)");
        let _ = std::fs::remove_dir_all(relay_root);
    }

    #[test]
    fn dispatch_source_rejects_non_digest_adapter_before_exposing_source() {
        let (spec, relay_root) = bootstrap_spec();
        let handle = spec.provisioner().unwrap();
        let error = LiveProviderRuntimeDispatchSource::from_handle(
            handle,
            launch_input(&root()),
            ProcessLimits::new(
                4096,
                4096,
                Duration::from_secs(1),
                Duration::from_millis(100),
                Duration::from_millis(5),
            )
            .unwrap(),
            "not-a-digest",
        )
        .unwrap_err();
        assert_eq!(error, LiveProviderProvisionError::InvalidConfiguration);
        let _ = std::fs::remove_dir_all(relay_root);
    }

    #[test]
    fn scheduler_composition_mints_a_frontend_factory_only_from_runtime_state() {
        let (spec, relay_root) = bootstrap_spec();
        let handle = spec.provisioner().unwrap();
        let input_root = root();
        let input = launch_input(&input_root);
        let scheduler = LiveProviderRuntimeScheduler::new(
            handle,
            input,
            ProcessLimits::new(
                4096,
                4096,
                Duration::from_secs(1),
                Duration::from_millis(100),
                Duration::from_millis(5),
            )
            .unwrap(),
            &"d".repeat(64),
        )
        .unwrap();
        let factory = scheduler.into_factory();
        assert!(matches!(
            factory.acquire(0, false),
            Err(LaunchAuthorityError::InvalidLaunchInput)
        ));
        let _ = std::fs::remove_dir_all(input_root);
        let _ = std::fs::remove_dir_all(relay_root);
    }

    #[test]
    fn bootstrap_rejects_symlinked_relay_root_before_backend_creation() {
        let (spec, relay_root) = bootstrap_spec();
        let linked = root().join("relay-link");
        std::os::unix::fs::symlink(&relay_root, &linked).unwrap();
        let error = LiveProviderBootstrapSpec::from_enrollment(
            spec.config,
            spec.policy,
            spec.allowlist,
            &linked,
            spec.tools[0].clone(),
            spec.tools[1].clone(),
            spec.tools[2].clone(),
            spec.tools[3].clone(),
            spec.tools[4].clone(),
        )
        .unwrap_err();
        assert_eq!(error, LiveProviderBootstrapError::RelayRoot);
        let _ = std::fs::remove_file(linked);
        let _ = std::fs::remove_dir_all(relay_root);
    }

    struct TestResolver {
        fail: bool,
    }

    struct OpaqueCredential;

    impl CredentialInjection for OpaqueCredential {
        fn inject(
            self: Box<Self>,
            _channel: &mut crate::sandbox_credential::SandboxCredentialChannel,
        ) -> Result<(), CredentialInjectionError> {
            drop(self);
            Ok(())
        }
    }

    impl LiveProviderResolver for TestResolver {
        fn resolve_credential(
            &mut self,
            _selection: &LiveProviderRuntimeSelection,
        ) -> Result<Box<dyn CredentialInjection>, LiveProviderResolveError> {
            if self.fail {
                Err(LiveProviderResolveError::CredentialUnavailable)
            } else {
                Ok(Box::new(OpaqueCredential))
            }
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
    fn resolver_seam_returns_only_opaque_capability_and_fails_closed() {
        let service = LiveProviderRuntimeService;
        let selected = selection(
            target(),
            "generation-1",
            "a".repeat(64),
            "b".repeat(64),
            NetworkPolicy::Deny,
        );
        let mut resolver = TestResolver { fail: false };
        assert!(service.resolve_credential(&mut resolver, &selected).is_ok());
        let mut failing = TestResolver { fail: true };
        assert!(matches!(
            service.resolve_credential(&mut failing, &selected),
            Err(LiveProviderResolveError::CredentialUnavailable)
        ));
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

    #[test]
    fn provisioner_rejects_invalid_attempt_before_creating_authority() {
        let (provisioner, relay_root) = provisioner();
        let input = launch_input(&relay_root);
        let limits = input.limits();
        assert!(matches!(
            provisioner.acquire(0, input, limits, &"c".repeat(64), 100),
            Err(LiveProviderProvisionError::InvalidConfiguration)
        ));
        assert!(!relay_root.join("asb-live-generation-1-0.sock").exists());
        let _ = std::fs::remove_dir_all(relay_root);
    }

    #[test]
    fn provisioner_rejects_bad_adapter_identity_without_effects() {
        let (provisioner, relay_root) = provisioner();
        let input = launch_input(&relay_root);
        let limits = input.limits();
        assert!(matches!(
            provisioner.acquire(1, input, limits, "not-a-digest", 100),
            Err(LiveProviderProvisionError::InvalidConfiguration)
        ));
        assert!(!relay_root.join("asb-live-generation-1-1.sock").exists());
        let _ = std::fs::remove_dir_all(relay_root);
    }

    #[test]
    fn provisioner_rejects_deadline_overflow_before_authority() {
        let (provisioner, relay_root) = provisioner();
        let input = launch_input(&relay_root);
        let limits = input.limits();
        assert!(matches!(
            provisioner.acquire(1, input, limits, &"c".repeat(64), u64::MAX),
            Err(LiveProviderProvisionError::InvalidConfiguration)
        ));
        assert!(!relay_root.join("asb-live-generation-1-1.sock").exists());
        let _ = std::fs::remove_dir_all(relay_root);
    }

    #[test]
    fn provisioner_does_not_issue_authority_when_backend_attestation_fails() {
        let (provisioner, relay_root) = provisioner();
        let input = launch_input(&relay_root);
        let limits = input.limits();
        assert!(matches!(
            provisioner.acquire(1, input, limits, &"c".repeat(64), 100),
            Err(LiveProviderProvisionError::BackendUnavailable)
        ));
        assert!(!relay_root.join("asb-live-generation-1-1.sock").exists());
        let _ = std::fs::remove_dir_all(relay_root);
    }

    #[test]
    fn control_claims_reject_invalid_identity_without_authority() {
        assert_eq!(
            LiveProviderControlClaims::new(
                "openrouter".into(),
                "not-a-digest".into(),
                "b".repeat(64),
                1,
                "203.0.113.10:443".parse().unwrap(),
                "c".repeat(64),
                "d".repeat(64),
                "e".repeat(64),
            ),
            Err(LiveProviderEnrollmentError::Unavailable)
        );
    }

    #[test]
    fn control_claims_reject_private_target_without_authority() {
        assert_eq!(
            LiveProviderControlClaims::new(
                "openrouter".into(),
                "a".repeat(64),
                "b".repeat(64),
                1,
                "10.0.0.10:443".parse().unwrap(),
                "c".repeat(64),
                "d".repeat(64),
                "e".repeat(64),
            ),
            Err(LiveProviderEnrollmentError::Unavailable)
        );
    }

    #[test]
    fn control_attestation_binds_issued_operator_identity() {
        let authority = CertificateAuthorityV1::with_trust_anchor_and_endpoint(
            vec![1, 2, 3],
            7,
            "b".repeat(64),
        )
        .unwrap();
        let identity = CertificateIdentityV1 {
            schema_version: 1,
            subject_sha256: "c".repeat(64),
            issuer_sha256: "d".repeat(64),
            certificate_sha256: "e".repeat(64),
            trust_anchor_sha256: format!("{:x}", Sha256::digest([1, 2, 3])),
            generation: 7,
            not_before: 900,
            not_after: 1_100,
            role: "operator".into(),
            endpoint_identity_sha256: "b".repeat(64),
        };
        let issued = authority
            .issue_metadata(&[identity], &"c".repeat(64), 1_000)
            .unwrap();
        let claims = LiveProviderControlClaims::new(
            "openrouter".into(),
            "b".repeat(64),
            "f".repeat(64),
            7,
            "203.0.113.10:443".parse().unwrap(),
            "1".repeat(64),
            "2".repeat(64),
            "3".repeat(64),
        )
        .unwrap();
        let attestation = LiveProviderControlAttestation::from_control(&issued, claims).unwrap();
        assert_eq!(attestation.claims().generation, 7);
        assert_eq!(attestation.chain_sha256().len(), 64);
    }

    fn attested_record() -> LiveProviderControlAttestation {
        let authority = CertificateAuthorityV1::with_trust_anchor_and_endpoint(
            vec![1, 2, 3],
            7,
            "b".repeat(64),
        )
        .unwrap();
        let identity = CertificateIdentityV1 {
            schema_version: 1,
            subject_sha256: "c".repeat(64),
            issuer_sha256: "d".repeat(64),
            certificate_sha256: "e".repeat(64),
            trust_anchor_sha256: format!("{:x}", Sha256::digest([1, 2, 3])),
            generation: 7,
            not_before: 900,
            not_after: 1_100,
            role: "operator".into(),
            endpoint_identity_sha256: "b".repeat(64),
        };
        let issued = authority
            .issue_metadata(&[identity], &"c".repeat(64), 1_000)
            .unwrap();
        let claims = LiveProviderControlClaims::new(
            "openrouter".into(),
            "b".repeat(64),
            "f".repeat(64),
            7,
            "203.0.113.10:443".parse().unwrap(),
            "1".repeat(64),
            "2".repeat(64),
            "3".repeat(64),
        )
        .unwrap();
        LiveProviderControlAttestation::from_control(&issued, claims).unwrap()
    }

    fn control_receipt() -> (IssuedCertificateChainV1, RuntimeEnrollmentReceiptV1) {
        let authority = CertificateAuthorityV1::with_trust_anchor_and_endpoint(
            vec![1, 2, 3],
            7,
            "b".repeat(64),
        )
        .unwrap();
        let mut identity = CertificateIdentityV1 {
            schema_version: 1,
            subject_sha256: "c".repeat(64),
            issuer_sha256: "d".repeat(64),
            certificate_sha256: "e".repeat(64),
            trust_anchor_sha256: format!("{:x}", Sha256::digest([1, 2, 3])),
            generation: 7,
            not_before: 900,
            not_after: 1_100,
            role: "operator".into(),
            endpoint_identity_sha256: "b".repeat(64),
        };
        identity.endpoint_identity_sha256 = "b".repeat(64);
        let issued = authority
            .issue_metadata(&[identity], &"c".repeat(64), 1_000)
            .unwrap();
        let receipt = issued
            .issue_runtime_receipt(
                "openrouter".into(),
                "f".repeat(64),
                "203.0.113.10:443".into(),
                "1".repeat(64),
                "2".repeat(64),
                "3".repeat(64),
                1_000,
                2_000,
            )
            .unwrap();
        (issued, receipt)
    }

    #[test]
    fn runtime_bridge_ingests_control_receipt_once_without_private_paths() {
        let (chain, receipt) = control_receipt();
        let bridge = LiveProviderRuntimeBridge::new();
        let record = bridge
            .ingest_control_receipt(&receipt, &chain, 1_500)
            .unwrap();
        let encoded = String::from_utf8(record.encode().unwrap()).unwrap();
        assert!(!encoded.contains('/'));
        assert!(matches!(
            bridge.ingest_control_receipt(&receipt, &chain, 1_500),
            Err(LiveProviderEnrollmentRecordError::Replay)
        ));
    }

    #[test]
    fn runtime_bridge_materializes_receipt_to_opaque_profile_and_rejects_replay() {
        let (chain, receipt) = control_receipt();
        let bridge = LiveProviderRuntimeBridge::new();
        let profile = bridge
            .materialize_control_receipt_profile(&receipt, &chain, 1_500)
            .unwrap();
        assert_eq!(profile.provider(), "openrouter");
        assert_eq!(profile.generation(), 7);
        assert!(matches!(
            bridge.materialize_control_receipt_profile(&receipt, &chain, 1_500),
            Err(LiveProviderEnrollmentRecordError::Replay)
        ));
    }

    #[test]
    fn runtime_bridge_receipt_materializer_rejects_tamper_before_profile() {
        let (chain, mut receipt) = control_receipt();
        receipt.credential_ref_sha256 = "0".repeat(64);
        let bridge = LiveProviderRuntimeBridge::new();
        assert!(matches!(
            bridge.materialize_control_receipt_profile(&receipt, &chain, 1_500),
            Err(LiveProviderEnrollmentRecordError::AttestationMismatch)
        ));
    }

    #[test]
    fn runtime_bridge_materializes_opaque_profile_once_and_rejects_replay() {
        let attestation = attested_record();
        let record = LiveProviderEnrollmentRecordV1::from_attestation(&attestation, 1_000, 2_000);
        let bridge = LiveProviderRuntimeBridge::new();
        let profile = bridge
            .materialize_profile(&record, &attestation, 1_500)
            .unwrap();
        assert_eq!(profile.provider(), "openrouter");
        assert_eq!(profile.generation(), 7);
        assert!(matches!(
            bridge.materialize_profile(&record, &attestation, 1_500),
            Err(LiveProviderEnrollmentRecordError::Replay)
        ));
    }

    #[test]
    fn authority_profile_mints_handle_only_for_matching_runtime_material() {
        let attestation = attested_record();
        let record = LiveProviderEnrollmentRecordV1::from_attestation(&attestation, 1_000, 2_000);
        let bridge = LiveProviderRuntimeBridge::new();
        let profile = bridge
            .materialize_profile(&record, &attestation, 1_500)
            .unwrap();
        let root = root();
        let selected = ProviderEgressTarget::test_only("203.0.113.10:443".parse().unwrap());
        let allowlist = ProviderEgressAllowlist::new(vec![selected]).unwrap();
        let config = LiveProviderRuntimeConfig::new(
            &root,
            &allowlist,
            selection(
                selected,
                "7",
                "1".repeat(64),
                "f".repeat(64),
                NetworkPolicy::Deny,
            ),
            CpuSet::new(vec![0]).unwrap(),
        )
        .unwrap();
        let policy =
            ProviderEgressPolicy::new("https://openrouter.ai/api/v1", "openrouter.ai").unwrap();
        let pin = |path: &str| ToolPin::new(path.into(), "test".into()).unwrap();
        let handle = profile
            .materialize_handle(
                config,
                policy,
                allowlist,
                &root,
                pin("/bin/true"),
                pin("/bin/true"),
                pin("/bin/true"),
                pin("/bin/true"),
                pin("/bin/true"),
            )
            .unwrap();
        assert!(format!("{handle:?}").contains("LiveProviderRuntimeHandle(..)"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn authority_profile_rejects_mismatched_target_before_bootstrap() {
        let attestation = attested_record();
        let record = LiveProviderEnrollmentRecordV1::from_attestation(&attestation, 1_000, 2_000);
        let bridge = LiveProviderRuntimeBridge::new();
        let profile = bridge
            .materialize_profile(&record, &attestation, 1_500)
            .unwrap();
        let config = config();
        let allowlist = ProviderEgressAllowlist::new(vec![target()]).unwrap();
        let policy =
            ProviderEgressPolicy::new("https://openrouter.ai/api/v1", "openrouter.ai").unwrap();
        let pin = |path: &str| ToolPin::new(path.into(), "test".into()).unwrap();
        assert!(matches!(
            profile.materialize_handle(
                config,
                policy,
                allowlist,
                &root(),
                pin("/bin/true"),
                pin("/bin/true"),
                pin("/bin/true"),
                pin("/bin/true"),
                pin("/bin/true"),
            ),
            Err(LiveProviderProvisionError::InvalidConfiguration)
        ));
    }

    #[test]
    fn runtime_bridge_consumes_bound_control_response_and_rejects_nonce_tamper() {
        let (chain, receipt) = control_receipt();
        let request = RuntimeReceiptRequestV1 {
            schema_version: 1,
            provider: receipt.provider.clone(),
            generation: receipt.generation,
            request_nonce_sha256: receipt.nonce_sha256.clone(),
        };
        let response = RuntimeReceiptResponseV1 {
            schema_version: 1,
            request_nonce_sha256: request.request_nonce_sha256.clone(),
            receipt: receipt.clone(),
        };
        let bridge = LiveProviderRuntimeBridge::new();
        bridge
            .ingest_control_response(&request, &response, &chain, 1_500)
            .unwrap();

        let mut tampered = response;
        tampered.request_nonce_sha256 = "f".repeat(64);
        assert_eq!(
            bridge.ingest_control_response(&request, &tampered, &chain, 1_500),
            Err(LiveProviderEnrollmentRecordError::AttestationMismatch)
        );
    }

    #[test]
    fn runtime_bridge_rejects_tampered_receipt_before_authority() {
        let (chain, mut receipt) = control_receipt();
        receipt.target = "203.0.113.11:443".into();
        let bridge = LiveProviderRuntimeBridge::new();
        assert!(matches!(
            bridge.ingest_control_receipt(&receipt, &chain, 1_500),
            Err(LiveProviderEnrollmentRecordError::AttestationMismatch)
        ));
    }

    #[test]
    fn local_authority_is_runtime_owned_loopback_and_private() {
        let authority = LocalProviderAuthorityProvisioner::provision().unwrap();
        assert_eq!(authority.provider(), "local-deterministic-mock");
        assert_eq!(authority.target().ip(), std::net::Ipv4Addr::LOCALHOST);
        assert_eq!(authority.target().port(), 4_000);
        assert_eq!(authority.credential_reference_sha256().len(), 64);
        assert_eq!(authority.tool_bundle_sha256().len(), 64);
        assert_eq!(
            authority.credential_reference_sha256(),
            LOCAL_PROVIDER_CREDENTIAL_SHA256
        );
        assert_eq!(authority.tool_bundle_sha256(), LOCAL_PROVIDER_TOOL_SHA256);
        assert_eq!(
            std::fs::metadata(authority.lease_root.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(&authority.lease_root)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(&authority.relay_root)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert!(authority.is_active());
        authority.revoke();
        assert!(!authority.is_active());
        let debug = format!("{authority:?}");
        assert!(debug.contains("LocalProviderAuthority(..)"));
        assert!(!debug.contains(authority.credential_reference_sha256()));
        assert!(!debug.contains(authority.lease_root.to_string_lossy().as_ref()));
        assert!(!debug.contains(authority.relay_root.to_string_lossy().as_ref()));
    }

    #[test]
    fn local_authority_generation_is_fenced_and_drop_tears_down_private_roots() {
        let first = LocalProviderAuthorityProvisioner::provision().unwrap();
        let first_generation = first.generation();
        let second = LocalProviderAuthorityProvisioner::provision().unwrap();
        assert!(second.generation() > first_generation);
        assert!(!first.is_active());
        assert!(second.is_active());
        assert_eq!(
            first.credential_reference_sha256(),
            second.credential_reference_sha256()
        );
        let first_root = first.lease_root.parent().unwrap().to_owned();
        drop(first);
        assert!(!first_root.exists());
        let second_root = second.lease_root.parent().unwrap().to_owned();
        drop(second);
        assert!(!second_root.exists());
    }

    #[test]
    fn local_mock_request_is_deterministic_and_secret_free() {
        let authority = LocalProviderAuthorityProvisioner::provision().unwrap();
        let credential = authority.credential_reference_sha256().to_owned();
        let first = authority
            .execute_mock_request(1, LOCAL_PROVIDER_MODEL, &credential, br#"{"prompt":"x"}"#)
            .unwrap();
        let second = authority
            .execute_mock_request(1, LOCAL_PROVIDER_MODEL, &credential, br#"{"prompt":"x"}"#)
            .unwrap();
        assert_eq!(first, second);
        assert_eq!(first.attempt_id(), 1);
        assert_eq!(first.response_sha256().len(), 64);
        let debug = format!("{first:?}");
        assert!(!debug.contains(&credential));
        assert!(!debug.contains("prompt"));
    }

    #[test]
    fn local_mock_rejects_stale_credentials_bad_model_oversize_and_revocation() {
        let authority = LocalProviderAuthorityProvisioner::provision().unwrap();
        let credential = authority.credential_reference_sha256().to_owned();
        assert_eq!(
            authority.execute_mock_request(0, LOCAL_PROVIDER_MODEL, &credential, b"{}"),
            Err(LocalProviderMockError::InvalidAttempt)
        );
        assert_eq!(
            authority.execute_mock_request(1, "moving-alias", &credential, b"{}"),
            Err(LocalProviderMockError::ModelMismatch)
        );
        assert_eq!(
            authority.execute_mock_request(1, LOCAL_PROVIDER_MODEL, &"f".repeat(64), b"{}"),
            Err(LocalProviderMockError::CredentialMismatch)
        );
        assert_eq!(
            authority.execute_mock_request(
                1,
                LOCAL_PROVIDER_MODEL,
                &credential,
                &vec![0_u8; LOCAL_PROVIDER_MAX_REQUEST_BYTES + 1]
            ),
            Err(LocalProviderMockError::RequestTooLarge)
        );
        authority.revoke();
        assert_eq!(
            authority.execute_mock_request(1, LOCAL_PROVIDER_MODEL, &credential, b"{}"),
            Err(LocalProviderMockError::Inactive)
        );
    }

    #[test]
    fn local_mock_target_cannot_be_promoted_to_provider_egress() {
        let authority = LocalProviderAuthorityProvisioner::provision().unwrap();
        assert_eq!(
            ProviderEgressTarget::new(authority.target()),
            Err(crate::provider_egress::ProviderEgressError::InvalidTarget)
        );
    }

    #[test]
    fn local_mock_backend_binds_attempts_and_cancellation_without_live_authority() {
        let backend = LocalProviderMockBackend::provision().unwrap();
        let credential = LOCAL_PROVIDER_CREDENTIAL_SHA256;
        let mut attempt = backend.issue_attempt(7).unwrap();
        assert_eq!(attempt.attempt_id(), 7);
        let response = attempt
            .execute(LOCAL_PROVIDER_MODEL, credential, b"{}")
            .unwrap();
        assert_eq!(response.attempt_id(), 7);
        attempt.cancel();
        assert_eq!(
            attempt.execute(LOCAL_PROVIDER_MODEL, credential, b"{}"),
            Err(LocalProviderMockError::Cancelled)
        );
        backend.revoke();
        assert!(matches!(
            backend.issue_attempt(8),
            Err(LocalProviderMockError::Inactive)
        ));
        let debug = format!("{backend:?} {attempt:?}");
        assert!(!debug.contains(credential));
    }

    #[test]
    fn local_mock_backend_rejects_zero_attempt_before_effect() {
        let backend = LocalProviderMockBackend::provision().unwrap();
        assert!(matches!(
            backend.issue_attempt(0),
            Err(LocalProviderMockError::InvalidAttempt)
        ));
    }

    #[test]
    fn enrollment_record_round_trips_and_binds_control_authority() {
        let attestation = attested_record();
        let record = LiveProviderEnrollmentRecordV1::from_attestation(&attestation, 1_000, 2_000);
        let encoded = record.encode().unwrap();
        let decoded = LiveProviderEnrollmentRecordV1::decode(&encoded).unwrap();
        decoded.validate_against(&attestation, 1_500).unwrap();
        assert!(!String::from_utf8(encoded).unwrap().contains("/"));
    }

    #[test]
    fn enrollment_record_rejects_forgery_unknown_fields_and_stale_time() {
        let attestation = attested_record();
        let mut record =
            LiveProviderEnrollmentRecordV1::from_attestation(&attestation, 1_000, 2_000);
        record.target = "203.0.113.11:443".parse().unwrap();
        assert_eq!(
            record.validate_against(&attestation, 1_500),
            Err(LiveProviderEnrollmentRecordError::AttestationMismatch)
        );
        let stale = LiveProviderEnrollmentRecordV1::from_attestation(&attestation, 1_000, 1_100);
        assert_eq!(
            stale.validate_against(&attestation, 1_101),
            Err(LiveProviderEnrollmentRecordError::Freshness)
        );
        let encoded = br#"{"schema_version":1,"unknown":true}"#;
        assert_eq!(
            LiveProviderEnrollmentRecordV1::decode(encoded),
            Err(LiveProviderEnrollmentRecordError::Malformed)
        );
    }

    #[test]
    fn enrollment_ledger_rejects_replayed_record() {
        let attestation = attested_record();
        let record = LiveProviderEnrollmentRecordV1::from_attestation(&attestation, 1_000, 2_000);
        let ledger = LiveProviderEnrollmentLedger::new();
        ledger.consume(&record, &attestation, 1_500).unwrap();
        assert_eq!(
            ledger.consume(&record, &attestation, 1_500),
            Err(LiveProviderEnrollmentRecordError::Replay)
        );
    }

    #[test]
    fn chain_store_installs_opaque_authenticated_chain_and_rejects_stale_replacement() {
        let (chain, _) = control_receipt();
        let store = RuntimeCertificateChainStore::new();
        store.install(chain.clone()).unwrap();
        assert_eq!(store.chain().unwrap(), chain);
        assert_eq!(
            store.install(chain),
            Err(RuntimeCertificateChainStoreError::StaleGeneration)
        );
    }

    #[test]
    fn empty_chain_store_fails_closed_without_authority() {
        let store = RuntimeCertificateChainStore::new();
        assert_eq!(
            store.chain(),
            Err(RuntimeCertificateChainStoreError::Unavailable)
        );
    }
}
