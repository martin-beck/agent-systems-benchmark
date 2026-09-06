// SPDX-License-Identifier: MIT
//! Native scheduler boundary tests.

use asb_runtime::scheduler::{
    AttemptOutcome, CapacityDecision, CapacityPoint, LoadModel, MAX_IN_FLIGHT, MAX_POINT_DURATION,
    MAX_QUEUE, MissReason, PlanError, PointPlan, Scheduler, StopReason, SystemClock,
    capacity_order, highest_confirmed_capacity,
};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Clone)]
struct RegressingClock(Arc<AtomicU32>);

impl asb_runtime::scheduler::MonotonicClock for RegressingClock {
    fn now(&self) -> Duration {
        if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
            Duration::from_millis(1)
        } else {
            Duration::ZERO
        }
    }
}

#[derive(Clone)]
struct FrozenClock;

impl asb_runtime::scheduler::MonotonicClock for FrozenClock {
    fn now(&self) -> Duration {
        Duration::ZERO
    }
}

#[derive(Clone)]
struct JumpingClock(Arc<AtomicU32>);

impl asb_runtime::scheduler::MonotonicClock for JumpingClock {
    fn now(&self) -> Duration {
        if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
            Duration::ZERO
        } else {
            Duration::MAX
        }
    }
}

#[derive(Clone)]
struct WorkerClockPanic {
    base: SystemClock,
    scheduler_thread: thread::ThreadId,
    panic_in_worker: Arc<AtomicBool>,
}

impl asb_runtime::scheduler::MonotonicClock for WorkerClockPanic {
    fn now(&self) -> Duration {
        if thread::current().id() != self.scheduler_thread
            && self.panic_in_worker.load(Ordering::SeqCst)
        {
            panic!("hostile external clock payload");
        }
        <SystemClock as asb_runtime::scheduler::MonotonicClock>::now(&self.base)
    }
}

fn plan(model: LoadModel, measured: u32, in_flight: u32, queue: u32) -> PointPlan {
    PointPlan::new(
        model,
        measured,
        1,
        in_flight,
        queue,
        0,
        Duration::from_secs(2),
        Duration::from_millis(1),
        17,
    )
    .unwrap()
}

