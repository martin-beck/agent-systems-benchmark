// SPDX-License-Identifier: MIT
//! Minimal external protocol-v1 plugin used by the conformance suite.

use std::collections::BTreeSet;
use std::io::{self, BufReader};

use asb_protocol::{
    CallLimits, Capability, ExtensionKind, ExtensionManifest, Id, JsonRpcVersion, MAX_FRAME_BYTES,
    Method, NegotiateParams, NegotiateResult, PROTOCOL_V1, RpcError, RpcErrorResponse, RpcRequest,
    RpcResponse, error_code, read_frame, write_frame,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = stdout.lock();
    let request: RpcRequest = read_frame(&mut reader, MAX_FRAME_BYTES as usize)?;
    if request.method != Method::Negotiate {
        let response = RpcErrorResponse {
            jsonrpc: JsonRpcVersion::V2,
            id: Some(request.id),
            error: RpcError {
                code: -32600,
                message: "negotiate must be first".into(),
                data: None,
            },
        };
        write_frame(&mut writer, &response, MAX_FRAME_BYTES as usize)?;
        return Ok(());
    }
    let params: NegotiateParams = serde_json::from_value(request.params)?;
    let protocol = match params.protocol.negotiate() {
        Ok(protocol) => protocol,
        Err(error) => {
            let response = RpcErrorResponse {
                jsonrpc: JsonRpcVersion::V2,
                id: Some(request.id),
                error: RpcError {
                    code: error_code::INCOMPATIBLE_VERSION,
                    message: error.to_string(),
                    data: None,
                },
            };
            write_frame(&mut writer, &response, MAX_FRAME_BYTES as usize)?;
            return Ok(());
        }
    };
    let supported = BTreeSet::from([Capability::Cancellation, Capability::Offline]);
    let capabilities = params
        .requested_capabilities
        .intersection(&supported)
        .cloned()
        .collect();
    let limits = params.limits.intersect(CallLimits {
        max_frame_bytes: 65_536,
        timeout_ms: 5_000,
        max_in_flight: 4,
    })?;
    let negotiated = RpcResponse {
        jsonrpc: JsonRpcVersion::V2,
        id: request.id,
        result: serde_json::to_value(NegotiateResult {
            protocol,
            limits,
            capabilities,
        })?,
    };
    write_frame(&mut writer, &negotiated, limits.max_frame_bytes as usize)?;

    let request: RpcRequest = read_frame(&mut reader, limits.max_frame_bytes as usize)?;
    let result = if request.method == Method::Describe {
        serde_json::to_value(ExtensionManifest {
            extension_id: Id("org.asb.example".into()),
            kind: ExtensionKind::Agent,
            implementation_version: "0.1.0".into(),
            protocol: PROTOCOL_V1,
            capabilities: supported,
            executable_sha256: "0000000000000000000000000000000000000000000000000000000000000000"
                .into(),
        })?
    } else {
        serde_json::json!({"unsupported": true})
    };
    write_frame(
        &mut writer,
        &RpcResponse {
            jsonrpc: JsonRpcVersion::V2,
            id: request.id,
            result,
        },
        limits.max_frame_bytes as usize,
    )?;
    Ok(())
}
