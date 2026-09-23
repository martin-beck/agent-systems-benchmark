// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Namespace-bound live provider relay capability.
use crate::provider_egress::{ProviderEgressHandoff, ProviderEgressPolicy};
use sha2::{Digest, Sha256};
use std::fs;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
const MAX: usize = 128;
const MAX_AGE: u64 = 86_400_000;
/// Version of the attested live relay contract.
pub const LIVE_NAMESPACE_HANDOFF_VERSION: u16 = 1;
/// Runtime-observed network namespace identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NamespaceIdentity(String);
impl NamespaceIdentity {
    /// Validate a non-secret namespace identity.
    pub fn new(value: impl Into<String>) -> Result<Self, LiveNamespaceError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > MAX
            || !value.bytes().all(|b| {
                b.is_ascii_alphanumeric() || matches!(b, b'.' | b':' | b'[' | b']' | b'-' | b'_')
            })
        {
            return Err(LiveNamespaceError::InvalidIdentity);
        }
        Ok(Self(value))
    }
    /// Read the current Linux network namespace identity.
    pub fn current() -> Result<Self, LiveNamespaceError> {
        Self::new(
            fs::read_link("/proc/self/ns/net")
                .map_err(|_| LiveNamespaceError::NamespaceUnavailable)?
                .to_string_lossy()
                .into_owned(),
        )
    }
    /// Opaque identity.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
