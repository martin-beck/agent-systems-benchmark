// SPDX-License-Identifier: MIT
//! Exhaustive and named hostile traces for crash/recovery behavior.

use asb_formal_models::recovery::{ATTEMPTS, Action, MAX_EPOCH, RecoveryState, SESSIONS};
use serde_json::Value;
use std::collections::{HashSet, VecDeque};

fn actions() -> Vec<Action> {
    let mut actions = vec![Action::ProjectNext];
    for attempt in 0..ATTEMPTS {
        actions.push(Action::Acquire { attempt });
        actions.push(Action::Crash { attempt });
        actions.push(Action::ReconcileUncertain { attempt });
        for epoch in 0..=MAX_EPOCH {
            actions.push(Action::Start { attempt, epoch });
            actions.push(Action::Complete { attempt, epoch });
        }
    }
    for session in 0..SESSIONS {
        actions.push(Action::AdvanceReplay { session });
    }
    actions
}

#[test]
fn explores_every_reachable_state_through_depth_eight() {
    let mut visited = HashSet::new();
    let mut queue = VecDeque::from([(RecoveryState::default(), 0_u8)]);
    while let Some((state, depth)) = queue.pop_front() {
        assert!(state.validate().is_ok());
        if !visited.insert(state.clone()) || depth == 8 {
            continue;
        }
        for action in actions() {
            let mut successor = state.clone();
            if successor.apply(action).is_ok() {
                queue.push_back((successor, depth + 1));
            }
        }
    }
    assert!(visited.len() >= 200, "exploration unexpectedly shallow");
}

fn parse_action(text: &str) -> Action {
    let fields = text.split(':').collect::<Vec<_>>();
    let number = |index: usize| fields[index].parse::<usize>().unwrap();
    match fields[0] {
        "acquire" => Action::Acquire { attempt: number(1) },
        "start" => Action::Start {
            attempt: number(1),
            epoch: u8::try_from(number(2)).unwrap(),
        },
        "complete" => Action::Complete {
            attempt: number(1),
            epoch: u8::try_from(number(2)).unwrap(),
        },
        "crash" => Action::Crash { attempt: number(1) },
        "reconcile" => Action::ReconcileUncertain { attempt: number(1) },
        "project" => Action::ProjectNext,
        "replay" => Action::AdvanceReplay { session: number(1) },
        _ => panic!("unknown fixture action"),
    }
}

#[test]
fn generated_named_hostile_traces_fail_closed_without_state_change() {
    let fixture: Value =
        serde_json::from_str(include_str!("../fixtures/recovery_hostile_traces.json")).unwrap();
    let traces = fixture["traces"].as_array().unwrap();
    assert_eq!(traces.len(), 6);
    for trace in traces {
        let rejected = trace["rejected_action"].as_u64().unwrap() as usize;
        let action_values = trace["actions"].as_array().unwrap();
        let mut state = RecoveryState::default();
        for (index, value) in action_values.iter().enumerate() {
            let before = state.clone();
            let result = state.apply(parse_action(value.as_str().unwrap()));
            if index == rejected {
                assert!(result.is_err(), "{} was accepted", trace["name"]);
                assert_eq!(state, before, "{} mutated on rejection", trace["name"]);
            } else {
                result.unwrap();
            }
        }
    }
}

#[test]
fn crash_reconciliation_is_terminal_without_duplicate_effect() {
    let mut state = RecoveryState::default();
    state.apply(Action::Acquire { attempt: 0 }).unwrap();
    state
        .apply(Action::Start {
            attempt: 0,
            epoch: 1,
        })
        .unwrap();
    state.apply(Action::Crash { attempt: 0 }).unwrap();
    assert!(state.apply(Action::Acquire { attempt: 0 }).is_err());
    state
        .apply(Action::ReconcileUncertain { attempt: 0 })
        .unwrap();
    assert_eq!(state.completion_count(0), Some(1));
    assert!(
        state
            .apply(Action::Start {
                attempt: 0,
                epoch: 1
            })
            .is_err()
    );
}

#[test]
fn replay_session_advance_never_changes_the_peer() {
    let mut state = RecoveryState::default();
    state.apply(Action::AdvanceReplay { session: 1 }).unwrap();
    assert_eq!(state.cursor(0), Some(0));
    assert_eq!(state.cursor(1), Some(1));
}

#[test]
fn public_bounds_and_observations_fail_closed() {
    let mut state = RecoveryState::default();
    assert_eq!(state.lease(), (None, 0));
    assert_eq!(state.journal_projection(), (0, 0));
    for action in [
        Action::Acquire { attempt: ATTEMPTS },
        Action::Start {
            attempt: ATTEMPTS,
            epoch: 0,
        },
        Action::Complete {
            attempt: ATTEMPTS,
            epoch: 0,
        },
        Action::Crash { attempt: ATTEMPTS },
        Action::ReconcileUncertain { attempt: ATTEMPTS },
        Action::AdvanceReplay { session: SESSIONS },
    ] {
        let before = state.clone();
        assert!(state.apply(action).is_err());
        assert_eq!(state, before);
    }
    assert_eq!(state.phase(ATTEMPTS), None);
    assert_eq!(state.completion_count(ATTEMPTS), None);
    assert_eq!(state.cursor(SESSIONS), None);
}
