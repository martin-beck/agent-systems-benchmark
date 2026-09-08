// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
#![cfg(target_os = "linux")]
#![allow(missing_docs)]

use asb_agents::gemini::{GeminiArtifact, GeminiConfig, TESTED_BUNDLE_TREE_SHA256};
use asb_protocol::{Event, ExtensionEvent, Id, TerminalStatus};
use asb_replay::{
    Cassette, CassetteContents, CassetteEvent, CassetteLimits, Header, Interaction, PolicyVersion,
    ProviderDialect, RecordedRequest, RecordedResponse, RedactionPolicy, Redactor, ReplayError,
    ReplayLimits, ReplayRoute, ResponseBody, StrictReplayService, TerminalEvent, decode_cassette,
    seal_cassette,
};
use asb_runtime::ProcessLimits;
use asb_workloads::OriginalWorkloads;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use url::Url;

const SESSION: &str = "gemini-replay-session";
const ATTEMPT: &str = "gemini-replay-attempt";

struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        let parent = PathBuf::from(std::env::var_os("ASB_TEST_ROOT").expect("ASB_TEST_ROOT"));
        assert!(parent.is_absolute());
        assert!(parent.starts_with("/srv/data/projects/"));
        fs::create_dir_all(&parent).unwrap();
        let path = parent.join(format!(
            "gemini-replay-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[derive(Clone, PartialEq)]
struct CapturedRequest {
    path: String,
    header_names: Vec<String>,
    stable_headers: Vec<(String, String)>,
    body: Value,
}

fn limits() -> ProcessLimits {
    ProcessLimits::new(
        8 * 1024 * 1024,
        1024 * 1024,
        Duration::from_secs(30),
        Duration::from_millis(250),
        Duration::from_millis(5),
    )
    .unwrap()
}

fn adapter(root: &Path, workspace: &Path, endpoint: Url) -> GeminiConfig {
    GeminiConfig::new(
        PathBuf::from(std::env::var_os("ASB_GEMINI_BIN").expect("Gemini entry")),
        PathBuf::from(std::env::var_os("ASB_GEMINI_NODE_BIN").expect("Node binary")),
        workspace,
        root.join("state"),
        endpoint,
        "fixture-model",
        4,
        4,
        GeminiArtifact::LinuxX86_64V0_58_0,
    )
    .unwrap()
}

fn read_request(stream: &mut TcpStream) -> CapturedRequest {
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 8192];
    let head_end = loop {
        let count = stream.read(&mut chunk).unwrap();
        assert!(count > 0, "request ended before headers");
        bytes.extend_from_slice(&chunk[..count]);
        assert!(bytes.len() <= 4 * 1024 * 1024);
        if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let head = std::str::from_utf8(&bytes[..head_end]).unwrap();
    let mut request_line = head.lines().next().unwrap().split_whitespace();
    assert_eq!(request_line.next(), Some("POST"));
    let path = request_line.next().unwrap().to_owned();
    assert_eq!(request_line.next(), Some("HTTP/1.1"));
    let mut content_length = None;
    let mut header_names = Vec::new();
    let mut stable_headers = Vec::new();
    let mut sentinel_count = 0;
    for line in head.lines().skip(1).filter(|line| !line.is_empty()) {
        let (name, value) = line.split_once(":").unwrap();
        let name = name.to_ascii_lowercase();
        let value = value.trim();
        header_names.push(name.clone());
        if name == "content-length" {
            content_length = Some(value.parse::<usize>().unwrap());
        }
        if name == "x-goog-api-key" {
            assert_eq!(value, "asb-credential-free");
            sentinel_count += 1;
        }
        assert!(!matches!(name.as_str(), "authorization" | "cookie"));
        if !matches!(
            name.as_str(),
            "connection" | "content-length" | "host" | "x-goog-api-key"
        ) {
            stable_headers.push((name, value.to_owned()));
        }
    }
    assert_eq!(sentinel_count, 1);
    header_names.sort();
    stable_headers.sort();
    let content_length = content_length.unwrap();
    while bytes.len() - head_end < content_length {
        let count = stream.read(&mut chunk).unwrap();
        assert!(count > 0, "request ended before body");
        bytes.extend_from_slice(&chunk[..count]);
        assert!(bytes.len() <= 4 * 1024 * 1024);
    }
    assert_eq!(bytes.len() - head_end, content_length);
    CapturedRequest {
        path,
        header_names,
        stable_headers,
        body: serde_json::from_slice(&bytes[head_end..]).unwrap(),
    }
}

fn accept_bounded(listener: &TcpListener) -> TcpStream {
    listener.set_nonblocking(true).unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match listener.accept() {
            Ok((stream, peer)) => {
                assert!(peer.ip().is_loopback());
                return stream;
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("Gemini loopback accept failed: {error}"),
        }
    }
}

fn send_sse(stream: &mut TcpStream, payload: &Value) {
    let body = format!("data: {}\n\n", serde_json::to_string(payload).unwrap());
    write!(stream, "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}", body.len(), body).unwrap();
    stream.flush().unwrap();
}

fn send_retryable_error(stream: &mut TcpStream) {
    let body = serde_json::to_vec(&json!({
        "error": {
            "code": 500,
            "message": "synthetic transient failure",
            "status": "INTERNAL"
        }
    }))
    .unwrap();
    write!(
        stream,
        "HTTP/1.1 500 Synthetic\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    )
    .unwrap();
    stream.write_all(&body).unwrap();
    stream.flush().unwrap();
}

fn tool_response(output: &Path) -> Value {
    json!({
        "candidates": [{"content": {"parts": [{"functionCall": {
            "id": "call_asb_fix",
            "name": "write_file",
            "args": {"file_path": output, "content": "def parse_line(line):\n    if line.endswith(\"\\r\"):\n        line = line[:-1]\n    return line\n"}
        }}], "role": "model"}, "finishReason": "STOP", "index": 0}],
        "usageMetadata": {"promptTokenCount": 7, "candidatesTokenCount": 2, "totalTokenCount": 9}
    })
}

fn completion_response() -> Value {
    json!({
        "candidates": [{"content": {"parts": [{"text": "done"}], "role": "model"}, "finishReason": "STOP", "index": 0}],
        "usageMetadata": {"promptTokenCount": 8, "candidatesTokenCount": 1, "totalTokenCount": 9}
    })
}

fn recorded_request(captured: &CapturedRequest) -> RecordedRequest {
    let object = captured.body.as_object().unwrap();
    let mut headers = captured
        .stable_headers
        .iter()
        .filter(|(name, _)| !matches!(name.as_str(), "accept-encoding" | "user-agent"))
        .map(|(name, value)| Header {
            name: name.clone(),
            value: value.clone(),
        })
        .collect::<Vec<_>>();
    headers.push(Header {
        name: "x-goog-api-key".into(),
        value: "asb-credential-free".into(),
    });
    headers.sort_by(|left, right| left.name.cmp(&right.name));
    RecordedRequest {
        method: "POST".into(),
        path: captured.path.clone(),
        headers,
        body: captured.body.clone(),
        body_sha256: String::new(),
        model: "fixture-model".into(),
        options: object
            .iter()
            .filter(|(name, _)| !matches!(name.as_str(), "contents" | "tools"))
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
        tools: object["tools"].as_array().unwrap().clone(),
        previous_response_id: None,
    }
}

fn recorded_sse(payload: Value) -> RecordedResponse {
    let transport_bytes = serde_json::to_vec(&payload).unwrap().len() + "data: \n\n".len();
    RecordedResponse {
        status: 200,
        headers: vec![Header {
            name: "content-type".into(),
            value: "text/event-stream".into(),
        }],
        body: ResponseBody::Events {
            events: vec![CassetteEvent {
                sequence: 0,
                monotonic_offset_ns: 0,
                event_type: "gemini.generate_content.chunk".into(),
                payload,
                payload_sha256: String::new(),
                response_id: None,
                previous_response_id: None,
                tool_call_id: None,
                terminal: Some(TerminalEvent::Completed),
            }],
            transport_chunk_bytes: vec![u32::try_from(transport_bytes).unwrap()],
        },
    }
}

fn recorded_retry() -> RecordedResponse {
    RecordedResponse {
        status: 500,
        headers: vec![Header {
            name: "content-type".into(),
            value: "application/json".into(),
        }],
        body: ResponseBody::Buffered {
            payload: json!({
                "error": {
                    "code": 500,
                    "message": "synthetic transient failure",
                    "status": "INTERNAL"
                }
            }),
            payload_sha256: String::new(),
            response_id: None,
            terminal: TerminalEvent::Failed,
        },
    }
}

fn cassette(captured: &[CapturedRequest], responses: Vec<RecordedResponse>) -> Cassette {
    assert_eq!(captured.len(), responses.len());
    let mut policy = RedactionPolicy::default();
    policy.header_names.insert("x-goog-api-key".into());
    let contents = CassetteContents {
        schema_version: 1,
        cassette_id: "gemini-real-loopback-v1".into(),
        normalization: PolicyVersion { version: 1 },
        redaction: policy.descriptor().unwrap(),
        interactions: captured
            .iter()
            .zip(responses)
            .enumerate()
            .map(|(ordinal, (request, response))| Interaction {
                session_id: SESSION.into(),
                attempt_id: ATTEMPT.into(),
                interaction_id: format!("gemini-{ordinal}"),
                ordinal: u32::try_from(ordinal).unwrap(),
                dialect: ProviderDialect::GeminiGenerateContent,
                request: recorded_request(request),
                response,
            })
            .collect(),
    };
    let redacted = Redactor::new(policy)
        .unwrap()
        .redact_contents(contents)
        .unwrap()
        .0;
    let bytes = seal_cassette(redacted, CassetteLimits::default()).unwrap();
    assert!(
        !bytes
            .windows("asb-credential-free".len())
            .any(|window| { window == "asb-credential-free".as_bytes() })
    );
    decode_cassette(&bytes, CassetteLimits::default()).unwrap()
}

fn serve(
    service: Arc<StrictReplayService>,
    listener: TcpListener,
    count: usize,
) -> thread::JoinHandle<Vec<u16>> {
    thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        let route = ReplayRoute {
            session_id: SESSION.into(),
            attempt_id: ATTEMPT.into(),
            dialect: ProviderDialect::GeminiGenerateContent,
        };
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut statuses = Vec::new();
        while statuses.len() < count && Instant::now() < deadline {
            match service.serve_once(&listener, &route) {
                Ok(status) => statuses.push(status),
                Err(ReplayError::Io(error)) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(2));
                }
                Err(error) => panic!("privacy-safe Gemini replay failure: {error}"),
            }
        }
        statuses
    })
}

