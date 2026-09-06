// SPDX-License-Identifier: MIT
//! Real Linux process-boundary tests for the native runtime.

use asb_runtime::{
    MAX_CAPTURE_BYTES, ProcessError, ProcessLifecycle, ProcessLimits, RunningProcess, Termination,
};
use rustix::process::{Pid, WaitId, WaitIdOptions, waitid};
use std::fs;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

fn limits(timeout: Duration) -> ProcessLimits {
    ProcessLimits::new(
        4096,
        4096,
        timeout,
        Duration::from_millis(50),
        Duration::from_millis(2),
    )
    .expect("test limits are valid")
}

fn shell(script: &str, timeout: Duration) -> RunningProcess {
    let mut command = Command::new("/bin/sh");
    command.arg("-c").arg(script);
    RunningProcess::spawn(command, limits(timeout)).expect("shell must start")
}

fn process_is_live(pid: u32) -> bool {
    fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|stat| stat.rsplit_once(") ").map(|(_, fields)| fields.to_owned()))
        .and_then(|fields| fields.chars().next())
        .is_some_and(|state| state != 'Z')
}

#[test]
fn startup_failure_is_explicit() {
    let command = Command::new("/definitely/not/an/asb-executable");
    assert!(matches!(
        RunningProcess::spawn(command, ProcessLimits::default()),
        Err(ProcessError::Spawn(_))
    ));
}

#[test]
fn blocked_stdout_and_stderr_are_drained_with_bounded_retention() {
    let script = "i=0; while [ $i -lt 20000 ]; do printf 0123456789; printf abcdefghij >&2; i=$((i+1)); done";
    let mut process = shell(script, Duration::from_secs(10));
    let output = process.wait().expect("large-output child must finish");
    assert_eq!(output.termination, Termination::Exited);
    assert_eq!(output.exit_code, Some(0));
    assert_eq!(output.stdout.bytes.len(), 4096);
    assert_eq!(output.stderr.bytes.len(), 4096);
    assert_eq!(output.stdout.total_bytes, 200_000);
    assert_eq!(output.stderr.total_bytes, 200_000);
    assert!(output.stdout.truncated);
    assert!(output.stderr.truncated);
    assert!(output.stdout.bytes.len() <= MAX_CAPTURE_BYTES);
    assert!(output.stderr.bytes.len() <= MAX_CAPTURE_BYTES);
}

#[test]
fn monotonic_timeout_terminates_promptly() {
    let started = Instant::now();
    let mut process = shell("trap '' TERM; sleep 60", Duration::from_millis(30));
    let output = process.wait().expect("timed-out child must be reaped");
    assert_eq!(output.termination, Termination::TimedOut);
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(process.lifecycle(), ProcessLifecycle::Terminal);
}

#[test]
fn cancellation_is_idempotent() {
    let mut process = shell("trap '' TERM; sleep 60", Duration::from_secs(10));
    process.cancel().expect("first cancellation must succeed");
    assert_eq!(process.lifecycle(), ProcessLifecycle::Cancelling);
    process
        .cancel()
        .expect("second cancellation must be a no-op");
    let first = process
        .wait()
        .expect("cancelled child must be reaped")
        .clone();
    assert_eq!(first.termination, Termination::Cancelled);
    process
        .cancel()
        .expect("terminal cancellation must be a no-op");
    assert_eq!(process.wait().expect("terminal wait is stable"), &first);
}

#[test]
fn timeout_cleans_up_grandchild_process() {
    let script = "trap 'trap - TERM; kill -TERM $kid 2>/dev/null; wait $kid 2>/dev/null; exit 143' TERM; sleep 60 & kid=$!; echo $kid; wait $kid";
    let mut process = shell(script, Duration::from_millis(50));
    let output = process.wait().expect("process tree must be reaped");
    assert_eq!(output.termination, Termination::TimedOut);
    let pid: u32 = String::from_utf8(output.stdout.bytes.clone())
        .expect("PID output is UTF-8")
        .trim()
        .parse()
        .expect("PID output is numeric");
    let deadline = Instant::now() + Duration::from_secs(2);
    while process_is_live(pid) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    assert!(!process_is_live(pid), "grandchild {pid} remained live");
}

#[test]
fn waitid_nowait_fences_pid_reuse_until_runtime_reaps() {
    let mut process = shell("exit 0", Duration::from_secs(2));
    let pid = process.pid();
    let rustix_pid = Pid::from_raw(pid as i32).expect("child PID is positive");
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let status = waitid(
            WaitId::Pid(rustix_pid),
            WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
        )
        .expect("WNOWAIT observation must succeed");
        if status.is_some() {
            break;
        }
        assert!(Instant::now() < deadline, "child did not exit");
        thread::sleep(Duration::from_millis(2));
    }
    assert!(
        fs::metadata(format!("/proc/{pid}")).is_ok(),
        "unreaped leader must retain its PID identity"
    );
    process
        .cancel()
        .expect("cancellation must recognize exited leader without signalling");
    assert_eq!(process.lifecycle(), ProcessLifecycle::ExitedPendingReap);
    let output = process.wait().expect("runtime must reap fenced leader");
    assert_eq!(output.termination, Termination::Exited);
    assert_eq!(output.exit_code, Some(0));
}

#[test]
fn successful_leader_cannot_leave_background_child_live() {
    let mut process = shell("sleep 60 & echo $!", Duration::from_secs(2));
    let output = process.wait().expect("successful leader must finish");
    assert_eq!(output.termination, Termination::Exited);
    let pid: u32 = String::from_utf8(output.stdout.bytes.clone())
        .expect("PID output is UTF-8")
        .trim()
        .parse()
        .expect("PID output is numeric");
    let deadline = Instant::now() + Duration::from_secs(2);
    while process_is_live(pid) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    assert!(
        !process_is_live(pid),
        "background child {pid} remained live"
    );
}

#[test]
fn dropping_handle_kills_owned_process_group() {
    let process = shell("trap '' TERM; sleep 60", Duration::from_secs(10));
    let pid = process.pid();
    assert!(process_is_live(pid));
    drop(process);
    let deadline = Instant::now() + Duration::from_secs(2);
    while process_is_live(pid) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    assert!(!process_is_live(pid), "dropped leader {pid} remained live");
}
