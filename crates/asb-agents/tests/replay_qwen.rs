// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
#![cfg(target_os = "linux")]
#![allow(missing_docs)]

use asb_agents::qwen_code::{LINUX_X86_64_ARCHIVE_SHA256, QwenCodeArtifact, QwenCodeConfig};
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

const SESSION: &str = "qwen-replay-session";
const ATTEMPT: &str = "qwen-replay-attempt";

struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        let parent = PathBuf::from(std::env::var_os("ASB_TEST_ROOT").expect("ASB_TEST_ROOT"));
        assert!(parent.is_absolute());
        assert!(parent.starts_with("/srv/data/projects/"));
        fs::create_dir_all(&parent).unwrap();
        let path = parent.join(format!(
            "qwen-replay-{label}-{}-{}",
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

fn adapter(
    binary: &Path,
    archive: &Path,
    workspace: &Path,
    state: &Path,
    endpoint: Url,
) -> QwenCodeConfig {
    QwenCodeConfig::new(
        binary,
        archive,
        workspace,
        state,
        endpoint,
        "fixture-model",
        QwenCodeArtifact::LinuxX86_64V0_23_0,
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
    assert_eq!(path, "/v1/chat/completions");
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
        Some("Bearer asb-credential-free")
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
        .filter(|(name, _)| !matches!(name.as_str(), "messages" | "model" | "tools"))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect::<BTreeMap<_, _>>();
    let tools = object
        .get("tools")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    RecordedRequest {
        method: "POST".into(),
        path,
        headers,
        body,
        body_sha256: String::new(),
        model: "fixture-model".into(),
        options,
        tools,
        previous_response_id: None,
    }
}

fn chunks(ordinal: usize, request: &RecordedRequest, output: &Path) -> Vec<Value> {
    let has_tools = !request.tools.is_empty();
    let tool_results = request.body["messages"].as_array().map_or(0, |messages| {
        messages
            .iter()
            .filter(|message| message["role"] == "tool")
            .count()
    });
    let id = format!("fixture-{ordinal}");
    if has_tools && tool_results < 2 {
        let (name, arguments) = if tool_results == 0 {
            ("read_file", json!({"file_path": output}))
        } else {
            (
                "write_file",
                json!({
                    "file_path": output,
                    "content": "def parse_line(line):\n    if line.endswith(\"\\r\"):\n        line = line[:-1]\n    return line\n"
                }),
            )
        };
        vec![
            json!({
                "id": id, "object": "chat.completion.chunk", "created": 1,
                "model": "fixture-model", "choices": [{"index": 0, "delta": {
                    "role": "assistant", "tool_calls": [{"index": 0,
                    "id": "call_asb_fix", "type": "function", "function": {
                        "name": name, "arguments": arguments.to_string()
                    }}]}, "finish_reason": null}]
            }),
            json!({
                "id": id, "object": "chat.completion.chunk", "created": 1,
                "model": "fixture-model",
                "choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]
            }),
        ]
    } else {
        vec![
            json!({
                "id": id, "object": "chat.completion.chunk", "created": 1,
                "model": "fixture-model",
                "choices": [{"index": 0, "delta": {"role": "assistant", "content": "done"},
                "finish_reason": null}]
            }),
            json!({
                "id": id, "object": "chat.completion.chunk", "created": 1,
                "model": "fixture-model",
                "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
                "usage": {"prompt_tokens": 7, "completion_tokens": 2, "total_tokens": 9}
            }),
        ]
    }
}

fn response(chunks: &[Value]) -> RecordedResponse {
    RecordedResponse {
        status: 200,
        headers: vec![Header {
            name: "content-type".into(),
            value: "text/event-stream".into(),
        }],
        body: ResponseBody::Events {
            events: chunks
                .iter()
                .enumerate()
                .map(|(index, payload)| CassetteEvent {
                    sequence: u32::try_from(index).unwrap(),
                    monotonic_offset_ns: u64::try_from(index).unwrap() * 1_000_000,
                    event_type: "chat.completion.chunk".into(),
                    payload: payload.clone(),
                    payload_sha256: String::new(),
                    response_id: (index == 0).then(|| payload["id"].as_str().unwrap().to_owned()),
                    previous_response_id: None,
                    tool_call_id: None,
                    terminal: (index + 1 == chunks.len()).then_some(TerminalEvent::Completed),
                })
                .collect(),
            // This in-memory capture owns the semantic SSE writes but does not
            // observe how the kernel segments bytes for the client.
            transport_chunk_bytes: Vec::new(),
        },
    }
}

fn retry_response() -> (RecordedResponse, Vec<Value>) {
    let chunks = vec![json!({
        "id": "fixture-retry",
        "error": {"message": "synthetic transient failure", "type": "server_error"}
    })];
    let mut response = response(&chunks);
    response.status = 429;
    if let ResponseBody::Events { events, .. } = &mut response.body {
        events[0].response_id = None;
        events[0].terminal = Some(TerminalEvent::Failed);
    }
    (response, chunks)
}

fn write_sse(stream: &mut TcpStream, status: u16, chunks: &[Value]) {
    let mut body = Vec::new();
    for chunk in chunks {
        body.extend_from_slice(b"data: ");
        body.extend_from_slice(serde_json::to_string(chunk).unwrap().as_bytes());
        body.extend_from_slice(b"\n\n");
    }
    body.extend_from_slice(b"data: [DONE]\n\n");
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
    output: PathBuf,
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
                    let ordinal = captured.lock().unwrap().len();
                    let has_tools = !request.tools.is_empty();
                    let has_tool_result =
                        request.body["messages"].as_array().is_some_and(|messages| {
                            messages.iter().any(|message| message["role"] == "tool")
                        });
                    if has_tools && !has_tool_result && !retry_injected {
                        retry_injected = true;
                        let (response, retry_chunks) = retry_response();
                        captured
                            .lock()
                            .unwrap()
                            .push(Exchange { request, response });
                        write_sse(&mut stream, 429, &retry_chunks);
                    } else {
                        let chunks = chunks(ordinal, &request, &output);
                        let response = response(&chunks);
                        captured
                            .lock()
                            .unwrap()
                            .push(Exchange { request, response });
                        write_sse(&mut stream, 200, &chunks);
                    }
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
    policy.header_names.insert("authorization".into());
    let contents = CassetteContents {
        schema_version: 1,
        cassette_id: "qwen-real-loopback-v1".into(),
        normalization: PolicyVersion { version: 1 },
        redaction: policy.descriptor().unwrap(),
        interactions: exchanges
            .iter()
            .enumerate()
            .map(|(ordinal, exchange)| Interaction {
                session_id: SESSION.into(),
                attempt_id: ATTEMPT.into(),
                interaction_id: format!("qwen-{ordinal}"),
                ordinal: u32::try_from(ordinal).unwrap(),
                dialect: ProviderDialect::OpenaiChatCompletions,
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
            .windows(b"Bearer asb-credential-free".len())
            .any(|window| { window == b"Bearer asb-credential-free" })
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
            dialect: ProviderDialect::OpenaiChatCompletions,
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

#[test]
fn malformed_and_tool_inconsistent_cassettes_are_rejected_before_service_start() {
    let valid = include_bytes!("../../asb-replay/fixtures/v1/events.json");
    let mut value: Value = serde_json::from_slice(valid).unwrap();
    value["contents"]["interactions"][0]["private_raw_response"] = json!("must-not-pass");
    let malformed = serde_json::to_vec(&value).unwrap();
    assert!(decode_cassette(&malformed, CassetteLimits::default()).is_err());

    let truncated = &valid[..valid.len() / 2];
    assert!(decode_cassette(truncated, CassetteLimits::default()).is_err());

    let policy = RedactionPolicy::default();
    let inconsistent = CassetteContents {
        schema_version: 1,
        cassette_id: "qwen-tool-mismatch".into(),
        normalization: PolicyVersion { version: 1 },
        redaction: policy.descriptor().unwrap(),
        interactions: vec![Interaction {
            session_id: SESSION.into(),
            attempt_id: ATTEMPT.into(),
            interaction_id: "qwen-tool-mismatch-0".into(),
            ordinal: 0,
            dialect: ProviderDialect::OpenaiChatCompletions,
            request: RecordedRequest {
                method: "POST".into(),
                path: "/v1/chat/completions".into(),
                headers: vec![],
                body: json!({"model": "fixture-model", "messages": [], "tools": []}),
                body_sha256: String::new(),
                model: "fixture-model".into(),
                options: BTreeMap::new(),
                tools: vec![json!({
                    "type": "function",
                    "function": {
                        "name": "write_file",
                        "parameters": {"type": "object"}
                    }
                })],
                previous_response_id: None,
            },
            response: response(&[json!({
                "id": "fixture-completed",
                "object": "chat.completion.chunk",
                "created": 1,
                "model": "fixture-model",
                "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]
            })]),
        }],
    };
    let bytes = seal_cassette(
        Redactor::new(policy)
            .unwrap()
            .redact_contents(inconsistent)
            .unwrap()
            .0,
        CassetteLimits::default(),
    )
    .unwrap();
    let inconsistent = decode_cassette(&bytes, CassetteLimits::default()).unwrap();
    assert!(matches!(
        StrictReplayService::new(inconsistent, ReplayLimits::default()),
        Err(ReplayError::InvalidCassette)
    ));
}

#[test]
#[ignore = "requires pinned Qwen Code 0.23.0 and a loopback-only network namespace"]
fn pinned_qwen_records_and_replays_the_same_graded_trajectory() {
    assert_eq!(std::env::consts::ARCH, "x86_64");
    assert_loopback_only_namespace();
    let binary = PathBuf::from(std::env::var_os("ASB_QWEN_EXECUTABLE").expect("binary path"));
    let archive = PathBuf::from(std::env::var_os("ASB_QWEN_ARCHIVE").expect("archive path"));
    assert_eq!(
        std::env::var("ASB_QWEN_ARCHIVE_SHA256").unwrap(),
        LINUX_X86_64_ARCHIVE_SHA256
    );
    let scratch = Scratch::new("journey");
    let prepared =
        OriginalWorkloads::prepare("original.bug-fix", scratch.0.join("workload")).unwrap();
    let output = prepared.workspace().join("parser.py");

    let capture_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let capture_endpoint = Url::parse(&format!(
        "http://{}/v1",
        capture_listener.local_addr().unwrap()
    ))
    .unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let exchanges = Arc::new(Mutex::new(Vec::new()));
    let capture = capture_server(
        capture_listener,
        output,
        Arc::clone(&stop),
        Arc::clone(&exchanges),
    );
    let mut recorded = adapter(
        &binary,
        &archive,
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
    let exchanges = exchanges.lock().unwrap().clone();
    let statuses = exchanges
        .iter()
        .map(|exchange| exchange.response.status)
        .collect::<Vec<_>>();
    let requested_tool_names = exchanges
        .iter()
        .flat_map(|exchange| exchange.request.tools.iter())
        .filter_map(|tool| tool.pointer("/function/name").and_then(Value::as_str))
        .collect::<std::collections::BTreeSet<_>>();
    let recorded_grade = prepared.evaluate().unwrap();
    assert!(
        recorded_grade.passed(),
        "privacy-safe capture did not grade: requests={}, statuses={statuses:?}, requested_tools={requested_tool_names:?}, events={}",
        exchanges.len(),
        recorded.events().len()
    );
    assert_eq!(statuses, [429, 200, 200, 200]);
    assert_eq!(recorded.events().len(), 9);
    assert!(matches!(
        &recorded.events()[3].event,
        Event::ToolStarted { name, .. } if name == "read_file"
    ));
    assert!(matches!(
        recorded.events()[4].event,
        Event::ToolFinished { success: true, .. }
    ));
    assert!(matches!(
        &recorded.events()[5].event,
        Event::ToolStarted { name, .. } if name == "write_file"
    ));
    assert!(matches!(
        recorded.events()[6].event,
        Event::ToolFinished { success: true, .. }
    ));
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
        prepared.workspace().join("parser.py"),
        Arc::clone(&comparison_stop),
        Arc::clone(&comparison_exchanges),
    );
    let mut comparison = adapter(
        &binary,
        &archive,
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
    assert_eq!(exchanges.len(), comparison_exchanges.len());
    for (index, (left, right)) in exchanges
        .iter()
        .zip(comparison_exchanges.iter())
        .enumerate()
    {
        let mut differences = Vec::new();
        changed_paths(
            &left.request.body,
            &right.request.body,
            "",
            &mut differences,
        );
        assert_eq!(
            left.request.headers, right.request.headers,
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
        &archive,
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
    assert!(
        statuses.iter().filter(|status| **status == 429).count() == 1
            && statuses.iter().filter(|status| **status == 200).count() + 1 == statuses.len(),
        "privacy-safe replay status classes did not reproduce one retry"
    );
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
            dialect: ProviderDialect::OpenaiChatCompletions,
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
        &archive,
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
    // Qwen Code performs local startup before its first provider request. The
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