fn capture_journey(
    root: &Path,
    workspace: &Path,
    prompt: &str,
) -> (Vec<CapturedRequest>, Vec<ExtensionEvent>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
    let output = workspace.join("parser.py");
    let captured = Arc::new(Mutex::new(Vec::new()));
    let server_captured = Arc::clone(&captured);
    let server = thread::spawn(move || {
        let mut first = accept_bounded(&listener);
        server_captured
            .lock()
            .unwrap()
            .push(read_request(&mut first));
        send_sse(&mut first, &tool_response(&output));
        let mut second = accept_bounded(&listener);
        server_captured
            .lock()
            .unwrap()
            .push(read_request(&mut second));
        send_sse(&mut second, &completion_response());
    });
    let mut running = adapter(root, workspace, endpoint)
        .start(Id(SESSION.into()), Id(ATTEMPT.into()), prompt, limits())
        .unwrap();
    let outcome = running.wait().unwrap();
    server.join().unwrap();
    assert_eq!(outcome.status(), TerminalStatus::Completed);
    let requests = captured.lock().unwrap().clone();
    (requests, outcome.events().to_vec())
}

fn assert_loopback_only_namespace() {
    let interfaces = fs::read_to_string("/proc/net/dev").unwrap();
    let names = interfaces
        .lines()
        .skip(2)
        .filter_map(|line| line.split_once(":").map(|(name, _)| name.trim()))
        .collect::<Vec<_>>();
    assert_eq!(names, ["lo"]);
}

