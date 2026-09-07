// SPDX-License-Identifier: MIT
#![cfg(target_os = "linux")]
#![allow(missing_docs)]

use asb_agents::goose::{
    GooseArtifact, GooseConfig, TESTED_LINUX_AARCH64_MUSL_SHA256, TESTED_LINUX_X86_64_MUSL_SHA256,
};
use asb_protocol::{Event, Id, TerminalStatus};
use asb_runtime::ProcessLimits;
use serde_json::{Value, json};
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use url::Url;

struct Remove(PathBuf);
impl Drop for Remove {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

static FIXTURE_NONCE: AtomicU64 = AtomicU64::new(0);

fn fixture_base(configured: Option<std::ffi::OsString>) -> PathBuf {
    fs::canonicalize(
        configured
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir),
    )
    .expect("canonical fixture base")
}

fn root(label: &str) -> PathBuf {
    let target = fixture_base(std::env::var_os("CARGO_TARGET_DIR"));
    target.join("asb-integration-fixtures").join(format!(
        "real-goose-{label}-{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        FIXTURE_NONCE.fetch_add(1, Ordering::Relaxed)
    ))
}

fn limits(timeout: Duration) -> ProcessLimits {
    ProcessLimits::new(
        16 * 1024 * 1024,
        1024 * 1024,
        timeout,
        Duration::from_millis(250),
        Duration::from_millis(5),
    )
    .unwrap()
}

struct Request {
    method: String,
    path: String,
    authorization: Option<String>,
    body: Option<Value>,
}

fn header_value<'a>(head: &'a str, wanted: &str) -> Option<&'a str> {
    head.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case(wanted).then(|| value.trim())
    })
}

fn read_request(stream: &mut TcpStream) -> Request {
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
    let mut request_line = head.lines().next().unwrap().split_whitespace();
    let method = request_line.next().unwrap().to_owned();
    let path = request_line.next().unwrap().to_owned();
    let authorization = header_value(head, "authorization").map(str::to_owned);
    if method == "GET" {
        return Request {
            method,
            path,
            authorization,
            body: None,
        };
    }
    assert_eq!(method, "POST");
    let length = header_value(head, "content-length");
    let body = if let Some(length) = length {
        let length = length.trim().parse::<usize>().unwrap();
        assert!(length <= 4 * 1024 * 1024);
        let present = bytes.len() - head_end;
        assert!(present <= length);
        bytes.resize(head_end + length, 0);
        stream
            .read_exact(&mut bytes[head_end + present..head_end + length])
            .unwrap();
        bytes[head_end..].to_vec()
    } else {
        assert_eq!(header_value(head, "transfer-encoding"), Some("chunked"));
        read_chunked(stream, bytes[head_end..].to_vec())
    };
    Request {
        method,
        path,
        authorization,
        body: Some(serde_json::from_slice(&body).unwrap()),
    }
}

fn read_chunked(stream: &mut TcpStream, mut encoded: Vec<u8>) -> Vec<u8> {
    let mut decoded = Vec::new();
    loop {
        let line_end = loop {
            if let Some(position) = encoded.windows(2).position(|item| item == b"\r\n") {
                break position;
            }
            let mut chunk = [0_u8; 4096];
            let count = stream.read(&mut chunk).unwrap();
            assert!(count > 0);
            encoded.extend_from_slice(&chunk[..count]);
            assert!(encoded.len() <= 4 * 1024 * 1024 + 64 * 1024);
        };
        let size_text = std::str::from_utf8(&encoded[..line_end]).unwrap();
        assert!(!size_text.contains(';'));
        let size = usize::from_str_radix(size_text, 16).unwrap();
        let required = size.checked_add(2).unwrap();
        assert!(required <= 4 * 1024 * 1024 + 2);
        encoded.drain(..line_end + 2);
        while encoded.len() < required {
            let mut chunk = [0_u8; 4096];
            let count = stream.read(&mut chunk).unwrap();
            assert!(count > 0);
            encoded.extend_from_slice(&chunk[..count]);
            assert!(encoded.len() <= 4 * 1024 * 1024 + 64 * 1024);
        }
        assert_eq!(&encoded[size..size + 2], b"\r\n");
        if size == 0 {
            break;
        }
        assert!(decoded.len().checked_add(size).unwrap() <= 4 * 1024 * 1024);
        decoded.extend_from_slice(&encoded[..size]);
        encoded.drain(..size + 2);
    }
    decoded
}

