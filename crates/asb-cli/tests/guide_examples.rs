// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Executes the public offline documentation workflow and support inventory.

use asb_replay::{CassetteLimits, canonical_contents_bytes, decode_cassette};
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
        .arg("--json")
        .args(arguments)
        .output()
        .unwrap()
}

fn human_asb(root: &Path, arguments: &[String]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_asb"))
        .args(arguments)
        .env("HOME", root)
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_STATE_HOME", root.join("state"))
        .env("XDG_DATA_HOME", root.join("data"))
        .env("COLUMNS", "80")
        .output()
        .unwrap()
}

fn json_asb_at(root: &Path, arguments: &[String]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_asb"))
        .arg("--json")
        .args(arguments)
        .env("HOME", root)
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_STATE_HOME", root.join("state"))
        .env("XDG_DATA_HOME", root.join("data"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn displayed_next_shell(output: &Output) -> String {
    let stdout = String::from_utf8(output.stdout.clone()).unwrap();
    let line = stdout
        .lines()
        .find_map(|line| line.strip_prefix("Next: "))
        .unwrap_or_else(|| panic!("missing displayed Next command: {stdout}"));
    assert!(!line.chars().any(char::is_control));
    line.to_owned()
}

fn execute_displayed_next(root: &Path, output: &Output) -> Output {
    let command = displayed_next_shell(output);
    let binary = Path::new(env!("CARGO_BIN_EXE_asb"));
    Command::new("/bin/sh")
        .args(["-c", &command])
        .env_clear()
        .env("PATH", binary.parent().unwrap())
        .env("HOME", root)
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_STATE_HOME", root.join("state"))
        .env("XDG_DATA_HOME", root.join("data"))
        .env("COLUMNS", "80")
        .output()
        .unwrap()
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
    let result_root = parent.join("results");
    let work_root = parent.join("work");
    let plan_path = parent.join("experiment.toml");
    let generated = Command::new(env!("CARGO_BIN_EXE_asb"))
        .arg("--json")
        .args(["plan", "create", "--workload", "original.bug-fix"])
        .args(["--agent-executable", executable.to_str().unwrap()])
        .args(["--run-id", run_id])
        .args(["--result-root", result_root.to_str().unwrap()])
        .args(["--work-root", work_root.to_str().unwrap()])
        .args(["--output", plan_path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        generated.status.success(),
        "plan create failed: {}{}",
        String::from_utf8_lossy(&generated.stdout),
        String::from_utf8_lossy(&generated.stderr)
    );
    if sweep {
        let mut plan: toml::Value =
            toml::from_str(&fs::read_to_string(&plan_path).unwrap()).unwrap();
        plan["point"]
            .as_table_mut()
            .unwrap()
            .insert("sweep_max_concurrency".into(), toml::Value::Integer(2));
        fs::write(&plan_path, toml::to_string(&plan).unwrap()).unwrap();
    }
    (plan_path, result_root, work_root)
}

fn bind_plan_to_persisted_selection(plan_path: &Path, selection: &Value) {
    let mut plan: toml::Value = toml::from_str(&fs::read_to_string(plan_path).unwrap()).unwrap();
    let mut experiment: asb_protocol::ExperimentManifestV1 =
        plan["experiment"].clone().try_into().unwrap();
    experiment.agent.implementation = "codex".to_owned();
    experiment.model.provider = "openrouter".to_owned();
    experiment.model.model = "cohere/north-mini-code:free".to_owned();
    experiment.model.settings.additional_settings_sha256 = Some(
        selection["provider_profile_sha256"]
            .as_str()
            .unwrap()
            .to_owned(),
    );
    experiment.refresh_content_address().unwrap();
    plan["experiment"] = toml::Value::try_from(experiment).unwrap();
    fs::write(plan_path, toml::to_string(&plan).unwrap()).unwrap();
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
    assert_eq!(comparison["comparable"], false);
    assert_eq!(comparison["differences"], json!([]));
    assert_eq!(
        comparison["unavailable_reasons"],
        json!([
            "baseline:provider_selection_unavailable",
            "guide-second:provider_selection_unavailable"
        ])
    );
    let repeated_candidate = successful_json(&[
        "compare",
        first_run.to_str().unwrap(),
        second_run.to_str().unwrap(),
        second_run.to_str().unwrap(),
    ]);
    assert_eq!(repeated_candidate["comparable"], false);
    assert_eq!(
        repeated_candidate["unavailable_reasons"],
        json!([
            "baseline:provider_selection_unavailable",
            "guide-second:provider_selection_unavailable"
        ])
    );
    assert_eq!(repeated_candidate["pairs"][0]["comparable"], false);
    assert_eq!(repeated_candidate["pairs"][1]["comparable"], false);

    let (sweep_plan, _, sweep_work) = write_plan(&scratch.0.join("sweep"), "guide-sweep", true);
    let sweep = successful_json(&["sweep", sweep_plan.to_str().unwrap()]);
    assert_eq!(sweep["ok"], true);
    assert_eq!(sweep["points"].as_array().unwrap().len(), 2);
    assert!(fs::read_dir(sweep_work).unwrap().next().is_none());
}

#[test]
fn human_journey_reaches_selection_sweep_report_compare_and_tui() {
    let scratch = Scratch::new();
    let journey = scratch.0.join("human journey's space");
    fs::DirBuilder::new().mode(0o700).create(&journey).unwrap();

    let preflight = human_asb(&journey, &["setup".to_owned()]);
    assert!(preflight.status.success());
    assert_eq!(displayed_next_shell(&preflight), "asb provider-catalog");
    let catalog = execute_displayed_next(&journey, &preflight);
    assert!(catalog.status.success());
    let catalog_text = String::from_utf8(catalog.stdout).unwrap();
    assert!(catalog_text.contains("Selectable providers:"));
    assert!(catalog_text.contains("Supported agents:"));
    assert!(catalog_text.contains("codex"));
    assert!(catalog_text.contains("openrouter"));
    assert!(catalog_text.contains("cohere/north-mini-code:free"));
    assert!(catalog_text.contains("--agent AGENT"));
    assert!(!catalog_text.contains("Next:"));

    let setup_args = [
        "setup",
        "--agent",
        "codex",
        "--provider-profile",
        "openrouter",
        "--model",
        "cohere/north-mini-code:free",
        "--persist",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    let selected = human_asb(&journey, &setup_args);
    assert!(
        selected.status.success(),
        "{}",
        String::from_utf8_lossy(&selected.stdout)
    );
    assert_eq!(displayed_next_shell(&selected), "asb workload-catalog");
    assert!(execute_displayed_next(&journey, &selected).status.success());

    let (plan, result_root, _) = write_plan(&journey.join("human-sweep"), "human-sweep", true);
    let catalog = json_asb_at(&journey, &["provider-catalog".to_owned()]);
    let config: Value =
        serde_json::from_slice(&fs::read(journey.join("config/asb/config.json")).unwrap()).unwrap();
    let selection_args = vec![
        "provider-plan".to_owned(),
        "--catalog-sha256".to_owned(),
        catalog["catalog_sha256"].as_str().unwrap().to_owned(),
        "--provider-profile".to_owned(),
        "openrouter".to_owned(),
        "--credential-reference-sha256".to_owned(),
        config["profiles"]["openrouter-cohere/north-mini-code:free"]["credential"]
            ["locator_sha256"]
            .as_str()
            .unwrap()
            .to_owned(),
        "--agent".to_owned(),
        "codex".to_owned(),
    ];
    let selection = json_asb_at(&journey, &selection_args);
    bind_plan_to_persisted_selection(&plan, &selection);
    let plan_args = vec![
        "plan".to_owned(),
        plan.to_string_lossy().into_owned(),
        "--use-config".to_owned(),
    ];
    let planned = human_asb(&journey, &plan_args);
    assert!(
        planned.status.success(),
        "{}{}",
        String::from_utf8_lossy(&planned.stdout),
        String::from_utf8_lossy(&planned.stderr)
    );

    let sweep_command = displayed_next_shell(&planned);
    assert!(sweep_command.starts_with("asb sweep "));
    assert!(sweep_command.ends_with(" --use-config"));
    assert!(sweep_command.contains("'\\''"));
    let swept = execute_displayed_next(&journey, &planned);
    assert!(
        swept.status.success(),
        "{}{}",
        String::from_utf8_lossy(&swept.stdout),
        String::from_utf8_lossy(&swept.stderr)
    );
    let report_command = displayed_next_shell(&swept);
    assert!(report_command.starts_with("asb report "));
    let run_refs = fs::read_dir(result_root.join("runs"))
        .unwrap()
        .map(|entry| fs::canonicalize(entry.unwrap().path()).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(run_refs.len(), 2);
    for path in &run_refs {
        assert!(!path.to_string_lossy().chars().any(char::is_control));
        assert!(
            path.is_dir(),
            "retained run reference does not exist: {}",
            path.display()
        );
        assert_eq!(fs::canonicalize(path).unwrap(), path.as_path());
        assert!(report_command.contains(&path.to_string_lossy().replace('\'', "'\\''")));
    }

    let report = execute_displayed_next(&journey, &swept);
    assert!(
        report.status.success(),
        "{}",
        String::from_utf8_lossy(&report.stdout)
    );
    let compare_command = displayed_next_shell(&report);
    assert!(compare_command.starts_with("asb compare "));
    let compared = execute_displayed_next(&journey, &report);
    assert!(compared.status.success());
    let compared_text = String::from_utf8(compared.stdout).unwrap();
    assert!(compared_text.starts_with("ASB compared the runs"));

    let tui = human_asb(
        &journey,
        &[
            "tui".to_owned(),
            "status".to_owned(),
            "--channel".to_owned(),
            "dev".to_owned(),
        ],
    );
    let tui_text = String::from_utf8(tui.stdout).unwrap();
    assert!(!tui_text.starts_with('{'));
    assert!(tui_text.starts_with("ASB "));
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
    let mut document: toml::Value = toml::from_str(&fs::read_to_string(&plan).unwrap()).unwrap();
    document["experiment"]["experiment_sha256"] = toml::Value::String("0".repeat(64));
    fs::write(&plan, toml::to_string(&document).unwrap()).unwrap();
    let output = asb(&["plan", plan.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(3));
    assert!(!results.exists());

    let (plan, _, _) = write_plan(&scratch.0.join("mismatched-agent"), "guide-mismatch", false);
    let mut document: toml::Value = toml::from_str(&fs::read_to_string(&plan).unwrap()).unwrap();
    document["agent"]["executable_sha256"] = toml::Value::String("f".repeat(64));
    fs::write(&plan, toml::to_string(&document).unwrap()).unwrap();
    let output = asb(&["plan", plan.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(3));
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
        let row = format!("| `{}` | supported", command.as_str().unwrap());
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

#[test]
fn record_replay_tutorial_is_synthetic_and_fail_closed() {
    let guide = include_str!("../../../docs/workflows/record-replay.md");
    let cassette: Value =
        serde_json::from_str(include_str!("../../asb-replay/fixtures/v1/buffered.json")).unwrap();

    assert!(guide.contains("Offline tutorial contract"));
    assert!(guide.contains("assert network == denied"));
    assert!(guide.contains("assert provider_fallback == forbidden"));
    assert!(guide.contains("no live-provider fallback"));
    assert!(guide.contains("selector_sha256"));
    assert!(guide.contains("fixture.integrity.digest"));
    assert!(guide.contains("dialect == synthetic"));
    assert!(!guide.contains("curl "));
    assert!(!guide.contains("wget "));
    assert!(!guide.contains("OPENAI_API_KEY"));
    assert_eq!(cassette["contents"]["schema_version"], 1);
    assert_eq!(
        cassette["contents"]["interactions"][0]["dialect"],
        "synthetic"
    );
    assert!(cassette["contents"]["redaction"]["selector_sha256"].is_string());
    assert!(
        cassette["integrity"]["digest"]
            .as_str()
            .is_some_and(|digest| {
                digest.len() == 64
                    && digest
                        .chars()
                        .all(|character| character.is_ascii_hexdigit())
            })
    );
    assert_eq!(cassette["integrity"]["algorithm"], "sha256");

    let bytes = include_bytes!("../../asb-replay/fixtures/v1/buffered.json");
    let authenticated = decode_cassette(bytes, CassetteLimits::default()).unwrap();
    let canonical = canonical_contents_bytes(&authenticated.contents).unwrap();
    let expected = format!("{:x}", Sha256::digest(canonical));
    assert_eq!(authenticated.integrity.digest, expected);

    let mut malformed = cassette;
    malformed["contents"]["unexpected"] = Value::Bool(true);
    let malformed = serde_json::to_vec(&malformed).unwrap();
    assert!(decode_cassette(&malformed, CassetteLimits::default()).is_err());
}
