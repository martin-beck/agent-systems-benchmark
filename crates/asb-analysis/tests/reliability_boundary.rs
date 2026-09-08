// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Public-boundary reliability and mixed-load fairness tests.

use asb_analysis::{
    AttemptIdentity, AttemptSlo, PlannedAttempt, ReliabilityError, ReliabilityOutcome,
    ReliabilityReport, TrialAttempt, WorkloadClass, analyze_reliability,
};

fn class(workload: &str, language: &str, difficulty: &str) -> WorkloadClass {
    WorkloadClass::new(workload, language, difficulty).unwrap()
}

fn attempt(
    class: &WorkloadClass,
    trial: u64,
    index: u32,
    outcome: ReliabilityOutcome,
    queue: Option<u64>,
    slo: AttemptSlo,
) -> TrialAttempt {
    TrialAttempt::new(
        class.clone(),
        AttemptIdentity::new(0, trial, index, trial.rotate_left(index) ^ u64::from(index)).unwrap(),
        outcome,
        queue,
        slo,
    )
    .unwrap()
}

fn roster(observations: &[TrialAttempt]) -> Vec<PlannedAttempt> {
    observations
        .iter()
        .map(|attempt| PlannedAttempt::new(attempt.class().clone(), attempt.identity()))
        .collect()
}

fn analyze(
    observations: &[TrialAttempt],
    attempts_per_trial: u32,
    starvation_threshold_ns: u64,
) -> Result<ReliabilityReport, ReliabilityError> {
    analyze_reliability(
        &roster(observations),
        observations,
        attempts_per_trial,
        starvation_threshold_ns,
    )
}

