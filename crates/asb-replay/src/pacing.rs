// SPDX-License-Identifier: MIT
//! Monotonic response pacing and independently supplied replay-headroom evidence.

use std::io::Write;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::{Duration, Instant};

use thiserror::Error;

const MAX_PACING_DELAY: Duration = Duration::from_secs(300);
const DEFAULT_CANCELLATION_POLL: Duration = Duration::from_millis(2);

/// Distinguishes replay of recorded timing from synthetic timing experiments.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PacingClassification {
    /// Timing is immediate, fixed, or copied from recorded event offsets.
    RecordedResponse,
    /// Timing or failure behavior is generated from a declared seed.
    SyntheticScenario,
}

/// Requested response timing behavior.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PacingMode {
    /// Write all response segments without an intentional delay.
    Immediate,
    /// Delay the first segment, then use a constant interval.
    Fixed {
        /// Delay from response start to the first segment.
        time_to_first_segment: Duration,
        /// Delay between later segments.
        inter_segment: Duration,
    },
    /// Reproduce cassette monotonic offsets for each semantic segment.
    Original,
    /// Generate deterministic per-segment delays and an optional injected failure.
    SeededSynthetic {
        /// Seed for the version-one SplitMix64 schedule.
        seed: u64,
        /// Inclusive minimum delay before each segment.
        minimum_delay: Duration,
        /// Inclusive maximum delay before each segment.
        maximum_delay: Duration,
        /// Fail before this zero-based segment, when present.
        fail_before_segment: Option<usize>,
    },
}

impl PacingMode {
    /// Evidence classification implied by this mode.
    pub const fn classification(self) -> PacingClassification {
        match self {
            Self::SeededSynthetic { .. } => PacingClassification::SyntheticScenario,
            _ => PacingClassification::RecordedResponse,
        }
    }
}

/// Tightenable delivery safety bounds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PacingConfig {
    /// Requested pacing behavior.
    pub mode: PacingMode,
    /// Maximum time one segment write may occupy before evidence is invalid.
    pub max_segment_write: Duration,
    /// Maximum wake-up lateness relative to the requested offset.
    pub max_lateness: Duration,
    /// Maximum interval between cancellation observations while waiting.
    pub cancellation_poll: Duration,
}

impl PacingConfig {
    /// Immediate delivery with conservative local bounds.
    pub const fn immediate() -> Self {
        Self {
            mode: PacingMode::Immediate,
            max_segment_write: Duration::from_secs(30),
            max_lateness: Duration::from_secs(30),
            cancellation_poll: DEFAULT_CANCELLATION_POLL,
        }
    }

    pub(crate) fn validate(self) -> Result<Self, PacingError> {
        if self.max_segment_write.is_zero()
            || self.max_segment_write > MAX_PACING_DELAY
            || self.max_lateness > MAX_PACING_DELAY
            || self.cancellation_poll.is_zero()
            || self.cancellation_poll > Duration::from_secs(1)
        {
            return Err(PacingError::InvalidConfiguration);
        }
        match self.mode {
            PacingMode::Fixed {
                time_to_first_segment,
                inter_segment,
            } if time_to_first_segment > MAX_PACING_DELAY || inter_segment > MAX_PACING_DELAY => {
                Err(PacingError::InvalidConfiguration)
            }
            PacingMode::SeededSynthetic {
                minimum_delay,
                maximum_delay,
                ..
            } if minimum_delay > maximum_delay || maximum_delay > MAX_PACING_DELAY => {
                Err(PacingError::InvalidConfiguration)
            }
            _ => Ok(self),
        }
    }
}

impl Default for PacingConfig {
    fn default() -> Self {
        Self::immediate()
    }
}

/// Cloneable cooperative cancellation shared with the caller.
#[derive(Clone, Debug, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    /// Request cancellation. A paced wait observes it within its configured poll interval.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    /// Whether cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

