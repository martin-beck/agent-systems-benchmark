// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Namespace-bound live provider relay capability.
use crate::provider_egress::{ProviderEgressHandoff, ProviderEgressPolicy};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::UnixListener;
use std::path::{Component, Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};
const MAX: usize = 128;
const MAX_AGE: u64 = 86_400_000;
const RELEASE: &[u8] = b"ASB-LIVE-RELEASE/1\n";
/// Version of the attested live relay contract.
pub const LIVE_NAMESPACE_HANDOFF_VERSION: u16 = 1;

/// Runtime-owned one-shot gate for a live child launch.
///
/// The child-side gate completes this handshake before it execs the provider
/// adapter. The runtime can therefore observe `/proc/<pid>/ns/net` before the
/// adapter receives the live relay capability.
pub struct LiveLaunchGate {
    path: PathBuf,
    listener: UnixListener,
}

impl LiveLaunchGate {
    /// Bind a fresh private gate socket.
    pub fn bind(generation: &str) -> Result<Self, LiveNamespaceError> {
        let generation = id(generation.to_owned())?;
        let path =
            std::env::temp_dir().join(format!("asb-live-gate-{}-{generation}", std::process::id()));
        let _ = fs::remove_file(&path);
        let listener =
            UnixListener::bind(&path).map_err(|_| LiveNamespaceError::GateUnavailable)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .map_err(|_| LiveNamespaceError::GateUnavailable)?;
        listener
            .set_nonblocking(true)
            .map_err(|_| LiveNamespaceError::GateUnavailable)?;
        Ok(Self { path, listener })
    }

    /// Host path to bind into the private child namespace.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Release one waiting child after the runtime has attested its namespace.
    pub fn release(self, capability_sha256: &str) -> Result<(), LiveNamespaceError> {
        if capability_sha256.len() != 64
            || !capability_sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err(LiveNamespaceError::InvalidDigest);
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        let (mut stream, _) = loop {
            match self.listener.accept() {
                Ok(connection) => break connection,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return Err(LiveNamespaceError::GateUnavailable);
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(_) => return Err(LiveNamespaceError::GateUnavailable),
            }
        };
        stream
            .write_all(RELEASE)
            .and_then(|_| stream.write_all(capability_sha256.as_bytes()))
            .and_then(|_| stream.write_all(b"\n"))
            .map_err(|_| LiveNamespaceError::GateUnavailable)
    }
}