fn sse(stream: &mut TcpStream, chunks: &[Value]) {
    let mut body = Vec::new();
    for chunk in chunks {
        body.extend_from_slice(b"data: ");
        body.extend_from_slice(serde_json::to_string(chunk).unwrap().as_bytes());
        body.extend_from_slice(b"\n\n");
    }
    body.extend_from_slice(b"data: [DONE]\n\n");
    write!(
        stream,
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    )
    .unwrap();
    stream.write_all(&body).unwrap();
    stream.flush().unwrap();
}

fn models(stream: &mut TcpStream) {
    let body = br#"{"data":[{"id":"fixture-model","object":"model","meta":{"n_ctx":8192}}]}"#;
    write!(
        stream,
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    )
    .unwrap();
    stream.write_all(body).unwrap();
    stream.flush().unwrap();
}

fn tool_chunks() -> Vec<Value> {
    vec![
        json!({
            "id":"fixture-tool","object":"chat.completion.chunk","created":1,
            "model":"fixture-model","choices":[{"index":0,"delta":{"role":"assistant",
            "tool_calls":[{"index":0,"id":"call_asb_fixture","type":"function",
            "function":{"name":"write","arguments":
                "{\"path\":\"result.txt\",\"content\":\"tool fixture\\n\"}"}}]},
            "finish_reason":null}]
        }),
        json!({
            "id":"fixture-tool","object":"chat.completion.chunk","created":1,
            "model":"fixture-model","choices":[{"index":0,"delta":{},
            "finish_reason":"tool_calls"}]
        }),
    ]
}

fn final_chunks() -> Vec<Value> {
    vec![
        json!({
            "id":"fixture-final","object":"chat.completion.chunk","created":1,
            "model":"fixture-model","choices":[{"index":0,"delta":
            {"role":"assistant","content":"done"},"finish_reason":null}]
        }),
        json!({
            "id":"fixture-final","object":"chat.completion.chunk","created":1,
            "model":"fixture-model","choices":[{"index":0,"delta":{},
            "finish_reason":"stop"}],"usage":
            {"prompt_tokens":11,"completion_tokens":3,"total_tokens":14}
        }),
    ]
}

fn server(
    listener: TcpListener,
    stop: Arc<AtomicBool>,
    requests: Arc<AtomicUsize>,
) -> thread::JoinHandle<()> {
    listener.set_nonblocking(true).unwrap();
    thread::spawn(move || {
        while !stop.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_nonblocking(false).unwrap();
                    let request = read_request(&mut stream);
                    if request.method == "GET" {
                        assert_eq!(request.path, "/v1/models");
                        assert_eq!(
                            request.authorization.as_deref(),
                            Some("Bearer asb-credential-free")
                        );
                        models(&mut stream);
                        continue;
                    }
                    assert_eq!(request.path, "/v1/chat/completions");
                    assert_eq!(
                        request.authorization.as_deref(),
                        Some("Bearer asb-credential-free")
                    );
                    assert_eq!(
                        request
                            .body
                            .as_ref()
                            .unwrap()
                            .get("model")
                            .and_then(Value::as_str),
                        Some("fixture-model")
                    );
                    assert!(
                        !request
                            .body
                            .as_ref()
                            .unwrap()
                            .to_string()
                            .contains("ASB_AMBIENT_CONTEXT_SENTINEL")
                    );
                    assert!(
                        !request
                            .body
                            .as_ref()
                            .unwrap()
                            .to_string()
                            .contains("ASB_GOOSEHINTS_SENTINEL")
                    );
                    let tools = request.body.as_ref().unwrap()["tools"].as_array().unwrap();
                    let names = tools
                        .iter()
                        .filter_map(|tool| tool["function"]["name"].as_str())
                        .collect::<Vec<_>>();
                    for required in ["edit", "read_image", "shell", "tree", "write"] {
                        assert!(names.contains(&required));
                    }
                    let number = requests.fetch_add(1, Ordering::SeqCst);
                    let chunks = if number == 0 {
                        tool_chunks()
                    } else {
                        final_chunks()
                    };
                    sse(&mut stream, &chunks);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(2));
                }
                Err(error) => panic!("fixture listener failed: {error}"),
            }
        }
    })
}

