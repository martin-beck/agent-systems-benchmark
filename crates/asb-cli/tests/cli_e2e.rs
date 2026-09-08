// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Native process-boundary checks for the public `asb` executable.

use asb_protocol::ExperimentManifestV1;
use asb_workloads::OriginalWorkloads;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

static NONCE: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "asb-cli-e2e-{}-{}",
            std::process::id(),
            NONCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&path)
            .unwrap();
        Self(fs::canonicalize(path).unwrap())
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn sha256(path: &Path) -> String {
    format!("{:x}", Sha256::digest(fs::read(path).unwrap()))
}

fn child_marker_exists(work_root: &Path) -> bool {
    fs::read_dir(work_root).is_ok_and(|entries| {
        entries
            .flatten()
            .any(|entry| entry.path().join("workspace/.asb-child").is_file())
    })
}

fn cancellation_plan(root: &Path) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
    let executable = root.join("cancellable-agent");
    let launches = root.join("launches");
    fs::write(
        &executable,
        format!(
            "#!/bin/sh\nif [ -f .asb-child ]; then exec /bin/sleep 30; fi\nprintf 'launch\\n' >> '{}'
: > .asb-child\n\"$0\" &\nwait\n",
            launches.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    let executable = fs::canonicalize(executable).unwrap();
    let executable_sha256 = sha256(&executable);
    let workload = OriginalWorkloads::describe("original.bug-fix").unwrap();
    let mut experiment: ExperimentManifestV1 = serde_json::from_str(include_str!(
        "../../asb-protocol/fixtures/v1/experiment-manifest.json"
    ))
    .unwrap();
    experiment
        .agent
        .binary_sha256
        .clone_from(&executable_sha256);
    experiment.workload.workload = workload.workload_id.0;
    experiment.workload.workload_revision = workload.version;
    experiment.workload.workload_sha256 = workload.content_sha256;
    experiment.workload.scorer_revision = workload.scoring_version;
    experiment.platform.architecture = std::env::consts::ARCH.to_owned();
    experiment.refresh_content_address().unwrap();
    let result_root = root.join("results");
    let work_root = root.join("work");
    let plan = toml::toml! {
        schema_version = 1
        run_id = "cancel-e2e"
        result_root = (result_root.to_str().unwrap())
        work_root = (work_root.to_str().unwrap())
        workload = "original.bug-fix"

        [agent]
        executable = (executable.to_str().unwrap())
        executable_sha256 = (executable_sha256)
        arguments = []

        [point]
        measured = 6
        warmups = 0
        concurrency = 1
        queue = 0
        max_failures = 100
        timeout_ms = 10_000
        poll_ms = 2
        seed = 1

        [experiment]
    };
    let mut document = plan;
    document["experiment"] = toml::Value::try_from(experiment).unwrap();
    let plan_path = root.join("cancel.toml");
    fs::write(&plan_path, toml::to_string(&document).unwrap()).unwrap();
    (plan_path, result_root, executable, launches)
}

#[test]
fn piped_doctor_is_json_without_terminal_escapes() {
    let output = Command::new(env!("CARGO_BIN_EXE_asb"))
        .arg("doctor")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert!(!output.stdout.contains(&0x1b));
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["command"],
        "doctor"
    );
}

#[test]
fn pseudo_terminal_doctor_remains_clean_and_machine_readable() {
    let command = format!("{} doctor", env!("CARGO_BIN_EXE_asb"));
    let output = Command::new("/usr/bin/script")
        .args(["-q", "-e", "-c", &command, "/dev/null"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(!output.stdout.contains(&0x1b));
    let rendered = String::from_utf8(output.stdout).unwrap();
    let line = rendered
        .lines()
        .find(|line| line.trim_start().starts_with('{'))
        .unwrap()
        .trim_end_matches('\r');
    let doctor: Value = serde_json::from_str(line).unwrap();
    assert_eq!(doctor["command"], "doctor");
    assert_eq!(doctor["interactive_stderr"], true);
}

#[test]
fn sigint_cancels_process_group_persists_terminal_state_and_returns_json() {
    let scratch = Scratch::new();
    let (plan, result_root, executable, launches) = cancellation_plan(&scratch.0);
    let child = Command::new(env!("CARGO_BIN_EXE_asb"))
        .args(["run", plan.to_str().unwrap()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !child_marker_exists(&scratch.0.join("work")) {
        assert!(Instant::now() < deadline, "agent process did not start");
        thread::sleep(Duration::from_millis(5));
    }
    assert!(
        Command::new("/bin/kill")
            .args(["-INT", &child.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(130));
    assert!(!output.stdout.contains(&0x1b));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["ok"], false);
    assert_eq!(result["cancelled"], true);
    assert_eq!(result["points"][0]["cancelled"], 6);
    assert_eq!(
        result["points"][0]["scheduler_attempts"]
            .as_array()
            .unwrap()
            .len(),
        6
    );
    assert_eq!(result["points"][0]["attempts"].as_array().unwrap().len(), 1);
    assert_eq!(fs::read_to_string(launches).unwrap().lines().count(), 1);
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("starting cancel-e2e")
    );
    let journal = fs::read_to_string(result_root.join("runs/cancel-e2e/journal.ndjson")).unwrap();
    assert!(journal.contains("\"state\":\"cancelled\""));
    for input_id in 0..6 {
        assert!(
            !scratch
                .0
                .join(format!("work/cancel-e2e-measured-{input_id}"))
                .exists()
        );
    }
    let executable_bytes = executable.as_os_str().as_encoded_bytes();
    for entry in fs::read_dir("/proc").unwrap().flatten() {
        if entry
            .file_name()
            .as_encoded_bytes()
            .iter()
            .all(u8::is_ascii_digit)
            && let Ok(command) = fs::read(entry.path().join("cmdline"))
        {
            assert!(
                !command
                    .windows(executable_bytes.len())
                    .any(|part| part == executable_bytes)
            );
        }
    }
}
