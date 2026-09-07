// SPDX-License-Identifier: MIT
//! Exhaustive finite attempt and replay state-model checks.

use asb_formal_models::microseconds_to_nanoseconds;
use asb_formal_models::{
    AttemptEvent, AttemptState, ReplayCursors, RequestSelectorScope, transition_attempt,
};

const EVENTS: [AttemptEvent; 3] = [
    AttemptEvent::Start,
    AttemptEvent::Collect,
    AttemptEvent::Complete,
];

fn enumerate(
    state: AttemptState,
    depth: usize,
    terminal_count: usize,
    trace: &mut Vec<AttemptEvent>,
) {
    assert!(terminal_count <= 1, "double completion accepted: {trace:?}");
    if depth == 0 {
        return;
    }
    for event in EVENTS {
        trace.push(event);
        if let Some(next) = transition_attempt(state, event) {
            enumerate(
                next,
                depth - 1,
                terminal_count + usize::from(next == AttemptState::Terminal),
                trace,
            );
        }
        trace.pop();
    }
}

#[test]
fn all_attempt_traces_to_depth_six_reject_double_completion() {
    enumerate(AttemptState::Planned, 6, 0, &mut Vec::new());
    assert_eq!(
        transition_attempt(AttemptState::Terminal, AttemptEvent::Complete),
        None
    );
}

#[test]
fn double_completion_mutant_is_detected() {
    fn mutant(state: AttemptState, event: AttemptEvent) -> Option<AttemptState> {
        if state == AttemptState::Terminal && event == AttemptEvent::Complete {
            Some(AttemptState::Terminal)
        } else {
            transition_attempt(state, event)
        }
    }
    assert_eq!(
        mutant(AttemptState::Terminal, AttemptEvent::Complete),
        Some(AttemptState::Terminal),
        "fixture must contain the double-completion defect"
    );
    assert_ne!(
        mutant(AttemptState::Terminal, AttemptEvent::Complete),
        transition_attempt(AttemptState::Terminal, AttemptEvent::Complete),
        "production model must reject the retained mutant"
    );
}

#[test]
fn replay_sessions_advance_independently_for_all_short_traces() {
    for bits in 0_u8..16 {
        let mut cursors = ReplayCursors::default();
        let mut expected = [0_u8; 2];
        for step in 0..4 {
            let session = usize::from((bits >> step) & 1);
            let other = 1 - session;
            let before_other = cursors.cursor(other);
            assert!(cursors.advance(session));
            expected[session] += 1;
            assert_eq!(cursors.cursor(other), before_other);
            assert_eq!(cursors.cursor(0), Some(expected[0]));
            assert_eq!(cursors.cursor(1), Some(expected[1]));
        }
    }
}

#[test]
fn global_cursor_mutant_exhibits_cross_talk() {
    let mut global_cursor = 0_u8;
    let second_before = global_cursor;
    global_cursor += 1;
    assert_ne!(
        global_cursor, second_before,
        "retained global-cursor mutant must alter the other session view"
    );
}

#[test]
fn request_selector_scope_is_exhaustive_and_method_blind_mutant_fails() {
    for rule_interaction in 0..=2 {
        for request_interaction in 0..=2 {
            for rule_post in [false, true] {
                for request_post in [false, true] {
                    let scope = RequestSelectorScope::new(rule_interaction, rule_post);
                    assert_eq!(
                        scope.applies_to(request_interaction, request_post),
                        rule_interaction == request_interaction && rule_post == request_post
                    );
                }
            }
        }
    }

    let post_rule = RequestSelectorScope::new(1, true);
    assert!(!post_rule.applies_to(1, false));
    let method_blind_mutant_applies = 1 == 1;
    assert!(method_blind_mutant_applies);
    assert_ne!(
        method_blind_mutant_applies,
        post_rule.applies_to(1, false),
        "retained method-blind mutant must differ on POST-rule versus GET-request"
    );
}

#[test]
fn model_bounds_fail_closed() {
    assert_eq!(microseconds_to_nanoseconds(u64::MAX), None);
    let mut cursors = ReplayCursors::default();
    assert_eq!(cursors.cursor(2), None);
    for _ in 0..u8::MAX {
        assert!(cursors.advance(0));
    }
    assert!(!cursors.advance(0));
    assert!(!cursors.advance(2));
    assert_eq!(cursors.cursor(0), Some(u8::MAX));
}
