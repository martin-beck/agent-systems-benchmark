// SPDX-License-Identifier: MIT
//! Cassette migration invariant tests.

mod support;

use asb_replay::{MigrationError, ResponseBody, verify_migration_references};

#[test]
fn identity_migration_preserves_complete_trajectory() {
    let before = support::contents();
    let after = before.clone();
    verify_migration_references(&before, &after).unwrap();
}

#[test]
fn changed_causal_reference_is_rejected() {
    let before = support::contents();
    let mut after = before.clone();
    after.interactions[1].request.previous_response_id = None;
    assert!(matches!(
        verify_migration_references(&before, &after),
        Err(MigrationError::InvariantViolation)
    ));
}

#[test]
fn changed_payload_offset_or_tool_identity_is_rejected() {
    let before = support::contents();
    let mut after = before.clone();
    if let ResponseBody::Events { events, .. } = &mut after.interactions[1].response.body {
        events[0].monotonic_offset_ns += 1;
        events[0].tool_call_id = Some("call-different".into());
    }
    assert!(matches!(
        verify_migration_references(&before, &after),
        Err(MigrationError::InvariantViolation)
    ));
}

#[test]
fn optional_buffered_and_event_identities_are_captured() {
    let before = support::contents();
    let mut changed_buffered = before.clone();
    if let ResponseBody::Buffered { response_id, .. } =
        &mut changed_buffered.interactions[0].response.body
    {
        *response_id = None;
    }
    assert!(matches!(
        verify_migration_references(&before, &changed_buffered),
        Err(MigrationError::InvariantViolation)
    ));

    let mut changed_event = before.clone();
    if let ResponseBody::Events { events, .. } = &mut changed_event.interactions[1].response.body {
        events[0].previous_response_id = None;
        events[0].tool_call_id = None;
    }
    assert!(matches!(
        verify_migration_references(&before, &changed_event),
        Err(MigrationError::InvariantViolation)
    ));
}

#[test]
fn changed_redaction_policy_descriptor_is_rejected() {
    let before = support::contents();
    let mut after = before.clone();
    after.redaction = asb_replay::RedactionPolicy {
        query_parameters: ["different-selector".to_owned()].into_iter().collect(),
        ..asb_replay::RedactionPolicy::default()
    }
    .descriptor()
    .unwrap();
    assert!(matches!(
        verify_migration_references(&before, &after),
        Err(MigrationError::InvariantViolation)
    ));
}
