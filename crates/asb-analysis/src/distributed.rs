// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Bounded coordination semantics for distributed experiment workers.

use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
};

const MAX_ID_BYTES: usize = 128;
const MAX_DURATION_NS: u64 = 86_400_000_000_000;

/// Stable identity of a worker or experiment attempt.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct WorkerId(String);

impl WorkerId {
    /// Construct a bounded, public worker identity.
    pub fn new(value: impl Into<String>) -> Result<Self, CoordinationError> {
        let value = value.into();
        if valid_id(&value) {
            Ok(Self(value))
        } else {
            Err(CoordinationError::InvalidIdentity)
        }
    }
    /// Return the worker identity.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Capability advertised by one worker.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkerCapability {
    worker: WorkerId,
    platform: String,
    slots: u16,
}

impl WorkerCapability {
    /// Construct a capability with a bounded platform and positive capacity.
    pub fn new(
        worker: WorkerId,
        platform: impl Into<String>,
        slots: u16,
    ) -> Result<Self, CoordinationError> {
        let platform = platform.into();
        if !valid_id(&platform) || slots == 0 {
            return Err(CoordinationError::InvalidCapability);
        }
        Ok(Self {
            worker,
            platform,
            slots,
        })
    }
    /// Return the worker identity.
    pub fn worker(&self) -> &WorkerId {
        &self.worker
    }
    /// Return the stable platform identity.
    pub fn platform(&self) -> &str {
        &self.platform
    }
    /// Return the advertised concurrent slot count.
    pub fn slots(&self) -> u16 {
        self.slots
    }
}

/// A lease fencing one attempt on one worker.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttemptLease {
    attempt: String,
    worker: WorkerId,
    fence: u64,
    expires_at_ns: u64,
}

impl AttemptLease {
    /// Return the attempt identity.
    pub fn attempt(&self) -> &str {
        &self.attempt
    }
    /// Return the owning worker.
    pub fn worker(&self) -> &WorkerId {
        &self.worker
    }
    /// Return the fencing token.
    pub const fn fence(&self) -> u64 {
        self.fence
    }
    /// Return the lease deadline in the worker's monotonic clock domain.
    pub const fn expires_at_ns(&self) -> u64 {
        self.expires_at_ns
    }
}

/// Host-local terminal evidence. Durations from different hosts are never subtracted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttemptCompletion {
    attempt: String,
    worker: WorkerId,
    fence: u64,
    duration_ns: u64,
    clock_uncertainty_ns: u64,
    artifact_sha256: String,
}

impl AttemptCompletion {
    /// Construct bounded completion evidence.
    pub fn new(
        attempt: impl Into<String>,
        worker: WorkerId,
        fence: u64,
        duration_ns: u64,
        clock_uncertainty_ns: u64,
        artifact_sha256: impl Into<String>,
    ) -> Result<Self, CoordinationError> {
        let attempt = attempt.into();
        let artifact_sha256 = artifact_sha256.into();
        if !valid_id(&attempt)
            || fence == 0
            || duration_ns > MAX_DURATION_NS
            || clock_uncertainty_ns > duration_ns
            || !valid_digest(&artifact_sha256)
        {
            return Err(CoordinationError::InvalidCompletion);
        }
        Ok(Self {
            attempt,
            worker,
            fence,
            duration_ns,
            clock_uncertainty_ns,
            artifact_sha256,
        })
    }
    /// Return the attempt identity.
    pub fn attempt(&self) -> &str {
        &self.attempt
    }
    /// Return the worker identity.
    pub fn worker(&self) -> &WorkerId {
        &self.worker
    }
    /// Return the fencing token.
    pub const fn fence(&self) -> u64 {
        self.fence
    }
    /// Return the local monotonic duration.
    pub const fn duration_ns(&self) -> u64 {
        self.duration_ns
    }
    /// Return the local clock uncertainty bound.
    pub const fn clock_uncertainty_ns(&self) -> u64 {
        self.clock_uncertainty_ns
    }
    /// Return the content-addressed artifact identity.
    pub fn artifact_sha256(&self) -> &str {
        &self.artifact_sha256
    }
}

/// Bounded errors from distributed coordination.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CoordinationError {
    /// Identity or attempt field is malformed.
    InvalidIdentity,
    /// Capability is malformed or has no slots.
    InvalidCapability,
    /// Completion evidence is malformed or exceeds bounds.
    InvalidCompletion,
    /// Worker is already registered.
    DuplicateWorker,
    /// Worker has not advertised a capability.
    UnknownWorker,
    /// No capacity is currently available.
    CapacityUnavailable,
    /// Attempt is already leased and the lease is still current.
    LeaseActive,
    /// Completion does not match the current lease fence.
    StaleLease,
    /// Completion for this attempt was already accepted.
    DuplicateCompletion,
}

impl fmt::Display for CoordinationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl Error for CoordinationError {}

/// In-memory coordinator state with explicit worker and attempt ownership.
#[derive(Default)]
pub struct Coordinator {
    workers: BTreeMap<WorkerId, WorkerCapability>,
    leases: BTreeMap<String, AttemptLease>,
    completions: BTreeMap<String, AttemptCompletion>,
    active_attempts: BTreeSet<(WorkerId, String)>,
    next_fence: u64,
}

