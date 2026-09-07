// SPDX-License-Identifier: MIT
#![cfg(target_os = "linux")]
#![allow(missing_docs)]

use asb_agents::codex::{CodexArtifact, CodexConfig, TESTED_LINUX_X86_64_SHA256};
use asb_protocol::{Event, ExtensionEvent, Id, TerminalStatus};
use asb_replay::{
    CancellationToken, Cassette, CassetteContents, CassetteEvent, CassetteLimits, Header,
    Interaction, PacingConfig, PacingError, PacingMode, PolicyVersion, ProviderDialect,
    RecordedRequest, RecordedResponse, RedactionPolicy, Redactor, ReplayError, ReplayLimits,
    ReplayRoute, ResponseBody, StrictReplayService, TerminalEvent, decode_cassette, seal_cassette,
};
use asb_runtime::ProcessLimits;
use asb_workloads::OriginalWorkloads;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use url::Url;

const SESSION: &str = "codex-replay-session";
const ATTEMPT: &str = "codex-replay-attempt";

struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        let parent = PathBuf::from(std::env::var_os("ASB_TEST_ROOT").expect("ASB_TEST_ROOT"));
        assert!(parent.is_absolute());
        assert!(parent.starts_with("/srv/data/projects/"));
        fs::create_dir_all(&parent).unwrap();
        let path = parent.join(format!(
            "codex-replay-{label}-{}-{}",
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

#[derive(Clone)]
struct Exchange {
    request: RecordedRequest,
    response: RecordedResponse,
}

fn limits(timeout: Duration) -> ProcessLimits {
    ProcessLimits::new(
        8 * 1024 * 1024,
        1024 * 1024,
        timeout,
        Duration::from_millis(250),
        Duration::from_millis(5),
    )
    .unwrap()
}

fn adapter(binary: &Path, workspace: &Path, state: &Path, endpoint: Url) -> CodexConfig {
    CodexConfig::new(
        binary,
        workspace,
        state,
        endpoint,
        "fixture-model",
        CodexArtifact::LinuxX86_64V0_153_4,
    )
    .unwrap()
}

fn read_request(stream: &mut TcpStream) -> RecordedRequest {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut bytes = Vec::new();
    let head_end = loop {
        let mut chunk = [0_u8; 4096];
        let count = stream.read(&mut chunk).unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&chunk[..count]);
        assert!(bytes.len() <= 4 * 1024 * 1024);
        if let Some(position) = bytes.windows(4).position(|item| item == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let head = std::str::from_utf8(&bytes[..head_end]).unwrap();
    let first = head.lines().next().unwrap();
    let mut first = first.split_whitespace();
    assert_eq!(first.next(), Some("POST"));
    let path = first.next().unwrap().to_owned();
    assert_eq!(path, "/v1/responses");
    let mut length = None;
    let mut headers = Vec::new();
    for line in head.lines().skip(1).filter(|line| !line.is_empty()) {
        let (name, value) = line.split_once(": ").unwrap();
        let name = name.to_ascii_lowercase();
        if name == "content-length" {
            length = Some(value.parse::<usize>().unwrap());
        }
        if !matches!(
            name.as_str(),
            "accept-encoding" | "connection" | "content-length" | "host" | "user-agent"
        ) {
            headers.push(Header {
                name,
                value: value.to_owned(),
            });
        }
    }
    headers.sort_by(|left, right| left.name.cmp(&right.name));
    assert_eq!(
        headers
            .iter()
            .find(|header| header.name == "authorization")
            .map(|header| header.value.as_str()),
        Some("Bearer asb-credential-free-fixture")
    );
    let length = length.unwrap();
    let already = bytes.len();
    assert!(already <= head_end + length);
    bytes.resize(head_end + length, 0);
    stream.read_exact(&mut bytes[already..]).unwrap();
    let body: Value = serde_json::from_slice(&bytes[head_end..]).unwrap();
    assert_eq!(
        body.get("model").and_then(Value::as_str),
        Some("fixture-model")
    );
    let object = body.as_object().unwrap();
    let options = object
        .iter()
        .filter(|(name, _)| {
            !matches!(
                name.as_str(),
                "input" | "model" | "previous_response_id" | "tools"
            )
        })
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect::<BTreeMap<_, _>>();
    let tools = object
        .get("tools")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let previous_response_id = object
        .get("previous_response_id")
        .and_then(Value::as_str)
        .map(str::to_owned);
    RecordedRequest {
        method: "POST".into(),
        path,
        headers,
        body,
        body_sha256: String::new(),
        model: "fixture-model".into(),
        options,
        tools,
        previous_response_id,
    }
}

fn response_base(id: &str, status: &str, output: Vec<Value>, usage: Value) -> Value {
    json!({
        "id": id, "object": "response", "created_at": 1,
        "status": status, "error": null, "incomplete_details": null,
        "instructions": null, "max_output_tokens": null, "model": "fixture-model",
        "output": output, "parallel_tool_calls": true, "previous_response_id": null,
        "reasoning": {"effort": null, "summary": null}, "store": false,
        "temperature": 1.0, "text": {"format": {"type": "text"}},
        "tool_choice": "auto", "tools": [], "top_p": 1.0,
        "truncation": "disabled", "usage": usage, "metadata": {}
    })
}

fn events(ordinal: usize, request: &RecordedRequest) -> (String, Vec<(String, Value)>) {
    let response_id = format!("resp_fixture_{ordinal}");
    let has_tool_result = request.body["input"].as_array().is_some_and(|inputs| {
        inputs
            .iter()
            .any(|input| input["type"] == "function_call_output")
    });
    if !has_tool_result {
        let command = "printf %s ZGVmIHBhcnNlX2xpbmUobGluZSk6CiAgICBpZiBsaW5lLmVuZHN3aXRoKCJcciIpOgogICAgICAgIGxpbmUgPSBsaW5lWzotMV0KICAgIHJldHVybiBsaW5lCg== | base64 -d > parser.py";
        let arguments = json!({
            "cmd": command,
            "yield_time_ms": 1000,
            "max_output_tokens": 2000
        })
        .to_string();
        let call = json!({
            "id": "fc_fixture", "type": "function_call", "status": "completed",
            "call_id": "call_fixture", "name": "exec_command", "arguments": arguments
        });
        let events = vec![
            (
                "response.created".into(),
                json!({"type":"response.created","response":response_base(&response_id, "in_progress", vec![], Value::Null)}),
            ),
            (
                "response.output_item.added".into(),
                json!({"type":"response.output_item.added","output_index":0,"item":call}),
            ),
            (
                "response.function_call_arguments.done".into(),
                json!({"type":"response.function_call_arguments.done","item_id":"fc_fixture","output_index":0,"name":"exec_command","arguments":arguments}),
            ),
            (
                "response.output_item.done".into(),
                json!({"type":"response.output_item.done","output_index":0,"item":call}),
            ),
            (
                "response.completed".into(),
                json!({"type":"response.completed","response":response_base(&response_id, "completed", vec![call], json!({"input_tokens":7,"input_tokens_details":{"cached_tokens":0},"output_tokens":3,"output_tokens_details":{"reasoning_tokens":0},"total_tokens":10}))}),
            ),
        ];
        (response_id, events)
    } else {
        let item = json!({
            "id":"msg_fixture", "type":"message", "status":"completed",
            "role":"assistant", "content":[{"type":"output_text","text":"fixture complete","annotations":[]}]
        });
        let events = vec![
            (
                "response.created".into(),
                json!({"type":"response.created","response":response_base(&response_id, "in_progress", vec![], Value::Null)}),
            ),
            (
                "response.output_item.added".into(),
                json!({"type":"response.output_item.added","output_index":0,"item":{"id":"msg_fixture","type":"message","status":"in_progress","role":"assistant","content":[]}}),
            ),
            (
                "response.output_item.done".into(),
                json!({"type":"response.output_item.done","output_index":0,"item":item}),
            ),
            (
                "response.completed".into(),
                json!({"type":"response.completed","response":response_base(&response_id, "completed", vec![item], json!({"input_tokens":7,"input_tokens_details":{"cached_tokens":0},"output_tokens":3,"output_tokens_details":{"reasoning_tokens":0},"total_tokens":10}))}),
            ),
        ];
        (response_id, events)
    }
}

fn response(response_id: &str, events: &[(String, Value)]) -> RecordedResponse {
    RecordedResponse {
        status: 200,
        headers: vec![Header {
            name: "content-type".into(),
            value: "text/event-stream".into(),
        }],
        body: ResponseBody::Events {
            events: events
                .iter()
                .enumerate()
                .map(|(index, (event_type, payload))| CassetteEvent {
                    sequence: u32::try_from(index).unwrap(),
                    monotonic_offset_ns: u64::try_from(index).unwrap() * 1_000_000,
                    event_type: event_type.clone(),
                    payload: payload.clone(),
                    payload_sha256: String::new(),
                    response_id: (index == 0).then(|| response_id.to_owned()),
                    previous_response_id: (index > 0).then(|| response_id.to_owned()),
                    tool_call_id: payload
                        .get("item")
                        .and_then(|item| item.get("call_id"))
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    terminal: (index + 1 == events.len()).then_some(TerminalEvent::Completed),
                })
                .collect(),
            // This in-memory capture owns the semantic SSE writes but does not
            // observe how the kernel segments bytes for the client.
            transport_chunk_bytes: Vec::new(),
        },
    }
}

fn retry_response() -> (RecordedResponse, Vec<(String, Value)>) {
    let events = vec![(
        "response.failed".into(),
        json!({
            "type": "response.failed",
            "error": {"message": "synthetic transient failure", "type": "server_error"}
        }),
    )];
    let response = RecordedResponse {
        status: 500,
        headers: vec![Header {
            name: "content-type".into(),
            value: "text/event-stream".into(),
        }],
        body: ResponseBody::Events {
            events: vec![CassetteEvent {
                sequence: 0,
                monotonic_offset_ns: 0,
                event_type: "response.failed".into(),
                payload: events[0].1.clone(),
                payload_sha256: String::new(),
                response_id: None,
                previous_response_id: None,
                tool_call_id: None,
                terminal: Some(TerminalEvent::Failed),
            }],
            transport_chunk_bytes: Vec::new(),
        },
    };
    (response, events)
}

fn write_sse(stream: &mut TcpStream, status: u16, events: &[(String, Value)]) {
    let mut body = Vec::new();
    for (event_type, payload) in events {
        body.extend_from_slice(b"event: ");
        body.extend_from_slice(event_type.as_bytes());
        body.push(b'\n');
        body.extend_from_slice(b"data: ");
        body.extend_from_slice(serde_json::to_string(payload).unwrap().as_bytes());
        body.extend_from_slice(b"\n\n");
    }
    write!(
        stream,
        "HTTP/1.1 {status} Synthetic\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    )
    .unwrap();
    stream.write_all(&body).unwrap();
    stream.flush().unwrap();
}

fn capture_server(
    listener: TcpListener,
    stop: Arc<AtomicBool>,
    captured: Arc<Mutex<Vec<Exchange>>>,
) -> thread::JoinHandle<()> {
    listener.set_nonblocking(true).unwrap();
    thread::spawn(move || {
        let mut retry_injected = false;
        while !stop.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((mut stream, peer)) => {
                    assert!(peer.ip().is_loopback());
                    let request = read_request(&mut stream);
                    if !retry_injected {
                        retry_injected = true;
                        let (response, events) = retry_response();
                        captured
                            .lock()
                            .unwrap()
                            .push(Exchange { request, response });
                        write_sse(&mut stream, 500, &events);
                        continue;
                    }
                    let ordinal = captured.lock().unwrap().len();
                    let (response_id, events) = events(ordinal, &request);
                    let response = response(&response_id, &events);
                    captured
                        .lock()
                        .unwrap()
                        .push(Exchange { request, response });
                    write_sse(&mut stream, 200, &events);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(2));
                }
                Err(error) => panic!("capture listener failed: {error}"),
            }
        }
    })
}

fn cassette(exchanges: &[Exchange]) -> Cassette {
    let mut policy = RedactionPolicy::default();
    for name in [
        "session-id",
        "thread-id",
        "x-client-request-id",
        "x-codex-turn-metadata",
        "x-codex-window-id",
    ] {
        policy.header_names.insert(name.into());
    }
    for pointer in [
        "/client_metadata/root_turn_id",
        "/client_metadata/session_id",
        "/client_metadata/thread_id",
        "/client_metadata/turn_id",
        "/client_metadata/x-codex-installation-id",
        "/client_metadata/x-codex-turn-metadata",
        "/client_metadata/x-codex-window-id",
        "/input",
        "/prompt_cache_key",
    ] {
        policy.request_body_pointers.insert(pointer.into());
    }
    let contents = CassetteContents {
        schema_version: 1,
        cassette_id: "codex-real-loopback-v1".into(),
        normalization: PolicyVersion { version: 1 },
        redaction: policy.descriptor().unwrap(),
        interactions: exchanges
            .iter()
            .enumerate()
            .map(|(ordinal, exchange)| Interaction {
                session_id: SESSION.into(),
                attempt_id: ATTEMPT.into(),
                interaction_id: format!("codex-{ordinal}"),
                ordinal: u32::try_from(ordinal).unwrap(),
                dialect: ProviderDialect::OpenaiResponses,
                request: exchange.request.clone(),
                response: exchange.response.clone(),
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
            .windows(b"Bearer asb-credential-free-fixture".len())
            .any(|window| { window == b"Bearer asb-credential-free-fixture" })
    );
    let cassette = decode_cassette(&bytes, CassetteLimits::default()).unwrap();
    for interaction in &cassette.contents.interactions {
        for name in [
            "authorization",
            "session-id",
            "thread-id",
            "x-client-request-id",
            "x-codex-turn-metadata",
            "x-codex-window-id",
        ] {
            assert!(
                interaction.request.headers.iter().any(|header| {
                    header.name == name && header.value.starts_with("[ASB_REDACTED:")
                }),
                "request header was not redacted: {name}"
            );
        }
        for pointer in [
            "/input",
            "/client_metadata/root_turn_id",
            "/client_metadata/session_id",
            "/client_metadata/thread_id",
            "/client_metadata/turn_id",
            "/client_metadata/x-codex-installation-id",
            "/client_metadata/x-codex-turn-metadata",
            "/client_metadata/x-codex-window-id",
            "/prompt_cache_key",
        ] {
            assert!(
                interaction
                    .request
                    .body
                    .pointer(pointer)
                    .is_some_and(|value| {
                        value
                            .as_str()
                            .is_some_and(|text| text.starts_with("[ASB_REDACTED:"))
                    }),
                "request pointer was not redacted: {pointer}"
            );
        }
    }
    cassette
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
            dialect: ProviderDialect::OpenaiResponses,
        };
        let deadline = Instant::now() + Duration::from_secs(40);
        let mut statuses = Vec::new();
        while statuses.len() < count && Instant::now() < deadline {
            match service.serve_once(&listener, &route) {
                Ok(status) => statuses.push(status),
                Err(ReplayError::Io(error)) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(2));
                }
                Err(error) => panic!("privacy-safe replay service failure: {error}"),
            }
        }
        statuses
    })
}

