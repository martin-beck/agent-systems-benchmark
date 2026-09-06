// SPDX-License-Identifier: MIT
//! Loom exploration of cancellation and terminal ownership.

use loom::sync::Arc;
use loom::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use loom::thread;

const RUNNING: u8 = 0;
const CANCEL_OWNED: u8 = 1;
const TERMINAL_CANCELLED: u8 = 2;
const TERMINAL_COMPLETED: u8 = 3;

fn cancel(state: &AtomicU8) {
    let _ = state.compare_exchange(RUNNING, CANCEL_OWNED, Ordering::AcqRel, Ordering::Acquire);
}

fn finish(state: &AtomicU8, terminalizations: &AtomicUsize) {
    loop {
        let observed = state.load(Ordering::Acquire);
        let terminal = match observed {
            RUNNING => TERMINAL_COMPLETED,
            CANCEL_OWNED => TERMINAL_CANCELLED,
            TERMINAL_CANCELLED | TERMINAL_COMPLETED => return,
            _ => unreachable!(),
        };
        if state
            .compare_exchange(observed, terminal, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            terminalizations.fetch_add(1, Ordering::AcqRel);
            return;
        }
        thread::yield_now();
    }
}

#[test]
fn cancellation_and_completion_have_one_terminal_owner() {
    loom::model(|| {
        let state = Arc::new(AtomicU8::new(RUNNING));
        let terminalizations = Arc::new(AtomicUsize::new(0));
        let cancel_state = Arc::clone(&state);
        let cancel_thread = thread::spawn(move || cancel(&cancel_state));
        let finish_state = Arc::clone(&state);
        let finish_count = Arc::clone(&terminalizations);
        let finish_thread = thread::spawn(move || finish(&finish_state, &finish_count));
        cancel(&state);
        finish(&state, &terminalizations);
        cancel_thread.join().unwrap();
        finish_thread.join().unwrap();
        assert!(matches!(
            state.load(Ordering::Acquire),
            TERMINAL_CANCELLED | TERMINAL_COMPLETED
        ));
        assert_eq!(terminalizations.load(Ordering::Acquire), 1);
    });
}

#[test]
fn split_state_and_owner_mutant_has_a_loom_counterexample() {
    let counterexample = std::panic::catch_unwind(|| {
        loom::model(|| {
            let state = Arc::new(AtomicU8::new(RUNNING));
            let owner = Arc::new(AtomicU8::new(0));
            let thread_state = Arc::clone(&state);
            let thread_owner = Arc::clone(&owner);
            let worker = thread::spawn(move || {
                if thread_state.load(Ordering::Acquire) == RUNNING {
                    thread::yield_now();
                    thread_owner.store(1, Ordering::Release);
                }
            });
            if state.load(Ordering::Acquire) == RUNNING {
                state.store(TERMINAL_COMPLETED, Ordering::Release);
                owner.store(2, Ordering::Release);
            }
            worker.join().unwrap();
            assert_eq!(owner.load(Ordering::Acquire), 2);
        });
    });
    assert!(
        counterexample.is_err(),
        "Loom must reject the retained split-ownership mutant"
    );
}
