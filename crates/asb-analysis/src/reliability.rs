// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Repeated-trial reliability and mixed-load fairness reports.

use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
};

/// Maximum attempts represented by one trial.
pub const MAX_ATTEMPTS_PER_TRIAL: u32 = 64;
/// Maximum observations accepted by one report.
pub const MAX_RELIABILITY_OBSERVATIONS: usize = 100_000;
const MAX_CLASS_COMPONENT_BYTES: usize = 128;

/// A workload class used for stable stratification.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct WorkloadClass {
    workload: String,
    language: String,
    difficulty: String,
}

impl WorkloadClass {
    /// Construct a bounded workload, language, and difficulty identity.
    pub fn new(
        workload: impl Into<String>,
        language: impl Into<String>,
        difficulty: impl Into<String>,
    ) -> Result<Self, ReliabilityError> {
        let value = Self {
            workload: workload.into(),
            language: language.into(),
            difficulty: difficulty.into(),
        };
        if [&value.workload, &value.language, &value.difficulty]
            .iter()
            .any(|part| !valid_component(part))
        {
            return Err(ReliabilityError::InvalidClass);
        }
        Ok(value)
    }
    /// Return the stable workload identity.
    pub fn workload(&self) -> &str {
        &self.workload
    }
    /// Return the language stratum.
    pub fn language(&self) -> &str {
        &self.language
    }
    /// Return the difficulty stratum.
    pub fn difficulty(&self) -> &str {
        &self.difficulty
    }
}

fn valid_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_CLASS_COMPONENT_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

/// Terminal disposition of one planned repeated attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReliabilityOutcome {
    /// Independent grading passed.
    Passed,
    /// Independent grading failed.
    Failed,
    /// Execution reached its deadline.
    TimedOut,
    /// Execution was cancelled before grading completed.
    Cancelled,
    /// The planned attempt never started, for example because its class starved.
    NotStarted,
}

/// SLO evidence retained for one planned attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttemptSlo {
    /// Every configured SLO was met.
    Met,
    /// At least one configured SLO was violated.
    Violated,
    /// Complete SLO evidence was unavailable.
    Unavailable,
}

/// One planned attempt in a repeated trial.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct AttemptIdentity {
    epoch_id: u64,
    trial_id: u64,
    attempt_index: u32,
    seed: u64,
}
impl AttemptIdentity {
    /// Construct a caller-scoped epoch, trial, attempt, and seed identity.
    pub fn new(
        epoch_id: u64,
        trial_id: u64,
        attempt_index: u32,
        seed: u64,
    ) -> Result<Self, ReliabilityError> {
        if attempt_index >= MAX_ATTEMPTS_PER_TRIAL {
            return Err(ReliabilityError::InvalidAttempt);
        }
        Ok(Self {
            epoch_id,
            trial_id,
            attempt_index,
            seed,
        })
    }
    /// Return the caller-scoped epoch identity.
    pub fn epoch_id(self) -> u64 {
        self.epoch_id
    }
    /// Return the caller-scoped trial identity.
    pub fn trial_id(self) -> u64 {
        self.trial_id
    }
    /// Return the zero-based attempt index.
    pub fn attempt_index(self) -> u32 {
        self.attempt_index
    }
    /// Return the deterministic seed.
    pub fn seed(self) -> u64 {
        self.seed
    }
}

/// One exact attempt expected by the benchmark plan before execution begins.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct PlannedAttempt {
    class: WorkloadClass,
    identity: AttemptIdentity,
}

impl PlannedAttempt {
    /// Construct one planned class, epoch, trial, attempt-index, and seed identity.
    pub fn new(class: WorkloadClass, identity: AttemptIdentity) -> Self {
        Self { class, identity }
    }
    /// Return the planned workload class.
    pub fn class(&self) -> &WorkloadClass {
        &self.class
    }
    /// Return the planned caller-scoped identity.
    pub fn identity(&self) -> AttemptIdentity {
        self.identity
    }
}

/// One planned attempt in a repeated trial.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrialAttempt {
    class: WorkloadClass,
    identity: AttemptIdentity,
    outcome: ReliabilityOutcome,
    queue_delay_ns: Option<u64>,
    slo: AttemptSlo,
}

