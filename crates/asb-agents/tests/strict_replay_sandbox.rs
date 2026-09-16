// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Native integration coverage for the strict replay sandbox launch seam.

#![cfg(target_os = "linux")]

use asb_agents::strict_replay::{
    EgressPolicy, StrictReplayLaunchRecord, StrictReplayLaunchV1, StrictReplaySandboxLaunch,
};
use asb_runtime::ProcessLimits;
use asb_runtime::sandbox::{
    CpuSet, LeaseClass, NetworkPolicy, ResourceLease, Resources, SandboxBackend,
    SandboxLaunchInput, SandboxSpec, ToolPin,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

const BWRAP_VERSION: &str = "bubblewrap 0.9.0";
const SYSTEMD_VERSION: &str = "systemd 255 (255.4-1ubuntu8.17)";
const TASKSET_VERSION: &str = "taskset from util-linux 2.39.3";

fn digest_command(program: &str, arguments: &[String]) -> String {
    let mut digest = Sha256::new();
    digest.update(b"asb-strict-replay-command-v1");
    digest.update(program.as_bytes());
    digest.update([0]);
    for argument in arguments {
        digest.update(argument.as_bytes());
        digest.update([0]);
    }
    format!("{:x}", digest.finalize())
}

fn launch_record(command_sha256: String) -> StrictReplayLaunchRecord {
    let input = StrictReplayLaunchV1 {
        schema_version: 1,
        cassette_sha256: "a".repeat(64),
        route_sha256: "b".repeat(64),
        provider_dialect: "openai-chat-v1".into(),
        adapter: "fixture".into(),
        run_id: "native-run".into(),
        attempt_id: "native-attempt".into(),
        workload_sha256: "c".repeat(64),
        command_sha256,
        egress: EgressPolicy::LoopbackOnly,
        timeout_ms: 5_000,
    };
    StrictReplayLaunchRecord {
        launch_sha256: input.digest().unwrap(),
        input,
    }
}

fn backend() -> Option<SandboxBackend> {
    let pin = |path: &str, version: &str| ToolPin::new(PathBuf::from(path), version.into()).ok();
    let backend = SandboxBackend::new(
        pin("/usr/bin/bwrap", BWRAP_VERSION)?,
        pin("/usr/bin/systemd-run", SYSTEMD_VERSION)?,
        pin("/usr/bin/systemctl", SYSTEMD_VERSION)?,
        pin("/usr/bin/taskset", TASKSET_VERSION)?,
    );
    backend.probe().ok().map(|_| backend)
}

fn root(label: &str) -> Option<PathBuf> {
    let parent = PathBuf::from(env::var_os("ASB_TEST_ROOT")?);
    if !parent.is_absolute() || !parent.starts_with("/srv/data/projects/") {
        return None;
    }
    let path = parent.join(format!("strict-replay-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(path.join("work")).ok()?;
    fs::create_dir(path.join("leases")).ok()?;
    Some(path)
}

fn resources() -> Resources {
    Resources::new(
        64 * 1024 * 1024,
        8,
        25,
        CpuSet::new(vec![first_cpu()]).unwrap(),
    )
    .unwrap()
}

fn first_cpu() -> u32 {
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

fn limits() -> ProcessLimits {
    ProcessLimits::new(
        1024 * 1024,
        1024 * 1024,
        Duration::from_secs(5),
        Duration::from_millis(250),
        Duration::from_millis(5),
    )
    .unwrap()
}

fn input(
    root: &Path,
    program: &str,
    environment: BTreeMap<String, String>,
) -> (SandboxLaunchInput, ResourceLease) {
    input_with_limits(root, program, Vec::new(), environment, limits())
}

fn input_with_limits(
    root: &Path,
    program: &str,
    arguments: Vec<String>,
    environment: BTreeMap<String, String>,
    process_limits: ProcessLimits,
) -> (SandboxLaunchInput, ResourceLease) {
    let resources = resources();
    let spec = SandboxSpec::new(
        root.parent().unwrap(),
        PathBuf::from(root.file_name().unwrap()).join("work"),
        program.into(),
        arguments,
        environment,
        resources.clone(),
        NetworkPolicy::Deny,
    )
    .unwrap();
    let input = SandboxLaunchInput::new(spec, process_limits).unwrap();
    let lease = ResourceLease::acquire(
        &root.join("leases"),
        LeaseClass::Benchmark,
        resources.cpus().clone(),
    )
    .unwrap();
    (input, lease)
}

fn short_limits() -> ProcessLimits {
    ProcessLimits::new(
        1024 * 1024,
        1024 * 1024,
        Duration::from_millis(100),
        Duration::from_millis(100),
        Duration::from_millis(5),
    )
    .unwrap()
}

#[test]
fn strict_launch_spawns_with_authenticated_loopback_environment() {
    let Some(backend) = backend() else { return };
    let Some(root) = root("success") else { return };
    let environment = BTreeMap::from([
        (
            "ASB_REPLAY_ROUTE_SHA256".into(),
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
        ),
        (
            "ASB_REPLAY_ENDPOINT".into(),
            "http://127.0.0.1:4317/replay".into(),
        ),
    ]);
    let (input, lease) = input(&root, "/usr/bin/env", environment);
    assert_eq!(input.spec().environment().len(), 2);
    let record = launch_record(digest_command(
        input.spec().program(),
        input.spec().arguments(),
    ));
    let mut process = StrictReplaySandboxLaunch::new(record)
        .unwrap()
        .spawn(&backend, input, lease)
        .unwrap();
    let output = process.wait().unwrap();
    assert_eq!(output.exit_code, Some(0));
    assert!(
        String::from_utf8_lossy(&output.stdout.bytes)
            .lines()
            .any(|line| line == "ASB_REPLAY_ENDPOINT=http://127.0.0.1:4317/replay")
    );
}

#[test]
fn strict_launch_rejects_command_identity_before_native_spawn() {
    let Some(root) = root("reject") else { return };
    let (input, lease) = input(
        &root,
        "/usr/bin/true",
        BTreeMap::from([
            ("ASB_REPLAY_ROUTE_SHA256".into(), "b".repeat(64)),
            (
                "ASB_REPLAY_ENDPOINT".into(),
                "http://127.0.0.1:4317/replay".into(),
            ),
        ]),
    );
    let record = launch_record(digest_command("/usr/bin/false", &[]));
    let backend = backend().expect("native backend was probed for this test");
    let error = match StrictReplaySandboxLaunch::new(record)
        .unwrap()
        .spawn(&backend, input, lease)
    {
        Ok(_) => panic!("mismatched command must be rejected before native spawn"),
        Err(error) => error,
    };
    assert_eq!(
        error,
        asb_agents::strict_replay::StrictReplayError::CommandMismatch
    );
}

#[test]
fn strict_launch_enforces_authenticated_timeout_on_child() {
    let Some(backend) = backend() else { return };
    let Some(root) = root("timeout") else { return };
    let (input, lease) = input_with_limits(
        &root,
        "/usr/bin/sleep",
        vec!["30".into()],
        BTreeMap::from([
            (
                "ASB_REPLAY_ROUTE_SHA256".into(),
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            ),
            (
                "ASB_REPLAY_ENDPOINT".into(),
                "http://127.0.0.1:4317/replay".into(),
            ),
        ]),
        short_limits(),
    );
    let mut record = launch_record(digest_command(
        input.spec().program(),
        input.spec().arguments(),
    ));
    record.input.timeout_ms = 100;
    record.launch_sha256 = record.input.digest().unwrap();
    let mut process = StrictReplaySandboxLaunch::new(record)
        .unwrap()
        .spawn(&backend, input, lease)
        .unwrap();
    assert_eq!(
        process.wait().unwrap().termination,
        asb_runtime::Termination::TimedOut
    );
}

#[test]
fn strict_launch_cancellation_is_terminal_and_reaped() {
    let Some(backend) = backend() else { return };
    let Some(root) = root("cancel") else { return };
    let (input, lease) = input_with_limits(
        &root,
        "/usr/bin/sleep",
        vec!["30".into()],
        BTreeMap::from([
            (
                "ASB_REPLAY_ROUTE_SHA256".into(),
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            ),
            (
                "ASB_REPLAY_ENDPOINT".into(),
                "http://127.0.0.1:4317/replay".into(),
            ),
        ]),
        limits(),
    );
    let record = launch_record(digest_command(
        input.spec().program(),
        input.spec().arguments(),
    ));
    let mut process = StrictReplaySandboxLaunch::new(record)
        .unwrap()
        .spawn(&backend, input, lease)
        .unwrap();
    process.cancel().unwrap();
    assert_eq!(
        process.wait().unwrap().termination,
        asb_runtime::Termination::Cancelled
    );
    assert_eq!(process.lifecycle(), asb_runtime::ProcessLifecycle::Terminal);
    assert!(process.cancel().is_ok());
}

#[test]
fn strict_launch_nonzero_child_exit_is_fail_closed() {
    let Some(backend) = backend() else { return };
    let Some(root) = root("crash") else { return };
    let (input, lease) = input_with_limits(
        &root,
        "/usr/bin/false",
        Vec::new(),
        BTreeMap::from([
            (
                "ASB_REPLAY_ROUTE_SHA256".into(),
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            ),
            (
                "ASB_REPLAY_ENDPOINT".into(),
                "http://127.0.0.1:4317/replay".into(),
            ),
        ]),
        limits(),
    );
    let record = launch_record(digest_command(
        input.spec().program(),
        input.spec().arguments(),
    ));
    let result = StrictReplaySandboxLaunch::new(record)
        .unwrap()
        .spawn(&backend, input, lease);
    match result {
        Ok(mut process) => {
            assert_ne!(process.wait().unwrap().exit_code, Some(0));
            assert_eq!(process.lifecycle(), asb_runtime::ProcessLifecycle::Terminal);
        }
        Err(asb_agents::strict_replay::StrictReplayError::SandboxUnavailable) => {}
        Err(error) => panic!("unexpected crash classification: {error:?}"),
    }
}

#[test]
fn strict_launch_child_provider_egress_is_denied() {
    let Some(backend) = backend() else { return };
    let Some(root) = root("egress") else { return };
    let (input, lease) = input_with_limits(
        &root,
        "/usr/bin/curl",
        vec![
            "--connect-timeout".into(),
            "0.1".into(),
            "http://198.51.100.1/".into(),
        ],
        BTreeMap::from([
            ("ASB_REPLAY_ROUTE_SHA256".into(), "b".repeat(64)),
            (
                "ASB_REPLAY_ENDPOINT".into(),
                "http://127.0.0.1:4317/replay".into(),
            ),
        ]),
        short_limits(),
    );
    let record = launch_record(digest_command(
        input.spec().program(),
        input.spec().arguments(),
    ));
    let result = StrictReplaySandboxLaunch::new(record)
        .unwrap()
        .spawn(&backend, input, lease);
    match result {
        Ok(mut process) => assert_ne!(process.wait().unwrap().exit_code, Some(0)),
        Err(asb_agents::strict_replay::StrictReplayError::SandboxUnavailable) => {}
        Err(error) => panic!("unexpected egress classification: {error:?}"),
    }
}
