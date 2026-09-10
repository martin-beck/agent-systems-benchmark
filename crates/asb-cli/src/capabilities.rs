// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Closed, side-effect-free capability contract for independent frontends.

use schemars::{JsonSchema, schema_for};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Stable protocol discriminator consumed by independently installed frontends.
pub const CAPABILITY_PROTOCOL: &str = "asb-cli-capabilities";
/// Current closed capability protocol version.
pub const CAPABILITY_PROTOCOL_VERSION: u8 = 1;
/// Maximum encoded capability response accepted at this boundary.
pub const MAX_CAPABILITY_RESPONSE_BYTES: usize = 4 * 1024;

/// Exact ASB control operations exposed to an independent frontend.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FrontendCapabilities {
    /// Bounded run analysis is implemented.
    pub analysis: bool,
    /// Privacy-safe artifact metadata is implemented.
    pub artifacts: bool,
    /// Causally fenced cancellation is implemented.
    pub cancel: bool,
    /// Public runner event cursors are implemented.
    pub events: bool,
    /// Bounded durable run history is implemented.
    pub history: bool,
    /// Durable plan launch is implemented.
    pub launch: bool,
    /// Settings validation and durable plan creation are implemented.
    pub planning: bool,
    /// Content-pinned repeat is implemented.
    pub repeat: bool,
}

/// Closed ASB capability response v1.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityResponse {
    /// Stable capability protocol discriminator.
    pub protocol: String,
    /// Exact capability protocol version.
    pub protocol_version: u8,
    /// Semantic version of the installed ASB executable.
    #[schemars(length(min = 1, max = 64))]
    pub asb_version: String,
    /// Operations backed by the authoritative ASB control API.
    pub capabilities: FrontendCapabilities,
}

impl CapabilityResponse {
    /// Describe the operations implemented by the authoritative control-v1 backend.
    #[must_use]
    pub fn control_v1() -> Self {
        Self {
            protocol: CAPABILITY_PROTOCOL.to_owned(),
            protocol_version: CAPABILITY_PROTOCOL_VERSION,
            asb_version: env!("CARGO_PKG_VERSION").to_owned(),
            capabilities: FrontendCapabilities {
                analysis: true,
                artifacts: true,
                cancel: true,
                events: true,
                history: true,
                launch: true,
                planning: true,
                repeat: true,
            },
        }
    }

    /// Parse and semantically validate one bounded closed-v1 response.
    pub fn parse(input: &[u8]) -> Result<Self, &'static str> {
        if input.len() > MAX_CAPABILITY_RESPONSE_BYTES {
            return Err("capability response exceeds its byte limit");
        }
        let response: Self = serde_json::from_slice(input)
            .map_err(|_| "capability response syntax or shape is invalid")?;
        if response.protocol != CAPABILITY_PROTOCOL {
            return Err("capability protocol is unsupported");
        }
        if response.protocol_version != CAPABILITY_PROTOCOL_VERSION {
            return Err("capability protocol version is unsupported");
        }
        if !is_safe_version(&response.asb_version) {
            return Err("ASB version is invalid");
        }
        Ok(response)
    }
}

fn is_safe_version(value: &str) -> bool {
    if value.is_empty() || value.len() > 64 || !value.is_ascii() {
        return false;
    }
    let (core, suffix) = value
        .split_once('-')
        .map_or((value, None), |(core, suffix)| (core, Some(suffix)));
    let mut components = core.split('.');
    let valid_number =
        |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    valid_number(components.next().unwrap_or_default())
        && valid_number(components.next().unwrap_or_default())
        && valid_number(components.next().unwrap_or_default())
        && components.next().is_none()
        && suffix.is_none_or(|suffix| !suffix.is_empty())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
}

/// Generate the checked public schema from the Rust response model.
#[must_use]
pub fn capability_schema() -> Value {
    let mut schema = serde_json::to_value(schema_for!(CapabilityResponse))
        .expect("the static capability schema is serializable");
    let object = schema
        .as_object_mut()
        .expect("schemars produces an object root");
    object.insert(
        "$id".into(),
        json!("https://github.com/martin-beck/asb-tui/protocol/v1/capabilities.schema.json"),
    );
    object.insert(
        "title".into(),
        json!("ASB external frontend capability negotiation v1"),
    );
    let definitions = object
        .remove("$defs")
        .and_then(|value| value.as_object().cloned())
        .expect("response schema has definitions");
    let capabilities = definitions
        .get("FrontendCapabilities")
        .cloned()
        .expect("frontend capability schema exists");
    let properties = object
        .get_mut("properties")
        .and_then(Value::as_object_mut)
        .expect("response schema has properties");
    properties.insert("protocol".into(), json!({"const": CAPABILITY_PROTOCOL}));
    properties.insert(
        "protocol_version".into(),
        json!({"const": CAPABILITY_PROTOCOL_VERSION}),
    );
    properties.insert("capabilities".into(), capabilities);
    strip_descriptions(&mut schema);
    schema
}

fn strip_descriptions(value: &mut Value) {
    match value {
        Value::Object(object) => {
            object.remove("description");
            for child in object.values_mut() {
                strip_descriptions(child);
            }
        }
        Value::Array(values) => {
            for child in values {
                strip_descriptions(child);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_is_closed_and_deterministic() {
        let response = CapabilityResponse::control_v1();
        assert_eq!(response, CapabilityResponse::control_v1());
        assert_eq!(response.protocol, CAPABILITY_PROTOCOL);
        assert_eq!(response.protocol_version, CAPABILITY_PROTOCOL_VERSION);
        assert!(response.capabilities.planning);
        assert!(response.capabilities.launch);
        assert!(response.capabilities.events);
        assert!(response.capabilities.cancel);
        assert!(response.capabilities.history);
        assert!(response.capabilities.repeat);
        assert!(response.capabilities.analysis);
        assert!(response.capabilities.artifacts);
    }

    #[test]
    fn parser_rejects_drift_and_unbounded_input() {
        let valid = serde_json::to_vec(&CapabilityResponse::control_v1()).unwrap();
        assert_eq!(
            CapabilityResponse::parse(&valid).unwrap(),
            CapabilityResponse::control_v1()
        );
        for invalid in [
            String::from_utf8(valid.clone())
                .unwrap()
                .replace(CAPABILITY_PROTOCOL, "mutable-latest"),
            String::from_utf8(valid.clone())
                .unwrap()
                .replace("\"protocol_version\":1", "\"protocol_version\":2"),
            String::from_utf8(valid.clone())
                .unwrap()
                .replace("\"asb_version\":\"0.1.0\"", "\"asb_version\":\"bad\""),
            String::from_utf8(valid)
                .unwrap()
                .replace("\"analysis\":true", "\"analysis\":true,\"analysis\":false"),
        ] {
            assert!(CapabilityResponse::parse(invalid.as_bytes()).is_err());
        }
        assert!(CapabilityResponse::parse(&vec![b' '; MAX_CAPABILITY_RESPONSE_BYTES + 1]).is_err());
    }
}
