// SPDX-License-Identifier: MIT
//! Optional kernel diagnostics backed by digest-pinned native tools.

use asb_runtime::{ProcessError, ProcessLimits, ProcessOutput, RunningProcess, Termination};
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

const MAX_TOOL_BYTES: u64 = 64 * 1024 * 1024;
const MAX_OUTPUT_BYTES: usize = 1024 * 1024;
const MAX_VERSION_BYTES: usize = 4096;
const MAX_CLEANUP_ENTRIES: usize = 32;
static RUN_NONCE: AtomicU64 = AtomicU64::new(0);

/// Why a diagnostic could not produce trusted evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnavailableReason {
    /// A required executable or kernel surface was absent.
    Absent,
    /// The kernel or operating system denied the operation.
    PermissionDenied,
    /// Tool bytes or exact version did not match the reviewed pin.
    ToolMismatch,
    /// The bounded process deadline elapsed.
    TimedOut,
    /// The tool rejected the requested probe for another reason.
    ProbeRejected,
    /// Output was truncated, malformed, or outside the accepted range.
    MalformedEvidence,
    /// A private staging directory could not be removed.
    CleanupUncertain,
}

impl fmt::Display for UnavailableReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Absent => "diagnostic prerequisite is absent",
            Self::PermissionDenied => "diagnostic permission was denied",
            Self::ToolMismatch => "diagnostic tool pin did not match",
            Self::TimedOut => "diagnostic timed out",
            Self::ProbeRejected => "diagnostic probe was rejected",
            Self::MalformedEvidence => "diagnostic evidence was malformed",
            Self::CleanupUncertain => "diagnostic cleanup could not be proven",
        })
    }
}

impl std::error::Error for UnavailableReason {}

/// Constructor-controlled optional diagnostic evidence.
///
/// ```compile_fail
/// let forged = asb_metrics::kernel::ProbeResult {
///     value: Some(1),
///     unavailable: None,
/// };
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProbeResult {
    value: Option<u64>,
    unavailable: Option<UnavailableReason>,
}

impl ProbeResult {
    /// The validated counter or capability-line count, when available.
    #[must_use]
    pub const fn value(self) -> Option<u64> {
        self.value
    }

    /// The explicit reason, when no value was accepted.
    #[must_use]
    pub const fn unavailable(self) -> Option<UnavailableReason> {
        self.unavailable
    }

    const fn available(value: u64) -> Self {
        Self {
            value: Some(value),
            unavailable: None,
        }
    }

    const fn unavailable_reason(reason: UnavailableReason) -> Self {
        Self {
            value: None,
            unavailable: Some(reason),
        }
    }
}

/// A reviewed executable identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PinnedTool {
    path: PathBuf,
    sha256: String,
    version_argument: String,
    exact_version: String,
}

impl PinnedTool {
    /// Construct a strict executable pin with a lowercase hexadecimal digest.
    pub fn new(
        path: PathBuf,
        sha256: String,
        version_argument: String,
        exact_version: String,
    ) -> Result<Self, UnavailableReason> {
        if !path.is_absolute()
            || sha256.len() != 64
            || !sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            || version_argument.is_empty()
            || exact_version.is_empty()
            || exact_version.len() > MAX_VERSION_BYTES
        {
            return Err(UnavailableReason::ToolMismatch);
        }
        Ok(Self {
            path,
            sha256,
            version_argument,
            exact_version,
        })
    }
}

/// Bounded optional perf/eBPF probes using private immutable launch copies.
pub struct KernelDiagnostics {
    scratch_root: PathBuf,
    timeout: Duration,
}

impl KernelDiagnostics {
    /// Validate an existing private scratch directory and deadline.
    pub fn new(scratch_root: PathBuf, timeout: Duration) -> Result<Self, UnavailableReason> {
        let metadata = fs::symlink_metadata(&scratch_root).map_err(map_io)?;
        if !scratch_root.is_absolute()
            || !metadata.file_type().is_dir()
            || metadata.file_type().is_symlink()
            || metadata.permissions().mode() & 0o077 != 0
            || timeout.is_zero()
            || timeout > Duration::from_secs(60)
        {
            return Err(UnavailableReason::PermissionDenied);
        }
        Ok(Self {
            scratch_root,
            timeout,
        })
    }

