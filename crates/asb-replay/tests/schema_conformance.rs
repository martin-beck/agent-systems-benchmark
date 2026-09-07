// SPDX-License-Identifier: MIT
//! Checked-in schema and public fixture conformance tests.

use asb_replay::{CassetteLimits, decode_cassette};
use serde_json::{Value, json};

const SCHEMA: &str = include_str!("../schema/v1/cassette.schema.json");
const FIXTURES: &[&str] = &[
    include_str!("../fixtures/v1/buffered.json"),
    include_str!("../fixtures/v1/events.json"),
];

#[test]
fn public_fixtures_pass_schema_and_rust_validation() {
    let schema: Value = serde_json::from_str(SCHEMA).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    for encoded in FIXTURES {
        let fixture: Value = serde_json::from_str(encoded).unwrap();
        validator.validate(&fixture).unwrap();
        decode_cassette(encoded.as_bytes(), CassetteLimits::default()).unwrap();
    }
}

#[test]
fn schema_rejects_unknown_fields_versions_and_unbounded_shapes() {
    let schema: Value = serde_json::from_str(SCHEMA).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let fixture: Value = serde_json::from_str(FIXTURES[0]).unwrap();

    let mut unknown = fixture.clone();
    unknown["private_capture"] = json!(true);
    assert!(!validator.is_valid(&unknown));

    let mut version = fixture.clone();
    version["contents"]["schema_version"] = json!(2);
    assert!(!validator.is_valid(&version));

    let mut headers = fixture.clone();
    headers["contents"]["interactions"][0]["request"]["headers"] = Value::Array(
        (0..1025)
            .map(|_| json!({"name": "x", "value": "y"}))
            .collect(),
    );
    assert!(!validator.is_valid(&headers));

    for path in ["//host/path", "/v1/synthetic#fragment", "/v1\\synthetic"] {
        let mut unsafe_path = fixture.clone();
        unsafe_path["contents"]["interactions"][0]["request"]["path"] = path.into();
        assert!(!validator.is_valid(&unsafe_path));
    }
    for value in ["line\r\ninjected", "nul\0byte", "tab\tvalue"] {
        let mut unsafe_header = fixture.clone();
        unsafe_header["contents"]["interactions"][0]["request"]["headers"][0]["value"] =
            value.into();
        assert!(!validator.is_valid(&unsafe_header));
    }

    let mut underscore = fixture.clone();
    underscore["contents"]["interactions"][0]["request"]["headers"][0]["name"] = "span_id".into();
    assert!(validator.is_valid(&underscore));
    for name in ["span:id", "span id", "span.id", "Span_id", "span/id"] {
        let mut unsafe_header = fixture.clone();
        unsafe_header["contents"]["interactions"][0]["request"]["headers"][0]["name"] = name.into();
        assert!(!validator.is_valid(&unsafe_header));
    }
}
