// SPDX-License-Identifier: MIT
//! Native rootless namespace and delegated cgroup boundary tests.

use asb_runtime::sandbox::{
    CpuSet, LeaseClass, NetworkPolicy, ResourceLease, Resources, SandboxBackend, SandboxError,
    SandboxSpec, ToolPin,
};
use asb_runtime::{ProcessLifecycle, ProcessLimits, Termination};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

const BWRAP_VERSION: &str = "bubblewrap 0.9.0";
const SYSTEMD_VERSION: &str = "systemd 255 (255.4-1ubuntu8.17)";
const TASKSET_VERSION: &str = "taskset from util-linux 2.39.3";

fn limits() -> ProcessLimits {
    ProcessLimits::new(
        64 * 1024,
        64 * 1024,
        Duration::from_secs(15),
        Duration::from_millis(250),
        Duration::from_millis(5),
    )
    .unwrap()
}

fn target_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("target")
}

fn test_root(name: &str) -> PathBuf {
    let path = target_root().join(format!("sandbox-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(path.join("work")).unwrap();
    fs::create_dir(path.join("leases")).unwrap();
    path
}

fn first_allowed_cpu() -> u32 {
    let status = fs::read_to_string("/proc/self/status").unwrap();
    let list = status
        .lines()
        .find_map(|line| line.strip_prefix("Cpus_allowed_list:\t"))
        .unwrap();
    list.split([',', '-']).next().unwrap().parse().unwrap()
}

fn native_backend() -> Option<SandboxBackend> {
    let backend = SandboxBackend::new(
        ToolPin::new(PathBuf::from("/usr/bin/bwrap"), BWRAP_VERSION.into()).ok()?,
        ToolPin::new(
            PathBuf::from("/usr/bin/systemd-run"),
            SYSTEMD_VERSION.into(),
        )
        .ok()?,
        ToolPin::new(PathBuf::from("/usr/bin/systemctl"), SYSTEMD_VERSION.into()).ok()?,
        ToolPin::new(PathBuf::from("/usr/bin/taskset"), TASKSET_VERSION.into()).ok()?,
    );
    match backend.probe() {
        Ok(()) => Some(backend),
        Err(error) => {
            eprintln!("native sandbox capability unavailable: {error}");
            None
        }
    }
}

fn helper_program(workspace: &Path) -> String {
    let executable = fs::canonicalize(env::current_exe().unwrap()).unwrap();
    let relative = executable
        .strip_prefix(fs::canonicalize(workspace).unwrap())
        .unwrap();
    format!("/workspace/{}", relative.display())
}

fn spec(
    root: &Path,
    _name: &str,
    action: &str,
    resources: Resources,
    extra: BTreeMap<String, String>,
) -> SandboxSpec {
    let workspace = root.parent().unwrap();
    let mut environment = extra;
    environment.insert("ASB_SANDBOX_ACTION".into(), action.into());
    SandboxSpec::new(
        workspace,
        PathBuf::from(root.file_name().unwrap()).join("work"),
        helper_program(workspace),
        vec![
            "--exact".into(),
            "sandbox_helper".into(),
            "--nocapture".into(),
        ],
        environment,
        resources,
        NetworkPolicy::Deny,
    )
    .unwrap()
}

fn resources(tasks: u32) -> Resources {
    Resources::new(
        96 * 1024 * 1024,
        tasks,
        25,
        CpuSet::new(vec![first_allowed_cpu()]).unwrap(),
    )
    .unwrap()
}

fn lease(root: &Path, resources: &Resources) -> ResourceLease {
    ResourceLease::acquire(
        &root.join("leases"),
        LeaseClass::Benchmark,
        resources.cpus().clone(),
    )
    .unwrap()
}

#[test]
fn sandbox_helper() {
    let Ok(action) = env::var("ASB_SANDBOX_ACTION") else {
        return;
    };
    match action.as_str() {
        "isolate" => {
            fs::write("inside.txt", b"inside").unwrap();
            assert!(fs::write("/usr/asb-must-not-write", b"x").is_err());
            assert!(!Path::new(&env::var("ASB_HOST_SENTINEL").unwrap()).exists());
            let address: SocketAddr = env::var("ASB_HOST_LISTENER").unwrap().parse().unwrap();
            assert!(TcpStream::connect_timeout(&address, Duration::from_millis(200)).is_err());
            println!("isolated");
        }
        "sleep" => thread::sleep(Duration::from_secs(30)),
        "memory" => {
            let mut chunks = Vec::new();
            for value in 0_u8..64 {
                let chunk = vec![value; 8 * 1024 * 1024];
                std::hint::black_box(chunk.iter().map(|byte| u64::from(*byte)).sum::<u64>());
                chunks.push(chunk);
            }
            std::hint::black_box(chunks);
        }
        "pids" => {
            let mut children = Vec::new();
            let mut failures = 0_u32;
            for _ in 0..64 {
                match Command::new("/usr/bin/sleep").arg("1").spawn() {
                    Ok(child) => children.push(child),
                    Err(_) => failures += 1,
                }
            }
            for mut child in children {
                let _ = child.wait();
            }
            println!("failures={failures}");
            assert!(failures > 0);
        }
        _ => panic!("unknown helper action"),
    }
}

#[test]
fn native_workspace_and_network_are_isolated() {
    let Some(backend) = native_backend() else {
        return;
    };
    let root = test_root("isolation");
    let sentinel = target_root()
        .parent()
        .unwrap()
        .join(format!("outside-{}", std::process::id()));
    fs::write(&sentinel, b"host").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let r = resources(16);
    let mut extra = BTreeMap::new();
    extra.insert("ASB_HOST_SENTINEL".into(), sentinel.display().to_string());
    extra.insert(
        "ASB_HOST_LISTENER".into(),
        listener.local_addr().unwrap().to_string(),
    );
    let mut process = backend
        .spawn(
            spec(&root, "isolation", "isolate", r.clone(), extra),
            lease(&root, &r),
            limits(),
        )
        .unwrap();
    let output = process.wait().unwrap();
    assert_eq!(output.exit_code, Some(0));
    assert!(root.join("work/inside.txt").is_file());
    assert_eq!(fs::read(&sentinel).unwrap(), b"host");
    let mut connection = None;
    let deadline = Instant::now() + Duration::from_millis(300);
    while Instant::now() < deadline {
        if let Ok(value) = listener.accept() {
            connection = Some(value);
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    assert!(connection.is_none(), "sandbox reached host loopback");
    fs::remove_file(sentinel).unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn short_process_completes_without_ambiguous_scope_ownership() {
    let Some(backend) = native_backend() else {
        return;
    };
    let root = test_root("short");
    let r = resources(8);
    let request = SandboxSpec::new(
        root.parent().unwrap(),
        PathBuf::from(root.file_name().unwrap()).join("work"),
        "/usr/bin/true".into(),
        vec![],
        BTreeMap::new(),
        r.clone(),
        NetworkPolicy::Deny,
    )
    .unwrap();
    let mut process = backend.spawn(request, lease(&root, &r), limits()).unwrap();
    assert_eq!(process.wait().unwrap().exit_code, Some(0));
    drop(process);
    assert_eq!(fs::read_dir(root.join("leases")).unwrap().count(), 0);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn native_cgroup_limits_and_cleanup_are_real() {
    let Some(backend) = native_backend() else {
        return;
    };
    let root = test_root("limits");
    let r = resources(12);
    let request = SandboxSpec::new(
        root.parent().unwrap(),
        PathBuf::from(root.file_name().unwrap()).join("work"),
        helper_program(root.parent().unwrap()),
        vec![
            "--exact".into(),
            "sandbox_helper".into(),
            "--nocapture".into(),
        ],
        BTreeMap::from([("ASB_SANDBOX_ACTION".into(), "sleep".into())]),
        r.clone(),
        NetworkPolicy::Deny,
    )
    .unwrap();
    let mut process = backend.spawn(request, lease(&root, &r), limits()).unwrap();
    let unit = process.unit().to_owned();
    assert!(unit.starts_with("asb-"));
    assert_eq!(process.lifecycle(), ProcessLifecycle::Running);
    let deadline = Instant::now() + Duration::from_secs(3);
    let cgroup = loop {
        let output = Command::new("/usr/bin/systemctl")
            .args([
                "--user",
                "show",
                &format!("{unit}.scope"),
                "--property=ControlGroup",
                "--value",
            ])
            .output()
            .unwrap();
        let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if !value.is_empty() {
            break value;
        }
        assert!(Instant::now() < deadline, "scope did not become observable");
        thread::sleep(Duration::from_millis(10));
    };
    let cgroup = Path::new("/sys/fs/cgroup").join(cgroup.trim_start_matches('/'));
    assert_eq!(
        fs::read_to_string(cgroup.join("memory.max"))
            .unwrap()
            .trim(),
        r.memory().to_string()
    );
    assert_eq!(
        fs::read_to_string(cgroup.join("memory.swap.max"))
            .unwrap()
            .trim(),
        "0"
    );
    assert_eq!(
        fs::read_to_string(cgroup.join("pids.max")).unwrap().trim(),
        r.tasks().to_string()
    );
    let expected_cpu = r.cpus().as_slice()[0].to_string();
    let affinity_deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let mut restricted = 0_u32;
        for pid in fs::read_to_string(cgroup.join("cgroup.procs"))
            .unwrap()
            .lines()
        {
            let status = fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
            let allowed = status
                .lines()
                .find_map(|line| line.strip_prefix("Cpus_allowed_list:\t"))
                .unwrap();
            if allowed == expected_cpu {
                restricted += 1;
            }
        }
        if restricted >= 2 {
            break;
        }
        assert!(
            Instant::now() < affinity_deadline,
            "sandbox processes were not CPU-pinned"
        );
        thread::sleep(Duration::from_millis(10));
    }
    let cpu = fs::read_to_string(cgroup.join("cpu.max")).unwrap();
    let values: Vec<u64> = cpu.split_whitespace().map(|v| v.parse().unwrap()).collect();
    assert_eq!(values[0] * 100, values[1] * u64::from(r.cpu_percent()));
    process.cancel().unwrap();
    assert_eq!(process.wait().unwrap().termination, Termination::Cancelled);
    assert_eq!(process.lifecycle(), ProcessLifecycle::Terminal);
    process.cancel().unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while cgroup.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(!cgroup.exists(), "transient cgroup leaked");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn dropping_sandbox_stops_scope_and_releases_lease() {
    let Some(backend) = native_backend() else {
        return;
    };
    let root = test_root("drop-cleanup");
    let r = resources(12);
    let request = SandboxSpec::new(
        root.parent().unwrap(),
        PathBuf::from(root.file_name().unwrap()).join("work"),
        helper_program(root.parent().unwrap()),
        vec![
            "--exact".into(),
            "sandbox_helper".into(),
            "--nocapture".into(),
        ],
        BTreeMap::from([("ASB_SANDBOX_ACTION".into(), "sleep".into())]),
        r.clone(),
        NetworkPolicy::Deny,
    )
    .unwrap();
    let process = backend.spawn(request, lease(&root, &r), limits()).unwrap();
    let unit = process.unit().to_owned();
    assert_eq!(process.lifecycle(), ProcessLifecycle::Running);
    drop(process);

    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let present = Command::new("/usr/bin/systemctl")
            .args(["--user", "--quiet", "is-active", &format!("{unit}.scope")])
            .status()
            .is_ok_and(|status| status.success());
        if !present {
            break;
        }
        assert!(Instant::now() < deadline, "dropped sandbox scope leaked");
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(fs::read_dir(root.join("leases")).unwrap().count(), 0);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cleanup_failure_retains_lease_until_drop_retry_cleans_descendant() {
    let root = test_root("cleanup-retry");
    let reject = root.join("reject-cleanup");
    let wrapper = root.join("systemctl-wrapper");
    fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nif [ \"$1\" = --version ]; then exec /usr/bin/systemctl --version; fi\nif [ -e \"{}\" ] && [ \"$2\" = is-active ]; then echo forced-cleanup-failure >&2; exit 2; fi\nexec /usr/bin/systemctl \"$@\"\n",
            reject.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).unwrap();
    let (Ok(bubblewrap), Ok(systemd_run), Ok(systemctl), Ok(taskset)) = (
        ToolPin::new(PathBuf::from("/usr/bin/bwrap"), BWRAP_VERSION.into()),
        ToolPin::new(
            PathBuf::from("/usr/bin/systemd-run"),
            SYSTEMD_VERSION.into(),
        ),
        ToolPin::new(wrapper, SYSTEMD_VERSION.into()),
        ToolPin::new(PathBuf::from("/usr/bin/taskset"), TASKSET_VERSION.into()),
    ) else {
        fs::remove_dir_all(root).unwrap();
        return;
    };
    let backend = SandboxBackend::new(bubblewrap, systemd_run, systemctl, taskset);
    if backend.probe().is_err() {
        fs::remove_dir_all(root).unwrap();
        return;
    }
    let r = resources(12);
    let mut process = backend
        .spawn(
            spec(&root, "cleanup-retry", "sleep", r.clone(), BTreeMap::new()),
            lease(&root, &r),
            limits(),
        )
        .unwrap();
    let unit = process.unit().to_owned();
    let control = Command::new("/usr/bin/systemctl")
        .args([
            "--user",
            "show",
            &format!("{unit}.scope"),
            "--property=ControlGroup",
            "--value",
        ])
        .output()
        .unwrap();
    let control = String::from_utf8_lossy(&control.stdout).trim().to_owned();
    let cgroup = Path::new("/sys/fs/cgroup").join(control.trim_start_matches('/'));
    assert!(
        fs::read_to_string(cgroup.join("cgroup.procs")).is_ok_and(|value| !value.trim().is_empty()),
        "sandbox did not start a real cgroup descendant"
    );

    fs::write(&reject, b"reject").unwrap();
    assert!(matches!(
        process.cancel(),
        Err(SandboxError::ScopeCleanup { .. })
    ));
    assert_eq!(fs::read_dir(root.join("leases")).unwrap().count(), 1);
    fs::remove_file(reject).unwrap();
    drop(process);
    let deadline = Instant::now() + Duration::from_secs(3);
    while cgroup.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(!cgroup.exists(), "drop retry left the cgroup behind");
    assert_eq!(fs::read_dir(root.join("leases")).unwrap().count(), 0);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn native_memory_pid_and_delegation_fail_closed() {
    let Some(backend) = native_backend() else {
        return;
    };
    for (name, action, tasks) in [("memory", "memory", 16), ("pids", "pids", 12)] {
        let root = test_root(name);
        let r = resources(tasks);
        let mut process = backend
            .spawn(
                spec(&root, name, action, r.clone(), BTreeMap::new()),
                lease(&root, &r),
                limits(),
            )
            .unwrap();
        let output = process.wait().unwrap();
        if action == "memory" {
            assert_ne!(output.exit_code, Some(0));
        } else {
            assert_eq!(output.exit_code, Some(0));
        }
        fs::remove_dir_all(root).unwrap();
    }
    let root = test_root("reject");
    let fake = root.join("fake-systemd-run");
    fs::write(
        &fake,
        "#!/bin/sh\nif [ \"$1\" = --version ]; then echo fake-v1; exit 0; fi\nexit 1\n",
    )
    .unwrap();
    let mut permissions = fs::metadata(&fake).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&fake, permissions).unwrap();
    let rejected = SandboxBackend::new(
        ToolPin::new(PathBuf::from("/usr/bin/bwrap"), BWRAP_VERSION.into()).unwrap(),
        ToolPin::new(fake, "fake-v1".into()).unwrap(),
        ToolPin::new(PathBuf::from("/usr/bin/systemctl"), SYSTEMD_VERSION.into()).unwrap(),
        ToolPin::new(PathBuf::from("/usr/bin/taskset"), TASKSET_VERSION.into()).unwrap(),
    )
    .probe();
    assert!(matches!(rejected, Err(SandboxError::DelegationRejected)));
    fs::remove_dir_all(root).unwrap();
}
