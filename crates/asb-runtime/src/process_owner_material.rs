// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Runtime-owned process-owner material and the lease-to-dispatch bridge.
//!
//! Public callers can request a lease only from an issuer created by the
//! runtime owner.  In particular, there is intentionally no public
//! constructor for the issuer or store: a public digest record must never be
//! mistaken for authenticated launch authority.

use crate::live_service::LiveProviderRuntimeDispatchSource;
use crate::provider_egress::{
    canonical_alternate_egress_binding, canonical_policy_binding_from_text,
};
use crate::sandbox::SandboxLaunchInput;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fmt;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Version of the process-owner material contract.
pub const PROCESS_OWNER_MATERIAL_SCHEMA_VERSION: u16 = 1;
const MAX_ID_BYTES: usize = 128;
const MAX_LIFETIME_MS: u64 = 15 * 60 * 1_000;
const MAX_TOOLS: usize = 16;
const MAX_PRIVATE_TEXT_BYTES: usize = 4 * 1024;
const MAX_PATH_BYTES: usize = 4 * 1024;

/// Authenticated, secret-free projection of owner material.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessOwnerMaterialContractV1 {
    /// Contract schema.
    pub schema_version: u16,
    /// Runtime owner identity.
    pub owner_id: String,
    /// Provider identity.
    pub provider: String,
    /// Authenticated control endpoint identity.
    pub endpoint_identity_sha256: String,
    /// Authenticated runtime namespace identity.
    pub namespace_sha256: String,
    /// Control-session identity.
    pub control_session_sha256: String,
    /// Enrollment generation.
    pub generation: u64,
    /// Inclusive issue time.
    pub issued_at_unix_ms: u64,
    /// Exclusive expiry time.
    pub expires_at_unix_ms: u64,
    /// Restart fence.
    pub restart_binding_sha256: String,
    /// Cancellation fence.
    pub cancellation_binding_sha256: String,
    /// Revocation fence.
    pub revocation_binding_sha256: String,
    /// Teardown fence.
    pub teardown_binding_sha256: String,
    /// Digest of the private material roots.
    pub private_roots_sha256: String,
    /// Digest of the pinned tool bundle.
    pub tool_bundle_sha256: String,
    /// Digest of the policy and allowlist.
    pub policy_sha256: String,
    /// Digest of the opaque credential capability.
    pub credential_capability_sha256: String,
    /// Digest of the launch target and alternate egress binding.
    pub target_sha256: String,
    /// Digest of the alternate egress binding.
    pub alternate_egress_sha256: String,
    /// Digest of the authenticated lease root.
    pub lease_root_sha256: String,
    /// Digest of the authenticated relay root.
    pub relay_root_sha256: String,
    /// Digest of the launch provenance.
    pub launch_provenance_sha256: String,
    /// Opaque capability reference.
    pub capability_ref_sha256: String,
    /// Digest binding this material to one authenticated enrollment.
    pub enrollment_binding_sha256: String,
}

impl ProcessOwnerMaterialContractV1 {
    /// Validate the public projection without resolving private material.
    pub fn validate(&self) -> Result<(), ProcessOwnerMaterialError> {
        if self.schema_version != PROCESS_OWNER_MATERIAL_SCHEMA_VERSION
            || self.owner_id.is_empty()
            || self.owner_id.len() > MAX_ID_BYTES
            || self.provider.is_empty()
            || self.provider.len() > MAX_ID_BYTES
            || self.generation == 0
            || self.issued_at_unix_ms >= self.expires_at_unix_ms
            || self.expires_at_unix_ms - self.issued_at_unix_ms > MAX_LIFETIME_MS
            || self.digests().iter().any(|value| !valid_digest(value))
        {
            return Err(ProcessOwnerMaterialError::InvalidContract);
        }
        Ok(())
    }

    /// Bind the complete public contract to a stable digest.
    pub fn binding_sha256(&self) -> Result<String, ProcessOwnerMaterialError> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(|_| ProcessOwnerMaterialError::Malformed)?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }

    fn digests(&self) -> [&str; 18] {
        [
            &self.control_session_sha256,
            &self.endpoint_identity_sha256,
            &self.namespace_sha256,
            &self.restart_binding_sha256,
            &self.cancellation_binding_sha256,
            &self.revocation_binding_sha256,
            &self.teardown_binding_sha256,
            &self.private_roots_sha256,
            &self.tool_bundle_sha256,
            &self.policy_sha256,
            &self.credential_capability_sha256,
            &self.target_sha256,
            &self.alternate_egress_sha256,
            &self.lease_root_sha256,
            &self.relay_root_sha256,
            &self.launch_provenance_sha256,
            &self.capability_ref_sha256,
            &self.enrollment_binding_sha256,
        ]
    }
}

/// One executable and adapter binding retained by the authenticated owner.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnerToolProvenance {
    executable: PathBuf,
    executable_sha256: String,
    adapter_id: String,
    adapter_sha256: String,
}

impl OwnerToolProvenance {
    /// Validate the shape of a pinned executable binding.
    pub fn new(
        executable: PathBuf,
        executable_sha256: String,
        adapter_id: String,
        adapter_sha256: String,
    ) -> Result<Self, ProcessOwnerMaterialError> {
        if !absolute_plain_path(&executable)
            || executable.as_os_str().len() > MAX_PATH_BYTES
            || !valid_digest(&executable_sha256)
            || adapter_id.is_empty()
            || adapter_id.len() > MAX_ID_BYTES
            || !valid_digest(&adapter_sha256)
        {
            return Err(ProcessOwnerMaterialError::InvalidPrivateMaterial);
        }
        Ok(Self {
            executable,
            executable_sha256,
            adapter_id,
            adapter_sha256,
        })
    }

