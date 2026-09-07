// SPDX-License-Identifier: MIT
//! Monotonic scheduling for bounded capacity experiments.

use std::cell::Cell;
use std::collections::VecDeque;
use std::fmt;
use std::io;
use std::panic;
use std::sync::{Arc, Mutex, Once, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

type Executor = dyn Fn(AttemptContext) -> AttemptOutcome + Send + Sync;

thread_local! {
    static IN_EXECUTOR: Cell<bool> = const { Cell::new(false) };
}

static INSTALL_PANIC_BOUNDARY: Once = Once::new();

trait SpawnThread {
    fn spawn(&self, task: Box<dyn FnOnce() + Send + 'static>) -> io::Result<JoinHandle<()>>;
}

struct SystemSpawner;

impl SpawnThread for SystemSpawner {
    fn spawn(&self, task: Box<dyn FnOnce() + Send + 'static>) -> io::Result<JoinHandle<()>> {
        thread::Builder::new().spawn(task)
    }
}

/// Attempt ceiling for one point.
pub const MAX_ATTEMPTS: u32 = 1_000_000;
/// Concurrency ceiling for one point.
pub const MAX_IN_FLIGHT: u32 = 65_536;
/// Queue ceiling for one point.
pub const MAX_QUEUE: u32 = 1_000_000;
/// Elapsed-time ceiling for one point.
pub const MAX_POINT_DURATION: Duration = Duration::from_secs(86_400);

/// Source of elapsed monotonic time.
///
/// Implementations must return promptly. The scheduler catches panics, rejects
/// regression, excessive jumps and non-progress, and uses an independent
/// real-time watchdog; Rust cannot forcibly cancel a clock method that never
/// returns.
pub trait MonotonicClock: Clone + Send + Sync + 'static {
    /// Elapsed time from a private epoch.
    fn now(&self) -> Duration;
}

/// Instant-backed monotonic clock.
#[derive(Clone, Debug)]
pub struct SystemClock(Instant);

impl SystemClock {
    /// Start a private epoch.
    #[must_use]
    pub fn start() -> Self {
        Self(Instant::now())
    }
}

impl MonotonicClock for SystemClock {
    fn now(&self) -> Duration {
        self.0.elapsed()
    }
}

#[derive(Clone)]
struct CheckedClock<C> {
    inner: C,
    state: Arc<Mutex<ClockState>>,
    stall_limit: Duration,
}

struct ClockState {
    last: Option<Duration>,
    initial: Option<Duration>,
    initial_real: Instant,
    last_advanced: Instant,
    advanced: bool,
    failed: bool,
}

impl<C: MonotonicClock> CheckedClock<C> {
    fn new(inner: C, poll: Duration) -> Self {
        Self {
            inner,
            state: Arc::new(Mutex::new(ClockState {
                last: None,
                initial: None,
                initial_real: Instant::now(),
                last_advanced: Instant::now(),
                advanced: false,
                failed: false,
            })),
            stall_limit: poll
                .saturating_mul(4)
                .max(Duration::from_millis(10))
                .min(Duration::from_secs(1)),
        }
    }

    fn sample(&self) -> Result<Duration, ()> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.failed {
            return Err(());
        }
        let reset = enter_sensitive_boundary();
        let sampled = panic::catch_unwind(panic::AssertUnwindSafe(|| self.inner.now()));
        drop(reset);
        let Ok(value) = sampled else {
            state.failed = true;
            return Err(());
        };
        if state.last.is_some_and(|last| value < last) {
            state.failed = true;
            return Err(());
        }
        if let Some(initial) = state.initial {
            let clock_elapsed = value.saturating_sub(initial);
            let allowed = state
                .initial_real
                .elapsed()
                .saturating_add(self.stall_limit);
            if clock_elapsed > allowed {
                state.failed = true;
                return Err(());
            }
        } else {
            state.initial = Some(value);
            state.initial_real = Instant::now();
        }
        if state.last.is_some_and(|last| value > last) {
            state.advanced = true;
            state.last = Some(value);
            state.last_advanced = Instant::now();
        } else if state.last.is_none() {
            state.last = Some(value);
            state.last_advanced = Instant::now();
        } else if state.last_advanced.elapsed() >= self.stall_limit {
            state.failed = true;
            return Err(());
        }
        Ok(value)
    }

    fn last(&self) -> Duration {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .last
            .unwrap_or(Duration::ZERO)
    }

    fn invalidate(&self) {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .failed = true;
    }

    fn has_advanced(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .advanced
    }
}