fn blocking_server(
    listener: TcpListener,
    accepted: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    listener.set_nonblocking(true).unwrap();
    thread::spawn(move || {
        while !stop.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let request = read_request(&mut stream);
                    if request.method == "GET" {
                        assert_eq!(request.path, "/v1/models");
                        assert_eq!(
                            request.authorization.as_deref(),
                            Some("Bearer asb-credential-free")
                        );
                        models(&mut stream);
                        continue;
                    }
                    assert_eq!(request.path, "/v1/chat/completions");
                    assert_eq!(
                        request.authorization.as_deref(),
                        Some("Bearer asb-credential-free")
                    );
                    accepted.store(true, Ordering::SeqCst);
                    while !stop.load(Ordering::SeqCst) {
                        thread::sleep(Duration::from_millis(2));
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(2));
                }
                Err(error) => panic!("fixture listener failed: {error}"),
            }
        }
    })
}

fn error_server(
    listener: TcpListener,
    stop: Arc<AtomicBool>,
    requests: Arc<AtomicUsize>,
) -> thread::JoinHandle<()> {
    listener.set_nonblocking(true).unwrap();
    thread::spawn(move || {
        while !stop.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let request = read_request(&mut stream);
                    if request.method == "GET" {
                        assert_eq!(request.path, "/v1/models");
                        assert_eq!(
                            request.authorization.as_deref(),
                            Some("Bearer asb-credential-free")
                        );
                        models(&mut stream);
                        continue;
                    }
                    assert_eq!(request.path, "/v1/chat/completions");
                    assert_eq!(
                        request.authorization.as_deref(),
                        Some("Bearer asb-credential-free")
                    );
                    requests.fetch_add(1, Ordering::SeqCst);
                    let body = br#"{"error":{"message":"ASB_PRIVATE_PROVIDER_DIAGNOSTIC","type":"fixture"}}"#;
                    write!(
                        stream,
                        "HTTP/1.1 400 Bad Request\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                        body.len()
                    )
                    .unwrap();
                    stream.write_all(body).unwrap();
                    stream.flush().unwrap();
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(2));
                }
                Err(error) => panic!("fixture listener failed: {error}"),
            }
        }
    })
}

fn selected_artifact() -> (GooseArtifact, &'static str) {
    match std::env::consts::ARCH {
        "x86_64" => (
            GooseArtifact::LinuxX86_64MuslV1_49_0,
            TESTED_LINUX_X86_64_MUSL_SHA256,
        ),
        "aarch64" => (
            GooseArtifact::LinuxAarch64MuslV1_49_0,
            TESTED_LINUX_AARCH64_MUSL_SHA256,
        ),
        architecture => panic!("unsupported native architecture {architecture}"),
    }
}

fn adapter(binary: &Path, root: &Path, endpoint: Url) -> GooseConfig {
    let (artifact, _) = selected_artifact();
    GooseConfig::new(
        binary,
        root.join("work"),
        root.join("state"),
        endpoint,
        "fixture-model",
        3,
        artifact,
    )
    .unwrap()
}

