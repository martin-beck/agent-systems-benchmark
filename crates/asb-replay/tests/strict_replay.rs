// SPDX-License-Identifier: MIT
//! Independent strict matcher, cursor, dialect, and loopback boundary tests.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use asb_replay::{
    Cassette, CassetteContents, CassetteEvent, CassetteLimits, Header, Interaction, MAX_IO_TIMEOUT,
    PolicyVersion, ProviderDialect, RecordedRequest, RecordedResponse, RedactionPolicy, Redactor,
    ReplayError, ReplayHttpRequest, ReplayLimits, ReplayRoute, ResponseBody, StrictReplayService,
    TerminalEvent, decode_cassette, dialect_capabilities, seal_cassette,
};
use serde_json::{Value, json};

fn headers() -> Vec<Header> {
    vec![
        Header {
            name: "authorization".into(),
            value: "synthetic-credential".into(),
        },
        Header {
            name: "content-type".into(),
            value: "application/json".into(),
        },
    ]
}

fn request(dialect: ProviderDialect, body: Value) -> RecordedRequest {
    let object = body.as_object().unwrap();
    let excluded: &[&str] = match dialect {
        ProviderDialect::OpenaiChatCompletions | ProviderDialect::AnthropicMessages => {
            &["messages", "model", "tools"]
        }
        ProviderDialect::OpenaiResponses => &["input", "model", "previous_response_id", "tools"],
        ProviderDialect::Synthetic => &[],
    };
    RecordedRequest {
        method: "POST".into(),
        path: match dialect {
            ProviderDialect::OpenaiChatCompletions => "/v1/chat/completions",
            ProviderDialect::OpenaiResponses => "/v1/responses",
            ProviderDialect::AnthropicMessages => "/v1/messages",
            ProviderDialect::Synthetic => "/v1/synthetic",
        }
        .into(),
        headers: headers(),
        model: object["model"].as_str().unwrap().into(),
        options: object
            .iter()
            .filter(|(name, _)| !excluded.contains(&name.as_str()))
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
        tools: object
            .get("tools")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        previous_response_id: object
            .get("previous_response_id")
            .and_then(Value::as_str)
            .map(str::to_owned),
        body,
        body_sha256: String::new(),
    }
}

fn buffered(payload: Value, response_id: &str) -> RecordedResponse {
    RecordedResponse {
        status: 200,
        headers: vec![Header {
            name: "content-type".into(),
            value: "application/json".into(),
        }],
        body: ResponseBody::Buffered {
            payload,
            payload_sha256: String::new(),
            response_id: Some(response_id.into()),
            terminal: TerminalEvent::Completed,
        },
    }
}

fn events(response_id: &str, prior: Option<&str>) -> RecordedResponse {
    RecordedResponse {
        status: 200,
        headers: vec![Header {
            name: "content-type".into(),
            value: "text/event-stream".into(),
        }],
        body: ResponseBody::Events {
            events: vec![
                CassetteEvent {
                    sequence: 0,
                    monotonic_offset_ns: 10,
                    event_type: "content.delta".into(),
                    payload: json!({"delta": "synthetic"}),
                    payload_sha256: String::new(),
                    response_id: Some(response_id.into()),
                    previous_response_id: prior.map(str::to_owned),
                    tool_call_id: Some(format!("tool-{response_id}")),
                    terminal: None,
                },
                CassetteEvent {
                    sequence: 1,
                    monotonic_offset_ns: 20,
                    event_type: "response.completed".into(),
                    payload: json!({"usage": {"output_tokens": 1}}),
                    payload_sha256: String::new(),
                    response_id: None,
                    previous_response_id: Some(response_id.into()),
                    tool_call_id: Some(format!("tool-{response_id}")),
                    terminal: Some(TerminalEvent::Completed),
                },
            ],
            transport_chunk_bytes: vec![2, 3, 5],
        },
    }
}