/// Offered-load model.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoadModel {
    /// Refill a fixed number of active slots.
    ClosedLoop,
    /// Admit work on a fixed timeline.
    OpenLoop {
        /// Time between declared arrivals.
        inter_arrival: Duration,
    },
}

/// Invalid schedule input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlanError {
    /// At least one measured attempt is required.
    NoMeasuredAttempts,
    /// Attempt count exceeds its ceiling.
    TooManyAttempts,
    /// In-flight work is outside its bound.
    InvalidInFlight,
    /// Queueing is invalid for the selected model.
    InvalidQueue,
    /// Open-loop interval is zero.
    InvalidInterArrival,
    /// Duration or polling interval is invalid.
    InvalidDuration,
    /// The declared open-loop timeline exceeds its point deadline or time domain.
    ScheduleOverflow,
}

impl fmt::Display for PlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for PlanError {}

/// Validated immutable plan for one capacity point.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PointPlan {
    model: LoadModel,
    measured: u32,
    warmups: u32,
    in_flight: u32,
    queue: u32,
    max_failures: u32,
    max_duration: Duration,
    poll: Duration,
    seed: u64,
}

impl PointPlan {
    /// Validate all scheduling bounds.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        model: LoadModel,
        measured: u32,
        warmups: u32,
        in_flight: u32,
        queue: u32,
        max_failures: u32,
        max_duration: Duration,
        poll: Duration,
        seed: u64,
    ) -> Result<Self, PlanError> {
        if measured == 0 {
            return Err(PlanError::NoMeasuredAttempts);
        }
        if measured
            .checked_add(warmups)
            .is_none_or(|n| n > MAX_ATTEMPTS)
        {
            return Err(PlanError::TooManyAttempts);
        }
        if in_flight == 0 || in_flight > MAX_IN_FLIGHT {
            return Err(PlanError::InvalidInFlight);
        }
        if queue > MAX_QUEUE || matches!(model, LoadModel::ClosedLoop) && queue != 0 {
            return Err(PlanError::InvalidQueue);
        }
        if max_duration.is_zero()
            || max_duration > MAX_POINT_DURATION
            || poll.is_zero()
            || poll > max_duration
        {
            return Err(PlanError::InvalidDuration);
        }
        if let LoadModel::OpenLoop { inter_arrival } = model {
            if inter_arrival.is_zero() {
                return Err(PlanError::InvalidInterArrival);
            }
            let last_index = measured.max(warmups).saturating_sub(1);
            if inter_arrival
                .checked_mul(last_index)
                .is_none_or(|span| span > max_duration)
            {
                return Err(PlanError::ScheduleOverflow);
            }
        }
        Ok(Self {
            model,
            measured,
            warmups,
            in_flight,
            queue,
            max_failures,
            max_duration,
            poll,
            seed,
        })
    }
}

/// Raw executor result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttemptOutcome {
    /// Work completed for independent grading.
    Completed,
    /// Normal attempt failure.
    Failed,
    /// Attempt deadline elapsed.
    TimedOut,
    /// Attempt was cancelled.
    Cancelled,
    /// Infrastructure failure invalidated the point.
    InfrastructureFailure,
}

/// Immutable identity supplied at the executor boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AttemptContext {
    input_id: u32,
    warmup: bool,
}

impl AttemptContext {
    /// Seeded point-local input identity.
    #[must_use]
    pub fn input_id(self) -> u32 {
        self.input_id
    }

    /// Whether this execution is unmeasured warmup work.
    #[must_use]
    pub fn is_warmup(self) -> bool {
        self.warmup
    }
}

/// Why a planned arrival did not enter the executor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MissReason {
    /// The bounded open-loop queue was full at arrival.
    Backpressure,
    /// The point stopped before this planned arrival could start.
    PointStopped(StopReason),
}

