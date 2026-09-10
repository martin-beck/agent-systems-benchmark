// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Reproduces the checked, privacy-safe CLI workflow transcript.

use asb_protocol::ExperimentManifestV1;
use asb_replay::{CassetteLimits, NetworkConsequence, RecordingCapture, RecordingConfirmation};
use asb_workloads::OriginalWorkloads;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

const SOURCE_REVISION: &str = "32df706413a6f165f086941426a5c793bd5e01e8";
const TRANSCRIPT: &str = include_str!("../../../docs/examples/asb-cli-workflow-v1.json");
const PROVENANCE: &str = include_str!("../../../docs/examples/asb-cli-workflow-v1.provenance.json");
const MERGE_ATTESTATION: &str =
    include_str!("../../../docs/examples/asb-cli-workflow-v1.merge-attestation.json");
static NONCE: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "asb-cli-transcript-{}-{}",
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

fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn sha256_file(path: &Path) -> String {
    sha256_bytes(&fs::read(path).unwrap())
}

fn write_plan(root: &Path, run_id: &str) -> (PathBuf, PathBuf) {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(root)
        .unwrap();
    let executable = root.join("offline-agent");
    fs::write(
        &executable,
        b"#!/bin/sh\ntest ! -e ../.asb-private/prompt || exit 97\nprintf '%s\\n' 'def parse_line(line):' '    if line.endswith(\"\\r\"):' '        line = line[:-1]' '    return line' > parser.py\n",
    )
    .unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    let executable = fs::canonicalize(executable).unwrap();
    let digest = sha256_file(&executable);
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
    let result_root = root.join("results");
    let work_root = root.join("work");
    let plan = toml::toml! {
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
    let mut document = plan;
    document["experiment"] = toml::Value::try_from(experiment).unwrap();
    let path = root.join("experiment.toml");
    fs::write(&path, toml::to_string(&document).unwrap()).unwrap();
    (path, result_root.join("runs").join(run_id))
}

fn run(arguments: &[String]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_asb"));
    command
        .env_clear()
        .env("LANG", "C")
        .env("LC_ALL", "C")
        .args(arguments);
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        command.env("LLVM_PROFILE_FILE", profile);
    }
    command.output().unwrap()
}

fn normalize(value: &mut Value, scratch: &Path) {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                if matches!(
                    key.as_str(),
                    "architecture"
                        | "interactive_stderr"
                        | "procfs"
                        | "cgroup_v2"
                        | "elapsed_ns"
                        | "scheduled_at_ns"
                        | "started_at_ns"
                        | "finished_at_ns"
                        | "queue_delay_ns"
                        | "metric_sample_count"
                        | "metric_available_count"
                        | "metric_unavailable_count"
                ) {
                    *value = Value::String(format!("<normalized-{key}>"));
                } else if key.ends_with("_sha256") {
                    *value = Value::String("<sha256>".into());
                } else {
                    normalize(value, scratch);
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                normalize(value, scratch);
            }
        }
        Value::String(text) => {
            *text = text.replace(scratch.to_str().unwrap(), "${SCENARIO_ROOT}");
        }
        _ => {}
    }
}

fn step(arguments: Vec<String>, scratch: &Path) -> Value {
    let output = run(&arguments);
    let displayed_arguments = arguments
        .iter()
        .map(|argument| argument.replace(scratch.to_str().unwrap(), "${SCENARIO_ROOT}"))
        .collect::<Vec<_>>();
    let mut stdout: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
        panic!(
            "command {arguments:?} did not emit JSON: {}",
            String::from_utf8_lossy(&output.stdout)
        )
    });
    normalize(&mut stdout, scratch);
    let stderr = String::from_utf8(output.stderr)
        .unwrap()
        .replace(scratch.to_str().unwrap(), "${SCENARIO_ROOT}")
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    json!({
        "argv": std::iter::once("asb".to_owned()).chain(displayed_arguments).collect::<Vec<_>>(),
        "exit_code": output.status.code().unwrap(),
        "stdout": stdout,
        "stderr": stderr,
    })
}

fn recording_capture(path: &Path) {
    let cassette = asb_replay::decode_cassette(
        include_bytes!("../../asb-replay/fixtures/v1/buffered.json"),
        CassetteLimits::default(),
    )
    .unwrap();
    let capture = RecordingCapture {
        schema_version: asb_replay::RECORDING_WORKFLOW_SCHEMA_VERSION,
        provider_profile_sha256: "a".repeat(64),
        agent_id: "codex".into(),
        network: NetworkConsequence::LoopbackOnly,
        estimated_cost_minor: 0,
        confirmation: RecordingConfirmation {
            record: true,
            network: true,
            cost: false,
        },
        contents: cassette.contents,
    };
    fs::write(path, serde_json::to_vec(&capture).unwrap()).unwrap();
}