/// Descendants cannot bypass the authenticated relay.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DescendantEgressPolicy {
    /// Direct and alternate provider sockets are denied.
    DenyDirect,
}
/// Secret-free child handoff.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChildRelayHandoff {
    endpoint: PathBuf,
    capability_sha256: String,
    namespace: NamespaceIdentity,
    generation: String,
    deadline_unix_ms: u64,
}
impl ChildRelayHandoff {
    /// Namespace-visible relay endpoint.
    pub fn endpoint(&self) -> &Path {
        &self.endpoint
    }
    /// Opaque capability digest.
    pub fn capability_sha256(&self) -> &str {
        &self.capability_sha256
    }
    /// Bound namespace.
    pub fn namespace(&self) -> &NamespaceIdentity {
        &self.namespace
    }
    /// Generation fence.
    pub fn generation(&self) -> &str {
        &self.generation
    }
    /// Expiry fence.
    pub fn deadline_unix_ms(&self) -> u64 {
        self.deadline_unix_ms
    }
}
/// Runtime-issued attestation binding one relay to one child namespace.
#[derive(Clone, Debug)]
pub struct LiveProviderNamespaceHandoff {
    version: u16,
    namespace: NamespaceIdentity,
    policy_sha256: String,
    generation: String,
    route_sha256: String,
    adapter_sha256: String,
    credential_ref_sha256: String,
    relay_socket: PathBuf,
    child_endpoint: PathBuf,
    deadline_unix_ms: u64,
    capability_sha256: String,
    revoked: Arc<AtomicBool>,
}
impl LiveProviderNamespaceHandoff {
    /// Issue from an authenticated provider policy and handoff only.
    #[allow(clippy::too_many_arguments)]
    pub fn issue(
        policy: &ProviderEgressPolicy,
        handoff: &ProviderEgressHandoff,
        namespace: NamespaceIdentity,
        generation: impl Into<String>,
        route: impl Into<String>,
        adapter: impl Into<String>,
        credential_ref: impl Into<String>,
        relay: PathBuf,
        child: PathBuf,
        deadline: u64,
        now: u64,
    ) -> Result<Self, LiveNamespaceError> {
        let generation = id(generation.into())?;
        let route = digest(route.into())?;
        let adapter = digest(adapter.into())?;
        let credential_ref = digest(credential_ref.into())?;
        socket(&relay)?;
        endpoint(&child)?;
        let policy_sha256 = format!("{:x}", Sha256::digest(policy.endpoint_sha256().as_bytes()));
        if deadline <= now
            || deadline - now > MAX_AGE
            || handoff.policy_sha256() != policy_sha256
            || handoff.generation() != generation
            || handoff.route_sha256() != route
        {
            return Err(LiveNamespaceError::HandoffMismatch);
        }
        let capability_sha256 = cap(
            &namespace,
            handoff.policy_sha256(),
            &generation,
            &route,
            &adapter,
            &credential_ref,
            &relay,
            &child,
            deadline,
        );
        Ok(Self {
            version: LIVE_NAMESPACE_HANDOFF_VERSION,
            namespace,
            policy_sha256,
            generation,
            route_sha256: route,
            adapter_sha256: adapter,
            credential_ref_sha256: credential_ref,
            relay_socket: relay,
            child_endpoint: child,
            deadline_unix_ms: deadline,
            capability_sha256,
            revoked: Arc::new(AtomicBool::new(false)),
        })
    }
    /// Validate identity, socket, revocation, and expiry before child effects.
    pub fn validate(
        &self,
        namespace: &NamespaceIdentity,
        now: u64,
    ) -> Result<(), LiveNamespaceError> {
        if self.version != LIVE_NAMESPACE_HANDOFF_VERSION || &self.namespace != namespace {
            return Err(LiveNamespaceError::NamespaceMismatch);
        }
        if self.revoked.load(Ordering::Acquire) {
            return Err(LiveNamespaceError::Revoked);
        }
        if now >= self.deadline_unix_ms {
            return Err(LiveNamespaceError::Expired);
        }
        socket(&self.relay_socket)?;
        endpoint(&self.child_endpoint)
    }
    /// Derive a child-visible handoff after validation.
    pub fn child_handoff(
        &self,
        namespace: &NamespaceIdentity,
        now: u64,
    ) -> Result<ChildRelayHandoff, LiveNamespaceError> {
        self.validate(namespace, now)?;
        Ok(ChildRelayHandoff {
            endpoint: self.child_endpoint.clone(),
            capability_sha256: self.capability_sha256.clone(),
            namespace: self.namespace.clone(),
            generation: self.generation.clone(),
            deadline_unix_ms: self.deadline_unix_ms,
        })
    }
    /// Revoke on cancellation or namespace teardown.
    pub fn revoke(&self) {
        self.revoked.store(true, Ordering::Release);
    }
    /// Descendant direct and alternate egress policy.
    pub fn descendant_policy(&self) -> DescendantEgressPolicy {
        DescendantEgressPolicy::DenyDirect
    }
    /// Provider policy digest.
    pub fn policy_sha256(&self) -> &str {
        &self.policy_sha256
    }
    /// Route digest.
    pub fn route_sha256(&self) -> &str {
        &self.route_sha256
    }
    /// Adapter digest.
    pub fn adapter_sha256(&self) -> &str {
        &self.adapter_sha256
    }
    /// Credential reference digest only.
    pub fn credential_ref_sha256(&self) -> &str {
        &self.credential_ref_sha256
    }
    /// Host-side relay socket.
    pub fn relay_socket(&self) -> &Path {
        &self.relay_socket
    }
    /// Capability digest.
    pub fn capability_sha256(&self) -> &str {
        &self.capability_sha256
    }
}
fn id(v: String) -> Result<String, LiveNamespaceError> {
    if v.is_empty()
        || v.len() > MAX
        || !v
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
    {
        Err(LiveNamespaceError::InvalidIdentity)
    } else {
        Ok(v)
    }
}
fn digest(v: String) -> Result<String, LiveNamespaceError> {
    if v.len() != 64
        || !v
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        Err(LiveNamespaceError::InvalidDigest)
    } else {
        Ok(v)
    }
}
fn socket(p: &Path) -> Result<(), LiveNamespaceError> {
    if !p.is_absolute() || p.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(LiveNamespaceError::InvalidRelay);
    }
    let m = fs::symlink_metadata(p).map_err(|_| LiveNamespaceError::RelayUnavailable)?;
    if !m.file_type().is_socket()
        || m.permissions().mode() & 0o077 != 0
        || m.uid() != rustix::process::geteuid().as_raw()
    {
        return Err(LiveNamespaceError::InvalidRelay);
    }
    Ok(())
}
fn endpoint(p: &Path) -> Result<(), LiveNamespaceError> {
    if !p.is_absolute() || p.components().any(|c| matches!(c, Component::ParentDir)) {
        Err(LiveNamespaceError::InvalidChildEndpoint)
    } else {
        Ok(())
    }
}
fn cap(
    n: &NamespaceIdentity,
    p: &str,
    g: &str,
    r: &str,
    a: &str,
    c: &str,
    relay: &Path,
    child: &Path,
    d: u64,
) -> String {
    let mut h = Sha256::new();
    for v in [n.as_str(), p, g, r, a, c] {
        h.update((v.len() as u64).to_le_bytes());
        h.update(v.as_bytes());
    }
    for x in [relay, child] {
        let v = x.to_string_lossy();
        h.update((v.len() as u64).to_le_bytes());
        h.update(v.as_bytes());
    }
    h.update(d.to_le_bytes());
    format!("{:x}", h.finalize())
}
/// Fail-closed live namespace errors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LiveNamespaceError {
    /// Namespace identity failed validation.
    InvalidIdentity,
    /// A required digest was not lowercase hexadecimal SHA-256.
    InvalidDigest,
    /// Relay socket path or metadata failed validation.
    InvalidRelay,
    /// Child endpoint path failed validation.
    InvalidChildEndpoint,
    /// The current network namespace could not be read.
    NamespaceUnavailable,
    /// The relay socket was unavailable.
    RelayUnavailable,
    /// Provider launch and namespace handoff fields disagreed.
    HandoffMismatch,
    /// The capability was presented in another namespace.
    NamespaceMismatch,
    /// The absolute handoff deadline elapsed.
    Expired,
    /// Cancellation or teardown revoked the capability.
    Revoked,
}
impl std::fmt::Display for LiveNamespaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidIdentity => "live namespace identity is invalid",
            Self::InvalidDigest => "live namespace digest is invalid",
            Self::InvalidRelay => "live relay socket is invalid",
            Self::InvalidChildEndpoint => "live child endpoint is invalid",
            Self::NamespaceUnavailable => "network namespace identity is unavailable",
            Self::RelayUnavailable => "live relay socket is unavailable",
            Self::HandoffMismatch => "live namespace handoff does not match provider launch",
            Self::NamespaceMismatch => "live namespace handoff is bound to another namespace",
            Self::Expired => "live namespace handoff has expired",
            Self::Revoked => "live namespace handoff was revoked",
        })
    }
}
impl std::error::Error for LiveNamespaceError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider_egress::ProviderEgressHandoff;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;
    fn setup() -> (
        ProviderEgressPolicy,
        ProviderEgressHandoff,
        PathBuf,
        PathBuf,
    ) {
        let p = ProviderEgressPolicy::new("https://openrouter.ai/api/v1", "openrouter.ai").unwrap();
        let h = ProviderEgressHandoff::issue_bound(&p, "g-1", "a".repeat(64), 10_000);
        let d = std::env::temp_dir().join(format!(
            "asb-live-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let s = d.join("relay.sock");
        let l = UnixListener::bind(&s).unwrap();
        std::fs::set_permissions(&s, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::mem::forget(l);
        (p, h, d, s)
    }
    fn issued() -> (LiveProviderNamespaceHandoff, NamespaceIdentity, PathBuf) {
        let (p, h, d, s) = setup();
        let n = NamespaceIdentity::new("net:[123]").unwrap();
        let x = LiveProviderNamespaceHandoff::issue(
            &p,
            &h,
            n.clone(),
            "g-1",
            "a".repeat(64),
            "b".repeat(64),
            "c".repeat(64),
            s,
            d.join("child.sock"),
            9_000,
            1_000,
        )
        .unwrap();
        (x, n, d)
    }
    #[test]
    fn admits_and_denies_descendants() {
        let (x, n, _d) = issued();
        assert_eq!(x.descendant_policy(), DescendantEgressPolicy::DenyDirect);
        assert!(x.child_handoff(&n, 2_000).is_ok());
        assert_eq!(x.credential_ref_sha256(), "c".repeat(64));
    }
    #[test]
    fn rejects_namespace_expiry_and_revocation() {
        let (x, n, _d) = issued();
        assert_eq!(
            x.child_handoff(&NamespaceIdentity::new("net:[124]").unwrap(), 2_000),
            Err(LiveNamespaceError::NamespaceMismatch)
        );
        assert_eq!(x.child_handoff(&n, 9_000), Err(LiveNamespaceError::Expired));
        x.revoke();
        assert_eq!(x.child_handoff(&n, 2_000), Err(LiveNamespaceError::Revoked));
    }
    #[test]
    fn rejects_secret_reference() {
        let (p, h, d, s) = setup();
        assert_eq!(
            LiveProviderNamespaceHandoff::issue(
                &p,
                &h,
                NamespaceIdentity::new("net:[123]").unwrap(),
                "g-1",
                "a".repeat(64),
                "b".repeat(64),
                "secret",
                s,
                d.join("child.sock"),
                9_000,
                1_000
            )
            .unwrap_err(),
            LiveNamespaceError::InvalidDigest
        );
    }
}
