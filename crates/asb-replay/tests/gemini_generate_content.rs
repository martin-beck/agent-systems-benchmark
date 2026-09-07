// SPDX-License-Identifier: MIT
//! Bounded Gemini GenerateContent dialect and fail-closed replay tests.

use std::collections::{BTreeMap, BTreeSet};

use asb_replay::{
    Cassette, CassetteContents, CassetteEvent, CassetteLimits, Header, Interaction, PolicyVersion,
    ProviderDialect, RecordedRequest, RecordedResponse, RedactionPolicy, Redactor, ReplayError,
    ReplayHttpRequest, ReplayLimits, ReplayRoute, ResponseBody, StrictReplayService, TerminalEvent,
    decode_cassette, seal_cassette,
};
use serde_json::{Map, Value, json};

const PATH: &str = "/v1beta/models/fixture-model:streamGenerateContent?alt=sse";
const EVENT_TYPE: &str = "gemini.generate_content.chunk";

fn body() -> Value {
    json!({
        "contents": [{"parts": [{"text": "synthetic request"}], "role": "user"}],
        "generationConfig": {
            "temperature": 0,
            "thinkingConfig": {},
            "topK": 1,
            "topP": 1
        },
        "systemInstruction": {
            "parts": [{"text": "synthetic system instruction"}],
            "role": "user"
        },
        "tools": [{"functionDeclarations": [{
            "description": "Write a synthetic fixture",
            "name": "write_file",
            "parametersJsonSchema": {
                "additionalProperties": false,
                "properties": {"content": {"type": "string"}},
                "required": ["content"],
                "type": "object"
            }
        }]}]
    })
}

fn sync_metadata(request: &mut RecordedRequest) {
    let object = request.body.as_object().unwrap();
    request.tools = object["tools"].as_array().unwrap().clone();
    request.options = object
        .iter()
        .filter(|(name, _)| !matches!(name.as_str(), "contents" | "tools"))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
}

fn request(body: Value) -> RecordedRequest {
    let mut request = RecordedRequest {
        method: "POST".into(),
        path: PATH.into(),
        headers: vec![
            Header {
                name: "content-type".into(),
                value: "application/json".into(),
            },
            Header {
                name: "x-goog-api-key".into(),
                value: "synthetic-public-sentinel".into(),
            },
        ],
        body,
        body_sha256: String::new(),
        model: "fixture-model".into(),
        options: BTreeMap::new(),
        tools: Vec::new(),
        previous_response_id: None,
    };
    sync_metadata(&mut request);
    request
}

fn event(sequence: u32, payload: Value, terminal: Option<TerminalEvent>) -> CassetteEvent {
    CassetteEvent {
        sequence,
        monotonic_offset_ns: u64::from(sequence) * 10,
        event_type: EVENT_TYPE.into(),
        payload,
        payload_sha256: String::new(),
        response_id: None,
        previous_response_id: None,
        tool_call_id: None,
        terminal,
    }
}

fn success() -> RecordedResponse {
    RecordedResponse {
        status: 200,
        headers: vec![Header {
            name: "content-type".into(),
            value: "text/event-stream".into(),
        }],
        body: ResponseBody::Events {
            events: vec![event(
                0,
                json!({"candidates": [{"content": {"parts": [{"text": "done"}], "role": "model"}, "finishReason": "STOP", "index": 0}], "usageMetadata": {"candidatesTokenCount": 1, "promptTokenCount": 2, "totalTokenCount": 3}}),
                Some(TerminalEvent::Completed),
            )],
            transport_chunk_bytes: vec![11],
        },
    }
}

fn tool_success() -> RecordedResponse {
    RecordedResponse {
        status: 200,
        headers: vec![Header {
            name: "content-type".into(),
            value: "text/event-stream".into(),
        }],
        body: ResponseBody::Events {
            events: vec![event(
                0,
                json!({"candidates": [{"content": {"parts": [{"functionCall": {"args": {"content": "synthetic"}, "id": "call_asb_fix", "name": "write_file"}}], "role": "model"}, "finishReason": "STOP", "index": 0}], "usageMetadata": {"candidatesTokenCount": 1, "promptTokenCount": 2, "totalTokenCount": 3}}),
                Some(TerminalEvent::Completed),
            )],
            transport_chunk_bytes: vec![7],
        },
    }
}

