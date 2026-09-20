// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Canonical generated JSON Schemas with the absolute wire bounds enforced by runtime validation.

use schemars::{JsonSchema, Schema, schema_for};
use serde_json::{Value, json};
use std::collections::BTreeSet;

use crate::{
    AnalysisEvidence, CertificateIdentityV1, ControlEvent, ControlRequest, ControlResponse,
    HistoryEvidence,
};

/// Canonical schema for the versioned certificate identity metadata.
pub fn certificate_identity_schema() -> Schema {
    let mut value = serde_json::to_value(canonical::<CertificateIdentityV1>())
        .expect("certificate schema serializes");
    let properties = value
        .get_mut("properties")
        .and_then(Value::as_object_mut)
        .expect("certificate schema properties");
    for name in [
        "subject_sha256",
        "issuer_sha256",
        "certificate_sha256",
        "trust_anchor_sha256",
        "endpoint_identity_sha256",
    ] {
        let property = properties
            .get_mut(name)
            .and_then(Value::as_object_mut)
            .expect("certificate digest property");
        property.insert("minLength".into(), json!(64));
        property.insert("maxLength".into(), json!(64));
        property.insert("pattern".into(), json!("^[0-9a-f]{64}$"));
    }
    properties
        .get_mut("schema_version")
        .and_then(Value::as_object_mut)
        .expect("certificate schema version")
        .insert("const".into(), json!(1));
    properties
        .get_mut("generation")
        .and_then(Value::as_object_mut)
        .expect("certificate generation")
        .insert("minimum".into(), json!(1));
    for name in ["not_before", "not_after"] {
        properties
            .get_mut(name)
            .and_then(Value::as_object_mut)
            .expect("certificate validity")
            .insert("minimum".into(), json!(1));
    }
    let role = properties
        .get_mut("role")
        .and_then(Value::as_object_mut)
        .expect("certificate role");
    role.insert("minLength".into(), json!(1));
    role.insert("maxLength".into(), json!(32));
    role.insert(
        "enum".into(),
        json!(["observer", "operator", "administrator"]),
    );
    serde_json::from_value(value).expect("certificate schema remains valid")
}

fn canonical<T: JsonSchema>() -> Schema {
    let mut value = serde_json::to_value(schema_for!(T)).expect("schema serializes");
    tighten(&mut value);
    serde_json::from_value(value).expect("canonical schema remains valid")
}

fn tighten(value: &mut Value) {
    let Value::Object(object) = value else {
        return;
    };
    if let Some(any_of) = object.remove("anyOf") {
        object.insert("oneOf".into(), any_of);
    }
    if let Some(properties) = object.get_mut("properties").and_then(Value::as_object_mut) {
        if let Some(jsonrpc) = properties.get_mut("jsonrpc").and_then(Value::as_object_mut) {
            jsonrpc.insert("const".into(), Value::String("2.0".into()));
        }
        for name in [
            "idempotency_key",
            "plan_id",
            "run_id",
            "attempt_id",
            "request_sha256",
            "plan_sha256",
            "analysis_sha256",
            "sha256",
            "digest",
        ] {
            if let Some(schema) = properties.get_mut(name).and_then(Value::as_object_mut) {
                schema.insert("minLength".into(), json!(1));
                schema.insert("maxLength".into(), json!(128));
                if name.ends_with("sha256") || name == "sha256" || name == "digest" {
                    schema.insert("minLength".into(), json!(64));
                    schema.insert("maxLength".into(), json!(64));
                    schema.insert("pattern".into(), json!("^[0-9a-f]{64}$"));
                } else {
                    schema.insert("pattern".into(), json!("^[A-Za-z0-9._:-]+$"));
                }
            }
        }
        if let Some(schema) = properties
            .get_mut("timeout_ms")
            .and_then(Value::as_object_mut)
        {
            schema.insert("minimum".into(), json!(1));
            schema.insert("maximum".into(), json!(300000));
        }
        if let Some(schema) = properties
            .get_mut("max_frame_bytes")
            .and_then(Value::as_object_mut)
        {
            schema.insert("minimum".into(), json!(1));
            schema.insert("maximum".into(), json!(1048576));
        }
        if let Some(schema) = properties
            .get_mut("max_timeout_ms")
            .and_then(Value::as_object_mut)
        {
            schema.insert("minimum".into(), json!(1));
            schema.insert("maximum".into(), json!(300000));
        }
        if let Some(schema) = properties
            .get_mut("max_page_items")
            .and_then(Value::as_object_mut)
        {
            schema.insert("minimum".into(), json!(1));
            schema.insert("maximum".into(), json!(256));
        }
        if let Some(schema) = properties
            .get_mut("max_in_flight")
            .and_then(Value::as_object_mut)
        {
            schema.insert("minimum".into(), json!(1));
            schema.insert("maximum".into(), json!(64));
        }
        if let Some(schema) = properties.get_mut("limit").and_then(Value::as_object_mut) {
            schema.insert("minimum".into(), json!(1));
            schema.insert("maximum".into(), json!(256));
        }
        if let Some(schema) = properties
            .get_mut("run_count")
            .and_then(Value::as_object_mut)
        {
            schema.insert("minimum".into(), json!(1));
            schema.insert("maximum".into(), json!(256));
        }
        if let Some(schema) = properties
            .get_mut("accepted")
            .and_then(Value::as_object_mut)
        {
            schema.insert("const".into(), Value::Bool(true));
        }
        if let Some(schema) = properties.get_mut("issues").and_then(Value::as_object_mut) {
            schema.insert("maxItems".into(), json!(256));
            schema.insert("uniqueItems".into(), Value::Bool(true));
        }
        if let Some(schema) = properties.get_mut("items").and_then(Value::as_object_mut) {
            schema.insert("maxItems".into(), json!(256));
        }
        if let Some(schema) = properties.get_mut("run_ids").and_then(Value::as_object_mut) {
            schema.insert("minItems".into(), json!(1));
            schema.insert("maxItems".into(), json!(256));
            schema.insert("uniqueItems".into(), Value::Bool(true));
        }
    }
    let keys = object.keys().cloned().collect::<Vec<_>>();
    for key in keys {
        if let Some(child) = object.get_mut(&key) {
            tighten(child);
        }
    }
}

