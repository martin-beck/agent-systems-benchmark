// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Executable-level human-output contract tests.

use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};
use unicode_width::UnicodeWidthStr;

static NONCE: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "asb-human-cli-{}-{}",
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

fn asb() -> Command {
    Command::new(env!("CARGO_BIN_EXE_asb"))
}

fn isolated_asb(root: &Path) -> Command {
    let mut command = asb();
    command
        .env("HOME", root)
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_STATE_HOME", root.join("state"))
        .env("XDG_DATA_HOME", root.join("data"))
        .env("LANG", "C.UTF-8")
        .env("LC_ALL", "C.UTF-8")
        .env("COLUMNS", "80");
    command
}

fn create_public_plan(root: &Path, name: &str, executable: &Path, timeout_ms: u64) -> PathBuf {
    let plan = root.join(format!("{name}.toml"));
    let output = isolated_asb(root)
        .args([
            "--json",
            "plan",
            "create",
            "--workload",
            "original.bug-fix",
            "--agent",
            "codex",
            "--agent-executable",
        ])
        .arg(executable)
        .args(["--run-id", name, "--result-root"])
        .arg(root.join(format!("{name}-results")))
        .arg("--work-root")
        .arg(root.join(format!("{name}-work")))
        .args(["--output"])
        .arg(&plan)
        .args(["--timeout-ms", &timeout_ms.to_string()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    plan
}

fn assert_private_human_output(output: &std::process::Output) -> String {
    assert!(!output.stdout.contains(&0x1b));
    let stdout = String::from_utf8(output.stdout.clone()).unwrap();
    assert!(!stdout.starts_with('{'));
    assert!(!stdout.contains("schema_version"));
    assert!(!stdout.contains("credential_reference_sha256"));
    stdout
}

fn assert_golden_outcome(scenario: &str, output: &std::process::Output) {
    let row = include_str!("../fixtures/human/outcome-matrix-v1.txt")
        .lines()
        .filter(|line| !line.starts_with('#'))
        .find(|line| line.split('\t').next() == Some(scenario))
        .unwrap_or_else(|| panic!("missing golden scenario {scenario}"));
    let fields = row.split('\t').collect::<Vec<_>>();
    assert_eq!(fields.len(), 4);
    assert_eq!(output.status.code(), Some(fields[1].parse().unwrap()));
    let normalized = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(normalized.contains(fields[2]), "{scenario}: {normalized}");
    assert_eq!(output.stderr.is_empty(), fields[3] == "empty", "{scenario}");
}

#[test]
fn redirected_output_honors_40_80_and_120_column_widths_and_stream_boundaries() {
    for width in [40, 80, 120] {
        let output = asb()
            .arg("provider-catalog")
            .env("COLUMNS", width.to_string())
            .output()
            .expect("ASB executable must run");
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(stdout.starts_with("ASB loaded the provider catalog."));
        assert!(
            stdout
                .lines()
                .all(|line| UnicodeWidthStr::width(line) <= width),
            "width={width}: {stdout}"
        );
    }
}

#[test]
fn unicode_and_combining_output_obeys_terminal_display_columns() {
    for width in [40, 80, 120] {
        let path = std::env::temp_dir().join(format!(
            "asb-\u{6e2c}\u{8a66}-e\u{301}-{}-{}.json",
            std::process::id(),
            width
        ));
        let output = asb()
            .args(["setup", "--output"])
            .arg(&path)
            .env("COLUMNS", width.to_string())
            .output()
            .expect("ASB executable must run");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(stdout.contains("\u{6e2c}\u{8a66}"));
        assert!(
            stdout
                .lines()
                .filter(|line| !line.starts_with("Next:"))
                .all(|line| UnicodeWidthStr::width(line) <= width)
        );
        std::fs::remove_file(path).unwrap();
    }
}

#[test]
fn local_mock_validation_failure_is_human_and_preserves_exit_three() {
    let output = asb()
        .args(["run", "/definitely/missing/asb-plan.toml", "--local-mock"])
        .output()
        .expect("ASB executable must run");
    assert_eq!(output.status.code(), Some(3));
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.starts_with("ASB could not complete the benchmark run:"));
    assert!(!stdout.starts_with('{'));
    assert!(!stdout.contains("could not write"));
}

#[test]
fn serve_progress_is_neutral_and_does_not_leak_or_inject_the_path() {
    let private = "/tmp/private\n\u{1b}[31m-control.toml";
    let output = asb()
        .args(["serve", private])
        .output()
        .expect("ASB executable must run");
    assert_eq!(output.status.code(), Some(3));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(
        stderr,
        "ASB is attempting to start the control service from the supplied configuration.\n"
    );
    assert!(!stderr.contains("private"));
    assert!(!stderr.contains('\u{1b}'));
}

#[test]
fn no_color_explicitly_suppresses_terminal_escape_sequences() {
    let output = asb()
        .arg("doctor")
        .env("NO_COLOR", "1")
        .output()
        .expect("ASB executable must run");
    assert!(output.status.success());
    assert!(!output.stdout.contains(&0x1b));
    assert!(!output.stderr.contains(&0x1b));
}

#[test]
fn executable_usage_failure_is_human_on_stdout_and_preserves_exit_two() {
    let output = asb()
        .arg("not-a-command")
        .output()
        .expect("ASB executable must run");
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.starts_with("ASB could not complete the command:"));
    assert!(stdout.ends_with("Next: asb --help\n"));
}

