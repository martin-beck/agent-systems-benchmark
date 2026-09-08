// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Parse public CSB fixtures through the strict shared protocol contracts.

use asb_csb_runner::{CSB_CONTRACT_V1, RunRequest};
use asb_protocol::{
    AttemptParams, ExtensionResult, Method, NegotiateParams, NegotiateResult, RequestId,
    RpcRequest, RpcResponse,
};

const NEGOTIATE_REQUEST: &[u8] =
    include_bytes!("../../../tests/fixtures/csb-runner/negotiate.request.json");
const NEGOTIATE_RESPONSE: &[u8] =
    include_bytes!("../../../tests/fixtures/csb-runner/negotiate.response.json");
const RUN_REQUEST: &[u8] = include_bytes!("../../../tests/fixtures/csb-runner/run.request.json");
const RUN_RESPONSE: &[u8] = include_bytes!("../../../tests/fixtures/csb-runner/run.response.json");

#[test]
fn negotiation_fixture_is_exact_and_bounded() {
    let envelope: RpcRequest = serde_json::from_slice(NEGOTIATE_REQUEST).unwrap();
    assert_eq!(envelope.id, RequestId::String("negotiation-1".into()));
    assert_eq!(envelope.method, Method::Negotiate);
    let params: NegotiateParams = serde_json::from_value(envelope.params).unwrap();
    assert_eq!(params.protocol, CSB_CONTRACT_V1);
    params.limits.validate().unwrap();

    let envelope: RpcResponse = serde_json::from_slice(NEGOTIATE_RESPONSE).unwrap();
    assert_eq!(envelope.id, RequestId::String("negotiation-1".into()));
    let result: NegotiateResult = serde_json::from_value(envelope.result).unwrap();
    assert_eq!(result.protocol, CSB_CONTRACT_V1);
    assert_eq!(result.limits, params.limits);
    assert_eq!(result.capabilities, params.requested_capabilities);
}

#[test]
fn run_fixture_binds_envelope_and_payload_identities() {
    let envelope: RpcRequest = serde_json::from_slice(RUN_REQUEST).unwrap();
    assert_eq!(envelope.id, RequestId::String("run-1".into()));
    assert_eq!(envelope.method, Method::Start);
    let params: AttemptParams = serde_json::from_value(envelope.params).unwrap();
    let request: RunRequest = serde_json::from_value(params.payload).unwrap();
    assert_eq!(params.session_id.0, request.operation_id);
    assert_eq!(params.attempt_id.0, request.attempt_id);
    assert_eq!(request.contract, CSB_CONTRACT_V1);

    let envelope: RpcResponse = serde_json::from_slice(RUN_RESPONSE).unwrap();
    assert_eq!(envelope.id, RequestId::String("run-1".into()));
    let result: ExtensionResult = serde_json::from_value(envelope.result).unwrap();
    result.validate().unwrap();
    assert_eq!(result.session_id, params.session_id);
    assert_eq!(result.attempt_id, params.attempt_id);
}

#[test]
fn fixtures_reject_unknown_fields_and_mismatched_methods() {
    let mut negotiate: serde_json::Value = serde_json::from_slice(NEGOTIATE_REQUEST).unwrap();
    negotiate["params"]["ambient"] = serde_json::json!(true);
    let envelope: RpcRequest = serde_json::from_value(negotiate).unwrap();
    assert!(serde_json::from_value::<NegotiateParams>(envelope.params).is_err());

    let mut run: serde_json::Value = serde_json::from_slice(RUN_REQUEST).unwrap();
    run["method"] = serde_json::json!("collect");
    let envelope: RpcRequest = serde_json::from_value(run).unwrap();
    assert_ne!(envelope.method, Method::Start);
}