fn settings_validation_invariant(schema: &mut Schema) {
    let mut value = serde_json::to_value(&*schema).expect("response schema serializes");
    let settings = value
        .pointer_mut("/$defs/SettingsValidation")
        .and_then(Value::as_object_mut)
        .expect("settings validation definition");
    settings.insert(
        "allOf".into(),
        json!([{
            "oneOf": [
                {
                    "properties": {
                        "valid": {"const": true},
                        "issues": {"maxItems": 0}
                    },
                    "not": {"required": ["measurement_issue"]}
                },
                {
                    "properties": {
                        "valid": {"const": false},
                        "issues": {"minItems": 1}
                    }
                }
            ]
        }]),
    );
    *schema = serde_json::from_value(value).expect("response schema remains valid");
}

fn restrict_settings_issues_to_legacy(schema: &mut Schema) {
    let mut value = serde_json::to_value(&*schema).expect("settings schema serializes");
    if let Some(properties) = value
        .pointer_mut("/$defs/SettingsValidation/properties")
        .and_then(Value::as_object_mut)
    {
        properties.remove("measurement_issue");
    }
    if let Some(valid_branch) = value
        .pointer_mut("/$defs/SettingsValidation/allOf/0/oneOf/0")
        .and_then(Value::as_object_mut)
    {
        valid_branch.remove("not");
    }
    let variants = value
        .pointer_mut("/$defs/SettingsIssue/oneOf")
        .and_then(Value::as_array_mut)
        .expect("settings issue variants");
    variants.retain(|variant| {
        matches!(
            variant.get("const").and_then(Value::as_str),
            Some(
                "invalid_format"
                    | "unsupported_capability"
                    | "unverified_component"
                    | "invalid_resource_bound"
            )
        )
    });
    *schema = serde_json::from_value(value).expect("legacy settings schema remains valid");
}

fn event_association(schema: &mut Schema) {
    let mut value = serde_json::to_value(&*schema).expect("event schema serializes");
    let object = value.as_object_mut().expect("event schema object");
    object.insert(
        "allOf".into(),
        json!([{
            "oneOf": [
                {
                    "properties": {
                        "kind": {"enum": ["runner_ready", "plan_created"]},
                        "run_id": {"type": "null"},
                        "attempt_id": {"type": "null"}
                    }
                },
                {
                    "properties": {
                        "kind": {"enum": [
                            "run_started", "run_updated", "run_completed",
                            "run_failed", "run_cancelled", "reconciliation_required"
                        ]},
                        "run_id": {"$ref": "#/$defs/RunId"},
                        "attempt_id": {"$ref": "#/$defs/AttemptId"}
                    },
                    "required": ["run_id", "attempt_id"]
                }
            ]
        }]),
    );
    *schema = serde_json::from_value(value).expect("event schema remains valid");
}