    /// Invoke perf for a short system-wide task-clock sample.
    ///
    /// This claims only the value returned by this invocation, not general PMU
    /// availability or workload attribution.
    #[must_use]
    pub fn perf_task_clock(&self, tool: &PinnedTool, milliseconds: u16) -> ProbeResult {
        if milliseconds == 0 || milliseconds > 10_000 {
            return ProbeResult::unavailable_reason(UnavailableReason::ProbeRejected);
        }
        self.run(
            tool,
            &[
                "stat".to_owned(),
                "--all-cpus".to_owned(),
                "--field-separator=;".to_owned(),
                "--log-fd=1".to_owned(),
                "--event=task-clock".to_owned(),
                format!("--timeout={milliseconds}"),
            ],
            parse_task_clock,
        )
    }

    /// Invoke bpftool's kernel feature probe.
    ///
    /// A positive count is not an attach-support claim. Attach and teardown
    /// qualification require a separate native oracle.
    #[must_use]
    pub fn ebpf_feature_count(&self, tool: &PinnedTool) -> ProbeResult {
        self.run(
            tool,
            &[
                "feature".to_owned(),
                "probe".to_owned(),
                "kernel".to_owned(),
            ],
            parse_ebpf_features,
        )
    }

    fn run(
        &self,
        tool: &PinnedTool,
        arguments: &[String],
        parse: fn(&[u8]) -> Option<u64>,
    ) -> ProbeResult {
        let mut staged = match StagedTool::new(&self.scratch_root, tool, self.timeout) {
            Ok(staged) => staged,
            Err(reason) => return ProbeResult::unavailable_reason(reason),
        };
        let result = execute(
            &staged.executable,
            &staged.directory,
            arguments,
            self.timeout,
        )
        .map_or_else(ProbeResult::unavailable_reason, |output| {
            if output.termination == Termination::TimedOut {
                ProbeResult::unavailable_reason(UnavailableReason::TimedOut)
            } else if output.stdout.truncated || output.stderr.truncated {
                ProbeResult::unavailable_reason(UnavailableReason::MalformedEvidence)
            } else if output.exit_code != Some(0) {
                ProbeResult::unavailable_reason(classify_failure(
                    &output.stdout.bytes,
                    &output.stderr.bytes,
                ))
            } else {
                parse(&output.stdout.bytes).map_or(
                    ProbeResult::unavailable_reason(UnavailableReason::MalformedEvidence),
                    ProbeResult::available,
                )
            }
        });
        if staged.cleanup() {
            result
        } else {
            ProbeResult::unavailable_reason(UnavailableReason::CleanupUncertain)
        }
    }
}

struct StagedTool {
    directory: PathBuf,
    executable: PathBuf,
    device: u64,
    inode: u64,
}