#[test]
fn executable_json_alias_is_machine_readable_and_not_styled() {
    let output = asb()
        .args(["--format", "json", "doctor"])
        .env("NO_COLOR", "1")
        .output()
        .expect("ASB executable must run");
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert!(!output.stdout.contains(&0x1b));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["command"], "doctor");
}

#[test]
fn executable_capabilities_accepts_both_json_selector_spellings() {
    for arguments in [
        &["--json", "capabilities"][..],
        &["--format", "json", "capabilities"][..],
    ] {
        let output = asb()
            .args(arguments)
            .output()
            .expect("ASB executable must run");
        assert!(output.status.success(), "{arguments:?}");
        assert!(output.stderr.is_empty());
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["protocol"], "asb-cli-capabilities");
    }
}

#[test]
fn project_init_human_output_distinguishes_initialization_from_recovery() {
    let scratch = Scratch::new();
    let project_root = scratch.0.join("private-project-root");

    for (expected_outcome, unexpected_outcome) in [
        (
            "ASB initialized the project workspace.",
            "ASB recovered the existing project workspace.",
        ),
        (
            "ASB recovered the existing project workspace.",
            "ASB initialized the project workspace.",
        ),
    ] {
        let output = isolated_asb(&scratch.0)
            .env("COLUMNS", "240")
            .args(["project", "init"])
            .arg(&project_root)
            .output()
            .expect("ASB executable must run");
        assert!(output.status.success());
        if expected_outcome.starts_with("ASB initialized") {
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(stderr.contains("ASB will create directory"));
            assert!(stderr.contains("for the ASB project workspace."));
        } else {
            assert!(output.stderr.is_empty());
        }
        let stdout = assert_private_human_output(&output);
        assert_eq!(stdout.lines().next(), Some(expected_outcome));
        assert!(!stdout.contains(unexpected_outcome));
        assert!(stdout.contains("Configuration: .asb/project.json."));
        assert!(stdout.contains("Results directory: results."));
        assert!(stdout.contains("Catalog directory: catalogs."));
        assert_eq!(
            stdout
                .lines()
                .filter(|line| line.starts_with("Next:"))
                .collect::<Vec<_>>(),
            vec!["Next: asb provider-catalog"]
        );
        assert!(!stdout.contains(project_root.to_str().unwrap()));
        assert!(!stdout.contains(scratch.0.to_str().unwrap()));
        assert!(!stdout.contains("initialized\":"));
        assert!(!stdout.contains("recovered\":"));
    }
}

