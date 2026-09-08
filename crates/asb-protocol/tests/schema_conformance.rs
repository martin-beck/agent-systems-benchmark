// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Generated-schema and fixture conformance checks.

use asb_protocol::{
    ExperimentManifestV1, ExtensionManifest, ExtensionResult, ProviderProfileCapabilities,
    ProviderProfileError, ProviderProfileV1, ProviderSettingField, RpcNotification, RpcRequest,
    TraceSpan, WorkloadManifest,
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
        "provider-profile.schema.json",
        include_str!("../schema/v1/provider-profile.schema.json"),
    ),
    (
        "provider-capabilities.schema.json",
        include_str!("../schema/v1/provider-capabilities.schema.json"),
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
    (
        "trace-span.schema.json",
        include_str!("../schema/v1/trace-span.schema.json"),
    ),
];

#[test]
fn checked_in_schemas_equal_rust_types() {
    assert_schema::<ExtensionManifest>(SCHEMAS[0].1);
    assert_schema::<WorkloadManifest>(SCHEMAS[1].1);
    assert_schema::<ExperimentManifestV1>(SCHEMAS[2].1);
    assert_schema::<ProviderProfileV1>(SCHEMAS[3].1);
    assert_schema::<ProviderProfileCapabilities>(SCHEMAS[4].1);
    assert_schema::<RpcRequest>(SCHEMAS[5].1);
    assert_schema::<RpcNotification>(SCHEMAS[6].1);
    assert_schema::<ExtensionResult>(SCHEMAS[7].1);
    assert_schema::<TraceSpan>(SCHEMAS[8].1);
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
        include_str!("../fixtures/v1/provider-profile.json"),
    );
    validate_fixture(
        SCHEMAS[4].1,
        include_str!("../fixtures/v1/provider-capabilities.json"),
    );
    validate_fixture(
        SCHEMAS[5].1,
        include_str!("../fixtures/v1/negotiate-request.json"),
    );
    validate_fixture(
        SCHEMAS[6].1,
        include_str!("../fixtures/v1/event-notification.json"),
    );
    validate_fixture(SCHEMAS[7].1, include_str!("../fixtures/v1/result.json"));
    validate_fixture(SCHEMAS[8].1, include_str!("../fixtures/v1/trace-span.json"));

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
    let schema: Value = serde_json::from_str(SCHEMAS[5].1).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let bad_version = serde_json::json!({"jsonrpc":"1.0","id":1,"method":"describe","params":{}});
    let unknown_field =
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"describe","params":{},"secret":true});
    assert!(!validator.is_valid(&bad_version));
    assert!(!validator.is_valid(&unknown_field));
}

