// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Cross-repository conformance for the standalone asb-tui capability parser.

use asb_cli::capabilities::{CapabilityResponse, MAX_CAPABILITY_RESPONSE_BYTES, capability_schema};
use asb_control::{
    AttemptId, CancelParams, ControlCall, LaunchParams, MutationParams, PageParams, RepeatParams,
    Revision, RunId,
};
use serde_json::Value;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

const FIXTURE: &[u8] = include_bytes!("../fixtures/asb-tui-capabilities-v1.json");
const SCHEMA: &str = include_str!("../schema/v1/capabilities.schema.json");
const PROVENANCE: &str = include_str!("../fixtures/asb-tui-capabilities-v1.provenance.json");
const MAX_COVERAGE_SINK_BYTES: usize = 4_096;
static TEMP_DIRECTORY_SEQUENCE: AtomicUsize = AtomicUsize::new(0);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        loop {
            let sequence = TEMP_DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "asb-capability-coverage-{}-{sequence}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("create isolated coverage directory: {error}"),
            }
        }
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).expect("remove isolated coverage directory");
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("asb-cli must remain beneath the workspace crates directory")
        .to_path_buf()
}

fn has_symlinked_ancestor(path: &Path) -> bool {
    let mut prefix = PathBuf::new();
    for component in path.components() {
        prefix.push(component);
        match fs::symlink_metadata(&prefix) {
            Ok(metadata) if metadata.file_type().is_symlink() => return true,
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(_) => return true,
        }
    }
    false
}

fn valid_coverage_file_pattern(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut index = 0;
    let mut pid_tokens = 0;
    let mut merge_tokens = 0;
    while index < bytes.len() {
        if bytes[index] != b'%' {
            index += 1;
            continue;
        }
        index += 1;
        match bytes.get(index) {
            Some(b'p') => {
                pid_tokens += 1;
                if pid_tokens > 1 {
                    return false;
                }
                index += 1;
            }
            Some(b'm') if merge_tokens == 0 => {
                merge_tokens += 1;
                index += 1;
            }
            Some(b'1'..=b'9') => {
                let mut pool_size = 0_u8;
                while let Some(digit @ b'0'..=b'9') = bytes.get(index) {
                    pool_size = match pool_size
                        .checked_mul(10)
                        .and_then(|size| size.checked_add(digit - b'0'))
                    {
                        Some(size) if size <= 32 => size,
                        _ => return false,
                    };
                    index += 1;
                }
                if bytes.get(index) != Some(&b'm') || merge_tokens == 1 {
                    return false;
                }
                merge_tokens += 1;
                index += 1;
            }
            _ => return false,
        }
    }
    pid_tokens == 1
}

fn validated_coverage_sink(value: &OsStr) -> Option<OsString> {
    let text = value.to_str()?;
    if text.is_empty()
        || text.len() > MAX_COVERAGE_SINK_BYTES
        || text.chars().any(char::is_control)
        || !text.ends_with(".profraw")
    {
        return None;
    }

    let path = Path::new(value);
    if !path.is_absolute()
        || path.components().any(|part| part == Component::ParentDir)
        || has_symlinked_ancestor(path)
    {
        return None;
    }
    let file_name = path.file_name()?.to_str()?;
    if !valid_coverage_file_pattern(file_name) {
        return None;
    }
    let parent = fs::canonicalize(path.parent()?).ok()?;
    let workspace = fs::canonicalize(workspace_root()).ok()?;
    let workspace_target = workspace.join("target");
    if parent.starts_with(&workspace) && !parent.starts_with(workspace_target) {
        return None;
    }
    // LLVM's profile runtime accepts only a path string, not a caller-owned directory FD. Passing
    // the canonical parent closes symlink aliases observed during validation and bounds the
    // remaining race to a same-user process renaming that canonical directory before child exec.
    // Such a process already has authority to alter this test checkout; untrusted repository input
    // cannot select the sink, and paths resolving into source directories are rejected above.
    Some(parent.join(path.file_name()?).into_os_string())
}

fn isolated_asb_command_with_sink(sink: Option<&OsStr>) -> Result<Command, ()> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_asb"));
    command.env_clear();
    if let Some(sink) = sink {
        let sink = validated_coverage_sink(sink).ok_or(())?;
        command.env("LLVM_PROFILE_FILE", &sink);
    }
    Ok(command)
}

fn isolated_asb_command() -> Command {
    isolated_asb_command_with_sink(std::env::var_os("LLVM_PROFILE_FILE").as_deref())
        .expect("LLVM_PROFILE_FILE must name a validated absolute per-process profraw sink")
}

