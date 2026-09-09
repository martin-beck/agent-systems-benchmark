// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Checked-in schemas and bounded public fixture conformance.

use asb_control::{
    AnalysisEvidence, ControlCall, ControlEvent, ControlLimits, ControlRequest, ControlResponse,
    HistoryEvidence, ProtocolError, analysis_evidence_schema, control_event_schema,
    control_request_schema, control_response_schema, history_evidence_schema, validate_request,
};
use serde_json::{Value, json};

const REQUEST_SCHEMA: &str = include_str!("../schema/v1/request.schema.json");
const RESPONSE_SCHEMA: &str = include_str!("../schema/v1/response.schema.json");
const EVENT_SCHEMA: &str = include_str!("../schema/v1/event.schema.json");
const NEGOTIATE: &str = include_str!("../fixtures/v1/negotiate-request.json");
const LAUNCH: &str = include_str!("../fixtures/v1/launch-request.json");
const RESPONSE: &str = include_str!("../fixtures/v1/success-response.json");
const EVENT: &str = include_str!("../fixtures/v1/run-event.json");
const HISTORY_EVIDENCE: &str = include_str!("../fixtures/v1/history-evidence.json");
const ANALYSIS_EVIDENCE: &str = include_str!("../fixtures/v1/analysis-evidence.json");
const HISTORY_SCHEMA: &str = include_str!("../schema/v1/history-evidence.schema.json");
const ANALYSIS_SCHEMA: &str = include_str!("../schema/v1/analysis-evidence.schema.json");

fn validate(schema: &str, document: &str) {
    let schema: Value = serde_json::from_str(schema).unwrap();
    let document: Value = serde_json::from_str(document).unwrap();
    jsonschema::validator_for(&schema)
        .unwrap()
        .validate(&document)
        .unwrap();
}

#[test]
fn public_fixtures_match_schemas_and_rust_types() {
    validate(REQUEST_SCHEMA, NEGOTIATE);
    validate(REQUEST_SCHEMA, LAUNCH);
    validate(RESPONSE_SCHEMA, RESPONSE);
    validate(EVENT_SCHEMA, EVENT);

    let negotiate: ControlRequest = serde_json::from_str(NEGOTIATE).unwrap();
    assert!(matches!(negotiate.call, ControlCall::Negotiate(_)));
    let launch: ControlRequest = serde_json::from_str(LAUNCH).unwrap();
    validate_request(&launch, ControlLimits::default()).unwrap();
    serde_json::from_str::<ControlResponse>(RESPONSE)
        .unwrap()
        .validate()
        .unwrap();
    serde_json::from_str::<ControlEvent>(EVENT).unwrap();
    validate(HISTORY_SCHEMA, HISTORY_EVIDENCE);
    validate(ANALYSIS_SCHEMA, ANALYSIS_EVIDENCE);
    serde_json::from_str::<HistoryEvidence>(HISTORY_EVIDENCE)
        .unwrap()
        .validate()
        .unwrap();
    serde_json::from_str::<AnalysisEvidence>(ANALYSIS_EVIDENCE)
        .unwrap()
        .validate()
        .unwrap();
}

#[test]
fn schema_and_rust_reject_unknown_or_ambiguous_envelopes() {
    let schema: Value = serde_json::from_str(REQUEST_SCHEMA).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let mut request: Value = serde_json::from_str(LAUNCH).unwrap();
    request["private_host_path"] = json!("/not/public");
    assert!(!validator.is_valid(&request));
    assert!(serde_json::from_value::<ControlRequest>(request).is_err());

    let mut wrong_rpc: Value = serde_json::from_str(LAUNCH).unwrap();
    wrong_rpc["jsonrpc"] = json!("1.0");
    let decoded: ControlRequest = serde_json::from_value(wrong_rpc).unwrap();
    assert_eq!(
        validate_request(&decoded, ControlLimits::default()),
        Err(ProtocolError::InvalidJsonRpc)
    );

    let response_schema: Value = serde_json::from_str(RESPONSE_SCHEMA).unwrap();
    let response_validator = jsonschema::validator_for(&response_schema).unwrap();
    let mut leaked: Value = serde_json::from_str(RESPONSE).unwrap();
    leaked["artifact_path"] = json!("/private/result");
    assert!(!response_validator.is_valid(&leaked));

    let mut missing_creation_cursor: Value = serde_json::from_str(RESPONSE).unwrap();
    missing_creation_cursor["result"]["value"]["result"]["value"]
        .as_object_mut()
        .unwrap()
        .remove("created_revision");
    assert!(!response_validator.is_valid(&missing_creation_cursor));

    let mut malformed_creation_cursor: Value = serde_json::from_str(RESPONSE).unwrap();
    malformed_creation_cursor["result"]["value"]["result"]["value"]["created_revision"] =
        json!("not-a-revision");
    assert!(!response_validator.is_valid(&malformed_creation_cursor));
}