fn assert_trajectory_parity(left: &[ExtensionEvent], right: &[ExtensionEvent]) {
    assert_eq!(left.len(), right.len());
    for (left, right) in left.iter().zip(right) {
        assert_eq!(left.session_id, right.session_id);
        assert_eq!(left.attempt_id, right.attempt_id);
        assert_eq!(left.sequence, right.sequence);
        match (&left.event, &right.event) {
            (Event::ToolStarted { name: left, .. }, Event::ToolStarted { name: right, .. }) => {
                assert_eq!(left, right);
            }
            (
                Event::ToolFinished { success: left, .. },
                Event::ToolFinished { success: right, .. },
            ) => assert_eq!(left, right),
            (left, right) => assert_eq!(left, right),
        }
    }
    for events in [left, right] {
        for pair in events.windows(2) {
            if let (
                Event::ToolStarted {
                    tool_call_id: started,
                    ..
                },
                Event::ToolFinished {
                    tool_call_id: finished,
                    ..
                },
            ) = (&pair[0].event, &pair[1].event)
            {
                assert_eq!(started, finished);
            }
        }
    }
}

fn changed_paths(left: &Value, right: &Value, path: &str, output: &mut Vec<String>) {
    match (left, right) {
        (Value::Object(left), Value::Object(right)) => {
            let names = left.keys().chain(right.keys()).collect::<BTreeSet<_>>();
            for name in names {
                let child = format!("{path}/{}", name.replace("~", "~0").replace("/", "~1"));
                match (left.get(name), right.get(name)) {
                    (Some(left), Some(right)) => changed_paths(left, right, &child, output),
                    _ => output.push(child),
                }
            }
        }
        (Value::Array(left), Value::Array(right)) if left.len() == right.len() => {
            for (index, (left, right)) in left.iter().zip(right).enumerate() {
                changed_paths(left, right, &format!("{path}/{index}"), output);
            }
        }
        _ if left != right => output.push(path.to_owned()),
        _ => {}
    }
}