#[test]
fn hand_calculated_rates_retain_censoring_and_hard_strata() {
    let easy = class("bug-fix", "rust", "easy");
    let hard = class("migration", "python", "hard");
    let observations = vec![
        attempt(
            &easy,
            1,
            0,
            ReliabilityOutcome::Passed,
            Some(10),
            AttemptSlo::Met,
        ),
        attempt(
            &easy,
            1,
            1,
            ReliabilityOutcome::Failed,
            Some(20),
            AttemptSlo::Violated,
        ),
        attempt(
            &easy,
            1,
            2,
            ReliabilityOutcome::TimedOut,
            Some(30),
            AttemptSlo::Unavailable,
        ),
        attempt(
            &easy,
            2,
            0,
            ReliabilityOutcome::Failed,
            Some(40),
            AttemptSlo::Violated,
        ),
        attempt(
            &easy,
            2,
            1,
            ReliabilityOutcome::Passed,
            Some(50),
            AttemptSlo::Met,
        ),
        attempt(
            &easy,
            2,
            2,
            ReliabilityOutcome::Passed,
            Some(60),
            AttemptSlo::Met,
        ),
        attempt(
            &hard,
            3,
            0,
            ReliabilityOutcome::Passed,
            Some(100),
            AttemptSlo::Met,
        ),
        attempt(
            &hard,
            3,
            1,
            ReliabilityOutcome::Passed,
            Some(100),
            AttemptSlo::Met,
        ),
        attempt(
            &hard,
            3,
            2,
            ReliabilityOutcome::Passed,
            Some(100),
            AttemptSlo::Met,
        ),
        attempt(
            &hard,
            4,
            0,
            ReliabilityOutcome::Cancelled,
            Some(200),
            AttemptSlo::Unavailable,
        ),
        attempt(
            &hard,
            4,
            1,
            ReliabilityOutcome::NotStarted,
            None,
            AttemptSlo::Unavailable,
        ),
        attempt(
            &hard,
            4,
            2,
            ReliabilityOutcome::NotStarted,
            None,
            AttemptSlo::Unavailable,
        ),
    ];

    let report = analyze(&observations, 3, 100).unwrap();
    assert_eq!(report.starvation_threshold_ns(), 100);
    let aggregate = report.aggregate();
    assert_eq!(aggregate.fairness().starvation_threshold_ns(), 100);
    assert_eq!(aggregate.reliability().trials(), 4);
    assert_eq!(aggregate.reliability().attempts_per_trial(), 3);
    assert_eq!(aggregate.reliability().first_attempt_pass().numerator(), 2);
    assert_eq!(aggregate.reliability().pass_at_k().numerator(), 3);
    assert_eq!(aggregate.reliability().all_k_pass().numerator(), 1);
    assert_eq!(aggregate.reliability().pass_at_k().denominator(), 4);
    assert_eq!(aggregate.outcomes().passed(), 6);
    assert_eq!(aggregate.outcomes().failed(), 2);
    assert_eq!(aggregate.outcomes().timed_out(), 1);
    assert_eq!(aggregate.outcomes().cancelled(), 1);
    assert_eq!(aggregate.outcomes().not_started(), 2);
    assert_eq!(aggregate.outcomes().total(), 12);
    assert_eq!(aggregate.fairness().planned(), 12);
    assert_eq!(aggregate.fairness().started(), 10);
    assert_eq!(aggregate.fairness().starved(), 3);
    assert_eq!(aggregate.fairness().slo_met(), 6);
    assert_eq!(aggregate.fairness().slo_violated(), 2);
    assert_eq!(aggregate.fairness().slo_unavailable(), 4);
    assert_eq!(aggregate.fairness().mean_queue_delay_ns(), Some(71.0));
    assert_eq!(aggregate.fairness().maximum_queue_delay_ns(), Some(200));

    assert_eq!(report.strata().len(), 2);
    assert_eq!(report.strata()[0].class().workload(), "bug-fix");
    assert_eq!(report.strata()[0].class().language(), "rust");
    assert_eq!(report.strata()[0].class().difficulty(), "easy");
    assert_eq!(report.strata()[1].class().workload(), "migration");
    let hard_report = &report.strata()[1];
    assert!(
        report
            .strata()
            .iter()
            .all(|stratum| stratum.fairness().starvation_threshold_ns() == 100)
    );
    assert!(report.epochs().iter().all(|epoch| {
        epoch.aggregate().fairness().starvation_threshold_ns() == 100
            && epoch
                .strata()
                .iter()
                .all(|stratum| stratum.fairness().starvation_threshold_ns() == 100)
    }));
    assert_eq!(hard_report.fairness().starvation_rate().numerator(), 3);
    assert_eq!(hard_report.fairness().starvation_rate().denominator(), 6);
    assert_eq!(hard_report.reliability().pass_at_k().proportion(), 0.5);
}

#[test]
fn repeated_seed_fixture_and_input_permutation_are_deterministic() {
    fn fixture(seed: u64) -> Vec<TrialAttempt> {
        let class = class("navigation", "rust", "medium");
        let mut state = seed;
        let mut observations = Vec::new();
        for trial in 0..8 {
            for index in 0..4 {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1);
                let outcome = if state & 3 == 0 {
                    ReliabilityOutcome::Passed
                } else {
                    ReliabilityOutcome::Failed
                };
                let slo = if outcome == ReliabilityOutcome::Passed {
                    AttemptSlo::Met
                } else {
                    AttemptSlo::Violated
                };
                observations.push(attempt(
                    &class,
                    trial,
                    index,
                    outcome,
                    Some(state & 255),
                    slo,
                ));
            }
        }
        observations
    }
    let original = fixture(17);
    let repeated = fixture(17);
    let mut reversed = repeated.clone();
    reversed.reverse();
    assert_eq!(original, repeated);
    assert_eq!(
        analyze(&original, 4, 200).unwrap(),
        analyze(&reversed, 4, 200).unwrap()
    );
    assert_ne!(
        analyze(&original, 4, 200).unwrap(),
        analyze(&fixture(18), 4, 200).unwrap()
    );
}

