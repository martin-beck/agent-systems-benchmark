// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
#![no_main]

use asb_replay::{
    CassetteContents, CassetteEvent, Interaction, PolicyVersion, ProviderDialect, RecordedRequest,
    RecordedResponse, RedactionPolicy, Redactor, ReplayHttpRequest, ReplayLimits, ReplayRoute,
    ResponseBody, StrictReplayService, TerminalEvent, decode_cassette, seal_cassette,
};
use libfuzzer_sys::fuzz_target;
use serde_json::json;
use std::collections::BTreeMap;

fuzz_target!(|data: &[u8]| {
    let event_type = String::from_utf8_lossy(data.get(..data.len().min(128)).unwrap_or_default())
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    let body = json!({"messages": [], "model": "synthetic-model", "stream": true, "tools": []});
    let contents = CassetteContents {
        schema_version: 1,
        cassette_id: "fuzz-sse".into(),
        normalization: PolicyVersion { version: 1 },
        redaction: RedactionPolicy::default()
            .descriptor()
            .expect("static policy"),
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
                    events: vec![CassetteEvent {
                        sequence: 0,
                        monotonic_offset_ns: u64::from(data.first().copied().unwrap_or(0)),
                        event_type,
                        payload: json!({"bytes": data.get(..data.len().min(1024)).unwrap_or_default()}),
                        payload_sha256: String::new(),
                        response_id: Some("response".into()),
                        previous_response_id: None,
                        tool_call_id: None,
                        terminal: Some(TerminalEvent::Completed),
                    }],
                    transport_chunk_bytes: vec![1],
                },
            },
        }],
    };
    let Ok((redacted, _)) = Redactor::new(RedactionPolicy::default())
        .and_then(|redactor| redactor.redact_contents(contents))
    else {
        return;
    };
    let limits = Default::default();
    let Ok(encoded) = seal_cassette(redacted, limits) else {
        return;
    };
    let Ok(cassette) = decode_cassette(&encoded, limits) else {
        return;
    };
    let Ok(service) = StrictReplayService::new(cassette, ReplayLimits::default()) else {
        return;
    };
    let request = ReplayHttpRequest {
        method: "POST".into(),
        path: "/v1/chat/completions".into(),
        headers: vec![],
        body: serde_json::to_vec(&body).expect("static body"),
    };
    let route = ReplayRoute {
        session_id: "session".into(),
        attempt_id: "attempt".into(),
        dialect: ProviderDialect::OpenaiChatCompletions,
    };
    if let Ok(response) = service.handle(&route, request) {
        assert_eq!(response.segments.len(), response.recorded_offsets.len());
    }
});
