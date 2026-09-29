// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Runtime-owned process-owner material and the lease-to-dispatch bridge.
//!
//! Public callers can request a lease only from an issuer created by the
//! runtime owner.  In particular, there is intentionally no public
//! constructor for the issuer or store: a public digest record must never be
//! mistaken for authenticated launch authority.

use crate::live_service::LiveProviderRuntimeDispatchSource;
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
    /// Digest of the launch provenance.
    pub launch_provenance_sha256: String,
    /// Opaque capability reference.
    pub capability_ref_sha256: String,
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

    fn digests(&self) -> [&str; 12] {
        [
            &self.control_session_sha256,
            &self.restart_binding_sha256,
            &self.cancellation_binding_sha256,
            &self.revocation_binding_sha256,
            &self.teardown_binding_sha256,
            &self.private_roots_sha256,
            &self.tool_bundle_sha256,
            &self.policy_sha256,
            &self.credential_capability_sha256,
            &self.target_sha256,
            &self.launch_provenance_sha256,
            &self.capability_ref_sha256,
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
        pinned_tools: Vec<OwnerToolProvenance>,
        policy: String,
        credential_capability: String,
        target: String,
        alternate_egress: String,
        launch_provenance: String,
    ) -> Result<Self, ProcessOwnerMaterialError> {
        if private_roots.is_empty()
            || private_roots.len() > 8
            || private_roots.iter().any(|path| !absolute_plain_path(path))
            || namespace.is_empty()
            || pinned_tools.is_empty()
            || pinned_tools.len() > MAX_TOOLS
            || policy.is_empty()
            || credential_capability.is_empty()
            || target.is_empty()
            || alternate_egress.is_empty()
            || launch_provenance.is_empty()
        {
            return Err(ProcessOwnerMaterialError::InvalidPrivateMaterial);
        }
        Ok(Self {
            private_roots,
            namespace,
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

    #[allow(dead_code)]
    fn digests(&self) -> (String, String, String, String, String, String) {
        let mut roots = Sha256::new();
        for root in &self.private_roots {
            add(&mut roots, &root.to_string_lossy());
        }
        let mut tools = Sha256::new();
        for tool in &self.pinned_tools {
            tool.digest_fields(&mut tools);
        }
        let mut policy = Sha256::new();
        add(&mut policy, &self.policy);
        let mut target = Sha256::new();
        add(&mut target, &self.target);
        add(&mut target, &self.alternate_egress);
        (
            digest_bytes(roots),
            digest_bytes(tools),
            digest_bytes(policy),
            digest_text(&self.credential_capability),
            digest_bytes(target),
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
    pub(crate) fn from_authenticated_issuer(
        issuer: AuthenticatedProcessOwnerIssuer,
    ) -> Result<Self, ProcessOwnerMaterialError> {
        let state = issuer
            .state
            .lock()
            .map_err(|_| ProcessOwnerMaterialError::StateUnavailable)?;
        state.contract.validate()?;
        state.material.validate_provenance()?;
        let (roots, tools, policy, credential, target, launch) = state.material.digests();
        if state.contract.private_roots_sha256 != roots
            || state.contract.tool_bundle_sha256 != tools
            || state.contract.policy_sha256 != policy
            || state.contract.credential_capability_sha256 != credential
            || state.contract.target_sha256 != target
            || state.contract.launch_provenance_sha256 != launch
        {
            return Err(ProcessOwnerMaterialError::MaterialMismatch);
        }
        drop(state);
        Ok(Self {
            state: issuer.state,
        })
    }

    /// Create a store from an already-authenticated runtime enrollment.
    /// This crate-private boundary is the only production issuer path;
    /// frontends and config parsers cannot manufacture an issuer.
    #[allow(dead_code)]
    pub(crate) fn from_runtime_authenticated(
        contract: ProcessOwnerMaterialContractV1,
        material: ProcessOwnerPrivateMaterialV1,
    ) -> Result<Self, ProcessOwnerMaterialError> {
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

#[cfg(test)]
mod tests {
    use super::*;
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
            vec![tool],
            "https://provider.example/v1".into(),
            "opaque-credential-capability".into(),
            "198.51.100.10:443".into(),
            "198.51.100.11:443".into(),
            "launch-provenance-v1".into(),
        )
        .unwrap();
        let (roots, tools, policy, credential, target, launch) = material.digests();
        let contract = ProcessOwnerMaterialContractV1 {
            schema_version: 1,
            owner_id: "owner-1".into(),
            provider: "provider-v1".into(),
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
            launch_provenance_sha256: launch,
            capability_ref_sha256: "6".repeat(64),
        };
        (contract, material, root)
    }

    #[test]
    fn authenticated_issuer_consumes_nonce_and_fences_lifecycle() {
        let (contract, material, root) = fixture();
        let store =
            ProcessOwnerMaterialStore::from_runtime_authenticated(contract, material).unwrap();
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
    fn provenance_rejects_hash_drift_missing_nonexec_and_symlink() {
        let (contract, material, root) = fixture();
        let mut wrong = material.clone();
        wrong.pinned_tools[0].executable_sha256 = "a".repeat(64);
        assert!(matches!(
            ProcessOwnerMaterialStore::from_runtime_authenticated(contract.clone(), wrong),
            Err(ProcessOwnerMaterialError::ExecutableDigestMismatch)
        ));
        let mut missing = material.clone();
        missing.pinned_tools[0].executable = root.join("tools/missing");
        assert!(matches!(
            ProcessOwnerMaterialStore::from_runtime_authenticated(contract.clone(), missing),
            Err(ProcessOwnerMaterialError::InvalidExecutable)
                | Err(ProcessOwnerMaterialError::MaterialMismatch)
        ));
        let mut symlink = material;
        let target = symlink.pinned_tools[0].executable.clone();
        let link = root.join("tools/link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        symlink.pinned_tools[0].executable = link;
        assert!(matches!(
            ProcessOwnerMaterialStore::from_runtime_authenticated(contract, symlink),
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
            ProcessOwnerMaterialStore::from_runtime_authenticated(contract, material),
            Err(ProcessOwnerMaterialError::InvalidContract)
        ));
        let _ = std::fs::remove_dir_all(root);
    }
}