fn assert_trajectory_parity(recorded: &[ExtensionEvent], replayed: &[ExtensionEvent]) {
    assert_eq!(recorded.len(), replayed.len());
    for (recorded, replayed) in recorded.iter().zip(replayed) {
        assert_eq!(recorded.session_id, replayed.session_id);
        assert_eq!(recorded.attempt_id, replayed.attempt_id);
        assert_eq!(recorded.sequence, replayed.sequence);
        match (&recorded.event, &replayed.event) {
            (
                Event::ToolStarted { name: recorded, .. },
                Event::ToolStarted { name: replayed, .. },
            ) => assert_eq!(recorded, replayed),
            (
                Event::ToolFinished {
                    success: recorded, ..
                },
                Event::ToolFinished {
                    success: replayed, ..
                },
            ) => assert_eq!(recorded, replayed),
            (recorded, replayed) => assert_eq!(recorded, replayed),
        }
    }
    for events in [recorded, replayed] {
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

fn assert_loopback_only_namespace() {
    assert_eq!(
        std::env::var("ASB_REQUIRE_LOOPBACK_ONLY").as_deref(),
        Ok("1")
    );
    let interfaces = fs::read_to_string("/proc/net/dev").unwrap();
    let names = interfaces
        .lines()
        .skip(2)
        .map(|line| line.split_once(':').unwrap().0.trim())
        .collect::<Vec<_>>();
    assert_eq!(names, ["lo"]);
}

fn changed_paths(left: &Value, right: &Value, path: &str, output: &mut Vec<String>) {
    match (left, right) {
        (Value::Object(left), Value::Object(right)) => {
            let keys = left
                .keys()
                .chain(right.keys())
                .collect::<std::collections::BTreeSet<_>>();
            for key in keys {
                let child = format!("{path}/{key}");
                match (left.get(key), right.get(key)) {
                    (Some(left), Some(right)) => changed_paths(left, right, &child, output),
                    _ => output.push(child),
                }
            }
        }
        (Value::Array(left), Value::Array(right)) => {
            if left.len() != right.len() {
                output.push(format!("{path}/length"));
            }
            for (index, (left, right)) in left.iter().zip(right).enumerate() {
                changed_paths(left, right, &format!("{path}/{index}"), output);
            }
        }
        _ if left != right => output.push(path.to_owned()),
        _ => {}
    }
}

fn stable_headers(headers: &[Header]) -> Vec<Header> {
    headers
        .iter()
        .map(|header| Header {
            name: header.name.clone(),
            value: if matches!(
                header.name.as_str(),
                "session-id"
                    | "thread-id"
                    | "x-client-request-id"
                    | "x-codex-turn-metadata"
                    | "x-codex-window-id"
            ) {
                "[volatile-correlation-id]".into()
            } else {
                header.value.clone()
            },
        })
        .collect()
}

fn stable_body(body: &Value) -> Value {
    let mut stable = body.clone();
    for pointer in [
        "/client_metadata/root_turn_id",
        "/client_metadata/session_id",
        "/client_metadata/thread_id",
        "/client_metadata/turn_id",
        "/client_metadata/x-codex-installation-id",
        "/client_metadata/x-codex-turn-metadata",
        "/client_metadata/x-codex-window-id",
        "/input",
        "/prompt_cache_key",
    ] {
        if let Some(value) = stable.pointer_mut(pointer) {
            *value = Value::String("[volatile-or-private]".into());
        }
    }
    stable
}

fn assert_tool_chain(exchanges: &[Exchange]) {
    assert_eq!(exchanges.len(), 3);
    assert_eq!(exchanges[0].response.status, 500);
    let first = exchanges[1].request.body["input"].as_array().unwrap();
    assert!(
        !first
            .iter()
            .any(|item| item["type"] == "function_call_output")
    );
    let second = exchanges[2].request.body["input"].as_array().unwrap();
    let output = second
        .iter()
        .find(|item| item["type"] == "function_call_output")
        .unwrap();
    assert_eq!(output["call_id"], "call_fixture");
    assert!(
        output["output"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    );
}

#[test]
fn malformed_cassette_is_rejected_before_service_start() {
    let valid = include_bytes!("../../asb-replay/fixtures/v1/events.json");
    let mut value: Value = serde_json::from_slice(valid).unwrap();
    value["contents"]["interactions"][0]["private_raw_response"] = json!("must-not-pass");
    let malformed = serde_json::to_vec(&value).unwrap();
    assert!(decode_cassette(&malformed, CassetteLimits::default()).is_err());

    let truncated = &valid[..valid.len() / 2];
    assert!(decode_cassette(truncated, CassetteLimits::default()).is_err());
}

#[test]
#[ignore = "requires pinned Codex 0.153.4 and a loopback-only network namespace"]
fn pinned_codex_records_and_replays_the_same_graded_trajectory() {
    assert_eq!(std::env::consts::ARCH, "x86_64");
    assert_loopback_only_namespace();
    let binary = PathBuf::from(std::env::var_os("ASB_CODEX_BIN").expect("binary path"));
    assert_eq!(
        std::env::var("ASB_CODEX_SHA256").unwrap(),
        TESTED_LINUX_X86_64_SHA256
    );
    let scratch = Scratch::new("journey");
    let prepared =
        OriginalWorkloads::prepare("original.bug-fix", scratch.0.join("workload")).unwrap();
    let capture_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let capture_endpoint = Url::parse(&format!(
        "http://{}/v1",
        capture_listener.local_addr().unwrap()
    ))
    .unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let exchanges = Arc::new(Mutex::new(Vec::new()));
    let capture = capture_server(capture_listener, Arc::clone(&stop), Arc::clone(&exchanges));
    let mut recorded = adapter(
        &binary,
        &prepared.workspace(),
        &scratch.0.join("record-state"),
        capture_endpoint,
    )
    .start(
        Id(SESSION.into()),
        Id(ATTEMPT.into()),
        prepared.prompt(),
        limits(Duration::from_secs(30)),
    )
    .unwrap();
    let recorded = recorded.wait().unwrap();
    stop.store(true, Ordering::Release);
    capture.join().unwrap();
    assert_eq!(recorded.status(), TerminalStatus::Completed);
    let recorded_grade = prepared.evaluate().unwrap();
    assert!(recorded_grade.passed());
    let exchanges = exchanges.lock().unwrap().clone();
    assert_tool_chain(&exchanges);
    let cassette = cassette(&exchanges);

    prepared.reset().unwrap();
    let comparison_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let comparison_endpoint = Url::parse(&format!(
        "http://{}/v1",
        comparison_listener.local_addr().unwrap()
    ))
    .unwrap();
    let comparison_stop = Arc::new(AtomicBool::new(false));
    let comparison_exchanges = Arc::new(Mutex::new(Vec::new()));
    let comparison_server = capture_server(
        comparison_listener,
        Arc::clone(&comparison_stop),
        Arc::clone(&comparison_exchanges),
    );
    let mut comparison = adapter(
        &binary,
        &prepared.workspace(),
        &scratch.0.join("comparison-state"),
        comparison_endpoint,
    )
    .start(
        Id(SESSION.into()),
        Id(ATTEMPT.into()),
        prepared.prompt(),
        limits(Duration::from_secs(30)),
    )
    .unwrap();
    assert_eq!(
        comparison.wait().unwrap().status(),
        TerminalStatus::Completed
    );
    comparison_stop.store(true, Ordering::Release);
    comparison_server.join().unwrap();
    let comparison_exchanges = comparison_exchanges.lock().unwrap();
    assert_tool_chain(&comparison_exchanges);
    assert_eq!(exchanges.len(), comparison_exchanges.len());
    for (index, (left, right)) in exchanges
        .iter()
        .zip(comparison_exchanges.iter())
        .enumerate()
    {
        let mut differences = Vec::new();
        changed_paths(
            &stable_body(&left.request.body),
            &stable_body(&right.request.body),
            "",
            &mut differences,
        );
        assert_eq!(
            stable_headers(&left.request.headers),
            stable_headers(&right.request.headers),
            "headers differ at request {index}"
        );
        assert!(
            differences.is_empty(),
            "request {index} changed at {differences:?}"
        );
    }

    prepared.reset().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = Url::parse(&format!("http://{}/v1", listener.local_addr().unwrap())).unwrap();
    let cancellation_cassette = cassette.clone();
    let service = Arc::new(StrictReplayService::new(cassette, ReplayLimits::default()).unwrap());
    let replay_server = serve(Arc::clone(&service), listener, exchanges.len());
    let mut replayed = adapter(
        &binary,
        &prepared.workspace(),
        &scratch.0.join("replay-state"),
        endpoint,
    )
    .start(
        Id(SESSION.into()),
        Id(ATTEMPT.into()),
        prepared.prompt(),
        limits(Duration::from_secs(30)),
    )
    .unwrap();
    let replayed = replayed.wait().unwrap();
    let statuses = replay_server.join().unwrap();
    assert_eq!(statuses, [500, 200, 200]);
    assert_eq!(replayed.status(), TerminalStatus::Completed);
    assert_trajectory_parity(recorded.events(), replayed.events());
    assert_eq!(prepared.evaluate().unwrap(), recorded_grade);
    assert!(
        fs::read_dir(scratch.0.join("record-state"))
            .unwrap()
            .next()
            .is_none()
    );
    assert!(
        fs::read_dir(scratch.0.join("replay-state"))
            .unwrap()
            .next()
            .is_none()
    );

    prepared.reset().unwrap();
    let cancel_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let cancel_endpoint = Url::parse(&format!(
        "http://{}/v1",
        cancel_listener.local_addr().unwrap()
    ))
    .unwrap();
    let cancel_service =
        StrictReplayService::new(cancellation_cassette, ReplayLimits::default()).unwrap();
    let replay_cancellation = CancellationToken::default();
    let server_cancellation = replay_cancellation.clone();
    let cancel_server = thread::spawn(move || {
        cancel_listener.set_nonblocking(true).unwrap();
        let route = ReplayRoute {
            session_id: SESSION.into(),
            attempt_id: ATTEMPT.into(),
            dialect: ProviderDialect::OpenaiResponses,
        };
        let pacing = PacingConfig {
            mode: PacingMode::Fixed {
                time_to_first_segment: Duration::from_secs(10),
                inter_segment: Duration::ZERO,
            },
            max_segment_write: Duration::from_secs(30),
            max_lateness: Duration::from_secs(30),
            cancellation_poll: Duration::from_millis(2),
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match cancel_service.serve_once_paced(
                &cancel_listener,
                &route,
                pacing,
                &server_cancellation,
            ) {
                Err(ReplayError::Io(error))
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && Instant::now() < deadline =>
                {
                    thread::sleep(Duration::from_millis(2));
                }
                result => break result,
            }
        }
    });
    let mut cancelled = adapter(
        &binary,
        &prepared.workspace(),
        &scratch.0.join("cancel-state"),
        cancel_endpoint,
    )
    .start(
        Id(SESSION.into()),
        Id(ATTEMPT.into()),
        prepared.prompt(),
        limits(Duration::from_secs(30)),
    )
    .unwrap();
    // Codex performs local startup before its first provider request. The
    // replay response itself is held for ten seconds, so five seconds reaches
    // the paced delivery boundary without racing process initialization.
    thread::sleep(Duration::from_secs(5));
    replay_cancellation.cancel();
    cancelled.cancel().unwrap();
    assert_eq!(
        cancelled.wait().unwrap().status(),
        TerminalStatus::Cancelled
    );
    let cancel_result = cancel_server.join().unwrap();
    assert!(
        matches!(
            cancel_result,
            Err(ReplayError::Pacing(PacingError::Cancelled { .. }))
        ),
        "privacy-safe cancel service result: {cancel_result:?}"
    );
    assert!(
        fs::read_dir(scratch.0.join("cancel-state"))
            .unwrap()
            .next()
            .is_none()
    );
}
