// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Deterministic statistical summaries and evidence-aware SLO assessment.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod comparison;
mod economics;
mod reliability;
mod scoring;

pub use comparison::*;
pub use economics::*;
pub use reliability::*;
pub use scoring::*;

use std::error::Error;
use std::fmt;

const Z_95: f64 = 1.959_963_984_540_054;
const ALPHA_95: f64 = 0.05;

/// Terminal outcome of one attempted operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttemptOutcome {
    /// The operation completed successfully.
    Completed,
    /// The operation returned a failure.
    Failed,
    /// The operation exceeded its deadline.
    TimedOut,
    /// The operation was cancelled.
    Cancelled,
}

/// One terminal observation. Latency is optional evidence, not an assumed zero.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AttemptObservation {
    /// Terminal outcome.
    pub outcome: AttemptOutcome,
    /// Successful latency in nanoseconds, when measured.
    pub latency_ns: Option<u64>,
}

/// Counts retaining every attempted operation in the quality denominator.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AttemptCounts {
    /// All attempts, including failures and censored outcomes.
    pub total: usize,
    /// Successful attempts.
    pub completed: usize,
    /// Explicit failures.
    pub failed: usize,
    /// Deadline expirations.
    pub timed_out: usize,
    /// Cancellations.
    pub cancelled: usize,
    /// Successful attempts for which latency evidence is absent.
    pub missing_success_latency: usize,
}

/// Method used to construct an interval.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IntervalMethod {
    /// Wilson score interval for a binomial proportion.
    WilsonScore,
    /// Student t interval over independent window rates.
    StudentT,
}

/// Point estimate and two-sided 95 percent interval.
///
/// Values are constructible only by this crate's validated statistical
/// routines. In particular, a caller cannot inject a well-shaped but
/// unsupported interval into [`assess_slo`].
///
/// ```compile_fail
/// use asb_analysis::{EstimateInterval, IntervalMethod};
///
/// let forged = EstimateInterval {
///     estimate: 1.0,
///     lower: 1.0,
///     upper: 1.0,
///     method: IntervalMethod::StudentT,
/// };
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EstimateInterval {
    estimate: f64,
    lower: f64,
    upper: f64,
    method: IntervalMethod,
}

impl EstimateInterval {
    /// Return the point estimate.
    pub fn estimate(self) -> f64 {
        self.estimate
    }

    /// Return the inclusive lower confidence bound.
    pub fn lower(self) -> f64 {
        self.lower
    }

    /// Return the inclusive upper confidence bound.
    pub fn upper(self) -> f64 {
        self.upper
    }

    /// Return the interval construction method.
    pub fn method(self) -> IntervalMethod {
        self.method
    }
}

/// Nonparametric p95 estimate and DKW confidence bounds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QuantileInterval {
    /// Point estimate.
    pub estimate: f64,
    /// Inclusive lower bound, using zero when the DKW band reaches the domain floor.
    pub lower: f64,
    /// Inclusive upper bound, absent when the DKW band does not identify a finite bound.
    pub upper: Option<f64>,
}

/// Successful-latency distribution and uncertainty around p95.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LatencySummary {
    /// Number of observed successful latencies.
    pub samples: usize,
    /// Minimum observed latency.
    pub minimum_ns: u64,
    /// Maximum observed latency.
    pub maximum_ns: u64,
    /// Type-7 median.
    pub p50_ns: f64,
    /// Type-7 p95 estimate with a DKW band.
    pub p95_ns: QuantileInterval,
    /// Type-7 p99.
    pub p99_ns: f64,
}

/// Complete attempt analysis. Missing evidence remains explicit.
///
/// The fields are private so callers cannot replace validated Wilson or DKW
/// evidence before assessment.
///
/// ```compile_fail
/// use asb_analysis::{AttemptAnalysis, AttemptCounts};
///
/// let forged = AttemptAnalysis {
///     counts: AttemptCounts::default(),
///     quality: None,
///     latency: None,
/// };
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AttemptAnalysis {
    counts: AttemptCounts,
    quality: Option<EstimateInterval>,
    latency: Option<LatencySummary>,
}

impl AttemptAnalysis {
    /// Return outcome and missing-evidence counts.
    pub fn counts(self) -> AttemptCounts {
        self.counts
    }