#[test]
fn invalid_bounds_fail_closed() {
    let make = |model, measured, warmups, active, queue, duration, poll| {
        PointPlan::new(
            model, measured, warmups, active, queue, 0, duration, poll, 0,
        )
    };
    assert_eq!(
        make(
            LoadModel::ClosedLoop,
            0,
            0,
            1,
            0,
            Duration::from_secs(1),
            Duration::from_millis(1)
        ),
        Err(PlanError::NoMeasuredAttempts)
    );
    assert_eq!(
        make(
            LoadModel::ClosedLoop,
            1_000_000,
            1,
            1,
            0,
            Duration::from_secs(1),
            Duration::from_millis(1)
        ),
        Err(PlanError::TooManyAttempts)
    );
    assert_eq!(
        make(
            LoadModel::ClosedLoop,
            1,
            0,
            0,
            0,
            Duration::from_secs(1),
            Duration::from_millis(1)
        ),
        Err(PlanError::InvalidInFlight)
    );
    assert_eq!(
        make(
            LoadModel::ClosedLoop,
            1,
            0,
            1,
            1,
            Duration::from_secs(1),
            Duration::from_millis(1)
        ),
        Err(PlanError::InvalidQueue)
    );
    assert_eq!(
        make(
            LoadModel::OpenLoop {
                inter_arrival: Duration::ZERO
            },
            1,
            0,
            1,
            0,
            Duration::from_secs(1),
            Duration::from_millis(1)
        ),
        Err(PlanError::InvalidInterArrival)
    );
    assert_eq!(
        make(
            LoadModel::ClosedLoop,
            1,
            0,
            1,
            0,
            Duration::ZERO,
            Duration::from_millis(1)
        ),
        Err(PlanError::InvalidDuration)
    );
    assert_eq!(
        make(
            LoadModel::ClosedLoop,
            1,
            0,
            MAX_IN_FLIGHT + 1,
            0,
            Duration::from_secs(1),
            Duration::from_millis(1)
        ),
        Err(PlanError::InvalidInFlight)
    );
    assert_eq!(
        make(
            LoadModel::OpenLoop {
                inter_arrival: Duration::from_millis(1)
            },
            1,
            0,
            1,
            MAX_QUEUE + 1,
            Duration::from_secs(1),
            Duration::from_millis(1)
        ),
        Err(PlanError::InvalidQueue)
    );
    assert_eq!(
        make(
            LoadModel::ClosedLoop,
            1,
            0,
            1,
            0,
            MAX_POINT_DURATION + Duration::from_nanos(1),
            Duration::from_millis(1)
        ),
        Err(PlanError::InvalidDuration)
    );
    assert_eq!(
        make(
            LoadModel::ClosedLoop,
            1,
            0,
            1,
            0,
            Duration::from_secs(1),
            Duration::ZERO
        ),
        Err(PlanError::InvalidDuration)
    );
    assert_eq!(PlanError::InvalidQueue.to_string(), "InvalidQueue");
    assert_eq!(
        make(
            LoadModel::ClosedLoop,
            u32::MAX,
            u32::MAX,
            1,
            0,
            Duration::from_secs(1),
            Duration::from_millis(1)
        ),
        Err(PlanError::TooManyAttempts)
    );
    assert_eq!(
        make(
            LoadModel::ClosedLoop,
            1,
            0,
            1,
            0,
            Duration::from_millis(1),
            Duration::from_millis(2)
        ),
        Err(PlanError::InvalidDuration)
    );
    assert_eq!(
        PointPlan::new(
            LoadModel::OpenLoop {
                inter_arrival: Duration::MAX,
            },
            3,
            0,
            1,
            0,
            0,
            Duration::from_secs(1),
            Duration::from_millis(1),
            0,
        ),
        Err(PlanError::ScheduleOverflow)
    );
}

#[test]
fn real_closed_loop_respects_concurrency_and_warmups() {
    let active = Arc::new(AtomicU32::new(0));
    let peak = Arc::new(AtomicU32::new(0));
    let a = Arc::clone(&active);
    let p = Arc::clone(&peak);
    let result = Scheduler::new(SystemClock::start()).run(
        plan(LoadModel::ClosedLoop, 12, 3, 0),
        move |_| {
            let current = a.fetch_add(1, Ordering::SeqCst) + 1;
            p.fetch_max(current, Ordering::SeqCst);
            thread::sleep(Duration::from_millis(4));
            a.fetch_sub(1, Ordering::SeqCst);
            AttemptOutcome::Completed
        },
    );
    assert_eq!(result.stop_reason(), StopReason::Completed);
    assert_eq!(result.measured().count(), 12);
    assert_eq!(result.attempts().len(), 13);
    assert_eq!(peak.load(Ordering::SeqCst), 3);
    assert_eq!(active.load(Ordering::SeqCst), 0);
}

#[test]
fn seeded_warmup_and_measured_permutations_are_directly_reproducible() {
    let run = |seed| {
        let point = PointPlan::new(
            LoadModel::ClosedLoop,
            16,
            16,
            16,
            0,
            0,
            Duration::from_secs(1),
            Duration::from_millis(1),
            seed,
        )
        .unwrap();
        let result = Scheduler::new(SystemClock::start()).run(point, |_| AttemptOutcome::Completed);
        let warmup = result
            .attempts()
            .iter()
            .filter(|attempt| attempt.is_warmup())
            .map(|attempt| attempt.input_id())
            .collect::<Vec<_>>();
        let measured = result
            .attempts()
            .iter()
            .filter(|attempt| !attempt.is_warmup())
            .map(|attempt| attempt.input_id())
            .collect::<Vec<_>>();
        (warmup, measured)
    };

    let first = run(17);
    assert_eq!(first, run(17));
    assert_ne!(first, run(18));
    assert_ne!(first.0, first.1);
    for phase in [&first.0, &first.1] {
        let mut sorted = phase.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..16).collect::<Vec<_>>());
    }
}

