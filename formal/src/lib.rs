// SPDX-License-Identifier: MIT
#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Small executable and machine-checked models for ASB safety boundaries.

/// Convert a microsecond kernel counter to nanoseconds without wrapping.
///
/// This mirrors the checked conversion used by the portable metric collector.
#[must_use]
pub const fn microseconds_to_nanoseconds(value: u64) -> Option<u64> {
    value.checked_mul(1_000)
}

/// Abstract state for one durable attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttemptState {
    /// The attempt has not started.
    Planned,
    /// External execution is active.
    Running,
    /// Collection follows a completed execution effect.
    Collecting,
    /// Exactly one terminal record has been accepted.
    Terminal,
}

/// Abstract attempt transition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttemptEvent {
    /// Start external execution.
    Start,
    /// Begin result collection.
    Collect,
    /// Commit the terminal record.
    Complete,
}

/// Apply one fail-closed attempt transition.
pub const fn transition_attempt(state: AttemptState, event: AttemptEvent) -> Option<AttemptState> {
    match (state, event) {
        (AttemptState::Planned, AttemptEvent::Start) => Some(AttemptState::Running),
        (AttemptState::Running, AttemptEvent::Collect) => Some(AttemptState::Collecting),
        (AttemptState::Collecting, AttemptEvent::Complete) => Some(AttemptState::Terminal),
        _ => None,
    }
}

/// Two independently owned replay cursors.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ReplayCursors {
    cursors: [u8; 2],
}

impl ReplayCursors {
    /// Return a session cursor.
    #[must_use]
    pub const fn cursor(&self, session: usize) -> Option<u8> {
        if session < self.cursors.len() {
            Some(self.cursors[session])
        } else {
            None
        }
    }

    /// Advance only the selected session cursor.
    pub fn advance(&mut self, session: usize) -> bool {
        let Some(cursor) = self.cursors.get_mut(session) else {
            return false;
        };
        let Some(next) = cursor.checked_add(1) else {
            return false;
        };
        *cursor = next;
        true
    }
}

#[cfg(kani)]
mod proofs {
    use super::*;
    use asb_analysis::{
        AttemptObservation, AttemptOutcome, CriterionDecision, SloPolicy, analyze_attempts,
        assess_slo,
    };
    use asb_core::{Assessment, ConcurrencyRange, assess_all};

    #[kani::proof]
    fn concurrency_range_preserves_all_u32_bounds() {
        let first: u32 = kani::any();
        let last: u32 = kani::any();
        match ConcurrencyRange::new(first, last) {
            Ok(range) => {
                assert!(first > 0);
                assert!(first <= last);
                assert_eq!(range.bounds(), (first, last));
            }
            Err(_) => assert!(first == 0 || last == 0 || first > last),
        }
    }

    fn assessment(raw: u8) -> Assessment {
        match raw {
            0 => Assessment::Pass,
            1 => Assessment::Fail,
            _ => Assessment::Inconclusive,
        }
    }

    #[kani::proof]
    fn aggregate_slo_decision_is_fail_closed() {
        let raw: [u8; 3] = kani::any();
        kani::assume(raw.iter().all(|value| *value < 3));
        let decisions = raw.map(assessment);
        let result = assess_all(&decisions);
        assert_eq!(
            result == Assessment::Fail,
            decisions.contains(&Assessment::Fail)
        );
        assert_eq!(
            result == Assessment::Pass,
            decisions.iter().all(|value| *value == Assessment::Pass)
        );
    }

    #[kani::proof]
    fn missing_success_latency_cannot_pass_latency_slo() {
        let maximum: u64 = kani::any();
        let analysis = analyze_attempts(&[AttemptObservation {
            outcome: AttemptOutcome::Completed,
            latency_ns: None,
        }]);
        let assessment = assess_slo(
            &analysis,
            None,
            SloPolicy {
                minimum_quality: None,
                maximum_p95_latency_ns: Some(maximum),
                minimum_throughput_per_second: None,
            },
        )
        .unwrap();
        assert_eq!(assessment.latency, CriterionDecision::Inconclusive);
        assert_ne!(assessment.overall, CriterionDecision::Pass);
    }

    #[kani::proof]
    fn unit_conversion_is_exact_for_every_u32_counter() {
        let value: u32 = kani::any();
        let nanoseconds = microseconds_to_nanoseconds(u64::from(value)).unwrap();
        assert_eq!(nanoseconds, u64::from(value) * 1_000);
        assert!(microseconds_to_nanoseconds(u64::MAX).is_none());
    }

    #[kani::proof]
    fn session_cursor_advance_cannot_cross_talk() {
        let first: u8 = kani::any();
        let second: u8 = kani::any();
        let mut cursors = ReplayCursors {
            cursors: [first, second],
        };
        let advanced = cursors.advance(0);
        assert_eq!(cursors.cursor(1), Some(second));
        assert_eq!(advanced, first < u8::MAX);
        if advanced {
            assert_eq!(cursors.cursor(0), Some(first + 1));
        } else {
            assert_eq!(cursors.cursor(0), Some(first));
        }
    }
}
