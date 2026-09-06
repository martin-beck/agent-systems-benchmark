// SPDX-License-Identifier: MIT
//! Rootless Bubblewrap isolation backed by delegated systemd cgroup scopes.

use crate::{ProcessError, ProcessLifecycle, ProcessLimits, ProcessOutput, RunningProcess};
use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const MAX_CPUS: usize = 4096;
const MAX_MEMORY: u64 = 16 * 1024 * 1024 * 1024;
const MAX_TASKS: u32 = 32_768;
const MAX_CPU_PERCENT: u32 = 100_000;
static PROBE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// A pinned executable at the isolation boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolPin {
    path: PathBuf,
    version_line: String,
    #[cfg(test)]
    shell_script: Option<PathBuf>,
}

impl ToolPin {
    /// Require an absolute path and exact first `--version` line.
    pub fn new(path: PathBuf, version_line: String) -> Result<Self, ConfigError> {
        if !path.is_absolute() || version_line.is_empty() || version_line.len() > 256 {
            return Err(ConfigError::ToolPin);
        }
        let path = fs::canonicalize(path).map_err(|_| ConfigError::ToolPin)?;
        let metadata = fs::metadata(&path).map_err(|_| ConfigError::ToolPin)?;
        if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
            return Err(ConfigError::ToolPin);
        }
        Ok(Self {
            path,
            version_line,
            #[cfg(test)]
            shell_script: None,
        })
    }
}

/// Sorted dedicated Linux CPU identifiers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CpuSet(Vec<u32>);

impl CpuSet {
    /// Validate a non-empty duplicate-free CPU set.
    pub fn new(mut cpus: Vec<u32>) -> Result<Self, ConfigError> {
        cpus.sort_unstable();
        if cpus.is_empty()
            || cpus.len() > MAX_CPUS
            || cpus.last().is_some_and(|cpu| *cpu >= MAX_CPUS as u32)
            || cpus.windows(2).any(|pair| pair[0] == pair[1])
        {
            return Err(ConfigError::CpuSet);
        }
        Ok(Self(cpus))
    }

    /// Return sorted CPU identifiers.
    pub fn as_slice(&self) -> &[u32] {
        &self.0
    }

    fn systemd_value(&self) -> String {
        self.0
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",")
    }
}

/// Cgroup memory, PID, CPU-bandwidth and placement budgets.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Resources {
    memory: u64,
    tasks: u32,
    cpu_percent: u32,
    cpus: CpuSet,
}

impl Resources {
    /// Validate all resource ceilings.
    pub fn new(
        memory: u64,
        tasks: u32,
        cpu_percent: u32,
        cpus: CpuSet,
    ) -> Result<Self, ConfigError> {
        if memory == 0 || memory > MAX_MEMORY {
            return Err(ConfigError::Memory);
        }
        if tasks == 0 || tasks > MAX_TASKS {
            return Err(ConfigError::Tasks);
        }
        if cpu_percent == 0 || cpu_percent > MAX_CPU_PERCENT {
            return Err(ConfigError::CpuQuota);
        }
        Ok(Self {
            memory,
            tasks,
            cpu_percent,
            cpus,
        })
    }
    /// Memory ceiling in bytes.
    pub fn memory(&self) -> u64 {
        self.memory
    }
    /// Process/thread ceiling.
    pub fn tasks(&self) -> u32 {
        self.tasks
    }
    /// CPU quota percent, where 100 is one CPU.
    pub fn cpu_percent(&self) -> u32 {
        self.cpu_percent
    }
    /// Dedicated CPU set.
    pub fn cpus(&self) -> &CpuSet {
        &self.cpus
    }
}

/// Network namespace policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkPolicy {
    /// Isolated namespace with no host interfaces.
    Deny,
    /// Deliberately unsupported for untrusted execution.
    Host,
}

/// Class sharing the same exclusive CPU lease namespace.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LeaseClass {
    /// Benchmark reservation.
    Benchmark,
    /// CI reservation.
    Ci,
}

/// Exclusive filesystem-backed CPU reservation.
#[derive(Debug)]
pub struct ResourceLease {
    files: Vec<(PathBuf, File)>,
    cpus: CpuSet,
    class: LeaseClass,
    release_on_drop: bool,
}

impl ResourceLease {
    /// Atomically reserve every CPU or roll back. The root must be trusted and
    /// existing; crash-stale files fail closed pending external reconciliation.
    pub fn acquire(root: &Path, class: LeaseClass, cpus: CpuSet) -> Result<Self, LeaseError> {
        let root = fs::canonicalize(root).map_err(LeaseError::Io)?;
        if !root.is_dir() {
            return Err(LeaseError::NotDirectory);
        }
        let mut files = Vec::with_capacity(cpus.0.len());
        for cpu in &cpus.0 {
            let path = root.join(format!("cpu-{cpu}.lease"));
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)
            {
                Ok(mut file) => {
                    if let Err(error) = writeln!(file, "{class:?}") {
                        let _ = fs::remove_file(&path);
                        rollback(&mut files);
                        return Err(LeaseError::Io(error));
                    }
                    files.push((path, file));
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    rollback(&mut files);
                    return Err(LeaseError::Conflict(*cpu));
                }
                Err(error) => {
                    rollback(&mut files);
                    return Err(LeaseError::Io(error));
                }
            }
        }
        Ok(Self {
            files,
            cpus,
            class,
            release_on_drop: true,
        })
    }
    /// Reserved CPUs.
    pub fn cpus(&self) -> &CpuSet {
        &self.cpus
    }
    /// Reservation class.
    pub fn class(&self) -> LeaseClass {
        self.class
    }

    fn quarantine(&mut self) {
        self.release_on_drop = false;
    }

    fn confirm_cleanup(&mut self) {
        self.release_on_drop = true;
    }
}

impl Drop for ResourceLease {
    fn drop(&mut self) {
        if self.release_on_drop {
            rollback(&mut self.files);
        }
    }
}

fn rollback(files: &mut Vec<(PathBuf, File)>) {
    for (path, file) in files.drain(..).rev() {
        drop(file);
        let _ = fs::remove_file(path);
    }
}

/// Validated rootless execution request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxSpec {
    workspace: PathBuf,
    working_directory: PathBuf,
    program: String,
    arguments: Vec<String>,
    environment: BTreeMap<String, String>,
    resources: Resources,
}

