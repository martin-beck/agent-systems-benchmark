// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Real loopback disconnect regression for retry-safe replay delivery.

use asb_replay::{
    CassetteContents, CassetteEvent, Interaction, PolicyVersion, ProviderDialect, RecordedRequest,
    RecordedResponse, RedactionPolicy, Redactor, ReplayHttpRequest, ReplayLimits, ReplayRoute,
    ResponseBody, StrictReplayService, TerminalEvent, decode_cassette, seal_cassette,
};
use serde_json::json;
use std::collections::BTreeMap;
use std::io::Write;
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

fn fixture() -> (asb_replay::Cassette, ReplayHttpRequest, ReplayRoute) {
    let body = json!({"messages": [], "model": "synthetic-model", "stream": true, "tools": []});
    let contents = CassetteContents {
        schema_version: 1,
        cassette_id: "network-fault".into(),
        normalization: PolicyVersion { version: 1 },
        redaction: RedactionPolicy::default().descriptor().expect("descriptor"),
        interactions: vec![Interaction {
            session_id: "session".into(),
            attempt_id: "attempt".into(),
            interaction_id: "interaction".into(),
            ordinal: 0,
            dialect: ProviderDialect::OpenaiChatCompletions,
            request: RecordedRequest {
                method: "POST".into(),
                path: "/v1/chat/completions".into(),
                headers: vec![],
                body: body.clone(),
                body_sha256: String::new(),
                model: "synthetic-model".into(),
                options: BTreeMap::from([("stream".into(), json!(true))]),
                tools: vec![],
                previous_response_id: None,
            },
            response: RecordedResponse {
                status: 200,
                headers: vec![],
                body: ResponseBody::Events {
                    events: vec![
                        CassetteEvent {
                            sequence: 0,
                            monotonic_offset_ns: 0,
                            event_type: "response.delta".into(),
                            payload: json!({"delta": "x".repeat(4 * 1024 * 1024)}),
                            payload_sha256: String::new(),
                            response_id: Some("response".into()),
                            previous_response_id: None,
                            tool_call_id: None,
                            terminal: None,
                        },
                        CassetteEvent {
                            sequence: 1,
                            monotonic_offset_ns: 1,
                            event_type: "response.completed".into(),
                            payload: json!({"done": true}),
                            payload_sha256: String::new(),
                            response_id: None,
                            previous_response_id: Some("response".into()),
                            tool_call_id: None,
                            terminal: Some(TerminalEvent::Completed),
                        },
                    ],
                    transport_chunk_bytes: vec![1, 1],
                },
            },
        }],
    };
    let (redacted, _) = Redactor::new(RedactionPolicy::default())
        .expect("redactor")
        .redact_contents(contents)
        .expect("redaction");
    let limits = Default::default();
    let encoded = seal_cassette(redacted, limits).expect("seal");
    let cassette = decode_cassette(&encoded, limits).expect("decode");
    let request = ReplayHttpRequest {
        method: "POST".into(),
        path: "/v1/chat/completions".into(),
        headers: vec![],
        body: serde_json::to_vec(&body).expect("body"),
    };
    let route = ReplayRoute {
        session_id: "session".into(),
        attempt_id: "attempt".into(),
        dialect: ProviderDialect::OpenaiChatCompletions,
    };
    (cassette, request, route)
}

#[test]
fn peer_disconnect_before_sse_body_completion_keeps_interaction_retryable() {
    let (cassette, request, route) = fixture();
    let service = Arc::new(
        StrictReplayService::new(
            cassette,
            ReplayLimits {
                io_timeout: Duration::from_millis(50),
                ..ReplayLimits::default()
            },
        )
        .expect("service"),
    );
    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback bind");
    let address = listener.local_addr().expect("address");
    let server_service = Arc::clone(&service);
    let server_route = route.clone();
    let server = thread::spawn(move || server_service.serve_once(&listener, &server_route));
    let body = request.body.clone();
    let wire = format!(
        "POST /v1/chat/completions HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );
    let mut client = TcpStream::connect(address).expect("connect");
    client.write_all(wire.as_bytes()).expect("head");
    client.write_all(&body).expect("body");
    client.shutdown(Shutdown::Both).expect("disconnect");
    drop(client);
    assert!(server.join().expect("server thread").is_err());
    let response = service
        .handle(&route, request)
        .expect("interaction remains retryable");
    assert_eq!(response.segments.len(), 2);
}
