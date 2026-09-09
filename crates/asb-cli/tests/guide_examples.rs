// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Executes the public offline documentation workflow and support inventory.

use asb_protocol::ExperimentManifestV1;
use asb_workloads::OriginalWorkloads;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NONCE: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "asb-guide-{}-{}",
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

fn asb(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_asb"))
        .args(arguments)
        .output()
        .unwrap()
}

fn sha256(path: &Path) -> String {
    format!("{:x}", Sha256::digest(fs::read(path).unwrap()))
}

fn write_plan(parent: &Path, run_id: &str, sweep: bool) -> (PathBuf, PathBuf, PathBuf) {
    fs::DirBuilder::new().mode(0o700).create(parent).unwrap();
    let executable = parent.join("offline-agent");
    fs::write(
        &executable,
        b"#!/bin/sh\ntest ! -e ../.asb-private/prompt || exit 97\nprintf '%s\\n' 'def parse_line(line):' '    if line.endswith(\"\\r\"):' '        line = line[:-1]' '    return line' > parser.py\n",
    )
    .unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    let executable = fs::canonicalize(executable).unwrap();
    let digest = sha256(&executable);
    let workload = OriginalWorkloads::describe("original.bug-fix").unwrap();
    let mut experiment: ExperimentManifestV1 = serde_json::from_str(include_str!(
        "../../asb-protocol/fixtures/v1/experiment-manifest.json"
    ))
    .unwrap();
    experiment.agent.binary_sha256.clone_from(&digest);
    experiment.workload.workload = workload.workload_id.0;
    experiment.workload.workload_revision = workload.version;
    experiment.workload.workload_sha256 = workload.content_sha256;
    experiment.workload.scorer_revision = workload.scoring_version;
    experiment.platform.architecture = std::env::consts::ARCH.to_owned();
    experiment.refresh_content_address().unwrap();
    let result_root = parent.join("results");
    let work_root = parent.join("work");
    let mut plan = toml::toml! {
        schema_version = 1
        run_id = (run_id)
        result_root = (result_root.to_str().unwrap())
        work_root = (work_root.to_str().unwrap())
        workload = "original.bug-fix"

        [agent]
        executable = (executable.to_str().unwrap())
        executable_sha256 = (digest)
        arguments = []

        [point]
        measured = 1
        warmups = 0
        concurrency = 1
        queue = 0
        max_failures = 0
        timeout_ms = 5_000
        poll_ms = 2
        seed = 7

        [experiment]
    };
    if sweep {
        plan["point"]
            .as_table_mut()
            .unwrap()
            .insert("sweep_max_concurrency".into(), toml::Value::Integer(2));
    }
    plan["experiment"] = toml::Value::try_from(experiment).unwrap();
    let path = parent.join("experiment.toml");
    fs::write(&path, toml::to_string(&plan).unwrap()).unwrap();
    (path, result_root, work_root)
}

