// SPDX-License-Identifier: MIT
//! Deterministic pacing, cancellation, backpressure, and calibration tests.

use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use asb_replay::{
    CalibrationLimits, CalibrationSample, CancellationToken, HeadroomVerdict, PacingClassification,
    PacingConfig, PacingError, PacingMode, assess_replay_headroom, write_paced_segments,
};

fn segments() -> Vec<Vec<u8>> {
    vec![b"one".to_vec(), b"two".to_vec(), b"three".to_vec()]
}

fn offsets() -> Vec<Duration> {
    vec![
        Duration::from_millis(1),
        Duration::from_millis(3),
        Duration::from_millis(5),
    ]
}

fn config(mode: PacingMode) -> PacingConfig {
    PacingConfig {
        mode,
        max_segment_write: Duration::from_secs(1),
        max_lateness: Duration::from_millis(100),
        cancellation_poll: Duration::from_millis(1),
    }
}

#[test]
fn immediate_fixed_original_and_seeded_modes_report_requested_and_actual_timing() {
    let mut immediate = Vec::new();
    let report = write_paced_segments(
        &mut immediate,
        &segments(),
        &offsets(),
        config(PacingMode::Immediate),
        &CancellationToken::default(),
    )
    .unwrap();
    assert_eq!(immediate, b"onetwothree");
    assert_eq!(
        report.classification,
        PacingClassification::RecordedResponse
    );
    assert!(
        report
            .segments
            .iter()
            .all(|item| item.desired_offset.is_zero())
    );

    let mut fixed = Vec::new();
    let report = write_paced_segments(
        &mut fixed,
        &segments(),
        &offsets(),
        config(PacingMode::Fixed {
            time_to_first_segment: Duration::from_millis(1),
            inter_segment: Duration::from_millis(2),
        }),
        &CancellationToken::default(),
    )
    .unwrap();
    assert_eq!(
        report
            .segments
            .iter()
            .map(|item| item.desired_offset)
            .collect::<Vec<_>>(),
        offsets()
    );
    assert!(report.segments.iter().all(|item| {
        item.write_started_offset >= item.desired_offset
            && item.write_completed_offset >= item.write_started_offset
            && item.lateness == item.write_started_offset - item.desired_offset
    }));

    let mut original = Vec::new();
    let report = write_paced_segments(
        &mut original,
        &segments(),
        &offsets(),
        config(PacingMode::Original),
        &CancellationToken::default(),
    )
    .unwrap();
    assert_eq!(report.segments[2].desired_offset, Duration::from_millis(5));

    let seeded = |seed| {
        let mut output = Vec::new();
        write_paced_segments(
            &mut output,
            &segments(),
            &offsets(),
            config(PacingMode::SeededSynthetic {
                seed,
                minimum_delay: Duration::from_micros(10),
                maximum_delay: Duration::from_micros(30),
                fail_before_segment: None,
            }),
            &CancellationToken::default(),
        )
        .unwrap()
    };
    let first = seeded(7);
    let second = seeded(7);
    let different = seeded(8);
    let desired = |report: &asb_replay::PacingReport| {
        report
            .segments
            .iter()
            .map(|item| item.desired_offset)
            .collect::<Vec<_>>()
    };
    assert_eq!(desired(&first), desired(&second));
    assert_ne!(desired(&first), desired(&different));
    assert_eq!(
        first.classification,
        PacingClassification::SyntheticScenario
    );
}

#[test]
fn cancellation_interrupts_a_monotonic_wait_within_the_poll_boundary() {
    let cancellation = CancellationToken::default();
    let child = cancellation.clone();
    let started = std::time::Instant::now();
    let handle = thread::spawn(move || {
        let mut output = Vec::new();
        write_paced_segments(
            &mut output,
            &[b"never".to_vec()],
            &[Duration::from_secs(2)],
            config(PacingMode::Original),
            &child,
        )
    });
    thread::sleep(Duration::from_millis(10));
    cancellation.cancel();
    assert!(matches!(
        handle.join().unwrap(),
        Err(PacingError::Cancelled {
            completed_segments: 0
        })
    ));
    assert!(started.elapsed() < Duration::from_millis(200));
}