#[test]
fn executable_outcome_matrix_covers_product_dependency_host_failed_and_inconclusive() {
    let scratch = Scratch::new();

    let product = isolated_asb(&scratch.0)
        .args([
            "auth",
            "status",
            "--provider",
            "openrouter",
            "--socket",
            "/definitely/missing/asb.sock",
        ])
        .output()
        .unwrap();
    assert_golden_outcome("product", &product);
    assert_eq!(product.status.code(), Some(4));
    assert!(product.stderr.is_empty());
    let product_text = assert_private_human_output(&product);
    assert!(
        product_text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .contains("auth control service connection failed")
    );

    let capture = scratch.0.join("capture.json");
    fs::write(
        &capture,
        serde_json::to_vec(&serde_json::json!({
            "schema_version":1,
            "provider_profile_sha256":"a".repeat(64),
            "agent_id":"codex",
            "request_body":{
                "model":"cohere/north-mini-code:free",
                "messages":[{"role":"user","content":"bounded test"}]
            }
        }))
        .unwrap(),
    )
    .unwrap();
    let dependency = isolated_asb(&scratch.0)
        .env_remove("OPENROUTER_API_KEY")
        .args(["record-live"])
        .arg(&capture)
        .arg(scratch.0.join("capture.cassette.json"))
        .args(["--openrouter", "--confirm-record"])
        .output()
        .unwrap();
    assert_golden_outcome("dependency", &dependency);
    assert_eq!(dependency.status.code(), Some(4));
    assert!(dependency.stderr.is_empty());
    let dependency_text = assert_private_human_output(&dependency);
    assert!(
        dependency_text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .contains("OpenRouter credential unavailable")
    );
    assert!(dependency_text.contains("OPENROUTER_API_KEY"));
    assert!(!dependency_text.contains("Next:"));

    let host = isolated_asb(&scratch.0)
        .env("ASB_DEV_RUSTUP_HOME", "/definitely/missing/asb-rustup")
        .args(["tui", "preflight"])
        .output()
        .unwrap();
    assert_golden_outcome("host", &host);
    assert_eq!(host.status.code(), Some(4));
    assert!(host.stderr.is_empty());
    let host_text = assert_private_human_output(&host);
    assert!(host_text.contains("terminal interface preflight"));

    let failed_plan = create_public_plan(&scratch.0, "failed", Path::new("/bin/false"), 1_000);
    let failed = isolated_asb(&scratch.0)
        .arg("run")
        .arg(&failed_plan)
        .output()
        .unwrap();
    assert_golden_outcome("failed", &failed);
    assert_eq!(failed.status.code(), Some(5));
    let failed_text = assert_private_human_output(&failed);
    assert!(failed_text.contains("benchmark points failed"));
    assert!(failed_text.contains("Next: asb report "));

    let absent_interpreter = scratch.0.join("absent-interpreter-agent");
    fs::write(
        &absent_interpreter,
        "#!/definitely/missing/asb-interpreter\n",
    )
    .unwrap();
    fs::set_permissions(&absent_interpreter, fs::Permissions::from_mode(0o700)).unwrap();
    let inconclusive_plan =
        create_public_plan(&scratch.0, "inconclusive", &absent_interpreter, 1_000);
    let inconclusive = isolated_asb(&scratch.0)
        .arg("run")
        .arg(&inconclusive_plan)
        .output()
        .unwrap();
    assert_golden_outcome("inconclusive", &inconclusive);
    assert_eq!(inconclusive.status.code(), Some(6));
    let inconclusive_text = assert_private_human_output(&inconclusive);
    assert!(inconclusive_text.contains("result is inconclusive"));
    assert!(inconclusive_text.contains("Next: asb report "));
}

#[test]
fn executable_golden_matrix_covers_zero_two_and_three() {
    let scratch = Scratch::new();
    let success = isolated_asb(&scratch.0).arg("doctor").output().unwrap();
    assert_golden_outcome("success", &success);
    let usage = isolated_asb(&scratch.0)
        .arg("not-a-command")
        .output()
        .unwrap();
    assert_golden_outcome("usage", &usage);
    let validation = isolated_asb(&scratch.0)
        .args(["run", "/definitely/missing/asb-plan.toml", "--local-mock"])
        .output()
        .unwrap();
    assert_golden_outcome("validation", &validation);
}

#[test]
fn executable_public_family_golden_is_inventory_complete_and_privacy_safe() {
    let scratch = Scratch::new();
    let secret = "asb-private-secret-sentinel";
    let fixture = include_str!("../fixtures/human/public-family-output-v1.tsv");
    let mut observed = Vec::new();
    for row in fixture.lines().filter(|line| !line.starts_with('#')) {
        let fields = row.split('\t').collect::<Vec<_>>();
        assert_eq!(fields.len(), 5, "malformed golden row: {row}");
        let command_name = fields[0];
        observed.push(command_name);
        let mut command = isolated_asb(&scratch.0);
        command
            .arg(command_name)
            .env("COLUMNS", "240")
            .env("OPENROUTER_API_KEY", secret);
        if fields[1] == "{scratch}/project" {
            command.arg("init").arg(scratch.0.join("project"));
        } else if fields[1] != "-" {
            command.args(fields[1].split_ascii_whitespace());
        }
        let output = command.output().unwrap();
        assert_eq!(
            output.status.code(),
            Some(fields[2].parse().unwrap()),
            "{command_name}: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert_eq!(
            stdout.lines().next(),
            Some(fields[3]),
            "public family {command_name}"
        );
        assert_eq!(output.stderr.is_empty(), fields[4] == "empty");
        assert!(!stdout.contains(secret));
        assert!(!stdout.contains(scratch.0.to_str().unwrap()));
        assert!(!stdout.contains("schema_version"));
        assert!(!stdout.contains("credential_reference_sha256"));
        assert!(!stdout.contains('\u{1b}'));
    }
    observed.sort_unstable();
    observed.dedup();
    assert_eq!(observed.len(), 26);
}