fn interaction(
    session: &str,
    ordinal: u32,
    dialect: ProviderDialect,
    body: Value,
    response: RecordedResponse,
) -> Interaction {
    Interaction {
        session_id: session.into(),
        attempt_id: "attempt-1".into(),
        interaction_id: format!("{session}-{ordinal}"),
        ordinal,
        dialect,
        request: request(dialect, body),
        response,
    }
}

fn cassette() -> Cassette {
    let chat = json!({
        "messages": [{"role": "user", "content": "synthetic"}],
        "model": "chat-model",
        "stream": true,
        "temperature": 0,
        "tools": [{"type": "function", "function": {"name": "lookup"}}]
    });
    let responses_first = json!({
        "input": "synthetic",
        "model": "responses-model",
        "stream": false,
        "tools": []
    });
    let responses_second = json!({
        "input": "continue",
        "model": "responses-model",
        "previous_response_id": "response-r0",
        "stream": true,
        "tools": [{"type": "function", "name": "lookup"}]
    });
    let messages = json!({
        "max_tokens": 16,
        "messages": [{"role": "user", "content": "synthetic"}],
        "model": "messages-model",
        "stream": false,
        "tools": []
    });
    let contents = CassetteContents {
        schema_version: 1,
        cassette_id: "strict-replay-synthetic".into(),
        normalization: PolicyVersion { version: 1 },
        redaction: RedactionPolicy::default().descriptor().unwrap(),
        interactions: vec![
            interaction(
                "chat-a",
                0,
                ProviderDialect::OpenaiChatCompletions,
                chat.clone(),
                events("response-chat-a", None),
            ),
            interaction(
                "chat-b",
                0,
                ProviderDialect::OpenaiChatCompletions,
                chat,
                events("response-chat-b", None),
            ),
            interaction(
                "responses-a",
                0,
                ProviderDialect::OpenaiResponses,
                responses_first,
                buffered(json!({"id": "response-r0", "output": []}), "response-r0"),
            ),
            interaction(
                "responses-a",
                1,
                ProviderDialect::OpenaiResponses,
                responses_second,
                events("response-r1", Some("response-r0")),
            ),
            interaction(
                "messages-a",
                0,
                ProviderDialect::AnthropicMessages,
                messages,
                buffered(
                    json!({"id": "response-m0", "content": [{"type": "text", "text": "ok"}]}),
                    "response-m0",
                ),
            ),
        ],
    };
    let redacted = Redactor::new(RedactionPolicy::default())
        .unwrap()
        .redact_contents(contents)
        .unwrap()
        .0;
    let bytes = seal_cassette(redacted, CassetteLimits::default()).unwrap();
    decode_cassette(&bytes, CassetteLimits::default()).unwrap()
}

fn reseal(mut source: Cassette, mutate: impl FnOnce(&mut CassetteContents)) -> Cassette {
    for interaction in &mut source.contents.interactions {
        for header in &mut interaction.request.headers {
            if header.name == "authorization" {
                header.value = "synthetic-credential".into();
            }
        }
    }
    mutate(&mut source.contents);
    let redacted = Redactor::new(RedactionPolicy::default())
        .unwrap()
        .redact_contents(source.contents)
        .unwrap()
        .0;
    let bytes = seal_cassette(redacted, CassetteLimits::default()).unwrap();
    decode_cassette(&bytes, CassetteLimits::default()).unwrap()
}

fn route(session: &str, dialect: ProviderDialect) -> ReplayRoute {
    ReplayRoute {
        session_id: session.into(),
        attempt_id: "attempt-1".into(),
        dialect,
    }
}

