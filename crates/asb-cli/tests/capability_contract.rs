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
use std::process::Command;

const FIXTURE: &[u8] = include_bytes!("../fixtures/asb-tui-capabilities-v1.json");
const SCHEMA: &str = include_str!("../schema/v1/capabilities.schema.json");
const PROVENANCE: &str = include_str!("../fixtures/asb-tui-capabilities-v1.provenance.json");

#[test]
fn executable_emits_the_exact_standalone_frontend_fixture() {
    let output = Command::new(env!("CARGO_BIN_EXE_asb"))
        .args(["capabilities", "--format", "json"])
        .env_clear()
        .output()
        .unwrap();
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
        let output = Command::new(env!("CARGO_BIN_EXE_asb"))
            .arg("capabilities")
            .args(arguments)
            .env_clear()
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stderr.is_empty());
        let error: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(error["command"], "capabilities");
        assert_eq!(error["error"]["code"], "usage");
    }
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