impl TrialAttempt {
    /// Construct an attempt while enforcing its local lifecycle invariants.
    pub fn new(
        class: WorkloadClass,
        identity: AttemptIdentity,
        outcome: ReliabilityOutcome,
        queue_delay_ns: Option<u64>,
        slo: AttemptSlo,
    ) -> Result<Self, ReliabilityError> {
        let not_started = outcome == ReliabilityOutcome::NotStarted;
        if not_started != queue_delay_ns.is_none()
            || (not_started && slo != AttemptSlo::Unavailable)
            || (slo == AttemptSlo::Met && outcome != ReliabilityOutcome::Passed)
        {
            return Err(ReliabilityError::InvalidAttempt);
        }
        Ok(Self {
            class,
            identity,
            outcome,
            queue_delay_ns,
            slo,
        })
    }
    /// Return the workload class.
    pub fn class(&self) -> &WorkloadClass {
        &self.class
    }
    /// Return the exact planned identity reported by this observation.
    pub fn identity(&self) -> AttemptIdentity {
        self.identity
    }
    /// Return the caller-scoped epoch identity.
    pub fn epoch_id(&self) -> u64 {
        self.identity.epoch_id
    }
    /// Return the caller-scoped trial identity.
    pub fn trial_id(&self) -> u64 {
        self.identity.trial_id
    }
    /// Return the zero-based attempt index.
    pub fn attempt_index(&self) -> u32 {
        self.identity.attempt_index
    }
    /// Return the deterministic seed.
    pub fn seed(&self) -> u64 {
        self.identity.seed
    }
    /// Return the terminal disposition.
    pub fn outcome(&self) -> ReliabilityOutcome {
        self.outcome
    }
    /// Return queue delay, absent exactly when the attempt never started.
    pub fn queue_delay_ns(&self) -> Option<u64> {
        self.queue_delay_ns
    }
    /// Return retained SLO evidence.
    pub fn slo(&self) -> AttemptSlo {
        self.slo
    }
}

/// Exact numerator and denominator for an empirical rate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExactRate {
    numerator: usize,
    denominator: usize,
}
impl ExactRate {
    /// Return the exact numerator.
    pub fn numerator(self) -> usize {
        self.numerator
    }
    /// Return the exact denominator.
    pub fn denominator(self) -> usize {
        self.denominator
    }
    /// Return the empirical proportion.
    pub fn proportion(self) -> f64 {
        self.numerator as f64 / self.denominator as f64
    }
}

/// Counts retaining every passed, failed, censored, and unstarted attempt.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ReliabilityCounts {
    passed: usize,
    failed: usize,
    timed_out: usize,
    cancelled: usize,
    not_started: usize,
}
impl ReliabilityCounts {
    /// Return all planned attempts.
    pub fn total(self) -> usize {
        self.passed + self.failed + self.timed_out + self.cancelled + self.not_started
    }
    /// Return passed attempts.
    pub fn passed(self) -> usize {
        self.passed
    }
    /// Return explicit grader failures.
    pub fn failed(self) -> usize {
        self.failed
    }
    /// Return timed-out attempts.
    pub fn timed_out(self) -> usize {
        self.timed_out
    }
    /// Return cancelled attempts.
    pub fn cancelled(self) -> usize {
        self.cancelled
    }
    /// Return attempts that never started.
    pub fn not_started(self) -> usize {
        self.not_started
    }
}

/// Repeated-trial reliability statistics.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrialReliability {
    trials: usize,
    attempts_per_trial: u32,
    first_attempt_pass: ExactRate,
    pass_at_k: ExactRate,
    all_k_pass: ExactRate,
}
impl TrialReliability {
    /// Return complete trial count.
    pub fn trials(self) -> usize {
        self.trials
    }
    /// Return configured attempts in every trial.
    pub fn attempts_per_trial(self) -> u32 {
        self.attempts_per_trial
    }
    /// Return trials whose first attempt passed.
    pub fn first_attempt_pass(self) -> ExactRate {
        self.first_attempt_pass
    }
    /// Return empirical pass@k: at least one pass among k attempts.
    pub fn pass_at_k(self) -> ExactRate {
        self.pass_at_k
    }
    /// Return empirical pass^k: every one of k attempts passed.
    pub fn all_k_pass(self) -> ExactRate {
        self.all_k_pass
    }
}