fn run_capabilities(arguments: &[&str]) -> Output {
    isolated_asb_command()
        .arg("capabilities")
        .args(arguments)
        .output()
        .unwrap()
}

fn default_profiles_beneath(path: &Path, found: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(path).unwrap() {
        let entry = entry.unwrap();
        let file_type = entry.file_type().unwrap();
        if file_type.is_dir() {
            default_profiles_beneath(&entry.path(), found);
        } else if file_type.is_file()
            && entry.file_name().to_str().is_some_and(|name| {
                name == "default.profraw"
                    || (name.starts_with("default_") && name.ends_with(".profraw"))
            })
        {
            found.push(entry.path());
        }
    }
}

fn assert_checkout_has_no_default_profiles() {
    let mut found = Vec::new();
    default_profiles_beneath(&workspace_root(), &mut found);
    assert!(
        found.is_empty(),
        "unexpected checkout coverage files: {found:?}"
    );
}

#[test]
fn executable_emits_the_exact_standalone_frontend_fixture() {
    let output = run_capabilities(&["--format", "json"]);
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(output.stdout, FIXTURE);
    assert!(output.stdout.len() <= MAX_CAPABILITY_RESPONSE_BYTES);
    assert!(CapabilityResponse::parse(&output.stdout).is_ok());
}

#[test]
fn generated_schema_matches_checked_asb_tui_contract() {
    let checked: Value = serde_json::from_str(SCHEMA).unwrap();
    assert_eq!(capability_schema(), checked);
    let validator = jsonschema::validator_for(&checked).unwrap();
    let fixture: Value = serde_json::from_slice(FIXTURE).unwrap();
    assert!(validator.is_valid(&fixture));
}

#[test]
fn checked_schema_is_digest_bound_to_the_published_asb_tui_revision() {
    let provenance: Value = serde_json::from_str(PROVENANCE).unwrap();
    assert_eq!(
        provenance["asb_tui_revision"],
        "3d2b6b537da817469bf8a39841ccf9a79a4c370f"
    );
    assert_eq!(
        provenance["schema_path"],
        "protocol/v1/capabilities.schema.json"
    );
    assert_eq!(
        format!("{:x}", Sha256::digest(SCHEMA.as_bytes())),
        provenance["schema_sha256"].as_str().unwrap()
    );
}

#[test]
fn every_advertised_boolean_has_an_authoritative_control_v1_method() {
    let calls = [
        ControlCall::ValidateSettings {
            settings: json!({}),
        },
        ControlCall::CreatePlan(MutationParams {
            idempotency_key: "capability-plan".into(),
            definition: json!({}),
        }),
        ControlCall::Launch(LaunchParams {
            idempotency_key: "capability-launch".into(),
            plan_id: "plan-capability".into(),
        }),
        ControlCall::Events(PageParams {
            after: Some(Revision(0)),
            limit: 1,
        }),
        ControlCall::Cancel(CancelParams {
            run_id: RunId("run-capability".into()),
            attempt_id: AttemptId("attempt-capability".into()),
            idempotency_key: "capability-cancel".into(),
        }),
        ControlCall::History(PageParams {
            after: None,
            limit: 1,
        }),
        ControlCall::Repeat(RepeatParams {
            run_id: RunId("run-capability".into()),
            idempotency_key: "capability-repeat".into(),
        }),
        ControlCall::Analyze {
            run_ids: vec![RunId("run-capability".into())],
        },
        ControlCall::ArtifactMetadata {
            run_id: RunId("run-capability".into()),
            digest: "0".repeat(64),
        },
    ];
    assert!(matches!(calls[0], ControlCall::ValidateSettings { .. }));
    assert!(matches!(calls[1], ControlCall::CreatePlan(_)));
    assert!(matches!(calls[2], ControlCall::Launch(_)));
    assert!(matches!(calls[3], ControlCall::Events(_)));
    assert!(matches!(calls[4], ControlCall::Cancel(_)));
    assert!(matches!(calls[5], ControlCall::History(_)));
    assert!(matches!(calls[6], ControlCall::Repeat(_)));
    assert!(matches!(calls[7], ControlCall::Analyze { .. }));
    assert!(matches!(calls[8], ControlCall::ArtifactMetadata { .. }));
}

#[test]
fn command_rejects_every_noncanonical_invocation_without_side_effects() {
    for arguments in [
        Vec::<&str>::new(),
        vec!["--format"],
        vec!["--format", "yaml"],
        vec!["json"],
        vec!["--format", "json", "extra"],
    ] {
        let output = run_capabilities(&arguments);
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stderr.is_empty());
        let error: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(error["command"], "capabilities");
        assert_eq!(error["error"]["code"], "usage");
    }
}

