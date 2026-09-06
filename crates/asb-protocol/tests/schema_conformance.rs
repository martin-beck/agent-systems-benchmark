// SPDX-License-Identifier: MIT
//! Generated-schema and fixture conformance checks.

use asb_protocol::{
    ExperimentManifestV1, ExtensionManifest, ExtensionResult, RpcNotification, RpcRequest,
    WorkloadManifest,
};
use schemars::{JsonSchema, schema_for};
use serde_json::Value;

const SCHEMAS: &[(&str, &str)] = &[
    (
        "extension-manifest.schema.json",
        include_str!("../schema/v1/extension-manifest.schema.json"),
    ),
    (
        "workload-manifest.schema.json",
        include_str!("../schema/v1/workload-manifest.schema.json"),
    ),
    (
        "experiment-manifest.schema.json",
        include_str!("../schema/v1/experiment-manifest.schema.json"),
    ),
    (
        "request.schema.json",
        include_str!("../schema/v1/request.schema.json"),
    ),
    (
        "notification.schema.json",
        include_str!("../schema/v1/notification.schema.json"),
    ),
    (
        "result.schema.json",
        include_str!("../schema/v1/result.schema.json"),
    ),
];

#[test]
fn checked_in_schemas_equal_rust_types() {
    assert_schema::<ExtensionManifest>(SCHEMAS[0].1);
    assert_schema::<WorkloadManifest>(SCHEMAS[1].1);
    assert_schema::<ExperimentManifestV1>(SCHEMAS[2].1);
    assert_schema::<RpcRequest>(SCHEMAS[3].1);
    assert_schema::<RpcNotification>(SCHEMAS[4].1);
    assert_schema::<ExtensionResult>(SCHEMAS[5].1);
}

#[test]
fn positive_fixtures_validate() {
    validate_fixture(
        SCHEMAS[0].1,
        include_str!("../fixtures/v1/extension-manifest.json"),
    );
    validate_fixture(
        SCHEMAS[1].1,
        include_str!("../fixtures/v1/workload-manifest.json"),
    );
    validate_fixture(
        SCHEMAS[2].1,
        include_str!("../fixtures/v1/experiment-manifest.json"),
    );
    validate_fixture(
        SCHEMAS[2].1,
        include_str!("../fixtures/v1/experiment-manifest-confounded.json"),
    );
    validate_fixture(
        SCHEMAS[3].1,
        include_str!("../fixtures/v1/negotiate-request.json"),
    );
    validate_fixture(
        SCHEMAS[4].1,
        include_str!("../fixtures/v1/event-notification.json"),
    );
    validate_fixture(SCHEMAS[5].1, include_str!("../fixtures/v1/result.json"));

    let manifest: ExperimentManifestV1 =
        serde_json::from_str(include_str!("../fixtures/v1/experiment-manifest.json")).unwrap();
    manifest.validate().unwrap();
    let confounded: ExperimentManifestV1 = serde_json::from_str(include_str!(
        "../fixtures/v1/experiment-manifest-confounded.json"
    ))
    .unwrap();
    confounded.validate().unwrap();
}

#[test]
fn malformed_and_unknown_fields_fail_schema_validation() {
    let schema: Value = serde_json::from_str(SCHEMAS[3].1).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let bad_version = serde_json::json!({"jsonrpc":"1.0","id":1,"method":"describe","params":{}});
    let unknown_field =
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"describe","params":{},"secret":true});
    assert!(!validator.is_valid(&bad_version));
    assert!(!validator.is_valid(&unknown_field));
}

#[test]
fn malformed_experiment_hash_and_unknown_fields_fail_schema_validation() {
    let schema: Value = serde_json::from_str(SCHEMAS[2].1).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let mut fixture: Value =
        serde_json::from_str(include_str!("../fixtures/v1/experiment-manifest.json")).unwrap();
    fixture["experiment_sha256"] = Value::String("abcd".into());
    assert!(!validator.is_valid(&fixture));
    fixture["experiment_sha256"] = Value::String("a".repeat(64));
    fixture["private_prompt"] = Value::String("must never be accepted".into());
    assert!(!validator.is_valid(&fixture));
}

#[test]
fn well_shaped_but_stale_experiment_address_fails_runtime_validation() {
    let mut fixture: ExperimentManifestV1 =
        serde_json::from_str(include_str!("../fixtures/v1/experiment-manifest.json")).unwrap();
    fixture.experiment_sha256 = "a".repeat(64);
    assert!(fixture.validate().is_err());
}

fn assert_schema<T: JsonSchema>(checked_in: &str) {
    let generated = serde_json::to_value(schema_for!(T)).unwrap();
    let checked_in: Value = serde_json::from_str(checked_in).unwrap();
    assert_eq!(checked_in, generated);
}

fn validate_fixture(schema: &str, fixture: &str) {
    let schema: Value = serde_json::from_str(schema).unwrap();
    let fixture: Value = serde_json::from_str(fixture).unwrap();
    jsonschema::validator_for(&schema)
        .unwrap()
        .validate(&fixture)
        .unwrap();
}