/// Timing evidence for one completely written segment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SegmentTiming {
    /// Zero-based semantic segment index.
    pub segment: usize,
    /// Requested offset from delivery start.
    pub desired_offset: Duration,
    /// Observed offset immediately before the write.
    pub write_started_offset: Duration,
    /// Observed offset after the complete write and flush.
    pub write_completed_offset: Duration,
    /// Amount by which write start exceeded the requested offset.
    pub lateness: Duration,
}

/// Complete successful pacing evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PacingReport {
    /// Recorded-response or synthetic-scenario classification.
    pub classification: PacingClassification,
    /// Requested and achieved observations for every segment.
    pub segments: Vec<SegmentTiming>,
}

/// Fail-closed delivery errors without response contents.
#[derive(Debug, Error)]
pub enum PacingError {
    /// Bounds, offsets, or a synthetic failure index are invalid.
    #[error("invalid replay pacing configuration")]
    InvalidConfiguration,
    /// Delivery was cancelled before all segments were written.
    #[error("replay pacing cancelled")]
    Cancelled {
        /// Number of complete segments accepted by the writer.
        completed_segments: usize,
    },
    /// A declared seeded failure occurred.
    #[error("seeded synthetic replay failure")]
    SyntheticFailure {
        /// Number of complete segments accepted by the writer.
        completed_segments: usize,
    },
    /// A write completed beyond its declared blocking bound.
    #[error("replay response backpressure bound exceeded")]
    Backpressure {
        /// Number of complete segments accepted by the writer.
        completed_segments: usize,
    },
    /// The writer rejected response bytes.
    #[error("paced replay response write failed")]
    Io(#[source] std::io::Error),
}

impl PacingError {
    /// Completely written segments known at the failure boundary.
    pub const fn completed_segments(&self) -> usize {
        match self {
            Self::Cancelled { completed_segments }
            | Self::SyntheticFailure { completed_segments }
            | Self::Backpressure { completed_segments } => *completed_segments,
            Self::InvalidConfiguration | Self::Io(_) => 0,
        }
    }
}

/// Clock boundary used to test monotonic scheduling independently of wall time.
pub trait MonotonicClock {
    /// Monotonic elapsed time relative to this clock's origin.
    fn elapsed(&self) -> Duration;
    /// Sleep for at most the requested duration.
    fn sleep(&self, duration: Duration);
}

/// Production monotonic clock backed by [`Instant`].
#[derive(Debug)]
pub struct SystemMonotonicClock(Instant);

impl SystemMonotonicClock {
    /// Start a fresh monotonic clock.
    pub fn start() -> Self {
        Self(Instant::now())
    }
}

impl MonotonicClock for SystemMonotonicClock {
    fn elapsed(&self) -> Duration {
        self.0.elapsed()
    }