#[test]
fn isolated_children_forward_only_a_valid_external_coverage_sink() {
    let valid = OsStr::new("/tmp/asb-capability-%p-%8m.profraw");
    let command = isolated_asb_command_with_sink(Some(valid)).unwrap();
    let variables: Vec<_> = command.get_envs().collect();
    assert_eq!(
        variables,
        vec![(OsStr::new("LLVM_PROFILE_FILE"), Some(valid))]
    );

    let hosted = workspace_root()
        .join("target")
        .join("agent-systems-benchmark-%p-%32m.profraw");
    fs::create_dir_all(hosted.parent().unwrap()).unwrap();
    let command = isolated_asb_command_with_sink(Some(hosted.as_os_str())).unwrap();
    assert_eq!(
        command.get_envs().collect::<Vec<_>>(),
        vec![(OsStr::new("LLVM_PROFILE_FILE"), Some(hosted.as_os_str()))]
    );

    let checkout_sink = workspace_root().join("default_%p.profraw");
    for malformed in [
        OsStr::new(""),
        OsStr::new("relative.profraw"),
        OsStr::new("/tmp/../checkout.profraw"),
        OsStr::new("/tmp/control\n.profraw"),
        OsStr::new("/tmp/shared.profraw"),
        OsStr::new("/tmp/asb-capability-%p.txt"),
        checkout_sink.as_os_str(),
    ] {
        assert!(isolated_asb_command_with_sink(Some(malformed)).is_err());
    }

    let oversized = format!("/tmp/{}.profraw", "a".repeat(MAX_COVERAGE_SINK_BYTES));
    assert!(isolated_asb_command_with_sink(Some(OsStr::new(&oversized))).is_err());
    assert!(
        isolated_asb_command_with_sink(None)
            .unwrap()
            .get_envs()
            .next()
            .is_none()
    );

    let symlink_root =
        std::env::temp_dir().join(format!("asb-capability-sink-{}", std::process::id()));
    let real_root = symlink_root.join("real");
    let linked_root = symlink_root.join("linked");
    fs::create_dir_all(&real_root).unwrap();
    std::os::unix::fs::symlink(&real_root, &linked_root).unwrap();
    let symlink_sink = linked_root.join("coverage-%p.profraw");
    assert!(isolated_asb_command_with_sink(Some(symlink_sink.as_os_str())).is_err());
    fs::remove_file(linked_root).unwrap();
    fs::remove_dir(real_root).unwrap();
    fs::remove_dir(symlink_root).unwrap();
}

#[test]
fn coverage_pattern_parser_rejects_runtime_fallback_and_shared_forms() {
    if let Some(hosted) = std::env::var_os("LLVM_PROFILE_FILE") {
        assert!(
            validated_coverage_sink(&hosted).is_some(),
            "hosted coverage sink rejected: {hosted:?}"
        );
    }
    let malformed = [
        "/tmp/coverage-%%p.profraw",
        "/tmp/coverage-%%p-%32m.profraw",
        "/tmp/coverage-%p-%0m.profraw",
        "/tmp/coverage-%p-%032m.profraw",
        "/tmp/coverage-%p-%33m.profraw",
        "/tmp/coverage-%p-%32m-%2m.profraw",
        "/tmp/coverage-%p-%m-%m.profraw",
        "/tmp/coverage-%p-%m-%2m.profraw",
        "/tmp/coverage-%p-%p.profraw",
        "/tmp/coverage-%t-%p.profraw",
        "/tmp/coverage-%h-%p.profraw",
        "/tmp/coverage-%b-%p.profraw",
        "/tmp/coverage-%c-%p.profraw",
        "/tmp/coverage-%q-%p.profraw",
        "/tmp/coverage-%999999999999999999999999999999m-%p.profraw",
    ];
    for value in malformed {
        assert!(
            isolated_asb_command_with_sink(Some(OsStr::new(value))).is_err(),
            "{value}"
        );
    }
    for valid in [
        "/tmp/coverage-%p.profraw",
        "/tmp/coverage-%p-%m.profraw",
        "/tmp/coverage-%p-%1m.profraw",
        "/tmp/coverage-%p-%32m.profraw",
    ] {
        assert!(
            isolated_asb_command_with_sink(Some(OsStr::new(valid))).is_ok(),
            "{valid}"
        );
    }
    assert_checkout_has_no_default_profiles();
}