#[test]
fn real_open_loop_counts_queue_delay_and_missed_arrivals() {
    let model = LoadModel::OpenLoop {
        inter_arrival: Duration::from_millis(1),
    };
    let result = Scheduler::new(SystemClock::start()).run(plan(model, 12, 1, 2), |_| {
        thread::sleep(Duration::from_millis(8));
        AttemptOutcome::Completed
    });
    assert_eq!(result.stop_reason(), StopReason::Completed);
    assert!(result.missed_arrivals() > 0);
    assert!(
        result
            .measured()
            .any(|attempt| !attempt.missed() && attempt.queue_delay() > Duration::ZERO)
    );
}

#[test]
fn failure_limits_deadlines_and_panics_stop_admission() {
    let failed_plan = PointPlan::new(
        LoadModel::ClosedLoop,
        20,
        0,
        2,
        0,
        1,
        Duration::from_secs(1),
        Duration::from_millis(1),
        1,
    )
    .unwrap();
    let failed = Scheduler::new(SystemClock::start()).run(failed_plan, |_| AttemptOutcome::Failed);
    assert_eq!(failed.stop_reason(), StopReason::FailureLimit);
    assert_eq!(failed.attempts().len(), 20);
    assert!(failed.attempts().iter().any(|attempt| {
        attempt.miss_reason() == Some(MissReason::PointStopped(StopReason::FailureLimit))
    }));

    let panic_plan = PointPlan::new(
        LoadModel::ClosedLoop,
        20,
        0,
        1,
        0,
        0,
        Duration::from_secs(1),
        Duration::from_millis(1),
        1,
    )
    .unwrap();
    let contaminated = Scheduler::new(SystemClock::start())
        .run(panic_plan, |_| panic!("synthetic infrastructure failure"));
    assert_eq!(contaminated.stop_reason(), StopReason::Contaminated);
    assert!(contaminated.contaminated());
    assert_eq!(contaminated.attempts().len(), 20);
    assert!(contaminated.attempts().iter().any(|attempt| {
        attempt.miss_reason() == Some(MissReason::PointStopped(StopReason::Contaminated))
    }));

    let deadline_plan = PointPlan::new(
        LoadModel::ClosedLoop,
        20,
        0,
        1,
        0,
        0,
        Duration::from_millis(2),
        Duration::from_millis(1),
        1,
    )
    .unwrap();
    let deadline = Scheduler::new(SystemClock::start()).run(deadline_plan, |_| {
        thread::sleep(Duration::from_millis(5));
        AttemptOutcome::Completed
    });
    assert_eq!(deadline.stop_reason(), StopReason::DurationLimit);
    assert_eq!(deadline.attempts().len(), 20);
    assert!(deadline.attempts().iter().any(|attempt| {
        attempt.miss_reason() == Some(MissReason::PointStopped(StopReason::DurationLimit))
    }));

    for outcome in [AttemptOutcome::TimedOut, AttemptOutcome::Cancelled] {
        let plan = PointPlan::new(
            LoadModel::ClosedLoop,
            3,
            0,
            1,
            0,
            1,
            Duration::from_secs(1),
            Duration::from_millis(1),
            1,
        )
        .unwrap();
        let result = Scheduler::new(SystemClock::start()).run(plan, move |_| outcome);
        assert_eq!(result.stop_reason(), StopReason::FailureLimit);
        assert_eq!(result.attempts().len(), 3);
        assert!(result.attempts().iter().any(|attempt| {
            attempt.miss_reason() == Some(MissReason::PointStopped(StopReason::FailureLimit))
        }));
    }

    let explicit = PointPlan::new(
        LoadModel::ClosedLoop,
        4,
        0,
        1,
        0,
        0,
        Duration::from_secs(1),
        Duration::from_millis(1),
        2,
    )
    .unwrap();
    let contaminated = Scheduler::new(SystemClock::start())
        .run(explicit, |_| AttemptOutcome::InfrastructureFailure);
    assert_eq!(contaminated.stop_reason(), StopReason::Contaminated);
    assert_eq!(contaminated.attempts().len(), 4);
}

