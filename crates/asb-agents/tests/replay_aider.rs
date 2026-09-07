// SPDX-License-Identifier: MIT
#![cfg(target_os = "linux")]
#![allow(missing_docs)]

use asb_agents::aider::{
    AiderArtifact, AiderConfig, RetryObservation, RetryUnavailableReason, WHEEL_SHA256,
};
use asb_protocol::{Capability, Event, ExtensionEvent, Id, TerminalStatus};
use asb_replay::{
    CancellationToken, Cassette, CassetteContents, CassetteLimits, Header, Interaction,
    PacingConfig, PacingError, PacingMode, PolicyVersion, ProviderDialect, RecordedRequest,
    RecordedResponse, RedactionPolicy, Redactor, ReplayError, ReplayLimits, ReplayRoute,
    ResponseBody, StrictReplayService, TerminalEvent, decode_cassette, seal_cassette,
};
use asb_runtime::ProcessLimits;
use asb_workloads::OriginalWorkloads;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use url::Url;

const SESSION: &str = "aider-replay-session";
const ATTEMPT: &str = "aider-replay-attempt";

struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        let parent = PathBuf::from(std::env::var_os("ASB_TEST_ROOT").expect("ASB_TEST_ROOT"));
        assert!(parent.is_absolute());
        assert!(parent.starts_with("/srv/data/projects/"));
        fs::create_dir_all(&parent).unwrap();
        let path = parent.join(format!(
            "aider-replay-{label}-{}-{}",
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
        Duration::from_millis(300),
        Duration::from_millis(5),
    )
    .unwrap()
}

fn adapter(
    python: &Path,
    wheel: &Path,
    workspace: &Path,
    state: &Path,
    endpoint: Url,
) -> AiderConfig {
    AiderConfig::new(
        python,
        wheel,
        workspace,
        state,
        endpoint,
        "gpt-4o-mini",
        AiderArtifact::LinuxX86_64V0_86_2,
    )
    .unwrap()
}

fn read_request(stream: &mut TcpStream) -> RecordedRequest {
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
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
        Some("gpt-4o-mini")
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
        model: "gpt-4o-mini".into(),
        options,
        tools,
        previous_response_id: None,
    }
}

fn failure_payload() -> Value {
    json!({"error": {"message": "synthetic transient failure", "type": "server_error"}})
}

fn success_payload() -> Value {
    json!({
        "id": "asb-aider-fixture",
        "object": "chat.completion",
        "created": 1,
        "model": "gpt-4o-mini",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "parser.py\n\u{0060}\u{0060}\u{0060}python\ndef parse_line(line):\n    if line.endswith(\"\\r\"):\n        line = line[:-1]\n    return line\n\u{0060}\u{0060}\u{0060}"
            },
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 10, "completion_tokens": 10, "total_tokens": 20}
    })
}

fn response(status: u16, payload: Value, terminal: TerminalEvent) -> RecordedResponse {
    let response_id = payload.get("id").and_then(Value::as_str).map(str::to_owned);
    RecordedResponse {
        status,
        headers: vec![Header {
            name: "content-type".into(),
            value: "application/json".into(),
        }],
        body: ResponseBody::Buffered {
            payload,
            payload_sha256: String::new(),
            response_id,
            terminal,
        },
    }
}

fn write_json(stream: &mut TcpStream, status: u16, payload: &Value) {
    let body = serde_json::to_vec(payload).unwrap();
    write!(
        stream,
        "HTTP/1.1 {status} Synthetic\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
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
        while !stop.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((mut stream, peer)) => {
                    assert!(peer.ip().is_loopback());
                    let request = read_request(&mut stream);
                    let first = captured.lock().unwrap().is_empty();
                    let (status, payload, terminal) = if first {
                        (500, failure_payload(), TerminalEvent::Failed)
                    } else {
                        (200, success_payload(), TerminalEvent::Completed)
                    };
                    let response = response(status, payload.clone(), terminal);
                    captured
                        .lock()
                        .unwrap()
                        .push(Exchange { request, response });
                    write_json(&mut stream, status, &payload);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(2));
                }
                Err(error) => panic!("privacy-safe capture listener failed: {error}"),
            }
        }
    })
}

