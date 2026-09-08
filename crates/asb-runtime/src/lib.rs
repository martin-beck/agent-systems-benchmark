// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Bounded Linux subprocess execution and process-tree cancellation.

/// Rootless namespace isolation and dedicated resource leases.
pub mod sandbox;
/// Bounded closed-loop and open-loop experiment scheduling.
pub mod scheduler;

#[cfg(not(target_os = "linux"))]
compile_error!("asb-runtime currently supports Linux process semantics only");

use rustix::io::Errno;
use rustix::process::{Pid, Signal, WaitId, WaitIdOptions, kill_process_group, waitid};
use std::fmt;
use std::io::{self, Read};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Child, Command, Stdio};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Hard ceiling for either retained output stream.
pub const MAX_CAPTURE_BYTES: usize = 64 * 1024 * 1024;
/// Hard ceiling for a process deadline.
pub const MAX_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);
/// Hard ceiling for graceful process-tree termination.
pub const MAX_TERMINATION_GRACE: Duration = Duration::from_secs(60);
/// Hard ceiling for exit polling latency.
pub const MAX_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Validated limits for one child process.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessLimits {
    stdout_bytes: usize,
    stderr_bytes: usize,
    timeout: Duration,
    termination_grace: Duration,
    poll_interval: Duration,
}

/// A rejected process limit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LimitError {
    /// A retained stream limit was zero or above the hard ceiling.
    CaptureBytes,
    /// The deadline was zero or above the hard ceiling.
    Timeout,
    /// The graceful termination interval exceeded the hard ceiling.
    TerminationGrace,
    /// The polling interval was zero or above the hard ceiling.
    PollInterval,
}

impl ProcessLimits {
    /// Validate all memory and time bounds.
    pub fn new(
        stdout_bytes: usize,
        stderr_bytes: usize,
        timeout: Duration,
        termination_grace: Duration,
        poll_interval: Duration,
    ) -> Result<Self, LimitError> {
        if stdout_bytes == 0
            || stderr_bytes == 0
            || stdout_bytes > MAX_CAPTURE_BYTES
            || stderr_bytes > MAX_CAPTURE_BYTES
        {
            return Err(LimitError::CaptureBytes);
        }
        if timeout.is_zero() || timeout > MAX_TIMEOUT {
            return Err(LimitError::Timeout);
        }
        if termination_grace > MAX_TERMINATION_GRACE {
            return Err(LimitError::TerminationGrace);
        }
        if poll_interval.is_zero() || poll_interval > MAX_POLL_INTERVAL {
            return Err(LimitError::PollInterval);
        }
        Ok(Self {
            stdout_bytes,
            stderr_bytes,
            timeout,
            termination_grace,
            poll_interval,
        })
    }

    /// Return the retained stdout ceiling.
    pub fn stdout_bytes(self) -> usize {
        self.stdout_bytes
    }
    /// Return the retained stderr ceiling.
    pub fn stderr_bytes(self) -> usize {
        self.stderr_bytes
    }
    /// Return the monotonic runtime deadline interval.
    pub fn timeout(self) -> Duration {
        self.timeout
    }
    /// Return the TERM-to-KILL grace interval.
    pub fn termination_grace(self) -> Duration {
        self.termination_grace
    }
    /// Return the exit polling interval.
    pub fn poll_interval(self) -> Duration {
        self.poll_interval
    }
}

impl Default for ProcessLimits {
    fn default() -> Self {
        Self {
            stdout_bytes: 1024 * 1024,
            stderr_bytes: 1024 * 1024,
            timeout: Duration::from_secs(5 * 60),
            termination_grace: Duration::from_secs(1),
            poll_interval: Duration::from_millis(5),
        }
    }
}

/// Retained output and its full drained byte count.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapturedStream {
    /// Bytes retained from the beginning of the stream.
    pub bytes: Vec<u8>,
    /// Total bytes drained, saturating at the largest unsigned 64-bit value.
    pub total_bytes: u64,
    /// Whether bytes beyond the retained prefix were discarded.
    pub truncated: bool,
}

/// Why the runtime finished a process.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Termination {
    /// The process leader exited before cancellation or its deadline.
    Exited,
    /// The caller requested cancellation.
    Cancelled,
    /// The monotonic deadline elapsed.
    TimedOut,
}