fn tool_followup_body(id: &str, name: &str) -> Value {
    let mut value = body();
    value["contents"] = json!([
        {"parts": [{"text": "synthetic request"}], "role": "user"},
        {"parts": [{"functionCall": {"args": {"content": "synthetic"}, "id": id, "name": name}, "thoughtSignature": "synthetic-signature"}], "role": "model"},
        {"parts": [{"functionResponse": {"id": id, "name": name, "response": {"output": "ok"}}}], "role": "user"}
    ]);
    value
}

fn retry_error() -> RecordedResponse {
    RecordedResponse {
        status: 500,
        headers: vec![Header {
            name: "content-type".into(),
            value: "application/json".into(),
        }],
        body: ResponseBody::Buffered {
            payload: json!({"error": {"code": 500, "message": "synthetic retry", "status": "INTERNAL"}}),
            payload_sha256: String::new(),
            response_id: None,
            terminal: TerminalEvent::Failed,
        },
    }
}

fn interaction(ordinal: u32, response: RecordedResponse) -> Interaction {
    Interaction {
        session_id: "gemini-session".into(),
        attempt_id: "attempt-1".into(),
        interaction_id: format!("gemini-{ordinal}"),
        ordinal,
        dialect: ProviderDialect::GeminiGenerateContent,
        request: request(body()),
        response,
    }
}

fn contents(responses: Vec<RecordedResponse>) -> CassetteContents {
    CassetteContents {
        schema_version: 1,
        cassette_id: "gemini-generate-content-synthetic".into(),
        normalization: PolicyVersion { version: 1 },
        redaction: RedactionPolicy::default().descriptor().unwrap(),
        interactions: responses
            .into_iter()
            .enumerate()
            .map(|(ordinal, response)| interaction(u32::try_from(ordinal).unwrap(), response))
            .collect(),
    }
}

fn policy(pointers: impl IntoIterator<Item = &'static str>) -> RedactionPolicy {
    RedactionPolicy {
        header_names: BTreeSet::from(["x-goog-api-key".into()]),
        request_body_pointers: pointers.into_iter().map(str::to_owned).collect(),
        ..RedactionPolicy::default()
    }
}

fn seal_with(contents: CassetteContents, policy: RedactionPolicy) -> Cassette {
    let redacted = Redactor::new(policy)
        .unwrap()
        .redact_contents(contents)
        .unwrap()
        .0;
    let bytes = seal_cassette(redacted, CassetteLimits::default()).unwrap();
    decode_cassette(&bytes, CassetteLimits::default()).unwrap()
}

fn seal(contents: CassetteContents) -> Cassette {
    seal_with(contents, policy([]))
}

fn route() -> ReplayRoute {
    ReplayRoute {
        session_id: "gemini-session".into(),
        attempt_id: "attempt-1".into(),
        dialect: ProviderDialect::GeminiGenerateContent,
    }
}

fn incoming(cassette: &Cassette, ordinal: usize) -> ReplayHttpRequest {
    let request = &cassette.contents.interactions[ordinal].request;
    ReplayHttpRequest {
        method: request.method.clone(),
        path: request.path.clone(),
        headers: vec![
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
            Header {
                name: "x-goog-api-key".into(),
                value: "different-runtime-sentinel".into(),
            },
        ],
        body: serde_json::to_vec(&request.body).unwrap(),
    }
}

fn rejected(contents: CassetteContents) {
    let Ok((redacted, _)) = Redactor::new(policy([])).unwrap().redact_contents(contents) else {
        return;
    };
    let Ok(bytes) = seal_cassette(redacted, CassetteLimits::default()) else {
        return;
    };
    let Ok(cassette) = decode_cassette(&bytes, CassetteLimits::default()) else {
        return;
    };
    assert!(matches!(
        StrictReplayService::new(cassette, ReplayLimits::default()),
        Err(ReplayError::InvalidCassette)
    ));
}