#[test]
fn generated_schemas_reject_runtime_boundary_negatives() {
    let request_schema: Value = serde_json::from_str(REQUEST_SCHEMA).unwrap();
    let request_validator = jsonschema::validator_for(&request_schema).unwrap();
    let mut wrong_rpc: Value = serde_json::from_str(LAUNCH).unwrap();
    wrong_rpc["jsonrpc"] = json!("1.0");
    assert!(!request_validator.is_valid(&wrong_rpc));
    let mut bad_timeout: Value = serde_json::from_str(LAUNCH).unwrap();
    bad_timeout["timeout_ms"] = json!(0);
    assert!(!request_validator.is_valid(&bad_timeout));
    let mut bad_key: Value = serde_json::from_str(LAUNCH).unwrap();
    bad_key["params"]["idempotency_key"] = json!("../private");
    assert!(!request_validator.is_valid(&bad_key));

    let response_schema: Value = serde_json::from_str(RESPONSE_SCHEMA).unwrap();
    let response_validator = jsonschema::validator_for(&response_schema).unwrap();
    let mut missing_creation_cursor: Value = serde_json::from_str(RESPONSE).unwrap();
    missing_creation_cursor["result"]["value"]["result"]["value"]
        .as_object_mut()
        .unwrap()
        .remove("created_revision");
    assert!(!response_validator.is_valid(&missing_creation_cursor));
    let mut malformed_creation_cursor: Value = serde_json::from_str(RESPONSE).unwrap();
    malformed_creation_cursor["result"]["value"]["result"]["value"]["created_revision"] =
        json!("not-a-revision");
    assert!(!response_validator.is_valid(&malformed_creation_cursor));
    let contradictory_settings = json!({
        "jsonrpc": "2.0",
        "id": 7,
        "result": {
            "kind": "operation",
            "value": {
                "request_sha256": "0".repeat(64),
                "result": {
                    "kind": "settings_validation",
                    "value": {
                        "valid": true,
                        "issues": ["invalid_format"]
                    }
                }
            }
        }
    });
    assert!(!response_validator.is_valid(&contradictory_settings));
    let mut bad_ack = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": {
            "kind": "operation",
            "value": {
                "request_sha256": "a".repeat(64),
                "result": {"kind": "acknowledged", "value": {"accepted": false}}
            }
        }
    });
    assert!(!response_validator.is_valid(&bad_ack));
    bad_ack["result"]["value"]["result"] = json!({
        "kind": "settings_validation",
        "value": {"valid": true, "issues": [{"kind": "invalid_format"}]}
    });
    assert!(!response_validator.is_valid(&bad_ack));

    let event_schema: Value = serde_json::from_str(EVENT_SCHEMA).unwrap();
    let event_validator = jsonschema::validator_for(&event_schema).unwrap();
    let mut global_with_run: Value = serde_json::from_str(EVENT).unwrap();
    global_with_run["kind"] = json!("runner_ready");
    global_with_run["run_id"] = json!("run-public-example");
    assert!(!event_validator.is_valid(&global_with_run));
    let mut run_without_attempt: Value = serde_json::from_str(EVENT).unwrap();
    run_without_attempt["kind"] = json!("run_started");
    run_without_attempt["attempt_id"] = Value::Null;
    assert!(!event_validator.is_valid(&run_without_attempt));
}

#[test]
fn checked_in_schemas_equal_fresh_generation() {
    let generated_request = serde_json::to_value(control_request_schema()).unwrap();
    let generated_response = serde_json::to_value(control_response_schema()).unwrap();
    let generated_event = serde_json::to_value(control_event_schema()).unwrap();
    let generated_history = serde_json::to_value(history_evidence_schema()).unwrap();
    let generated_analysis = serde_json::to_value(analysis_evidence_schema()).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(REQUEST_SCHEMA).unwrap(),
        generated_request
    );
    assert_eq!(
        serde_json::from_str::<Value>(RESPONSE_SCHEMA).unwrap(),
        generated_response
    );
    assert_eq!(
        serde_json::from_str::<Value>(EVENT_SCHEMA).unwrap(),
        generated_event
    );
    assert_eq!(
        serde_json::from_str::<Value>(HISTORY_SCHEMA).unwrap(),
        generated_history
    );
    assert_eq!(
        serde_json::from_str::<Value>(ANALYSIS_SCHEMA).unwrap(),
        generated_analysis
    );
}