#[derive(Clone)]
struct SlowWriter {
    delay: Duration,
    bytes: Arc<Mutex<Vec<u8>>>,
}

impl Write for SlowWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        thread::sleep(self.delay);
        self.bytes.lock().unwrap().extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn slow_or_failed_writers_fail_closed_without_claiming_complete_delivery() {
    let mut slow = SlowWriter {
        delay: Duration::from_millis(5),
        bytes: Arc::new(Mutex::new(Vec::new())),
    };
    let result = write_paced_segments(
        &mut slow,
        &[b"segment".to_vec()],
        &[Duration::ZERO],
        PacingConfig {
            max_segment_write: Duration::from_millis(1),
            ..config(PacingMode::Immediate)
        },
        &CancellationToken::default(),
    );
    assert!(matches!(
        result,
        Err(PacingError::Backpressure {
            completed_segments: 1
        })
    ));

    struct FailedWriter;
    impl Write for FailedWriter {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "synthetic"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    assert!(matches!(
        write_paced_segments(
            &mut FailedWriter,
            &[b"segment".to_vec()],
            &[Duration::ZERO],
            config(PacingMode::Immediate),
            &CancellationToken::default()
        ),
        Err(PacingError::Io(_))
    ));
}

#[test]
fn completion_accounting_covers_post_write_cancellation_and_error_kinds() {
    struct CancellingWriter(CancellationToken);
    impl Write for CancellingWriter {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.0.cancel();
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    let cancellation = CancellationToken::default();
    let error = write_paced_segments(
        &mut CancellingWriter(cancellation.clone()),
        &[b"first".to_vec(), b"second".to_vec()],
        &[Duration::ZERO; 2],
        PacingConfig::default(),
        &cancellation,
    )
    .unwrap_err();
    assert_eq!(error.completed_segments(), 1);
    assert_eq!(PacingError::InvalidConfiguration.completed_segments(), 0);
    assert_eq!(
        PacingError::Io(io::Error::other("synthetic")).completed_segments(),
        0
    );
    assert_eq!(
        PacingError::SyntheticFailure {
            completed_segments: 2
        }
        .completed_segments(),
        2
    );
    assert_eq!(
        PacingError::Backpressure {
            completed_segments: 3
        }
        .completed_segments(),
        3
    );
}

#[test]
fn invalid_offsets_bounds_and_seeded_failures_are_rejected_or_classified() {
    let cases = [
        (
            vec![Duration::ZERO; 3],
            PacingConfig {
                cancellation_poll: Duration::ZERO,
                ..config(PacingMode::Immediate)
            },
        ),
        (
            vec![
                Duration::from_millis(2),
                Duration::from_millis(1),
                Duration::from_millis(3),
            ],
            config(PacingMode::Original),
        ),
        (
            vec![Duration::ZERO; 3],
            config(PacingMode::SeededSynthetic {
                seed: 1,
                minimum_delay: Duration::from_millis(2),
                maximum_delay: Duration::from_millis(1),
                fail_before_segment: None,
            }),
        ),
        (
            vec![Duration::from_secs(301); 3],
            config(PacingMode::Original),
        ),
        (
            vec![Duration::ZERO; 3],
            config(PacingMode::Fixed {
                time_to_first_segment: Duration::from_secs(200),
                inter_segment: Duration::from_secs(200),
            }),
        ),
        (
            vec![Duration::ZERO; 3],
            config(PacingMode::SeededSynthetic {
                seed: 1,
                minimum_delay: Duration::from_secs(200),
                maximum_delay: Duration::from_secs(200),
                fail_before_segment: None,
            }),
        ),
        (
            vec![Duration::ZERO; 3],
            config(PacingMode::Fixed {
                time_to_first_segment: Duration::from_secs(301),
                inter_segment: Duration::ZERO,
            }),
        ),
        (
            vec![Duration::ZERO; 3],
            config(PacingMode::SeededSynthetic {
                seed: 1,
                minimum_delay: Duration::ZERO,
                maximum_delay: Duration::from_secs(301),
                fail_before_segment: None,
            }),
        ),
        (
            vec![Duration::ZERO; 3],
            config(PacingMode::SeededSynthetic {
                seed: 1,
                minimum_delay: Duration::ZERO,
                maximum_delay: Duration::ZERO,
                fail_before_segment: Some(3),
            }),
        ),
    ];
    for (recorded, config) in cases {
        assert!(matches!(
            write_paced_segments(
                &mut Vec::new(),
                &segments(),
                &recorded,
                config,
                &CancellationToken::default()
            ),
            Err(PacingError::InvalidConfiguration)
        ));
    }

    let excessive_count = usize::try_from(asb_replay::MAX_EVENTS).unwrap() + 1;
    assert!(matches!(
        write_paced_segments(
            &mut Vec::new(),
            &vec![Vec::new(); excessive_count],
            &vec![Duration::ZERO; excessive_count],
            config(PacingMode::Immediate),
            &CancellationToken::default(),
        ),
        Err(PacingError::InvalidConfiguration)
    ));

    let result = write_paced_segments(
        &mut Vec::new(),
        &segments(),
        &offsets(),
        config(PacingMode::SeededSynthetic {
            seed: 1,
            minimum_delay: Duration::ZERO,
            maximum_delay: Duration::ZERO,
            fail_before_segment: Some(1),
        }),
        &CancellationToken::default(),
    );
    assert!(matches!(
        result,
        Err(PacingError::SyntheticFailure {
            completed_segments: 1
        })
    ));
}

fn sample(concurrency: usize, failures: u64, queue_ms: u64, lateness_ms: u64) -> CalibrationSample {
    CalibrationSample {
        concurrency,
        offered: 100,
        completed: 100 - failures,
        max_queue_delay: Duration::from_millis(queue_ms),
        p99_lateness: Duration::from_millis(lateness_ms),
        service_cpu: Duration::from_millis(40),
        wall_time: Duration::from_millis(100),
    }
}

#[test]
fn calibration_requires_independent_above_client_load_and_retains_saturation() {
    let limits = CalibrationLimits {
        max_queue_delay: Duration::from_millis(5),
        max_p99_lateness: Duration::from_millis(2),
        max_cpu_wall_basis_points: 8_000,
    };
    assert!(matches!(
        assess_replay_headroom(8, &[sample(1, 0, 0, 0), sample(8, 0, 0, 0)], limits),
        Err(PacingError::InvalidConfiguration)
    ));
    let assessment = assess_replay_headroom(
        8,
        &[
            sample(4, 0, 1, 1),
            sample(8, 0, 2, 1),
            sample(16, 0, 3, 2),
            sample(32, 1, 20, 8),
        ],
        limits,
    )
    .unwrap();
    assert_eq!(assessment.verdict, HeadroomVerdict::Sufficient);
    assert_eq!(assessment.highest_sustainable, Some(16));
    assert_eq!(assessment.first_saturated, Some(32));

    let insufficient =
        assess_replay_headroom(8, &[sample(8, 0, 0, 0), sample(16, 1, 0, 0)], limits).unwrap();
    assert_eq!(insufficient.verdict, HeadroomVerdict::Insufficient);
}

#[test]
fn malformed_calibration_samples_fail_closed() {
    let limits = CalibrationLimits {
        max_queue_delay: Duration::from_secs(1),
        max_p99_lateness: Duration::from_secs(1),
        max_cpu_wall_basis_points: 10_000,
    };
    let mut invalid = sample(16, 0, 0, 0);
    invalid.completed = invalid.offered + 1;
    for samples in [
        vec![sample(16, 0, 0, 0), sample(8, 0, 0, 0)],
        vec![invalid],
        vec![CalibrationSample {
            wall_time: Duration::ZERO,
            ..sample(16, 0, 0, 0)
        }],
    ] {
        assert!(matches!(
            assess_replay_headroom(8, &samples, limits),
            Err(PacingError::InvalidConfiguration)
        ));
    }
}
