// SPDX-License-Identifier: MIT
//! Opt-in native execution of the exact pinned credential-free CSB fixture.

#![cfg(target_os = "linux")]

use asb_csb_runner::{
    BoundarySession, CSB_CONTRACT_V1, CSB_SOURCE_COMMIT, CSB_SOURCE_TREE, CsbExecutor, CsbPin,
    ExecutionMode, RunRequest,
};
use asb_protocol::{CallLimits, Id};
use asb_runtime::sandbox::{CpuSet, LeaseClass, ResourceLease, Resources, SandboxBackend, ToolPin};
use asb_runtime::{ProcessLimits, Termination};
use asb_store::{
    AtomicStore, ExecutionState, JOURNAL_SCHEMA_VERSION, JournalEvent, MANIFEST_SCHEMA_VERSION,
    RunManifest, StoreLimits,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use tempfile::{TempDir, tempdir_in};

const FIXTURE_SHA256: &str = "e1228516fe0648db35cfb7b6669b09f25dc32f62691c8fc8cca4ee1e935eee88";
const PYTHON_SHA256: &str = "1643dacd9feaedc58f3cc581e4d22577dfe25c09b10282936186ccf0f2e61118";
const PYTHON_VERSION: &str = "Python 3.12.3";
const SYSTEMD_VERSION: &str = "systemd 255 (255.4-1ubuntu8.17)";

fn native_source() -> Option<PathBuf> {
    env::var_os("ASB_CSB_SOURCE_ROOT").map(PathBuf::from)
}

fn git_value(root: &Path, argument: &str) -> String {
    let output = Command::new("git")
        .args(["-C", root.to_str().unwrap(), "rev-parse", argument])
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn backend() -> SandboxBackend {
    SandboxBackend::new(
        ToolPin::new(PathBuf::from("/usr/bin/bwrap"), "bubblewrap 0.9.0".into()).unwrap(),
        ToolPin::new(
            PathBuf::from("/usr/bin/systemd-run"),
            SYSTEMD_VERSION.into(),
        )
        .unwrap(),
        ToolPin::new(PathBuf::from("/usr/bin/systemctl"), SYSTEMD_VERSION.into()).unwrap(),
        ToolPin::new(
            PathBuf::from("/usr/bin/taskset"),
            "taskset from util-linux 2.39.3".into(),
        )
        .unwrap(),
    )
}

fn first_allowed_cpu() -> u32 {
    fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .find_map(|line| line.strip_prefix("Cpus_allowed_list:\t"))
        .unwrap()
        .split([',', '-'])
        .next()
        .unwrap()
        .parse()
        .unwrap()
}

fn append_state(
    store: &AtomicStore,
    run_id: &str,
    attempt_id: &str,
    sequence: u64,
    state: ExecutionState,
) {
    store
        .append(
            run_id,
            &JournalEvent {
                schema_version: JOURNAL_SCHEMA_VERSION,
                sequence,
                attempt_id: Id(attempt_id.into()),
                monotonic_offset_ns: sequence,
                state,
                evidence: json!({}),
            },
        )
        .unwrap();
}

fn private_test_root() -> TempDir {
    let target = env::var_os("CARGO_TARGET_DIR").expect("set an external CARGO_TARGET_DIR");
    let target = fs::canonicalize(target).unwrap();
    let checkout = fs::canonicalize(env!("CARGO_MANIFEST_DIR")).unwrap();
    assert!(!target.starts_with(&checkout));
    tempdir_in(target).unwrap()
}

fn assert_interpreter_identity() {
    let interpreter = fs::canonicalize("/usr/bin/python3").unwrap();
    assert_eq!(interpreter, PathBuf::from("/usr/bin/python3.12"));
    assert_eq!(
        format!("{:x}", Sha256::digest(fs::read(&interpreter).unwrap())),
        PYTHON_SHA256
    );
    let output = Command::new(&interpreter)
        .arg("--version")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        PYTHON_VERSION
    );
}

#[test]
fn exact_csb_fixture_runs_three_times_without_residual_lease() {
    let Some(source) = native_source() else {
        return;
    };
    let source = fs::canonicalize(source).unwrap();
    assert_interpreter_identity();
    assert_eq!(git_value(&source, "HEAD"), CSB_SOURCE_COMMIT);
    assert_eq!(git_value(&source, "HEAD^{tree}"), CSB_SOURCE_TREE);
    let fixture = source.join("bm-external/bwrap/bm-bwrap.py");
    let fixture_bytes = fs::read(&fixture).unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(&fixture_bytes)),
        FIXTURE_SHA256
    );

    let root = private_test_root();
    let workspace = root.path().join("workspace");
    let state = root.path().join("state");
    let leases = root.path().join("leases");
    fs::create_dir(&workspace).unwrap();
    fs::create_dir(&state).unwrap();
    fs::create_dir(&leases).unwrap();
    let program = workspace.join("csb-fixture");
    fs::write(&program, fixture_bytes).unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();

    let call_limits = CallLimits {
        max_frame_bytes: 64 * 1024,
        timeout_ms: 20_000,
        max_in_flight: 1,
    };
    let process_limits = ProcessLimits::new(
        64 * 1024,
        64 * 1024,
        Duration::from_secs(20),
        Duration::from_millis(250),
        Duration::from_millis(5),
    )
    .unwrap();
    let cpu = first_allowed_cpu();
    let cpus = CpuSet::new(vec![cpu]).unwrap();
    let resources = Resources::new(128 * 1024 * 1024, 32, 100, cpus.clone()).unwrap();
    let executor = CsbExecutor::new(backend());
    let mut session = BoundarySession::new(call_limits).unwrap();
    session.negotiate(CSB_CONTRACT_V1, call_limits).unwrap();

    for index in 0..3 {
        let run_id = format!("csb-native-{index}");
        let attempt_id = format!("attempt-{index}");
        let store = AtomicStore::open(&state, StoreLimits::default()).unwrap();
        store
            .create_run(&RunManifest {
                schema_version: MANIFEST_SCHEMA_VERSION,
                run_id: Id(run_id.clone()),
                attempt_id: Id(attempt_id.clone()),
                definition: json!({"fixture": "csb-bwrap-baseline-v1"}),
            })
            .unwrap();
        append_state(&store, &run_id, &attempt_id, 0, ExecutionState::Planned);
        append_state(&store, &run_id, &attempt_id, 1, ExecutionState::Prepared);
        let lease = ResourceLease::acquire(&leases, LeaseClass::Benchmark, cpus.clone()).unwrap();
        let running = executor
            .start(
                &mut session,
                RunRequest {
                    contract: CSB_CONTRACT_V1,
                    operation_id: format!("operation-{index}"),
                    attempt_id: attempt_id.clone(),
                    pin: CsbPin {
                        source_commit: CSB_SOURCE_COMMIT.into(),
                        source_tree: CSB_SOURCE_TREE.into(),
                        executable_sha256: FIXTURE_SHA256.into(),
                    },
                    mode: ExecutionMode::ExternalApplication,
                    arguments: vec![
                        "--scenario".into(),
                        "baseline".into(),
                        "--iterations".into(),
                        "1".into(),
                        "--units".into(),
                        "1".into(),
                        "--command".into(),
                        "/usr/bin/true".into(),
                    ],
                    environment: BTreeMap::from([("TZ".into(), "UTC".into())]),
                    artifacts: Vec::new(),
                },
                &workspace,
                &state,
                "csb-fixture",
                PathBuf::from("."),
                resources.clone(),
                lease,
                process_limits,
                StoreLimits::default(),
                &run_id,
                2,
            )
            .unwrap();
        let output = running.wait(&mut session).unwrap();
        assert_eq!(output.exit_code, Some(0));
        assert!(
            output
                .stdout
                .bytes
                .windows(15)
                .any(|part| part == b"success_count=1")
        );
        assert!(!leases.join(format!("cpu-{cpu}.lease")).exists());
        assert_eq!(
            AtomicStore::open(&state, StoreLimits::default())
                .unwrap()
                .load_journal(&run_id)
                .unwrap()
                .last()
                .unwrap()
                .state,
            ExecutionState::Completed
        );
    }
}