    /// Return the successful-proportion interval, if any attempts exist.
    pub fn quality(self) -> Option<EstimateInterval> {
        self.quality
    }

    /// Return the successful-latency summary, if any latency was measured.
    pub fn latency(self) -> Option<LatencySummary> {
        self.latency
    }
}

/// One fixed-duration throughput window.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ThroughputWindow {
    /// Successful completions observed in the window.
    pub successful_completions: u64,
    /// Measured window duration in nanoseconds.
    pub duration_ns: u64,
}

/// One criterion decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CriterionDecision {
    /// The criterion is not configured.
    NotRequired,
    /// The complete confidence interval satisfies the bound.
    Pass,
    /// The complete confidence interval violates the bound.
    Fail,
    /// Evidence is absent or the interval straddles the bound.
    Inconclusive,
}

/// Required SLO bounds. At least one bound must be configured.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SloPolicy {
    /// Minimum successful-attempt proportion in the closed unit interval.
    pub minimum_quality: Option<f64>,
    /// Maximum successful p95 latency in nanoseconds.
    pub maximum_p95_latency_ns: Option<u64>,
    /// Minimum successful completions per second.
    pub minimum_throughput_per_second: Option<f64>,
}

/// Per-criterion and aggregate SLO result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SloAssessment {
    /// Quality criterion result.
    pub quality: CriterionDecision,
    /// Latency criterion result.
    pub latency: CriterionDecision,
    /// Throughput criterion result.
    pub throughput: CriterionDecision,
    /// Failure dominates, all required passes pass, otherwise inconclusive.
    pub overall: CriterionDecision,
}

/// Invalid analysis input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AnalysisError {
    /// A throughput window has zero duration.
    ZeroDuration,
    /// Repeated throughput windows do not have one common duration.
    UnequalWindowDuration,
    /// A policy bound is non-finite, out of range, or absent.
    InvalidPolicy,
    /// Supplied summary or interval evidence is internally inconsistent.
    InvalidEvidence,
}

impl fmt::Display for AnalysisError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroDuration => formatter.write_str("throughput duration must be positive"),
            Self::UnequalWindowDuration => {
                formatter.write_str("throughput windows must have equal durations")
            }
            Self::InvalidPolicy => formatter.write_str("SLO policy is invalid"),
            Self::InvalidEvidence => formatter.write_str("analysis evidence is invalid"),
        }
    }
}

impl Error for AnalysisError {}

/// Analyze all attempts without dropping failed or censored outcomes.
pub fn analyze_attempts(observations: &[AttemptObservation]) -> AttemptAnalysis {
    let mut counts = AttemptCounts {
        total: observations.len(),
        ..AttemptCounts::default()
    };
    let mut latencies = Vec::new();
    for observation in observations {
        match observation.outcome {
            AttemptOutcome::Completed => {
                counts.completed += 1;
                if let Some(latency) = observation.latency_ns {
                    latencies.push(latency);
                } else {
                    counts.missing_success_latency += 1;
                }
            }
            AttemptOutcome::Failed => counts.failed += 1,
            AttemptOutcome::TimedOut => counts.timed_out += 1,
            AttemptOutcome::Cancelled => counts.cancelled += 1,
        }
    }
    latencies.sort_unstable();
    AttemptAnalysis {
        counts,
        quality: wilson_95(counts.completed, counts.total),
        latency: latency_summary(&latencies),
    }
}

/// Compute the type-7 empirical quantile used by R and NumPy defaults.
pub fn type7_quantile(sorted: &[u64], probability: f64) -> Option<f64> {
    if sorted.is_empty()
        || sorted.windows(2).any(|pair| pair[0] > pair[1])
        || !probability.is_finite()
        || !(0.0..=1.0).contains(&probability)
    {
        return None;
    }
    let position = (sorted.len() - 1) as f64 * probability;
    let lower = position.floor() as usize;
    let upper = position.ceil() as usize;
    let weight = position - lower as f64;
    Some(sorted[lower] as f64 + (sorted[upper] as f64 - sorted[lower] as f64) * weight)
}

