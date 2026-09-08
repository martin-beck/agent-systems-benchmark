// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Bounded executable recovery model shared by exhaustive and production traces.

/// Number of attempts in the finite model.
pub const ATTEMPTS: usize = 2;
/// Number of independent replay sessions in the finite model.
pub const SESSIONS: usize = 2;
/// Largest modeled lease epoch.
pub const MAX_EPOCH: u8 = 2;
/// Largest modeled journal.
pub const MAX_JOURNAL: usize = 4;
/// Largest modeled replay cursor.
pub const MAX_CURSOR: u8 = 2;

/// Attempt lifecycle in the crash/recovery model.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Phase {
    /// No external effect has started.
    Planned,
    /// External execution might have effects.
    Running,
    /// One known terminal completion was accepted.
    Terminal,
    /// A crash left the external effect uncertain and restart is forbidden.
    Uncertain,
}

/// Durable event kind.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum EventKind {
    /// Execution intent became externally active.
    Started,
    /// A known terminal result was accepted.
    Completed,
    /// Reconciliation preserved an uncertain terminal result.
    Uncertain,
}

/// One bounded durable journal event.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Event {
    /// Attempt index.
    pub attempt: usize,
    /// Lease epoch carried by the event.
    pub epoch: u8,
    /// Event kind.
    pub kind: EventKind,
}

/// State transition attempted by the explorer.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Action {
    /// Acquire the one global lease for an attempt.
    Acquire {
        /// Attempt index.
        attempt: usize,
    },
    /// Start under an exact lease epoch.
    Start {
        /// Attempt index.
        attempt: usize,
        /// Presented lease epoch.
        epoch: u8,
    },
    /// Complete under an exact lease epoch.
    Complete {
        /// Attempt index.
        attempt: usize,
        /// Presented lease epoch.
        epoch: u8,
    },
    /// Lose ownership after an external effect may have happened.
    Crash {
        /// Attempt index.
        attempt: usize,
    },
    /// Persist uncertainty without repeating the effect.
    ReconcileUncertain {
        /// Attempt index.
        attempt: usize,
    },
    /// Advance the durable projection by one journal entry.
    ProjectNext,
    /// Advance exactly one replay session.
    AdvanceReplay {
        /// Replay-session index.
        session: usize,
    },
}

/// Complete bounded model state.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct RecoveryState {
    lease: Option<usize>,
    lease_epoch: u8,
    attempt_epoch: [u8; ATTEMPTS],
    phase: [Phase; ATTEMPTS],
    journal: Vec<Event>,
    projection: usize,
    completion_count: [u8; ATTEMPTS],
    cursors: [u8; SESSIONS],
}

impl Default for RecoveryState {
    fn default() -> Self {
        Self {
            lease: None,
            lease_epoch: 0,
            attempt_epoch: [0; ATTEMPTS],
            phase: [Phase::Planned; ATTEMPTS],
            journal: Vec::new(),
            projection: 0,
            completion_count: [0; ATTEMPTS],
            cursors: [0; SESSIONS],
        }
    }
}