#[test]
fn exact_csb_fixture_cancels_and_times_out_without_residual_lease() {
    let Some(source) = native_source() else {
        return;
    };
    let source = fs::canonicalize(source).unwrap();
    assert_interpreter_identity();
    assert_eq!(git_value(&source, "HEAD"), CSB_SOURCE_COMMIT);
    assert_eq!(git_value(&source, "HEAD^{tree}"), CSB_SOURCE_TREE);
    let fixture_bytes = fs::read(source.join("bm-external/bwrap/bm-bwrap.py")).unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(&fixture_bytes)),
        FIXTURE_SHA256
    );

    for (suffix, timeout, cancel, expected_termination, expected_state) in [
        (
            "cancel",
            Duration::from_secs(20),
            true,
            Termination::Cancelled,
            ExecutionState::Cancelled,
        ),
        (
            "timeout",
            Duration::from_millis(150),
            false,
            Termination::TimedOut,
            ExecutionState::Failed,
        ),
    ] {
        let root = private_test_root();
        let workspace = root.path().join("workspace");
        let state = root.path().join("state");
        let leases = root.path().join("leases");
        fs::create_dir(&workspace).unwrap();
        fs::create_dir(&state).unwrap();
        fs::create_dir(&leases).unwrap();
        let program = workspace.join("csb-fixture");
        fs::write(&program, &fixture_bytes).unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();

        let call_limits = CallLimits {
            max_frame_bytes: 64 * 1024,
            timeout_ms: 20_000,
            max_in_flight: 1,
        };
        let process_limits = ProcessLimits::new(
            64 * 1024,
            64 * 1024,
            timeout,
            Duration::from_millis(100),
            Duration::from_millis(5),
        )
        .unwrap();
        let cpu = first_allowed_cpu();
        let cpus = CpuSet::new(vec![cpu]).unwrap();
        let resources = Resources::new(128 * 1024 * 1024, 32, 100, cpus.clone()).unwrap();
        let executor = CsbExecutor::new(backend());
        let mut session = BoundarySession::new(call_limits).unwrap();
        session.negotiate(CSB_CONTRACT_V1, call_limits).unwrap();
        let run_id = format!("csb-native-{suffix}");
        let attempt_id = format!("attempt-{suffix}");
        let store = AtomicStore::open(&state, StoreLimits::default()).unwrap();
        store
            .create_run(&RunManifest {
                schema_version: MANIFEST_SCHEMA_VERSION,
                run_id: Id(run_id.clone()),
                attempt_id: Id(attempt_id.clone()),
                definition: json!({"fixture": "csb-bwrap-baseline-v1"}),
            })
            .unwrap();
        append_state(&store, &run_id, &attempt_id, 0, ExecutionState::Planned);
        append_state(&store, &run_id, &attempt_id, 1, ExecutionState::Prepared);
        let lease = ResourceLease::acquire(&leases, LeaseClass::Benchmark, cpus).unwrap();
        let mut running = executor
            .start(
                &mut session,
                RunRequest {
                    contract: CSB_CONTRACT_V1,
                    operation_id: format!("operation-{suffix}"),
                    attempt_id: attempt_id.clone(),
                    pin: CsbPin {
                        source_commit: CSB_SOURCE_COMMIT.into(),
                        source_tree: CSB_SOURCE_TREE.into(),
                        executable_sha256: FIXTURE_SHA256.into(),
                    },
                    mode: ExecutionMode::ExternalApplication,
                    arguments: vec![
                        "--scenario".into(),
                        "baseline".into(),
                        "--iterations".into(),
                        "1".into(),
                        "--units".into(),
                        "1".into(),
                        "--command".into(),
                        "/usr/bin/sleep".into(),
                        "--command-arg".into(),
                        "10".into(),
                    ],
                    environment: BTreeMap::from([("TZ".into(), "UTC".into())]),
                    artifacts: Vec::new(),
                },
                &workspace,
                &state,
                "csb-fixture",
                PathBuf::from("."),
                resources,
                lease,
                process_limits,
                StoreLimits::default(),
                &run_id,
                2,
            )
            .unwrap();
        if cancel {
            running.cancel().unwrap();
        }
        let output = running.wait(&mut session).unwrap();
        assert_eq!(output.termination, expected_termination);
        assert_eq!(
            AtomicStore::open(&state, StoreLimits::default())
                .unwrap()
                .load_journal(&run_id)
                .unwrap()
                .last()
                .unwrap()
                .state,
            expected_state
        );
        assert!(!leases.join(format!("cpu-{cpu}.lease")).exists());
    }
}