fn produce_transcript() -> Value {
    let scratch = Scratch::new();
    let (first_plan, first_run) = write_plan(&scratch.0.join("first"), "capture-first");
    let (second_plan, second_run) = write_plan(&scratch.0.join("second"), "capture-second");
    let capture = scratch.0.join("capture.json");
    let cassette = scratch.0.join("cassette.json");
    recording_capture(&capture);
    let path = |value: &Path| value.to_str().unwrap().to_owned();
    let mut steps = vec![
        step(vec!["doctor".into()], &scratch.0),
        step(vec!["plan".into(), path(&first_plan)], &scratch.0),
        step(vec!["run".into(), path(&first_plan)], &scratch.0),
        step(vec!["report".into(), path(&first_run)], &scratch.0),
        step(vec!["run".into(), path(&second_plan)], &scratch.0),
        step(
            vec!["compare".into(), path(&first_run), path(&second_run)],
            &scratch.0,
        ),
        step(
            vec!["record".into(), path(&capture), path(&cassette)],
            &scratch.0,
        ),
    ];
    steps.push(step(
        vec![
            "replay".into(),
            path(&cassette),
            "a".repeat(64),
            "codex".into(),
        ],
        &scratch.0,
    ));
    json!({
        "schema_version": 1,
        "source_revision": SOURCE_REVISION,
        "scenario": "credential-free offline original.bug-fix",
        "normalization": ["scenario paths", "platform probes", "timing and metric counts", "content digests"],
        "steps": steps,
    })
}

#[test]
fn checked_transcript_is_reproducible_complete_and_privacy_safe() {
    let actual = produce_transcript();
    let expected: Value = serde_json::from_str(TRANSCRIPT).unwrap();
    if actual != expected {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/asb-cli-workflow-v1.actual.json");
        fs::write(&path, serde_json::to_string_pretty(&actual).unwrap() + "\n").unwrap();
        panic!(
            "CLI workflow transcript drifted; inspect {}",
            path.display()
        );
    }
    assert_eq!(actual["steps"].as_array().unwrap().len(), 8);
    assert!(
        actual["steps"]
            .as_array()
            .unwrap()
            .iter()
            .all(|step| step["exit_code"] == 0)
    );
    let encoded = serde_json::to_string(&actual).unwrap();
    assert!(encoded.contains("${SCENARIO_ROOT}"));
    for forbidden in [
        "/home/",
        "/srv/",
        "/tmp/",
        "authorization",
        "api_key",
        "api-key",
        "prompt",
        "response",
        "\\u001b",
    ] {
        assert!(
            !encoded.to_ascii_lowercase().contains(forbidden),
            "transcript contains forbidden material: {forbidden}"
        );
    }
}

#[test]
fn provenance_binds_the_exact_cli_and_public_fixture_sources() {
    let provenance: Value = serde_json::from_str(PROVENANCE).unwrap();
    assert_eq!(provenance["schema_version"], 1);
    assert_eq!(provenance["asb_revision"], SOURCE_REVISION);
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for (path, digest) in [
        (
            "crates/asb-cli/src/lib.rs",
            provenance["cli_source_sha256"].as_str().unwrap(),
        ),
        (
            "crates/asb-replay/fixtures/v1/buffered.json",
            provenance["replay_fixture_sha256"].as_str().unwrap(),
        ),
        (
            "docs/examples/guide-contract.json",
            provenance["guide_contract_sha256"].as_str().unwrap(),
        ),
        (
            "docs/examples/asb-cli-workflow-v1.json",
            provenance["transcript_sha256"].as_str().unwrap(),
        ),
    ] {
        assert_eq!(
            sha256_file(&root.join(path)),
            digest,
            "provenance drift: {path}"
        );
    }
}

#[test]
fn merge_attestation_preserves_the_unsigned_publication_boundary() {
    let attestation: Value = serde_json::from_str(MERGE_ATTESTATION).unwrap();
    assert_eq!(attestation["schema_version"], 1);
    assert_eq!(attestation["pull_request"], 129);
    assert_eq!(
        attestation["reviewed_commit"],
        "3db6e6be7f0fe457ee0cb8d44d7434868e157a1a"
    );
    assert_eq!(
        attestation["reviewed_tree"],
        "d90537bb476d45a15c95f04d86a9728eb3bb23e5"
    );
    assert_eq!(attestation["published_tree"], attestation["reviewed_tree"]);
    assert_eq!(attestation["exact_head_checks"]["expected"], 12);
    assert_eq!(attestation["exact_head_checks"]["successful"], 12);
    assert_eq!(attestation["published_commit_signature"], "unsigned");
    assert_eq!(attestation["publication_method"], "rebase");
    let policy = include_str!("../../../docs/DEVELOPMENT.md");
    assert!(policy.contains("gh pr merge --merge"));
    assert!(policy.contains("Rebase\nand squash publication are prohibited"));
}