fn remove_tagged_variant(value: &mut Value, pointer: &str, tag: &str) {
    let Some(variants) = value.pointer_mut(pointer).and_then(Value::as_array_mut) else {
        return;
    };
    variants.retain(|variant| {
        variant
            .pointer("/properties/method/const")
            .and_then(Value::as_str)
            != Some(tag)
            && variant
                .pointer("/properties/kind/const")
                .and_then(Value::as_str)
                != Some(tag)
    });
}

fn remove_lifecycle_variants(value: &mut Value) {
    for tag in [
        "agent_install",
        "agent_status",
        "agent_cancel",
        "agent_retry",
        "agent_remove",
        "agent_lifecycle",
        "auth_enroll",
        "auth_status",
        "auth_rotate",
        "auth_revoke",
    ] {
        remove_tagged_variant(value, "/oneOf", tag);
        remove_tagged_variant(value, "/$defs/ControlResult/oneOf", tag);
    }
}

fn remove_auth_variants(value: &mut Value) {
    for tag in ["auth_enroll", "auth_status", "auth_rotate", "auth_revoke"] {
        remove_tagged_variant(value, "/oneOf", tag);
        remove_tagged_variant(value, "/$defs/ControlResult/oneOf", tag);
    }
}

fn remove_provider_catalog_variants(value: &mut Value) {
    remove_tagged_variant(value, "/oneOf", "provider_catalog");
    remove_tagged_variant(value, "/$defs/ControlResult/oneOf", "provider_catalog");
}

fn remove_setup_variants(value: &mut Value) {
    for tag in [
        "configuration_status",
        "configuration_apply",
        "provider_profile_upsert",
        "recording_campaign_estimate",
        "recording_campaign_plan",
        "recording_campaign_status",
        "configuration",
        "provider_profile",
        "recording_campaign",
    ] {
        remove_tagged_variant(value, "/oneOf", tag);
        remove_tagged_variant(value, "/$defs/ControlResult/oneOf", tag);
    }
}

fn remove_provider_registration_variants(value: &mut Value) {
    remove_tagged_variant(value, "/oneOf", "provider_profile_upsert");
    remove_tagged_variant(value, "/$defs/ControlResult/oneOf", "provider_profile");
}

fn remove_recording_lifecycle_variants(value: &mut Value) {
    for tag in [
        "recording_campaign_execute",
        "recording_campaign_progress",
        "recording_campaign_cancel",
        "recording_campaign_reconcile",
        "recording_campaign_offline_default",
        "recording_campaign_lifecycle",
    ] {
        remove_tagged_variant(value, "/oneOf", tag);
        remove_tagged_variant(value, "/$defs/ControlResult/oneOf", tag);
    }
}

fn collect_definition_refs(value: &Value, refs: &mut BTreeSet<String>) {
    match value {
        Value::Object(object) => {
            if let Some(name) = object
                .get("$ref")
                .and_then(Value::as_str)
                .and_then(|reference| reference.strip_prefix("#/$defs/"))
            {
                refs.insert(name.to_owned());
            }
            for (key, child) in object {
                if key != "$defs" {
                    collect_definition_refs(child, refs);
                }
            }
        }
        Value::Array(items) => items
            .iter()
            .for_each(|item| collect_definition_refs(item, refs)),
        _ => {}
    }
}

fn prune_unused_definitions(value: &mut Value) {
    let mut reachable = BTreeSet::new();
    collect_definition_refs(value, &mut reachable);
    loop {
        let before = reachable.len();
        let definitions = value
            .get("$defs")
            .and_then(Value::as_object)
            .expect("schema definitions");
        for name in reachable.clone() {
            if let Some(definition) = definitions.get(&name) {
                collect_definition_refs(definition, &mut reachable);
            }
        }
        if reachable.len() == before {
            break;
        }
    }
    value
        .get_mut("$defs")
        .and_then(Value::as_object_mut)
        .expect("schema definitions")
        .retain(|name, _| reachable.contains(name));
}

/// Canonical request schema.
pub fn control_request_schema() -> Schema {
    let mut value = serde_json::to_value(canonical::<ControlRequest>()).expect("schema serializes");
    remove_tagged_variant(&mut value, "/oneOf", "measurement_catalog");
    remove_tagged_variant(&mut value, "/oneOf", "agent_catalog");
    remove_lifecycle_variants(&mut value);
    remove_auth_variants(&mut value);
    remove_provider_catalog_variants(&mut value);
    remove_setup_variants(&mut value);
    remove_recording_lifecycle_variants(&mut value);
    prune_unused_definitions(&mut value);
    serde_json::from_value(value).expect("v1 request schema remains valid")
}