#[test]
#[ignore = "requires exact pinned Goose 1.49.0 native Linux musl executable"]
fn pinned_binary_completes_tool_fixture_and_cancels() {
    let binary = PathBuf::from(std::env::var_os("ASB_GOOSE_BIN").expect("binary path"));
    let (_, digest) = selected_artifact();
    assert_eq!(std::env::var("ASB_GOOSE_SHA256").unwrap(), digest);
    let test_root = root("journey");
    let _remove = Remove(test_root.clone());
    fs::create_dir_all(test_root.join("work")).unwrap();
    fs::create_dir_all(test_root.join("state")).unwrap();
    fs::write(
        test_root.join("work/AGENTS.md"),
        "ASB_AMBIENT_CONTEXT_SENTINEL",
    )
    .unwrap();
    fs::write(
        test_root.join("work/.goosehints"),
        "ASB_GOOSEHINTS_SENTINEL",
    )
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = Url::parse(&format!("http://{}/v1", listener.local_addr().unwrap())).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let requests = Arc::new(AtomicUsize::new(0));
    let thread = server(listener, Arc::clone(&stop), Arc::clone(&requests));
    let mut run = adapter(&binary, &test_root, endpoint)
        .start(
            Id("session".into()),
            Id("attempt".into()),
            "Create result.txt.",
            limits(Duration::from_secs(20)),
        )
        .unwrap();
    let outcome = run.wait().unwrap();
    stop.store(true, Ordering::SeqCst);
    thread.join().unwrap();
    assert_eq!(outcome.status(), TerminalStatus::Completed);
    assert_eq!(outcome.exit_code(), Some(0));
    assert_eq!(requests.load(Ordering::SeqCst), 2);
    assert_eq!(
        fs::read_to_string(test_root.join("work/result.txt")).unwrap(),
        "tool fixture\n"
    );
    assert!(outcome.events().iter().any(|event| matches!(
        event.event,
        Event::ToolStarted { ref tool_call_id, ref name }
            if tool_call_id == &Id("call_asb_fixture".into()) && name == "write"
    )));
    assert!(
        outcome
            .events()
            .iter()
            .any(|event| matches!(event.event, Event::ToolFinished { success: true, .. }))
    );
    assert!(outcome.events().iter().any(|event| matches!(
        event.event,
        Event::Usage(ref usage)
            if usage.input_tokens == Some(11) && usage.output_tokens == Some(3)
    )));
    assert_eq!(fs::read_dir(test_root.join("state")).unwrap().count(), 0);

    // Goose 1.49.0 converts an HTTP 400 into an assistant diagnostic, emits a
    // zero-token complete event, and exits zero. The adapter must still fail.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = Url::parse(&format!("http://{}/v1", listener.local_addr().unwrap())).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let failed_requests = Arc::new(AtomicUsize::new(0));
    let thread = error_server(listener, Arc::clone(&stop), Arc::clone(&failed_requests));
    let mut run = adapter(&binary, &test_root, endpoint)
        .start(
            Id("failure-session".into()),
            Id("failure-attempt".into()),
            "Trigger a provider rejection.",
            limits(Duration::from_secs(20)),
        )
        .unwrap();
    let failed = run.wait().unwrap();
    stop.store(true, Ordering::SeqCst);
    thread.join().unwrap();
    assert!(failed_requests.load(Ordering::SeqCst) > 0);
    assert_eq!(failed.status(), TerminalStatus::Failed);
    assert_eq!(failed.exit_code(), Some(0));
    assert!(matches!(
        failed.events().last().unwrap().event,
        Event::Failed(_)
    ));
    assert!(
        !serde_json::to_string(failed.events())
            .unwrap()
            .contains("ASB_PRIVATE_PROVIDER_DIAGNOSTIC")
    );
    assert_eq!(fs::read_dir(test_root.join("state")).unwrap().count(), 0);

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = Url::parse(&format!("http://{}/v1", listener.local_addr().unwrap())).unwrap();
    let accepted = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));
    let thread = blocking_server(listener, Arc::clone(&accepted), Arc::clone(&stop));
    let mut run = adapter(&binary, &test_root, endpoint)
        .start(
            Id("cancel-session".into()),
            Id("cancel-attempt".into()),
            "Wait for the provider.",
            limits(Duration::from_secs(20)),
        )
        .unwrap();
    for _ in 0..2_000 {
        if accepted.load(Ordering::SeqCst) {
            break;
        }
        thread::sleep(Duration::from_millis(2));
    }
    assert!(accepted.load(Ordering::SeqCst));
    run.cancel().unwrap();
    run.cancel().unwrap();
    let cancelled = run.wait().unwrap();
    stop.store(true, Ordering::SeqCst);
    thread.join().unwrap();
    assert_eq!(cancelled.status(), TerminalStatus::Cancelled);
    assert_eq!(fs::read_dir(test_root.join("state")).unwrap().count(), 0);
}