/// Compute a two-sided 95 percent Wilson interval.
pub fn wilson_95(successes: usize, total: usize) -> Option<EstimateInterval> {
    if total == 0 || successes > total {
        return None;
    }
    let n = total as f64;
    let estimate = successes as f64 / n;
    let z2 = Z_95 * Z_95;
    let denominator = 1.0 + z2 / n;
    let center = (estimate + z2 / (2.0 * n)) / denominator;
    let radius = Z_95 * (estimate * (1.0 - estimate) / n + z2 / (4.0 * n * n)).sqrt() / denominator;
    Some(EstimateInterval {
        estimate,
        lower: (center - radius).max(0.0).min(estimate),
        upper: (center + radius).min(1.0).max(estimate),
        method: IntervalMethod::WilsonScore,
    })
}

fn latency_summary(sorted: &[u64]) -> Option<LatencySummary> {
    let samples = sorted.len();
    let estimate = type7_quantile(sorted, 0.95)?;
    let epsilon = ((2.0 / ALPHA_95).ln() / (2.0 * samples as f64)).sqrt();
    Some(LatencySummary {
        samples,
        minimum_ns: sorted[0],
        maximum_ns: sorted[samples - 1],
        p50_ns: type7_quantile(sorted, 0.5)?,
        p95_ns: QuantileInterval {
            estimate,
            lower: if 0.95 <= epsilon {
                0.0
            } else {
                type7_quantile(sorted, 0.95 - epsilon)?
            },
            upper: if 0.95 + epsilon >= 1.0 {
                None
            } else {
                type7_quantile(sorted, 0.95 + epsilon)
            },
        },
        p99_ns: type7_quantile(sorted, 0.99)?,
    })
}

/// Estimate successful throughput across independent fixed-duration windows.
///
/// Fewer than two windows are inconclusive. Degrees of freedom above 30 use
/// the conservative 30-degree-of-freedom critical value.
pub fn throughput_95(
    windows: &[ThroughputWindow],
) -> Result<Option<EstimateInterval>, AnalysisError> {
    if windows.iter().any(|window| window.duration_ns == 0) {
        return Err(AnalysisError::ZeroDuration);
    }
    if windows.first().is_some_and(|first| {
        windows
            .iter()
            .any(|window| window.duration_ns != first.duration_ns)
    }) {
        return Err(AnalysisError::UnequalWindowDuration);
    }
    if windows.len() < 2 {
        return Ok(None);
    }
    let rates: Vec<f64> = windows
        .iter()
        .map(|window| {
            window.successful_completions as f64 * 1_000_000_000.0 / window.duration_ns as f64
        })
        .collect();
    let estimate = rates.iter().sum::<f64>() / rates.len() as f64;
    let squared_error = rates
        .iter()
        .map(|rate| (rate - estimate) * (rate - estimate))
        .sum::<f64>();
    let standard_error =
        (squared_error / (rates.len() - 1) as f64).sqrt() / (rates.len() as f64).sqrt();
    let radius = t_critical_95(rates.len() - 1) * standard_error;
    Ok(Some(EstimateInterval {
        estimate,
        lower: (estimate - radius).max(0.0),
        upper: estimate + radius,
        method: IntervalMethod::StudentT,
    }))
}

fn t_critical_95(degrees_of_freedom: usize) -> f64 {
    const VALUES: [f64; 30] = [
        12.706_204_736_432_095,
        4.302_652_729_696_142,
        3.182_446_305_284_263,
        2.776_445_105_197_799,
        2.570_581_835_636_314,
        2.446_911_851_144_969,
        2.364_624_251_592_784,
        2.306_004_135_204_166,
        2.262_157_162_854_099,
        2.228_138_851_964_939,
        2.200_985_160_082_949,
        2.178_812_829_663_418,
        2.160_368_656_461_013,
        2.144_786_687_916_927,
        2.131_449_545_559_323,
        2.119_905_299_221_011,
        2.109_815_577_833_181,
        2.100_922_040_240_96,
        2.093_024_054_408_263,
        2.085_963_447_265_836,
        2.079_613_844_727_662,
        2.073_873_067_904_015,
        2.068_657_610_419_041,
        2.063_898_561_628_021,
        2.059_538_552_753_294,
        2.055_529_438_642_871,
        2.051_830_516_480_283,
        2.048_407_141_795_244,
        2.045_229_642_132_703,
        2.042_272_456_301_237,
    ];
    VALUES
        .get(degrees_of_freedom - 1)
        .copied()
        .unwrap_or(VALUES[VALUES.len() - 1])
}