/// Evidence for an admitted or missed arrival.
///
/// Fields are scheduler-owned so external callers cannot forge inconsistent
/// timing, outcome, or rejection evidence.
///
/// ```compile_fail
/// use asb_runtime::scheduler::{AttemptOutcome, AttemptRecord, MissReason};
/// use std::time::Duration;
/// let _forged = AttemptRecord {
///     input_id: 0,
///     warmup: false,
///     scheduled_at: Duration::ZERO,
///     started_at: None,
///     finished_at: None,
///     outcome: Some(AttemptOutcome::Completed),
///     queue_delay: Duration::ZERO,
///     missed: true,
///     miss_reason: Some(MissReason::Backpressure),
/// };
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AttemptRecord {
    /// Seeded point-local input identity.
    input_id: u32,
    /// Whether this is unmeasured warmup work.
    warmup: bool,
    /// Declared monotonic arrival.
    scheduled_at: Duration,
    /// Actual executor-boundary start, absent when the executor never started.
    started_at: Option<Duration>,
    /// Terminal time, absent for a miss.
    finished_at: Option<Duration>,
    /// Terminal result, absent for a miss.
    outcome: Option<AttemptOutcome>,
    /// Delay from arrival to start.
    queue_delay: Duration,
    /// Whether backpressure or a point stop rejected this arrival before start.
    missed: bool,
    /// Causal rejection evidence, present exactly when `missed` is true.
    miss_reason: Option<MissReason>,
}

impl AttemptRecord {
    /// Seeded point-local input identity.
    #[must_use]
    pub fn input_id(&self) -> u32 {
        self.input_id
    }

    /// Whether this is unmeasured warmup work.
    #[must_use]
    pub fn is_warmup(&self) -> bool {
        self.warmup
    }

    /// Declared monotonic arrival.
    #[must_use]
    pub fn scheduled_at(&self) -> Duration {
        self.scheduled_at
    }

    /// Actual executor-boundary start.
    #[must_use]
    pub fn started_at(&self) -> Option<Duration> {
        self.started_at
    }

    /// Terminal time when the clock remained valid.
    #[must_use]
    pub fn finished_at(&self) -> Option<Duration> {
        self.finished_at
    }

    /// Terminal executor result.
    #[must_use]
    pub fn outcome(&self) -> Option<AttemptOutcome> {
        self.outcome
    }

    /// Delay from arrival to actual executor-boundary start.
    #[must_use]
    pub fn queue_delay(&self) -> Duration {
        self.queue_delay
    }

    /// Whether this planned arrival was rejected before executor start.
    #[must_use]
    pub fn missed(&self) -> bool {
        self.missed
    }

    /// Causal rejection evidence.
    #[must_use]
    pub fn miss_reason(&self) -> Option<MissReason> {
        self.miss_reason
    }
}

/// Why admission stopped.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StopReason {
    /// All arrivals were handled.
    Completed,
    /// Failure limit was reached.
    FailureLimit,
    /// Wall-time limit was reached.
    DurationLimit,
    /// Infrastructure invalidated the point.
    Contaminated,
}

/// Complete raw result for one point.
///
/// Construction and mutation remain scheduler-owned.
///
/// ```compile_fail
/// use asb_runtime::scheduler::{PointResult, StopReason};
/// fn forge(mut result: PointResult) {
///     result.stop_reason = StopReason::Completed;
///     result.contaminated = false;
/// }
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PointResult {
    /// Warmup and measured records.
    attempts: Vec<AttemptRecord>,
    /// Terminal scheduler decision.
    stop_reason: StopReason,
    /// First reason admission ceased; later terminal evidence never rewrites it.
    admission_stop_reason: StopReason,
    /// Whether capacity claims are invalid.
    contaminated: bool,
}

impl PointResult {
    /// Iterate measured records only.
    pub fn measured(&self) -> impl Iterator<Item = &AttemptRecord> {
        self.attempts.iter().filter(|a| !a.warmup)
    }

    /// All immutable warmup and measured records in planned order.
    #[must_use]
    pub fn attempts(&self) -> &[AttemptRecord] {
        &self.attempts
    }

    /// Terminal scheduler decision after draining admitted work.
    #[must_use]
    pub fn stop_reason(&self) -> StopReason {
        self.stop_reason
    }

    /// Immutable reason that admission first ceased.
    #[must_use]
    pub fn admission_stop_reason(&self) -> StopReason {
        self.admission_stop_reason
    }

    /// Whether infrastructure evidence invalidated capacity claims.
    #[must_use]
    pub fn contaminated(&self) -> bool {
        self.contaminated
    }