    fn sleep(&self, duration: Duration) {
        thread::sleep(duration);
    }
}

/// Pace and completely write semantic response segments using a monotonic clock.
///
/// `recorded_offsets` must contain one nondecreasing offset per segment. It is
/// used only by [`PacingMode::Original`] but is always shape-checked. The writer
/// must itself impose an operating-system or transport timeout: elapsed-time
/// checking can classify a completed slow write but cannot interrupt a generic
/// blocking [`Write`] implementation.
pub fn write_paced_segments<W: Write>(
    writer: &mut W,
    segments: &[Vec<u8>],
    recorded_offsets: &[Duration],
    config: PacingConfig,
    cancellation: &CancellationToken,
) -> Result<PacingReport, PacingError> {
    let clock = SystemMonotonicClock::start();
    write_paced_segments_with_clock(
        writer,
        segments,
        recorded_offsets,
        config,
        cancellation,
        &clock,
    )
}

pub(crate) fn write_paced_segments_with_clock<W: Write, C: MonotonicClock>(
    writer: &mut W,
    segments: &[Vec<u8>],
    recorded_offsets: &[Duration],
    config: PacingConfig,
    cancellation: &CancellationToken,
    clock: &C,
) -> Result<PacingReport, PacingError> {
    let config = config.validate()?;
    let desired = desired_offsets(config.mode, segments.len(), recorded_offsets)?;
    let mut timings = Vec::with_capacity(segments.len());
    for (index, (segment, desired_offset)) in segments.iter().zip(desired.into_iter()).enumerate() {
        if matches!(
            config.mode,
            PacingMode::SeededSynthetic {
                fail_before_segment: Some(failure),
                ..
            } if failure == index
        ) {
            return Err(PacingError::SyntheticFailure {
                completed_segments: index,
            });
        }
        wait_until(
            clock,
            desired_offset,
            config.cancellation_poll,
            cancellation,
        )
        .map_err(|()| PacingError::Cancelled {
            completed_segments: index,
        })?;
        let started = clock.elapsed();
        let lateness = started.saturating_sub(desired_offset);
        if lateness > config.max_lateness {
            return Err(PacingError::Backpressure {
                completed_segments: index,
            });
        }
        writer.write_all(segment).map_err(PacingError::Io)?;
        writer.flush().map_err(PacingError::Io)?;
        let completed = clock.elapsed();
        if completed.saturating_sub(started) > config.max_segment_write {
            return Err(PacingError::Backpressure {
                completed_segments: index + 1,
            });
        }
        if index + 1 < segments.len() && cancellation.is_cancelled() {
            return Err(PacingError::Cancelled {
                completed_segments: index + 1,
            });
        }
        timings.push(SegmentTiming {
            segment: index,
            desired_offset,
            write_started_offset: started,
            write_completed_offset: completed,
            lateness,
        });
    }
    Ok(PacingReport {
        classification: config.mode.classification(),
        segments: timings,
    })
}

fn wait_until<C: MonotonicClock>(
    clock: &C,
    target: Duration,
    poll: Duration,
    cancellation: &CancellationToken,
) -> Result<(), ()> {
    loop {
        if cancellation.is_cancelled() {
            return Err(());
        }
        let remaining = target.saturating_sub(clock.elapsed());
        if remaining.is_zero() {
            return Ok(());
        }
        clock.sleep(remaining.min(poll));
    }
}

fn desired_offsets(
    mode: PacingMode,
    count: usize,
    recorded: &[Duration],
) -> Result<Vec<Duration>, PacingError> {
    if count > usize::try_from(crate::cassette::MAX_EVENTS).expect("u32 fits usize")
        || recorded.len() != count
        || recorded.windows(2).any(|pair| pair[0] > pair[1])
        || recorded
            .last()
            .is_some_and(|offset| *offset > MAX_PACING_DELAY)
    {
        return Err(PacingError::InvalidConfiguration);
    }
    if let PacingMode::SeededSynthetic {
        fail_before_segment: Some(index),
        ..
    } = mode
        && index >= count
    {
        return Err(PacingError::InvalidConfiguration);
    }
    match mode {
        PacingMode::Immediate => Ok(vec![Duration::ZERO; count]),
        PacingMode::Original => Ok(recorded.to_vec()),
        PacingMode::Fixed {
            time_to_first_segment,
            inter_segment,
        } => (0..count)
            .map(|index| {
                let repetitions =
                    u32::try_from(index).map_err(|_| PacingError::InvalidConfiguration)?;
                let offset = inter_segment
                    .checked_mul(repetitions)
                    .and_then(|delay| time_to_first_segment.checked_add(delay))
                    .ok_or(PacingError::InvalidConfiguration)?;
                if offset > MAX_PACING_DELAY {
                    return Err(PacingError::InvalidConfiguration);
                }
                Ok(offset)
            })
            .collect(),
        PacingMode::SeededSynthetic {
            mut seed,
            minimum_delay,
            maximum_delay,
            ..
        } => {
            let minimum = u64::try_from(minimum_delay.as_nanos())
                .map_err(|_| PacingError::InvalidConfiguration)?;
            let maximum = u64::try_from(maximum_delay.as_nanos())
                .map_err(|_| PacingError::InvalidConfiguration)?;
            let width = maximum
                .checked_sub(minimum)
                .and_then(|difference| difference.checked_add(1))
                .ok_or(PacingError::InvalidConfiguration)?;
            let mut cumulative = Duration::ZERO;
            let mut offsets = Vec::with_capacity(count);
            for _ in 0..count {
                seed = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
                let mut value = seed;
                value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
                value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
                value ^= value >> 31;
                let delay = Duration::from_nanos(minimum + value % width);
                cumulative = cumulative
                    .checked_add(delay)
                    .ok_or(PacingError::InvalidConfiguration)?;
                if cumulative > MAX_PACING_DELAY {
                    return Err(PacingError::InvalidConfiguration);
                }
                offsets.push(cumulative);
            }
            Ok(offsets)
        }
    }
}

/// One independently measured replay-service load point.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CalibrationSample {
    /// Concurrent replay requests offered by the calibration driver.
    pub concurrency: usize,
    /// Total requests offered at this point.
    pub offered: u64,
    /// Requests completed without replay errors.
    pub completed: u64,
    /// Maximum admission queue delay observed by the replay service.
    pub max_queue_delay: Duration,
    /// Measured p99 pacing lateness at this point.
    pub p99_lateness: Duration,
    /// Replay-service user plus system CPU time.
    pub service_cpu: Duration,
    /// Monotonic wall time of this calibration point.
    pub wall_time: Duration,
}