impl Coordinator {
    /// Register one worker capability exactly once.
    pub fn register(&mut self, capability: WorkerCapability) -> Result<(), CoordinationError> {
        if self
            .workers
            .insert(capability.worker.clone(), capability)
            .is_some()
        {
            return Err(CoordinationError::DuplicateWorker);
        }
        Ok(())
    }
    /// Remove a worker; its leases become stale and cannot complete.
    pub fn remove(&mut self, worker: &WorkerId) {
        self.workers.remove(worker);
        self.active_attempts.retain(|(owner, _)| owner != worker);
    }
    /// Lease an attempt, fencing an expired prior lease with a new token.
    pub fn acquire(
        &mut self,
        attempt: impl Into<String>,
        worker: &WorkerId,
        now_ns: u64,
        ttl_ns: u64,
    ) -> Result<AttemptLease, CoordinationError> {
        let attempt = attempt.into();
        if !valid_id(&attempt) || ttl_ns == 0 || ttl_ns > MAX_DURATION_NS {
            return Err(CoordinationError::InvalidIdentity);
        }
        if let Some(old) = self.leases.get(&attempt) {
            if old.expires_at_ns > now_ns {
                return Err(CoordinationError::LeaseActive);
            }
            self.active_attempts
                .remove(&(old.worker.clone(), attempt.clone()));
        }
        let capability = self
            .workers
            .get(worker)
            .ok_or(CoordinationError::UnknownWorker)?;
        if self
            .active_attempts
            .iter()
            .filter(|(owner, _)| owner == worker)
            .count()
            >= capability.slots as usize
        {
            return Err(CoordinationError::CapacityUnavailable);
        }
        self.next_fence = self
            .next_fence
            .checked_add(1)
            .ok_or(CoordinationError::StaleLease)?;
        let lease = AttemptLease {
            attempt: attempt.clone(),
            worker: worker.clone(),
            fence: self.next_fence,
            expires_at_ns: now_ns.saturating_add(ttl_ns),
        };
        self.active_attempts.insert((worker.clone(), attempt));
        self.leases.insert(lease.attempt.clone(), lease.clone());
        Ok(lease)
    }
    /// Accept exactly one completion matching the current unexpired lease.
    pub fn complete(
        &mut self,
        completion: AttemptCompletion,
        now_ns: u64,
    ) -> Result<(), CoordinationError> {
        if self.completions.contains_key(completion.attempt()) {
            return Err(CoordinationError::DuplicateCompletion);
        }
        if !self.workers.contains_key(completion.worker()) {
            return Err(CoordinationError::StaleLease);
        }
        let lease = self
            .leases
            .get(completion.attempt())
            .ok_or(CoordinationError::StaleLease)?;
        if lease.worker != *completion.worker()
            || lease.fence != completion.fence()
            || lease.expires_at_ns <= now_ns
        {
            return Err(CoordinationError::StaleLease);
        }
        self.active_attempts
            .remove(&(lease.worker.clone(), lease.attempt.clone()));
        self.completions
            .insert(completion.attempt.clone(), completion);
        Ok(())
    }
    /// Return accepted completion evidence for an attempt.
    pub fn completion(&self, attempt: &str) -> Option<&AttemptCompletion> {
        self.completions.get(attempt)
    }
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ID_BYTES
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}
fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn worker() -> (WorkerId, WorkerCapability) {
        let id = WorkerId::new("worker-a").unwrap();
        (
            id.clone(),
            WorkerCapability::new(id, "linux-amd64", 1).unwrap(),
        )
    }

    #[test]
    fn fences_expired_retry_and_rejects_old_completion() {
        let (id, capability) = worker();
        let mut coordinator = Coordinator::default();
        coordinator.register(capability).unwrap();
        let first = coordinator.acquire("attempt-1", &id, 10, 5).unwrap();
        assert!(matches!(
            coordinator.acquire("attempt-2", &id, 11, 5),
            Err(CoordinationError::CapacityUnavailable)
        ));
        let second = coordinator.acquire("attempt-1", &id, 20, 5).unwrap();
        assert!(second.fence() > first.fence());
        let stale =
            AttemptCompletion::new("attempt-1", id.clone(), first.fence(), 2, 1, HASH).unwrap();
        assert!(matches!(
            coordinator.complete(stale, 21),
            Err(CoordinationError::StaleLease)
        ));
        let accepted = AttemptCompletion::new("attempt-1", id, second.fence(), 2, 1, HASH).unwrap();
        coordinator.complete(accepted, 21).unwrap();
    }

    #[test]
    fn duplicate_completion_and_worker_loss_fail_closed() {
        let (id, capability) = worker();
        let mut coordinator = Coordinator::default();
        coordinator.register(capability).unwrap();
        let lease = coordinator.acquire("attempt-1", &id, 0, 10).unwrap();
        coordinator.remove(&id);
        let completion =
            AttemptCompletion::new("attempt-1", id.clone(), lease.fence(), 2, 0, HASH).unwrap();
        assert!(matches!(
            coordinator.complete(completion, 1),
            Err(CoordinationError::StaleLease)
        ));
        assert!(matches!(
            coordinator.acquire("attempt-2", &id, 2, 10),
            Err(CoordinationError::UnknownWorker)
        ));
    }

    #[test]
    fn malformed_timing_and_digest_are_rejected() {
        let id = WorkerId::new("worker-a").unwrap();
        assert!(matches!(
            AttemptCompletion::new("a", id.clone(), 1, 2, 3, HASH),
            Err(CoordinationError::InvalidCompletion)
        ));
        assert!(matches!(
            AttemptCompletion::new("a", id, 1, 2, 0, "bad"),
            Err(CoordinationError::InvalidCompletion)
        ));
    }
}