/// Canonical request schema for control v1.2.
pub fn control_request_schema_v1_2() -> Schema {
    let mut value = serde_json::to_value(canonical::<ControlRequest>()).expect("schema serializes");
    remove_tagged_variant(&mut value, "/oneOf", "agent_catalog");
    remove_lifecycle_variants(&mut value);
    remove_provider_catalog_variants(&mut value);
    remove_setup_variants(&mut value);
    remove_recording_lifecycle_variants(&mut value);
    prune_unused_definitions(&mut value);
    serde_json::from_value(value).expect("v1.2 request schema remains valid")
}

/// Canonical response schema.
pub fn control_response_schema() -> Schema {
    let mut schema = canonical::<ControlResponse>();
    settings_validation_invariant(&mut schema);
    restrict_settings_issues_to_legacy(&mut schema);
    let mut value = serde_json::to_value(schema).expect("schema serializes");
    remove_tagged_variant(
        &mut value,
        "/$defs/ControlResult/oneOf",
        "measurement_catalog",
    );
    remove_tagged_variant(&mut value, "/$defs/ControlResult/oneOf", "agent_catalog");
    remove_lifecycle_variants(&mut value);
    remove_auth_variants(&mut value);
    remove_provider_catalog_variants(&mut value);
    remove_setup_variants(&mut value);
    remove_recording_lifecycle_variants(&mut value);
    prune_unused_definitions(&mut value);
    serde_json::from_value(value).expect("v1 response schema remains valid")
}

/// Canonical response schema for control v1.2.
pub fn control_response_schema_v1_2() -> Schema {
    let mut schema = canonical::<ControlResponse>();
    settings_validation_invariant(&mut schema);
    restrict_settings_issues_to_legacy(&mut schema);
    let mut value = serde_json::to_value(schema).expect("v1.2 response schema serializes");
    remove_tagged_variant(&mut value, "/$defs/ControlResult/oneOf", "agent_catalog");
    remove_lifecycle_variants(&mut value);
    remove_provider_catalog_variants(&mut value);
    remove_setup_variants(&mut value);
    remove_recording_lifecycle_variants(&mut value);
    prune_unused_definitions(&mut value);
    serde_json::from_value(value).expect("v1.2 response schema remains valid")
}

/// Canonical request schema for control v1.3.
pub fn control_request_schema_v1_3() -> Schema {
    let mut value = serde_json::to_value(canonical::<ControlRequest>()).expect("schema serializes");
    remove_tagged_variant(&mut value, "/oneOf", "agent_catalog");
    remove_lifecycle_variants(&mut value);
    remove_provider_catalog_variants(&mut value);
    remove_setup_variants(&mut value);
    remove_recording_lifecycle_variants(&mut value);
    prune_unused_definitions(&mut value);
    serde_json::from_value(value).expect("v1.3 request schema remains valid")
}

/// Canonical response schema for control v1.3.
pub fn control_response_schema_v1_3() -> Schema {
    let mut schema = canonical::<ControlResponse>();
    settings_validation_invariant(&mut schema);
    let mut value = serde_json::to_value(schema).expect("v1.3 response schema serializes");
    remove_tagged_variant(&mut value, "/$defs/ControlResult/oneOf", "agent_catalog");
    remove_lifecycle_variants(&mut value);
    remove_provider_catalog_variants(&mut value);
    remove_setup_variants(&mut value);
    remove_recording_lifecycle_variants(&mut value);
    prune_unused_definitions(&mut value);
    serde_json::from_value(value).expect("v1.3 response schema remains valid")
}

/// Canonical request schema for control v1.4.
pub fn control_request_schema_v1_4() -> Schema {
    let mut value = serde_json::to_value(canonical::<ControlRequest>()).expect("schema serializes");
    remove_lifecycle_variants(&mut value);
    remove_provider_catalog_variants(&mut value);
    remove_setup_variants(&mut value);
    remove_recording_lifecycle_variants(&mut value);
    prune_unused_definitions(&mut value);
    serde_json::from_value(value).expect("v1.4 request schema remains valid")
}

/// Canonical response schema for control v1.4.
pub fn control_response_schema_v1_4() -> Schema {
    let mut schema = canonical::<ControlResponse>();
    settings_validation_invariant(&mut schema);
    let mut value = serde_json::to_value(schema).expect("v1.4 response schema serializes");
    remove_lifecycle_variants(&mut value);
    remove_provider_catalog_variants(&mut value);
    remove_setup_variants(&mut value);
    remove_recording_lifecycle_variants(&mut value);
    prune_unused_definitions(&mut value);
    serde_json::from_value(value).expect("v1.4 response schema remains valid")
}

