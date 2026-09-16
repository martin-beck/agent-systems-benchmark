// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Conformance for the versioned setup preflight output.

use serde_json::Value;
use std::ffi::OsString;

const SCHEMA: &str = include_str!("../schema/v1/setup-output.schema.json");

fn output(args: &[&str]) -> Value {
    let args: Vec<OsString> = args.iter().map(OsString::from).collect();
    let mut bytes = Vec::new();
    assert_eq!(asb_cli::run(&args, &mut bytes, &mut Vec::new()), 0);
    serde_json::from_slice(&bytes).expect("setup output is JSON")
}

#[test]
fn setup_output_matches_checked_in_schema() {
    let schema: Value = serde_json::from_str(SCHEMA).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let value = output(&[
        "setup",
        "--provider-profile",
        "openai",
        "--model",
        "gpt-4o-mini",
    ]);
    assert!(validator.is_valid(&value));
}

#[test]
fn setup_schema_rejects_unknown_fields() {
    let schema: Value = serde_json::from_str(SCHEMA).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let mut value = output(&["setup"]);
    value
        .as_object_mut()
        .unwrap()
        .insert("ambient_secret".into(), Value::String("x".into()));
    assert!(!validator.is_valid(&value));
}