impl StagedTool {
    fn new(root: &Path, pin: &PinnedTool, timeout: Duration) -> Result<Self, UnavailableReason> {
        let mut source = File::open(&pin.path).map_err(map_io)?;
        let metadata = source.metadata().map_err(map_io)?;
        if !metadata.is_file() || metadata.len() > MAX_TOOL_BYTES {
            return Err(UnavailableReason::ToolMismatch);
        }
        let capacity =
            usize::try_from(metadata.len()).map_err(|_| UnavailableReason::ToolMismatch)?;
        let mut bytes = Vec::with_capacity(capacity);
        Read::by_ref(&mut source)
            .take(MAX_TOOL_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(map_io)?;
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) != metadata.len()
            || format!("{:x}", Sha256::digest(&bytes)) != pin.sha256
        {
            return Err(UnavailableReason::ToolMismatch);
        }
        let directory = unique_directory(root);
        fs::create_dir(&directory).map_err(map_io)?;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).map_err(map_io)?;
        let directory_metadata = fs::symlink_metadata(&directory).map_err(map_io)?;
        let executable = directory.join("tool");
        let mut target = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o500)
            .open(&executable)
            .map_err(map_io)?;
        target.write_all(&bytes).map_err(map_io)?;
        target.sync_all().map_err(map_io)?;
        drop(target);
        let staged = Self {
            directory,
            executable,
            device: directory_metadata.dev(),
            inode: directory_metadata.ino(),
        };
        if staged.version(pin, timeout)? {
            Ok(staged)
        } else {
            Err(UnavailableReason::ToolMismatch)
        }
    }

    fn version(&self, pin: &PinnedTool, timeout: Duration) -> Result<bool, UnavailableReason> {
        let output = execute(
            &self.executable,
            &self.directory,
            std::slice::from_ref(&pin.version_argument),
            timeout,
        )?;
        if output.stdout.bytes.len() > MAX_VERSION_BYTES
            || output.stderr.bytes.len() > MAX_VERSION_BYTES
        {
            return Ok(false);
        }
        let observed = if output.stdout.bytes.is_empty() {
            &output.stderr.bytes
        } else {
            &output.stdout.bytes
        };
        let observed = observed.strip_suffix(b"\n").unwrap_or(observed);
        let observed = observed.strip_suffix(b"\r").unwrap_or(observed);
        Ok(output.termination == Termination::Exited
            && output.exit_code == Some(0)
            && !output.stdout.truncated
            && !output.stderr.truncated
            && observed == pin.exact_version.as_bytes())
    }

    fn cleanup(&mut self) -> bool {
        let metadata = match fs::symlink_metadata(&self.directory) {
            Ok(metadata) => metadata,
            Err(error) => return error.kind() == io::ErrorKind::NotFound,
        };
        if !metadata.is_dir() || metadata.dev() != self.device || metadata.ino() != self.inode {
            return false;
        }
        let mut paths = Vec::new();
        let entries = match fs::read_dir(&self.directory) {
            Ok(entries) => entries,
            Err(_) => return false,
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => return false,
            };
            if paths.len() == MAX_CLEANUP_ENTRIES {
                return false;
            }
            let metadata = match entry.file_type() {
                Ok(metadata) => metadata,
                Err(_) => return false,
            };
            if metadata.is_dir() {
                return false;
            }
            paths.push(entry.path());
        }
        for path in paths {
            if fs::remove_file(path).is_err() {
                return false;
            }
        }
        let metadata = match fs::symlink_metadata(&self.directory) {
            Ok(metadata) => metadata,
            Err(_) => return false,
        };
        metadata.is_dir()
            && metadata.dev() == self.device
            && metadata.ino() == self.inode
            && fs::remove_dir(&self.directory).is_ok()
    }
}

fn execute(
    executable: &Path,
    directory: &Path,
    arguments: &[String],
    timeout: Duration,
) -> Result<ProcessOutput, UnavailableReason> {
    let limits = ProcessLimits::new(
        MAX_OUTPUT_BYTES,
        MAX_OUTPUT_BYTES,
        timeout,
        Duration::from_millis(250),
        Duration::from_millis(5),
    )
    .map_err(|_| UnavailableReason::ProbeRejected)?;
    let mut command = Command::new(executable);
    command
        .args(arguments)
        .env_clear()
        .env("LC_ALL", "C")
        .current_dir(directory)
        .stdin(Stdio::null());
    let mut process = RunningProcess::spawn(command, limits).map_err(map_process)?;
    process
        .wait()
        .cloned()
        .map_err(|_| UnavailableReason::ProbeRejected)
}