/// Canonical request schema for control v1.5.
pub fn control_request_schema_v1_5() -> Schema {
    let mut value = serde_json::to_value(canonical::<ControlRequest>()).expect("schema serializes");
    remove_auth_variants(&mut value);
    remove_provider_catalog_variants(&mut value);
    remove_setup_variants(&mut value);
    remove_recording_lifecycle_variants(&mut value);
    prune_unused_definitions(&mut value);
    serde_json::from_value(value).expect("v1.5 request schema remains valid")
}

/// Canonical response schema for control v1.5.
pub fn control_response_schema_v1_5() -> Schema {
    let mut schema = canonical::<ControlResponse>();
    settings_validation_invariant(&mut schema);
    let mut value = serde_json::to_value(schema).expect("v1.5 response schema serializes");
    remove_auth_variants(&mut value);
    remove_provider_catalog_variants(&mut value);
    remove_setup_variants(&mut value);
    remove_recording_lifecycle_variants(&mut value);
    prune_unused_definitions(&mut value);
    serde_json::from_value(value).expect("v1.5 response schema remains valid")
}

/// Canonical request schema for control v1.6 authentication operations.
#[allow(dead_code)]
pub fn control_request_schema_v1_6() -> Schema {
    let mut value = serde_json::to_value(canonical::<ControlRequest>()).expect("schema serializes");
    remove_provider_catalog_variants(&mut value);
    remove_setup_variants(&mut value);
    remove_recording_lifecycle_variants(&mut value);
    prune_unused_definitions(&mut value);
    serde_json::from_value(value).expect("v1.6 request schema remains valid")
}

/// Canonical response schema for control v1.6 authentication operations.
#[allow(dead_code)]
pub fn control_response_schema_v1_6() -> Schema {
    let mut schema = canonical::<ControlResponse>();
    settings_validation_invariant(&mut schema);
    let mut value = serde_json::to_value(schema).expect("v1.6 response schema serializes");
    remove_provider_catalog_variants(&mut value);
    remove_setup_variants(&mut value);
    remove_recording_lifecycle_variants(&mut value);
    prune_unused_definitions(&mut value);
    serde_json::from_value(value).expect("v1.6 response schema remains valid")
}

/// Canonical request schema for control v1.7 provider/model catalogs.
pub fn control_request_schema_v1_7() -> Schema {
    let mut value = serde_json::to_value(canonical::<ControlRequest>()).expect("schema serializes");
    remove_recording_lifecycle_variants(&mut value);
    remove_provider_registration_variants(&mut value);
    prune_unused_definitions(&mut value);
    serde_json::from_value(value).expect("v1.7 request schema remains valid")
}

/// Canonical response schema for control v1.7 provider/model catalogs.
pub fn control_response_schema_v1_7() -> Schema {
    let mut schema = canonical::<ControlResponse>();
    settings_validation_invariant(&mut schema);
    let mut value = serde_json::to_value(schema).expect("v1.7 response schema serializes");
    remove_recording_lifecycle_variants(&mut value);
    remove_provider_registration_variants(&mut value);
    prune_unused_definitions(&mut value);
    serde_json::from_value(value).expect("v1.7 response schema remains valid")
}

/// Canonical request schema for control v1.8 recording lifecycle operations.
pub fn control_request_schema_v1_8() -> Schema {
    canonical::<ControlRequest>()
}

/// Canonical response schema for control v1.8 recording lifecycle operations.
pub fn control_response_schema_v1_8() -> Schema {
    let mut schema = canonical::<ControlResponse>();
    settings_validation_invariant(&mut schema);
    schema
}

/// Canonical request schema for control v1.9 provider-profile registration.
pub fn control_request_schema_v1_9() -> Schema {
    canonical::<ControlRequest>()
}

/// Canonical response schema for control v1.9 provider-profile registration.
pub fn control_response_schema_v1_9() -> Schema {
    let mut schema = canonical::<ControlResponse>();
    settings_validation_invariant(&mut schema);
    schema
}

/// Canonical event schema.
pub fn control_event_schema() -> Schema {
    let mut schema = canonical::<ControlEvent>();
    event_association(&mut schema);
    schema
}

/// Canonical schema for the additive history evidence extension.
pub fn history_evidence_schema() -> Schema {
    let mut schema = canonical::<HistoryEvidence>();
    restrict_settings_issues_to_legacy(&mut schema);
    schema
}

/// Canonical schema for the additive analysis evidence extension.
pub fn analysis_evidence_schema() -> Schema {
    canonical::<AnalysisEvidence>()
}