/// Assess configured SLOs using complete confidence bounds.
pub fn assess_slo(
    analysis: &AttemptAnalysis,
    throughput: Option<EstimateInterval>,
    policy: SloPolicy,
) -> Result<SloAssessment, AnalysisError> {
    validate_policy(policy)?;
    validate_evidence(analysis, throughput)?;
    let quality = match policy.minimum_quality {
        None => CriterionDecision::NotRequired,
        Some(bound) => minimum_decision(analysis.quality, bound),
    };
    let latency = match policy.maximum_p95_latency_ns {
        None => CriterionDecision::NotRequired,
        Some(bound) => match analysis.latency {
            None => CriterionDecision::Inconclusive,
            Some(_) if analysis.counts.missing_success_latency > 0 => {
                CriterionDecision::Inconclusive
            }
            Some(summary) if summary.p95_ns.lower > bound as f64 => CriterionDecision::Fail,
            Some(summary)
                if summary
                    .p95_ns
                    .upper
                    .is_some_and(|upper| upper <= bound as f64) =>
            {
                CriterionDecision::Pass
            }
            Some(_) => CriterionDecision::Inconclusive,
        },
    };
    let throughput = match policy.minimum_throughput_per_second {
        None => CriterionDecision::NotRequired,
        Some(bound) => minimum_decision(throughput, bound),
    };
    let decisions = [quality, latency, throughput];
    let overall = if decisions.contains(&CriterionDecision::Fail) {
        CriterionDecision::Fail
    } else if decisions.iter().all(|decision| {
        matches!(
            decision,
            CriterionDecision::Pass | CriterionDecision::NotRequired
        )
    }) {
        CriterionDecision::Pass
    } else {
        CriterionDecision::Inconclusive
    };
    Ok(SloAssessment {
        quality,
        latency,
        throughput,
        overall,
    })
}

fn validate_evidence(
    analysis: &AttemptAnalysis,
    throughput: Option<EstimateInterval>,
) -> Result<(), AnalysisError> {
    let counts = analysis.counts;
    let classified = counts
        .completed
        .checked_add(counts.failed)
        .and_then(|value| value.checked_add(counts.timed_out))
        .and_then(|value| value.checked_add(counts.cancelled));
    if classified != Some(counts.total) || counts.missing_success_latency > counts.completed {
        return Err(AnalysisError::InvalidEvidence);
    }
    if analysis.quality.is_some() != (counts.total > 0)
        || analysis.quality.is_some_and(|value| {
            value.method != IntervalMethod::WilsonScore
                || !valid_interval(value)
                || value.lower < 0.0
                || value.upper > 1.0
                || (value.estimate - counts.completed as f64 / counts.total as f64).abs()
                    > f64::EPSILON
        })
    {
        return Err(AnalysisError::InvalidEvidence);
    }
    match analysis.latency {
        None if counts.completed != counts.missing_success_latency => {
            return Err(AnalysisError::InvalidEvidence);
        }
        Some(summary)
            if summary.samples
                != counts
                    .completed
                    .saturating_sub(counts.missing_success_latency)
                || summary.samples == 0
                || summary.minimum_ns > summary.maximum_ns
                || !within_latency_range(summary.p50_ns, summary)
                || !within_latency_range(summary.p95_ns.estimate, summary)
                || !within_latency_range(summary.p99_ns, summary)
                || summary.p50_ns > summary.p95_ns.estimate
                || summary.p95_ns.estimate > summary.p99_ns
                || !valid_quantile_interval(summary.p95_ns) =>
        {
            return Err(AnalysisError::InvalidEvidence);
        }
        _ => {}
    }
    if throughput.is_some_and(|value| {
        value.method != IntervalMethod::StudentT || !valid_interval(value) || value.lower < 0.0
    }) {
        return Err(AnalysisError::InvalidEvidence);
    }
    Ok(())
}

fn within_latency_range(value: f64, summary: LatencySummary) -> bool {
    value.is_finite() && value >= summary.minimum_ns as f64 && value <= summary.maximum_ns as f64
}

fn valid_quantile_interval(value: QuantileInterval) -> bool {
    value.estimate.is_finite()
        && value.lower.is_finite()
        && value.lower >= 0.0
        && value.lower <= value.estimate
        && value
            .upper
            .is_none_or(|upper| upper.is_finite() && value.estimate <= upper)
}