impl SandboxSpec {
    /// Validate workspace, bounded command data and network policy.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        workspace: &Path,
        working_directory: PathBuf,
        program: String,
        arguments: Vec<String>,
        environment: BTreeMap<String, String>,
        resources: Resources,
        network: NetworkPolicy,
    ) -> Result<Self, ConfigError> {
        let workspace = fs::canonicalize(workspace).map_err(ConfigError::Workspace)?;
        if !workspace.is_dir() {
            return Err(ConfigError::WorkingDirectory);
        }
        if working_directory.is_absolute()
            || working_directory
                .components()
                .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir))
            || !workspace.join(&working_directory).is_dir()
        {
            return Err(ConfigError::WorkingDirectory);
        }
        if !program.starts_with('/') || program.len() > 16_384 {
            return Err(ConfigError::Program);
        }
        let argument_bytes = arguments
            .iter()
            .try_fold(0_usize, |total, value| total.checked_add(value.len()));
        if arguments.len() > 512
            || argument_bytes.is_none_or(|total| total > 65_536)
            || arguments.iter().any(|value| value.len() > 16_384)
        {
            return Err(ConfigError::Arguments);
        }
        if environment.len() > 128
            || environment.iter().any(|(key, value)| {
                !valid_key(key)
                    || matches!(key.as_str(), "HOME" | "PATH" | "TMPDIR" | "ASB_SCOPE_NONCE")
                    || value.len() > 16_384
            })
        {
            return Err(ConfigError::Environment);
        }
        if network != NetworkPolicy::Deny {
            return Err(ConfigError::NetworkPolicy);
        }
        Ok(Self {
            workspace,
            working_directory,
            program,
            arguments,
            environment,
            resources,
        })
    }
}

fn valid_key(key: &str) -> bool {
    let mut bytes = key.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_uppercase() || byte == b'_')
        && bytes.all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}

/// Exact-pinned rootless namespace/cgroup backend.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxBackend {
    bubblewrap: ToolPin,
    systemd_run: ToolPin,
    systemctl: ToolPin,
    taskset: ToolPin,
}

impl SandboxBackend {
    /// Configure Bubblewrap and systemd-run pins.
    pub fn new(
        bubblewrap: ToolPin,
        systemd_run: ToolPin,
        systemctl: ToolPin,
        taskset: ToolPin,
    ) -> Self {
        Self {
            bubblewrap,
            systemd_run,
            systemctl,
            taskset,
        }
    }

    /// Prove executable identity and disposable delegated isolation.
    pub fn probe(&self) -> Result<(), SandboxError> {
        probe_tool(&self.bubblewrap)?;
        probe_tool(&self.systemd_run)?;
        probe_tool(&self.systemctl)?;
        probe_tool(&self.taskset)?;
        let unit = format!(
            "asb-probe-{}-{}",
            std::process::id(),
            PROBE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        );
        let mut command = pinned_command(&self.systemd_run);
        add_scope(
            &mut command,
            &unit,
            64 * 1024 * 1024,
            16,
            100,
            None,
            Duration::from_secs(10),
        );
        command.arg(&self.bubblewrap.path).args(namespace_args());
        add_runtime_mounts(&mut command);
        command.arg("--").arg("/usr/bin/true");
        let mut process =
            RunningProcess::spawn(command, probe_limits()).map_err(SandboxError::Run)?;
        let output = process.wait().map_err(SandboxError::Run)?;
        if output.exit_code == Some(0) {
            Ok(())
        } else {
            Err(SandboxError::DelegationRejected)
        }
    }

    /// Spawn while retaining a matching benchmark CPU lease.
    pub fn spawn(
        &self,
        spec: SandboxSpec,
        lease: ResourceLease,
        limits: ProcessLimits,
    ) -> Result<SandboxProcess, SandboxError> {
        let (unit, nonce) = new_scope_identity()?;
        self.spawn_with_identity(spec, lease, limits, unit, nonce)
    }

    fn spawn_with_identity(
        &self,
        spec: SandboxSpec,
        mut lease: ResourceLease,
        limits: ProcessLimits,
        unit: String,
        nonce: String,
    ) -> Result<SandboxProcess, SandboxError> {
        if lease.class != LeaseClass::Benchmark || lease.cpus != spec.resources.cpus {
            return Err(SandboxError::LeaseMismatch);
        }
        self.probe()?;
        let mut command = pinned_command(&self.systemd_run);
        command.env("ASB_SCOPE_NONCE", &nonce);
        add_scope(
            &mut command,
            &unit,
            spec.resources.memory,
            spec.resources.tasks,
            spec.resources.cpu_percent,
            Some(&spec.resources.cpus),
            limits.timeout(),
        );
        command
            .arg(&self.taskset.path)
            .args(["--cpu-list", &spec.resources.cpus.systemd_value()])
            .arg(&self.bubblewrap.path)
            .args(namespace_args());
        add_runtime_mounts(&mut command);
        command
            .args(["--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp"])
            .arg("--bind")
            .arg(&spec.workspace)
            .arg("/workspace")
            .arg("--chdir")
            .arg(Path::new("/workspace").join(&spec.working_directory))
            .args(["--setenv", "PATH", "/usr/bin:/bin"])
            .args(["--setenv", "HOME", "/workspace"])
            .args(["--setenv", "TMPDIR", "/tmp"])
            .args(["--setenv", "ASB_SCOPE_NONCE", &nonce]);
        for (key, value) in &spec.environment {
            command.args(["--setenv", key, value]);
        }
        command.arg("--").arg(&spec.program).args(&spec.arguments);
        let mut process = RunningProcess::spawn(command, limits).map_err(SandboxError::Run)?;
        let scope_cleanup_required = match await_scope_ownership(
            &mut process,
            &self.systemctl,
            &unit,
            &nonce,
            spec.resources.tasks,
        ) {
            Ok(owned) => owned,
            Err(error) => {
                lease.quarantine();
                return Err(error);
            }
        };
        Ok(SandboxProcess {
            process,
            lease,
            unit,
            systemctl: self.systemctl.clone(),
            scope_cleanup_required,
        })
    }
}

fn new_scope_identity() -> Result<(String, String), SandboxError> {
    let file =
        File::open("/proc/sys/kernel/random/uuid").map_err(|_| SandboxError::ScopeIdentity)?;
    let mut bytes = Vec::with_capacity(64);
    file.take(64)
        .read_to_end(&mut bytes)
        .map_err(|_| SandboxError::ScopeIdentity)?;
    let uuid = String::from_utf8(bytes).map_err(|_| SandboxError::ScopeIdentity)?;
    let uuid = uuid.trim();
    if !valid_scope_uuid(uuid) {
        return Err(SandboxError::ScopeIdentity);
    }
    Ok((format!("asb-{uuid}"), uuid.to_owned()))
}

fn valid_scope_uuid(uuid: &str) -> bool {
    let raw = uuid.as_bytes();
    raw.len() == 36
        && raw[8] == 45
        && raw[13] == 45
        && raw[18] == 45
        && raw[23] == 45
        && !raw
            .iter()
            .enumerate()
            .any(|(index, byte)| !matches!(index, 8 | 13 | 18 | 23) && !byte.is_ascii_hexdigit())
}

