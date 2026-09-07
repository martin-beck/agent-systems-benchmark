// SPDX-License-Identifier: MIT
//! Mechanical schema and strict fixture parity checks.

use asb_bundle::{RuntimeBundleManifest, content_digest, manifest_schema};
use serde_json::{Value, json};

#[test]
fn checked_schema_equals_rust_model() {
    let checked: Value =
        serde_json::from_str(include_str!("../schema/v1/runtime-bundle.schema.json"))
            .expect("checked schema");
    let generated = serde_json::to_value(manifest_schema()).expect("generated schema");
    assert_eq!(checked, generated);
    assert_eq!(
        checked.pointer("/properties/schema_version/const"),
        Some(&json!(1))
    );
    assert_eq!(
        checked.pointer("/additionalProperties"),
        Some(&Value::Bool(false))
    );
}

#[test]
fn public_fixture_matches_strict_rust_contract() {
    let fixture = include_str!("../fixtures/v1/runtime-bundle.json");
    let manifest: RuntimeBundleManifest =
        serde_json::from_str(fixture).expect("fixture follows Rust contract");
    assert_eq!(manifest.schema_version, 1);
    assert_eq!(manifest.artifacts.len(), 2);
    assert_eq!(manifest.content_sha256, content_digest(&manifest.artifacts));

    let mut value: Value = serde_json::from_str(fixture).expect("fixture JSON");
    value["ambient_secret"] = json!("must be rejected");
    assert!(serde_json::from_value::<RuntimeBundleManifest>(value).is_err());
}
