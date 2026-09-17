// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Native rootless namespace and delegated cgroup boundary tests.

use asb_replay::{
    CassetteLimits, Header, ProviderDialect, ReplayHttpRequest, ReplayRoute, StrictReplayService,
    decode_cassette,
};
use asb_runtime::launch_factory::ReplayLaunchFactory;
use asb_runtime::relay::ReplayRelay;
use asb_runtime::sandbox::{
    CpuSet, LeaseClass, NetworkPolicy, ResourceLease, Resources, SandboxBackend, SandboxError,
    SandboxLaunchInput, SandboxSpec, ToolPin,
};
use asb_runtime::supervisor::{PinnedCommand, SupervisorPlan};
use asb_runtime::{ProcessLifecycle, ProcessLimits, Termination};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};
const BWRAP_VERSION: &str = "bubblewrap 0.9.0";
const SYSTEMD_VERSION: &str = "systemd 255 (255.4-1ubuntu8.17)";
const TASKSET_VERSION: &str = "taskset from util-linux 2.39.3";
static HELPER_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct RemoveFileOnDrop(PathBuf);

impl Drop for RemoveFileOnDrop {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

struct TestRoot(PathBuf);

impl std::ops::Deref for TestRoot {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<Path> for TestRoot {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

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

fn short_limits() -> ProcessLimits {
    ProcessLimits::new(
        64 * 1024,
        64 * 1024,
        Duration::from_millis(100),
        Duration::from_millis(100),
        Duration::from_millis(5),
    )
    .unwrap()
}

fn resolve_target_root(value: Option<std::ffi::OsString>, current: &Path) -> PathBuf {
    let Some(value) = value else {
        return env::temp_dir();
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

fn target_root() -> PathBuf {
    resolve_target_root(
        env::var_os("CARGO_TARGET_DIR"),
        &env::current_dir().unwrap(),
    )
}

fn test_root(name: &str) -> TestRoot {
    let sequence = HELPER_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let path = target_root().join(format!("sandbox-{name}-{}-{sequence}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(path.join("work")).unwrap();
    fs::create_dir(path.join("leases")).unwrap();
    TestRoot(path)
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
    env::var_os("ASB_REQUIRE_NATIVE_SANDBOX")?;
    fn unavailable(error: impl std::fmt::Display) -> Option<SandboxBackend> {
        if env::var_os("ASB_REQUIRE_NATIVE_SANDBOX").is_some() {
            panic!("required native sandbox capability unavailable: {error}");
        }
        eprintln!("native sandbox capability unavailable: {error}");
        None
    }
    let bubblewrap = match ToolPin::new(PathBuf::from("/usr/bin/bwrap"), BWRAP_VERSION.into()) {
        Ok(value) => value,
        Err(error) => return unavailable(error),
    };
    let systemd_run = match ToolPin::new(
        PathBuf::from("/usr/bin/systemd-run"),
        SYSTEMD_VERSION.into(),
    ) {
        Ok(value) => value,
        Err(error) => return unavailable(error),
    };
    let systemctl = match ToolPin::new(PathBuf::from("/usr/bin/systemctl"), SYSTEMD_VERSION.into())
    {
        Ok(value) => value,
        Err(error) => return unavailable(error),
    };
    let taskset = match ToolPin::new(PathBuf::from("/usr/bin/taskset"), TASKSET_VERSION.into()) {
        Ok(value) => value,
        Err(error) => return unavailable(error),
    };
    let backend = SandboxBackend::new(bubblewrap, systemd_run, systemctl, taskset);
    match backend.probe() {
        Ok(()) => Some(backend),
        Err(error) => unavailable(error),
    }
}

fn stage_helper(root: &Path, executable: &Path) -> String {
    let work = root.join("work");
    let destination = work.join("asb-sandbox-helper");
    let temporary = work.join(format!(
        ".asb-sandbox-helper-{}-{}.tmp",
        std::process::id(),
        HELPER_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::copy(executable, &temporary).unwrap();
    fs::set_permissions(&temporary, fs::Permissions::from_mode(0o700)).unwrap();
    fs::File::open(&temporary).unwrap().sync_all().unwrap();
    fs::rename(&temporary, &destination).unwrap();
    fs::File::open(&work).unwrap().sync_all().unwrap();
    format!(
        "/workspace/{}/work/asb-sandbox-helper",
        root.file_name().unwrap().to_string_lossy()
    )
}

fn helper_program(root: &Path) -> String {
    stage_helper(
        root,
        &fs::canonicalize(env::current_exe().unwrap()).unwrap(),
    )
}

fn file_sha256(path: &Path) -> String {
    let mut file = fs::File::open(path).unwrap();
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).unwrap();
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    format!("{:x}", digest.finalize())
}

fn spec(
    root: &Path,
    _name: &str,
    action: &str,
    resources: Resources,
    extra: BTreeMap<String, String>,
) -> SandboxSpec {
    let mut environment = extra;
    environment.insert("ASB_SANDBOX_ACTION".into(), action.into());
    SandboxSpec::new(
        root.parent().unwrap(),
        PathBuf::from(root.file_name().unwrap()).join("work"),
        helper_program(root),
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

#[test]
fn current_test_executable_is_staged_inside_the_workspace() {
    let root = test_root("external-target");
    let executable = fs::canonicalize(env::current_exe().unwrap()).unwrap();
    if env::var_os("ASB_REQUIRE_EXTERNAL_TARGET").is_some() {
        let checkout = fs::canonicalize(env!("CARGO_MANIFEST_DIR"))
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_owned();
        assert!(
            !executable.starts_with(checkout),
            "test executable was not in the required external target"
        );
    }
    let sandbox_path = helper_program(&root);
    let host_path = root
        .parent()
        .unwrap()
        .join(sandbox_path.strip_prefix("/workspace/").unwrap());
    assert!(host_path.starts_with(root.join("work")));
    assert_eq!(
        fs::read(&host_path).unwrap(),
        fs::read(&executable).unwrap()
    );
    assert!(
        Command::new(&host_path)
            .args(["--exact", "sandbox_helper"])
            .env_remove("ASB_SANDBOX_ACTION")
            .status()
            .unwrap()
            .success()
    );
}

#[test]
fn host_sentinel_is_removed_during_panic_unwind() {
    let root = test_root("panic-sentinel");
    let sentinel = root.join("host");
    let root_path = root.to_path_buf();
    fs::write(&sentinel, b"host").unwrap();
    let result = std::panic::catch_unwind({
        let sentinel = sentinel.clone();
        move || {
            let _cleanup = RemoveFileOnDrop(sentinel);
            panic!("controlled fixture panic");
        }
    });
    assert!(result.is_err());
    assert!(!sentinel.exists());
    drop(root);
    assert!(!root_path.exists());
}

#[test]
fn relative_target_roots_are_normalized_before_classification() {
    let current = env::current_dir().unwrap();
    let repository_target = current.join("target");
    let sequence = HELPER_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let alias_name = format!(".asb-target-alias-{}-{sequence}", std::process::id());
    let alias_path = current.join(&alias_name);
    fs::create_dir(&alias_path).unwrap();
    let alias_cleanup = TestRoot(alias_path.clone());
    assert_eq!(
        resolve_target_root(Some("target".into()), &current),
        repository_target
    );
    assert_eq!(
        resolve_target_root(Some(format!("{alias_name}/../target").into()), &current),
        repository_target
    );
    assert_eq!(
        resolve_target_root(Some("../external-target".into()), &current),
        current.parent().unwrap().join("external-target")
    );

    let symlink_name = format!(".asb-target-symlink-{}-{sequence}", std::process::id());
    let symlink_path = current.join(&symlink_name);
    std::os::unix::fs::symlink(&current, &symlink_path).unwrap();
    let symlink_cleanup = RemoveFileOnDrop(symlink_path);
    assert_eq!(
        resolve_target_root(Some(format!("{symlink_name}/target").into()), &current),
        repository_target
    );
    drop(symlink_cleanup);
    drop(alias_cleanup);
    assert!(!alias_path.exists());
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
    let sentinel = target_root().join(format!(
        "outside-{}-{}",
        std::process::id(),
        HELPER_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(&sentinel, b"host").unwrap();
    let _sentinel_cleanup = RemoveFileOnDrop(sentinel.clone());
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
    let input = SandboxLaunchInput::new(request, limits()).unwrap();
    let mut process = backend.spawn_launch(input, lease(&root, &r)).unwrap();
    assert_eq!(process.wait().unwrap().exit_code, Some(0));
    drop(process);
    assert_eq!(fs::read_dir(root.join("leases")).unwrap().count(), 0);
}

#[test]
fn launch_wrapper_timeout_and_crash_are_terminal() {
    let Some(backend) = native_backend() else {
        return;
    };
    for (name, action) in [("launch-timeout", "sleep"), ("launch-crash", "crash")] {
        let root = test_root(name);
        let r = resources(8);
        let input = SandboxLaunchInput::new(
            spec(&root, name, action, r.clone(), BTreeMap::new()),
            if action == "sleep" {
                short_limits()
            } else {
                limits()
            },
        )
        .unwrap();
        if action == "sleep" {
            let mut process = backend.spawn_launch(input, lease(&root, &r)).unwrap();
            let output = process.wait().unwrap();
            assert_eq!(output.termination, Termination::TimedOut);
            assert_eq!(process.lifecycle(), ProcessLifecycle::Terminal);
        } else {
            match backend.spawn_launch(input, lease(&root, &r)) {
                Err(SandboxError::ScopeOwnership { .. }) => {}
                Ok(mut process) => {
                    let output = process.wait().unwrap();
                    assert_ne!(output.exit_code, Some(0));
                    assert_eq!(process.lifecycle(), ProcessLifecycle::Terminal);
                }
                Err(error) => panic!("unexpected crash launch result: {error:?}"),
            }
        }
    }
}

#[test]
fn native_supervisor_forwards_cassette_http_and_reaps_children() {
    let Some(backend) = native_backend() else {
        return;
    };
    let root = test_root("supervisor-cassette");
    let generation = "native-cassette-generation";
    let mut relay = ReplayRelay::bind(&root, generation).unwrap();
    let handoff = relay.handoff("/tmp/asb-replay-relay.sock").unwrap();
    let cassette_bytes =
        include_bytes!("../../asb-replay/fixtures/v1/gemini-generate-content.json");
    let cassette = decode_cassette(cassette_bytes, CassetteLimits::default()).unwrap();
    let request_body =
        serde_json::to_string(&cassette.contents.interactions[0].request.body).unwrap();
    let service = StrictReplayService::new(cassette, Default::default()).unwrap();
    let route = ReplayRoute {
        session_id: "gemini-public-session".into(),
        attempt_id: "attempt-1".into(),
        dialect: ProviderDialect::GeminiGenerateContent,
    };
    let probe_service = StrictReplayService::new(
        decode_cassette(cassette_bytes, CassetteLimits::default()).unwrap(),
        Default::default(),
    )
    .unwrap();
    let probe_response = probe_service.handle(
        &route,
        ReplayHttpRequest {
            method: "POST".into(),
            path: "/v1beta/models/fixture-model:streamGenerateContent?alt=sse".into(),
            headers: vec![
                Header {
                    name: "content-type".into(),
                    value: "application/json".into(),
                },
                Header {
                    name: "x-goog-api-key".into(),
                    value: "fixture-key".into(),
                },
            ],
            body: request_body.as_bytes().to_vec(),
        },
    );
    assert!(
        probe_response.is_ok(),
        "fixture request did not match: {probe_response:?}"
    );
    let server = thread::spawn(move || -> Result<u16, String> {
        let mut stream = relay
            .accept_authenticated()
            .map_err(|error| format!("relay authentication failed: {error}"))?;
        service
            .serve_authenticated_connection(&mut stream, &route)
            .map_err(|error| format!("strict cassette service failed: {error}"))
    });
    let executable_dir = fs::canonicalize(env::current_exe().unwrap()).unwrap();
    let executable_dir = executable_dir
        .parent()
        .and_then(|path| {
            (path.file_name().and_then(|name| name.to_str()) == Some("deps"))
                .then(|| path.parent().unwrap())
        })
        .unwrap_or_else(|| executable_dir.parent().unwrap());
    let sidecar_path = executable_dir.join("asb_loopback_sidecar");
    let supervisor_path = executable_dir.join("asb_loopback_supervisor");
    assert!(sidecar_path.is_file(), "pinned sidecar fixture is missing");
    assert!(
        supervisor_path.is_file(),
        "pinned supervisor fixture is missing"
    );
    assert!(
        Path::new("/usr/bin/curl").is_file(),
        "pinned curl fixture is missing"
    );
    let port_probe = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = port_probe.local_addr().unwrap().port();
    drop(port_probe);
    let sidecar = PinnedCommand::new_verified(
        sidecar_path.clone(),
        vec![
            "--listen".into(),
            format!("127.0.0.1:{port}"),
            "--relay".into(),
            "/tmp/asb-replay-relay.sock".into(),
            "--generation".into(),
            generation.into(),
        ],
        &file_sha256(&sidecar_path),
    )
    .unwrap();
    let adapter = PinnedCommand::new_verified(
        PathBuf::from("/usr/bin/curl"),
        vec![
            "--fail".into(),
            "--silent".into(),
            "--header".into(),
            "accept:".into(),
            "--header".into(),
            "content-type: application/json".into(),
            "--header".into(),
            "x-goog-api-key: fixture-key".into(),
            "--data-binary".into(),
            request_body,
            format!(
                "http://127.0.0.1:{port}/v1beta/models/fixture-model:streamGenerateContent?alt=sse"
            ),
        ],
        &file_sha256(Path::new("/usr/bin/curl")),
    )
    .unwrap();
    let supervisor = PinnedCommand::new_verified(
        supervisor_path.clone(),
        Vec::new(),
        &file_sha256(&supervisor_path),
    )
    .unwrap();
    let plan = SupervisorPlan::new(
        sidecar,
        adapter,
        handoff.socket_path().to_path_buf(),
        generation.into(),
        file_sha256(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../asb-replay/fixtures/v1/gemini-generate-content.json")
                .as_path(),
        ),
        Duration::from_secs(10),
    )
    .unwrap()
    .with_supervisor(supervisor);
    let resources = Resources::new(
        128 * 1024 * 1024,
        16,
        100,
        CpuSet::new(vec![first_allowed_cpu()]).unwrap(),
    )
    .unwrap();
    let lease = ResourceLease::acquire(
        &root.join("leases"),
        LeaseClass::Benchmark,
        resources.cpus().clone(),
    )
    .unwrap();
    let sandbox = SandboxSpec::new(
        &root,
        PathBuf::from("work"),
        "/bin/true".into(),
        Vec::new(),
        BTreeMap::new(),
        resources,
        NetworkPolicy::Deny,
    )
    .unwrap()
    .with_supervisor(plan);
    let input = SandboxLaunchInput::new(sandbox, limits())
        .unwrap()
        .with_replay_handoff(handoff)
        .unwrap();
    let cassette_digest = file_sha256(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../asb-replay/fixtures/v1/gemini-generate-content.json")
            .as_path(),
    );
    let token = backend
        .attest_replay_launch(&input, &lease, &cassette_digest)
        .unwrap();
    let authority =
        ReplayLaunchFactory::issue(token, input, lease, cassette_digest.clone()).unwrap();
    let context = authority.consume_for(&cassette_digest).unwrap();
    let mut process = context.spawn(&backend).unwrap();
    let output = process.wait().unwrap().clone();
    let server_result = server.join().unwrap();
    assert!(
        server_result == Ok(200),
        "relay failure: {server_result:?}; child output: {output:?}"
    );
    assert_eq!(output.exit_code, Some(0));
    assert_eq!(output.termination, Termination::Exited);
    assert!(String::from_utf8_lossy(&output.stdout.bytes).contains("done"));
    assert!(!root.join("asb-replay-relay.sock").exists());
}

#[test]
fn native_supervisor_authenticated_negative_matrix_has_no_fallback() {
    if env::var_os("ASB_REQUIRE_NATIVE_SANDBOX").is_none() {
        return;
    }
    let Some(backend) = native_backend() else {
        return;
    };
    for mode in ["stale", "malformed", "duplicate", "mismatch"] {
        let mode = mode.to_owned();
        let root_name = format!("supervisor-authenticated-{mode}");
        let root = test_root(&root_name);
        let generation = format!("authenticated-negative-{mode}");
        let mut relay = ReplayRelay::bind(&root, &generation).unwrap();
        let handoff = relay.handoff("/tmp/asb-replay-relay.sock").unwrap();
        let cassette_bytes =
            include_bytes!("../../asb-replay/fixtures/v1/gemini-generate-content.json");
        let cassette = decode_cassette(cassette_bytes, CassetteLimits::default()).unwrap();
        let request_body =
            serde_json::to_string(&cassette.contents.interactions[0].request.body).unwrap();
        let service = StrictReplayService::new(cassette, Default::default()).unwrap();
        let route = ReplayRoute {
            session_id: "gemini-public-session".into(),
            attempt_id: "attempt-1".into(),
            dialect: ProviderDialect::GeminiGenerateContent,
        };
        let server_mode = mode.clone();
        let server = thread::spawn(move || -> Result<(), String> {
            let first = relay.accept_authenticated();
            match server_mode.as_str() {
                "stale" => matches!(first, Err(asb_runtime::relay::RelayError::StaleGeneration))
                    .then_some(())
                    .ok_or_else(|| "stale generation was accepted".into()),
                "malformed" => {
                    matches!(first, Err(asb_runtime::relay::RelayError::InvalidHandshake))
                        .then_some(())
                        .ok_or_else(|| "malformed handshake was accepted".into())
                }
                "duplicate" => {
                    first.map_err(|error| format!("first authenticated peer failed: {error}"))?;
                    matches!(
                        relay.accept_authenticated(),
                        Err(asb_runtime::relay::RelayError::Duplicate)
                    )
                    .then_some(())
                    .ok_or_else(|| "duplicate authenticated peer was accepted".into())
                }
                "mismatch" => {
                    let mut stream =
                        first.map_err(|error| format!("authenticated peer failed: {error}"))?;
                    let status = service
                        .serve_authenticated_connection(&mut stream, &route)
                        .map_err(|error| format!("strict mismatch service failed: {error}"))?;
                    (status != 200)
                        .then_some(())
                        .ok_or_else(|| "strict route mismatch unexpectedly succeeded".into())
                }
                _ => Err("unknown negative mode".into()),
            }
        });
        let executable_dir = fs::canonicalize(env::current_exe().unwrap()).unwrap();
        let executable_dir = executable_dir
            .parent()
            .and_then(|path| {
                (path.file_name().and_then(|name| name.to_str()) == Some("deps"))
                    .then(|| path.parent().unwrap())
            })
            .unwrap_or_else(|| executable_dir.parent().unwrap());
        let sidecar_path = executable_dir.join("asb_loopback_sidecar");
        let supervisor_path = executable_dir.join("asb_loopback_supervisor");
        assert!(sidecar_path.is_file());
        assert!(supervisor_path.is_file());
        let port_probe = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = port_probe.local_addr().unwrap().port();
        drop(port_probe);
        let mut sidecar_args = vec![
            "--listen".into(),
            format!("127.0.0.1:{port}"),
            "--relay".into(),
            "/tmp/asb-replay-relay.sock".into(),
            "--generation".into(),
            generation.clone(),
        ];
        if mode != "mismatch" {
            sidecar_args.extend(["--handshake-mode".into(), mode.clone()]);
        }
        let sidecar = PinnedCommand::new_verified(
            sidecar_path.clone(),
            sidecar_args,
            &file_sha256(&sidecar_path),
        )
        .unwrap();
        let request_path = if mode == "mismatch" {
            "/wrong-route"
        } else {
            "/v1beta/models/fixture-model:streamGenerateContent?alt=sse"
        };
        let adapter = PinnedCommand::new_verified(
            PathBuf::from("/usr/bin/curl"),
            vec![
                "--fail".into(),
                "--silent".into(),
                "--header".into(),
                "content-type: application/json".into(),
                "--header".into(),
                "x-goog-api-key: fixture-key".into(),
                "--data-binary".into(),
                request_body,
                format!("http://127.0.0.1:{port}{request_path}"),
            ],
            &file_sha256(Path::new("/usr/bin/curl")),
        )
        .unwrap();
        let supervisor = PinnedCommand::new_verified(
            supervisor_path.clone(),
            Vec::new(),
            &file_sha256(&supervisor_path),
        )
        .unwrap();
        let plan = SupervisorPlan::new(
            sidecar,
            adapter,
            handoff.socket_path().to_path_buf(),
            generation,
            "c".repeat(64),
            Duration::from_secs(3),
        )
        .unwrap()
        .with_supervisor(supervisor);
        let resources = Resources::new(
            128 * 1024 * 1024,
            16,
            100,
            CpuSet::new(vec![first_allowed_cpu()]).unwrap(),
        )
        .unwrap();
        let lease = ResourceLease::acquire(
            &root.join("leases"),
            LeaseClass::Benchmark,
            resources.cpus().clone(),
        )
        .unwrap();
        let sandbox = SandboxSpec::new(
            &root,
            PathBuf::from("work"),
            "/bin/true".into(),
            Vec::new(),
            BTreeMap::new(),
            resources,
            NetworkPolicy::Deny,
        )
        .unwrap()
        .with_supervisor(plan);
        let input = SandboxLaunchInput::new(sandbox, limits())
            .unwrap()
            .with_replay_handoff(handoff)
            .unwrap();
        let cassette_digest = file_sha256(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../asb-replay/fixtures/v1/gemini-generate-content.json")
                .as_path(),
        );
        let token = backend
            .attest_replay_launch(&input, &lease, &cassette_digest)
            .unwrap();
        let authority =
            ReplayLaunchFactory::issue(token, input, lease, cassette_digest.clone()).unwrap();
        let context = authority.consume_for(&cassette_digest).unwrap();
        match context.spawn(&backend) {
            Ok(mut process) => {
                let output = process.wait().unwrap().clone();
                assert_ne!(
                    output.exit_code,
                    Some(0),
                    "negative mode {mode} unexpectedly succeeded"
                );
            }
            Err(SandboxError::ScopeOwnership { exit_code, .. }) => {
                assert_ne!(
                    exit_code,
                    Some(0),
                    "negative mode {mode} unexpectedly succeeded"
                );
            }
            Err(error) => {
                panic!("negative mode {mode} failed before supervised execution: {error:?}");
            }
        }
        assert!(
            server.join().unwrap().is_ok(),
            "negative mode {mode} was not rejected"
        );
        assert!(!root.join("asb-replay-relay.sock").exists());
    }
}

#[test]
fn authenticated_replay_boundary_rejects_stale_malformed_duplicate_and_mismatch() {
    let root = test_root("authenticated-negative-boundary");
    let mut relay = ReplayRelay::bind(&root, "negative-generation").unwrap();
    let relay_path = relay.socket_path().to_owned();

    let stale = UnixStream::connect(&relay_path).unwrap();
    let mut stale_writer = stale.try_clone().unwrap();
    stale_writer.write_all(b"stale-generation\n").unwrap();
    drop(stale_writer);
    assert!(matches!(
        relay.accept_authenticated(),
        Err(asb_runtime::relay::RelayError::StaleGeneration)
    ));

    let malformed = UnixStream::connect(&relay_path).unwrap();
    let mut malformed_writer = malformed.try_clone().unwrap();
    malformed_writer.write_all(b"malformed").unwrap();
    malformed.shutdown(std::net::Shutdown::Write).unwrap();
    malformed_writer
        .shutdown(std::net::Shutdown::Write)
        .unwrap();
    drop(malformed_writer);
    assert!(matches!(
        relay.accept_authenticated(),
        Err(asb_runtime::relay::RelayError::InvalidHandshake)
    ));

    let valid = UnixStream::connect(&relay_path).unwrap();
    let mut valid_writer = valid.try_clone().unwrap();
    valid_writer.write_all(b"negative-generation\n").unwrap();
    let accepted = relay.accept_authenticated().unwrap();
    assert!(matches!(
        relay.accept_authenticated(),
        Err(asb_runtime::relay::RelayError::Duplicate)
    ));
    drop(accepted);
    drop(valid_writer);
    drop(valid);

    let cassette = decode_cassette(
        include_bytes!("../../asb-replay/fixtures/v1/gemini-generate-content.json"),
        CassetteLimits::default(),
    )
    .unwrap();
    let service = StrictReplayService::new(cassette, Default::default()).unwrap();
    let mismatch = service.handle(
        &ReplayRoute {
            session_id: "wrong-session".into(),
            attempt_id: "attempt-1".into(),
            dialect: ProviderDialect::GeminiGenerateContent,
        },
        ReplayHttpRequest {
            method: "POST".into(),
            path: "/wrong-route".into(),
            headers: Vec::new(),
            body: b"{}".to_vec(),
        },
    );
    assert!(mismatch.is_err(), "strict mismatch unexpectedly succeeded");
}

#[allow(clippy::too_many_arguments)]
fn run_supervised_fault(
    backend: &SandboxBackend,
    name: &str,
    sidecar_executable: &Path,
    sidecar_arguments: Vec<String>,
    adapter_executable: &Path,
    adapter_arguments: Vec<String>,
    timeout: Duration,
    cancel_after: Option<Duration>,
) -> (Termination, Option<i32>, TestRoot, String) {
    let root = test_root(name);
    let generation = format!("fault-{name}");
    // Use the runtime-owned generation-authenticated relay in every matrix
    // case. A raw UnixListener would let an unrelated child failure look like
    // a valid supervised replay launch.
    let relay = ReplayRelay::bind(&root, &generation).unwrap();
    let relay_path = relay.socket_path().to_owned();
    let executable_dir = fs::canonicalize(env::current_exe().unwrap()).unwrap();
    let supervisor_path = executable_dir
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("asb_loopback_supervisor");
    assert!(supervisor_path.is_file());
    let sidecar = PinnedCommand::new_verified(
        sidecar_executable.to_owned(),
        sidecar_arguments,
        &file_sha256(sidecar_executable),
    )
    .unwrap();
    let adapter = PinnedCommand::new_verified(
        adapter_executable.to_owned(),
        adapter_arguments,
        &file_sha256(adapter_executable),
    )
    .unwrap();
    let supervisor = PinnedCommand::new_verified(
        supervisor_path.clone(),
        Vec::new(),
        &file_sha256(&supervisor_path),
    )
    .unwrap();
    let plan = SupervisorPlan::new(
        sidecar,
        adapter,
        relay_path.clone(),
        generation,
        "c".repeat(64),
        timeout,
    )
    .unwrap()
    .with_supervisor(supervisor);
    let resources = resources(16);
    let lease = lease(&root, &resources);
    let sandbox = SandboxSpec::new(
        &root,
        PathBuf::from("work"),
        "/bin/true".into(),
        Vec::new(),
        BTreeMap::new(),
        resources,
        NetworkPolicy::Deny,
    )
    .unwrap()
    .with_supervisor(plan);
    let result = match backend.spawn(sandbox, lease, limits()) {
        Ok(mut process) => {
            if let Some(delay) = cancel_after {
                thread::sleep(delay);
                process.cancel().unwrap();
            }
            let output = process.wait().unwrap();
            (
                output.termination,
                output.exit_code,
                root,
                String::from_utf8_lossy(&output.stdout.bytes).into_owned(),
            )
        }
        Err(SandboxError::ScopeOwnership { exit_code, .. }) => {
            (Termination::Exited, exit_code, root, String::new())
        }
        Err(error) => panic!("supervised fault {name} failed to spawn: {error:?}"),
    };
    drop(relay);
    result
}

#[test]
fn native_supervisor_fault_matrix_is_terminal_and_noninterfering() {
    let Some(backend) = native_backend() else {
        return;
    };
    native_supervisor_forwards_cassette_http_and_reaps_children();
    let shell = Path::new("/bin/sh");
    let true_bin = Path::new("/bin/true");
    let cases = [
        (
            "sidecar-crash",
            vec!["-c".into(), "exit 17".into()],
            true_bin,
            Vec::new(),
        ),
        (
            "adapter-crash",
            vec!["-c".into(), "sleep 30".into()],
            shell,
            vec!["-c".into(), "exit 19".into()],
        ),
        (
            "supervisor-timeout",
            vec!["-c".into(), "sleep 30".into()],
            shell,
            vec!["-c".into(), "sleep 30".into()],
        ),
        (
            "provider-egress-denied",
            vec!["-c".into(), "sleep 30".into()],
            Path::new("/usr/bin/curl"),
            vec![
                "--fail".into(),
                "--silent".into(), "--connect-timeout".into(), "1".into(), "--write-out".into(), "ASB_PROVIDER_EGRESS_RC=%{exitcode}".into(),
                "http://192.0.2.1/".into(),
            ],
        ),
        (
            "descendant-egress-denied",
            vec!["-c".into(), "sleep 30".into()],
            shell,
            vec!["-c".into(), "curl --fail --silent --connect-timeout 1 --write-out ASB_DESCENDANT_EGRESS_RC=%{exitcode} http://192.0.2.1/".into()],
        ),
    ];
    for (name, sidecar_args, adapter, adapter_args) in cases {
        let (termination, exit_code, _root, output) = run_supervised_fault(
            &backend,
            name,
            shell,
            sidecar_args,
            adapter,
            adapter_args,
            Duration::from_millis(250),
            None,
        );
        assert_eq!(
            termination,
            Termination::Exited,
            "fault {name} did not reach a terminal state"
        );
        assert_ne!(exit_code, Some(0), "fault {name} unexpectedly succeeded");
        if name == "provider-egress-denied" {
            assert!(
                output.contains("ASB_PROVIDER_EGRESS_RC=7"),
                "provider denial marker missing: {output:?}"
            );
        }
        if name == "descendant-egress-denied" {
            assert!(
                output.contains("ASB_DESCENDANT_EGRESS_RC=7"),
                "descendant denial marker missing: {output:?}"
            );
        }
    }
    let (termination, exit_code, crash_root, _) = run_supervised_fault(
        &backend,
        "crash-before-fresh-generation",
        shell,
        vec!["-c".into(), "exit 23".into()],
        true_bin,
        Vec::new(),
        Duration::from_secs(2),
        None,
    );
    assert_eq!(termination, Termination::Exited);
    assert_ne!(exit_code, Some(0));
    assert!(!crash_root.join("relay.sock").exists());
    let (termination, exit_code, restart_root, _) = run_supervised_fault(
        &backend,
        "fresh-generation-after-crash",
        true_bin,
        Vec::new(),
        true_bin,
        Vec::new(),
        Duration::from_secs(2),
        None,
    );
    assert_eq!(termination, Termination::Exited);
    assert_eq!(exit_code, Some(0));
    assert!(!restart_root.join("relay.sock").exists());
    let mut unrelated = Command::new("/bin/sleep").arg("30").spawn().unwrap();
    let (termination, exit_code, _root, _) = run_supervised_fault(
        &backend,
        "unrelated-process",
        shell,
        vec!["-c".into(), "exit 23".into()],
        true_bin,
        Vec::new(),
        Duration::from_secs(2),
        None,
    );
    assert_eq!(termination, Termination::Exited);
    assert_ne!(exit_code, Some(0));
    assert!(
        unrelated.try_wait().unwrap().is_none(),
        "unrelated process was reaped"
    );
    unrelated.kill().unwrap();
    unrelated.wait().unwrap();

    for attempt in 0..3 {
        let name = format!("restart-{attempt}");
        let (termination, exit_code, root, _) = run_supervised_fault(
            &backend,
            &name,
            true_bin,
            Vec::new(),
            true_bin,
            Vec::new(),
            Duration::from_secs(2),
            None,
        );
        assert_eq!(termination, Termination::Exited);
        assert_eq!(exit_code, Some(0));
        assert!(
            !root.join("relay.sock").exists(),
            "relay leaked after {name}"
        );
    }

    let (termination, exit_code, _root, _) = run_supervised_fault(
        &backend,
        "cancellation",
        shell,
        vec!["-c".into(), "sleep 30".into()],
        shell,
        vec!["-c".into(), "sleep 30".into()],
        Duration::from_secs(2),
        Some(Duration::from_millis(20)),
    );
    assert_eq!(termination, Termination::Cancelled);
    assert_eq!(exit_code, None);
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
        helper_program(&root),
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
        helper_program(&root),
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
    let input = SandboxLaunchInput::new(request, limits()).unwrap();
    let process = backend.spawn_launch(input, lease(&root, &r)).unwrap();
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
}

#[test]
fn cleanup_failure_retains_lease_until_drop_retry_cleans_descendant() {
    if env::var_os("ASB_REQUIRE_NATIVE_SANDBOX").is_none() {
        return;
    }
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
        drop(root);
        return;
    };
    let backend = SandboxBackend::new(bubblewrap, systemd_run, systemctl, taskset);
    if backend.probe().is_err() {
        drop(root);
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
}