fn await_scope_ownership(
    process: &mut RunningProcess,
    systemctl: &ToolPin,
    unit: &str,
    nonce: &str,
    task_limit: u32,
) -> Result<bool, SandboxError> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if scope_owns_nonce(systemctl, unit, nonce, task_limit)? {
            return Ok(true);
        }
        if process.leader_has_exited().map_err(SandboxError::Run)? {
            let output = process.wait().cloned().map_err(SandboxError::Run)?;
            if output.exit_code == Some(0) {
                loop {
                    let (state_output, state) = scope_state(systemctl, unit)?;
                    if matches!(
                        (state_output.exit_code, state.as_str()),
                        (Some(3 | 4), "inactive" | "failed")
                    ) {
                        return Ok(false);
                    }
                    if Instant::now() >= deadline {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
            return Err(SandboxError::ScopeOwnership {
                exit_code: output.exit_code,
                stderr: bounded_diagnostic(&output.stderr.bytes),
            });
        }
        if Instant::now() >= deadline {
            let _ = process.cancel();
            let _ = process.wait();
            return Err(SandboxError::ScopeOwnership {
                exit_code: None,
                stderr: "scope ownership was not proven before deadline".into(),
            });
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn scope_owns_nonce(
    systemctl: &ToolPin,
    unit: &str,
    nonce: &str,
    task_limit: u32,
) -> Result<bool, SandboxError> {
    let mut command = pinned_command(systemctl);
    command.args([
        "--user",
        "show",
        &format!("{unit}.scope"),
        "--property=ControlGroup",
        "--value",
    ]);
    let mut process = RunningProcess::spawn(command, probe_limits()).map_err(SandboxError::Run)?;
    let output = process.wait().map_err(SandboxError::Run)?;
    if output.exit_code != Some(0) {
        return Ok(false);
    }
    let control = String::from_utf8_lossy(&output.stdout.bytes)
        .trim()
        .to_owned();
    if control.is_empty() {
        return Ok(false);
    }
    let relative = Path::new(&control)
        .strip_prefix(Path::new("/"))
        .map_err(|_| SandboxError::ScopeIdentity)?;
    if relative
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(SandboxError::ScopeIdentity);
    }
    let process_path = Path::new("/sys/fs/cgroup")
        .join(relative)
        .join("cgroup.procs");
    let processes = match fs::read_to_string(process_path) {
        Ok(value) if value.len() <= usize::try_from(task_limit).unwrap_or(usize::MAX) * 16 => value,
        Ok(_) => return Err(SandboxError::ScopeIdentity),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err(SandboxError::ScopeIdentity),
    };
    let expected = format!("ASB_SCOPE_NONCE={nonce}").into_bytes();
    for pid in processes.lines() {
        if !pid.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(SandboxError::ScopeIdentity);
        }
        let Ok(file) = File::open(Path::new("/proc").join(pid).join("environ")) else {
            continue;
        };
        let mut environment = Vec::with_capacity(4096);
        file.take(64 * 1024)
            .read_to_end(&mut environment)
            .map_err(|_| SandboxError::ScopeIdentity)?;
        if environment
            .split(|byte| *byte == 0)
            .any(|entry| entry == expected)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn bounded_diagnostic(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&bytes[..bytes.len().min(4096)])
        .trim()
        .to_owned()
}

fn namespace_args() -> [&'static str; 9] {
    [
        "--die-with-parent",
        "--new-session",
        "--unshare-all",
        "--unshare-user",
        "--clearenv",
        "--disable-userns",
        "--assert-userns-disabled",
        "--cap-drop",
        "ALL",
    ]
}

fn add_runtime_mounts(command: &mut Command) {
    command.args(["--ro-bind", "/usr", "/usr"]);
    for path in ["/bin", "/lib", "/lib64", "/etc/ld.so.cache"] {
        command.args(["--ro-bind-try", path, path]);
    }
}

fn add_scope(
    command: &mut Command,
    unit: &str,
    memory: u64,
    tasks: u32,
    cpu_percent: u32,
    cpus: Option<&CpuSet>,
    runtime: Duration,
) {
    command
        .args(["--user", "--scope", "--quiet", "--collect"])
        .arg(format!("--unit={unit}.scope"))
        .arg("--property")
        .arg(format!("MemoryMax={memory}"))
        .arg("--property")
        .arg("MemorySwapMax=0")
        .arg("--property")
        .arg(format!("TasksMax={tasks}"))
        .arg("--property")
        .arg(format!("CPUQuota={cpu_percent}%"))
        .arg("--property")
        .arg(format!("RuntimeMaxSec={}ms", runtime.as_millis()));
    if let Some(cpus) = cpus {
        command
            .arg("--property")
            .arg(format!("AllowedCPUs={}", cpus.systemd_value()));
    }
}

fn pinned_command(pin: &ToolPin) -> Command {
    #[cfg(test)]
    if let Some(script) = &pin.shell_script {
        let mut command = Command::new("/bin/sh");
        command.arg(script);
        return command;
    }
    Command::new(&pin.path)
}

fn probe_tool(pin: &ToolPin) -> Result<(), SandboxError> {
    let mut command = pinned_command(pin);
    command.arg("--version");
    let mut process = RunningProcess::spawn(command, probe_limits()).map_err(SandboxError::Run)?;
    let output = process.wait().map_err(SandboxError::Run)?;
    let observed = String::from_utf8_lossy(&output.stdout.bytes)
        .lines()
        .next()
        .unwrap_or_default()
        .to_owned();
    if output.exit_code == Some(0) && observed == pin.version_line {
        Ok(())
    } else {
        Err(SandboxError::ToolVersion {
            path: pin.path.clone(),
            expected: pin.version_line.clone(),
            observed,
        })
    }
}

fn probe_limits() -> ProcessLimits {
    ProcessLimits::new(
        4096,
        4096,
        Duration::from_secs(10),
        Duration::from_millis(100),
        Duration::from_millis(5),
    )
    .expect("static probe limits are valid")
}

/// Running sandbox retaining its resource lease.
pub struct SandboxProcess {
    process: RunningProcess,
    lease: ResourceLease,
    unit: String,
    systemctl: ToolPin,
    scope_cleanup_required: bool,
}

impl SandboxProcess {
    /// Systemd scope name without suffix.
    pub fn unit(&self) -> &str {
        &self.unit
    }
    /// Current process lifecycle.
    pub fn lifecycle(&self) -> ProcessLifecycle {
        self.process.lifecycle()
    }
    /// Request idempotent cancellation.
    pub fn cancel(&mut self) -> Result<(), SandboxError> {
        let process_result = if self.process.lifecycle() != ProcessLifecycle::Terminal {
            self.process.cancel().map_err(SandboxError::Run)
        } else {
            Ok(())
        };
        let cleanup_result = if self.scope_cleanup_required {
            self.cleanup_scope()
        } else {
            Ok(())
        };
        process_result?;
        cleanup_result?;
        Ok(())
    }
    /// Wait and return owned terminal evidence.
    pub fn wait(&mut self) -> Result<ProcessOutput, SandboxError> {
        let output = self.process.wait().cloned().map_err(SandboxError::Run);
        if self.scope_cleanup_required {
            self.cleanup_scope()?;
        }
        output
    }

    fn cleanup_scope(&mut self) -> Result<(), SandboxError> {
        match stop_scope(&self.systemctl, &self.unit) {
            Ok(()) => {
                self.scope_cleanup_required = false;
                self.lease.confirm_cleanup();
                Ok(())
            }
            Err(error) => {
                self.lease.quarantine();
                Err(error)
            }
        }
    }
}

impl Drop for SandboxProcess {
    fn drop(&mut self) {
        if self.scope_cleanup_required {
            if stop_scope(&self.systemctl, &self.unit).is_ok() {
                self.scope_cleanup_required = false;
                self.lease.confirm_cleanup();
            } else {
                self.lease.quarantine();
            }
        }
    }
}

fn stop_scope(systemctl: &ToolPin, unit: &str) -> Result<(), SandboxError> {
    stop_scope_with_timeout(systemctl, unit, Duration::from_secs(5))
}

fn stop_scope_with_timeout(
    systemctl: &ToolPin,
    unit: &str,
    cleanup_timeout: Duration,
) -> Result<(), SandboxError> {
    let (output, state) = scope_state(systemctl, unit)?;
    if matches!(
        (output.exit_code, state.as_str()),
        (Some(3 | 4), "inactive" | "failed")
    ) {
        return Ok(());
    }
    if !matches!(
        (output.exit_code, state.as_str()),
        (
            Some(0 | 3),
            "active" | "activating" | "deactivating" | "reloading"
        )
    ) {
        return Err(scope_cleanup_error(&output));
    }

    let mut command = pinned_command(systemctl);
    command.args([
        "--user",
        "kill",
        "--kill-whom=all",
        "--signal=KILL",
        &format!("{unit}.scope"),
    ]);
    let mut process = RunningProcess::spawn(command, probe_limits()).map_err(SandboxError::Run)?;
    let output = process.wait().map_err(SandboxError::Run)?;
    if output.exit_code != Some(0) {
        let (current, state) = scope_state(systemctl, unit)?;
        if matches!(
            (current.exit_code, state.as_str()),
            (Some(3 | 4), "inactive" | "failed")
        ) {
            return Ok(());
        }
        return Err(scope_cleanup_error(output));
    }

    let mut command = pinned_command(systemctl);
    command.args([
        "--user",
        "--no-block",
        "--quiet",
        "stop",
        &format!("{unit}.scope"),
    ]);
    let mut process = RunningProcess::spawn(command, probe_limits()).map_err(SandboxError::Run)?;
    let output = process.wait().map_err(SandboxError::Run)?;
    if !matches!(output.exit_code, Some(0 | 5)) {
        return Err(scope_cleanup_error(output));
    }

    let deadline = Instant::now() + cleanup_timeout;
    loop {
        let (output, state) = scope_state(systemctl, unit)?;
        match (output.exit_code, state.as_str()) {
            (Some(3 | 4), "inactive" | "failed") => return Ok(()),
            (Some(0 | 3), "active" | "activating" | "deactivating" | "reloading")
                if Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            (Some(0 | 3), "active" | "activating" | "deactivating" | "reloading") => {
                return Err(SandboxError::ScopeCleanup {
                    exit_code: None,
                    stderr: "scope remained active after stop deadline".into(),
                });
            }
            _ => return Err(scope_cleanup_error(&output)),
        }
    }
}

fn scope_state(systemctl: &ToolPin, unit: &str) -> Result<(ProcessOutput, String), SandboxError> {
    let mut command = pinned_command(systemctl);
    command.args(["--user", "is-active", &format!("{unit}.scope")]);
    let mut process = RunningProcess::spawn(command, probe_limits()).map_err(SandboxError::Run)?;
    let output = process.wait().cloned().map_err(SandboxError::Run)?;
    let state = String::from_utf8_lossy(&output.stdout.bytes)
        .trim()
        .to_owned();
    Ok((output, state))
}

fn scope_cleanup_error(output: &ProcessOutput) -> SandboxError {
    SandboxError::ScopeCleanup {
        exit_code: output.exit_code,
        stderr: String::from_utf8_lossy(&output.stderr.bytes)
            .trim()
            .to_owned(),
    }
}

/// Invalid sandbox configuration.
#[derive(Debug)]
pub enum ConfigError {
    /// Invalid executable pin.
    ToolPin,
    /// Invalid CPU set.
    CpuSet,
    /// Invalid memory ceiling.
    Memory,
    /// Invalid task ceiling.
    Tasks,
    /// Invalid CPU quota.
    CpuQuota,
    /// Workspace resolution failure.
    Workspace(io::Error),
    /// Invalid working directory.
    WorkingDirectory,
    /// Invalid in-sandbox executable.
    Program,
    /// Unbounded arguments.
    Arguments,
    /// Invalid or unbounded environment.
    Environment,
    /// Unsupported host networking.
    NetworkPolicy,
}

/// Resource lease failure.
#[derive(Debug)]
pub enum LeaseError {
    /// Filesystem failure.
    Io(io::Error),
    /// Root is not a directory.
    NotDirectory,
    /// CPU is already reserved by benchmark or CI.
    Conflict(u32),
}

/// Sandbox execution failure.
#[derive(Debug)]
pub enum SandboxError {
    /// Process boundary failure.
    Run(ProcessError),
    /// Executable identity mismatch.
    ToolVersion {
        /// Executable path.
        path: PathBuf,
        /// Required first line.
        expected: String,
        /// Observed first line.
        observed: String,
    },
    /// Delegated namespace/cgroup probe failed.
    DelegationRejected,
    /// A collision-resistant internal scope identity could not be obtained.
    ScopeIdentity,
    /// The launched process was not proven to own the generated scope.
    ScopeOwnership {
        /// Observed systemd-run exit status.
        exit_code: Option<i32>,
        /// Bounded diagnostic text.
        stderr: String,
    },
    /// The delegated scope could not be stopped.
    ScopeCleanup {
        /// systemctl exit status.
        exit_code: Option<i32>,
        /// Bounded diagnostic text.
        stderr: String,
    },
    /// Lease class or CPUs did not match.
    LeaseMismatch,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid sandbox configuration: {self:?}")
    }
}
impl fmt::Display for LeaseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "resource lease failure: {self:?}")
    }
}
impl fmt::Display for SandboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sandbox failure: {self:?}")
    }
}
impl std::error::Error for ConfigError {}
impl std::error::Error for LeaseError {}
impl std::error::Error for SandboxError {}

