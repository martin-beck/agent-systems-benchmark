// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
#![cfg(target_os = "linux")]
#![allow(missing_docs)]

//! Disposable synthetic OpenRouter-compatible loopback service.
//!
//! These tests never contact a real provider. A synthetic HTTP/1.1 service
//! verifies each observed request against the pinned credential-free profile and
//! retains only redacted evidence, proving model, settings, streaming, tool
//! calls, retries, cancellation, and secret non-disclosure.

use asb_agents::credential::CredentialResolutionError;
use asb_agents::openrouter::{
    OPENROUTER_MODEL, OpenRouterAgent, OpenRouterProfile, VerifiedOpenRouterRequest,
    openrouter_credential_reference, resolve_openrouter_credential,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const FIXTURE_CREDENTIAL: &str = "synthetic-openrouter-key";
const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct RedactedObservation {
    path: String,
    api_mode: String,
    profile_sha256: String,
    model: String,
    stream: bool,
    tools: usize,
    forbidden_settings_present: Vec<String>,
    authorization_digest_sha256: String,
    attempts: usize,
}

#[derive(Default)]
struct ServiceState {
    observations: Mutex<Vec<RedactedObservation>>,
    rejected: AtomicUsize,
    closed_after_request: AtomicUsize,
}

#[derive(Clone, Copy)]
enum ServiceMode {
    Serve,
    Retry { transient_failures: usize },
    Malformed,
    Hang,
}

struct SyntheticOpenRouter {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
    state: Arc<ServiceState>,
    attempts: Arc<AtomicUsize>,
}

impl SyntheticOpenRouter {
    fn start(profile: OpenRouterProfile, agent: OpenRouterAgent, mode: ServiceMode) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback bind");
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let state = Arc::new(ServiceState::default());
        let attempts = Arc::new(AtomicUsize::new(0));
        let server_stop = Arc::clone(&stop);
        let server_state = Arc::clone(&state);
        let server_attempts = Arc::clone(&attempts);
        let thread = thread::spawn(move || {
            serve_loop(
                listener,
                profile,
                agent,
                mode,
                server_stop,
                server_state,
                server_attempts,
            )
        });
        Self {
            address,
            stop,
            thread: Some(thread),
            state,
            attempts,
        }
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            thread.join().expect("synthetic service thread join");
        }
    }

    fn observations(&self) -> Vec<RedactedObservation> {
        self.state.observations.lock().unwrap().clone()
    }

    fn rejected(&self) -> usize {
        self.state.rejected.load(Ordering::SeqCst)
    }

    fn closed_after_request(&self) -> usize {
        self.state.closed_after_request.load(Ordering::SeqCst)
    }

    fn attempts(&self) -> usize {
        self.attempts.load(Ordering::SeqCst)
    }
}

impl Drop for SyntheticOpenRouter {
    fn drop(&mut self) {
        self.stop();
    }
}

struct Request {
    path: String,
    authorization: String,
    body: Vec<u8>,
}

fn read_request(stream: &mut TcpStream) -> Option<Request> {
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    let mut bytes = Vec::new();
    let head_end = loop {
        let mut chunk = [0_u8; 4096];
        let count = match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return None,
            Ok(count) => count,
        };
        bytes.extend_from_slice(&chunk[..count]);
        if bytes.len() > MAX_BODY_BYTES + 64 * 1024 {
            return None;
        }
        if let Some(position) = bytes.windows(4).position(|item| item == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let head = std::str::from_utf8(&bytes[..head_end]).ok()?;
    let mut request_line = head.lines().next()?.split_whitespace();
    let _method = request_line.next()?;
    let path = request_line.next()?.to_owned();
    let authorization = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("authorization")
                .then(|| value.trim().to_owned())
        })
        .unwrap_or_default();
    let content_length = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    if content_length > MAX_BODY_BYTES {
        return None;
    }
    let present = bytes.len() - head_end;
    bytes.resize(head_end + content_length, 0);
    let mut total = present;
    while total < content_length {
        let count = stream.read(&mut bytes[head_end + total..]).ok()?;
        if count == 0 {
            return None;
        }
        total += count;
    }
    Some(Request {
        path,
        authorization,
        body: bytes[head_end..head_end + content_length].to_vec(),
    })
}

