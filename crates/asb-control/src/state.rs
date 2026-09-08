// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Pure admission, idempotency, and reconnect state machines.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::{Duration, Instant};

use serde_json::Value;
use thiserror::Error;

use crate::{
    CONTROL_V1, ControlCall, ControlEvent, ControlLimits, ControlRequest, ControlVersion,
    NegotiateParams, Page, ProtocolError, RequestId, Revision, validate_request,
};

/// Per-connection protocol state. Dropping it never changes runner-owned runs.
#[derive(Debug)]
pub struct ControlSession {
    server_limits: ControlLimits,
    effective_limits: Option<ControlLimits>,
    in_flight: BTreeSet<RequestId>,
}

impl ControlSession {
    /// Create an unnegotiated session.
    pub fn new(server_limits: ControlLimits) -> Result<Self, SessionError> {
        Ok(Self {
            server_limits: server_limits.validate()?,
            effective_limits: None,
            in_flight: BTreeSet::new(),
        })
    }

    /// Select v1 and intersect client/server limits exactly once.
    pub fn negotiate(
        &mut self,
        offer: &NegotiateParams,
    ) -> Result<(ControlVersion, ControlLimits), SessionError> {
        if self.effective_limits.is_some() {
            return Err(SessionError::AlreadyNegotiated);
        }
        if !offer.versions.contains(&CONTROL_V1) {
            return Err(SessionError::IncompatibleVersion);
        }
        let selected = CONTROL_V1;
        let limits = self.server_limits.intersect(offer.limits)?;
        self.effective_limits = Some(limits);
        Ok((selected, limits))
    }

    /// Validate and process the complete first JSON-RPC request envelope.
    pub fn negotiate_request(
        &mut self,
        request: &ControlRequest,
    ) -> Result<(ControlVersion, ControlLimits), SessionError> {
        let ControlCall::Negotiate(offer) = &request.call else {
            return Err(SessionError::NegotiationRequired);
        };
        crate::validate_negotiation_request(request, self.server_limits)?;
        self.negotiate(offer)
    }

    /// Admit one request before any backend work.
    pub fn admit(&mut self, request: &ControlRequest) -> Result<Admission, SessionError> {
        let ingress = RequestDeadline::start(request.timeout_ms)?;
        self.admit_with_deadline(request, ingress)
    }

    /// Admit a request using the budget that began before frame receipt.
    pub fn admit_with_deadline(
        &mut self,
        request: &ControlRequest,
        ingress: RequestDeadline,
    ) -> Result<Admission, SessionError> {
        if matches!(request.call, ControlCall::Negotiate(_)) {
            return Err(SessionError::NegotiationHandledSeparately);
        }
        let limits = self
            .effective_limits
            .ok_or(SessionError::NegotiationRequired)?;
        validate_request(request, limits)?;
        if self.in_flight.contains(&request.id) {
            return Err(SessionError::DuplicateRequest);
        }
        if self.in_flight.len() == usize::from(limits.max_in_flight) {
            return Err(SessionError::Backpressure);
        }
        self.in_flight.insert(request.id);
        Ok(Admission {
            id: request.id,
            timeout_ms: request.timeout_ms,
            deadline: ingress.tighten(request.timeout_ms)?,
        })
    }

    /// Mark a terminal response committed and free admission capacity.
    pub fn finish(&mut self, id: RequestId) -> Result<(), SessionError> {
        if self.in_flight.remove(&id) {
            Ok(())
        } else {
            Err(SessionError::UnknownRequest)
        }
    }

    /// Effective limits after successful negotiation.
    #[must_use]
    pub fn limits(&self) -> Option<ControlLimits> {
        self.effective_limits
    }

    /// Number of requests admitted without terminal responses.
    #[must_use]
    pub fn in_flight(&self) -> usize {
        self.in_flight.len()
    }
}

/// Request admitted under a relative monotonic deadline.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Admission {
    /// Matching request identity.
    pub id: RequestId,
    /// Maximum processing time beginning at admission.
    pub timeout_ms: u64,
    deadline: RequestDeadline,
}

impl Admission {
    /// Absolute monotonic budget captured once at admission.
    #[must_use]
    pub fn deadline(&self) -> RequestDeadline {
        self.deadline
    }
}

/// Absolute monotonic request budget. It is never reset after partial I/O.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestDeadline {
    started_at: Instant,
    expires_at: Instant,
}

impl RequestDeadline {
    /// Start one bounded budget from the current monotonic clock.
    pub fn start(timeout_ms: u64) -> Result<Self, SessionError> {
        if timeout_ms == 0 || timeout_ms > crate::MAX_CONTROL_TIMEOUT_MS {
            return Err(SessionError::Protocol(ProtocolError::InvalidTimeout));
        }
        let started_at = Instant::now();
        let expires_at = started_at
            .checked_add(Duration::from_millis(timeout_ms))
            .ok_or(SessionError::DeadlineExceeded)?;
        Ok(Self {
            started_at,
            expires_at,
        })
    }