/// Complete bounded evidence from a reaped process.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessOutput {
    /// Normal exit code, when available.
    pub exit_code: Option<i32>,
    /// Unix terminating signal, when available.
    pub signal: Option<i32>,
    /// Runtime termination classification.
    pub termination: Termination,
    /// Bounded stdout evidence.
    pub stdout: CapturedStream,
    /// Bounded stderr evidence.
    pub stderr: CapturedStream,
    /// Monotonic elapsed duration observed by the runtime.
    pub elapsed: Duration,
}

/// Observable lifecycle of a process handle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessLifecycle {
    /// The leader is running.
    Running,
    /// TERM was sent because the caller cancelled the process.
    Cancelling,
    /// TERM was sent because the monotonic deadline elapsed.
    TimingOut,
    /// The exited leader remains unreaped while its group is cleaned up.
    ExitedPendingReap,
    /// The group was cleaned up and the leader was reaped.
    Terminal,
}

/// A process runtime failure.
#[derive(Debug)]
pub enum ProcessError {
    /// The operating system rejected process creation.
    Spawn(io::Error),
    /// Exit observation or reaping failed.
    Wait(io::Error),
    /// Signalling the owned process group failed.
    Signal(io::Error),
    /// Draining stdout or stderr failed.
    Output(io::Error),
    /// An output-draining thread panicked.
    OutputThreadPanicked,
    /// The process is terminal but prior output collection failed.
    TerminalEvidenceUnavailable,
}

impl fmt::Display for ProcessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn(error) => write!(formatter, "failed to spawn process: {error}"),
            Self::Wait(error) => write!(formatter, "failed to wait for process: {error}"),
            Self::Signal(error) => write!(formatter, "failed to signal process group: {error}"),
            Self::Output(error) => write!(formatter, "failed to drain process output: {error}"),
            Self::OutputThreadPanicked => formatter.write_str("output draining thread panicked"),
            Self::TerminalEvidenceUnavailable => {
                formatter.write_str("terminal process evidence is unavailable")
            }
        }
    }
}

impl std::error::Error for ProcessError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PendingTermination {
    Cancelled,
    TimedOut,
}

/// A running child and the unique process group it leads.
pub struct RunningProcess {
    child: Child,
    pid: Pid,
    limits: ProcessLimits,
    started: Instant,
    deadline: Instant,
    pending: Option<(PendingTermination, Instant)>,
    lifecycle: ProcessLifecycle,
    stdout: Option<JoinHandle<io::Result<CapturedStream>>>,
    stderr: Option<JoinHandle<io::Result<CapturedStream>>>,
    output: Option<ProcessOutput>,
}

impl RunningProcess {
    /// Spawn a command as a new process-group leader with piped output.
    ///
    /// The caller controls stdin and the environment. Native execution is
    /// intended only for trusted programs.
    pub fn spawn(mut command: Command, limits: ProcessLimits) -> Result<Self, ProcessError> {
        command
            .process_group(0)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let started = Instant::now();
        let mut child = command.spawn().map_err(ProcessError::Spawn)?;
        let pid = Pid::from_child(&child);
        let stdout = child.stdout.take().expect("piped stdout must be present");
        let stderr = child.stderr.take().expect("piped stderr must be present");
        let deadline = started
            .checked_add(limits.timeout)
            .expect("validated timeout must fit Instant");
        Ok(Self {
            child,
            pid,
            limits,
            started,
            deadline,
            pending: None,
            lifecycle: ProcessLifecycle::Running,
            stdout: Some(spawn_drain(stdout, limits.stdout_bytes)),
            stderr: Some(spawn_drain(stderr, limits.stderr_bytes)),
            output: None,
        })
    }

    /// Return the process-group leader PID.
    pub fn pid(&self) -> u32 {
        self.pid.as_raw_nonzero().get() as u32
    }

    /// Return the current lifecycle.
    pub fn lifecycle(&self) -> ProcessLifecycle {
        self.lifecycle
    }

    /// Observe whether the process-group leader has exited without reaping it.
    ///
    /// The retained zombie continues to fence its PID until wait is called.
    pub fn leader_has_exited(&self) -> Result<bool, ProcessError> {
        self.leader_exited()
    }

