// SPDX-License-Identifier: MIT
//! Checked domain primitives for benchmark experiments.
mod budget;

pub use budget::*;

use std::num::NonZeroU32;

/// A validated inclusive concurrency range.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConcurrencyRange {
    first: NonZeroU32,
    last: NonZeroU32,
}

/// Rejected experiment configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigError {
    /// Zero workers cannot form a benchmark experiment.
    ZeroConcurrency,
    /// The upper concurrency bound is below the lower bound.
    ReversedRange,
}

impl ConcurrencyRange {
    /// Validate bounds without allocating workers.
    pub fn new(first: u32, last: u32) -> Result<Self, ConfigError> {
        let first = NonZeroU32::new(first).ok_or(ConfigError::ZeroConcurrency)?;
        let last = NonZeroU32::new(last).ok_or(ConfigError::ZeroConcurrency)?;
        if first > last {
            return Err(ConfigError::ReversedRange);
        }
        Ok(Self { first, last })
    }

    /// Return the inclusive bounds.
    pub fn bounds(self) -> (u32, u32) {
        (self.first.get(), self.last.get())
    }
}

/// The outcome of a required service-level measurement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Assessment {
    /// Sufficient evidence establishes compliance.
    Pass,
    /// Sufficient evidence establishes a violation.
    Fail,
    /// Required evidence is absent or insufficient.
    Inconclusive,
}

/// Aggregate required bounds; missing measurements never establish compliance.
pub fn assess_all(measurements: &[Assessment]) -> Assessment {
    if measurements.contains(&Assessment::Fail) {
        Assessment::Fail
    } else if measurements.is_empty() || measurements.contains(&Assessment::Inconclusive) {
        Assessment::Inconclusive
    } else {
        Assessment::Pass
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_workers_are_rejected() {
        assert_eq!(
            ConcurrencyRange::new(0, 1),
            Err(ConfigError::ZeroConcurrency)
        );
        assert_eq!(
            ConcurrencyRange::new(1, 0),
            Err(ConfigError::ZeroConcurrency)
        );
    }

    #[test]
    fn reversed_range_is_rejected() {
        assert_eq!(ConcurrencyRange::new(8, 2), Err(ConfigError::ReversedRange));
    }

    #[test]
    fn full_integer_domain_is_representable_without_overflow() {
        assert_eq!(
            ConcurrencyRange::new(1, u32::MAX).map(ConcurrencyRange::bounds),
            Ok((1, u32::MAX))
        );
    }

    #[test]
    fn one_worker_is_a_valid_baseline() {
        assert_eq!(
            ConcurrencyRange::new(1, 1).map(ConcurrencyRange::bounds),
            Ok((1, 1))
        );
    }

    #[test]
    fn empty_evidence_is_inconclusive() {
        assert_eq!(assess_all(&[]), Assessment::Inconclusive);
    }

    #[test]
    fn absent_metrics_do_not_pass() {
        assert_eq!(
            assess_all(&[Assessment::Pass, Assessment::Inconclusive]),
            Assessment::Inconclusive
        );
    }

    #[test]
    fn any_failure_dominates_incomplete_evidence() {
        assert_eq!(
            assess_all(&[Assessment::Inconclusive, Assessment::Fail]),
            Assessment::Fail
        );
    }

    #[test]
    fn all_required_bounds_must_pass() {
        assert_eq!(
            assess_all(&[Assessment::Pass, Assessment::Pass]),
            Assessment::Pass
        );
    }
}