    /// Tighten this ingress budget to a timeout measured from its original start.
    ///
    /// This prevents frame receipt, backend work, and response writes from each
    /// receiving a fresh relative timeout.
    pub fn tighten(self, timeout_ms: u64) -> Result<Self, SessionError> {
        if timeout_ms == 0 || timeout_ms > crate::MAX_CONTROL_TIMEOUT_MS {
            return Err(SessionError::Protocol(ProtocolError::InvalidTimeout));
        }
        let requested = self
            .started_at
            .checked_add(Duration::from_millis(timeout_ms))
            .ok_or(SessionError::DeadlineExceeded)?;
        Ok(Self {
            started_at: self.started_at,
            expires_at: self.expires_at.min(requested),
        })
    }

    /// Remaining budget for the next blocking or backend operation.
    pub fn remaining(self) -> Result<Duration, SessionError> {
        self.expires_at
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or(SessionError::DeadlineExceeded)
    }

    /// Fail if all processing time has already been consumed.
    pub fn check(self) -> Result<(), SessionError> {
        self.remaining().map(|_| ())
    }
}

/// Connection admission failure.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum SessionError {
    /// Common protocol validation failed.
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
    /// A non-negotiation call arrived first.
    #[error("protocol negotiation is required")]
    NegotiationRequired,
    /// No offered version has a compatible major.
    #[error("no compatible control protocol version")]
    IncompatibleVersion,
    /// Renegotiation on an active connection is forbidden.
    #[error("control protocol is already negotiated")]
    AlreadyNegotiated,
    /// Negotiation must be processed before ordinary admission.
    #[error("negotiation is handled separately")]
    NegotiationHandledSeparately,
    /// The same request id remains outstanding.
    #[error("duplicate outstanding request")]
    DuplicateRequest,
    /// Effective in-flight capacity is exhausted.
    #[error("connection backpressure")]
    Backpressure,
    /// A response attempted to finish an untracked request.
    #[error("unknown outstanding request")]
    UnknownRequest,
    /// The absolute monotonic request budget was consumed.
    #[error("control request deadline exceeded")]
    DeadlineExceeded,
    /// Runner backend ended without producing a terminal outcome.
    #[error("control runner backend disconnected")]
    BackendDisconnected,
}

/// Durable mutation record restored from the runner journal after restart.
#[derive(Clone, Debug, PartialEq)]
pub struct MutationRecord {
    /// Opaque frontend idempotency key.
    pub key: String,
    /// Digest of the canonical mutation request.
    pub request_sha256: String,
    /// Previously committed public terminal result.
    pub result: Value,
}

/// Bounded projection of durable mutation records.
#[derive(Debug)]
pub struct IdempotencyIndex {
    maximum: usize,
    records: BTreeMap<String, MutationRecord>,
}

impl IdempotencyIndex {
    /// Restore an index from authoritative journal records.
    pub fn restore(
        maximum: usize,
        records: impl IntoIterator<Item = MutationRecord>,
    ) -> Result<Self, IdempotencyError> {
        if maximum == 0 {
            return Err(IdempotencyError::InvalidCapacity);
        }
        let mut index = Self {
            maximum,
            records: BTreeMap::new(),
        };
        for record in records {
            if index.records.len() == maximum {
                return Err(IdempotencyError::Capacity);
            }
            match index.records.insert(record.key.clone(), record) {
                None => {}
                Some(_) => return Err(IdempotencyError::DuplicateJournalKey),
            }
        }
        Ok(index)
    }

    /// Decide whether a mutation is new, a safe retry, or a conflicting reuse.
    pub fn check(
        &self,
        key: &str,
        request_sha256: &str,
    ) -> Result<IdempotencyDecision<'_>, IdempotencyError> {
        match self.records.get(key) {
            Some(record) if record.request_sha256 == request_sha256 => {
                Ok(IdempotencyDecision::Replay(&record.result))
            }
            Some(_) => Err(IdempotencyError::Conflict),
            None if self.records.len() == self.maximum => Err(IdempotencyError::Capacity),
            None => Ok(IdempotencyDecision::New),
        }
    }

    /// Index a result only after its authoritative journal commit succeeds.
    pub fn record_committed(&mut self, record: MutationRecord) -> Result<(), IdempotencyError> {
        match self.check(&record.key, &record.request_sha256)? {
            IdempotencyDecision::New => {
                self.records.insert(record.key.clone(), record);
                Ok(())
            }
            IdempotencyDecision::Replay(_) => Err(IdempotencyError::DuplicateJournalKey),
        }
    }

    /// Number of retained durable keys.
    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Whether no durable keys are retained.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