#[cfg(test)]
mod tests {
    use super::*;

    static FAKE_TOOL_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    static SCRATCH_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn resolve_target_root(value: Option<std::ffi::OsString>, current: &Path) -> PathBuf {
        let Some(value) = value else {
            return std::env::temp_dir();
        };
        let path = PathBuf::from(value);
        let absolute = if path.is_absolute() {
            path
        } else {
            current.join(path)
        };
        let mut normalized = PathBuf::new();
        for component in absolute.components() {
            match component {
                std::path::Component::CurDir => {}
                std::path::Component::ParentDir => {
                    normalized.pop();
                }
                _ => normalized.push(component.as_os_str()),
            }
        }

        let mut prefix = normalized.clone();
        let mut suffix = Vec::new();
        loop {
            if let Ok(mut canonical) = fs::canonicalize(&prefix) {
                for component in suffix.iter().rev() {
                    canonical.push(component);
                }
                return canonical;
            }
            let Some(component) = prefix.file_name().map(ToOwned::to_owned) else {
                return normalized;
            };
            suffix.push(component);
            if !prefix.pop() {
                return normalized;
            }
        }
    }

    fn scratch(name: &str) -> ScratchPath {
        let base = resolve_target_root(
            std::env::var_os("CARGO_TARGET_DIR"),
            &std::env::current_dir().unwrap(),
        );
        let sequence = SCRATCH_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        ScratchPath(base.join(format!(
            "asb-sandbox-{name}-{}-{sequence}",
            std::process::id()
        )))
    }