#[test]
fn exact_retry_then_data_only_sse_is_ordered_and_byte_exact() {
    let cassette = seal(contents(vec![retry_error(), success()]));
    let service = StrictReplayService::new(cassette.clone(), ReplayLimits::default()).unwrap();
    let retry = service.handle(&route(), incoming(&cassette, 0)).unwrap();
    assert_eq!(retry.status, 500);
    assert_eq!(
        retry.segments,
        [br#"{"error":{"code":500,"message":"synthetic retry","status":"INTERNAL"}}"#.to_vec()]
    );
    let response = service.handle(&route(), incoming(&cassette, 1)).unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(response.segments.len(), 1);
    for segment in &response.segments {
        assert!(segment.starts_with(b"data: {") && segment.ends_with(b"\n\n"));
        assert!(!segment.windows(7).any(|part| part == b"event: "));
        assert!(!segment.windows(6).any(|part| part == b"[DONE]"));
    }
    assert!(matches!(
        service.handle(&route(), incoming(&cassette, 1)),
        Err(ReplayError::Exhausted)
    ));
}

#[test]
fn body_text_and_paired_tool_ids_remain_exact_and_markers_fail_closed() {
    let mut value = body();
    value["contents"] = json!([
        {"parts": [{"text": "semantic prompt"}], "role": "user"},
        {"parts": [{"functionCall": {"args": {}, "id": "call-recorded", "name": "write_file"}, "thoughtSignature": "synthetic-signature"}], "role": "model"},
        {"parts": [{"functionResponse": {"id": "call-recorded", "name": "write_file", "response": {"output": "ok"}}}], "role": "user"}
    ]);
    let mut source = contents(vec![success()]);
    source.interactions[0].request = request(value);
    let cassette = seal(source);
    let recorded = cassette.contents.interactions[0].request.body.clone();

    for pointer in [
        "/contents/0/parts/0/text",
        "/contents/1/parts/0/functionCall/id",
        "/contents/2/parts/0/functionResponse/id",
    ] {
        let service = StrictReplayService::new(cassette.clone(), ReplayLimits::default()).unwrap();
        let mut changed = recorded.clone();
        *changed.pointer_mut(pointer).unwrap() = json!("different-runtime-value");
        let mut request = incoming(&cassette, 0);
        request.body = serde_json::to_vec(&changed).unwrap();
        assert!(matches!(
            service.handle(&route(), request),
            Err(ReplayError::Mismatch)
        ));
    }

    let service = StrictReplayService::new(cassette.clone(), ReplayLimits::default()).unwrap();
    let mut injected = recorded;
    injected["contents"][0]["parts"][0]["text"] = json!("[ASB_REDACTED:000001]");
    let mut request = incoming(&cassette, 0);
    request.body = serde_json::to_vec(&injected).unwrap();
    assert!(matches!(
        service.handle(&route(), request),
        Err(ReplayError::InvalidHttp)
    ));
}

#[test]
fn tool_call_requires_exact_followup_id_and_name_pair() {
    let mut source = contents(vec![tool_success(), success()]);
    source.interactions[1].request = request(tool_followup_body("call_asb_fix", "write_file"));
    let cassette = seal(source.clone());
    StrictReplayService::new(cassette, ReplayLimits::default()).unwrap();

    for pointer in [
        "/contents/1/parts/0/functionCall/id",
        "/contents/1/parts/0/functionCall/name",
        "/contents/2/parts/0/functionResponse/id",
        "/contents/2/parts/0/functionResponse/name",
    ] {
        let mut changed = source.clone();
        *changed.interactions[1]
            .request
            .body
            .pointer_mut(pointer)
            .unwrap() = json!("contradictory");
        sync_metadata(&mut changed.interactions[1].request);
        rejected(changed);
    }

    rejected(contents(vec![tool_success()]));
}

#[test]
fn gemini_rejects_nonempty_request_body_selector_policy() {
    let mut source = contents(vec![success()]);
    source.interactions[0].request.body["contents"][0]["parts"][0]["text"] =
        json!("semantic prompt");
    sync_metadata(&mut source.interactions[0].request);
    let cassette = seal_with(source, policy(["/contents/0/parts/0/text"]));
    assert!(matches!(
        StrictReplayService::new(cassette, ReplayLimits::default()),
        Err(ReplayError::InvalidCassette)
    ));
}

#[test]
fn unsupported_routes_models_and_top_level_shapes_fail_closed() {
    for path in [
        "/v1beta/models/fixture-model:streamGenerateContent",
        "/v1beta/models/fixture-model:streamGenerateContent?alt=json",
        "/v1beta/models/other:streamGenerateContent?alt=sse",
        "/v1/models/fixture-model:streamGenerateContent?alt=sse",
    ] {
        let mut source = contents(vec![success()]);
        source.interactions[0].request.path = path.into();
        rejected(source);
    }
    for model in ["", "models/fixture", "fixture:model", "fixture model"] {
        let mut source = contents(vec![success()]);
        source.interactions[0].request.model = model.into();
        source.interactions[0].request.path =
            format!("/v1beta/models/{model}:streamGenerateContent?alt=sse");
        rejected(source);
    }
    for field in ["model", "stream", "safetySettings", "toolConfig"] {
        let mut source = contents(vec![success()]);
        source.interactions[0]
            .request
            .body
            .as_object_mut()
            .unwrap()
            .insert(field.into(), json!(true));
        sync_metadata(&mut source.interactions[0].request);
        rejected(source);
    }
}

#[test]
fn incoming_route_model_query_and_unrecorded_fields_do_not_advance() {
    let cassette = seal(contents(vec![success()]));
    for path in [
        "/v1beta/models/other:streamGenerateContent?alt=sse",
        "/v1beta/models/fixture-model:streamGenerateContent?alt=json",
        "/v1beta/models/fixture-model:streamGenerateContent?alt=sse&key=extra",
    ] {
        let service = StrictReplayService::new(cassette.clone(), ReplayLimits::default()).unwrap();
        let mut changed = incoming(&cassette, 0);
        changed.path = path.into();
        assert!(matches!(
            service.handle(&route(), changed),
            Err(ReplayError::Mismatch)
        ));
        assert_eq!(
            service
                .handle(&route(), incoming(&cassette, 0))
                .unwrap()
                .status,
            200
        );
    }

    let service = StrictReplayService::new(cassette.clone(), ReplayLimits::default()).unwrap();
    let mut changed = cassette.contents.interactions[0].request.body.clone();
    changed
        .as_object_mut()
        .unwrap()
        .insert("unrecorded".into(), json!(true));
    let mut request = incoming(&cassette, 0);
    request.body = serde_json::to_vec(&changed).unwrap();
    assert!(matches!(
        service.handle(&route(), request),
        Err(ReplayError::Mismatch)
    ));
    assert_eq!(
        service
            .handle(&route(), incoming(&cassette, 0))
            .unwrap()
            .status,
        200
    );
}

#[test]
fn malformed_generation_tool_content_and_metadata_shapes_fail_closed() {
    let mutate_body = |mutate: fn(&mut Map<String, Value>)| {
        let mut source = contents(vec![success()]);
        mutate(source.interactions[0].request.body.as_object_mut().unwrap());
        sync_metadata(&mut source.interactions[0].request);
        rejected(source);
    };
    mutate_body(|body| body["generationConfig"]["temperature"] = json!("zero"));
    mutate_body(|body| body["generationConfig"]["thinkingConfig"] = json!({"budget": 1}));
    mutate_body(|body| {
        body["generationConfig"]
            .as_object_mut()
            .unwrap()
            .insert("seed".into(), json!(1));
    });
    mutate_body(|body| body["tools"] = json!([]));
    mutate_body(|body| body["tools"][0]["functionDeclarations"][0]["name"] = json!(false));
    mutate_body(|body| {
        body["tools"][0]
            .as_object_mut()
            .unwrap()
            .insert("googleSearch".into(), json!({}));
    });
    mutate_body(|body| body["systemInstruction"]["parts"][0] = json!({"inlineData": {}}));
    mutate_body(|body| body["contents"][0]["parts"][0] = json!({"fileData": {}}));
    mutate_body(|body| body["contents"][0]["role"] = json!("tool"));
    mutate_body(|body| body["systemInstruction"]["role"] = json!("model"));
    mutate_body(|body| body["contents"][0]["parts"][0]["text"] = json!(""));
    mutate_body(|body| {
        body["contents"] = json!([
            {"parts": [{"functionCall": {"args": {}, "name": "write_file"}, "thoughtSignature": "signature"}], "role": "model"}
        ]);
    });
    mutate_body(|body| {
        body["contents"] = json!([
            {"parts": [{"functionResponse": {"id": "call", "name": "write_file"}}], "role": "user"}
        ]);
    });

    let mut mismatched_tools = contents(vec![success()]);
    mismatched_tools.interactions[0].request.tools.clear();
    rejected(mismatched_tools);
    let mut mismatched_options = contents(vec![success()]);
    mismatched_options.interactions[0].request.options.clear();
    rejected(mismatched_options);
    let mut causal = contents(vec![success()]);
    causal.interactions[0].request.previous_response_id = Some("unsupported".into());
    rejected(causal);
}

#[test]
fn contradictory_stream_and_error_responses_fail_closed() {
    let mut missing_retry = contents(vec![retry_error()]);
    rejected(missing_retry.clone());
    missing_retry.interactions.push(interaction(1, success()));
    missing_retry.interactions[1].request.body["contents"][0]["parts"][0]["text"] =
        json!("changed retry request");
    sync_metadata(&mut missing_retry.interactions[1].request);
    rejected(missing_retry);

    let mut buffered_success = contents(vec![retry_error()]);
    buffered_success.interactions[0].response.status = 200;
    rejected(buffered_success);

    let mut event_error = contents(vec![success()]);
    event_error.interactions[0].response.status = 500;
    rejected(event_error);

    let mut wrong_sse = contents(vec![success()]);
    wrong_sse.interactions[0].response.headers[0].value = "text/event-stream; charset=utf-8".into();
    rejected(wrong_sse);

    let mut named_sse = contents(vec![success()]);
    if let ResponseBody::Events { events, .. } = &mut named_sse.interactions[0].response.body {
        events[0].event_type = "message".into();
    }
    rejected(named_sse);

    for pointer in [
        "/candidates/0/content/role",
        "/candidates/0/finishReason",
        "/candidates/0/index",
        "/usageMetadata/totalTokenCount",
    ] {
        let mut malformed = contents(vec![success()]);
        let ResponseBody::Events { events, .. } = &mut malformed.interactions[0].response.body
        else {
            unreachable!();
        };
        *events[0].payload.pointer_mut(pointer).unwrap() = json!("invalid");
        rejected(malformed);
    }
    let mut extra_payload = contents(vec![success()]);
    if let ResponseBody::Events { events, .. } = &mut extra_payload.interactions[0].response.body {
        events[0].payload["extra"] = json!(true);
    }
    rejected(extra_payload);
    let mut multiple_events = contents(vec![success()]);
    if let ResponseBody::Events { events, .. } = &mut multiple_events.interactions[0].response.body
    {
        events.push(events[0].clone());
        events[1].sequence = 1;
        events[1].monotonic_offset_ns = 10;
    }
    rejected(multiple_events);

    let mut completed_error = contents(vec![retry_error()]);
    if let ResponseBody::Buffered { terminal, .. } =
        &mut completed_error.interactions[0].response.body
    {
        *terminal = TerminalEvent::Completed;
    }
    rejected(completed_error);

    let mut extra_error = contents(vec![retry_error()]);
    if let ResponseBody::Buffered { payload, .. } = &mut extra_error.interactions[0].response.body {
        payload["error"]["retryAfter"] = json!(1);
    }
    rejected(extra_error);

    let mut fractional_code = contents(vec![retry_error()]);
    if let ResponseBody::Buffered { payload, .. } =
        &mut fractional_code.interactions[0].response.body
    {
        payload["error"]["code"] = json!(500.5);
    }
    rejected(fractional_code);

    let mut wrong_status = contents(vec![retry_error()]);
    if let ResponseBody::Buffered { payload, .. } = &mut wrong_status.interactions[0].response.body
    {
        payload["error"]["status"] = json!("UNAVAILABLE");
    }
    rejected(wrong_status);
}