/// Per-class queue, starvation, and SLO evidence.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FairnessSummary {
    starvation_threshold_ns: u64,
    planned: usize,
    started: usize,
    starved: usize,
    queue_total_ns: u128,
    queue_max_ns: Option<u64>,
    slo_met: usize,
    slo_violated: usize,
    slo_unavailable: usize,
}
impl FairnessSummary {
    /// Return the queue-delay threshold used to classify started attempts as starved.
    pub fn starvation_threshold_ns(self) -> u64 {
        self.starvation_threshold_ns
    }
    /// Return planned attempts.
    pub fn planned(self) -> usize {
        self.planned
    }
    /// Return attempts crossing the execution-start boundary.
    pub fn started(self) -> usize {
        self.started
    }
    /// Return unstarted or over-threshold attempts.
    pub fn starved(self) -> usize {
        self.starved
    }
    /// Return starvation over all planned attempts.
    pub fn starvation_rate(self) -> ExactRate {
        ExactRate {
            numerator: self.starved,
            denominator: self.planned,
        }
    }
    /// Return mean queue delay over started attempts only.
    pub fn mean_queue_delay_ns(self) -> Option<f64> {
        (self.started > 0).then(|| self.queue_total_ns as f64 / self.started as f64)
    }
    /// Return maximum queue delay over started attempts only.
    pub fn maximum_queue_delay_ns(self) -> Option<u64> {
        self.queue_max_ns
    }
    /// Return attempts meeting every configured SLO.
    pub fn slo_met(self) -> usize {
        self.slo_met
    }
    /// Return attempts violating at least one SLO.
    pub fn slo_violated(self) -> usize {
        self.slo_violated
    }
    /// Return attempts with unavailable SLO evidence.
    pub fn slo_unavailable(self) -> usize {
        self.slo_unavailable
    }
}

/// One stable workload/language/difficulty stratum.
#[derive(Clone, Debug, PartialEq)]
pub struct StratumReport {
    class: WorkloadClass,
    reliability: TrialReliability,
    outcomes: ReliabilityCounts,
    fairness: FairnessSummary,
}
impl StratumReport {
    /// Return class identity.
    pub fn class(&self) -> &WorkloadClass {
        &self.class
    }
    /// Return repeated-trial reliability.
    pub fn reliability(&self) -> TrialReliability {
        self.reliability
    }
    /// Return all terminal outcome counts.
    pub fn outcomes(&self) -> ReliabilityCounts {
        self.outcomes
    }
    /// Return queue, starvation, and SLO evidence.
    pub fn fairness(&self) -> FairnessSummary {
        self.fairness
    }
}

/// Complete aggregate and stably ordered per-class report.
///
/// ```compile_fail
/// use asb_analysis::ReliabilityReport;
/// let forged = ReliabilityReport {
///     starvation_threshold_ns: 0,
///     aggregate: todo!(),
///     strata: vec![],
///     epochs: vec![],
/// };
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct ReliabilityReport {
    starvation_threshold_ns: u64,
    aggregate: StratumReport,
    strata: Vec<StratumReport>,
    epochs: Vec<EpochReport>,
}
impl ReliabilityReport {
    /// Return the queue-delay threshold used by every fairness summary.
    pub fn starvation_threshold_ns(&self) -> u64 {
        self.starvation_threshold_ns
    }
    /// Return the aggregate across all classes.
    pub fn aggregate(&self) -> &StratumReport {
        &self.aggregate
    }
    /// Return every class in stable workload/language/difficulty order.
    pub fn strata(&self) -> &[StratumReport] {
        &self.strata
    }
    /// Return every epoch in ascending identity order.
    pub fn epochs(&self) -> &[EpochReport] {
        &self.epochs
    }
}

/// Aggregate and class evidence for one execution epoch.
#[derive(Clone, Debug, PartialEq)]
pub struct EpochReport {
    epoch_id: u64,
    aggregate: StratumReport,
    strata: Vec<StratumReport>,
}
impl EpochReport {
    /// Return the caller-scoped epoch identity.
    pub fn epoch_id(&self) -> u64 {
        self.epoch_id
    }
    /// Return the aggregate across classes in this epoch.
    pub fn aggregate(&self) -> &StratumReport {
        &self.aggregate
    }
    /// Return every class observed in this epoch in stable order.
    pub fn strata(&self) -> &[StratumReport] {
        &self.strata
    }
}