    struct ScratchPath(PathBuf);

    impl ScratchPath {
        fn create(name: &str) -> Self {
            let path = scratch(name);
            fs::create_dir_all(&path).unwrap();
            path
        }
    }

    impl std::ops::Deref for ScratchPath {
        type Target = Path;

        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    impl AsRef<Path> for ScratchPath {
        fn as_ref(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for ScratchPath {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
            let _ = fs::remove_file(&self.0);
        }
    }

    fn fake_systemctl(name: &str, body: &str) -> (ScratchPath, ToolPin) {
        let root = scratch(name);
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let tool = root.join("systemctl");
        let pending = root.join("systemctl.pending");
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o700)
            .open(&pending)
            .unwrap();
        write!(file, "#!/bin/sh\nstate=\"$0.state\"\n{body}\n").unwrap();
        file.sync_all().unwrap();
        drop(file);
        fs::rename(pending, &tool).unwrap();
        let pin = ToolPin {
            path: PathBuf::from("/bin/sh"),
            version_line: "fake-systemctl".into(),
            shell_script: Some(tool),
        };
        (root, pin)
    }

    fn test_resources() -> Resources {
        Resources::new(1024, 2, 100, CpuSet::new(vec![0]).unwrap()).unwrap()
    }

    fn workspace() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_owned()
    }

    fn spec_with(
        working: PathBuf,
        program: &str,
        arguments: Vec<String>,
        environment: BTreeMap<String, String>,
        _unit: &str,
        network: NetworkPolicy,
    ) -> Result<SandboxSpec, ConfigError> {
        SandboxSpec::new(
            &workspace(),
            working,
            program.into(),
            arguments,
            environment,
            test_resources(),
            network,
        )
    }

    #[test]
    fn limits_fail_closed() {
        assert!(CpuSet::new(vec![]).is_err());
        assert!(CpuSet::new(vec![1, 1]).is_err());
        let cpus = CpuSet::new(vec![0]).unwrap();
        assert!(Resources::new(0, 1, 100, cpus.clone()).is_err());
        assert!(Resources::new(1, 0, 100, cpus.clone()).is_err());
        assert!(Resources::new(1, 1, 0, cpus).is_err());
        assert!(ToolPin::new(PathBuf::from("relative"), "v1".into()).is_err());
        assert!(ToolPin::new(PathBuf::from("/definitely/missing"), "v1".into()).is_err());
        assert!(ToolPin::new(PathBuf::from("/etc/hosts"), "v1".into()).is_err());
        assert!(Resources::new(MAX_MEMORY + 1, 1, 100, CpuSet::new(vec![0]).unwrap()).is_err());
        assert!(Resources::new(1, MAX_TASKS + 1, 100, CpuSet::new(vec![0]).unwrap()).is_err());
        assert!(Resources::new(1, 1, MAX_CPU_PERCENT + 1, CpuSet::new(vec![0]).unwrap()).is_err());
        assert!(CpuSet::new(vec![MAX_CPUS as u32]).is_err());
    }

    #[test]
    fn internally_generated_scope_identities_are_unique_and_bounded() {
        let mut units = Vec::new();
        for _ in 0..64 {
            let (unit, nonce) = new_scope_identity().unwrap();
            assert!(unit.starts_with("asb-"));
            assert_eq!(unit.len(), 40);
            assert_eq!(nonce.len(), 36);
            assert!(!units.contains(&unit));
            units.push(unit);
        }
        for malformed in [
            "",
            "0000000-0000-0000-0000-000000000000",
            "00000000_0000-0000-0000-000000000000",
            "00000000-0000_0000-0000-000000000000",
            "00000000-0000-0000_0000-000000000000",
            "00000000-0000-0000-0000_000000000000",
            "00000000-0000-0000-0000-00000000000z",
        ] {
            assert!(!valid_scope_uuid(malformed));
        }
    }

