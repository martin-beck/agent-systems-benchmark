// SPDX-License-Identifier: MIT
//! External-process conformance tests for the sample protocol-v1 plugin.

use std::collections::BTreeSet;
use std::io::BufReader;
use std::process::{Command, Stdio};

use asb_protocol::{
    CallLimits, Capability, ExtensionManifest, JsonRpcVersion, Method, NegotiateParams,
    NegotiateResult, PROTOCOL_V1, RequestId, RpcErrorResponse, RpcRequest, RpcResponse, error_code,
    read_frame, write_frame,
};

#[test]
fn independent_plugin_negotiates_and_describes_over_stdio() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_asb-example-plugin"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn sample plugin");
    let mut input = child.stdin.take().expect("plugin stdin");
    let mut output = BufReader::new(child.stdout.take().expect("plugin stdout"));
    let limits = CallLimits {
        max_frame_bytes: 4096,
        timeout_ms: 1_000,
        max_in_flight: 2,
    };
    let negotiate = RpcRequest {
        jsonrpc: JsonRpcVersion::V2,
        id: RequestId::Number(1),
        method: Method::Negotiate,
        params: serde_json::to_value(NegotiateParams {
            protocol: PROTOCOL_V1,
            limits,
            requested_capabilities: BTreeSet::from([
                Capability::Cancellation,
                Capability::StreamingEvents,
            ]),
        })
        .unwrap(),
    };
    write_frame(&mut input, &negotiate, 4096).unwrap();
    let response: RpcResponse = read_frame(&mut output, 4096).unwrap();
    let selected: NegotiateResult = serde_json::from_value(response.result).unwrap();
    assert_eq!(selected.protocol, PROTOCOL_V1);
    assert_eq!(
        selected.capabilities,
        BTreeSet::from([Capability::Cancellation])
    );

    write_frame(
        &mut input,
        &RpcRequest {
            jsonrpc: JsonRpcVersion::V2,
            id: RequestId::String("describe-1".into()),
            method: Method::Describe,
            params: serde_json::json!({}),
        },
        4096,
    )
    .unwrap();
    drop(input);
    let response: RpcResponse = read_frame(&mut output, 4096).unwrap();
    let manifest: ExtensionManifest = serde_json::from_value(response.result).unwrap();
    assert_eq!(manifest.protocol, PROTOCOL_V1);
    assert_eq!(child.wait().unwrap().code(), Some(0));
}

#[test]
fn independent_plugin_rejects_unknown_major() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_asb-example-plugin"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn sample plugin");
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let request = RpcRequest {
        jsonrpc: JsonRpcVersion::V2,
        id: RequestId::Number(7),
        method: Method::Negotiate,
        params: serde_json::to_value(NegotiateParams {
            protocol: asb_protocol::ProtocolVersion {
                major: 99,
                minor: 0,
            },
            limits: CallLimits {
                max_frame_bytes: 4096,
                timeout_ms: 1_000,
                max_in_flight: 2,
            },
            requested_capabilities: BTreeSet::new(),
        })
        .unwrap(),
    };
    write_frame(&mut input, &request, 4096).unwrap();
    drop(input);
    let response: RpcErrorResponse = read_frame(&mut output, 4096).unwrap();
    assert_eq!(response.error.code, error_code::INCOMPATIBLE_VERSION);
    assert_eq!(child.wait().unwrap().code(), Some(0));
}

#[test]
fn independent_plugin_requires_negotiation_first() {
    let mut child = spawn_plugin();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    write_frame(
        &mut input,
        &RpcRequest {
            jsonrpc: JsonRpcVersion::V2,
            id: RequestId::Number(8),
            method: Method::Describe,
            params: serde_json::json!({}),
        },
        4096,
    )
    .unwrap();
    drop(input);
    let response: RpcErrorResponse = read_frame(&mut output, 4096).unwrap();
    assert_eq!(response.error.code, -32600);
    assert_eq!(child.wait().unwrap().code(), Some(0));
}

#[test]
fn independent_plugin_returns_explicit_unsupported_result() {
    let mut child = spawn_plugin();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    write_frame(
        &mut input,
        &RpcRequest {
            jsonrpc: JsonRpcVersion::V2,
            id: RequestId::Number(9),
            method: Method::Negotiate,
            params: serde_json::to_value(NegotiateParams {
                protocol: PROTOCOL_V1,
                limits: CallLimits {
                    max_frame_bytes: 4096,
                    timeout_ms: 1000,
                    max_in_flight: 1,
                },
                requested_capabilities: BTreeSet::new(),
            })
            .unwrap(),
        },
        4096,
    )
    .unwrap();
    let _: RpcResponse = read_frame(&mut output, 4096).unwrap();
    write_frame(
        &mut input,
        &RpcRequest {
            jsonrpc: JsonRpcVersion::V2,
            id: RequestId::Number(10),
            method: Method::Sample,
            params: serde_json::json!({}),
        },
        4096,
    )
    .unwrap();
    drop(input);
    let response: RpcResponse = read_frame(&mut output, 4096).unwrap();
    assert_eq!(response.result, serde_json::json!({"unsupported": true}));
    assert_eq!(child.wait().unwrap().code(), Some(0));
}

fn spawn_plugin() -> std::process::Child {
    Command::new(env!("CARGO_BIN_EXE_asb-example-plugin"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn sample plugin")
}