    fn validate_file(&self) -> Result<(), ProcessOwnerMaterialError> {
        let metadata = std::fs::symlink_metadata(&self.executable)
            .map_err(|_| ProcessOwnerMaterialError::InvalidExecutable)?;
        if !metadata.file_type().is_file()
            || metadata.permissions().mode() & 0o111 == 0
            || metadata.ino() == 0
        {
            return Err(ProcessOwnerMaterialError::InvalidExecutable);
        }
        let canonical = std::fs::canonicalize(&self.executable)
            .map_err(|_| ProcessOwnerMaterialError::InvalidExecutable)?;
        if canonical != self.executable {
            return Err(ProcessOwnerMaterialError::InvalidExecutable);
        }
        let bytes = std::fs::read(&self.executable)
            .map_err(|_| ProcessOwnerMaterialError::InvalidExecutable)?;
        if format!("{:x}", Sha256::digest(bytes)) != self.executable_sha256 {
            return Err(ProcessOwnerMaterialError::ExecutableDigestMismatch);
        }
        Ok(())
    }

    #[allow(dead_code)]
    fn digest_fields(&self, digest: &mut Sha256) {
        add(digest, &self.executable.to_string_lossy());
        add(digest, &self.executable_sha256);
        add(digest, &self.adapter_id);
        add(digest, &self.adapter_sha256);
    }
}

/// Private material owned by an authenticated runtime owner.
#[derive(Clone, Eq, PartialEq)]
pub struct ProcessOwnerPrivateMaterialV1 {
    private_roots: Vec<PathBuf>,
    namespace: String,
    endpoint_identity_sha256: String,
    pinned_tools: Vec<OwnerToolProvenance>,
    policy: String,
    credential_capability: String,
    target: String,
    alternate_egress: String,
    launch_provenance: String,
}

impl fmt::Debug for ProcessOwnerPrivateMaterialV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProcessOwnerPrivateMaterialV1")
            .field("private_root_count", &self.private_roots.len())
            .field("tool_count", &self.pinned_tools.len())
            .field("credential_capability", &"<opaque>")
            .finish()
    }
}

impl ProcessOwnerPrivateMaterialV1 {
    /// Shape-check private material. The authenticated issuer performs the
    /// filesystem and digest checks before it creates a store.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        private_roots: Vec<PathBuf>,
        namespace: String,
        endpoint_identity_sha256: String,
        pinned_tools: Vec<OwnerToolProvenance>,
        policy: String,
        credential_capability: String,
        target: String,
        alternate_egress: String,
        launch_provenance: String,
    ) -> Result<Self, ProcessOwnerMaterialError> {
        if private_roots.is_empty()
            || private_roots.len() > 8
            || private_roots
                .iter()
                .any(|path| !absolute_plain_path(path) || path.as_os_str().len() > MAX_PATH_BYTES)
            || namespace.is_empty()
            || namespace.len() > MAX_PRIVATE_TEXT_BYTES
            || !valid_digest(&endpoint_identity_sha256)
            || pinned_tools.is_empty()
            || pinned_tools.len() > MAX_TOOLS
            || policy.is_empty()
            || policy.len() > MAX_PRIVATE_TEXT_BYTES
            || !valid_digest(&credential_capability)
            || target.is_empty()
            || target.len() > MAX_PRIVATE_TEXT_BYTES
            || alternate_egress.is_empty()
            || alternate_egress.len() > MAX_PRIVATE_TEXT_BYTES
            || launch_provenance.is_empty()
            || launch_provenance.len() > MAX_PRIVATE_TEXT_BYTES
        {
            return Err(ProcessOwnerMaterialError::InvalidPrivateMaterial);
        }
        Ok(Self {
            private_roots,
            namespace,
            endpoint_identity_sha256,
            pinned_tools,
            policy,
            credential_capability,
            target,
            alternate_egress,
            launch_provenance,
        })
    }

    fn validate_provenance(&self) -> Result<(), ProcessOwnerMaterialError> {
        for root in &self.private_roots {
            let metadata = std::fs::symlink_metadata(root)
                .map_err(|_| ProcessOwnerMaterialError::InvalidPrivateMaterial)?;
            if !metadata.file_type().is_dir()
                || std::fs::canonicalize(root)
                    .map_err(|_| ProcessOwnerMaterialError::InvalidPrivateMaterial)?
                    != *root
            {
                return Err(ProcessOwnerMaterialError::InvalidPrivateMaterial);
            }
        }
        for tool in &self.pinned_tools {
            tool.validate_file()?;
        }
        Ok(())
    }

    fn matches_enrollment_roots(&self, lease_root_sha256: &str, relay_root_sha256: &str) -> bool {
        let roots = self
            .private_roots
            .iter()
            .map(|root| digest_text(&root.to_string_lossy()))
            .collect::<BTreeSet<_>>();
        self.private_roots.len() <= 2
            && roots.contains(lease_root_sha256)
            && roots.contains(relay_root_sha256)
            && roots
                .iter()
                .all(|root| root == lease_root_sha256 || root == relay_root_sha256)
    }

    #[allow(dead_code)]
    fn digests(&self) -> (String, String, String, String, String, String, String) {
        let mut roots = Sha256::new();
        for root in &self.private_roots {
            add(&mut roots, &root.to_string_lossy());
        }
        let mut tools = Sha256::new();
        for tool in &self.pinned_tools {
            tool.digest_fields(&mut tools);
        }
        (
            digest_bytes(roots),
            digest_bytes(tools),
            canonical_policy_binding_from_text(&self.policy),
            digest_text(&self.credential_capability),
            digest_text(&self.target),
            canonical_alternate_egress_binding(&self.alternate_egress),
            digest_text(&self.launch_provenance),
        )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ProcessOwnerMaterialRequestV1 {
    schema_version: u16,
    contract_sha256: String,
    owner_id: String,
    control_session_sha256: String,
    generation: u64,
    request_nonce_sha256: String,
    restart_binding_sha256: String,
    cancellation_binding_sha256: String,
    revocation_binding_sha256: String,
    teardown_binding_sha256: String,
}

impl ProcessOwnerMaterialRequestV1 {
    fn validate(&self) -> Result<(), ProcessOwnerMaterialError> {
        if self.schema_version != PROCESS_OWNER_MATERIAL_SCHEMA_VERSION
            || self.owner_id.is_empty()
            || self.generation == 0
            || [
                self.contract_sha256.as_str(),
                self.control_session_sha256.as_str(),
                self.request_nonce_sha256.as_str(),
                self.restart_binding_sha256.as_str(),
                self.cancellation_binding_sha256.as_str(),
                self.revocation_binding_sha256.as_str(),
                self.teardown_binding_sha256.as_str(),
            ]
            .iter()
            .any(|value| !valid_digest(value))
        {
            return Err(ProcessOwnerMaterialError::InvalidRequest);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OwnerLifecycle {
    Active,
    Cancelled,
    Restarted,
    Revoked,
    TornDown,
}

struct StoreState {
    contract: ProcessOwnerMaterialContractV1,
    material: ProcessOwnerPrivateMaterialV1,
    consumed_nonces: BTreeSet<String>,
    lifecycle: OwnerLifecycle,
}

/// Opaque issuer minted by an authenticated runtime/control owner.
#[allow(dead_code)]
pub struct AuthenticatedProcessOwnerIssuer {
    state: Arc<Mutex<StoreState>>,
}

impl fmt::Debug for AuthenticatedProcessOwnerIssuer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthenticatedProcessOwnerIssuer(<private>)")
    }
}

/// Owner-side material source. It has no public constructor by design.
#[derive(Clone)]
pub struct ProcessOwnerMaterialStore {
    state: Arc<Mutex<StoreState>>,
}

impl fmt::Debug for ProcessOwnerMaterialStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ProcessOwnerMaterialStore(<private>)")
    }
}