#[test]
fn backlog_and_terminal_stops_preserve_complete_causal_accounting() {
    let point = PointPlan::new(
        LoadModel::OpenLoop {
            inter_arrival: Duration::from_millis(1),
        },
        16,
        0,
        1,
        3,
        1,
        Duration::from_secs(1),
        Duration::from_millis(1),
        41,
    )
    .unwrap();
    let result = Scheduler::new(SystemClock::start()).run(point, |_| {
        thread::sleep(Duration::from_millis(20));
        AttemptOutcome::Failed
    });
    assert_eq!(result.stop_reason(), StopReason::FailureLimit);
    assert_eq!(result.attempts().len(), 16);
    let mut ids = result
        .attempts()
        .iter()
        .map(|attempt| attempt.input_id())
        .collect::<Vec<_>>();
    ids.sort_unstable();
    assert_eq!(ids, (0..16).collect::<Vec<_>>());
    assert!(
        result
            .attempts()
            .iter()
            .any(|attempt| { attempt.miss_reason() == Some(MissReason::Backpressure) })
    );
    assert!(result.attempts().iter().any(|attempt| {
        attempt.miss_reason() == Some(MissReason::PointStopped(StopReason::FailureLimit))
    }));
    assert!(
        result
            .attempts()
            .iter()
            .all(|attempt| { attempt.missed() == attempt.miss_reason().is_some() })
    );
}

#[test]
fn warmup_stop_still_accounts_for_every_measured_input() {
    let point = PointPlan::new(
        LoadModel::ClosedLoop,
        7,
        1,
        1,
        0,
        1,
        Duration::from_secs(1),
        Duration::from_millis(1),
        9,
    )
    .unwrap();
    let result = Scheduler::new(SystemClock::start()).run(point, |_| AttemptOutcome::Failed);
    assert_eq!(result.stop_reason(), StopReason::FailureLimit);
    assert_eq!(result.attempts().len(), 8);
    assert_eq!(result.measured().count(), 7);
    assert!(result.measured().all(|attempt| {
        attempt.miss_reason() == Some(MissReason::PointStopped(StopReason::FailureLimit))
    }));
}

#[test]
fn late_terminal_evidence_never_rewrites_the_causal_admission_stop() {
    let failure_first = PointPlan::new(
        LoadModel::ClosedLoop,
        6,
        0,
        2,
        0,
        1,
        Duration::from_secs(1),
        Duration::from_millis(1),
        1,
    )
    .unwrap();
    let completion_order = Arc::new(AtomicU32::new(0));
    let worker_order = Arc::clone(&completion_order);
    let result = Scheduler::new(SystemClock::start()).run(failure_first, move |_| {
        if worker_order.fetch_add(1, Ordering::SeqCst) == 0 {
            thread::sleep(Duration::from_millis(1));
            AttemptOutcome::Failed
        } else {
            thread::sleep(Duration::from_millis(20));
            AttemptOutcome::InfrastructureFailure
        }
    });
    assert_eq!(result.admission_stop_reason(), StopReason::FailureLimit);
    assert_eq!(result.stop_reason(), StopReason::Contaminated);
    assert!(
        result
            .attempts()
            .iter()
            .filter(|attempt| attempt.missed())
            .all(|attempt| {
                attempt.miss_reason() == Some(MissReason::PointStopped(StopReason::FailureLimit))
            })
    );

    let duration_first = PointPlan::new(
        LoadModel::ClosedLoop,
        4,
        0,
        1,
        0,
        1,
        Duration::from_millis(2),
        Duration::from_millis(1),
        2,
    )
    .unwrap();
    let result = Scheduler::new(SystemClock::start()).run(duration_first, |_| {
        thread::sleep(Duration::from_millis(10));
        AttemptOutcome::Failed
    });
    assert_eq!(result.admission_stop_reason(), StopReason::DurationLimit);
    assert_eq!(result.stop_reason(), StopReason::FailureLimit);
    assert!(
        result
            .attempts()
            .iter()
            .filter(|attempt| attempt.missed())
            .all(|attempt| {
                attempt.miss_reason() == Some(MissReason::PointStopped(StopReason::DurationLimit))
            })
    );
}