/// Idempotency lookup outcome.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum IdempotencyDecision<'a> {
    /// Caller may begin one new durable mutation transaction.
    New,
    /// Caller must return this already committed result without repeating effects.
    Replay(&'a Value),
}

/// Durable idempotency projection failure.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum IdempotencyError {
    /// Retention capacity must be positive.
    #[error("idempotency capacity must be positive")]
    InvalidCapacity,
    /// Retention is full and eviction would make an old retry unsafe.
    #[error("idempotency retention is exhausted")]
    Capacity,
    /// One key names a different request.
    #[error("idempotency key conflicts with its durable request")]
    Conflict,
    /// Authoritative input contains duplicate keys.
    #[error("durable journal contains a duplicate idempotency key")]
    DuplicateJournalKey,
}

/// Bounded public event projection used for reconnect pagination.
#[derive(Debug)]
pub struct EventWindow {
    maximum: usize,
    events: VecDeque<ControlEvent>,
}

impl EventWindow {
    /// Restore a retained window, rejecting gaps and unordered revisions.
    pub fn restore(
        maximum: usize,
        events: impl IntoIterator<Item = ControlEvent>,
    ) -> Result<Self, CursorError> {
        if maximum == 0 {
            return Err(CursorError::InvalidCapacity);
        }
        let mut window = Self {
            maximum,
            events: VecDeque::new(),
        };
        for event in events {
            window.append(event)?;
        }
        Ok(window)
    }

    /// Append the next durable public event and evict only the oldest projection.
    pub fn append(&mut self, event: ControlEvent) -> Result<(), CursorError> {
        if let Some(previous) = self.events.back() {
            let expected = previous
                .revision
                .0
                .checked_add(1)
                .ok_or(CursorError::RevisionOverflow)?;
            if event.revision.0 != expected {
                return Err(CursorError::NonContiguous {
                    expected: Revision(expected),
                    actual: event.revision,
                });
            }
        }
        if self.events.len() == self.maximum {
            self.events.pop_front();
        }
        self.events.push_back(event);
        Ok(())
    }

    /// Return records strictly newer than a cursor, or fail if history was lost.
    pub fn page(
        &self,
        after: Option<Revision>,
        limit: u16,
    ) -> Result<Page<ControlEvent>, CursorError> {
        if limit == 0 {
            return Err(CursorError::InvalidPage);
        }
        let Some(oldest) = self.events.front().map(|event| event.revision) else {
            if after.is_some_and(|revision| revision.0 != 0) {
                return Err(CursorError::Future);
            }
            return Ok(Page {
                items: Vec::new(),
                next: after,
                has_more: false,
            });
        };
        let latest = self.events.back().expect("nonempty window").revision;
        if let Some(cursor) = after {
            if cursor.0 > latest.0 {
                return Err(CursorError::Future);
            }
            if cursor.0.checked_add(1).is_none_or(|next| next < oldest.0) {
                return Err(CursorError::Stale { oldest });
            }
        }
        let start = after.map_or(oldest.0, |cursor| cursor.0.saturating_add(1));
        let take = usize::from(limit);
        let items: Vec<_> = self
            .events
            .iter()
            .filter(|event| event.revision.0 >= start)
            .take(take)
            .cloned()
            .collect();
        let next = items.last().map(|event| event.revision).or(after);
        let has_more = next.is_some_and(|cursor| cursor.0 < latest.0);
        Ok(Page {
            items,
            next,
            has_more,
        })
    }

    /// Oldest retained revision, or zero for an empty runner.
    #[must_use]
    pub fn oldest_revision(&self) -> Revision {
        self.events
            .front()
            .map_or(Revision(0), |event| event.revision)
    }

    /// Latest retained revision, or zero for an empty runner.
    #[must_use]
    pub fn latest_revision(&self) -> Revision {
        self.events
            .back()
            .map_or(Revision(0), |event| event.revision)
    }
}

/// Reconnect cursor validation failure.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum CursorError {
    /// Retention capacity must be positive.
    #[error("event retention capacity must be positive")]
    InvalidCapacity,
    /// Page size must be positive.
    #[error("event page size must be positive")]
    InvalidPage,
    /// Durable revisions cannot advance beyond u64.
    #[error("event revision overflow")]
    RevisionOverflow,
    /// Restored or appended revisions contain a gap.
    #[error("event revision is non-contiguous")]
    NonContiguous {
        /// Required next revision.
        expected: Revision,
        /// Observed revision.
        actual: Revision,
    },
    /// Cursor predates retained history.
    #[error("event cursor is stale")]
    Stale {
        /// Oldest retained revision.
        oldest: Revision,
    },
    /// Cursor is newer than runner truth.
    #[error("event cursor is in the future")]
    Future,
}