fn cassette(exchanges: &[Exchange]) -> Cassette {
    let policy = RedactionPolicy::default();
    let contents = CassetteContents {
        schema_version: 1,
        cassette_id: "aider-real-loopback-v1".into(),
        normalization: PolicyVersion { version: 1 },
        redaction: policy.descriptor().unwrap(),
        interactions: exchanges
            .iter()
            .enumerate()
            .map(|(ordinal, exchange)| Interaction {
                session_id: SESSION.into(),
                attempt_id: ATTEMPT.into(),
                interaction_id: format!("aider-{ordinal}"),
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
            .any(|window| window == b"Bearer asb-credential-free")
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
    assert_eq!(recorded, replayed);
    assert!(
        recorded
            .iter()
            .any(|event| matches!(&event.event, Event::Completed))
    );
    assert!(recorded.iter().all(|event| !matches!(
        &event.event,
        Event::ToolStarted { .. } | Event::ToolFinished { .. } | Event::Usage(_)
    )));
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

#[test]
fn malformed_and_tool_inconsistent_cassettes_fail_before_service_start() {
    let valid = include_bytes!("../../asb-replay/fixtures/v1/buffered.json");
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

    let policy = RedactionPolicy::default();
    let body = json!({"model": "gpt-4o-mini", "messages": []});
    let inconsistent = CassetteContents {
        schema_version: 1,
        cassette_id: "aider-tool-mismatch".into(),
        normalization: PolicyVersion { version: 1 },
        redaction: policy.descriptor().unwrap(),
        interactions: vec![Interaction {
            session_id: SESSION.into(),
            attempt_id: ATTEMPT.into(),
            interaction_id: "aider-tool-mismatch-0".into(),
            ordinal: 0,
            dialect: ProviderDialect::OpenaiChatCompletions,
            request: RecordedRequest {
                method: "POST".into(),
                path: "/v1/chat/completions".into(),
                headers: vec![],
                body,
                body_sha256: String::new(),
                model: "gpt-4o-mini".into(),
                options: BTreeMap::new(),
                tools: vec![json!({"type": "function", "function": {"name": "not-supported"}})],
                previous_response_id: None,
            },
            response: response(200, success_payload(), TerminalEvent::Completed),
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
#[ignore = "requires pinned aider 0.86.2, CPython 3.12, and a loopback-only network namespace"]
fn pinned_aider_records_and_replays_the_same_graded_trajectory() {
    assert_eq!(std::env::consts::ARCH, "x86_64");
    assert_loopback_only_namespace();
    let python = PathBuf::from(std::env::var_os("ASB_AIDER_PYTHON").expect("Python path"));
    let wheel = PathBuf::from(std::env::var_os("ASB_AIDER_WHEEL").expect("wheel path"));
    assert_eq!(
        std::env::var("ASB_AIDER_WHEEL_SHA256").unwrap(),
        WHEEL_SHA256
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
        &python,
        &wheel,
        prepared.workspace(),
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
    assert_eq!(
        recorded.retry_observation(),
        RetryObservation::Unavailable {
            reason: RetryUnavailableReason::UnstructuredBatchDiagnostics,
        }
    );
    assert_eq!(
        adapter(
            &python,
            &wheel,
            prepared.workspace(),
            &scratch.0.join("manifest-state"),
            Url::parse("http://127.0.0.1:1/v1").unwrap(),
        )
        .manifest()
        .capabilities,
        BTreeSet::from([Capability::Cancellation])
    );
    let recorded_grade = prepared.evaluate().unwrap();
    assert!(recorded_grade.passed());
    let exchanges = exchanges.lock().unwrap().clone();
    assert_eq!(exchanges.len(), 2);
    assert_eq!(exchanges[0].response.status, 500);
    assert_eq!(exchanges[1].response.status, 200);
    for exchange in &exchanges {
        assert!(exchange.request.tools.is_empty());
        assert!(
            exchange
                .request
                .body
                .get("tools")
                .is_none_or(|tools| tools.as_array().is_some_and(Vec::is_empty))
        );
    }
    let replay_cassette = cassette(&exchanges);
    let cancellation_cassette = replay_cassette.clone();

    prepared.reset().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = Url::parse(&format!("http://{}/v1", listener.local_addr().unwrap())).unwrap();
    let service =
        Arc::new(StrictReplayService::new(replay_cassette, ReplayLimits::default()).unwrap());
    let replay_server = serve(Arc::clone(&service), listener, exchanges.len());
    let mut replayed = adapter(
        &python,
        &wheel,
        prepared.workspace(),
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
    assert_eq!(replay_server.join().unwrap(), [500, 200]);
    assert_eq!(replayed.status(), TerminalStatus::Completed);
    assert_eq!(replayed.retry_observation(), recorded.retry_observation());
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
        &python,
        &wheel,
        prepared.workspace(),
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