impl Drop for LiveLaunchGate {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
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
        Self::for_pid(std::process::id())
    }
    /// Read the network namespace actually occupied by a running process.
    ///
    /// The process identifier is supplied by the runtime immediately after
    /// the sandbox boundary has been created.  It is never accepted from a
    /// caller or from the child environment.
    pub fn for_pid(pid: u32) -> Result<Self, LiveNamespaceError> {
        if pid == 0 {
            return Err(LiveNamespaceError::NamespaceUnavailable);
        }
        Self::new(
            fs::read_link(format!("/proc/{pid}/ns/net"))
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
        if self.version != LIVE_NAMESPACE_HANDOFF_VERSION
            || &self.namespace != namespace
            || self.capability_sha256
                != cap(
                    &self.namespace,
                    &self.policy_sha256,
                    &self.generation,
                    &self.route_sha256,
                    &self.adapter_sha256,
                    &self.credential_ref_sha256,
                    &self.relay_socket,
                    &self.child_endpoint,
                    self.deadline_unix_ms,
                )
        {
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

    /// Validate against the identity observed by the runtime process rather
    /// than accepting a caller-provided string as an attestation.
    pub fn validate_runtime_observed_namespace(
        &self,
        now_unix_ms: u64,
    ) -> Result<NamespaceIdentity, LiveNamespaceError> {
        let observed = NamespaceIdentity::current()?;
        self.validate(&observed, now_unix_ms)?;
        Ok(observed)
    }
    /// Validate against the namespace observed for the launched child.
    pub fn validate_runtime(
        &self,
        child_pid: u32,
        now: u64,
    ) -> Result<NamespaceIdentity, LiveNamespaceError> {
        let observed = NamespaceIdentity::for_pid(child_pid)?;
        self.validate(&observed, now)?;
        Ok(observed)
    }
    /// Rebind a metadata-validated handoff to the namespace observed for the
    /// actual gated child. The caller cannot choose this identity.
    pub fn rebind_runtime_namespace(
        &self,
        child_pid: u32,
        now: u64,
    ) -> Result<(Self, ChildRelayHandoff), LiveNamespaceError> {
        self.validate_metadata(now)?;
        let observed = NamespaceIdentity::for_pid(child_pid)?;
        let mut rebound = self.clone();
        rebound.namespace = observed;
        rebound.capability_sha256 = cap(
            &rebound.namespace,
            &rebound.policy_sha256,
            &rebound.generation,
            &rebound.route_sha256,
            &rebound.adapter_sha256,
            &rebound.credential_ref_sha256,
            &rebound.relay_socket,
            &rebound.child_endpoint,
            rebound.deadline_unix_ms,
        );
        let child = rebound.child_handoff(&rebound.namespace, now)?;
        Ok((rebound, child))
    }
    /// Derive a child handoff only after observing the actual child namespace.
    pub fn child_handoff_runtime(
        &self,
        child_pid: u32,
        now: u64,
    ) -> Result<ChildRelayHandoff, LiveNamespaceError> {
        let observed = self.validate_runtime(child_pid, now)?;
        self.child_handoff(&observed, now)
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
    fn validate_metadata(&self, now: u64) -> Result<(), LiveNamespaceError> {
        if self.version != LIVE_NAMESPACE_HANDOFF_VERSION
            || self.capability_sha256
                != cap(
                    &self.namespace,
                    &self.policy_sha256,
                    &self.generation,
                    &self.route_sha256,
                    &self.adapter_sha256,
                    &self.credential_ref_sha256,
                    &self.relay_socket,
                    &self.child_endpoint,
                    self.deadline_unix_ms,
                )
        {
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
    /// Launch generation fence.
    pub fn generation(&self) -> &str {
        &self.generation
    }
    /// Namespace identity bound by the runtime.
    pub fn namespace(&self) -> &NamespaceIdentity {
        &self.namespace
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
#[allow(clippy::too_many_arguments)]
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
    /// The pre-effect child launch gate could not be established.
    GateUnavailable,
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
            Self::GateUnavailable => "live child launch gate is unavailable",
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
    use std::io::Read;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;
    use std::os::unix::net::UnixStream;
    use std::thread;
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

    #[test]
    fn launch_gate_withholds_and_then_releases_exact_marker() {
        let gate = LiveLaunchGate::bind("g-gate").unwrap();
        let path = gate.path().to_owned();
        let client = thread::spawn(move || {
            let mut stream = loop {
                match UnixStream::connect(&path) {
                    Ok(stream) => break stream,
                    Err(_) => thread::yield_now(),
                }
            };
            let mut marker = [0; RELEASE.len()];
            stream.read_exact(&mut marker).unwrap();
            let mut digest = [0; 65];
            stream.read_exact(&mut digest).unwrap();
            (marker, digest)
        });
        gate.release(&"a".repeat(64)).unwrap();
        let (marker, digest) = client.join().unwrap();
        assert_eq!(marker, RELEASE);
        assert_eq!(&digest[..64], b"a".repeat(64).as_slice());
        assert_eq!(digest[64], b'\n');
    }

    #[test]
    fn launch_gate_fails_closed_without_child_connection() {
        let gate = LiveLaunchGate::bind("g-no-child").unwrap();
        let path = gate.path().to_owned();
        assert_eq!(
            gate.release(&"a".repeat(64)),
            Err(LiveNamespaceError::GateUnavailable)
        );
        assert!(!path.exists());
    }

    #[test]
    fn runtime_observation_rejects_missing_pid() {
        assert_eq!(
            NamespaceIdentity::for_pid(0),
            Err(LiveNamespaceError::NamespaceUnavailable)
        );
    }

    #[test]
    fn copied_capability_digest_is_rejected() {
        let (p, h, d, s) = setup();
        let n = NamespaceIdentity::current().unwrap();
        let mut x = LiveProviderNamespaceHandoff::issue(
            &p,
            &h,
            n,
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
        x.capability_sha256.replace_range(..1, "0");
        assert_eq!(
            x.validate_runtime_observed_namespace(2_000),
            Err(LiveNamespaceError::NamespaceMismatch)
        );
    }

    #[test]
    fn runtime_rebind_uses_observed_pid_not_parent_metadata() {
        let (x, _namespace, _d) = issued();
        let original = x.capability_sha256().to_owned();
        let (rebound, child) = x
            .rebind_runtime_namespace(std::process::id(), 2_000)
            .unwrap();
        assert_eq!(rebound.namespace(), &NamespaceIdentity::current().unwrap());
        assert_eq!(child.capability_sha256(), rebound.capability_sha256());
        assert_ne!(child.capability_sha256(), original);
        assert!(
            rebound
                .validate(&NamespaceIdentity::current().unwrap(), 2_000)
                .is_ok()
        );
    }
}