impl Drop for StagedTool {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

fn unique_directory(root: &Path) -> PathBuf {
    let nonce = RUN_NONCE.fetch_add(1, Ordering::Relaxed);
    let epoch = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    root.join(format!("asb-kernel-{}-{epoch}-{nonce}", std::process::id()))
}

fn parse_task_clock(bytes: &[u8]) -> Option<u64> {
    let text = std::str::from_utf8(bytes).ok()?;
    text.lines().find_map(|line| {
        let mut fields = line.split(';');
        let value = fields
            .next()?
            .trim()
            .replace(',', ".")
            .parse::<f64>()
            .ok()?;
        let unit = fields.next()?.trim();
        let event = fields.next()?.trim();
        (unit == "msec" && event == "task-clock" && value.is_finite() && value >= 0.0)
            .then(|| (value * 1_000_000.0).round() as u64)
    })
}

fn parse_ebpf_features(bytes: &[u8]) -> Option<u64> {
    let text = std::str::from_utf8(bytes).ok()?;
    let count = text
        .lines()
        .filter(|line| line.contains("CONFIG_BPF") || line.contains("BTF"))
        .count();
    (count > 0).then(|| u64::try_from(count).unwrap_or(u64::MAX))
}

fn classify_failure(stdout: &[u8], stderr: &[u8]) -> UnavailableReason {
    let stdout = String::from_utf8_lossy(stdout).to_ascii_lowercase();
    let stderr = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    if [stdout.as_str(), stderr.as_str()].iter().any(|output| {
        output.contains("permission")
            || output.contains("operation not permitted")
            || output.contains("access denied")
            || output.contains("access to performance monitoring")
            || output.contains("missing cap_")
            || output.contains("required for full feature probing")
    }) {
        UnavailableReason::PermissionDenied
    } else {
        UnavailableReason::ProbeRejected
    }
}

fn map_io(error: io::Error) -> UnavailableReason {
    match error.kind() {
        io::ErrorKind::NotFound => UnavailableReason::Absent,
        io::ErrorKind::PermissionDenied => UnavailableReason::PermissionDenied,
        _ => UnavailableReason::ProbeRejected,
    }
}

fn map_process(error: ProcessError) -> UnavailableReason {
    match error {
        ProcessError::Spawn(error) => map_io(error),
        _ => UnavailableReason::ProbeRejected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestRoot(PathBuf);

    impl TestRoot {
        fn new() -> Self {
            let executable = std::env::current_exe().unwrap();
            let path = unique_directory(executable.parent().unwrap());
            fs::create_dir(&path).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            Self(path)
        }

        fn harness_tool(&self) -> PinnedTool {
            let path = std::env::current_exe().unwrap();
            let bytes = fs::read(&path).unwrap();
            let output = Command::new(&path).arg("--list").output().unwrap();
            let version = if output.stdout.is_empty() {
                output.stderr
            } else {
                output.stdout
            };
            let version = version.strip_suffix(b"\n").unwrap_or(&version);
            let version = version.strip_suffix(b"\r").unwrap_or(version);
            PinnedTool::new(
                path,
                format!("{:x}", Sha256::digest(bytes)),
                "--list".into(),
                String::from_utf8(version.to_vec()).unwrap(),
            )
            .unwrap()
        }

        fn assert_clean(&self) {
            assert_eq!(fs::read_dir(&self.0).unwrap().count(), 0);
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn parses_outputs() {
        assert_eq!(
            parse_task_clock(b"12.5;msec;task-clock;1;100.0\n"),
            Some(12_500_000)
        );
        assert_eq!(parse_task_clock(b"<not counted>;msec;task-clock\n"), None);
        assert_eq!(
            parse_ebpf_features(b"CONFIG_BPF is set to y\nBTF is available\n"),
            Some(2)
        );
        assert_eq!(parse_ebpf_features(b"unrelated\n"), None);
    }

    #[test]
    fn failures_are_content_free_classifications() {
        assert_eq!(
            classify_failure(b"", b"Operation not permitted: /private/name"),
            UnavailableReason::PermissionDenied
        );
        assert_eq!(
            classify_failure(b"unexpected private payload", b""),
            UnavailableReason::ProbeRejected
        );
        assert_eq!(
            UnavailableReason::ProbeRejected.to_string(),
            "diagnostic probe was rejected"
        );
        assert_eq!(
            map_process(ProcessError::Spawn(io::Error::from(
                io::ErrorKind::PermissionDenied
            ))),
            UnavailableReason::PermissionDenied
        );
        assert_eq!(
            map_process(ProcessError::Spawn(io::Error::from(
                io::ErrorKind::NotFound
            ))),
            UnavailableReason::Absent
        );
    }

    #[test]
    fn rejects_invalid_pins() {
        assert_eq!(
            PinnedTool::new(
                PathBuf::from("relative"),
                "0".repeat(64),
                "--version".into(),
                "v1\n".into()
            ),
            Err(UnavailableReason::ToolMismatch)
        );
        assert_eq!(
            PinnedTool::new(
                PathBuf::from("/bin/tool"),
                "Z".repeat(64),
                "--version".into(),
                "v1".into()
            ),
            Err(UnavailableReason::ToolMismatch)
        );
    }

    #[test]
    fn bounded_tool_boundary_covers_success_denial_timeout_and_cleanup() {
        let root = TestRoot::new();
        let diagnostics =
            KernelDiagnostics::new(root.0.clone(), Duration::from_millis(100)).unwrap();
        let tool = root.harness_tool();

        assert_eq!(
            diagnostics.run(
                &tool,
                &[
                    "--ignored".into(),
                    "--exact".into(),
                    "kernel::tests::fixture_process_success".into(),
                    "--nocapture".into(),
                ],
                parse_task_clock,
            ),
            ProbeResult::available(12_500_000)
        );
        root.assert_clean();

        assert_eq!(
            diagnostics.run(
                &tool,
                &[
                    "--ignored".into(),
                    "--exact".into(),
                    "kernel::tests::fixture_process_rejected".into(),
                    "--nocapture".into(),
                ],
                parse_ebpf_features,
            ),
            ProbeResult::unavailable_reason(UnavailableReason::ProbeRejected)
        );
        root.assert_clean();

        assert_eq!(
            diagnostics.run(
                &tool,
                &[
                    "--ignored".into(),
                    "--exact".into(),
                    "kernel::tests::fixture_process_timeout".into(),
                    "--nocapture".into(),
                ],
                parse_ebpf_features,
            ),
            ProbeResult::unavailable_reason(UnavailableReason::TimedOut)
        );
        root.assert_clean();

        let mut mismatch = tool.clone();
        mismatch.sha256 = "0".repeat(64);
        assert_eq!(
            diagnostics.perf_task_clock(&mismatch, 10),
            ProbeResult::unavailable_reason(UnavailableReason::ToolMismatch)
        );
        root.assert_clean();
    }

    #[test]
    fn missing_malformed_and_unsafe_configuration_fail_closed() {
        let root = TestRoot::new();
        let diagnostics = KernelDiagnostics::new(root.0.clone(), Duration::from_secs(1)).unwrap();
        let missing = PinnedTool::new(
            root.0.join("missing"),
            "0".repeat(64),
            "--version".into(),
            "fixture-v1".into(),
        )
        .unwrap();
        assert_eq!(
            diagnostics.perf_task_clock(&missing, 10),
            ProbeResult::unavailable_reason(UnavailableReason::Absent)
        );
        assert_eq!(
            diagnostics.perf_task_clock(&missing, 0),
            ProbeResult::unavailable_reason(UnavailableReason::ProbeRejected)
        );

        assert_eq!(
            diagnostics.run(
                &root.harness_tool(),
                &[
                    "--ignored".into(),
                    "--exact".into(),
                    "kernel::tests::fixture_process_malformed".into(),
                    "--nocapture".into(),
                ],
                parse_task_clock
            ),
            ProbeResult::unavailable_reason(UnavailableReason::MalformedEvidence)
        );
        root.assert_clean();

        let public = std::env::temp_dir();
        assert_eq!(
            KernelDiagnostics::new(public, Duration::from_secs(1))
                .err()
                .unwrap(),
            UnavailableReason::PermissionDenied
        );
    }

    #[test]
    fn cleanup_removes_bounded_sidecars_and_rejects_excess() {
        let root = TestRoot::new();
        let tool = root.harness_tool();
        let mut staged = StagedTool::new(&root.0, &tool, Duration::from_secs(1)).unwrap();
        fs::write(staged.directory.join("default.profraw"), b"coverage").unwrap();
        std::os::unix::fs::symlink("absent", staged.directory.join("sidecar-link")).unwrap();
        assert!(staged.cleanup());
        root.assert_clean();

        let mut staged = StagedTool::new(&root.0, &tool, Duration::from_secs(1)).unwrap();
        for index in 0..MAX_CLEANUP_ENTRIES {
            fs::write(staged.directory.join(format!("sidecar-{index}")), b"x").unwrap();
        }
        assert!(!staged.cleanup());
    }

    #[test]
    #[ignore = "subprocess fixture selected explicitly by the bounded boundary test"]
    fn fixture_process_success() {
        println!("12.5;msec;task-clock;1;100.0");
    }

    #[test]
    #[ignore = "subprocess fixture selected explicitly by the bounded boundary test"]
    fn fixture_process_rejected() {
        panic!("synthetic rejection");
    }

    #[test]
    #[ignore = "subprocess fixture selected explicitly by the bounded boundary test"]
    fn fixture_process_timeout() {
        std::thread::sleep(Duration::from_secs(2));
    }

    #[test]
    #[ignore = "subprocess fixture selected explicitly by the bounded boundary test"]
    fn fixture_process_malformed() {
        println!("synthetic-unparseable");
    }
}