fn object_keys(value: &Value) -> Vec<&str> {
    value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect()
}

#[test]
fn malformed_cassette_is_rejected_before_gemini_service_start() {
    let valid = include_bytes!("../../asb-replay/fixtures/v1/events.json");
    let mut value: Value = serde_json::from_slice(valid).unwrap();
    value["contents"]["interactions"][0]["private_raw_response"] = json!("must-not-pass");
    assert!(
        decode_cassette(
            &serde_json::to_vec(&value).unwrap(),
            CassetteLimits::default()
        )
        .is_err()
    );
    assert!(decode_cassette(&valid[..valid.len() / 2], CassetteLimits::default()).is_err());
}

#[test]
#[ignore = "requires pinned Gemini CLI 0.58.0, Node.js 26.3.0, and loopback-only namespace"]
fn pinned_gemini_capture_is_stable_and_privacy_bounded() {
    assert_eq!(std::env::consts::ARCH, "x86_64");
    assert_loopback_only_namespace();
    assert_eq!(
        std::env::var("ASB_GEMINI_BUNDLE_TREE_SHA256").unwrap(),
        TESTED_BUNDLE_TREE_SHA256
    );
    let scratch = Scratch::new("capture");
    let prepared =
        OriginalWorkloads::prepare("original.bug-fix", scratch.0.join("workload")).unwrap();
    let (first, first_events) = capture_journey(
        &scratch.0.join("attempt"),
        &prepared.workspace(),
        prepared.prompt(),
    );
    let grade = prepared.evaluate().unwrap();
    assert!(grade.passed());
    prepared.reset().unwrap();
    let (second, second_events) = capture_journey(
        &scratch.0.join("attempt"),
        &prepared.workspace(),
        prepared.prompt(),
    );
    assert_eq!(prepared.evaluate().unwrap(), grade);
    assert_trajectory_parity(&first_events, &second_events);
    assert_eq!(first.len(), 2);
    assert_eq!(first.len(), second.len());
    assert_eq!(
        first[0].header_names,
        [
            "accept",
            "accept-encoding",
            "accept-language",
            "connection",
            "content-length",
            "content-type",
            "host",
            "sec-fetch-mode",
            "user-agent",
            "x-goog-api-client",
            "x-goog-api-key",
        ]
    );
    assert_eq!(
        object_keys(&first[0].body),
        ["contents", "generationConfig", "systemInstruction", "tools"]
    );
    assert_eq!(
        object_keys(&first[0].body["generationConfig"]),
        ["temperature", "thinkingConfig", "topK", "topP"]
    );
    assert!(first[0].body["generationConfig"]["temperature"].is_number());
    assert!(first[0].body["generationConfig"]["thinkingConfig"].is_object());
    assert!(first[0].body["generationConfig"]["topK"].is_number());
    assert!(first[0].body["generationConfig"]["topP"].is_number());
    assert_eq!(
        object_keys(&first[0].body["tools"][0]),
        ["functionDeclarations"]
    );
    assert_eq!(
        object_keys(&first[0].body["tools"][0]["functionDeclarations"][0]),
        ["description", "name", "parametersJsonSchema"]
    );
    assert_eq!(
        object_keys(&first[0].body["systemInstruction"]),
        ["parts", "role"]
    );
    assert_eq!(
        object_keys(&first[0].body["systemInstruction"]["parts"][0]),
        ["text"]
    );
    assert_eq!(
        first[1].body["contents"]
            .as_array()
            .unwrap()
            .iter()
            .map(|content| content["parts"]
                .as_array()
                .unwrap()
                .iter()
                .map(object_keys)
                .collect::<Vec<_>>())
            .collect::<Vec<_>>(),
        [
            vec![vec!["text"], vec!["text"]],
            vec![vec!["functionCall", "thoughtSignature"]],
            vec![vec!["functionResponse"]],
        ]
    );
    for (index, (left, right)) in first.iter().zip(&second).enumerate() {
        assert_eq!(
            left.path,
            "/v1beta/models/fixture-model:streamGenerateContent?alt=sse"
        );
        assert_eq!(left.path, right.path);
        assert_eq!(left.header_names, right.header_names);
        assert_eq!(left.stable_headers, right.stable_headers);
        let mut differences = Vec::new();
        changed_paths(&left.body, &right.body, "", &mut differences);
        assert!(
            differences.is_empty(),
            "request {index} volatility changed at {differences:?}"
        );
    }
    assert!(
        first[0]
            .body
            .pointer("/tools/0/functionDeclarations")
            .is_some_and(Value::is_array)
    );
    assert!(
        first[1]
            .body
            .pointer("/contents")
            .and_then(Value::as_array)
            .is_some_and(|contents| contents
                .iter()
                .any(|content| content.pointer("/parts/0/functionResponse").is_some()))
    );
    assert!(
        fs::read_dir(scratch.0.join("attempt/state"))
            .unwrap()
            .next()
            .is_none()
    );
}