#[test]
fn provider_fixtures_cover_schema_and_runtime_fail_closed_boundaries() {
    let profile_schema: Value = serde_json::from_str(SCHEMAS[3].1).unwrap();
    let profile_validator = jsonschema::validator_for(&profile_schema).unwrap();
    let capabilities_schema: Value = serde_json::from_str(SCHEMAS[4].1).unwrap();
    let capabilities_validator = jsonschema::validator_for(&capabilities_schema).unwrap();

    let profile_text = include_str!("../fixtures/v1/provider-profile.json");
    let profile_json: Value = serde_json::from_str(profile_text).unwrap();
    let profile: ProviderProfileV1 = serde_json::from_str(profile_text).unwrap();
    assert!(profile_validator.is_valid(&profile_json));
    profile.validate().unwrap();

    for malformed in [
        include_str!("../fixtures/v1/provider-profile-malformed.json"),
        include_str!("../fixtures/v1/provider-profile-unknown-field.json"),
    ] {
        let value: Value = serde_json::from_str(malformed).unwrap();
        assert!(!profile_validator.is_valid(&value));
        assert!(serde_json::from_value::<ProviderProfileV1>(value).is_err());
    }

    let capabilities_text = include_str!("../fixtures/v1/provider-capabilities.json");
    let capabilities_json: Value = serde_json::from_str(capabilities_text).unwrap();
    let capabilities: ProviderProfileCapabilities =
        serde_json::from_str(capabilities_text).unwrap();
    assert!(capabilities_validator.is_valid(&capabilities_json));
    capabilities.validate().unwrap();
    profile.negotiate(&capabilities).unwrap();

    let lossy_text = include_str!("../fixtures/v1/provider-capabilities-lossy.json");
    let lossy_json: Value = serde_json::from_str(lossy_text).unwrap();
    let lossy: ProviderProfileCapabilities = serde_json::from_str(lossy_text).unwrap();
    assert!(capabilities_validator.is_valid(&lossy_json));
    assert!(matches!(
        profile.negotiate(&lossy),
        Err(ProviderProfileError::UnsupportedSetting {
            field: ProviderSettingField::Seed,
            present: true
        })
    ));

    let unsupported_text = include_str!("../fixtures/v1/provider-capabilities-unsupported.json");
    let unsupported_json: Value = serde_json::from_str(unsupported_text).unwrap();
    let unsupported: ProviderProfileCapabilities = serde_json::from_str(unsupported_text).unwrap();
    assert!(capabilities_validator.is_valid(&unsupported_json));
    assert!(matches!(
        profile.negotiate(&unsupported),
        Err(ProviderProfileError::UnsupportedProvider(_))
    ));

    let mut missing_setting = capabilities_json;
    missing_setting["settings"]
        .as_object_mut()
        .unwrap()
        .remove("seed");
    assert!(!capabilities_validator.is_valid(&missing_setting));

    let mut unknown_setting: Value = serde_json::from_str(capabilities_text).unwrap();
    unknown_setting["settings"]["future_setting"] = serde_json::json!({
        "exact_value": true,
        "explicit_omission": true
    });
    assert!(!capabilities_validator.is_valid(&unknown_setting));

    let mut bad_capability_version: Value = serde_json::from_str(capabilities_text).unwrap();
    bad_capability_version["maximum_version"]["major"] = 2.into();
    assert!(!capabilities_validator.is_valid(&bad_capability_version));

    for (pointer, bad) in [
        ("/model", Value::String(String::new())),
        ("/model", Value::String("x".repeat(1_025))),
        ("/endpoint/identity_sha256", Value::String("bad".into())),
        ("/settings/temperature_milli", Value::from(2_001)),
        ("/settings/top_p_millionth", Value::from(1_000_001)),
        ("/settings/max_output_tokens", Value::from(0)),
        ("/settings/reasoning_effort", Value::String(String::new())),
        (
            "/settings/additional_settings_sha256",
            Value::String("bad".into()),
        ),
        ("/transport/max_request_bytes", Value::from(0)),
        ("/transport/max_response_bytes", Value::from(16_777_217)),
        ("/transport/connect_timeout_ms", Value::from(0)),
        ("/transport/request_timeout_ms", Value::from(3_600_001)),
        ("/transport/max_concurrent_requests", Value::from(0)),
    ] {
        let mut invalid = profile_json.clone();
        *invalid.pointer_mut(pointer).unwrap() = bad;
        assert!(!profile_validator.is_valid(&invalid), "{pointer}");
    }
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
fn experiment_schema_rejects_feasible_text_and_setting_bounds() {
    let schema: Value = serde_json::from_str(SCHEMAS[2].1).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let fixture: Value =
        serde_json::from_str(include_str!("../fixtures/v1/experiment-manifest.json")).unwrap();

    let mut empty = fixture.clone();
    empty["agent"]["implementation"] = Value::String(String::new());
    assert!(!validator.is_valid(&empty));

    let mut oversized = fixture.clone();
    oversized["platform"]["cpu_model"] = Value::String("x".repeat(1_025));
    assert!(!validator.is_valid(&oversized));

    let mut temperature = fixture.clone();
    temperature["model"]["settings"]["temperature_milli"] = Value::from(2_001);
    assert!(!validator.is_valid(&temperature));

    let mut top_p = fixture.clone();
    top_p["model"]["settings"]["top_p_millionth"] = Value::from(1_000_001);
    assert!(!validator.is_valid(&top_p));

    let mut output = fixture.clone();
    output["model"]["settings"]["max_output_tokens"] = Value::from(0);
    assert!(!validator.is_valid(&output));

    let mut reasoning = fixture.clone();
    reasoning["model"]["settings"]["reasoning_effort"] = Value::String(String::new());
    assert!(!validator.is_valid(&reasoning));

    let mut additional_digest = fixture.clone();
    additional_digest["model"]["settings"]["additional_settings_sha256"] =
        Value::String("bad".into());
    assert!(!validator.is_valid(&additional_digest));

    let mut cassette_digest = fixture.clone();
    cassette_digest["controls"]["replay"]["cassette_sha256"] = Value::String("bad".into());
    assert!(!validator.is_valid(&cassette_digest));

    let mut live_with_cassette = fixture.clone();
    live_with_cassette["controls"]["replay"]["mode"] = Value::String("live".into());
    assert!(!validator.is_valid(&live_with_cassette));

    let mut replay_without_cassette = fixture;
    replay_without_cassette["controls"]["replay"]["cassette_sha256"] = Value::Null;
    assert!(!validator.is_valid(&replay_without_cassette));
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
