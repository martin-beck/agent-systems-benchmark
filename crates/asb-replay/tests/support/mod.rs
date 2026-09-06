// SPDX-License-Identifier: MIT
//! Synthetic cassette construction shared across independent integration tests.

#![allow(dead_code)]

use std::collections::BTreeMap;

use asb_replay::{
    CassetteContents, CassetteEvent, Header, Interaction, PolicyVersion, ProviderDialect,
    RecordedRequest, RecordedResponse, RedactedCassetteContents, RedactionPolicy, Redactor,
    ResponseBody, TerminalEvent,
};
use serde_json::json;

pub fn contents() -> CassetteContents {
    CassetteContents {
        schema_version: 1,
        cassette_id: "synthetic-cassette".into(),
        normalization: PolicyVersion { version: 1 },
        redaction: PolicyVersion { version: 1 },
        interactions: vec![buffered(), streamed()],
    }
}

pub fn redacted_contents() -> RedactedCassetteContents {
    Redactor::new(RedactionPolicy::default())
        .unwrap()
        .redact_contents(contents())
        .unwrap()
        .0
}

pub fn request() -> RecordedRequest {
    RecordedRequest {
        method: "POST".into(),
        path: "/v1/synthetic?mode=test".into(),
        headers: vec![Header {
            name: "content-type".into(),
            value: "application/json".into(),
        }],
        body: json!({"messages": [{"content": "synthetic prompt"}]}),
        body_sha256: String::new(),
        model: "synthetic-model".into(),
        options: BTreeMap::from([("temperature".into(), json!(0))]),
        tools: vec![json!({"name": "synthetic_tool"})],
        previous_response_id: None,
    }
}

fn buffered() -> Interaction {
    Interaction {
        session_id: "session-alpha".into(),
        attempt_id: "attempt-alpha".into(),
        interaction_id: "interaction-alpha-0".into(),
        ordinal: 0,
        dialect: ProviderDialect::Synthetic,
        request: request(),
        response: RecordedResponse {
            status: 200,
            headers: vec![Header {
                name: "content-type".into(),
                value: "application/json".into(),
            }],
            body: ResponseBody::Buffered {
                payload: json!({"id": "response-alpha", "output": "synthetic answer"}),
                payload_sha256: String::new(),
                response_id: Some("response-alpha".into()),
                terminal: TerminalEvent::Completed,
            },
        },
    }
}

fn streamed() -> Interaction {
    let mut request = request();
    request.previous_response_id = Some("response-alpha".into());
    Interaction {
        session_id: "session-alpha".into(),
        attempt_id: "attempt-alpha".into(),
        interaction_id: "interaction-alpha-1".into(),
        ordinal: 1,
        dialect: ProviderDialect::Synthetic,
        request,
        response: RecordedResponse {
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
                        event_type: "tool_call".into(),
                        payload: json!({"name": "synthetic_tool"}),
                        payload_sha256: String::new(),
                        response_id: Some("response-beta".into()),
                        previous_response_id: Some("response-alpha".into()),
                        tool_call_id: Some("call-alpha".into()),
                        terminal: None,
                    },
                    CassetteEvent {
                        sequence: 1,
                        monotonic_offset_ns: 20,
                        event_type: "completed".into(),
                        payload: json!({"usage": {"output_tokens": 2}}),
                        payload_sha256: String::new(),
                        response_id: None,
                        previous_response_id: Some("response-beta".into()),
                        tool_call_id: Some("call-alpha".into()),
                        terminal: Some(TerminalEvent::Completed),
                    },
                ],
                transport_chunk_bytes: vec![1, 7, 3],
            },
        },
    }
}