#[test]
fn artifact_scan_catches_both_llvm_default_filename_forms() {
    let directory = TestDirectory::new();
    fs::write(directory.0.join("default.profraw"), []).unwrap();
    fs::write(directory.0.join("default_123.profraw"), []).unwrap();
    fs::write(directory.0.join("not-default.profraw"), []).unwrap();
    let mut found = Vec::new();
    default_profiles_beneath(&directory.0, &mut found);
    found.sort();
    assert_eq!(
        found,
        [
            directory.0.join("default.profraw"),
            directory.0.join("default_123.profraw"),
        ]
    );
}

#[test]
fn instrumented_canonical_and_failing_children_use_distinct_nondefault_profiles() {
    let coverage_enabled = std::env::var_os("LLVM_PROFILE_FILE").is_some();
    let directory = TestDirectory::new();
    let sink = directory.0.join("child-%p.profraw");
    let invocations = [
        vec!["--format", "json"],
        Vec::new(),
        vec!["--format"],
        vec!["--format", "yaml"],
        vec!["json"],
        vec!["--format", "json", "extra"],
    ];
    let outputs = std::thread::scope(|scope| {
        let handles: Vec<_> = invocations
            .iter()
            .map(|arguments| {
                let sink = &sink;
                scope.spawn(move || {
                    isolated_asb_command_with_sink(Some(sink.as_os_str()))
                        .unwrap()
                        .arg("capabilities")
                        .args(arguments)
                        .output()
                        .unwrap()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert!(outputs[0].status.success());
    for output in &outputs[1..] {
        assert_eq!(output.status.code(), Some(2));
    }

    let profiles: BTreeSet<_> = fs::read_dir(&directory.0)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    if coverage_enabled {
        assert_eq!(profiles.len(), invocations.len());
        assert!(profiles.iter().all(|name| {
            name.to_str().is_some_and(|name| {
                name.starts_with("child-")
                    && name.ends_with(".profraw")
                    && name != "default.profraw"
                    && !name.starts_with("default_")
            })
        }));
    } else {
        assert!(profiles.is_empty());
    }
    assert_checkout_has_no_default_profiles();
}

#[test]
fn canonical_and_failing_children_are_parallel_safe_and_leave_checkout_clean() {
    assert_checkout_has_no_default_profiles();
    let invocations = [
        vec!["--format", "json"],
        Vec::new(),
        vec!["--format"],
        vec!["--format", "yaml"],
        vec!["json"],
        vec!["--format", "json", "extra"],
    ];
    let outputs = std::thread::scope(|scope| {
        let handles: Vec<_> = invocations
            .iter()
            .map(|arguments| scope.spawn(move || run_capabilities(arguments)))
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert!(outputs[0].status.success());
    assert_eq!(outputs[0].stdout, FIXTURE);
    for output in &outputs[1..] {
        assert_eq!(output.status.code(), Some(2));
    }
    assert_checkout_has_no_default_profiles();
}

#[test]
fn command_ignores_hostile_environment_and_help_completion_are_explicit() {
    let output = Command::new(env!("CARGO_BIN_EXE_asb"))
        .args(["capabilities", "--format", "json"])
        .env("ASB_SECRET_SENTINEL", "must-not-appear")
        .env("USER", "private-user")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, FIXTURE);
    assert!(!output.stdout.windows(7).any(|part| part == b"private"));

    let help = Command::new(env!("CARGO_BIN_EXE_asb"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(
        String::from_utf8(help.stdout)
            .unwrap()
            .contains("asb capabilities --format json")
    );
    let completion = Command::new(env!("CARGO_BIN_EXE_asb"))
        .args(["completion", "bash"])
        .output()
        .unwrap();
    assert!(
        String::from_utf8(completion.stdout)
            .unwrap()
            .contains("doctor capabilities provider-catalog")
    );
}

#[test]
fn closed_parser_and_schema_reject_contract_drift() {
    let validator =
        jsonschema::validator_for(&serde_json::from_str::<Value>(SCHEMA).unwrap()).unwrap();
    let valid = String::from_utf8(FIXTURE.to_vec()).unwrap();
    let duplicate = valid.replace("\"analysis\":true", "\"analysis\":true,\"analysis\":false");
    assert!(CapabilityResponse::parse(duplicate.as_bytes()).is_err());
    let invalid = [
        valid.replace("\"analysis\":true", "\"analysis\":1"),
        valid.replace("\"repeat\":true", "\"repeat\":true,\"future\":false"),
        valid.replace("\"repeat\":true", "\"future\":false"),
        valid.replace("asb-cli-capabilities", "mutable-latest"),
        valid.replace("\"protocol_version\":1", "\"protocol_version\":2"),
    ];
    for input in invalid {
        assert!(CapabilityResponse::parse(input.as_bytes()).is_err());
        let value: Value = serde_json::from_str(&input).unwrap_or(Value::Null);
        assert!(!validator.is_valid(&value));
    }
}