/// One-shot opaque capability issued by the authenticated owner.
pub struct ProcessOwnerMaterialCapability {
    store: Arc<Mutex<StoreState>>,
    request: ProcessOwnerMaterialRequestV1,
    contract: ProcessOwnerMaterialContractV1,
}

impl fmt::Debug for ProcessOwnerMaterialCapability {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProcessOwnerMaterialCapability")
            .field("owner_id", &self.contract.owner_id)
            .field("generation", &self.contract.generation)
            .finish_non_exhaustive()
    }
}

/// Lease containing private material only after authenticated issuance.
pub struct ProcessOwnerMaterialLease {
    store: Arc<Mutex<StoreState>>,
    request: ProcessOwnerMaterialRequestV1,
    material: ProcessOwnerPrivateMaterialV1,
    contract: ProcessOwnerMaterialContractV1,
}

impl fmt::Debug for ProcessOwnerMaterialLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProcessOwnerMaterialLease")
            .field("owner_id", &self.contract.owner_id)
            .field("generation", &self.contract.generation)
            .finish_non_exhaustive()
    }
}

impl ProcessOwnerMaterialLease {
    /// Return only the authenticated public projection.
    pub fn contract(&self) -> &ProcessOwnerMaterialContractV1 {
        &self.contract
    }

    /// Verify that a launch request names the exact adapter executable and
    /// adapter material authenticated by this owner lease.
    pub(crate) fn validate_dispatch_input(
        &self,
        input: &SandboxLaunchInput,
        adapter_sha256: &str,
    ) -> Result<(), ProcessOwnerMaterialError> {
        if input.spec().network_policy() != crate::sandbox::NetworkPolicy::Deny
            || !valid_digest(adapter_sha256)
            || !self.material.pinned_tools.iter().any(|tool| {
                tool.executable.as_path() == Path::new(input.spec().program())
                    && tool.adapter_sha256 == adapter_sha256
            })
        {
            return Err(ProcessOwnerMaterialError::SourceBindingMismatch);
        }
        Ok(())
    }

    /// Verify a runtime-minted dispatch source against this authenticated
    /// lease before ordinary run/sweep dispatch consumes it.
    pub(crate) fn validate_dispatch_source(
        &self,
        source: &LiveProviderRuntimeDispatchSource,
    ) -> Result<(), ProcessOwnerMaterialError> {
        source
            .validate_process_owner_material(&self.contract)
            .map_err(|_| ProcessOwnerMaterialError::SourceBindingMismatch)
    }

    fn active(&self, now_unix_ms: u64) -> Result<(), ProcessOwnerMaterialError> {
        let state = self
            .store
            .lock()
            .map_err(|_| ProcessOwnerMaterialError::StateUnavailable)?;
        validate_request(&state.contract, &self.request)?;
        if state.contract != self.contract
            || state.lifecycle != OwnerLifecycle::Active
            || now_unix_ms < state.contract.issued_at_unix_ms
            || now_unix_ms >= state.contract.expires_at_unix_ms
        {
            return Err(ProcessOwnerMaterialError::Unavailable);
        }
        self.material.validate_provenance()?;
        Ok(())
    }

    /// Consume this lease together with an opaque runtime-minted dispatch
    /// source. The source is never available after the owner is fenced.
    pub fn into_dispatch_source(
        self,
        source: LiveProviderRuntimeDispatchSource,
        now_unix_ms: u64,
    ) -> Result<ProcessOwnerMaterialDispatchBridge, ProcessOwnerMaterialError> {
        self.active(now_unix_ms)?;
        if source.owner_enrollment_binding_sha256() != self.contract.enrollment_binding_sha256 {
            return Err(ProcessOwnerMaterialError::SourceBindingMismatch);
        }
        let store = Arc::clone(&self.store);
        let request = self.request.clone();
        let contract = self.contract.clone();
        let source = source
            .bind_process_owner_fence(Arc::new(move || {
                let Ok(state) = store.lock() else {
                    return false;
                };
                let now = current_unix_ms();
                validate_request(&state.contract, &request).is_ok()
                    && state.contract == contract
                    && state.lifecycle == OwnerLifecycle::Active
                    && state.contract.issued_at_unix_ms <= now
                    && now < state.contract.expires_at_unix_ms
            }))
            .map_err(|_| ProcessOwnerMaterialError::SourceBindingMismatch)?;
        Ok(ProcessOwnerMaterialDispatchBridge {
            lease: self,
            source: Some(source),
        })
    }
}