/// Predeclared saturation limits for replay-service calibration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CalibrationLimits {
    /// Largest acceptable queue delay.
    pub max_queue_delay: Duration,
    /// Largest acceptable p99 pacing lateness.
    pub max_p99_lateness: Duration,
    /// Maximum CPU/wall ratio in basis points; 10,000 is one saturated core.
    pub max_cpu_wall_basis_points: u32,
}

/// Evidence-aware replay-service headroom verdict.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HeadroomVerdict {
    /// At least one passing point was independently measured above client load.
    Sufficient,
    /// Above-client points were measured, but none passed all declared limits.
    Insufficient,
}

/// Bounded summary retaining both sustainable and first failing load points.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HeadroomAssessment {
    /// Client concurrency the replay service must exceed.
    pub client_concurrency: usize,
    /// Highest measured point satisfying all limits.
    pub highest_sustainable: Option<usize>,
    /// Lowest measured point failing any limit.
    pub first_saturated: Option<usize>,
    /// Whether measured replay headroom exists beyond client concurrency.
    pub verdict: HeadroomVerdict,
}

/// Assess independently collected service samples; this function performs no client sweep.
pub fn assess_replay_headroom(
    client_concurrency: usize,
    samples: &[CalibrationSample],
    limits: CalibrationLimits,
) -> Result<HeadroomAssessment, PacingError> {
    if client_concurrency == 0
        || samples.is_empty()
        || limits.max_cpu_wall_basis_points == 0
        || samples
            .iter()
            .all(|sample| sample.concurrency <= client_concurrency)
    {
        return Err(PacingError::InvalidConfiguration);
    }
    let mut previous = 0;
    let mut highest = None;
    let mut first_saturated = None;
    for sample in samples {
        if sample.concurrency <= previous
            || sample.offered == 0
            || sample.completed > sample.offered
            || sample.wall_time.is_zero()
        {
            return Err(PacingError::InvalidConfiguration);
        }
        previous = sample.concurrency;
        let cpu_basis_points =
            sample.service_cpu.as_nanos().saturating_mul(10_000) / sample.wall_time.as_nanos();
        let passes = sample.completed == sample.offered
            && sample.max_queue_delay <= limits.max_queue_delay
            && sample.p99_lateness <= limits.max_p99_lateness
            && cpu_basis_points <= u128::from(limits.max_cpu_wall_basis_points);
        if passes {
            highest = Some(sample.concurrency);
        } else if first_saturated.is_none() {
            first_saturated = Some(sample.concurrency);
        }
    }
    let verdict = if highest.is_some_and(|point| point > client_concurrency) {
        HeadroomVerdict::Sufficient
    } else {
        HeadroomVerdict::Insufficient
    };
    Ok(HeadroomAssessment {
        client_concurrency,
        highest_sustainable: highest,
        first_saturated,
        verdict,
    })
}