fn observe(
    proof: &VerifiedOpenRouterRequest,
    request: &Request,
    attempt: usize,
) -> RedactedObservation {
    let object = serde_json::from_slice::<Value>(&request.body)
        .ok()
        .and_then(|value| value.as_object().cloned());
    let forbidden = [
        "temperature",
        "top_p",
        "seed",
        "max_tokens",
        "max_completion_tokens",
        "max_output_tokens",
        "reasoning_effort",
        "reasoning",
        "api_key",
        "authorization",
    ];
    RedactedObservation {
        path: request.path.clone(),
        api_mode: format!("{:?}", proof.api_mode()),
        profile_sha256: proof.profile_sha256().to_owned(),
        model: object
            .as_ref()
            .and_then(|o| o.get("model"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        stream: object
            .as_ref()
            .and_then(|o| o.get("stream"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        tools: object
            .as_ref()
            .and_then(|o| o.get("tools"))
            .and_then(Value::as_array)
            .map_or(0, Vec::len),
        forbidden_settings_present: forbidden
            .iter()
            .filter(|key| object.as_ref().is_some_and(|o| o.contains_key(**key)))
            .map(|key| (*key).to_owned())
            .collect(),
        authorization_digest_sha256: format!(
            "{:x}",
            Sha256::digest(request.authorization.as_bytes())
        ),
        attempts: attempt + 1,
    }
}

fn serve_loop(
    listener: TcpListener,
    profile: OpenRouterProfile,
    agent: OpenRouterAgent,
    mode: ServiceMode,
    stop: Arc<AtomicBool>,
    state: Arc<ServiceState>,
    attempts: Arc<AtomicUsize>,
) {
    listener.set_nonblocking(true).unwrap();
    while !stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((mut stream, _)) => {
                stream.set_nonblocking(false).unwrap();
                let Some(request) = read_request(&mut stream) else {
                    continue;
                };
                let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                match profile.verify_effective_request(
                    agent,
                    profile.provider_profile(),
                    &request.path,
                    &request.authorization,
                    &request.body,
                ) {
                    Ok(proof) => {
                        state
                            .observations
                            .lock()
                            .unwrap()
                            .push(observe(&proof, &request, attempt));
                        if matches!(mode, ServiceMode::Hang) {
                            let mut byte = [0_u8; 1];
                            let _ = stream.read(&mut byte);
                            state.closed_after_request.fetch_add(1, Ordering::SeqCst);
                        } else {
                            respond(&mut stream, &mode, attempt, &request);
                        }
                    }
                    Err(_) => {
                        state.rejected.fetch_add(1, Ordering::SeqCst);
                        let body = br#"{"error":{"message":"profile_mismatch","type":"fixture"}}"#;
                        write_response(&mut stream, 400, "Bad Request", b"application/json", body);
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(2));
            }
            Err(error) => panic!("synthetic service listener failed: {error}"),
        }
    }
}

fn respond(stream: &mut TcpStream, mode: &ServiceMode, attempt: usize, request: &Request) {
    match *mode {
        ServiceMode::Serve | ServiceMode::Hang => {
            let body = completion_body(request);
            write_response(stream, 200, "OK", b"application/json", &body);
        }
        ServiceMode::Retry { transient_failures } if attempt < transient_failures => {
            let body = br#"{"error":{"message":"temporarily unavailable","type":"fixture"}}"#;
            write_response(
                stream,
                503,
                "Service Unavailable",
                b"application/json",
                body,
            );
        }
        ServiceMode::Retry { .. } => {
            let body = completion_body(request);
            write_response(stream, 200, "OK", b"application/json", &body);
        }
        ServiceMode::Malformed => {
            write_response(stream, 200, "OK", b"application/json", b"not-json");
        }
    }
}

fn completion_body(request: &Request) -> Vec<u8> {
    let model = serde_json::from_slice::<Value>(&request.body)
        .ok()
        .and_then(|value| {
            value
                .get("model")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| OPENROUTER_MODEL.to_owned());
    serde_json::to_vec(&json!({
        "id": "fixture-completion",
        "object": "chat.completion",
        "created": 1,
        "model": model,
        "choices": [{"index": 0, "message": {"role": "assistant", "content": "fixture"},
            "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 5, "completion_tokens": 1, "total_tokens": 6}
    }))
    .expect("serialize fixture completion")
}

fn write_response(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    content_type: &[u8],
    body: &[u8],
) {
    let content_type = std::str::from_utf8(content_type).expect("content type is ASCII");
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

#[derive(Debug, Eq, PartialEq)]
enum ClientError {
    Cancelled,
    Transport,
    InvalidResponse,
    HttpStatus(u16),
}

#[derive(Debug, Eq, PartialEq)]
struct HttpResponse {
    status: u16,
    body: Vec<u8>,
}

fn chat_body() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "model": OPENROUTER_MODEL,
        "stream": true,
        "messages": [{"role": "user", "content": "asb-openrouter-fixture-prompt"}],
        "tools": [{"type": "function", "function": {
            "name": "asb_fixture_tool",
            "description": "fixture",
            "parameters": {"type": "object", "properties": {}}
        }}]
    }))
    .expect("serialize chat body")
}

fn responses_body() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "model": OPENROUTER_MODEL,
        "stream": true,
        "input": [{"role": "user", "content": [{"type": "input_text", "text": "asb-openrouter-fixture-prompt"}]}],
        "tools": [{"type": "function", "function": {
            "name": "asb_fixture_tool",
            "description": "fixture",
            "parameters": {"type": "object", "properties": {}}
        }}]
    }))
    .expect("serialize responses body")
}