/// Runtime/control bridge consumed by ordinary run/sweep dispatch.
pub struct ProcessOwnerMaterialDispatchBridge {
    lease: ProcessOwnerMaterialLease,
    source: Option<LiveProviderRuntimeDispatchSource>,
}

impl fmt::Debug for ProcessOwnerMaterialDispatchBridge {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ProcessOwnerMaterialDispatchBridge(..)")
    }
}

impl ProcessOwnerMaterialDispatchBridge {
    /// Consume the one-shot source after rechecking expiry/revoke/teardown.
    pub fn into_dispatch_source(
        mut self,
        now_unix_ms: u64,
    ) -> Result<LiveProviderRuntimeDispatchSource, ProcessOwnerMaterialError> {
        self.lease.active(now_unix_ms)?;
        self.source
            .take()
            .ok_or(ProcessOwnerMaterialError::ReplayOrExpired)
    }

    /// Authenticated owner projection for binding control lifecycle events.
    pub fn contract(&self) -> &ProcessOwnerMaterialContractV1 {
        self.lease.contract()
    }
}

/// Lifecycle operations that fence an owner-issued lease.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessOwnerMaterialOperation {
    /// Cancel current work.
    Cancel,
    /// Fence the generation for restart.
    Restart,
    /// Revoke provider authority.
    Revoke,
    /// Teardown all owner material.
    Teardown,
}

impl ProcessOwnerMaterialStore {
    /// Construct a store only from an opaque runtime-owner issuer.
    #[allow(dead_code)]
    fn from_authenticated_issuer(
        issuer: AuthenticatedProcessOwnerIssuer,
    ) -> Result<Self, ProcessOwnerMaterialError> {
        let state = issuer
            .state
            .lock()
            .map_err(|_| ProcessOwnerMaterialError::StateUnavailable)?;
        state.contract.validate()?;
        state.material.validate_provenance()?;
        let (roots, tools, policy, credential, target, alternate_egress, launch) =
            state.material.digests();
        if state.contract.private_roots_sha256 != roots
            || state.contract.tool_bundle_sha256 != tools
            || state.contract.policy_sha256 != policy
            || state.contract.credential_capability_sha256 != credential
            || state.contract.target_sha256 != target
            || state.contract.alternate_egress_sha256 != alternate_egress
            || state.contract.launch_provenance_sha256 != launch
        {
            return Err(ProcessOwnerMaterialError::MaterialMismatch);
        }
        drop(state);
        Ok(Self {
            state: issuer.state,
        })
    }

    /// Construct a store only after the authenticated bootstrap has matched
    /// every identity carried by the private material and public contract.
    pub(crate) fn from_runtime_enrollment(
        contract: ProcessOwnerMaterialContractV1,
        material: ProcessOwnerPrivateMaterialV1,
        enrollment: &ProcessOwnerEnrollmentBinding,
    ) -> Result<Self, ProcessOwnerMaterialError> {
        contract.validate()?;
        if contract.owner_id != enrollment.owner_id
            || contract.provider != enrollment.provider
            || contract.endpoint_identity_sha256 != enrollment.endpoint_identity_sha256
            || contract.namespace_sha256 != enrollment.namespace_sha256
            || contract.control_session_sha256 != enrollment.control_session_sha256
            || contract.generation != enrollment.generation
            || contract.expires_at_unix_ms > enrollment.expires_at_unix_ms
            || contract.restart_binding_sha256 != enrollment.restart_binding_sha256
            || contract.cancellation_binding_sha256 != enrollment.cancellation_binding_sha256
            || contract.revocation_binding_sha256 != enrollment.revocation_binding_sha256
            || contract.teardown_binding_sha256 != enrollment.teardown_binding_sha256
            || contract.credential_capability_sha256 != enrollment.credential_ref_sha256
            || contract.target_sha256 != enrollment.target_sha256
            || contract.alternate_egress_sha256 != enrollment.alternate_egress_sha256
            || contract.policy_sha256 != enrollment.policy_sha256
            || contract.tool_bundle_sha256 != enrollment.tool_bundle_sha256
            || contract.lease_root_sha256 != enrollment.lease_root_sha256
            || contract.relay_root_sha256 != enrollment.relay_root_sha256
            || contract.enrollment_binding_sha256 != enrollment.binding_sha256
            || material.endpoint_identity_sha256 != enrollment.endpoint_identity_sha256
            || digest_text(&material.namespace) != enrollment.namespace_sha256
            || !material.matches_enrollment_roots(
                &enrollment.lease_root_sha256,
                &enrollment.relay_root_sha256,
            )
        {
            return Err(ProcessOwnerMaterialError::MaterialMismatch);
        }
        let issuer = AuthenticatedProcessOwnerIssuer {
            state: Arc::new(Mutex::new(StoreState {
                contract,
                material,
                consumed_nonces: BTreeSet::new(),
                lifecycle: OwnerLifecycle::Active,
            })),
        };
        Self::from_authenticated_issuer(issuer)
    }