#[test]
fn malformed_inputs_and_silent_denominator_loss_fail_closed() {
    assert_eq!(
        WorkloadClass::new("", "rust", "easy"),
        Err(ReliabilityError::InvalidClass)
    );
    assert_eq!(
        WorkloadClass::new("bug fix", "rust", "easy"),
        Err(ReliabilityError::InvalidClass)
    );
    let class = class("bug-fix", "rust", "easy");
    let identity = AttemptIdentity::new(0, 1, 0, 0).unwrap();
    assert_eq!(
        TrialAttempt::new(
            class.clone(),
            identity,
            ReliabilityOutcome::NotStarted,
            Some(1),
            AttemptSlo::Unavailable
        ),
        Err(ReliabilityError::InvalidAttempt)
    );
    assert_eq!(
        TrialAttempt::new(
            class.clone(),
            identity,
            ReliabilityOutcome::Failed,
            Some(1),
            AttemptSlo::Met
        ),
        Err(ReliabilityError::InvalidAttempt)
    );
    assert_eq!(
        TrialAttempt::new(
            class.clone(),
            identity,
            ReliabilityOutcome::NotStarted,
            None,
            AttemptSlo::Met
        ),
        Err(ReliabilityError::InvalidAttempt)
    );
    let zero = attempt(
        &class,
        1,
        0,
        ReliabilityOutcome::Failed,
        Some(0),
        AttemptSlo::Unavailable,
    );
    let zero_plan = roster(std::slice::from_ref(&zero));
    assert_eq!(
        analyze_reliability(&[], &[], 1, 0),
        Err(ReliabilityError::InvalidPlanCount)
    );
    assert_eq!(
        analyze_reliability(&zero_plan, &[], 1, 0),
        Err(ReliabilityError::InvalidObservationCount)
    );
    assert_eq!(
        analyze(std::slice::from_ref(&zero), 0, 0),
        Err(ReliabilityError::InvalidTrialWidth)
    );
    assert_eq!(
        analyze_reliability(&zero_plan, &[zero.clone(), zero.clone()], 1, 0),
        Err(ReliabilityError::DuplicateAttempt)
    );
    assert_eq!(
        analyze_reliability(
            &[zero_plan[0].clone(), zero_plan[0].clone()],
            std::slice::from_ref(&zero),
            1,
            0
        ),
        Err(ReliabilityError::DuplicatePlannedAttempt)
    );
    let duplicate_slot_different_seed = PlannedAttempt::new(
        zero_plan[0].class().clone(),
        AttemptIdentity::new(0, 1, 0, 99).unwrap(),
    );
    assert_eq!(
        analyze_reliability(
            &[zero_plan[0].clone(), duplicate_slot_different_seed],
            std::slice::from_ref(&zero),
            1,
            0
        ),
        Err(ReliabilityError::DuplicatePlannedAttempt)
    );
    assert_eq!(
        analyze(std::slice::from_ref(&zero), 2, 0),
        Err(ReliabilityError::IncompleteTrial)
    );
    let index_one = attempt(
        &class,
        1,
        1,
        ReliabilityOutcome::Failed,
        Some(0),
        AttemptSlo::Unavailable,
    );
    assert_eq!(
        analyze(&[zero, index_one], 1, 0),
        Err(ReliabilityError::InvalidAttempt)
    );
}

#[test]
fn expected_roster_rejects_a_wholly_omitted_hard_stratum_and_wrong_seed() {
    let easy = attempt(
        &class("edit", "rust", "easy"),
        1,
        0,
        ReliabilityOutcome::Passed,
        Some(1),
        AttemptSlo::Met,
    );
    let hard = attempt(
        &class("edit", "rust", "hard"),
        2,
        0,
        ReliabilityOutcome::NotStarted,
        None,
        AttemptSlo::Unavailable,
    );
    let expected = roster(&[easy.clone(), hard]);
    assert_eq!(
        analyze_reliability(&expected, std::slice::from_ref(&easy), 1, 0),
        Err(ReliabilityError::PlanObservationMismatch)
    );

    let wrong_seed = TrialAttempt::new(
        easy.class().clone(),
        AttemptIdentity::new(
            easy.epoch_id(),
            easy.trial_id(),
            easy.attempt_index(),
            easy.seed() + 1,
        )
        .unwrap(),
        easy.outcome(),
        easy.queue_delay_ns(),
        easy.slo(),
    )
    .unwrap();
    assert_eq!(
        analyze_reliability(&roster(std::slice::from_ref(&easy)), &[wrong_seed], 1, 0),
        Err(ReliabilityError::PlanObservationMismatch)
    );
}