fn valid_interval(value: EstimateInterval) -> bool {
    value.estimate.is_finite()
        && value.lower.is_finite()
        && value.upper.is_finite()
        && value.lower <= value.estimate
        && value.estimate <= value.upper
}

fn minimum_decision(interval: Option<EstimateInterval>, bound: f64) -> CriterionDecision {
    match interval {
        None => CriterionDecision::Inconclusive,
        Some(value) if value.lower >= bound => CriterionDecision::Pass,
        Some(value) if value.upper < bound => CriterionDecision::Fail,
        Some(_) => CriterionDecision::Inconclusive,
    }
}

fn validate_policy(policy: SloPolicy) -> Result<(), AnalysisError> {
    if policy.minimum_quality.is_none()
        && policy.maximum_p95_latency_ns.is_none()
        && policy.minimum_throughput_per_second.is_none()
    {
        return Err(AnalysisError::InvalidPolicy);
    }
    if policy
        .minimum_quality
        .is_some_and(|value| !value.is_finite() || !(0.0..=1.0).contains(&value))
        || policy
            .minimum_throughput_per_second
            .is_some_and(|value| !value.is_finite() || value < 0.0)
    {
        return Err(AnalysisError::InvalidPolicy);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 1e-8, "{actual} != {expected}");
    }

    #[test]
    fn checked_in_reference_vectors_match() {
        for line in include_str!("../tests/fixtures/reference-vectors.tsv").lines() {
            if line.starts_with('#') {
                continue;
            }
            let fields: Vec<&str> = line.split('\t').collect();
            let expected_estimate: f64 = fields[3].parse().unwrap();
            let expected_lower: f64 = fields[4].parse().unwrap();
            let expected_upper: f64 = fields[5].parse().unwrap();
            let parameter: f64 = fields[2].parse().unwrap();
            if fields[0] != "quantile" {
                close(parameter, 0.95);
            }
            match fields[0] {
                "quantile" => {
                    let values: Vec<u64> = fields[1]
                        .split(',')
                        .map(|value| value.parse().unwrap())
                        .collect();
                    let actual = type7_quantile(&values, parameter).unwrap();
                    close(actual, expected_estimate);
                    close(actual, expected_lower);
                    close(actual, expected_upper);
                }
                "wilson" => {
                    let counts: Vec<usize> = fields[1]
                        .split(',')
                        .map(|value| value.parse().unwrap())
                        .collect();
                    let actual = wilson_95(counts[0], counts[1]).unwrap();
                    close(actual.estimate, expected_estimate);
                    close(actual.lower, expected_lower);
                    close(actual.upper, expected_upper);
                }
                "throughput" => {
                    let windows: Vec<ThroughputWindow> = fields[1]
                        .split(',')
                        .map(|value| ThroughputWindow {
                            successful_completions: value.parse().unwrap(),
                            duration_ns: 1_000_000_000,
                        })
                        .collect();
                    let actual = throughput_95(&windows).unwrap().unwrap();
                    close(actual.estimate, expected_estimate);
                    close(actual.lower, expected_lower);
                    close(actual.upper, expected_upper);
                }
                kind => panic!("unknown fixture kind {kind}"),
            }
        }
    }

    #[test]
    fn checked_in_dkw_reference_vectors_match() {
        for line in include_str!("../tests/fixtures/dkw-reference-vectors.tsv").lines() {
            if line.starts_with('#') {
                continue;
            }
            let fields: Vec<&str> = line.split('\t').collect();
            let range: Vec<u64> = fields[0]
                .strip_prefix("range:")
                .unwrap()
                .split(':')
                .map(|value| value.parse().unwrap())
                .collect();
            let confidence: f64 = fields[2].parse().unwrap();
            assert_eq!(confidence, 0.95);
            let observations: Vec<AttemptObservation> = (range[0]..=range[1])
                .map(|latency_ns| AttemptObservation {
                    outcome: AttemptOutcome::Completed,
                    latency_ns: Some(latency_ns),
                })
                .collect();
            let actual = analyze_attempts(&observations).latency.unwrap().p95_ns;
            close(actual.estimate, fields[3].parse().unwrap());
            close(actual.lower, fields[4].parse().unwrap());
            match fields[5] {
                "none" => assert_eq!(actual.upper, None),
                value => close(actual.upper.unwrap(), value.parse().unwrap()),
            }
        }
    }

    #[test]
    fn failures_and_censoring_remain_in_quality_denominator() {
        let analysis = analyze_attempts(&[
            AttemptObservation {
                outcome: AttemptOutcome::Completed,
                latency_ns: Some(10),
            },
            AttemptObservation {
                outcome: AttemptOutcome::Failed,
                latency_ns: Some(20),
            },
            AttemptObservation {
                outcome: AttemptOutcome::TimedOut,
                latency_ns: None,
            },
            AttemptObservation {
                outcome: AttemptOutcome::Cancelled,
                latency_ns: None,
            },
        ]);
        assert_eq!(analysis.counts.total, 4);
        assert_eq!(analysis.counts.completed, 1);
        assert_eq!(analysis.counts.failed, 1);
        assert_eq!(analysis.counts.timed_out, 1);
        assert_eq!(analysis.counts.cancelled, 1);
        close(analysis.quality.unwrap().estimate, 0.25);
        assert_eq!(analysis.latency.unwrap().samples, 1);

        let counts = analysis.counts();
        assert_eq!(counts.total, 4);
        let quality = analysis.quality().unwrap();
        close(quality.estimate(), 0.25);
        assert!(quality.lower() < quality.estimate());
        assert!(quality.upper() > quality.estimate());
        assert_eq!(quality.method(), IntervalMethod::WilsonScore);
        assert_eq!(analysis.latency().unwrap().samples, 1);
    }

    #[test]
    fn sparse_missing_and_decisive_failure_behave_conservatively() {
        let policy = SloPolicy {
            minimum_quality: Some(0.5),
            maximum_p95_latency_ns: Some(100),
            minimum_throughput_per_second: None,
        };
        let empty = assess_slo(&analyze_attempts(&[]), None, policy).unwrap();
        assert_eq!(empty.overall, CriterionDecision::Inconclusive);

        let failures = vec![
            AttemptObservation {
                outcome: AttemptOutcome::Failed,
                latency_ns: None,
            };
            20
        ];
        let failed = assess_slo(&analyze_attempts(&failures), None, policy).unwrap();
        assert_eq!(failed.quality, CriterionDecision::Fail);
        assert_eq!(failed.latency, CriterionDecision::Inconclusive);
        assert_eq!(failed.overall, CriterionDecision::Fail);

        let missing = analyze_attempts(&[AttemptObservation {
            outcome: AttemptOutcome::Completed,
            latency_ns: None,
        }]);
        let latency_only = assess_slo(
            &missing,
            None,
            SloPolicy {
                minimum_quality: None,
                maximum_p95_latency_ns: Some(100),
                minimum_throughput_per_second: None,
            },
        )
        .unwrap();
        assert_eq!(latency_only.latency, CriterionDecision::Inconclusive);
    }

    #[test]
    fn invalid_inputs_fail_closed() {
        assert_eq!(wilson_95(2, 1), None);
        assert_eq!(type7_quantile(&[], 0.5), None);
        assert_eq!(type7_quantile(&[2, 1], 0.5), None);
        assert_eq!(type7_quantile(&[1], -0.1), None);
        assert_eq!(type7_quantile(&[1], 1.1), None);
        assert_eq!(type7_quantile(&[1], f64::NAN), None);
        assert_eq!(
            throughput_95(&[ThroughputWindow {
                successful_completions: 1,
                duration_ns: 0,
            }]),
            Err(AnalysisError::ZeroDuration)
        );
        assert_eq!(
            throughput_95(&[
                ThroughputWindow {
                    successful_completions: 1,
                    duration_ns: 1,
                },
                ThroughputWindow {
                    successful_completions: 1,
                    duration_ns: 2,
                },
            ]),
            Err(AnalysisError::UnequalWindowDuration)
        );
        let analysis = analyze_attempts(&[]);
        for policy in [
            SloPolicy {
                minimum_quality: None,
                maximum_p95_latency_ns: None,
                minimum_throughput_per_second: None,
            },
            SloPolicy {
                minimum_quality: Some(f64::NAN),
                maximum_p95_latency_ns: None,
                minimum_throughput_per_second: None,
            },
            SloPolicy {
                minimum_quality: Some(1.1),
                maximum_p95_latency_ns: None,
                minimum_throughput_per_second: None,
            },
            SloPolicy {
                minimum_quality: None,
                maximum_p95_latency_ns: None,
                minimum_throughput_per_second: Some(-1.0),
            },
        ] {
            assert_eq!(
                assess_slo(&analysis, None, policy),
                Err(AnalysisError::InvalidPolicy)
            );
        }
    }

    #[test]
    fn externally_constructed_evidence_is_validated() {
        let analysis = analyze_attempts(&[AttemptObservation {
            outcome: AttemptOutcome::Completed,
            latency_ns: Some(10),
        }]);
        let invalid = EstimateInterval {
            estimate: f64::NAN,
            lower: 0.0,
            upper: 1.0,
            method: IntervalMethod::StudentT,
        };
        assert_eq!(
            assess_slo(
                &analysis,
                Some(invalid),
                SloPolicy {
                    minimum_quality: None,
                    maximum_p95_latency_ns: None,
                    minimum_throughput_per_second: Some(1.0),
                },
            ),
            Err(AnalysisError::InvalidEvidence)
        );

        let mut inconsistent = analysis;
        inconsistent.counts.failed = 1;
        assert_eq!(
            assess_slo(
                &inconsistent,
                None,
                SloPolicy {
                    minimum_quality: Some(0.5),
                    maximum_p95_latency_ns: None,
                    minimum_throughput_per_second: None,
                },
            ),
            Err(AnalysisError::InvalidEvidence)
        );

        let mut misordered = analysis;
        let latency = misordered.latency.as_mut().unwrap();
        latency.p50_ns = latency.p95_ns.estimate + 1.0;
        assert_eq!(
            assess_slo(
                &misordered,
                None,
                SloPolicy {
                    minimum_quality: None,
                    maximum_p95_latency_ns: Some(10),
                    minimum_throughput_per_second: None,
                },
            ),
            Err(AnalysisError::InvalidEvidence)
        );
    }

    #[test]
    fn missing_success_latency_prevents_a_decisive_latency_result() {
        let mut observations = vec![
            AttemptObservation {
                outcome: AttemptOutcome::Completed,
                latency_ns: Some(1_000),
            };
            1_000
        ];
        observations.push(AttemptObservation {
            outcome: AttemptOutcome::Completed,
            latency_ns: None,
        });
        let assessment = assess_slo(
            &analyze_attempts(&observations),
            None,
            SloPolicy {
                minimum_quality: None,
                maximum_p95_latency_ns: Some(100),
                minimum_throughput_per_second: None,
            },
        )
        .unwrap();
        assert_eq!(assessment.latency, CriterionDecision::Inconclusive);
        assert_eq!(assessment.overall, CriterionDecision::Inconclusive);
    }

    #[test]
    fn complete_bounds_drive_pass_fail_and_inconclusive() {
        let completed = vec![
            AttemptObservation {
                outcome: AttemptOutcome::Completed,
                latency_ns: Some(10),
            };
            1_000
        ];
        let analysis = analyze_attempts(&completed);
        let passing_rate = EstimateInterval {
            estimate: 12.0,
            lower: 11.0,
            upper: 13.0,
            method: IntervalMethod::StudentT,
        };
        let pass = assess_slo(
            &analysis,
            Some(passing_rate),
            SloPolicy {
                minimum_quality: Some(0.9),
                maximum_p95_latency_ns: Some(10),
                minimum_throughput_per_second: Some(10.0),
            },
        )
        .unwrap();
        assert_eq!(pass.overall, CriterionDecision::Pass);

        let straddling_rate = EstimateInterval {
            estimate: 10.0,
            lower: 9.0,
            upper: 11.0,
            method: IntervalMethod::StudentT,
        };
        assert_eq!(
            assess_slo(
                &analysis,
                Some(straddling_rate),
                SloPolicy {
                    minimum_quality: None,
                    maximum_p95_latency_ns: None,
                    minimum_throughput_per_second: Some(10.0),
                },
            )
            .unwrap()
            .overall,
            CriterionDecision::Inconclusive
        );

        let failing_rate = EstimateInterval {
            estimate: 8.0,
            lower: 7.0,
            upper: 9.0,
            method: IntervalMethod::StudentT,
        };
        assert_eq!(
            assess_slo(
                &analysis,
                Some(failing_rate),
                SloPolicy {
                    minimum_quality: None,
                    maximum_p95_latency_ns: None,
                    minimum_throughput_per_second: Some(10.0),
                },
            )
            .unwrap()
            .overall,
            CriterionDecision::Fail
        );
    }

    #[test]
    fn dkw_band_matches_the_published_closed_form() {
        let observations: Vec<AttemptObservation> = (1..=100)
            .map(|latency_ns| AttemptObservation {
                outcome: AttemptOutcome::Completed,
                latency_ns: Some(latency_ns),
            })
            .collect();
        let summary = analyze_attempts(&observations).latency.unwrap();
        let epsilon = ((2.0_f64 / 0.05).ln() / 200.0).sqrt();
        close(
            summary.p95_ns.lower,
            type7_quantile(&(1..=100).collect::<Vec<_>>(), 0.95 - epsilon).unwrap(),
        );
        assert_eq!(summary.p95_ns.upper, None);
    }

    #[test]
    fn large_window_sets_use_the_documented_conservative_critical_value() {
        let windows = vec![
            ThroughputWindow {
                successful_completions: 10,
                duration_ns: 1_000_000_000,
            };
            32
        ];
        let interval = throughput_95(&windows).unwrap().unwrap();
        assert_eq!(interval.estimate, 10.0);
        assert_eq!(interval.lower, 10.0);
        assert_eq!(interval.upper, 10.0);
    }

    #[test]
    fn exhaustive_wilson_bounds_are_ordered_and_monotone() {
        for total in 1..=100 {
            let mut previous_lower = 0.0;
            for successes in 0..=total {
                let interval = wilson_95(successes, total).unwrap();
                assert!((0.0..=1.0).contains(&interval.lower));
                assert!((0.0..=1.0).contains(&interval.upper));
                assert!(interval.lower <= interval.estimate);
                assert!(interval.estimate <= interval.upper);
                assert!(interval.lower >= previous_lower);
                previous_lower = interval.lower;
            }
        }
    }

    #[test]
    fn ordering_does_not_change_analysis() {
        let forward = [
            AttemptObservation {
                outcome: AttemptOutcome::Completed,
                latency_ns: Some(5),
            },
            AttemptObservation {
                outcome: AttemptOutcome::Completed,
                latency_ns: Some(1),
            },
            AttemptObservation {
                outcome: AttemptOutcome::Failed,
                latency_ns: None,
            },
        ];
        assert_eq!(
            analyze_attempts(&forward),
            analyze_attempts(&[forward[2], forward[1], forward[0]])
        );
    }

    #[test]
    fn nonmonotonic_points_are_assessed_independently() {
        let policy = SloPolicy {
            minimum_quality: Some(0.5),
            maximum_p95_latency_ns: None,
            minimum_throughput_per_second: None,
        };
        let point = |successes| {
            let mut attempts = vec![
                AttemptObservation {
                    outcome: AttemptOutcome::Failed,
                    latency_ns: None,
                };
                100
            ];
            for attempt in attempts.iter_mut().take(successes) {
                attempt.outcome = AttemptOutcome::Completed;
                attempt.latency_ns = Some(1);
            }
            assess_slo(&analyze_attempts(&attempts), None, policy)
                .unwrap()
                .quality
        };
        assert_eq!(
            [point(90), point(10), point(90)],
            [
                CriterionDecision::Pass,
                CriterionDecision::Fail,
                CriterionDecision::Pass
            ]
        );
    }

    #[test]
    fn throughput_requires_repetition_and_constant_windows_are_exact() {
        assert_eq!(
            throughput_95(&[ThroughputWindow {
                successful_completions: 10,
                duration_ns: 1_000_000_000,
            }])
            .unwrap(),
            None
        );
        let interval = throughput_95(&[
            ThroughputWindow {
                successful_completions: 10,
                duration_ns: 1_000_000_000,
            },
            ThroughputWindow {
                successful_completions: 10,
                duration_ns: 1_000_000_000,
            },
        ])
        .unwrap()
        .unwrap();
        assert_eq!(interval.lower, 10.0);
        assert_eq!(interval.upper, 10.0);
    }
}