fn send_request(
    address: SocketAddr,
    path: &str,
    credential: &str,
    body: &[u8],
    cancelled: &AtomicBool,
) -> Result<HttpResponse, ClientError> {
    if cancelled.load(Ordering::SeqCst) {
        return Err(ClientError::Cancelled);
    }
    let mut stream = TcpStream::connect(address).map_err(|_| ClientError::Transport)?;
    stream
        .set_read_timeout(Some(Duration::from_millis(200)))
        .map_err(|_| ClientError::Transport)?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| ClientError::Transport)?;
    let head = format!(
        "POST {path} HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {credential}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(head.as_bytes())
        .map_err(|_| ClientError::Transport)?;
    stream
        .write_all(body)
        .and_then(|_| stream.flush())
        .map_err(|_| ClientError::Transport)?;

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut raw = Vec::new();
    let mut chunk = [0_u8; 4096];
    let head_end = loop {
        if cancelled.load(Ordering::SeqCst) {
            return Err(ClientError::Cancelled);
        }
        if Instant::now() > deadline {
            return Err(ClientError::Transport);
        }
        match stream.read(&mut chunk) {
            Ok(0) => return Err(ClientError::Transport),
            Ok(count) => {
                raw.extend_from_slice(&chunk[..count]);
                if let Some(position) = raw.windows(4).position(|item| item == b"\r\n\r\n") {
                    break position + 4;
                }
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.kind() == std::io::ErrorKind::TimedOut =>
            {
                thread::sleep(Duration::from_millis(2));
            }
            Err(_) => return Err(ClientError::Transport),
        }
    };
    let head = std::str::from_utf8(&raw[..head_end]).map_err(|_| ClientError::InvalidResponse)?;
    let mut status_parts = head
        .lines()
        .next()
        .ok_or(ClientError::InvalidResponse)?
        .split_whitespace();
    let _http = status_parts.next().ok_or(ClientError::InvalidResponse)?;
    let status = status_parts
        .next()
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or(ClientError::InvalidResponse)?;
    let content_length = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    let mut body = raw[head_end..].to_vec();
    while body.len() < content_length {
        if cancelled.load(Ordering::SeqCst) {
            return Err(ClientError::Cancelled);
        }
        if Instant::now() > deadline {
            return Err(ClientError::Transport);
        }
        match stream.read(&mut chunk) {
            Ok(0) => return Err(ClientError::Transport),
            Ok(count) => body.extend_from_slice(&chunk[..count]),
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.kind() == std::io::ErrorKind::TimedOut =>
            {
                thread::sleep(Duration::from_millis(2));
            }
            Err(_) => return Err(ClientError::Transport),
        }
    }
    if (200..300).contains(&status) && serde_json::from_slice::<Value>(&body).is_err() {
        return Err(ClientError::InvalidResponse);
    }
    Ok(HttpResponse { status, body })
}

fn post_with_retries(
    address: SocketAddr,
    path: &str,
    credential: &str,
    body: &[u8],
    max_attempts: usize,
    cancelled: &Arc<AtomicBool>,
) -> Result<HttpResponse, ClientError> {
    let mut backoff = Duration::from_millis(5);
    for attempt in 1..=max_attempts {
        if cancelled.load(Ordering::SeqCst) {
            return Err(ClientError::Cancelled);
        }
        match send_request(address, path, credential, body, cancelled) {
            Ok(response) if (400..500).contains(&response.status) => {
                return Err(ClientError::HttpStatus(response.status));
            }
            Ok(response) if response.status < 400 => return Ok(response),
            Ok(response) if attempt == max_attempts => {
                return Err(ClientError::HttpStatus(response.status));
            }
            Ok(_) => {}
            Err(error) => return Err(error),
        }
        let wake = Instant::now() + backoff;
        while Instant::now() < wake {
            if cancelled.load(Ordering::SeqCst) {
                return Err(ClientError::Cancelled);
            }
            thread::sleep(Duration::from_millis(1));
        }
        backoff = backoff.saturating_mul(2).min(Duration::from_millis(80));
    }
    Err(ClientError::Transport)
}

fn profile() -> OpenRouterProfile {
    OpenRouterProfile::new(openrouter_credential_reference().unwrap()).unwrap()
}

#[test]
fn synthetic_service_proves_model_settings_streaming_tools_and_redaction() {
    let profile = profile();
    let mut service =
        SyntheticOpenRouter::start(profile.clone(), OpenRouterAgent::Aider, ServiceMode::Serve);
    let cancelled = Arc::new(AtomicBool::new(false));
    let response = post_with_retries(
        service.address,
        "/api/v1/chat/completions",
        FIXTURE_CREDENTIAL,
        &chat_body(),
        1,
        &cancelled,
    )
    .unwrap();
    assert_eq!(response.status, 200);
    assert!(
        serde_json::from_slice::<Value>(&response.body)
            .unwrap()
            .get("model")
            .and_then(Value::as_str)
            == Some(OPENROUTER_MODEL)
    );

    let observations = service.observations();
    assert_eq!(observations.len(), 1);
    assert_eq!(observations[0].path, "/api/v1/chat/completions");
    assert_eq!(observations[0].api_mode, "ChatCompletions");
    assert_eq!(
        observations[0].profile_sha256,
        profile.provider_profile().settings_sha256
    );
    assert_eq!(observations[0].model, OPENROUTER_MODEL);
    assert!(observations[0].stream);
    assert_eq!(observations[0].tools, 1);
    assert!(observations[0].forbidden_settings_present.is_empty());
    assert_eq!(observations[0].authorization_digest_sha256.len(), 64);

    let serialized = serde_json::to_string(&observations).unwrap();
    assert!(!serialized.contains(FIXTURE_CREDENTIAL));
    service.stop();
}

#[test]
fn responses_route_is_verified_for_the_codex_adapter() {
    let profile = profile();
    let mut service =
        SyntheticOpenRouter::start(profile.clone(), OpenRouterAgent::Codex, ServiceMode::Serve);
    let cancelled = Arc::new(AtomicBool::new(false));
    let response = post_with_retries(
        service.address,
        "/api/v1/responses",
        FIXTURE_CREDENTIAL,
        &responses_body(),
        1,
        &cancelled,
    )
    .unwrap();
    assert_eq!(response.status, 200);
    let observations = service.observations();
    assert_eq!(observations.len(), 1);
    assert_eq!(observations[0].path, "/api/v1/responses");
    assert_eq!(observations[0].api_mode, "Responses");
    assert_eq!(observations[0].model, OPENROUTER_MODEL);
    assert!(observations[0].stream);
    assert_eq!(observations[0].tools, 1);
    assert!(
        !serde_json::to_string(&observations)
            .unwrap()
            .contains(FIXTURE_CREDENTIAL)
    );
    service.stop();
}

#[test]
fn transient_failures_are_retried_with_bounded_backoff() {
    let profile = profile();
    let mut service = SyntheticOpenRouter::start(
        profile.clone(),
        OpenRouterAgent::Aider,
        ServiceMode::Retry {
            transient_failures: 2,
        },
    );
    let cancelled = Arc::new(AtomicBool::new(false));
    let started = Instant::now();
    let response = post_with_retries(
        service.address,
        "/api/v1/chat/completions",
        FIXTURE_CREDENTIAL,
        &chat_body(),
        4,
        &cancelled,
    )
    .unwrap();
    assert_eq!(response.status, 200);
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(service.attempts(), 3);
    let observations = service.observations();
    assert_eq!(observations.len(), 3);
    let attempt_numbers = observations
        .iter()
        .map(|item| item.attempts)
        .collect::<Vec<_>>();
    assert_eq!(attempt_numbers, vec![1, 2, 3]);
    assert!(
        observations
            .iter()
            .all(|item| item.profile_sha256 == profile.provider_profile().settings_sha256)
    );
    assert!(
        !serde_json::to_string(&observations)
            .unwrap()
            .contains(FIXTURE_CREDENTIAL)
    );
    service.stop();
}

#[test]
fn cancellation_fails_closed_and_never_leaks_secret() {
    let profile = profile();
    let service =
        SyntheticOpenRouter::start(profile.clone(), OpenRouterAgent::Aider, ServiceMode::Hang);
    let cancelled = Arc::new(AtomicBool::new(false));
    let client_cancelled = Arc::clone(&cancelled);
    let client = thread::spawn(move || {
        post_with_retries(
            service.address,
            "/api/v1/chat/completions",
            FIXTURE_CREDENTIAL,
            &chat_body(),
            3,
            &client_cancelled,
        )
    });
    for _ in 0..2_000 {
        if !service.observations().is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(service.observations().len(), 1);
    cancelled.store(true, Ordering::SeqCst);
    let result = client.join().expect("client thread join");
    assert_eq!(result, Err(ClientError::Cancelled));
    for _ in 0..2_000 {
        if service.closed_after_request() >= 1 {
            break;
        }
        thread::sleep(Duration::from_millis(2));
    }
    assert!(service.closed_after_request() >= 1);
    assert!(
        !serde_json::to_string(&service.observations())
            .unwrap()
            .contains(FIXTURE_CREDENTIAL)
    );
}

#[test]
fn malformed_responses_and_endpoint_mismatch_fail_closed() {
    let profile = profile();
    let cancelled = Arc::new(AtomicBool::new(false));

    let mut malformed = SyntheticOpenRouter::start(
        profile.clone(),
        OpenRouterAgent::Aider,
        ServiceMode::Malformed,
    );
    let malformed_result = post_with_retries(
        malformed.address,
        "/api/v1/chat/completions",
        FIXTURE_CREDENTIAL,
        &chat_body(),
        1,
        &cancelled,
    );
    assert_eq!(malformed_result, Err(ClientError::InvalidResponse));
    assert_eq!(malformed.attempts(), 1);
    malformed.stop();

    let mut mismatched =
        SyntheticOpenRouter::start(profile.clone(), OpenRouterAgent::Aider, ServiceMode::Serve);
    let mismatch_result = post_with_retries(
        mismatched.address,
        "/v1/chat/completions",
        FIXTURE_CREDENTIAL,
        &chat_body(),
        1,
        &cancelled,
    );
    assert_eq!(mismatch_result, Err(ClientError::HttpStatus(400)));
    assert_eq!(mismatched.rejected(), 1);
    assert!(mismatched.observations().is_empty());
    mismatched.stop();
}

#[test]
fn absent_credentials_fail_closed_before_any_network_contact() {
    let profile = profile();
    assert!(matches!(
        resolve_openrouter_credential(&profile, |_| None),
        Err(CredentialResolutionError::Unavailable)
    ));
    let mut calls = 0;
    let wrong = OpenRouterProfile::new("b".repeat(64)).unwrap();
    assert!(matches!(
        resolve_openrouter_credential(&wrong, |_| {
            calls += 1;
            None
        }),
        Err(CredentialResolutionError::ReferenceMismatch)
    ));
    assert_eq!(calls, 0);
}