/// Invalid reliability or fairness input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReliabilityError {
    /// A class component is empty, excessive, or outside the portable alphabet.
    InvalidClass,
    /// An attempt violates a local lifecycle or bound invariant.
    InvalidAttempt,
    /// Expected roster count is zero or excessive.
    InvalidPlanCount,
    /// Observation count is zero or excessive.
    InvalidObservationCount,
    /// Attempts per trial is zero or excessive.
    InvalidTrialWidth,
    /// A class/trial/index identity occurs more than once.
    DuplicateAttempt,
    /// The expected roster contains the same class and identity more than once.
    DuplicatePlannedAttempt,
    /// The observed class, epoch, trial, attempt-index, or seed roster differs from the plan.
    PlanObservationMismatch,
    /// A trial does not contain exactly indices zero through k minus one.
    IncompleteTrial,
}
impl fmt::Display for ReliabilityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidClass => "invalid reliability class",
            Self::InvalidAttempt => "invalid reliability attempt",
            Self::InvalidPlanCount => "reliability plan count is invalid",
            Self::InvalidObservationCount => "reliability observation count is invalid",
            Self::InvalidTrialWidth => "attempts per trial is invalid",
            Self::DuplicateAttempt => "duplicate reliability attempt",
            Self::DuplicatePlannedAttempt => "duplicate planned reliability attempt",
            Self::PlanObservationMismatch => "reliability observations do not match the plan",
            Self::IncompleteTrial => "reliability trial is incomplete",
        })
    }
}
impl Error for ReliabilityError {}

/// Analyze complete repeated trials and mixed-load fairness by class.
pub fn analyze_reliability(
    expected: &[PlannedAttempt],
    observations: &[TrialAttempt],
    attempts_per_trial: u32,
    starvation_threshold_ns: u64,
) -> Result<ReliabilityReport, ReliabilityError> {
    if expected.is_empty() || expected.len() > MAX_RELIABILITY_OBSERVATIONS {
        return Err(ReliabilityError::InvalidPlanCount);
    }
    if observations.is_empty() || observations.len() > MAX_RELIABILITY_OBSERVATIONS {
        return Err(ReliabilityError::InvalidObservationCount);
    }
    if attempts_per_trial == 0 || attempts_per_trial > MAX_ATTEMPTS_PER_TRIAL {
        return Err(ReliabilityError::InvalidTrialWidth);
    }
    let mut expected_set = BTreeSet::new();
    let mut expected_slots = BTreeSet::new();
    for planned in expected {
        if planned.identity.attempt_index >= attempts_per_trial {
            return Err(ReliabilityError::InvalidAttempt);
        }
        if !expected_set.insert(planned.clone())
            || !expected_slots.insert((
                planned.class.clone(),
                planned.identity.epoch_id,
                planned.identity.trial_id,
                planned.identity.attempt_index,
            ))
        {
            return Err(ReliabilityError::DuplicatePlannedAttempt);
        }
    }
    let mut observed_set = BTreeSet::new();
    for observation in observations {
        if !observed_set.insert(PlannedAttempt {
            class: observation.class.clone(),
            identity: observation.identity,
        }) {
            return Err(ReliabilityError::DuplicateAttempt);
        }
    }
    if expected_set != observed_set {
        return Err(ReliabilityError::PlanObservationMismatch);
    }
    let mut trials: BTreeMap<(WorkloadClass, u64, u64), Vec<&TrialAttempt>> = BTreeMap::new();
    for observation in observations {
        trials
            .entry((
                observation.class.clone(),
                observation.epoch_id(),
                observation.trial_id(),
            ))
            .or_default()
            .push(observation);
    }
    for attempts in trials.values_mut() {
        attempts.sort_by_key(|attempt| attempt.attempt_index());
        if attempts.len() != attempts_per_trial as usize
            || attempts
                .iter()
                .enumerate()
                .any(|(index, attempt)| attempt.attempt_index() as usize != index)
        {
            return Err(ReliabilityError::IncompleteTrial);
        }
    }
    let mut by_class: BTreeMap<WorkloadClass, Vec<&TrialAttempt>> = BTreeMap::new();
    for ((class, _, _), attempts) in &trials {
        by_class
            .entry(class.clone())
            .or_default()
            .extend(attempts.iter().copied());
    }
    let strata = by_class
        .into_iter()
        .map(|(class, attempts)| {
            summarize(
                class,
                &attempts,
                attempts_per_trial,
                starvation_threshold_ns,
            )
        })
        .collect();
    let all = trials
        .values()
        .flat_map(|attempts| attempts.iter().copied())
        .collect::<Vec<_>>();
    let aggregate = summarize(
        WorkloadClass::new("all", "all", "all").expect("static class"),
        &all,
        attempts_per_trial,
        starvation_threshold_ns,
    );
    let mut by_epoch: BTreeMap<u64, BTreeMap<WorkloadClass, Vec<&TrialAttempt>>> = BTreeMap::new();
    for ((class, epoch_id, _), attempts) in &trials {
        by_epoch
            .entry(*epoch_id)
            .or_default()
            .entry(class.clone())
            .or_default()
            .extend(attempts.iter().copied());
    }
    let epochs = by_epoch
        .into_iter()
        .map(|(epoch_id, classes)| {
            let epoch_attempts = classes
                .values()
                .flat_map(|attempts| attempts.iter().copied())
                .collect::<Vec<_>>();
            let strata = classes
                .into_iter()
                .map(|(class, attempts)| {
                    summarize(
                        class,
                        &attempts,
                        attempts_per_trial,
                        starvation_threshold_ns,
                    )
                })
                .collect();
            EpochReport {
                epoch_id,
                aggregate: summarize(
                    WorkloadClass::new("all", "all", "all").expect("static class"),
                    &epoch_attempts,
                    attempts_per_trial,
                    starvation_threshold_ns,
                ),
                strata,
            }
        })
        .collect();
    Ok(ReliabilityReport {
        starvation_threshold_ns,
        aggregate,
        strata,
        epochs,
    })
}

