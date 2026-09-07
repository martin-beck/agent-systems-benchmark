// SPDX-License-Identifier: MIT
//! Production storage trace conformance for the recovery model.

use asb_formal_models::recovery::{Action, Phase, RecoveryState};
use asb_protocol::Id;
use asb_store::{
    AtomicStore, ExecutionState, JOURNAL_SCHEMA_VERSION, JournalEvent, MANIFEST_SCHEMA_VERSION,
    RecoveryDecision, RunManifest, StoreError, StoreLimits,
};
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let base = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .unwrap_or_else(std::env::temp_dir);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = base.join(format!(
            "asb-recovery-production-{}-{nonce}",
            std::process::id()
        ));
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn event(sequence: u64, attempt_id: &str, state: ExecutionState) -> JournalEvent {
    JournalEvent {
        schema_version: JOURNAL_SCHEMA_VERSION,
        sequence,
        attempt_id: Id(attempt_id.to_owned()),
        monotonic_offset_ns: sequence,
        state,
        evidence: json!({"classification": "public-synthetic"}),
    }
}

fn append_started(store: &AtomicStore, run_id: &str, attempt_id: &str) {
    for (sequence, state) in [
        ExecutionState::Planned,
        ExecutionState::Prepared,
        ExecutionState::Running,
    ]
    .into_iter()
    .enumerate()
    {
        store
            .append(
                run_id,
                &event(u64::try_from(sequence).unwrap(), attempt_id, state),
            )
            .unwrap();
    }
}

#[test]
fn durable_running_state_matches_uncertain_model_and_fences_stale_attempt() {
    let scratch = Scratch::new();
    let store = AtomicStore::open(&scratch.0, StoreLimits::default()).unwrap();
    store
        .create_run(&RunManifest {
            schema_version: MANIFEST_SCHEMA_VERSION,
            run_id: Id("formal-run".to_owned()),
            attempt_id: Id("attempt-current".to_owned()),
            definition: json!({"fixture": "recovery-production"}),
        })
        .unwrap();
    assert_eq!(
        store.recovery_decision("formal-run").unwrap(),
        RecoveryDecision::StartAllowed
    );

    let mut model = RecoveryState::default();
    model.apply(Action::Acquire { attempt: 0 }).unwrap();
    model
        .apply(Action::Start {
            attempt: 0,
            epoch: 1,
        })
        .unwrap();
    append_started(&store, "formal-run", "attempt-current");
    assert_eq!(model.phase(0), Some(Phase::Running));
    assert_eq!(
        store.recovery_decision("formal-run").unwrap(),
        RecoveryDecision::NeedsReconciliation
    );

    let before = store.load_journal("formal-run").unwrap();
    let stale = store.append(
        "formal-run",
        &event(3, "attempt-stale", ExecutionState::Completed),
    );
    assert!(matches!(stale, Err(StoreError::StaleAttempt)));
    assert_eq!(store.load_journal("formal-run").unwrap(), before);

    model.apply(Action::Crash { attempt: 0 }).unwrap();
    assert_eq!(model.phase(0), Some(Phase::Uncertain));
    assert!(
        model
            .apply(Action::Start {
                attempt: 0,
                epoch: 1
            })
            .is_err()
    );
    assert_eq!(
        store.recovery_decision("formal-run").unwrap(),
        RecoveryDecision::NeedsReconciliation,
        "production refuses a duplicate effect until external reconciliation"
    );
}

#[test]
fn durable_terminal_state_and_model_both_reject_duplicate_completion() {
    let scratch = Scratch::new();
    let store = AtomicStore::open(&scratch.0, StoreLimits::default()).unwrap();
    store
        .create_run(&RunManifest {
            schema_version: MANIFEST_SCHEMA_VERSION,
            run_id: Id("formal-terminal".to_owned()),
            attempt_id: Id("attempt-terminal".to_owned()),
            definition: json!({"fixture": "terminal-production"}),
        })
        .unwrap();
    append_started(&store, "formal-terminal", "attempt-terminal");
    store
        .append(
            "formal-terminal",
            &event(3, "attempt-terminal", ExecutionState::Collecting),
        )
        .unwrap();
    store
        .append(
            "formal-terminal",
            &event(4, "attempt-terminal", ExecutionState::Completed),
        )
        .unwrap();
    assert_eq!(
        store.recovery_decision("formal-terminal").unwrap(),
        RecoveryDecision::Terminal(ExecutionState::Completed)
    );
    assert!(
        store
            .append(
                "formal-terminal",
                &event(5, "attempt-terminal", ExecutionState::Completed)
            )
            .is_err()
    );

    let mut model = RecoveryState::default();
    model.apply(Action::Acquire { attempt: 0 }).unwrap();
    model
        .apply(Action::Start {
            attempt: 0,
            epoch: 1,
        })
        .unwrap();
    model
        .apply(Action::Complete {
            attempt: 0,
            epoch: 1,
        })
        .unwrap();
    let before = model.clone();
    assert!(
        model
            .apply(Action::Complete {
                attempt: 0,
                epoch: 1
            })
            .is_err()
    );
    assert_eq!(model, before);
}