    /// Request cancellation once; repeated calls have no additional effect.
    ///
    /// Exit is checked without reaping before signalling, so an exited leader
    /// continues to fence its PID until wait reaps it.
    pub fn cancel(&mut self) -> Result<(), ProcessError> {
        if self.lifecycle != ProcessLifecycle::Running {
            return Ok(());
        }
        if self.leader_exited()? {
            self.lifecycle = ProcessLifecycle::ExitedPendingReap;
            return Ok(());
        }
        self.send_group(Signal::TERM)?;
        let kill_at = Instant::now()
            .checked_add(self.limits.termination_grace)
            .expect("validated grace must fit Instant");
        self.pending = Some((PendingTermination::Cancelled, kill_at));
        self.lifecycle = ProcessLifecycle::Cancelling;
        Ok(())
    }

    /// Wait for terminal state, enforcing the monotonic deadline and cleanup
    /// for processes that remain in the owned process group.
    ///
    /// Commands must not daemonize, change session/process group, or let an
    /// escaped descendant retain an inherited output pipe. A cgroup-backed
    /// runtime is required for a hard process-tree containment/deadline claim.
    ///
    /// Calling this again returns the same evidence and never signals a stale
    /// process identity.
    pub fn wait(&mut self) -> Result<&ProcessOutput, ProcessError> {
        if self.output.is_none() {
            if self.lifecycle == ProcessLifecycle::Terminal {
                return Err(ProcessError::TerminalEvidenceUnavailable);
            }
            self.drive_to_terminal()?;
        }
        self.output
            .as_ref()
            .ok_or(ProcessError::TerminalEvidenceUnavailable)
    }

    fn drive_to_terminal(&mut self) -> Result<(), ProcessError> {
        loop {
            if self.leader_exited()? {
                let termination = self
                    .pending
                    .map(|(reason, _)| reason.into())
                    .unwrap_or(Termination::Exited);
                self.lifecycle = ProcessLifecycle::ExitedPendingReap;
                return self.finish(termination);
            }
            let now = Instant::now();
            if let Some((reason, kill_at)) = self.pending {
                if now >= kill_at {
                    self.send_group(Signal::KILL)?;
                    return self.finish(reason.into());
                }
            } else if now >= self.deadline {
                self.send_group(Signal::TERM)?;
                let kill_at = now
                    .checked_add(self.limits.termination_grace)
                    .expect("validated grace must fit Instant");
                self.pending = Some((PendingTermination::TimedOut, kill_at));
                self.lifecycle = ProcessLifecycle::TimingOut;
            }
            thread::sleep(self.limits.poll_interval);
        }
    }

    fn leader_exited(&self) -> Result<bool, ProcessError> {
        let options = WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT;
        waitid(WaitId::Pid(self.pid), options)
            .map(|status| status.is_some())
            .map_err(|error| ProcessError::Wait(error.into()))
    }

    fn send_group(&self, signal: Signal) -> Result<(), ProcessError> {
        match kill_process_group(self.pid, signal) {
            Ok(()) | Err(Errno::SRCH) => Ok(()),
            Err(error) => Err(ProcessError::Signal(error.into())),
        }
    }

    fn finish(&mut self, termination: Termination) -> Result<(), ProcessError> {
        // The unreaped leader still owns its PID here, so this cannot target a
        // newly reused process group. KILL closes pipes held by descendants.
        self.send_group(Signal::KILL)?;
        let stdout = join_drain(self.stdout.take());
        let stderr = join_drain(self.stderr.take());
        let status = self.child.wait().map_err(ProcessError::Wait)?;
        self.lifecycle = ProcessLifecycle::Terminal;
        let stdout = stdout?;
        let stderr = stderr?;
        self.output = Some(ProcessOutput {
            exit_code: status.code(),
            signal: status.signal(),
            termination,
            stdout,
            stderr,
            elapsed: self.started.elapsed(),
        });
        Ok(())
    }
}

impl Drop for RunningProcess {
    fn drop(&mut self) {
        if self.lifecycle != ProcessLifecycle::Terminal {
            let _ = self.send_group(Signal::KILL);
            let _ = self.child.wait();
        }
    }
}

impl From<PendingTermination> for Termination {
    fn from(value: PendingTermination) -> Self {
        match value {
            PendingTermination::Cancelled => Self::Cancelled,
            PendingTermination::TimedOut => Self::TimedOut,
        }
    }
}