#[test]
#[ignore = "requires pinned Gemini CLI 0.58.0, Node.js 26.3.0, and loopback-only namespace"]
fn pinned_gemini_retries_transient_failure_without_request_drift() {
    assert_eq!(std::env::consts::ARCH, "x86_64");
    assert_loopback_only_namespace();
    assert_eq!(
        std::env::var("ASB_GEMINI_BUNDLE_TREE_SHA256").unwrap(),
        TESTED_BUNDLE_TREE_SHA256
    );
    let scratch = Scratch::new("retry");
    let prepared =
        OriginalWorkloads::prepare("original.bug-fix", scratch.0.join("workload")).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
    let output = prepared.workspace().join("parser.py");
    let captured = Arc::new(Mutex::new(Vec::new()));
    let server_captured = Arc::clone(&captured);
    let server = thread::spawn(move || {
        let mut failed = accept_bounded(&listener);
        server_captured
            .lock()
            .unwrap()
            .push(read_request(&mut failed));
        send_retryable_error(&mut failed);
        let mut retried = accept_bounded(&listener);
        server_captured
            .lock()
            .unwrap()
            .push(read_request(&mut retried));
        send_sse(&mut retried, &tool_response(&output));
        let mut completed = accept_bounded(&listener);
        server_captured
            .lock()
            .unwrap()
            .push(read_request(&mut completed));
        send_sse(&mut completed, &completion_response());
    });
    let mut running = adapter(&scratch.0.join("attempt"), &prepared.workspace(), endpoint)
        .start(
            Id(SESSION.into()),
            Id(ATTEMPT.into()),
            prepared.prompt(),
            limits(),
        )
        .unwrap();
    let recorded = running.wait().unwrap();
    server.join().unwrap();
    assert_eq!(recorded.status(), TerminalStatus::Completed);
    let recorded_grade = prepared.evaluate().unwrap();
    assert!(recorded_grade.passed());
    let captured = captured.lock().unwrap().clone();
    assert_eq!(captured.len(), 3);
    assert!(
        captured[0] == captured[1],
        "retry request must remain byte-semantic exact"
    );
    let output = prepared.workspace().join("parser.py");
    let thinking_shapes = captured
        .iter()
        .map(|request| {
            request.body["generationConfig"]["thinkingConfig"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(name, value)| {
                    let kind = if value.is_boolean() {
                        "boolean"
                    } else if value.is_number() {
                        "number"
                    } else if value.is_string() {
                        "string"
                    } else if value.is_array() {
                        "array"
                    } else if value.is_object() {
                        "object"
                    } else {
                        "null"
                    };
                    (name.as_str(), kind)
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert!(
        thinking_shapes
            .iter()
            .all(|shape| shape.as_slice() == [("includeThoughts", "boolean")]),
        "captured thinkingConfig key/type shapes changed: {thinking_shapes:?}"
    );
    assert!(
        StrictReplayService::new(
            cassette(&captured[2..], vec![recorded_sse(completion_response())]),
            ReplayLimits::default()
        )
        .is_ok(),
        "captured completion contract changed"
    );
    assert!(
        StrictReplayService::new(
            cassette(
                &captured[..2],
                vec![recorded_retry(), recorded_sse(completion_response())]
            ),
            ReplayLimits::default()
        )
        .is_ok(),
        "captured retry contract changed"
    );
    assert!(
        StrictReplayService::new(
            cassette(
                &captured[1..],
                vec![
                    recorded_sse(tool_response(&output)),
                    recorded_sse(completion_response())
                ]
            ),
            ReplayLimits::default()
        )
        .is_ok(),
        "captured tool causality contract changed"
    );
    let cassette = cassette(
        &captured,
        vec![
            recorded_retry(),
            recorded_sse(tool_response(&output)),
            recorded_sse(completion_response()),
        ],
    );

    prepared.reset().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
    let service = Arc::new(StrictReplayService::new(cassette, ReplayLimits::default()).unwrap());
    let replay_server = serve(Arc::clone(&service), listener, captured.len());
    let mut replayed = adapter(&scratch.0.join("attempt"), &prepared.workspace(), endpoint)
        .start(
            Id(SESSION.into()),
            Id(ATTEMPT.into()),
            prepared.prompt(),
            limits(),
        )
        .unwrap();
    let replayed = replayed.wait().unwrap();
    let replay_statuses = replay_server.join().unwrap();
    assert_eq!(replay_statuses, [500, 200, 200]);
    assert_eq!(
        replayed.status(),
        TerminalStatus::Completed,
        "Gemini replay terminal mismatch: status={:?}, exit={:?}, events={}",
        replayed.status(),
        replayed.exit_code(),
        replayed.events().len()
    );
    assert_eq!(prepared.evaluate().unwrap(), recorded_grade);
    assert_trajectory_parity(recorded.events(), replayed.events());
    assert!(
        fs::read_dir(scratch.0.join("attempt/state"))
            .unwrap()
            .next()
            .is_none()
    );
}

#[test]
#[ignore = "requires pinned Gemini CLI 0.58.0, Node.js 26.3.0, and loopback-only namespace"]
fn pinned_gemini_cancellation_reaps_process_and_private_state() {
    assert_eq!(std::env::consts::ARCH, "x86_64");
    assert_loopback_only_namespace();
    assert_eq!(
        std::env::var("ASB_GEMINI_BUNDLE_TREE_SHA256").unwrap(),
        TESTED_BUNDLE_TREE_SHA256
    );
    let scratch = Scratch::new("cancel");
    let workspace = scratch.0.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
    let (seen_tx, seen_rx) = mpsc::sync_channel(1);
    let server = thread::spawn(move || {
        let mut stream = accept_bounded(&listener);
        let request = read_request(&mut stream);
        assert_eq!(
            request.path,
            "/v1beta/models/fixture-model:streamGenerateContent?alt=sse"
        );
        seen_tx.send(()).unwrap();
        thread::sleep(Duration::from_secs(2));
    });
    let mut running = adapter(&scratch.0.join("attempt"), &workspace, endpoint)
        .start(
            Id(SESSION.into()),
            Id(ATTEMPT.into()),
            "Wait for the replay response.",
            limits(),
        )
        .unwrap();
    seen_rx.recv_timeout(Duration::from_secs(15)).unwrap();
    running.cancel().unwrap();
    running.cancel().unwrap();
    assert_eq!(running.wait().unwrap().status(), TerminalStatus::Cancelled);
    server.join().unwrap();
    assert!(
        fs::read_dir(scratch.0.join("attempt/state"))
            .unwrap()
            .next()
            .is_none()
    );
}
