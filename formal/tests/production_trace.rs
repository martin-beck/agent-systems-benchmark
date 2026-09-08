// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Real Linux traces connecting the finite models to production boundaries.

use asb_formal_models::{
    AttemptEvent, AttemptState, microseconds_to_nanoseconds, transition_attempt,
};
use asb_metrics::LinuxCollector;
use asb_protocol::MetricValue;
use asb_replay::{
    CassetteContents, CassetteLimits, Header, Interaction, PolicyVersion, ProviderDialect,
    RecordedRequest, RecordedResponse, RedactionPolicy, Redactor, ReplayError, ReplayHttpRequest,
    ReplayLimits, ReplayRoute, ResponseBody, StrictReplayService, TerminalEvent, decode_cassette,
    seal_cassette,
};
use asb_runtime::{ProcessLifecycle, ProcessLimits, RunningProcess, Termination};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::process::Command;
use std::time::{Duration, SystemTime};

fn unique_root() -> std::path::PathBuf {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("asb-formal-trace-{unique}"))
}

#[test]
fn checked_unit_model_matches_the_real_cgroup_collector() {
    let root = unique_root();
    let group = root.join("trace");
    fs::create_dir_all(&group).unwrap();
    fs::write(
        group.join("cpu.stat"),
        "usage_usec 7\nuser_usec 3\nsystem_usec 4\nnr_periods 1\n\
         nr_throttled 0\nthrottled_usec 0\n",
    )
    .unwrap();
    let collection = LinuxCollector::with_roots("/unused", &root)
        .collect_cgroup("trace", 0)
        .unwrap();
    fs::remove_dir_all(root).unwrap();
    let production_value = collection
        .samples()
        .iter()
        .find(|sample| sample.descriptor.metric_id.0 == "cgroup.cpu.usage")
        .and_then(|sample| match sample.value {
            MetricValue::Available { value } => Some(value),
            MetricValue::Unavailable { .. } => None,
        })
        .unwrap();
    assert_eq!(
        production_value,
        microseconds_to_nanoseconds(7).unwrap() as f64
    );
}

#[test]
fn attempt_model_matches_real_idempotent_process_cancellation() {
    let limits = ProcessLimits::new(
        4096,
        4096,
        Duration::from_secs(5),
        Duration::from_millis(20),
        Duration::from_millis(1),
    )
    .unwrap();
    let mut command = Command::new("/bin/sh");
    command.args(["-c", "trap '' TERM; sleep 10"]);

    let mut model = transition_attempt(AttemptState::Planned, AttemptEvent::Start).unwrap();
    let mut process = RunningProcess::spawn(command, limits).unwrap();
    process.cancel().unwrap();
    process.cancel().unwrap();
    let first = process.wait().unwrap().clone();
    assert_eq!(first.termination, Termination::Cancelled);
    assert_eq!(process.lifecycle(), ProcessLifecycle::Terminal);
    assert_eq!(process.wait().unwrap(), &first);

    model = transition_attempt(model, AttemptEvent::Collect).unwrap();
    model = transition_attempt(model, AttemptEvent::Complete).unwrap();
    assert_eq!(model, AttemptState::Terminal);
    assert_eq!(
        transition_attempt(model, AttemptEvent::Complete),
        None,
        "real terminal evidence cannot authorize a second model completion"
    );
}

fn replay_interaction(session_id: &str, response_id: &str) -> Interaction {
    let body = json!({
        "messages": [{"role": "user", "content": "public synthetic prompt"}],
        "model": "synthetic-model",
        "stream": false,
        "temperature": 0
    });
    let mut options = BTreeMap::new();
    options.insert("stream".to_owned(), Value::Bool(false));
    options.insert("temperature".to_owned(), Value::from(0));
    Interaction {
        session_id: session_id.to_owned(),
        attempt_id: "attempt-1".to_owned(),
        interaction_id: format!("{session_id}-0"),
        ordinal: 0,
        dialect: ProviderDialect::OpenaiChatCompletions,
        request: RecordedRequest {
            method: "POST".to_owned(),
            path: "/v1/chat/completions".to_owned(),
            headers: vec![
                Header {
                    name: "authorization".to_owned(),
                    value: "synthetic-capture-value".to_owned(),
                },
                Header {
                    name: "content-type".to_owned(),
                    value: "application/json".to_owned(),
                },
            ],
            model: "synthetic-model".to_owned(),
            options,
            tools: Vec::new(),
            previous_response_id: None,
            body,
            body_sha256: String::new(),
        },
        response: RecordedResponse {
            status: 200,
            headers: vec![Header {
                name: "content-type".to_owned(),
                value: "application/json".to_owned(),
            }],
            body: ResponseBody::Buffered {
                payload: json!({"id": response_id, "choices": []}),
                payload_sha256: String::new(),
                response_id: Some(response_id.to_owned()),
                terminal: TerminalEvent::Completed,
            },
        },
    }
}

fn replay_request(interaction: &Interaction) -> ReplayHttpRequest {
    ReplayHttpRequest {
        method: interaction.request.method.clone(),
        path: interaction.request.path.clone(),
        headers: vec![
            Header {
                name: "authorization".to_owned(),
                value: "different-runtime-value".to_owned(),
            },
            Header {
                name: "content-type".to_owned(),
                value: "application/json".to_owned(),
            },
        ],
        body: serde_json::to_vec(&interaction.request.body).unwrap(),
    }
}

fn replay_route(session_id: &str) -> ReplayRoute {
    ReplayRoute {
        session_id: session_id.to_owned(),
        attempt_id: "attempt-1".to_owned(),
        dialect: ProviderDialect::OpenaiChatCompletions,
    }
}

#[test]
fn cursor_model_matches_real_strict_replay_isolation_and_mismatch_rollback() {
    let interactions = vec![
        replay_interaction("session-a", "response-a"),
        replay_interaction("session-b", "response-b"),
    ];
    let request_a = replay_request(&interactions[0]);
    let request_b = replay_request(&interactions[1]);
    assert_eq!(request_a.body, request_b.body);
    let contents = CassetteContents {
        schema_version: 1,
        cassette_id: "formal-production-trace".to_owned(),
        normalization: PolicyVersion { version: 1 },
        redaction: RedactionPolicy::default().descriptor().unwrap(),
        interactions,
    };
    let redacted = Redactor::new(RedactionPolicy::default())
        .unwrap()
        .redact_contents(contents)
        .unwrap()
        .0;
    let encoded = seal_cassette(redacted, CassetteLimits::default()).unwrap();
    let cassette = decode_cassette(&encoded, CassetteLimits::default()).unwrap();
    let service = StrictReplayService::new(cassette, ReplayLimits::default()).unwrap();

    let mut mismatch = request_a.clone();
    mismatch.body = br#"{"model":"wrong"}"#.to_vec();
    assert!(matches!(
        service.handle(&replay_route("session-a"), mismatch),
        Err(ReplayError::Mismatch)
    ));
    let response_b = service
        .handle(&replay_route("session-b"), request_b)
        .unwrap();
    let response_a = service
        .handle(&replay_route("session-a"), request_a)
        .unwrap();
    assert_ne!(response_a.segments, response_b.segments);

    let mut cursors = asb_formal_models::ReplayCursors::default();
    assert!(cursors.advance(1));
    assert_eq!(cursors.cursor(0), Some(0));
    assert!(cursors.advance(0));
    assert_eq!(cursors.cursor(0), Some(1));
    assert_eq!(cursors.cursor(1), Some(1));
}