#[test]
fn hostile_external_clocks_fail_boundedly_without_fabricated_timestamps() {
    let point = || {
        PointPlan::new(
            LoadModel::ClosedLoop,
            4,
            0,
            1,
            0,
            0,
            Duration::from_millis(100),
            Duration::from_millis(1),
            3,
        )
        .unwrap()
    };

    let regressed = Scheduler::new(RegressingClock(Arc::new(AtomicU32::new(0))))
        .run(point(), |_| AttemptOutcome::Completed);
    assert_eq!(regressed.stop_reason(), StopReason::Contaminated);
    assert_eq!(regressed.attempts().len(), 4);
    assert!(
        regressed
            .attempts()
            .iter()
            .all(|attempt| { attempt.started_at().is_none() && attempt.finished_at().is_none() })
    );

    let jumped = Scheduler::new(JumpingClock(Arc::new(AtomicU32::new(0))))
        .run(point(), |_| AttemptOutcome::Completed);
    assert_eq!(jumped.stop_reason(), StopReason::Contaminated);
    assert_eq!(jumped.attempts().len(), 4);

    let started = Instant::now();
    let frozen = Scheduler::new(FrozenClock).run(point(), |_| {
        thread::sleep(Duration::from_millis(30));
        AttemptOutcome::Completed
    });
    assert!(started.elapsed() < Duration::from_millis(500));
    assert_eq!(frozen.stop_reason(), StopReason::Contaminated);
    assert_eq!(frozen.attempts().len(), 4);

    let started = Instant::now();
    let frozen_fast = Scheduler::new(FrozenClock).run(point(), |_| AttemptOutcome::Completed);
    assert!(started.elapsed() < Duration::from_millis(500));
    assert_eq!(frozen_fast.stop_reason(), StopReason::Contaminated);
}

#[test]
fn worker_clock_panics_at_start_and_finish_are_drained_as_contamination() {
    for panic_at_start in [true, false] {
        let panic_in_worker = Arc::new(AtomicBool::new(panic_at_start));
        let clock = WorkerClockPanic {
            base: SystemClock::start(),
            scheduler_thread: thread::current().id(),
            panic_in_worker: Arc::clone(&panic_in_worker),
        };
        let executor_flag = Arc::clone(&panic_in_worker);
        let result = Scheduler::new(clock).run(
            PointPlan::new(
                LoadModel::ClosedLoop,
                2,
                0,
                1,
                0,
                0,
                Duration::from_secs(1),
                Duration::from_millis(1),
                4,
            )
            .unwrap(),
            move |_| {
                executor_flag.store(true, Ordering::SeqCst);
                AttemptOutcome::Completed
            },
        );
        assert_eq!(result.stop_reason(), StopReason::Contaminated);
        assert_eq!(result.attempts().len(), 2);
        let fault = &result.attempts()[0];
        assert_eq!(fault.outcome(), Some(AttemptOutcome::InfrastructureFailure));
        assert_eq!(fault.finished_at(), None);
        assert_eq!(fault.started_at().is_some(), !panic_at_start);
    }
}