impl RecoveryState {
    /// Apply one transition, rejecting invalid, stale, duplicate, or out-of-bound actions.
    pub fn apply(&mut self, action: Action) -> Result<(), &'static str> {
        match action {
            Action::Acquire { attempt } => {
                self.require_attempt(attempt)?;
                if self.lease.is_some() || self.lease_epoch >= MAX_EPOCH {
                    return Err("lease unavailable");
                }
                if self.phase[attempt] != Phase::Planned {
                    return Err("attempt cannot acquire");
                }
                self.lease_epoch += 1;
                self.lease = Some(attempt);
                self.attempt_epoch[attempt] = self.lease_epoch;
            }
            Action::Start { attempt, epoch } => {
                self.require_exact_lease(attempt, epoch)?;
                if self.phase[attempt] != Phase::Planned {
                    return Err("attempt is not planned");
                }
                self.push(attempt, epoch, EventKind::Started)?;
                self.phase[attempt] = Phase::Running;
            }
            Action::Complete { attempt, epoch } => {
                self.require_exact_lease(attempt, epoch)?;
                if self.phase[attempt] != Phase::Running || self.completion_count[attempt] != 0 {
                    return Err("completion is not unique");
                }
                self.push(attempt, epoch, EventKind::Completed)?;
                self.phase[attempt] = Phase::Terminal;
                self.completion_count[attempt] = 1;
                self.lease = None;
            }
            Action::Crash { attempt } => {
                self.require_attempt(attempt)?;
                if self.lease != Some(attempt) || self.phase[attempt] != Phase::Running {
                    return Err("attempt is not running");
                }
                self.phase[attempt] = Phase::Uncertain;
                self.lease = None;
            }
            Action::ReconcileUncertain { attempt } => {
                self.require_attempt(attempt)?;
                if self.lease.is_some()
                    || self.phase[attempt] != Phase::Uncertain
                    || self.completion_count[attempt] != 0
                {
                    return Err("attempt is not unreconciled");
                }
                self.push(attempt, self.attempt_epoch[attempt], EventKind::Uncertain)?;
                self.completion_count[attempt] = 1;
            }
            Action::ProjectNext => {
                if self.projection >= self.journal.len() {
                    return Err("projection has no next event");
                }
                self.projection += 1;
            }
            Action::AdvanceReplay { session } => {
                let Some(cursor) = self.cursors.get_mut(session) else {
                    return Err("session is out of range");
                };
                if *cursor >= MAX_CURSOR {
                    return Err("cursor bound reached");
                }
                *cursor += 1;
            }
        }
        self.validate()
    }

    /// Check all model invariants.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.lease_epoch > MAX_EPOCH
            || self.journal.len() > MAX_JOURNAL
            || self.projection > self.journal.len()
            || self.completion_count.iter().any(|count| *count > 1)
            || self.cursors.iter().any(|cursor| *cursor > MAX_CURSOR)
        {
            return Err("bounded invariant violated");
        }
        if let Some(owner) = self.lease {
            self.require_attempt(owner)?;
            if self.phase[owner] == Phase::Running && self.attempt_epoch[owner] != self.lease_epoch
            {
                return Err("stale running lease");
            }
        }
        for attempt in 0..ATTEMPTS {
            if self.phase[attempt] == Phase::Uncertain && self.lease == Some(attempt) {
                return Err("uncertain attempt owns lease");
            }
        }
        Ok(())
    }

    /// Current global lease owner and epoch.
    #[must_use]
    pub const fn lease(&self) -> (Option<usize>, u8) {
        (self.lease, self.lease_epoch)
    }

    /// Attempt phase.
    #[must_use]
    pub const fn phase(&self, attempt: usize) -> Option<Phase> {
        if attempt < ATTEMPTS {
            Some(self.phase[attempt])
        } else {
            None
        }
    }

    /// Durable journal length and applied projection prefix.
    #[must_use]
    pub const fn journal_projection(&self) -> (usize, usize) {
        (self.journal.len(), self.projection)
    }

    /// Accepted terminal count for an attempt.
    #[must_use]
    pub const fn completion_count(&self, attempt: usize) -> Option<u8> {
        if attempt < ATTEMPTS {
            Some(self.completion_count[attempt])
        } else {
            None
        }
    }

    /// Replay cursor for one session.
    #[must_use]
    pub const fn cursor(&self, session: usize) -> Option<u8> {
        if session < SESSIONS {
            Some(self.cursors[session])
        } else {
            None
        }
    }

    fn require_attempt(&self, attempt: usize) -> Result<(), &'static str> {
        if attempt < ATTEMPTS {
            Ok(())
        } else {
            Err("attempt is out of range")
        }
    }

    fn require_exact_lease(&self, attempt: usize, epoch: u8) -> Result<(), &'static str> {
        self.require_attempt(attempt)?;
        if self.lease == Some(attempt)
            && epoch == self.lease_epoch
            && self.attempt_epoch[attempt] == epoch
        {
            Ok(())
        } else {
            Err("stale or foreign lease")
        }
    }

    fn push(&mut self, attempt: usize, epoch: u8, kind: EventKind) -> Result<(), &'static str> {
        if self.journal.len() >= MAX_JOURNAL {
            return Err("journal bound reached");
        }
        self.journal.push(Event {
            attempt,
            epoch,
            kind,
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_invalid(mutant: RecoveryState) {
        assert!(mutant.validate().is_err());
    }

    #[test]
    fn deliberate_bounded_state_mutations_are_detected() {
        let mut epoch = RecoveryState::default();
        epoch.lease_epoch = MAX_EPOCH + 1;
        assert_invalid(epoch);

        let mut projection = RecoveryState::default();
        projection.projection = 1;
        assert_invalid(projection);

        let mut completion = RecoveryState::default();
        completion.completion_count[0] = 2;
        assert_invalid(completion);

        let mut cursor = RecoveryState::default();
        cursor.cursors[0] = MAX_CURSOR + 1;
        assert_invalid(cursor);

        let mut foreign_owner = RecoveryState::default();
        foreign_owner.lease = Some(ATTEMPTS);
        assert_invalid(foreign_owner);

        let mut stale = RecoveryState::default();
        stale.lease = Some(0);
        stale.lease_epoch = 1;
        stale.phase[0] = Phase::Running;
        assert_invalid(stale);

        let mut uncertain_owner = RecoveryState::default();
        uncertain_owner.lease = Some(0);
        uncertain_owner.phase[0] = Phase::Uncertain;
        assert_invalid(uncertain_owner);
    }

    #[test]
    fn full_journal_rejects_another_event_without_mutation() {
        let mut state = RecoveryState::default();
        state.journal = vec![
            Event {
                attempt: 0,
                epoch: 1,
                kind: EventKind::Started,
            };
            MAX_JOURNAL
        ];
        let before = state.clone();
        assert!(state.push(0, 1, EventKind::Completed).is_err());
        assert_eq!(state, before);
    }
}