    fn request(
        &self,
        nonce: String,
    ) -> Result<ProcessOwnerMaterialRequestV1, ProcessOwnerMaterialError> {
        let state = self
            .state
            .lock()
            .map_err(|_| ProcessOwnerMaterialError::StateUnavailable)?;
        let request = ProcessOwnerMaterialRequestV1 {
            schema_version: PROCESS_OWNER_MATERIAL_SCHEMA_VERSION,
            contract_sha256: state.contract.binding_sha256()?,
            owner_id: state.contract.owner_id.clone(),
            control_session_sha256: state.contract.control_session_sha256.clone(),
            generation: state.contract.generation,
            request_nonce_sha256: nonce,
            restart_binding_sha256: state.contract.restart_binding_sha256.clone(),
            cancellation_binding_sha256: state.contract.cancellation_binding_sha256.clone(),
            revocation_binding_sha256: state.contract.revocation_binding_sha256.clone(),
            teardown_binding_sha256: state.contract.teardown_binding_sha256.clone(),
        };
        request.validate()?;
        Ok(request)
    }

    fn issue(
        &self,
        request: ProcessOwnerMaterialRequestV1,
        now: u64,
    ) -> Result<ProcessOwnerMaterialCapability, ProcessOwnerMaterialError> {
        request.validate()?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| ProcessOwnerMaterialError::StateUnavailable)?;
        validate_request(&state.contract, &request)?;
        if state.lifecycle != OwnerLifecycle::Active
            || now < state.contract.issued_at_unix_ms
            || now >= state.contract.expires_at_unix_ms
            || !state
                .consumed_nonces
                .insert(request.request_nonce_sha256.clone())
        {
            return Err(ProcessOwnerMaterialError::ReplayOrExpired);
        }
        Ok(ProcessOwnerMaterialCapability {
            store: Arc::clone(&self.state),
            request,
            contract: state.contract.clone(),
        })
    }

    /// Fence the owner generation before cancel, restart, revoke or teardown.
    pub fn transition(
        &self,
        capability: &ProcessOwnerMaterialCapability,
        operation: ProcessOwnerMaterialOperation,
        now: u64,
    ) -> Result<(), ProcessOwnerMaterialError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| ProcessOwnerMaterialError::StateUnavailable)?;
        validate_request(&state.contract, &capability.request)?;
        if state.contract != capability.contract || now >= state.contract.expires_at_unix_ms {
            return Err(ProcessOwnerMaterialError::Unavailable);
        }
        state.lifecycle = match (state.lifecycle, operation) {
            (OwnerLifecycle::Active, ProcessOwnerMaterialOperation::Cancel) => {
                OwnerLifecycle::Cancelled
            }
            (OwnerLifecycle::Active, ProcessOwnerMaterialOperation::Restart) => {
                OwnerLifecycle::Restarted
            }
            (OwnerLifecycle::Active, ProcessOwnerMaterialOperation::Revoke) => {
                OwnerLifecycle::Revoked
            }
            (
                OwnerLifecycle::Active
                | OwnerLifecycle::Cancelled
                | OwnerLifecycle::Restarted
                | OwnerLifecycle::Revoked,
                ProcessOwnerMaterialOperation::Teardown,
            ) => OwnerLifecycle::TornDown,
            _ => return Err(ProcessOwnerMaterialError::InvalidTransition),
        };
        Ok(())
    }
}

/// Runtime-only identity binding minted from one authenticated enrollment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProcessOwnerEnrollmentBinding {
    pub(crate) owner_id: String,
    pub(crate) provider: String,
    pub(crate) endpoint_identity_sha256: String,
    pub(crate) namespace_sha256: String,
    pub(crate) control_session_sha256: String,
    pub(crate) generation: u64,
    pub(crate) expires_at_unix_ms: u64,
    pub(crate) restart_binding_sha256: String,
    pub(crate) cancellation_binding_sha256: String,
    pub(crate) revocation_binding_sha256: String,
    pub(crate) teardown_binding_sha256: String,
    pub(crate) credential_ref_sha256: String,
    pub(crate) target_sha256: String,
    pub(crate) alternate_egress_sha256: String,
    pub(crate) policy_sha256: String,
    pub(crate) tool_bundle_sha256: String,
    pub(crate) lease_root_sha256: String,
    pub(crate) relay_root_sha256: String,
    pub(crate) binding_sha256: String,
}

impl ProcessOwnerMaterialCapability {
    /// Materialize one private lease while the authenticated owner is active.
    pub fn materialize(
        &self,
        now: u64,
    ) -> Result<ProcessOwnerMaterialLease, ProcessOwnerMaterialError> {
        let state = self
            .store
            .lock()
            .map_err(|_| ProcessOwnerMaterialError::StateUnavailable)?;
        validate_request(&state.contract, &self.request)?;
        if state.contract != self.contract
            || state.lifecycle != OwnerLifecycle::Active
            || now < state.contract.issued_at_unix_ms
            || now >= state.contract.expires_at_unix_ms
        {
            return Err(ProcessOwnerMaterialError::Unavailable);
        }
        Ok(ProcessOwnerMaterialLease {
            store: Arc::clone(&self.store),
            request: self.request.clone(),
            material: state.material.clone(),
            contract: state.contract.clone(),
        })
    }
}

/// Ordinary runtime/control caller. It accepts only a nonce; all authority
/// fields originate in the authenticated issuer.
#[derive(Clone)]
pub struct RuntimeProcessOwnerMaterialCaller {
    store: ProcessOwnerMaterialStore,
}

impl fmt::Debug for RuntimeProcessOwnerMaterialCaller {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RuntimeProcessOwnerMaterialCaller(<owner>)")
    }
}

impl RuntimeProcessOwnerMaterialCaller {
    /// Bind a caller to the runtime-owned authenticated source.
    #[allow(dead_code)]
    pub(crate) fn new(store: ProcessOwnerMaterialStore) -> Self {
        Self { store }
    }

