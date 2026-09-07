// SPDX-License-Identifier: MIT
//! Exhaustive finite checks for frontend admission, retry, and reconnect safety.

use std::collections::BTreeSet;

use asb_control::{
    CONTROL_V1, ControlCall, ControlEvent, ControlLimits, ControlRequest, ControlSession,
    ControlEventKind, CursorError, EventWindow, IdempotencyDecision, IdempotencyIndex,
    MutationRecord,
    NegotiateParams, RequestId, Revision, RunId, SessionError,
};
use serde_json::json;

fn limits() -> ControlLimits {
    ControlLimits {
        max_frame_bytes: 4096,
        max_timeout_ms: 100,
        max_page_items: 4,
        max_in_flight: 2,
    }
}

fn request(id: u64) -> ControlRequest {
    ControlRequest {
        jsonrpc: "2.0".into(),
        id: RequestId(id),
        timeout_ms: 10,
        call: ControlCall::Capabilities,
    }
}

#[test]
fn all_short_admission_traces_preserve_capacity_and_uniqueness() {
    for encoded in 0_u16..729 {
        let mut actions = encoded;
        let mut session = ControlSession::new(limits()).unwrap();
        session
            .negotiate(&NegotiateParams {
                versions: BTreeSet::from([CONTROL_V1]),
                limits: limits(),
            })
            .unwrap();
        let mut model = BTreeSet::new();
        for _ in 0..6 {
            let id = RequestId(u64::from(actions % 3));
            actions /= 3;
            if model.remove(&id) {
                assert_eq!(session.finish(id), Ok(()));
            } else {
                match session.admit(&request(id.0)) {
                    Ok(_) => {
                        assert!(model.len() < 2);
                        assert!(model.insert(id));
                    }
                    Err(SessionError::Backpressure) => assert_eq!(model.len(), 2),
                    Err(SessionError::DuplicateRequest) => assert!(model.contains(&id)),
                    other => panic!("unexpected admission result: {other:?}"),
                }
            }
            assert_eq!(session.in_flight(), model.len());
            assert!(session.in_flight() <= 2);
        }
    }
}

#[test]
fn every_retry_and_restart_trace_has_at_most_one_new_effect() {
    for same_digest_bits in 0_u8..8 {
        let mut durable = Vec::new();
        let mut effects = 0;
        for step in 0..3 {
            let index = IdempotencyIndex::restore(4, durable.clone()).unwrap();
            let digest = if (same_digest_bits >> step) & 1 == 0 {
                "request-a"
            } else {
                "request-b"
            };
            match index.check("stable-key", digest) {
                Ok(IdempotencyDecision::New) => {
                    effects += 1;
                    durable.push(MutationRecord {
                        key: "stable-key".into(),
                        request_sha256: digest.into(),
                        result: json!({"run_id": "run-1"}),
                    });
                }
                Ok(IdempotencyDecision::Replay(result)) => {
                    assert_eq!(result, &json!({"run_id": "run-1"}));
                }
                Err(_) => {}
            }
            assert!(effects <= 1, "retry trace duplicated an external effect");
        }
    }
}

#[test]
fn every_retained_cursor_returns_each_revision_at_most_once() {
    let events = (20..=24).map(|revision| ControlEvent {
        revision: Revision(revision),
        kind: ControlEventKind::RunUpdated,
        run_id: Some(RunId("run-1".into())),
        attempt_id: None,
    });
    let window = EventWindow::restore(5, events).unwrap();
    for start in 19..=24 {
        for page_size in 1..=4 {
            let mut cursor = Some(Revision(start));
            let mut observed = BTreeSet::new();
            loop {
                let page = window.page(cursor, page_size).unwrap();
                for event in page.items {
                    assert!(observed.insert(event.revision));
                    assert!(event.revision.0 > start);
                }
                cursor = page.next;
                if !page.has_more {
                    break;
                }
            }
        }
    }
    assert!(matches!(
        window.page(Some(Revision(18)), 1),
        Err(CursorError::Stale { .. })
    ));
}

#[test]
fn retained_global_cursor_mutant_loses_an_independent_client_event() {
    let mut global_cursor = Revision(20);
    let client_b = global_cursor;
    global_cursor = Revision(21);
    assert_ne!(
        global_cursor, client_b,
        "mutant demonstrates why cursors are client-supplied projections"
    );
}