fn summarize(
    class: WorkloadClass,
    attempts: &[&TrialAttempt],
    k: u32,
    threshold: u64,
) -> StratumReport {
    let mut outcomes = ReliabilityCounts::default();
    let mut fairness = FairnessSummary {
        starvation_threshold_ns: threshold,
        planned: attempts.len(),
        started: 0,
        starved: 0,
        queue_total_ns: 0,
        queue_max_ns: None,
        slo_met: 0,
        slo_violated: 0,
        slo_unavailable: 0,
    };
    for attempt in attempts {
        match attempt.outcome {
            ReliabilityOutcome::Passed => outcomes.passed += 1,
            ReliabilityOutcome::Failed => outcomes.failed += 1,
            ReliabilityOutcome::TimedOut => outcomes.timed_out += 1,
            ReliabilityOutcome::Cancelled => outcomes.cancelled += 1,
            ReliabilityOutcome::NotStarted => outcomes.not_started += 1,
        }
        match attempt.queue_delay_ns {
            Some(delay) => {
                fairness.started += 1;
                fairness.queue_total_ns += u128::from(delay);
                fairness.queue_max_ns = Some(fairness.queue_max_ns.unwrap_or(0).max(delay));
                if delay > threshold {
                    fairness.starved += 1;
                }
            }
            None => fairness.starved += 1,
        }
        match attempt.slo {
            AttemptSlo::Met => fairness.slo_met += 1,
            AttemptSlo::Violated => fairness.slo_violated += 1,
            AttemptSlo::Unavailable => fairness.slo_unavailable += 1,
        }
    }
    let (mut first, mut any, mut all) = (0, 0, 0);
    for chunk in attempts.chunks_exact(k as usize) {
        first += usize::from(chunk[0].outcome == ReliabilityOutcome::Passed);
        any += usize::from(
            chunk
                .iter()
                .any(|attempt| attempt.outcome == ReliabilityOutcome::Passed),
        );
        all += usize::from(
            chunk
                .iter()
                .all(|attempt| attempt.outcome == ReliabilityOutcome::Passed),
        );
    }
    let trials = attempts.len() / k as usize;
    let rate = |numerator| ExactRate {
        numerator,
        denominator: trials,
    };
    StratumReport {
        class,
        reliability: TrialReliability {
            trials,
            attempts_per_trial: k,
            first_attempt_pass: rate(first),
            pass_at_k: rate(any),
            all_k_pass: rate(all),
        },
        outcomes,
        fairness,
    }
}