#[test]
fn hostile_clock_panic_payload_does_not_reach_stderr() {
    const CHILD: &str = "ASB_SCHEDULER_CLOCK_PANIC_CHILD";
    const SECRET: &str = "hostile external clock payload";
    if std::env::var_os(CHILD).is_some() {
        let panic_in_worker = Arc::new(AtomicBool::new(true));
        let result = Scheduler::new(WorkerClockPanic {
            base: SystemClock::start(),
            scheduler_thread: thread::current().id(),
            panic_in_worker,
        })
        .run(
            PointPlan::new(
                LoadModel::ClosedLoop,
                1,
                0,
                1,
                0,
                0,
                Duration::from_secs(1),
                Duration::from_millis(1),
                1,
            )
            .unwrap(),
            |_| AttemptOutcome::Completed,
        );
        assert_eq!(result.stop_reason(), StopReason::Contaminated);
        return;
    }
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "hostile_clock_panic_payload_does_not_reach_stderr",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(!String::from_utf8_lossy(&output.stderr).contains(SECRET));
}

#[test]
fn warmups_share_deadline_and_failure_budget_with_measurement() {
    let failed = PointPlan::new(
        LoadModel::ClosedLoop,
        3,
        2,
        1,
        0,
        2,
        Duration::from_secs(1),
        Duration::from_millis(1),
        6,
    )
    .unwrap();
    let result = Scheduler::new(SystemClock::start()).run(failed, |_| AttemptOutcome::Failed);
    assert_eq!(result.admission_stop_reason(), StopReason::FailureLimit);
    assert_eq!(result.measured().count(), 3);
    assert!(result.measured().all(|attempt| attempt.missed()));

    let deadline = PointPlan::new(
        LoadModel::ClosedLoop,
        3,
        1,
        1,
        0,
        0,
        Duration::from_millis(2),
        Duration::from_millis(1),
        7,
    )
    .unwrap();
    let result = Scheduler::new(SystemClock::start()).run(deadline, |_| {
        thread::sleep(Duration::from_millis(8));
        AttemptOutcome::Completed
    });
    assert_eq!(result.admission_stop_reason(), StopReason::DurationLimit);
    assert_eq!(result.measured().count(), 3);
    assert!(result.measured().all(|attempt| attempt.missed()));
}

#[test]
fn infrastructure_contamination_dominates_both_completion_orders() {
    for infrastructure_first in [false, true] {
        let plan = PointPlan::new(
            LoadModel::ClosedLoop,
            2,
            0,
            2,
            0,
            1,
            Duration::from_secs(1),
            Duration::from_millis(10),
            u64::from(infrastructure_first),
        )
        .unwrap();
        let result = Scheduler::new(SystemClock::start()).run(plan, move |id| {
            let infrastructure = id == u32::from(infrastructure_first);
            if infrastructure_first == infrastructure {
                thread::sleep(Duration::from_millis(1));
            } else {
                thread::sleep(Duration::from_millis(4));
            }
            if infrastructure {
                AttemptOutcome::InfrastructureFailure
            } else {
                AttemptOutcome::Failed
            }
        });
        assert_eq!(result.stop_reason(), StopReason::Contaminated);
        assert!(result.contaminated());
    }
}

#[test]
fn executor_panic_payload_does_not_reach_stderr() {
    const CHILD: &str = "ASB_SCHEDULER_PANIC_CHILD";
    const SECRET: &str = "private-panic-payload-should-never-leak";
    if std::env::var_os(CHILD).is_some() {
        let panic_plan = PointPlan::new(
            LoadModel::ClosedLoop,
            1,
            0,
            1,
            0,
            0,
            Duration::from_secs(1),
            Duration::from_millis(1),
            1,
        )
        .unwrap();
        let result = Scheduler::new(SystemClock::start()).run(panic_plan, |_| panic!("{SECRET}"));
        assert_eq!(result.stop_reason(), StopReason::Contaminated);
        return;
    }

    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "executor_panic_payload_does_not_reach_stderr",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(!String::from_utf8_lossy(&output.stderr).contains(SECRET));
}