#[test]
fn serve_streams_startup_before_blocking_and_remains_a_raw_long_running_command() {
    let scratch = Scratch::new();
    let config = scratch.0.join("control.toml");
    let control_endpoint = scratch.0.join("control-endpoint");
    let provisioning_endpoint = scratch.0.join("provisioning-endpoint");
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&control_endpoint)
        .unwrap();
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&provisioning_endpoint)
        .unwrap();
    let socket = control_endpoint.join("control.sock");
    let provisioning = provisioning_endpoint.join("provision.sock");
    let state = scratch.0.join("control-state");
    fs::write(
        &config,
        format!(
            "schema_version = 1\nsocket_path = {:?}\nprovisioning_socket_path = {:?}\nstate_root = {:?}\n",
            socket, provisioning, state
        ),
    )
    .unwrap();
    let mut child = isolated_asb(&scratch.0)
        .arg("serve")
        .arg(&config)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut startup = String::new();
    let mut stderr = BufReader::new(child.stderr.take().unwrap());
    stderr.read_line(&mut startup).unwrap();
    assert_eq!(
        startup,
        "ASB is attempting to start the control service from the supplied configuration.\n"
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while !socket.exists() {
        if let Some(status) = child.try_wait().unwrap() {
            let mut rest = String::new();
            stderr.read_to_string(&mut rest).unwrap();
            let mut result = String::new();
            child
                .stdout
                .take()
                .unwrap()
                .read_to_string(&mut result)
                .unwrap();
            panic!("serve exited before binding ({status}): {rest}{result}");
        }
        assert!(Instant::now() < deadline, "serve did not bind its socket");
        thread::sleep(Duration::from_millis(5));
    }
    assert!(child.try_wait().unwrap().is_none());
    assert!(
        Command::new("/bin/kill")
            .args(["-TERM", &child.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    let status = child.wait().unwrap();
    assert!(!status.success());
    let mut stdout = child.stdout.take().unwrap();
    let mut bytes = Vec::new();
    stdout.read_to_end(&mut bytes).unwrap();
    assert!(bytes.is_empty());
}

#[test]
fn plan_create_new_options_are_discoverable_and_fail_closed() {
    let help = asb().arg("--help").output().unwrap();
    let help = String::from_utf8(help.stdout).unwrap();
    for option in ["--agent AGENT", "--use-config", "--sweep-max-concurrency N"] {
        assert!(help.contains(option), "missing {option}: {help}");
    }

    let scratch = Scratch::new();
    let common = [
        "plan",
        "create",
        "--workload",
        "original.bug-fix",
        "--agent-executable",
        "/bin/false",
        "--output",
    ];
    let missing_agent_path = scratch.0.join("missing-agent.toml");
    let missing_agent = isolated_asb(&scratch.0)
        .args(common)
        .arg(&missing_agent_path)
        .arg("--use-config")
        .output()
        .unwrap();
    assert_eq!(missing_agent.status.code(), Some(2));
    assert!(!missing_agent_path.exists());
    assert!(assert_private_human_output(&missing_agent).contains("requires --agent"));

    let duplicate_path = scratch.0.join("duplicate-agent.toml");
    let duplicate = isolated_asb(&scratch.0)
        .args(common)
        .arg(&duplicate_path)
        .args(["--agent", "codex", "--agent", "aider"])
        .output()
        .unwrap();
    assert_eq!(duplicate.status.code(), Some(2));
    assert!(!duplicate_path.exists());

    let zero_sweep_path = scratch.0.join("zero-sweep.toml");
    let zero_sweep = isolated_asb(&scratch.0)
        .args(common)
        .arg(&zero_sweep_path)
        .args(["--agent", "codex", "--sweep-max-concurrency", "0"])
        .output()
        .unwrap();
    assert_eq!(zero_sweep.status.code(), Some(3));
    assert!(!zero_sweep_path.exists());
}