    /// Issue one capability and lease for a fresh nonce.
    pub fn acquire(
        &self,
        nonce: String,
        now: u64,
    ) -> Result<
        (ProcessOwnerMaterialCapability, ProcessOwnerMaterialLease),
        ProcessOwnerMaterialError,
    > {
        let capability = self.store.issue(self.store.request(nonce)?, now)?;
        let lease = capability.materialize(now)?;
        Ok((capability, lease))
    }

    /// Fence one issued capability.
    pub fn fence(
        &self,
        capability: &ProcessOwnerMaterialCapability,
        operation: ProcessOwnerMaterialOperation,
        now: u64,
    ) -> Result<(), ProcessOwnerMaterialError> {
        self.store.transition(capability, operation, now)
    }
}

/// Fail-closed owner-material failures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessOwnerMaterialError {
    /// Public contract is malformed.
    InvalidContract,
    /// Private material shape is malformed.
    InvalidPrivateMaterial,
    /// Request binding is malformed or mismatched.
    InvalidRequest,
    /// Private material does not match authenticated digests.
    MaterialMismatch,
    /// Executable is absent, a symlink, non-regular or non-executable.
    InvalidExecutable,
    /// Executable bytes differ from the pinned digest.
    ExecutableDigestMismatch,
    /// Replay or expiry.
    ReplayOrExpired,
    /// Owner has been fenced.
    Unavailable,
    /// Lifecycle transition is invalid.
    InvalidTransition,
    /// Runtime state is unavailable.
    StateUnavailable,
    /// Dispatch source was not minted for this authenticated enrollment.
    SourceBindingMismatch,
    /// Bounded encoding failed.
    Malformed,
}

fn validate_request(
    contract: &ProcessOwnerMaterialContractV1,
    request: &ProcessOwnerMaterialRequestV1,
) -> Result<(), ProcessOwnerMaterialError> {
    request.validate()?;
    if request.owner_id != contract.owner_id
        || request.generation != contract.generation
        || request.control_session_sha256 != contract.control_session_sha256
        || request.restart_binding_sha256 != contract.restart_binding_sha256
        || request.cancellation_binding_sha256 != contract.cancellation_binding_sha256
        || request.revocation_binding_sha256 != contract.revocation_binding_sha256
        || request.teardown_binding_sha256 != contract.teardown_binding_sha256
        || request.contract_sha256 != contract.binding_sha256()?
    {
        return Err(ProcessOwnerMaterialError::InvalidRequest);
    }
    Ok(())
}

fn absolute_plain_path(path: &Path) -> bool {
    path.is_absolute()
        && path
            .components()
            .all(|component| !matches!(component, Component::ParentDir | Component::CurDir))
}

fn add(digest: &mut Sha256, value: &str) {
    digest.update((value.len() as u64).to_le_bytes());
    digest.update(value.as_bytes());
}

fn digest_bytes(digest: Sha256) -> String {
    format!("{:x}", digest.finalize())
}