#[test]
fn queue_threshold_is_inclusive_and_extreme_arithmetic_is_bounded() {
    let class = class("build", "c", "hard");
    let observations = vec![
        attempt(
            &class,
            1,
            0,
            ReliabilityOutcome::Failed,
            Some(u64::MAX),
            AttemptSlo::Violated,
        ),
        attempt(
            &class,
            1,
            1,
            ReliabilityOutcome::NotStarted,
            None,
            AttemptSlo::Unavailable,
        ),
    ];
    let report = analyze(&observations, 2, u64::MAX).unwrap();
    assert_eq!(report.aggregate().fairness().starved(), 1);
    assert_eq!(
        report.aggregate().fairness().mean_queue_delay_ns(),
        Some(u64::MAX as f64)
    );
    assert_eq!(report.aggregate().reliability().pass_at_k().numerator(), 0);
    assert_eq!(report.aggregate().reliability().all_k_pass().numerator(), 0);
}

#[test]
fn public_inputs_and_error_boundaries_are_explicit() {
    let class = class("build", "rust", "medium");
    let identity = AttemptIdentity::new(0, 19, 0, 23).unwrap();
    assert_eq!(identity.epoch_id(), 0);
    assert_eq!(identity.trial_id(), 19);
    assert_eq!(identity.attempt_index(), 0);
    assert_eq!(identity.seed(), 23);
    let value = TrialAttempt::new(
        class.clone(),
        identity,
        ReliabilityOutcome::Passed,
        Some(29),
        AttemptSlo::Met,
    )
    .unwrap();
    assert_eq!(value.class(), &class);
    assert_eq!(value.identity(), identity);
    assert_eq!(value.trial_id(), 19);
    assert_eq!(value.attempt_index(), 0);
    assert_eq!(value.seed(), 23);
    assert_eq!(value.outcome(), ReliabilityOutcome::Passed);
    assert_eq!(value.queue_delay_ns(), Some(29));
    assert_eq!(value.slo(), AttemptSlo::Met);
    let planned = PlannedAttempt::new(class.clone(), identity);
    assert_eq!(planned.class(), &class);
    assert_eq!(planned.identity(), identity);

    let messages = [
        (ReliabilityError::InvalidClass, "invalid reliability class"),
        (
            ReliabilityError::InvalidAttempt,
            "invalid reliability attempt",
        ),
        (
            ReliabilityError::InvalidPlanCount,
            "reliability plan count is invalid",
        ),
        (
            ReliabilityError::InvalidObservationCount,
            "reliability observation count is invalid",
        ),
        (
            ReliabilityError::InvalidTrialWidth,
            "attempts per trial is invalid",
        ),
        (
            ReliabilityError::DuplicateAttempt,
            "duplicate reliability attempt",
        ),
        (
            ReliabilityError::DuplicatePlannedAttempt,
            "duplicate planned reliability attempt",
        ),
        (
            ReliabilityError::PlanObservationMismatch,
            "reliability observations do not match the plan",
        ),
        (
            ReliabilityError::IncompleteTrial,
            "reliability trial is incomplete",
        ),
    ];
    for (error, message) in messages {
        assert_eq!(error.to_string(), message);
        let as_error: &dyn std::error::Error = &error;
        assert!(as_error.source().is_none());
    }

    assert_eq!(
        WorkloadClass::new("x".repeat(129), "rust", "easy"),
        Err(ReliabilityError::InvalidClass)
    );
    assert_eq!(
        AttemptIdentity::new(0, 1, 64, 0),
        Err(ReliabilityError::InvalidAttempt)
    );
    assert_eq!(
        analyze(std::slice::from_ref(&value), 65, 0),
        Err(ReliabilityError::InvalidTrialWidth)
    );

    let unstarted = TrialAttempt::new(
        class,
        AttemptIdentity::new(0, 1, 0, 0).unwrap(),
        ReliabilityOutcome::NotStarted,
        None,
        AttemptSlo::Unavailable,
    )
    .unwrap();
    let report = analyze(&[unstarted], 1, 0).unwrap();
    assert_eq!(report.aggregate().fairness().started(), 0);
    assert_eq!(report.aggregate().fairness().mean_queue_delay_ns(), None);
    assert_eq!(report.aggregate().fairness().maximum_queue_delay_ns(), None);
}