    #[test]
    fn scope_ownership_metadata_is_bounded_and_fail_closed() {
        let _guard = FAKE_TOOL_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (root, pin) = fake_systemctl("ownership-empty", "exit 0");
        assert!(!scope_owns_nonce(&pin, "unit", "nonce", 1).unwrap());
        fs::remove_dir_all(root).unwrap();

        let (root, pin) = fake_systemctl("ownership-escape", "echo ../escape; exit 0");
        assert!(matches!(
            scope_owns_nonce(&pin, "unit", "nonce", 1),
            Err(SandboxError::ScopeIdentity)
        ));
        fs::remove_dir_all(root).unwrap();

        let (root, pin) = fake_systemctl(
            "ownership-missing-cgroup",
            "echo /asb-cgroup-that-does-not-exist; exit 0",
        );
        assert!(!scope_owns_nonce(&pin, "unit", "nonce", 1).unwrap());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn malformed_specs_fail_before_spawn() {
        assert!(matches!(
            spec_with(
                PathBuf::from("../outside"),
                "/bin/true",
                vec![],
                BTreeMap::new(),
                "valid",
                NetworkPolicy::Deny
            ),
            Err(ConfigError::WorkingDirectory)
        ));
        assert!(matches!(
            spec_with(
                PathBuf::new(),
                "relative",
                vec![],
                BTreeMap::new(),
                "valid",
                NetworkPolicy::Deny
            ),
            Err(ConfigError::Program)
        ));
        assert!(matches!(
            spec_with(
                PathBuf::new(),
                "/bin/true",
                vec!["x".repeat(65_537)],
                BTreeMap::new(),
                "valid",
                NetworkPolicy::Deny
            ),
            Err(ConfigError::Arguments)
        ));
        assert!(matches!(
            spec_with(
                PathBuf::new(),
                "/bin/true",
                vec![],
                BTreeMap::from([("PATH".into(), "bad".into())]),
                "valid",
                NetworkPolicy::Deny
            ),
            Err(ConfigError::Environment)
        ));
        assert!(matches!(
            spec_with(
                PathBuf::new(),
                "/bin/true",
                vec![],
                BTreeMap::new(),
                "valid",
                NetworkPolicy::Host
            ),
            Err(ConfigError::NetworkPolicy)
        ));
        spec_with(
            PathBuf::new(),
            "/bin/true",
            vec![],
            BTreeMap::from([("VALID_1".into(), "value".into())]),
            "valid",
            NetworkPolicy::Deny,
        )
        .unwrap();
    }

    #[test]
    fn every_bounded_spec_edge_fails_closed() {
        let missing = scratch("missing-workspace");
        let _ = fs::remove_dir_all(&missing);
        assert!(matches!(
            SandboxSpec::new(
                &missing,
                PathBuf::new(),
                "/bin/true".into(),
                vec![],
                BTreeMap::new(),
                test_resources(),
                NetworkPolicy::Deny,
            ),
            Err(ConfigError::Workspace(_))
        ));

        let root = scratch("spec-edges");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("work")).unwrap();
        let file = root.join("file");
        File::create(&file).unwrap();
        let resources = test_resources();
        let make = |working, program: String, arguments, environment, _unit: String| {
            SandboxSpec::new(
                &root,
                working,
                program,
                arguments,
                environment,
                resources.clone(),
                NetworkPolicy::Deny,
            )
        };
        assert!(matches!(
            SandboxSpec::new(
                &file,
                PathBuf::new(),
                "/bin/true".into(),
                vec![],
                BTreeMap::new(),
                resources.clone(),
                NetworkPolicy::Deny,
            ),
            Err(ConfigError::WorkingDirectory)
        ));
        assert!(matches!(
            make(
                PathBuf::from("/work"),
                "/bin/true".into(),
                vec![],
                BTreeMap::new(),
                "valid".into()
            ),
            Err(ConfigError::WorkingDirectory)
        ));
        assert!(matches!(
            make(
                PathBuf::from("absent"),
                "/bin/true".into(),
                vec![],
                BTreeMap::new(),
                "valid".into()
            ),
            Err(ConfigError::WorkingDirectory)
        ));
        assert!(matches!(
            make(
                PathBuf::new(),
                format!("/{}", "p".repeat(16_384)),
                vec![],
                BTreeMap::new(),
                "valid".into()
            ),
            Err(ConfigError::Program)
        ));
        assert!(matches!(
            make(
                PathBuf::new(),
                "/bin/true".into(),
                vec![String::new(); 513],
                BTreeMap::new(),
                "valid".into()
            ),
            Err(ConfigError::Arguments)
        ));
        assert!(matches!(
            make(
                PathBuf::new(),
                "/bin/true".into(),
                vec!["a".repeat(16_384); 5],
                BTreeMap::new(),
                "valid".into()
            ),
            Err(ConfigError::Arguments)
        ));
        assert!(matches!(
            make(
                PathBuf::new(),
                "/bin/true".into(),
                vec![],
                BTreeMap::from_iter((0..129).map(|i| (format!("K_{i}"), String::new()))),
                "valid".into()
            ),
            Err(ConfigError::Environment)
        ));
        assert!(matches!(
            make(
                PathBuf::new(),
                "/bin/true".into(),
                vec![],
                BTreeMap::from([("lower".into(), String::new())]),
                "valid".into()
            ),
            Err(ConfigError::Environment)
        ));
        assert!(matches!(
            make(
                PathBuf::new(),
                "/bin/true".into(),
                vec![],
                BTreeMap::from([("VALUE".into(), "x".repeat(16_385))]),
                "valid".into()
            ),
            Err(ConfigError::Environment)
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tool_mismatch_and_lease_mismatch_are_explicit() {
        let wrong = ToolPin::new(PathBuf::from("/bin/true"), "not-the-version".into()).unwrap();
        assert!(matches!(
            probe_tool(&wrong),
            Err(SandboxError::ToolVersion { .. })
        ));

        let root = scratch("mismatch");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("leases")).unwrap();
        let ci = ResourceLease::acquire(
            &root.join("leases"),
            LeaseClass::Ci,
            CpuSet::new(vec![0]).unwrap(),
        )
        .unwrap();
        let pin = ToolPin::new(PathBuf::from("/bin/true"), "unused".into()).unwrap();
        let backend = SandboxBackend::new(pin.clone(), pin.clone(), pin.clone(), pin);
        let spec = SandboxSpec::new(
            &root,
            PathBuf::new(),
            "/bin/true".into(),
            vec![],
            BTreeMap::new(),
            test_resources(),
            NetworkPolicy::Deny,
        )
        .unwrap();
        assert!(matches!(
            backend.spawn(spec, ci, probe_limits()),
            Err(SandboxError::LeaseMismatch)
        ));
        assert_eq!(fs::read_dir(root.join("leases")).unwrap().count(), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn leases_exclude_benchmark_and_ci_overlap() {
        let fixture = ScratchPath::create("lease-overlap");
        let root = fixture.as_ref();
        let ci =
            ResourceLease::acquire(root, LeaseClass::Ci, CpuSet::new(vec![2]).unwrap()).unwrap();
        assert!(matches!(
            ResourceLease::acquire(
                root,
                LeaseClass::Benchmark,
                CpuSet::new(vec![1, 2]).unwrap()
            ),
            Err(LeaseError::Conflict(2))
        ));
        let benchmark =
            ResourceLease::acquire(root, LeaseClass::Benchmark, CpuSet::new(vec![1]).unwrap())
                .unwrap();
        drop(ci);
        drop(benchmark);
        assert_eq!(fs::read_dir(root).unwrap().count(), 0);
    }

    #[test]
    fn scratch_fixtures_are_unique_external_and_panic_cleaned() {
        let repository_target = workspace().join("target");
        let target_existed = repository_target.exists();
        let current = std::env::current_dir().unwrap();
        assert_eq!(
            resolve_target_root(Some("target".into()), &current),
            current.join("target")
        );
        assert_eq!(
            resolve_target_root(Some("existing/../target".into()), &current),
            current.join("target")
        );
        assert_eq!(
            resolve_target_root(Some("../external-target".into()), &current),
            current.parent().unwrap().join("external-target")
        );
        let first = ScratchPath::create("portability");
        let second = ScratchPath::create("portability");
        assert_ne!(first.as_ref(), second.as_ref());
        assert!(!first.starts_with(&repository_target));
        let first_path = first.to_path_buf();
        let second_path = second.to_path_buf();

        let failure = std::panic::catch_unwind(move || {
            let _fixture = first;
            panic!("injected fixture failure");
        });
        assert!(failure.is_err());
        assert!(!first_path.exists());
        drop(second);
        assert!(!second_path.exists());
        assert_eq!(repository_target.exists(), target_existed);
    }

    #[test]
    fn leases_report_bad_roots_and_roll_back_partial_acquisition() {
        let missing = scratch("missing-lease-root");
        let _ = fs::remove_dir_all(&missing);
        assert!(matches!(
            ResourceLease::acquire(
                &missing,
                LeaseClass::Benchmark,
                CpuSet::new(vec![0]).unwrap()
            ),
            Err(LeaseError::Io(_))
        ));

        let file = scratch("lease-file");
        let _ = fs::remove_file(&file);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        File::create(&file).unwrap();
        assert!(matches!(
            ResourceLease::acquire(&file, LeaseClass::Benchmark, CpuSet::new(vec![0]).unwrap()),
            Err(LeaseError::NotDirectory)
        ));
        fs::remove_file(file).unwrap();

        let root = scratch("lease-rollback");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).unwrap();
        File::create(root.join("cpu-2.lease")).unwrap();
        assert!(matches!(
            ResourceLease::acquire(
                &root,
                LeaseClass::Benchmark,
                CpuSet::new(vec![1, 2]).unwrap()
            ),
            Err(LeaseError::Conflict(2))
        ));
        assert!(!root.join("cpu-1.lease").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn accessors_and_error_messages_expose_owned_evidence() {
        let cpus = CpuSet::new(vec![3, 1]).unwrap();
        assert_eq!(cpus.as_slice(), &[1, 3]);
        assert_eq!(cpus.systemd_value(), "1,3");
        let resources = Resources::new(4096, 7, 250, cpus.clone()).unwrap();
        assert_eq!(resources.memory(), 4096);
        assert_eq!(resources.tasks(), 7);
        assert_eq!(resources.cpu_percent(), 250);
        assert_eq!(resources.cpus(), &cpus);

        let root = scratch("lease-accessors");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let lease = ResourceLease::acquire(&root, LeaseClass::Ci, cpus.clone()).unwrap();
        assert_eq!(lease.cpus(), &cpus);
        assert_eq!(lease.class(), LeaseClass::Ci);
        drop(lease);
        fs::remove_dir(root).unwrap();

        assert!(ConfigError::Program.to_string().contains("configuration"));
        assert!(LeaseError::Conflict(1).to_string().contains("lease"));
        assert!(
            SandboxError::DelegationRejected
                .to_string()
                .contains("sandbox")
        );
    }

    #[test]
    fn cleanup_failure_is_typed_and_bounded() {
        let _guard = FAKE_TOOL_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (root, pin) = fake_systemctl("cleanup-error", "echo cleanup-rejected >&2; exit 2");
        assert!(matches!(
            stop_scope(&pin, "unit"),
            Err(SandboxError::ScopeCleanup {
                exit_code: Some(2),
                ref stderr,
            }) if stderr == "cleanup-rejected"
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn scope_cleanup_handles_completion_races_and_command_failures() {
        let _guard = FAKE_TOOL_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (root, pin) = fake_systemctl(
            "cleanup-race",
            "case \"$2\" in\n  is-active) if [ -e \"$state\" ]; then echo inactive; exit 4; else echo active; exit 0; fi;;\n  kill) touch \"$state\"; exit 1;;\nesac\nexit 2",
        );
        assert!(stop_scope(&pin, "unit").is_ok());
        fs::remove_dir_all(root).unwrap();

        let (root, pin) = fake_systemctl(
            "cleanup-kill-error",
            "case \"$2\" in\n  is-active) echo active; exit 0;;\n  kill) echo kill-rejected >&2; exit 2;;\nesac\nexit 2",
        );
        assert!(matches!(
            stop_scope(&pin, "unit"),
            Err(SandboxError::ScopeCleanup {
                exit_code: Some(2),
                ref stderr,
            }) if stderr == "kill-rejected"
        ));
        fs::remove_dir_all(root).unwrap();

        let (root, pin) = fake_systemctl(
            "cleanup-stop-error",
            "case \"$2\" in\n  is-active) echo active; exit 0;;\n  kill) exit 0;;\n  --no-block) echo stop-rejected >&2; exit 2;;\nesac\nexit 2",
        );
        assert!(matches!(
            stop_scope(&pin, "unit"),
            Err(SandboxError::ScopeCleanup {
                exit_code: Some(2),
                ref stderr,
            }) if stderr == "stop-rejected"
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn successful_short_scope_waits_through_terminal_state_transition() {
        let _guard = FAKE_TOOL_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (root, pin) = fake_systemctl(
            "ownership-terminal-transition",
            "case \"$2\" in\n  show) exit 1;;\n  is-active) if [ -e \"$state\" ]; then echo inactive; exit 3; else touch \"$state\"; echo deactivating; exit 3; fi;;\nesac\nexit 2",
        );
        let mut process = RunningProcess::spawn(Command::new("/bin/true"), probe_limits()).unwrap();
        assert!(!await_scope_ownership(&mut process, &pin, "asb-short", "nonce", 1).unwrap());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unowned_live_process_is_cancelled_at_ownership_deadline() {
        let _guard = FAKE_TOOL_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (root, pin) = fake_systemctl("ownership-timeout", "exit 1");
        let mut command = Command::new("/usr/bin/sleep");
        command.arg("30");
        let mut process = RunningProcess::spawn(command, probe_limits()).unwrap();
        let start = Instant::now();
        assert!(matches!(
            await_scope_ownership(&mut process, &pin, "asb-unowned", "nonce", 1),
            Err(SandboxError::ScopeOwnership {
                exit_code: None,
                ref stderr,
            }) if stderr.contains("deadline")
        ));
        assert!(start.elapsed() < Duration::from_secs(3));
        assert_eq!(process.lifecycle(), ProcessLifecycle::Terminal);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn scope_cleanup_waits_for_inactive_and_rejects_unknown_state() {
        let _guard = FAKE_TOOL_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (root, pin) = fake_systemctl(
            "cleanup-transition",
            "case \"$2\" in\n  is-active) if [ -e \"$state.stop\" ]; then if [ -e \"$state.polled\" ]; then echo inactive; exit 4; else touch \"$state.polled\"; echo deactivating; exit 3; fi; else echo active; exit 0; fi;;\n  kill) exit 0;;\n  --no-block) touch \"$state.stop\"; exit 0;;\nesac\nexit 2",
        );
        assert!(stop_scope(&pin, "unit").is_ok());
        fs::remove_dir_all(root).unwrap();

        let (root, pin) = fake_systemctl(
            "cleanup-unknown",
            "case \"$2\" in\n  is-active) if [ -e \"$state\" ]; then echo mystery; exit 2; else echo active; exit 0; fi;;\n  kill) exit 0;;\n  --no-block) touch \"$state\"; exit 0;;\nesac\nexit 2",
        );
        let result = stop_scope(&pin, "unit");
        assert!(
            matches!(
                result,
                Err(SandboxError::ScopeCleanup {
                    exit_code: Some(2),
                    ..
                })
            ),
            "unexpected cleanup result: {result:?}"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn scope_cleanup_has_a_hard_poll_deadline() {
        let _guard = FAKE_TOOL_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (root, pin) = fake_systemctl(
            "cleanup-timeout",
            "case \"$2\" in\n  is-active) echo active; exit 0;;\n  kill|--no-block) exit 0;;\nesac\nexit 2",
        );
        let start = Instant::now();
        let result = stop_scope_with_timeout(&pin, "unit", Duration::from_millis(100));
        assert!(
            matches!(
                result,
                Err(SandboxError::ScopeCleanup {
                    exit_code: None,
                    ref stderr,
                }) if stderr.contains("deadline")
            ),
            "unexpected cleanup result: {result:?}"
        );
        assert!(start.elapsed() < Duration::from_secs(2));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cleanup_uncertainty_quarantines_lease_and_drop_retries_terminal_scope() {
        let _guard = FAKE_TOOL_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let body = concat!(
            "case \"$2\" in\n",
            "  is-active) echo active; exit 0;;\n",
            "  kill) echo attempt >> \"$state\"; echo kill-rejected >&2; exit 2;;\n",
            "esac\nexit 2",
        );
        let (tool_root, pin) = fake_systemctl("cleanup-quarantine", body);
        let lease_root = scratch("cleanup-quarantine-lease");
        let _ = fs::remove_dir_all(&lease_root);
        fs::create_dir_all(&lease_root).unwrap();
        let lease = ResourceLease::acquire(
            &lease_root,
            LeaseClass::Benchmark,
            CpuSet::new(vec![0]).unwrap(),
        )
        .unwrap();
        let command = Command::new("/bin/true");
        let mut process = RunningProcess::spawn(command, probe_limits()).unwrap();
        process.wait().unwrap();
        let mut sandbox = SandboxProcess {
            process,
            lease,
            unit: "asb-quarantine-test".into(),
            systemctl: pin,
            scope_cleanup_required: true,
        };
        assert!(matches!(
            sandbox.wait(),
            Err(SandboxError::ScopeCleanup { .. })
        ));
        assert_eq!(fs::read_dir(&lease_root).unwrap().count(), 1);
        drop(sandbox);
        assert_eq!(fs::read_dir(&lease_root).unwrap().count(), 1);
        let attempts = fs::read_to_string(tool_root.join("systemctl.state")).unwrap();
        assert_eq!(attempts.lines().count(), 2);
        fs::remove_dir_all(tool_root).unwrap();
        fs::remove_dir_all(lease_root).unwrap();
    }

    #[test]
    fn live_preexisting_scope_collision_is_never_stopped() {
        let observed_pin = |path: &str| -> Option<ToolPin> {
            let output = Command::new(path).arg("--version").output().ok()?;
            if !output.status.success() {
                return None;
            }
            let version = String::from_utf8_lossy(&output.stdout)
                .lines()
                .next()?
                .to_owned();
            ToolPin::new(PathBuf::from(path), version).ok()
        };
        let Some(bubblewrap) = observed_pin("/usr/bin/bwrap") else {
            return;
        };
        let Some(systemd_run) = observed_pin("/usr/bin/systemd-run") else {
            return;
        };
        let Some(systemctl) = observed_pin("/usr/bin/systemctl") else {
            return;
        };
        let Some(taskset) = observed_pin("/usr/bin/taskset") else {
            return;
        };
        let backend =
            SandboxBackend::new(bubblewrap, systemd_run.clone(), systemctl.clone(), taskset);
        if backend.probe().is_err() {
            return;
        }

        let unit = format!("asb-collision-{}", std::process::id());
        let mut existing = Command::new(&systemd_run.path)
            .args([
                "--user",
                "--scope",
                "--quiet",
                "--collect",
                &format!("--unit={unit}.scope"),
                "/usr/bin/sleep",
                "30",
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while !Command::new(&systemctl.path)
            .args(["--user", "--quiet", "is-active", &format!("{unit}.scope")])
            .status()
            .is_ok_and(|status| status.success())
        {
            assert!(Instant::now() < deadline, "collision scope did not start");
            std::thread::sleep(Duration::from_millis(10));
        }

        let root = scratch("scope-collision");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("work")).unwrap();
        fs::create_dir(root.join("leases")).unwrap();
        let allowed = fs::read_to_string("/proc/self/status")
            .unwrap()
            .lines()
            .find_map(|line| line.strip_prefix("Cpus_allowed_list:\t"))
            .unwrap()
            .split([',', '-'])
            .next()
            .unwrap()
            .parse()
            .unwrap();
        let resources = Resources::new(
            64 * 1024 * 1024,
            16,
            100,
            CpuSet::new(vec![allowed]).unwrap(),
        )
        .unwrap();
        let lease = ResourceLease::acquire(
            &root.join("leases"),
            LeaseClass::Benchmark,
            resources.cpus.clone(),
        )
        .unwrap();
        let spec = SandboxSpec::new(
            &root,
            PathBuf::from("work"),
            "/usr/bin/sleep".into(),
            vec!["1".into()],
            BTreeMap::new(),
            resources,
            NetworkPolicy::Deny,
        )
        .unwrap();
        assert!(matches!(
            backend.spawn_with_identity(
                spec,
                lease,
                probe_limits(),
                unit.clone(),
                "different-owner-nonce".into(),
            ),
            Err(SandboxError::ScopeOwnership { .. })
        ));
        assert!(
            Command::new(&systemctl.path)
                .args(["--user", "--quiet", "is-active", &format!("{unit}.scope")])
                .status()
                .is_ok_and(|status| status.success()),
            "collision handling stopped the pre-existing scope"
        );
        assert_eq!(fs::read_dir(root.join("leases")).unwrap().count(), 1);

        let _ = Command::new(&systemctl.path)
            .args([
                "--user",
                "kill",
                "--kill-whom=all",
                "--signal=KILL",
                &format!("{unit}.scope"),
            ])
            .status();
        let _ = Command::new(&systemctl.path)
            .args(["--user", "--no-block", "stop", &format!("{unit}.scope")])
            .stderr(std::process::Stdio::null())
            .status();
        let _ = existing.wait();
        fs::remove_dir_all(root).unwrap();
    }
}