fn successful_json(arguments: &[&str]) -> Value {
    let output = asb(arguments);
    assert!(
        output.status.success(),
        "{arguments:?}: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn documented_offline_workflow_produces_validated_artifacts() {
    let scratch = Scratch::new();
    let (first_plan, first_results, first_work) =
        write_plan(&scratch.0.join("first"), "guide-first", false);
    let first_plan = first_plan.to_str().unwrap();
    assert_eq!(successful_json(&["plan", first_plan])["command"], "plan");
    let first = successful_json(&["run", first_plan]);
    assert_eq!(first["ok"], true);
    assert_eq!(first["points"][0]["decision"], "pass");
    let first_run = first_results.join("runs/guide-first");
    assert!(first_run.join("manifest.json").is_file());
    assert!(first_run.join("journal.ndjson").is_file());
    assert!(fs::read_dir(first_work).unwrap().next().is_none());
    assert_eq!(
        successful_json(&["report", first_run.to_str().unwrap()])["runs"][0]["terminal_state"],
        "completed"
    );

    let (second_plan, second_results, _) =
        write_plan(&scratch.0.join("second"), "guide-second", false);
    successful_json(&["run", second_plan.to_str().unwrap()]);
    let second_run = second_results.join("runs/guide-second");
    let comparison = successful_json(&[
        "compare",
        first_run.to_str().unwrap(),
        second_run.to_str().unwrap(),
    ]);
    assert_eq!(comparison["comparable"], true);
    assert_eq!(comparison["differences"], json!([]));

    let (sweep_plan, _, sweep_work) = write_plan(&scratch.0.join("sweep"), "guide-sweep", true);
    let sweep = successful_json(&["sweep", sweep_plan.to_str().unwrap()]);
    assert_eq!(sweep["ok"], true);
    assert_eq!(sweep["points"].as_array().unwrap().len(), 2);
    assert!(fs::read_dir(sweep_work).unwrap().next().is_none());
}

#[test]
fn guide_inventory_matches_doctor_and_stale_claims_fail_closed() {
    let contract: Value =
        serde_json::from_str(include_str!("../../../docs/examples/guide-contract.json")).unwrap();
    assert_eq!(contract["schema_version"], 1);
    let doctor = successful_json(&["doctor"]);
    assert_eq!(doctor["commands"], contract["commands"]);
    assert_eq!(doctor["workloads"], contract["workloads"]);
    for command in contract["unsupported_commands"].as_array().unwrap() {
        let output = asb(&[command.as_str().unwrap()]);
        assert_eq!(output.status.code(), Some(2));
        let body: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(body["error"]["code"], "usage");
    }

    let scratch = Scratch::new();
    let (plan, results, _) = write_plan(&scratch.0.join("stale"), "guide-stale", false);
    let text =
        fs::read_to_string(&plan)
            .unwrap()
            .replacen("schema_version = 1", "schema_version = 2", 1);
    fs::write(&plan, text).unwrap();
    let output = asb(&["plan", plan.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(3));
    assert!(!results.exists());
}

#[test]
fn public_guides_reference_the_executable_contract() {
    let contract: Value =
        serde_json::from_str(include_str!("../../../docs/examples/guide-contract.json")).unwrap();
    let quickstart = include_str!("../../../docs/QUICKSTART.md");
    let extensions = include_str!("../../../docs/EXTENSIONS.md");
    let reproducibility = include_str!("../../../docs/REPRODUCIBILITY.md");
    for guide in [quickstart, extensions, reproducibility] {
        assert!(guide.contains("guide-contract.json") || guide.contains("guide_examples"));
    }
    assert!(quickstart.contains("asb record CAPTURE.json CASSETTE.json"));
    assert!(quickstart.contains("asb replay CASSETTE.json PROVIDER_PROFILE_SHA256 AGENT"));
    for command in contract["commands"].as_array().unwrap() {
        let row = format!("| `{}` | supported |", command.as_str().unwrap());
        assert!(quickstart.contains(&row));
    }
    for command in contract["unsupported_commands"].as_array().unwrap() {
        let row = format!("| `{}` | unsupported |", command.as_str().unwrap());
        assert!(quickstart.contains(&row));
    }
}

#[test]
fn beginner_workflow_hub_has_complete_links_and_honest_boundaries() {
    let hub = include_str!("../../../docs/workflows/README.md");
    let cli = include_str!("../../../docs/workflows/cli-first-run.md");
    let tui = include_str!("../../../docs/workflows/tui-first-run.md");
    let multi = include_str!("../../../docs/workflows/multi-agent-provider.md");
    let replay = include_str!("../../../docs/workflows/record-replay.md");
    let recovery = include_str!("../../../docs/workflows/troubleshooting.md");
    for link in [
        "cli-first-run.md",
        "tui-first-run.md",
        "multi-agent-provider.md",
        "record-replay.md",
        "troubleshooting.md",
    ] {
        assert!(hub.contains(link), "workflow hub missing {link}");
    }
    for command in [
        "asb doctor",
        "asb plan",
        "asb run",
        "asb sweep",
        "asb report",
        "asb compare",
        "asb provider-catalog",
        "asb provider-plan",
        "asb record",
        "asb replay",
    ] {
        assert!(
            cli.contains(command) || multi.contains(command) || replay.contains(command),
            "workflow docs missing {command}"
        );
    }
    for model in [
        "MultiAgentWizard",
        "RunControl",
        "ControlCall::ValidateSettings",
        "ControlCall::CreatePlan",
    ] {
        assert!(tui.contains(model), "TUI guide missing {model}");
    }
    assert!(replay.contains("RecordingWorkflow"));
    assert!(hub.contains("does not promise a keyboard map"));
    assert!(recovery.contains("needs_reconciliation"));
    let home_prefix = format!("{}home{}", '/', '/');
    let private_host = ["ai", "ws"].join("-");
    for guide in [hub, cli, tui, multi, replay, recovery] {
        assert!(!guide.contains(&home_prefix));
        assert!(!guide.contains(&private_host));
    }
}