#[test]
fn epochs_preserve_temporal_hard_classes_and_scope_trial_identity() {
    let class = class("migration", "rust", "hard");
    let early = TrialAttempt::new(
        class.clone(),
        AttemptIdentity::new(3, 7, 0, 11).unwrap(),
        ReliabilityOutcome::Passed,
        Some(5),
        AttemptSlo::Met,
    )
    .unwrap();
    let late = TrialAttempt::new(
        class,
        AttemptIdentity::new(9, 7, 0, 13).unwrap(),
        ReliabilityOutcome::NotStarted,
        None,
        AttemptSlo::Unavailable,
    )
    .unwrap();
    let report = analyze(&[late, early], 1, 10).unwrap();
    assert_eq!(report.starvation_threshold_ns(), 10);
    assert_eq!(report.aggregate().reliability().trials(), 2);
    assert_eq!(report.epochs().len(), 2);
    assert_eq!(report.epochs()[0].epoch_id(), 3);
    assert_eq!(report.epochs()[0].aggregate().outcomes().passed(), 1);
    assert_eq!(report.epochs()[0].strata().len(), 1);
    assert_eq!(report.epochs()[1].epoch_id(), 9);
    assert_eq!(report.epochs()[1].aggregate().outcomes().not_started(), 1);
    assert_eq!(report.epochs()[1].aggregate().fairness().starved(), 1);
    assert_eq!(
        report.epochs()[1]
            .aggregate()
            .fairness()
            .starvation_threshold_ns(),
        10
    );
}

#[test]
fn every_two_attempt_outcome_pair_matches_exact_rate_definitions() {
    let class = class("matrix", "rust", "all");
    let outcomes = [
        ReliabilityOutcome::Passed,
        ReliabilityOutcome::Failed,
        ReliabilityOutcome::TimedOut,
        ReliabilityOutcome::Cancelled,
        ReliabilityOutcome::NotStarted,
    ];
    for (left_index, left) in outcomes.into_iter().enumerate() {
        for (right_index, right) in outcomes.into_iter().enumerate() {
            let make = |index, outcome| {
                let queue = (outcome != ReliabilityOutcome::NotStarted).then_some(0);
                TrialAttempt::new(
                    class.clone(),
                    AttemptIdentity::new(0, 0, index, index.into()).unwrap(),
                    outcome,
                    queue,
                    AttemptSlo::Unavailable,
                )
                .unwrap()
            };
            let report = analyze(&[make(0, left), make(1, right)], 2, 0).unwrap();
            let reliability = report.aggregate().reliability();
            assert_eq!(
                reliability.first_attempt_pass().numerator(),
                usize::from(left == ReliabilityOutcome::Passed),
                "left={left_index} right={right_index}"
            );
            assert_eq!(
                reliability.pass_at_k().numerator(),
                usize::from(
                    left == ReliabilityOutcome::Passed || right == ReliabilityOutcome::Passed
                )
            );
            assert_eq!(
                reliability.all_k_pass().numerator(),
                usize::from(
                    left == ReliabilityOutcome::Passed && right == ReliabilityOutcome::Passed
                )
            );
            assert_eq!(report.aggregate().outcomes().total(), 2);
        }
    }
}