fn incoming(service_cassette: &Cassette, session: &str, ordinal: usize) -> ReplayHttpRequest {
    let interaction = service_cassette
        .contents
        .interactions
        .iter()
        .filter(|interaction| interaction.session_id == session)
        .nth(ordinal)
        .unwrap();
    ReplayHttpRequest {
        method: interaction.request.method.clone(),
        path: interaction.request.path.clone(),
        headers: vec![
            Header {
                name: "authorization".into(),
                value: "different-runtime-dummy-token".into(),
            },
            Header {
                name: "content-length".into(),
                value: "1".into(),
            },
            Header {
                name: "content-type".into(),
                value: "application/json".into(),
            },
            Header {
                name: "host".into(),
                value: "127.0.0.1".into(),
            },
        ],
        body: serde_json::to_vec(&interaction.request.body).unwrap(),
    }
}

#[test]
fn capabilities_are_explicit_and_do_not_include_synthetic() {
    let capabilities = dialect_capabilities();
    assert_eq!(capabilities.len(), 3);
    assert_eq!(capabilities[0].endpoint, "/v1/chat/completions");
    assert!(!capabilities[0].causal_response_ids);
    assert_eq!(capabilities[1].endpoint, "/v1/responses");
    assert!(capabilities[1].causal_response_ids);
    assert_eq!(capabilities[2].endpoint, "/v1/messages");
    assert!(
        capabilities
            .iter()
            .all(|item| { item.buffered && item.server_sent_events && item.tool_calls })
    );
}