    /// Count measured missed arrivals.
    #[must_use]
    pub fn missed_arrivals(&self) -> usize {
        self.measured().filter(|a| a.missed).count()
    }
}

/// Executes immutable point plans.
#[derive(Clone, Debug)]
pub struct Scheduler<C>(C);

impl<C: MonotonicClock> Scheduler<C> {
    /// Use an injected monotonic clock.
    pub fn new(clock: C) -> Self {
        Self(clock)
    }

    /// Run warmups and measurements, draining every admitted thread.
    pub fn run<E>(&self, plan: PointPlan, executor: E) -> PointResult
    where
        E: Fn(u32) -> AttemptOutcome + Send + Sync + 'static,
    {
        self.run_with_context(plan, move |attempt| executor(attempt.input_id()))
    }

    /// Run with explicit phase identity at the executor boundary.
    pub fn run_with_context<E>(&self, plan: PointPlan, executor: E) -> PointResult
    where
        E: Fn(AttemptContext) -> AttemptOutcome + Send + Sync + 'static,
    {
        self.run_with_spawner(plan, executor, &SystemSpawner)
    }

    fn run_with_spawner<E, S>(&self, plan: PointPlan, executor: E, spawner: &S) -> PointResult
    where
        E: Fn(AttemptContext) -> AttemptOutcome + Send + Sync + 'static,
        S: SpawnThread,
    {
        install_panic_boundary();
        let executor: Arc<Executor> = Arc::new(executor);
        let clock = CheckedClock::new(self.0.clone(), plan.poll);
        let Some(real_deadline) = Instant::now().checked_add(plan.max_duration) else {
            return rejected_point(plan, StopReason::Contaminated, Duration::ZERO);
        };
        let Ok(initial) = clock.sample() else {
            return rejected_point(plan, StopReason::Contaminated, Duration::ZERO);
        };
        let Some(domain_span) = plan.max_duration.checked_mul(2) else {
            return rejected_point(plan, StopReason::Contaminated, initial);
        };
        if initial.checked_add(domain_span).is_none() {
            return rejected_point(plan, StopReason::Contaminated, initial);
        }
        let deadline = initial
            .checked_add(plan.max_duration)
            .expect("clock domain was checked");
        let mut attempts = Vec::new();
        let mut failures = 0;
        let mut stop = StopReason::Completed;
        let mut admission_stop = None;
        for (warmup, count, seed) in [
            (true, plan.warmups, !plan.seed),
            (false, plan.measured, plan.seed),
        ] {
            if count == 0 {
                continue;
            }
            let mut order: Vec<u32> = (0..count).collect();
            shuffle(&mut order, seed);
            if let Some(reason) = admission_stop {
                let now = clock.last().min(deadline);
                let mut rejected = Vec::new();
                let mut rejected_next = 0;
                reject_remaining(
                    &mut rejected,
                    &mut VecDeque::new(),
                    &order,
                    &mut rejected_next,
                    warmup,
                    plan.model,
                    now,
                    now,
                    reason,
                );
                attempts.extend(rejected);
                continue;
            }
            let phase = self.phase(
                plan,
                warmup,
                order,
                deadline,
                real_deadline,
                failures,
                Arc::clone(&executor),
                clock.clone(),
                spawner,
            );
            attempts.extend(phase.attempts);
            failures = phase.failures;
            stop = phase.stop;
            admission_stop = admission_stop.or(phase.admission_stop);
        }
        while !clock.has_advanced() {
            if clock.sample().is_err() {
                stop = StopReason::Contaminated;
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
        PointResult {
            attempts,
            stop_reason: stop,
            admission_stop_reason: admission_stop.unwrap_or(StopReason::Completed),
            contaminated: stop == StopReason::Contaminated,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn phase<S: SpawnThread>(
        &self,
        plan: PointPlan,
        warmup: bool,
        order: Vec<u32>,
        deadline: Duration,
        real_deadline: Instant,
        mut failures: u32,
        executor: Arc<Executor>,
        clock: CheckedClock<C>,
        spawner: &S,
    ) -> Phase {
        let (sender, receiver) = mpsc::channel();
        let mut handles = Vec::new();
        let mut attempts = Vec::new();
        let mut queue: VecDeque<usize> = VecDeque::new();
        let mut next = 0;
        let mut active = 0;
        let started = match clock.sample() {
            Ok(now) => now,
            Err(()) => {
                return rejected_phase(
                    order,
                    warmup,
                    plan.model,
                    clock.last(),
                    failures,
                    StopReason::Contaminated,
                );
            }
        };
        if started >= deadline {
            return rejected_phase(
                order,
                warmup,
                plan.model,
                deadline,
                failures,
                StopReason::DurationLimit,
            );
        }
        let mut stop = StopReason::Completed;
        let mut admission_stop = None;
        while next < order.len() || active > 0 || !queue.is_empty() {
            while let Ok(event) = receiver.try_recv() {
                match event {
                    WorkerEvent::Started { id, at } => start(&mut attempts, id, at),
                    WorkerEvent::Finished { id, at, outcome } => {
                        active -= 1;
                        finish(&mut attempts, id, at, outcome);
                        if outcome != AttemptOutcome::Completed {
                            failures += 1;
                        }
                        observe_outcome(&mut stop, outcome, failures, plan.max_failures);
                        if admission_stop.is_none() && stop != StopReason::Completed {
                            admission_stop = Some(stop);
                        }
                    }
                    WorkerEvent::ClockFault { id } => {
                        active -= 1;
                        clock_fault(&mut attempts, id);
                        failures += 1;
                        stop = StopReason::Contaminated;
                        admission_stop.get_or_insert(StopReason::Contaminated);
                    }
                }
            }
            let now = match clock.sample() {
                Ok(now) => now,
                Err(()) => {
                    stop = StopReason::Contaminated;
                    admission_stop.get_or_insert(StopReason::Contaminated);
                    clock.last()
                }
            };
            if now >= deadline && admission_stop.is_none() {
                stop = StopReason::DurationLimit;
                admission_stop = Some(StopReason::DurationLimit);
            } else if Instant::now() >= real_deadline && admission_stop.is_none() {
                clock.invalidate();
                stop = StopReason::Contaminated;
                admission_stop = Some(StopReason::Contaminated);
            }
            if let Some(reason) = admission_stop {
                reject_remaining(
                    &mut attempts,
                    &mut queue,
                    &order,
                    &mut next,
                    warmup,
                    plan.model,
                    started,
                    now,
                    reason,
                );
            } else {
                while active < plan.in_flight {
                    let Some(index) = queue.pop_front() else {
                        break;
                    };
                    if !launch(
                        &mut attempts,
                        index,
                        now,
                        &mut active,
                        &mut handles,
                        sender.clone(),
                        Arc::clone(&executor),
                        clock.clone(),
                        spawner,
                    ) {
                        failures += 1;
                        stop = StopReason::Contaminated;
                        admission_stop.get_or_insert(StopReason::Contaminated);
                        break;
                    }
                }
                if stop != StopReason::Completed {
                    continue;
                }
                match plan.model {
                    LoadModel::ClosedLoop => {
                        while active < plan.in_flight && next < order.len() {
                            let index = arrival(&mut attempts, order[next], warmup, now);
                            next += 1;
                            if !launch(
                                &mut attempts,
                                index,
                                now,
                                &mut active,
                                &mut handles,
                                sender.clone(),
                                Arc::clone(&executor),
                                clock.clone(),
                                spawner,
                            ) {
                                failures += 1;
                                stop = StopReason::Contaminated;
                                admission_stop.get_or_insert(StopReason::Contaminated);
                                break;
                            }
                        }
                    }
                    LoadModel::OpenLoop { inter_arrival } => {
                        while next < order.len() {
                            let scheduled = inter_arrival
                                .checked_mul(next as u32)
                                .and_then(|offset| started.checked_add(offset))
                                .expect(
                                    "validated open-loop timeline fits the checked clock domain",
                                );
                            if scheduled > now {
                                break;
                            }
                            let index = arrival(&mut attempts, order[next], warmup, scheduled);
                            next += 1;
                            if active < plan.in_flight {
                                if !launch(
                                    &mut attempts,
                                    index,
                                    now,
                                    &mut active,
                                    &mut handles,
                                    sender.clone(),
                                    Arc::clone(&executor),
                                    clock.clone(),
                                    spawner,
                                ) {
                                    failures += 1;
                                    stop = StopReason::Contaminated;
                                    admission_stop.get_or_insert(StopReason::Contaminated);
                                    break;
                                }
                            } else if queue.len() < plan.queue as usize {
                                queue.push_back(index);
                            } else {
                                attempts[index].missed = true;
                                attempts[index].miss_reason = Some(MissReason::Backpressure);
                            }
                        }
                    }
                }
            }
            if active > 0 || next < order.len() || !queue.is_empty() {
                let real_remaining = real_deadline.saturating_duration_since(Instant::now());
                let clock_remaining = deadline.saturating_sub(now);
                let delay = plan.poll.min(real_remaining).min(clock_remaining);
                if delay.is_zero() {
                    thread::yield_now();
                } else {
                    thread::sleep(delay);
                }
            }
        }
        for handle in handles {
            let _ = handle.join();
        }
        Phase {
            attempts,
            failures,
            stop,
            admission_stop,
        }
    }
}

struct Phase {
    attempts: Vec<AttemptRecord>,
    failures: u32,
    stop: StopReason,
    admission_stop: Option<StopReason>,
}

fn arrival(records: &mut Vec<AttemptRecord>, id: u32, warmup: bool, at: Duration) -> usize {
    records.push(AttemptRecord {
        input_id: id,
        warmup,
        scheduled_at: at,
        started_at: None,
        finished_at: None,
        outcome: None,
        queue_delay: Duration::ZERO,
        missed: false,
        miss_reason: None,
    });
    records.len() - 1
}

#[allow(clippy::too_many_arguments)]
fn launch<C>(
    records: &mut [AttemptRecord],
    index: usize,
    at: Duration,
    active: &mut u32,
    handles: &mut Vec<JoinHandle<()>>,
    sender: mpsc::Sender<WorkerEvent>,
    executor: Arc<Executor>,
    clock: CheckedClock<C>,
    spawner: &impl SpawnThread,
) -> bool
where
    C: MonotonicClock,
{
    let context = AttemptContext {
        input_id: records[index].input_id,
        warmup: records[index].warmup,
    };
    let id = context.input_id;
    let task = Box::new(move || {
        let Ok(started) = clock.sample() else {
            let _ = sender.send(WorkerEvent::ClockFault { id });
            return;
        };
        let _ = sender.send(WorkerEvent::Started { id, at: started });
        let reset = enter_sensitive_boundary();
        let outcome = panic::catch_unwind(panic::AssertUnwindSafe(|| executor(context)))
            .unwrap_or(AttemptOutcome::InfrastructureFailure);
        drop(reset);
        match clock.sample() {
            Ok(at) => {
                let _ = sender.send(WorkerEvent::Finished { id, at, outcome });
            }
            Err(()) => {
                let _ = sender.send(WorkerEvent::ClockFault { id });
            }
        }
    });
    match spawner.spawn(task) {
        Ok(handle) => {
            *active += 1;
            handles.push(handle);
            true
        }
        Err(_) => {
            records[index].finished_at = Some(at);
            records[index].outcome = Some(AttemptOutcome::InfrastructureFailure);
            false
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum WorkerEvent {
    Started {
        id: u32,
        at: Duration,
    },
    Finished {
        id: u32,
        at: Duration,
        outcome: AttemptOutcome,
    },
    ClockFault {
        id: u32,
    },
}

#[allow(clippy::too_many_arguments)]
fn reject_remaining(
    attempts: &mut Vec<AttemptRecord>,
    queue: &mut VecDeque<usize>,
    order: &[u32],
    next: &mut usize,
    warmup: bool,
    model: LoadModel,
    phase_started: Duration,
    now: Duration,
    stop: StopReason,
) {
    for index in queue.drain(..) {
        attempts[index].missed = true;
        attempts[index].miss_reason = Some(MissReason::PointStopped(stop));
    }
    while *next < order.len() {
        let scheduled = match model {
            LoadModel::ClosedLoop => now,
            LoadModel::OpenLoop { inter_arrival } => inter_arrival
                .checked_mul(*next as u32)
                .and_then(|offset| phase_started.checked_add(offset))
                .expect("validated open-loop timeline fits the checked clock domain"),
        };
        let index = arrival(attempts, order[*next], warmup, scheduled);
        attempts[index].missed = true;
        attempts[index].miss_reason = Some(MissReason::PointStopped(stop));
        *next += 1;
    }
}

fn observe_outcome(
    stop: &mut StopReason,
    outcome: AttemptOutcome,
    failures: u32,
    max_failures: u32,
) {
    if outcome == AttemptOutcome::InfrastructureFailure {
        *stop = StopReason::Contaminated;
    } else if *stop != StopReason::Contaminated && max_failures != 0 && failures >= max_failures {
        *stop = StopReason::FailureLimit;
    }
}

struct ExecutorBoundaryReset(bool);

impl Drop for ExecutorBoundaryReset {
    fn drop(&mut self) {
        IN_EXECUTOR.with(|inside| inside.set(self.0));
    }
}

fn enter_sensitive_boundary() -> ExecutorBoundaryReset {
    let previous = IN_EXECUTOR.with(|inside| {
        let previous = inside.get();
        inside.set(true);
        previous
    });
    ExecutorBoundaryReset(previous)
}

fn install_panic_boundary() {
    INSTALL_PANIC_BOUNDARY.call_once(|| {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |information| {
            if !IN_EXECUTOR.with(Cell::get) {
                previous(information);
            }
        }));
    });
}

fn finish(records: &mut [AttemptRecord], id: u32, at: Duration, outcome: AttemptOutcome) {
    if let Some(record) = records.iter_mut().rev().find(|record| {
        record.input_id == id && record.started_at.is_some() && record.outcome.is_none()
    }) {
        record.finished_at = Some(at);
        record.outcome = Some(outcome);
    }
}

fn start(records: &mut [AttemptRecord], id: u32, at: Duration) {
    if let Some(record) = records.iter_mut().rev().find(|record| {
        record.input_id == id && record.started_at.is_none() && record.outcome.is_none()
    }) {
        record.started_at = Some(at);
        record.queue_delay = at.saturating_sub(record.scheduled_at);
    }
}

fn clock_fault(records: &mut [AttemptRecord], id: u32) {
    if let Some(record) = records
        .iter_mut()
        .rev()
        .find(|record| record.input_id == id && record.outcome.is_none())
    {
        record.outcome = Some(AttemptOutcome::InfrastructureFailure);
    }
}

fn rejected_phase(
    order: Vec<u32>,
    warmup: bool,
    model: LoadModel,
    now: Duration,
    failures: u32,
    reason: StopReason,
) -> Phase {
    let mut attempts = Vec::new();
    let mut next = 0;
    reject_remaining(
        &mut attempts,
        &mut VecDeque::new(),
        &order,
        &mut next,
        warmup,
        model,
        now,
        now,
        reason,
    );
    Phase {
        attempts,
        failures,
        stop: reason,
        admission_stop: Some(reason),
    }
}

fn rejected_point(plan: PointPlan, reason: StopReason, now: Duration) -> PointResult {
    let mut attempts = Vec::new();
    for (warmup, count, seed) in [
        (true, plan.warmups, !plan.seed),
        (false, plan.measured, plan.seed),
    ] {
        let mut order = (0..count).collect::<Vec<_>>();
        shuffle(&mut order, seed);
        attempts.extend(rejected_phase(order, warmup, plan.model, now, 0, reason).attempts);
    }
    PointResult {
        attempts,
        stop_reason: reason,
        admission_stop_reason: reason,
        contaminated: reason == StopReason::Contaminated,
    }
}

/// Conservative assessment supplied by the analysis layer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapacityDecision {
    /// Required evidence passed.
    Pass,
    /// Required evidence failed.
    Fail,
    /// Evidence was incomplete.
    Inconclusive,
}

/// One tested capacity point.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapacityPoint {
    /// Tested concurrency.
    pub concurrency: u32,
    /// Conservative assessment.
    pub decision: CapacityDecision,
}

/// Build a seeded exhaustive order, exploring powers of two first.
pub fn capacity_order(max: u32, seed: u64) -> Result<Vec<u32>, PlanError> {
    if max == 0 || max > MAX_IN_FLIGHT {
        return Err(PlanError::InvalidInFlight);
    }
    let mut exploratory = Vec::new();
    let mut point: u32 = 1;
    loop {
        exploratory.push(point);
        if point >= max {
            break;
        }
        point = point.saturating_mul(2).min(max);
    }
    let mut refinement: Vec<_> = (1..=max)
        .filter(|candidate| !exploratory.contains(candidate))
        .collect();
    shuffle(&mut exploratory, seed ^ u64::MAX);
    shuffle(&mut refinement, seed);
    exploratory.extend(refinement);
    Ok(exploratory)
}

/// Return only the highest actually tested passing capacity.
#[must_use]
pub fn highest_confirmed_capacity(points: &[CapacityPoint]) -> Option<u32> {
    points
        .iter()
        .filter(|point| point.decision == CapacityDecision::Pass)
        .map(|point| point.concurrency)
        .max()
}

fn shuffle(values: &mut [u32], mut state: u64) {
    if state == 0 {
        state = 0x9e37_79b9_7f4a_7c15;
    }
    for index in (1..values.len()).rev() {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        values.swap(index, state as usize % (index + 1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct FailSecondSpawn(AtomicUsize);

    struct DelayedStart(Duration);

    impl SpawnThread for FailSecondSpawn {
        fn spawn(&self, task: Box<dyn FnOnce() + Send + 'static>) -> io::Result<JoinHandle<()>> {
            if self.0.fetch_add(1, Ordering::SeqCst) == 1 {
                Err(io::Error::other("injected spawn exhaustion"))
            } else {
                thread::Builder::new().spawn(task)
            }
        }
    }

    impl SpawnThread for DelayedStart {
        fn spawn(&self, task: Box<dyn FnOnce() + Send + 'static>) -> io::Result<JoinHandle<()>> {
            let delay = self.0;
            thread::Builder::new().spawn(move || {
                thread::sleep(delay);
                task();
            })
        }
    }

    #[test]
    fn queue_delay_includes_actual_thread_start_delay() {
        let plan = PointPlan::new(
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
        let result = Scheduler::new(SystemClock::start()).run_with_spawner(
            plan,
            |_| AttemptOutcome::Completed,
            &DelayedStart(Duration::from_millis(30)),
        );
        assert_eq!(result.stop_reason(), StopReason::Completed);
        assert_eq!(result.attempts().len(), 1);
        assert!(result.attempts()[0].queue_delay() >= Duration::from_millis(20));
    }

    #[test]
    fn contextual_executor_distinguishes_duplicate_ids_across_phases() {
        let plan = PointPlan::new(
            LoadModel::ClosedLoop,
            2,
            2,
            2,
            0,
            0,
            Duration::from_secs(1),
            Duration::from_millis(1),
            9,
        )
        .unwrap();
        let observed = Arc::new(Mutex::new(Vec::new()));
        let worker_observed = Arc::clone(&observed);
        let result = Scheduler::new(SystemClock::start()).run_with_context(plan, move |context| {
            worker_observed
                .lock()
                .unwrap()
                .push((context.is_warmup(), context.input_id()));
            AttemptOutcome::Completed
        });
        let observed = observed.lock().unwrap();
        assert_eq!(observed.len(), 4);
        assert!(observed[..2].iter().all(|(warmup, _)| *warmup));
        assert!(observed[2..].iter().all(|(warmup, _)| !*warmup));
        assert_eq!(
            observed
                .iter()
                .filter(|(_, input_id)| *input_id == 0)
                .count(),
            2
        );
        assert_eq!(result.attempts().len(), 4);
    }

    #[test]
    fn spawn_failure_contaminates_and_drains_admitted_threads() {
        let plan = PointPlan::new(
            LoadModel::ClosedLoop,
            3,
            0,
            2,
            0,
            1,
            Duration::from_secs(1),
            Duration::from_millis(1),
            1,
        )
        .unwrap();
        let completed = Arc::new(AtomicUsize::new(0));
        let worker_completed = Arc::clone(&completed);
        let result = Scheduler::new(SystemClock::start()).run_with_spawner(
            plan,
            move |_| {
                thread::sleep(Duration::from_millis(5));
                worker_completed.fetch_add(1, Ordering::SeqCst);
                AttemptOutcome::Completed
            },
            &FailSecondSpawn(AtomicUsize::new(0)),
        );
        assert_eq!(result.stop_reason(), StopReason::Contaminated);
        assert!(result.contaminated());
        assert_eq!(completed.load(Ordering::SeqCst), 1);
        assert_eq!(
            result
                .attempts()
                .iter()
                .filter(|attempt| {
                    attempt.outcome() == Some(AttemptOutcome::InfrastructureFailure)
                })
                .count(),
            1
        );
        assert_eq!(
            result
                .attempts()
                .iter()
                .filter(|attempt| attempt.outcome() == Some(AttemptOutcome::Completed))
                .count(),
            1
        );
    }
}