#[test]
fn fast_open_loop_and_disabled_failure_limit_finish_all_work() {
    let open = PointPlan::new(
        LoadModel::OpenLoop {
            inter_arrival: Duration::from_millis(2),
        },
        3,
        0,
        3,
        0,
        0,
        Duration::from_secs(1),
        Duration::from_millis(1),
        0,
    )
    .unwrap();
    let completed = Scheduler::new(SystemClock::start()).run(open, |_| AttemptOutcome::Completed);
    assert_eq!(completed.stop_reason(), StopReason::Completed);
    assert_eq!(completed.missed_arrivals(), 0);

    let closed = PointPlan::new(
        LoadModel::ClosedLoop,
        3,
        0,
        1,
        0,
        0,
        Duration::from_secs(1),
        Duration::from_millis(1),
        0,
    )
    .unwrap();
    let failed = Scheduler::new(SystemClock::start()).run(closed, |_| AttemptOutcome::Failed);
    assert_eq!(failed.stop_reason(), StopReason::Completed);
    assert_eq!(failed.attempts().len(), 3);

    let clock = SystemClock::start();
    let _ = <SystemClock as asb_runtime::scheduler::MonotonicClock>::now(&clock);
}

#[test]
fn exhaustive_sweep_finds_known_capacity_without_monotonic_interpolation() {
    let order = capacity_order(8, 91).unwrap();
    let mut sorted = order.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, (1..=8).collect::<Vec<_>>());
    assert_eq!(order, capacity_order(8, 91).unwrap());
    assert_ne!(order, capacity_order(8, 92).unwrap());
    let points: Vec<_> = order
        .into_iter()
        .map(|concurrency| {
            let active = Arc::new(AtomicU32::new(0));
            let worker_active = Arc::clone(&active);
            let point = PointPlan::new(
                LoadModel::ClosedLoop,
                concurrency * 2,
                1,
                concurrency,
                0,
                0,
                Duration::from_secs(1),
                Duration::from_millis(1),
                4,
            )
            .unwrap();
            let result = Scheduler::new(SystemClock::start()).run(point, move |_| {
                let observed = worker_active.fetch_add(1, Ordering::SeqCst) + 1;
                thread::sleep(Duration::from_millis(3));
                worker_active.fetch_sub(1, Ordering::SeqCst);
                if observed <= 4 {
                    AttemptOutcome::Completed
                } else {
                    AttemptOutcome::Failed
                }
            });
            CapacityPoint {
                concurrency,
                decision: if result
                    .measured()
                    .all(|attempt| attempt.outcome() == Some(AttemptOutcome::Completed))
                {
                    CapacityDecision::Pass
                } else {
                    CapacityDecision::Fail
                },
            }
        })
        .collect();
    assert_eq!(highest_confirmed_capacity(&points), Some(4));
    assert_eq!(highest_confirmed_capacity(&[]), None);
    assert_eq!(capacity_order(0, 1), Err(PlanError::InvalidInFlight));
    assert_eq!(
        capacity_order(MAX_IN_FLIGHT + 1, 1),
        Err(PlanError::InvalidInFlight)
    );
    let zero_seed = capacity_order(8, 0).unwrap();
    assert_eq!(zero_seed.len(), 8);
    assert_eq!(capacity_order(1, 0).unwrap(), vec![1]);
    assert_eq!(
        highest_confirmed_capacity(&[CapacityPoint {
            concurrency: 1,
            decision: CapacityDecision::Inconclusive,
        }]),
        None
    );

    let nonmonotonic = [
        CapacityPoint {
            concurrency: 4,
            decision: CapacityDecision::Fail,
        },
        CapacityPoint {
            concurrency: 7,
            decision: CapacityDecision::Pass,
        },
    ];
    assert_eq!(highest_confirmed_capacity(&nonmonotonic), Some(7));
}