#[test]
fn each_dialect_has_distinct_buffered_or_sse_wire_semantics() {
    let source = cassette();
    let chat_request = incoming(&source, "chat-a", 0);
    let responses_first = incoming(&source, "responses-a", 0);
    let responses_second = incoming(&source, "responses-a", 1);
    let messages_request = incoming(&source, "messages-a", 0);
    let service = StrictReplayService::new(source, ReplayLimits::default()).unwrap();

    let chat = service
        .handle(
            &route("chat-a", ProviderDialect::OpenaiChatCompletions),
            chat_request,
        )
        .unwrap();
    assert_eq!(chat.segments.len(), 3);
    assert!(chat.segments[0].starts_with(b"data: {"));
    assert_eq!(chat.segments[2], b"data: [DONE]\n\n");

    let buffered = service
        .handle(
            &route("responses-a", ProviderDialect::OpenaiResponses),
            responses_first,
        )
        .unwrap();
    assert_eq!(
        buffered.segments,
        [br#"{"id":"response-r0","output":[]}"#.to_vec()]
    );
    let streamed = service
        .handle(
            &route("responses-a", ProviderDialect::OpenaiResponses),
            responses_second,
        )
        .unwrap();
    assert!(streamed.segments[0].starts_with(b"event: content.delta\ndata: "));
    assert_eq!(streamed.segments.len(), 2);

    let messages = service
        .handle(
            &route("messages-a", ProviderDialect::AnthropicMessages),
            messages_request,
        )
        .unwrap();
    assert_eq!(messages.status, 200);
    assert!(messages.segments[0].starts_with(b"{\"content\":"));
}

#[test]
fn mismatch_and_wrong_dialect_do_not_advance_cursor() {
    let source = cassette();
    let correct = incoming(&source, "chat-a", 0);
    let mut changed = correct.clone();
    changed.body = br#"{"model":"different"}"#.to_vec();
    let service = StrictReplayService::new(source, ReplayLimits::default()).unwrap();
    let selected = route("chat-a", ProviderDialect::OpenaiChatCompletions);
    assert!(matches!(
        service.handle(&selected, changed),
        Err(ReplayError::Mismatch)
    ));
    let wrong = route("chat-a", ProviderDialect::AnthropicMessages);
    assert!(matches!(
        service.handle(&wrong, correct.clone()),
        Err(ReplayError::UnsupportedDialect)
    ));
    service.handle(&selected, correct.clone()).unwrap();
    assert!(matches!(
        service.handle(&selected, correct),
        Err(ReplayError::Exhausted)
    ));
}

#[test]
fn parallel_identical_sessions_have_independent_atomic_cursors() {
    let source = cassette();
    let request_a = incoming(&source, "chat-a", 0);
    let request_b = incoming(&source, "chat-b", 0);
    assert_eq!(request_a.body, request_b.body);
    let service = Arc::new(StrictReplayService::new(source, ReplayLimits::default()).unwrap());
    let handles: Vec<_> = [("chat-a", request_a), ("chat-b", request_b)]
        .into_iter()
        .map(|(session, request)| {
            let service = Arc::clone(&service);
            thread::spawn(move || {
                let response = service
                    .handle(
                        &route(session, ProviderDialect::OpenaiChatCompletions),
                        request,
                    )
                    .unwrap();
                assert_eq!(response.segments.len(), 3);
                assert_eq!(response.segments[2], b"data: [DONE]\n\n");
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }
}

#[test]
fn unknown_attempt_header_difference_and_duplicate_json_fail_closed() {
    let source = cassette();
    let correct = incoming(&source, "messages-a", 0);
    let service = StrictReplayService::new(source, ReplayLimits::default()).unwrap();
    let mut unknown = route("messages-a", ProviderDialect::AnthropicMessages);
    unknown.attempt_id = "attempt-missing".into();
    assert!(matches!(
        service.handle(&unknown, correct.clone()),
        Err(ReplayError::UnknownRoute)
    ));

    let mut changed_header = correct.clone();
    changed_header.headers.push(Header {
        name: "x-provider-feature".into(),
        value: "changed".into(),
    });
    changed_header
        .headers
        .sort_by(|left, right| left.name.cmp(&right.name));
    assert!(matches!(
        service.handle(
            &route("messages-a", ProviderDialect::AnthropicMessages),
            changed_header
        ),
        Err(ReplayError::Mismatch)
    ));

    let mut duplicate = correct.clone();
    duplicate.body = br#"{"model":"messages-model","model":"messages-model"}"#.to_vec();
    assert!(matches!(
        service.handle(
            &route("messages-a", ProviderDialect::AnthropicMessages),
            duplicate
        ),
        Err(ReplayError::InvalidHttp)
    ));
    service
        .handle(
            &route("messages-a", ProviderDialect::AnthropicMessages),
            correct,
        )
        .unwrap();
}

#[test]
fn request_bounds_and_normalization_fail_before_matching() {
    let source = cassette();
    let mut request = incoming(&source, "chat-a", 0);
    let service = StrictReplayService::new(
        source,
        ReplayLimits {
            max_body_bytes: request.body.len(),
            ..ReplayLimits::default()
        },
    )
    .unwrap();
    request.body.push(b' ');
    assert!(matches!(
        service.handle(
            &route("chat-a", ProviderDialect::OpenaiChatCompletions),
            request
        ),
        Err(ReplayError::InvalidHttp)
    ));

    for bad in [
        ReplayLimits {
            max_head_bytes: 0,
            ..ReplayLimits::default()
        },
        ReplayLimits {
            max_body_bytes: usize::MAX,
            ..ReplayLimits::default()
        },
        ReplayLimits {
            io_timeout: Duration::ZERO,
            ..ReplayLimits::default()
        },
        ReplayLimits {
            io_timeout: MAX_IO_TIMEOUT + Duration::from_nanos(1),
            ..ReplayLimits::default()
        },
        ReplayLimits {
            max_headers: 0,
            ..ReplayLimits::default()
        },
        ReplayLimits {
            max_headers: usize::MAX,
            ..ReplayLimits::default()
        },
    ] {
        assert!(matches!(
            StrictReplayService::new(cassette(), bad),
            Err(ReplayError::InvalidLimits)
        ));
    }
    assert!(
        StrictReplayService::new(
            cassette(),
            ReplayLimits {
                io_timeout: MAX_IO_TIMEOUT,
                ..ReplayLimits::default()
            }
        )
        .is_ok()
    );
}

#[test]
fn constructor_rejects_synthetic_and_inconsistent_metadata() {
    let synthetic = decode_cassette(
        include_bytes!("../fixtures/v1/buffered.json"),
        CassetteLimits::default(),
    )
    .unwrap();
    assert!(matches!(
        StrictReplayService::new(synthetic, ReplayLimits::default()),
        Err(ReplayError::UnsupportedDialect)
    ));

    let mut source = cassette();
    source.contents.interactions[0].dialect = ProviderDialect::Synthetic;
    assert!(matches!(
        StrictReplayService::new(source, ReplayLimits::default()),
        Err(ReplayError::InvalidCassette)
    ));
    let mut source = cassette();
    source.contents.interactions[0].request.model = "wrong".into();
    assert!(matches!(
        StrictReplayService::new(source, ReplayLimits::default()),
        Err(ReplayError::InvalidCassette)
    ));
    let mut source = cassette();
    source.contents.interactions[0].request.options = BTreeMap::new();
    assert!(matches!(
        StrictReplayService::new(source, ReplayLimits::default()),
        Err(ReplayError::InvalidCassette)
    ));

    let empty = CassetteContents {
        schema_version: 1,
        cassette_id: "empty-strict-replay".into(),
        normalization: PolicyVersion { version: 1 },
        redaction: RedactionPolicy::default().descriptor().unwrap(),
        interactions: Vec::new(),
    };
    let empty = Redactor::new(RedactionPolicy::default())
        .unwrap()
        .redact_contents(empty)
        .unwrap()
        .0;
    let empty = seal_cassette(empty, CassetteLimits::default()).unwrap();
    let empty = decode_cassette(&empty, CassetteLimits::default()).unwrap();
    assert!(matches!(
        StrictReplayService::new(empty, ReplayLimits::default()),
        Err(ReplayError::InvalidCassette)
    ));
}

#[test]
fn authenticated_dialect_metadata_inconsistencies_fail_before_serving() {
    let cases = [
        reseal(cassette(), |contents| {
            contents.interactions[0].request.path = "/v1/wrong".into();
        }),
        reseal(cassette(), |contents| {
            contents.interactions[0]
                .request
                .body
                .as_object_mut()
                .unwrap()
                .remove("messages");
        }),
        reseal(cassette(), |contents| {
            contents.interactions[0].request.tools.clear();
        }),
        reseal(cassette(), |contents| {
            contents.interactions[2]
                .request
                .body
                .as_object_mut()
                .unwrap()
                .insert("previous_response_id".into(), json!("body-only-reference"));
        }),
        reseal(cassette(), |contents| {
            contents.interactions[0]
                .request
                .body
                .as_object_mut()
                .unwrap()
                .insert("previous_response_id".into(), json!("body-only-reference"));
        }),
        reseal(cassette(), |contents| {
            contents.interactions[0].request.options.clear();
        }),
        reseal(cassette(), |contents| {
            contents.interactions[0].response = buffered(json!({"id": "other"}), "other");
        }),
        reseal(cassette(), |contents| {
            let ResponseBody::Events { events, .. } = &mut contents.interactions[0].response.body
            else {
                unreachable!()
            };
            events[0].event_type = "bad\r\ndata:".into();
        }),
    ];
    for invalid in cases {
        assert!(matches!(
            StrictReplayService::new(invalid, ReplayLimits::default()),
            Err(ReplayError::InvalidCassette)
        ));
    }
}

#[test]
fn direct_request_shape_and_header_adversaries_do_not_advance() {
    let source = cassette();
    let correct = incoming(&source, "chat-a", 0);
    let service = StrictReplayService::new(source, ReplayLimits::default()).unwrap();
    let selected = route("chat-a", ProviderDialect::OpenaiChatCompletions);
    let mut bad = correct.clone();
    bad.method = "GET".into();
    assert!(matches!(
        service.handle(&selected, bad),
        Err(ReplayError::InvalidHttp)
    ));
    let mut bad = correct.clone();
    bad.path = "/v1/responses".into();
    assert!(matches!(
        service.handle(&selected, bad),
        Err(ReplayError::Mismatch)
    ));
    let mut bad = correct.clone();
    bad.headers.push(Header {
        name: "cookie".into(),
        value: "dummy".into(),
    });
    bad.headers
        .sort_by(|left, right| left.name.cmp(&right.name));
    assert!(matches!(
        service.handle(&selected, bad),
        Err(ReplayError::Mismatch)
    ));
    for headers in [
        vec![Header {
            name: "Upper".into(),
            value: "x".into(),
        }],
        vec![
            Header {
                name: "x".into(),
                value: "a".into(),
            },
            Header {
                name: "x".into(),
                value: "b".into(),
            },
        ],
        vec![Header {
            name: "x".into(),
            value: "bad\r\nvalue".into(),
        }],
    ] {
        let mut bad = correct.clone();
        bad.headers = headers;
        assert!(matches!(
            service.handle(&selected, bad),
            Err(ReplayError::InvalidHttp)
        ));
    }
    service.handle(&selected, correct).unwrap();
}

#[test]
fn direct_entry_enforces_header_count_and_aggregate_head_bytes() {
    let source = cassette();
    let correct = incoming(&source, "chat-a", 0);
    let service = StrictReplayService::new(
        source,
        ReplayLimits {
            max_headers: correct.headers.len() - 1,
            ..ReplayLimits::default()
        },
    )
    .unwrap();
    assert!(matches!(
        service.handle(
            &route("chat-a", ProviderDialect::OpenaiChatCompletions),
            correct.clone()
        ),
        Err(ReplayError::InvalidHttp)
    ));

    let service = StrictReplayService::new(
        cassette(),
        ReplayLimits {
            max_head_bytes: 1,
            ..ReplayLimits::default()
        },
    )
    .unwrap();
    assert!(matches!(
        service.handle(
            &route("chat-a", ProviderDialect::OpenaiChatCompletions),
            correct
        ),
        Err(ReplayError::InvalidHttp)
    ));
}

fn serve_raw(service: Arc<StrictReplayService>, route: ReplayRoute, raw: Vec<u8>) -> Vec<u8> {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || service.serve_once(&listener, &route).unwrap());
    let mut client = TcpStream::connect(address).unwrap();
    client.write_all(&raw).unwrap();
    client.shutdown(std::net::Shutdown::Write).unwrap();
    let mut response = Vec::new();
    client.read_to_end(&mut response).unwrap();
    let status = server.join().unwrap();
    let wire_status: u16 = std::str::from_utf8(&response)
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(status, wire_status);
    response
}

#[test]
fn loopback_http_boundary_serves_exact_body_then_fails_closed() {
    let source = cassette();
    let request = incoming(&source, "messages-a", 0);
    let body = request.body.clone();
    let raw: Vec<u8> = format!(
        "POST /v1/messages HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: dummy\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        body.len()
    )
    .into_bytes()
    .into_iter()
    .chain(body)
    .collect();
    let service = Arc::new(StrictReplayService::new(source, ReplayLimits::default()).unwrap());
    let selected = route("messages-a", ProviderDialect::AnthropicMessages);
    let response = serve_raw(Arc::clone(&service), selected.clone(), raw.clone());
    assert!(response.starts_with(b"HTTP/1.1 200 OK\r\n"));
    assert!(
        response
            .windows(b"\"id\":\"response-m0\"".len())
            .any(|window| window == b"\"id\":\"response-m0\"")
    );

    let exhausted = serve_raw(service, selected, raw);
    assert!(exhausted.starts_with(b"HTTP/1.1 409 Conflict\r\n"));
    assert!(exhausted.ends_with(br#"{"error":"replay_mismatch"}"#));
}

#[test]
fn malformed_or_chunked_http_gets_generic_local_error() {
    let service = Arc::new(StrictReplayService::new(cassette(), ReplayLimits::default()).unwrap());
    let raw = b"POST /v1/messages HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked\r\nContent-Length: 0\r\n\r\n".to_vec();
    let response = serve_raw(
        Arc::clone(&service),
        route("messages-a", ProviderDialect::AnthropicMessages),
        raw,
    );
    assert!(response.starts_with(b"HTTP/1.1 400 Bad Request\r\n"));
    assert!(response.ends_with(br#"{"error":"invalid_request"}"#));

    let response = serve_raw(
        service,
        route("messages-a", ProviderDialect::AnthropicMessages),
        b"BROKEN\r\n\r\n".to_vec(),
    );
    assert!(response.starts_with(b"HTTP/1.1 400 Bad Request\r\n"));
}

#[test]
fn bounded_http_parser_rejects_each_unsupported_framing_form() {
    let cases = [
        b"POST /v1/messages HTTP/1.0\r\nContent-Length: 0\r\n\r\n".to_vec(),
        b"POST /v1/messages HTTP/1.1\r\nContent-Length:0\r\n\r\n".to_vec(),
        b"POST /v1/messages HTTP/1.1\r\nHost: x\r\n\r\n".to_vec(),
        b"POST /v1/messages HTTP/1.1\r\nContent-Length: 0\r\n\r\nextra".to_vec(),
        b"POST /v1/messages HTTP/1.1\r\nContent-Length: 999\r\n\r\n".to_vec(),
        b"POST /v1/messages HTTP/1.1\r\nContent-Length: +0\r\n\r\n".to_vec(),
        b"POST /v1/messages HTTP/1.1\r\nContent-Length: 00\r\n\r\n".to_vec(),
        b"POST /v1/messages HTTP/1.1\r\nContent-Length: 0".to_vec(),
    ];
    for raw in cases {
        let limits = if raw
            .windows(19)
            .any(|window| window == b"Content-Length: 999")
        {
            ReplayLimits {
                max_body_bytes: 8,
                ..ReplayLimits::default()
            }
        } else {
            ReplayLimits::default()
        };
        let service = Arc::new(StrictReplayService::new(cassette(), limits).unwrap());
        let response = serve_raw(
            service,
            route("messages-a", ProviderDialect::AnthropicMessages),
            raw,
        );
        assert!(response.starts_with(b"HTTP/1.1 400 Bad Request\r\n"));
    }

    let service = Arc::new(
        StrictReplayService::new(
            cassette(),
            ReplayLimits {
                max_head_bytes: 12,
                ..ReplayLimits::default()
            },
        )
        .unwrap(),
    );
    let response = serve_raw(
        service,
        route("messages-a", ProviderDialect::AnthropicMessages),
        b"POST /v1/messages HTTP/1.1\r\nContent-Length: 0\r\n\r\n".to_vec(),
    );
    assert!(response.starts_with(b"HTTP/1.1 400 Bad Request\r\n"));
}

#[test]
fn http_boundary_reports_unsupported_dialect_without_consuming() {
    let source = cassette();
    let body = incoming(&source, "chat-a", 0).body;
    let raw: Vec<u8> = format!(
        "POST /v1/chat/completions HTTP/1.1\r\nHost: localhost\r\nAuthorization: dummy\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        body.len()
    )
    .into_bytes()
    .into_iter()
    .chain(body)
    .collect();
    let service = Arc::new(StrictReplayService::new(source, ReplayLimits::default()).unwrap());
    let response = serve_raw(
        Arc::clone(&service),
        route("chat-a", ProviderDialect::AnthropicMessages),
        raw.clone(),
    );
    assert!(response.starts_with(b"HTTP/1.1 422 Unprocessable Content\r\n"));
    let response = serve_raw(
        service,
        route("chat-a", ProviderDialect::OpenaiChatCompletions),
        raw,
    );
    assert!(response.starts_with(b"HTTP/1.1 200 OK\r\n"));
}