fn digest_text(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn current_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| duration.as_millis().try_into().ok())
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider_egress::canonical_policy_binding;
    use crate::sandbox::{CpuSet, NetworkPolicy, Resources, SandboxSpec};
    use std::collections::BTreeMap;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn fixture() -> (
        ProcessOwnerMaterialContractV1,
        ProcessOwnerPrivateMaterialV1,
        PathBuf,
    ) {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "asb-owner-material-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let tools = root.join("tools");
        std::fs::create_dir_all(&tools).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::set_permissions(&tools, std::fs::Permissions::from_mode(0o700)).unwrap();
        let executable = tools.join("adapter");
        let bytes = b"provider-adapter-fixture-v1\n";
        std::fs::write(&executable, bytes).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let executable_sha256 = format!("{:x}", Sha256::digest(bytes));
        let tool = OwnerToolProvenance::new(
            executable.clone(),
            executable_sha256,
            "adapter-v1".into(),
            "b".repeat(64),
        )
        .unwrap();
        let material = ProcessOwnerPrivateMaterialV1::new(
            vec![root.clone()],
            "net:[owner]".into(),
            "b".repeat(64),
            vec![tool],
            "https://provider.example/v1".into(),
            "c".repeat(64),
            "198.51.100.10:443".into(),
            "198.51.100.11:443".into(),
            "launch-provenance-v1".into(),
        )
        .unwrap();
        let (roots, tools, policy, credential, target, alternate_egress, launch) =
            material.digests();
        let contract = ProcessOwnerMaterialContractV1 {
            schema_version: 1,
            owner_id: "owner-1".into(),
            provider: "provider-v1".into(),
            endpoint_identity_sha256: "b".repeat(64),
            namespace_sha256: digest_text("net:[owner]"),
            control_session_sha256: "1".repeat(64),
            generation: 7,
            issued_at_unix_ms: 1_000,
            expires_at_unix_ms: 2_000,
            restart_binding_sha256: "2".repeat(64),
            cancellation_binding_sha256: "3".repeat(64),
            revocation_binding_sha256: "4".repeat(64),
            teardown_binding_sha256: "5".repeat(64),
            private_roots_sha256: roots,
            tool_bundle_sha256: tools,
            policy_sha256: policy,
            credential_capability_sha256: credential,
            target_sha256: target,
            alternate_egress_sha256: alternate_egress,
            lease_root_sha256: digest_text(&root.to_string_lossy()),
            relay_root_sha256: digest_text(&root.to_string_lossy()),
            launch_provenance_sha256: launch,
            capability_ref_sha256: "6".repeat(64),
            enrollment_binding_sha256: "7".repeat(64),
        };
        (contract, material, root)
    }

    #[test]
    fn policy_digest_accepts_an_already_authenticated_endpoint_identity() {
        let (_, mut material, root) = fixture();
        material.policy = "b".repeat(64);
        let (_, _, policy, _, _, _, _) = material.digests();
        assert_eq!(policy, canonical_policy_binding("b".repeat(64).as_str()));
        let _ = std::fs::remove_dir_all(root);
    }

    fn test_store(
        contract: ProcessOwnerMaterialContractV1,
        material: ProcessOwnerPrivateMaterialV1,
    ) -> Result<ProcessOwnerMaterialStore, ProcessOwnerMaterialError> {
        let issuer = AuthenticatedProcessOwnerIssuer {
            state: Arc::new(Mutex::new(StoreState {
                contract,
                material,
                consumed_nonces: BTreeSet::new(),
                lifecycle: OwnerLifecycle::Active,
            })),
        };
        ProcessOwnerMaterialStore::from_authenticated_issuer(issuer)
    }

    #[test]
    fn authenticated_issuer_consumes_nonce_and_fences_lifecycle() {
        let (contract, material, root) = fixture();
        let store = test_store(contract, material).unwrap();
        let caller = RuntimeProcessOwnerMaterialCaller::new(store.clone());
        let (capability, lease) = caller.acquire("7".repeat(64), 1_100).unwrap();
        assert_eq!(lease.contract().generation, 7);
        assert!(matches!(
            caller.acquire("7".repeat(64), 1_101),
            Err(ProcessOwnerMaterialError::ReplayOrExpired)
        ));
        caller
            .fence(&capability, ProcessOwnerMaterialOperation::Revoke, 1_200)
            .unwrap();
        assert!(matches!(
            lease.active(1_201),
            Err(ProcessOwnerMaterialError::Unavailable)
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn dispatch_input_requires_the_pinned_executable_and_adapter_digest() {
        let (contract, material, root) = fixture();
        let adapter = material.pinned_tools[0].executable.clone();
        let adapter_sha256 = material.pinned_tools[0].adapter_sha256.clone();
        let store = test_store(contract, material).unwrap();
        let caller = RuntimeProcessOwnerMaterialCaller::new(store);
        let (_, lease) = caller.acquire("8".repeat(64), 1_100).unwrap();
        let resources =
            Resources::new(64 * 1024 * 1024, 16, 100, CpuSet::new(vec![0]).unwrap()).unwrap();
        let input = SandboxLaunchInput::new(
            SandboxSpec::new(
                &root,
                PathBuf::from("."),
                adapter.to_string_lossy().into_owned(),
                Vec::new(),
                BTreeMap::new(),
                resources,
                NetworkPolicy::Deny,
            )
            .unwrap(),
            crate::ProcessLimits::default(),
        )
        .unwrap();
        assert!(
            lease
                .validate_dispatch_input(&input, &adapter_sha256)
                .is_ok()
        );
        assert_eq!(
            lease.validate_dispatch_input(&input, &"c".repeat(64)),
            Err(ProcessOwnerMaterialError::SourceBindingMismatch)
        );
        let wrong_program = SandboxLaunchInput::new(
            SandboxSpec::new(
                &root,
                PathBuf::from("."),
                "/bin/true".into(),
                Vec::new(),
                BTreeMap::new(),
                Resources::new(64 * 1024 * 1024, 16, 100, CpuSet::new(vec![0]).unwrap()).unwrap(),
                NetworkPolicy::Deny,
            )
            .unwrap(),
            crate::ProcessLimits::default(),
        )
        .unwrap();
        assert_eq!(
            lease.validate_dispatch_input(&wrong_program, &adapter_sha256),
            Err(ProcessOwnerMaterialError::SourceBindingMismatch)
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn provenance_rejects_hash_drift_missing_nonexec_and_symlink() {
        let (contract, material, root) = fixture();
        let mut wrong = material.clone();
        wrong.pinned_tools[0].executable_sha256 = "a".repeat(64);
        assert!(matches!(
            test_store(contract.clone(), wrong),
            Err(ProcessOwnerMaterialError::ExecutableDigestMismatch)
        ));
        let mut missing = material.clone();
        missing.pinned_tools[0].executable = root.join("tools/missing");
        assert!(matches!(
            test_store(contract.clone(), missing),
            Err(ProcessOwnerMaterialError::InvalidExecutable)
                | Err(ProcessOwnerMaterialError::MaterialMismatch)
        ));
        let mut symlink = material;
        let target = symlink.pinned_tools[0].executable.clone();
        let link = root.join("tools/link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        symlink.pinned_tools[0].executable = link;
        assert!(matches!(
            test_store(contract, symlink),
            Err(ProcessOwnerMaterialError::InvalidExecutable)
                | Err(ProcessOwnerMaterialError::MaterialMismatch)
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn contract_unknown_fields_and_expiry_fail_closed() {
        let (mut contract, material, root) = fixture();
        let mut value = serde_json::to_value(&contract).unwrap();
        value["authority"] = serde_json::Value::String("caller".into());
        assert!(serde_json::from_value::<ProcessOwnerMaterialContractV1>(value).is_err());
        contract.expires_at_unix_ms = contract.issued_at_unix_ms + MAX_LIFETIME_MS + 1;
        assert!(matches!(
            test_store(contract, material),
            Err(ProcessOwnerMaterialError::InvalidContract)
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn shape_rejections_and_debug_projections_are_bounded() {
        let (contract, material, root) = fixture();
        assert!(
            OwnerToolProvenance::new(
                PathBuf::from("relative/tool"),
                "a".repeat(64),
                "adapter-v1".into(),
                "b".repeat(64),
            )
            .is_err()
        );
        assert!(
            ProcessOwnerPrivateMaterialV1::new(
                Vec::new(),
                String::new(),
                String::new(),
                Vec::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
            )
            .is_err()
        );

        let store = test_store(contract, material).unwrap();
        let caller = RuntimeProcessOwnerMaterialCaller::new(store.clone());
        let (capability, lease) = caller.acquire("8".repeat(64), 1_100).unwrap();
        assert!(format!("{store:?}").contains("<private>"));
        assert!(format!("{caller:?}").contains("<owner>"));
        assert!(format!("{capability:?}").contains("owner-1"));
        assert!(format!("{lease:?}").contains("owner-1"));
        assert!(format!("{:?}", lease.material).contains("<opaque>"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn lifecycle_transitions_and_time_fences_are_fail_closed() {
        let (contract, material, root) = fixture();
        let mut mismatched = contract.clone();
        mismatched.private_roots_sha256 = "f".repeat(64);
        assert!(matches!(
            test_store(mismatched, material.clone()),
            Err(ProcessOwnerMaterialError::MaterialMismatch)
        ));

        let store = test_store(contract, material).unwrap();
        let caller = RuntimeProcessOwnerMaterialCaller::new(store.clone());
        assert!(matches!(
            caller.acquire("9".repeat(64), 999),
            Err(ProcessOwnerMaterialError::ReplayOrExpired)
        ));
        let (capability, lease) = caller.acquire("a".repeat(64), 1_100).unwrap();
        assert_eq!(
            lease.active(999),
            Err(ProcessOwnerMaterialError::Unavailable)
        );
        assert!(matches!(
            capability.materialize(2_000),
            Err(ProcessOwnerMaterialError::Unavailable)
        ));
        caller
            .fence(&capability, ProcessOwnerMaterialOperation::Cancel, 1_200)
            .unwrap();
        assert_eq!(
            caller.fence(&capability, ProcessOwnerMaterialOperation::Revoke, 1_201),
            Err(ProcessOwnerMaterialError::InvalidTransition)
        );
        let _ = std::fs::remove_dir_all(root);

        let (contract, material, root) = fixture();
        let store = test_store(contract, material).unwrap();
        let caller = RuntimeProcessOwnerMaterialCaller::new(store);
        let (capability, _) = caller.acquire("b".repeat(64), 1_100).unwrap();
        caller
            .fence(&capability, ProcessOwnerMaterialOperation::Restart, 1_200)
            .unwrap();
        caller
            .fence(&capability, ProcessOwnerMaterialOperation::Teardown, 1_201)
            .unwrap();
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn enrollment_binding_rejects_self_consistent_caller_material() {
        let (contract, material, root) = fixture();
        let enrollment = ProcessOwnerEnrollmentBinding {
            owner_id: contract.owner_id.clone(),
            provider: contract.provider.clone(),
            endpoint_identity_sha256: contract.endpoint_identity_sha256.clone(),
            namespace_sha256: contract.namespace_sha256.clone(),
            control_session_sha256: contract.control_session_sha256.clone(),
            generation: contract.generation,
            expires_at_unix_ms: contract.expires_at_unix_ms,
            restart_binding_sha256: contract.restart_binding_sha256.clone(),
            cancellation_binding_sha256: contract.cancellation_binding_sha256.clone(),
            revocation_binding_sha256: contract.revocation_binding_sha256.clone(),
            teardown_binding_sha256: contract.teardown_binding_sha256.clone(),
            credential_ref_sha256: contract.credential_capability_sha256.clone(),
            target_sha256: contract.target_sha256.clone(),
            alternate_egress_sha256: contract.alternate_egress_sha256.clone(),
            policy_sha256: contract.policy_sha256.clone(),
            tool_bundle_sha256: contract.tool_bundle_sha256.clone(),
            lease_root_sha256: contract.lease_root_sha256.clone(),
            relay_root_sha256: contract.relay_root_sha256.clone(),
            binding_sha256: contract.enrollment_binding_sha256.clone(),
        };
        assert!(
            ProcessOwnerMaterialStore::from_runtime_enrollment(
                contract.clone(),
                material.clone(),
                &enrollment,
            )
            .is_ok()
        );

        for mutate in [
            |value: &mut ProcessOwnerMaterialContractV1| value.owner_id = "forged-owner".into(),
            |value: &mut ProcessOwnerMaterialContractV1| {
                value.endpoint_identity_sha256 = "8".repeat(64)
            },
            |value: &mut ProcessOwnerMaterialContractV1| value.namespace_sha256 = "8".repeat(64),
            |value: &mut ProcessOwnerMaterialContractV1| {
                value.credential_capability_sha256 = "8".repeat(64)
            },
            |value: &mut ProcessOwnerMaterialContractV1| value.lease_root_sha256 = "8".repeat(64),
            |value: &mut ProcessOwnerMaterialContractV1| value.relay_root_sha256 = "8".repeat(64),
            |value: &mut ProcessOwnerMaterialContractV1| value.target_sha256 = "8".repeat(64),
            |value: &mut ProcessOwnerMaterialContractV1| {
                value.alternate_egress_sha256 = "8".repeat(64)
            },
            |value: &mut ProcessOwnerMaterialContractV1| value.tool_bundle_sha256 = "8".repeat(64),
            |value: &mut ProcessOwnerMaterialContractV1| {
                value.restart_binding_sha256 = "8".repeat(64)
            },
            |value: &mut ProcessOwnerMaterialContractV1| {
                value.cancellation_binding_sha256 = "8".repeat(64)
            },
            |value: &mut ProcessOwnerMaterialContractV1| {
                value.revocation_binding_sha256 = "8".repeat(64)
            },
            |value: &mut ProcessOwnerMaterialContractV1| {
                value.teardown_binding_sha256 = "8".repeat(64)
            },
            |value: &mut ProcessOwnerMaterialContractV1| {
                value.enrollment_binding_sha256 = "8".repeat(64)
            },
        ] {
            let mut forged = contract.clone();
            mutate(&mut forged);
            assert!(matches!(
                ProcessOwnerMaterialStore::from_runtime_enrollment(
                    forged,
                    material.clone(),
                    &enrollment,
                ),
                Err(ProcessOwnerMaterialError::MaterialMismatch)
            ));
        }
        let _ = std::fs::remove_dir_all(root);
    }
}