fn spawn_drain<R>(mut reader: R, limit: usize) -> JoinHandle<io::Result<CapturedStream>>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut bytes = Vec::with_capacity(limit.min(8192));
        let mut total_bytes = 0_u64;
        let mut buffer = [0_u8; 8192];
        loop {
            let count = reader.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            total_bytes = total_bytes.saturating_add(count as u64);
            let remaining = limit.saturating_sub(bytes.len());
            bytes.extend_from_slice(&buffer[..count.min(remaining)]);
        }
        Ok(CapturedStream {
            truncated: total_bytes > bytes.len() as u64,
            bytes,
            total_bytes,
        })
    })
}

fn join_drain(
    handle: Option<JoinHandle<io::Result<CapturedStream>>>,
) -> Result<CapturedStream, ProcessError> {
    handle
        .expect("drain handle must be present before terminal state")
        .join()
        .map_err(|_| ProcessError::OutputThreadPanicked)?
        .map_err(ProcessError::Output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_zero_and_excessive_limits() {
        let valid = ProcessLimits::default();
        assert_eq!(
            ProcessLimits::new(
                0,
                valid.stderr_bytes,
                valid.timeout,
                valid.termination_grace,
                valid.poll_interval
            ),
            Err(LimitError::CaptureBytes)
        );
        assert_eq!(
            ProcessLimits::new(
                valid.stdout_bytes,
                MAX_CAPTURE_BYTES + 1,
                valid.timeout,
                valid.termination_grace,
                valid.poll_interval
            ),
            Err(LimitError::CaptureBytes)
        );
        assert_eq!(
            ProcessLimits::new(
                1,
                1,
                Duration::ZERO,
                Duration::ZERO,
                Duration::from_millis(1)
            ),
            Err(LimitError::Timeout)
        );
        assert_eq!(
            ProcessLimits::new(
                1,
                1,
                Duration::from_secs(1),
                MAX_TERMINATION_GRACE + Duration::from_nanos(1),
                Duration::from_millis(1)
            ),
            Err(LimitError::TerminationGrace)
        );
        assert_eq!(
            ProcessLimits::new(1, 1, Duration::from_secs(1), Duration::ZERO, Duration::ZERO),
            Err(LimitError::PollInterval)
        );
    }

    #[test]
    fn default_limits_are_valid_and_bounded() {
        let limits = ProcessLimits::default();
        assert!(limits.stdout_bytes() <= MAX_CAPTURE_BYTES);
        assert!(limits.stderr_bytes() <= MAX_CAPTURE_BYTES);
        assert!(limits.timeout() <= MAX_TIMEOUT);
        assert!(limits.termination_grace() <= MAX_TERMINATION_GRACE);
        assert!(limits.poll_interval() <= MAX_POLL_INTERVAL);
    }

    #[test]
    fn runtime_errors_have_specific_messages() {
        assert!(
            ProcessError::Spawn(io::Error::other("spawn"))
                .to_string()
                .contains("spawn")
        );
        assert!(
            ProcessError::Wait(io::Error::other("wait"))
                .to_string()
                .contains("wait")
        );
        assert!(
            ProcessError::Signal(io::Error::other("signal"))
                .to_string()
                .contains("signal")
        );
        assert!(
            ProcessError::Output(io::Error::other("output"))
                .to_string()
                .contains("output")
        );
        assert_eq!(
            ProcessError::OutputThreadPanicked.to_string(),
            "output draining thread panicked"
        );
        assert_eq!(
            ProcessError::TerminalEvidenceUnavailable.to_string(),
            "terminal process evidence is unavailable"
        );
    }

    #[test]
    fn drain_failures_are_typed() {
        let io_failure = thread::spawn(|| Err(io::Error::other("read failed")));
        assert!(matches!(
            join_drain(Some(io_failure)),
            Err(ProcessError::Output(_))
        ));
        let panic = thread::spawn(|| -> io::Result<CapturedStream> {
            panic!("controlled drain panic");
        });
        assert!(matches!(
            join_drain(Some(panic)),
            Err(ProcessError::OutputThreadPanicked)
        ));
    }

    #[test]
    fn terminal_handle_without_evidence_never_signals_again() {
        let command = Command::new("/bin/true");
        let mut process = RunningProcess::spawn(command, ProcessLimits::default()).unwrap();
        process.lifecycle = ProcessLifecycle::Terminal;
        assert!(matches!(
            process.wait(),
            Err(ProcessError::TerminalEvidenceUnavailable)
        ));
        process.lifecycle = ProcessLifecycle::Running;
        process.wait().unwrap();
    }
}
