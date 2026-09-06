// SPDX-License-Identifier: MIT
#![cfg(target_os = "linux")]
#![allow(missing_docs)]

use asb_agents::opencode::{OpenCodeArtifact, OpenCodeConfig, TESTED_LINUX_X86_64_SHA256};
use asb_protocol::{Event, Id, TerminalStatus};
use asb_runtime::ProcessLimits;
use serde_json::{Value, json};
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use url::Url;

fn root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "asb-real-opencode-{label}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
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

struct FixtureRequest {
    path: String,
    authorization: Option<String>,
    body: Value,
}

fn read_request(stream: &mut TcpStream) -> FixtureRequest {
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
    let path = head
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_owned();
    let authorization = head.lines().find_map(|line| {
        let (name, value) = line.split_once(": ")?;
        name.eq_ignore_ascii_case("authorization")
            .then(|| value.to_owned())
    });
    let length: usize = head
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length: ")
                .map(str::to_owned)
        })
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let already = bytes.len();
    assert!(already <= head_end + length);
    bytes.resize(head_end + length, 0);
    stream.read_exact(&mut bytes[already..]).unwrap();
    FixtureRequest {
        path,
        authorization,
        body: serde_json::from_slice(&bytes[head_end..]).unwrap(),
    }
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

fn text_response() -> Vec<Value> {
    vec![
        json!({
            "id": "fixture",
            "object": "chat.completion.chunk",
            "created": 1,
            "model": "fixture-model",
            "choices": [{"index": 0, "delta": {"role": "assistant", "content": "done"}, "finish_reason": null}]
        }),
        json!({
            "id": "fixture",
            "object": "chat.completion.chunk",
            "created": 1,
            "model": "fixture-model",
            "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 7, "completion_tokens": 2, "total_tokens": 9}
        }),
    ]
}

#[derive(Clone)]
enum ToolFixture {
    Write(PathBuf),
    DeniedBash(PathBuf),
}

fn tool_response(tool: &ToolFixture) -> Vec<Value> {
    let (name, arguments) = match tool {
        ToolFixture::Write(path) => (
            "write",
            json!({"filePath": path, "content": "tool fixture\n"}).to_string(),
        ),
        ToolFixture::DeniedBash(path) => (
            "bash",
            json!({"command": format!("/usr/bin/touch {}", path.display())}).to_string(),
        ),
    };
    vec![
        json!({
            "id": "fixture-tool",
            "object": "chat.completion.chunk",
            "created": 1,
            "model": "fixture-model",
            "choices": [{
                "index": 0,
                "delta": {
                    "role": "assistant",
                    "tool_calls": [{
                        "index": 0,
                        "id": "call_asb_fixture",
                        "type": "function",
                        "function": {"name": name, "arguments": arguments}
                    }]
                },
                "finish_reason": null
            }]
        }),
        json!({
            "id": "fixture-tool",
            "object": "chat.completion.chunk",
            "created": 1,
            "model": "fixture-model",
            "choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]
        }),
    ]
}

fn server(
    listener: TcpListener,
    stop: Arc<AtomicBool>,
    requests: Arc<AtomicUsize>,
    tool: ToolFixture,
) -> thread::JoinHandle<()> {
    listener.set_nonblocking(true).unwrap();
    thread::spawn(move || {
        while !stop.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_nonblocking(false).unwrap();
                    let request = read_request(&mut stream);
                    assert_eq!(request.path, "/v1/chat/completions");
                    assert_eq!(
                        request.authorization.as_deref(),
                        Some("Bearer asb-credential-free")
                    );
                    assert_eq!(
                        request.body.get("model").and_then(Value::as_str),
                        Some("fixture-model")
                    );
                    requests.fetch_add(1, Ordering::SeqCst);
                    let tools = request
                        .body
                        .get("tools")
                        .and_then(Value::as_array)
                        .is_some_and(|items| !items.is_empty());
                    let has_tool_result = request
                        .body
                        .get("messages")
                        .and_then(Value::as_array)
                        .is_some_and(|messages| {
                            messages.iter().any(|message| {
                                message.get("role").and_then(Value::as_str) == Some("tool")
                            })
                        });
                    let response = if tools && !has_tool_result {
                        tool_response(&tool)
                    } else {
                        text_response()
                    };
                    sse(&mut stream, &response);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(2));
                }
                Err(error) => panic!("fixture listener failed: {error}"),
            }
        }
    })
}

fn adapter(binary: &Path, root: &Path, endpoint: Url) -> OpenCodeConfig {
    OpenCodeConfig::new(
        binary,
        root.join("work"),
        root.join("state"),
        endpoint,
        "asb/fixture-model",
        OpenCodeArtifact::LinuxX86_64V1_18_29,
    )
    .unwrap()
}

#[test]
#[ignore = "requires exact pinned OpenCode 1.18.29 Linux x86_64 executable"]
fn pinned_binary_completes_tool_fixture_and_cancels() {
    assert_eq!(std::env::consts::ARCH, "x86_64");
    let binary = PathBuf::from(std::env::var_os("ASB_OPENCODE_BIN").expect("binary path"));
    assert_eq!(
        std::env::var("ASB_OPENCODE_SHA256").unwrap(),
        TESTED_LINUX_X86_64_SHA256
    );
    let test_root = root("journey");
    fs::create_dir_all(&test_root).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let endpoint = Url::parse(&format!("http://{address}/v1")).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let requests = Arc::new(AtomicUsize::new(0));
    let output_path = test_root.join("work/tool-output.txt");
    fs::create_dir_all(test_root.join("work")).unwrap();
    fs::write(
        test_root.join("work/opencode.json"),
        r#"{"model":"ambient/forbidden","provider":{"ambient":{"options":{"baseURL":"http://127.0.0.1:1"}}}}"#,
    )
    .unwrap();
    let fixture_server = server(
        listener,
        Arc::clone(&stop),
        Arc::clone(&requests),
        ToolFixture::Write(output_path.clone()),
    );
    let mut attempt = adapter(&binary, &test_root, endpoint)
        .start(
            Id("fixture-session".into()),
            Id("fixture-attempt".into()),
            "Use the write tool once, then finish.",
            limits(Duration::from_secs(30)),
        )
        .unwrap();
    let cmdline = fs::read(format!("/proc/{}/cmdline", attempt.pid())).unwrap();
    let environ = fs::read(format!("/proc/{}/environ", attempt.pid())).unwrap();
    for process_data in [&cmdline, &environ] {
        assert!(
            !process_data
                .windows(b"Use the write tool once, then finish.".len())
                .any(|window| window == b"Use the write tool once, then finish.")
        );
        assert!(
            !process_data
                .windows(b"ambient/forbidden".len())
                .any(|window| window == b"ambient/forbidden")
        );
    }
    let outcome = attempt.wait().unwrap();
    stop.store(true, Ordering::SeqCst);
    fixture_server.join().unwrap();
    assert_eq!(outcome.status(), TerminalStatus::Completed);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "tool fixture\n");
    assert!(requests.load(Ordering::SeqCst) >= 3);
    assert!(outcome.events().windows(2).any(|events| {
        matches!(
            (&events[0].event, &events[1].event),
            (
                Event::ToolStarted {
                    tool_call_id: started,
                    name
                },
                Event::ToolFinished {
                    tool_call_id: finished,
                    success: true
                }
            ) if started == finished && name == "write"
        )
    }));
    assert!(
        fs::read_dir(test_root.join("state"))
            .unwrap()
            .next()
            .is_none()
    );

    let denied_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let denied_endpoint = Url::parse(&format!(
        "http://{}/v1",
        denied_listener.local_addr().unwrap()
    ))
    .unwrap();
    let denied_stop = Arc::new(AtomicBool::new(false));
    let denied_requests = Arc::new(AtomicUsize::new(0));
    let outside_sentinel = test_root.join("forbidden-outside-workspace");
    let denied_server = server(
        denied_listener,
        Arc::clone(&denied_stop),
        Arc::clone(&denied_requests),
        ToolFixture::DeniedBash(outside_sentinel.clone()),
    );
    let mut denied = adapter(&binary, &test_root.join("denied"), denied_endpoint)
        .start(
            Id("denied-session".into()),
            Id("denied-attempt".into()),
            "Try the requested shell tool, then finish.",
            limits(Duration::from_secs(30)),
        )
        .unwrap();
    let denied_outcome = denied.wait().unwrap();
    denied_stop.store(true, Ordering::SeqCst);
    denied_server.join().unwrap();
    assert_eq!(denied_outcome.status(), TerminalStatus::Completed);
    assert!(!outside_sentinel.exists());
    assert!(denied_requests.load(Ordering::SeqCst) >= 3);
    assert!(denied_outcome.events().windows(2).any(|events| {
        matches!(
            (&events[0].event, &events[1].event),
            (
                Event::ToolStarted {
                    tool_call_id: started,
                    name
                },
                Event::ToolFinished {
                    tool_call_id: finished,
                    success: false
                }
            ) if started == finished && name == "bash"
        )
    }));

    let cancel_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let cancel_endpoint = Url::parse(&format!(
        "http://{}/v1",
        cancel_listener.local_addr().unwrap()
    ))
    .unwrap();
    let mut cancelled = adapter(&binary, &test_root.join("cancel"), cancel_endpoint)
        .start(
            Id("cancel-session".into()),
            Id("cancel-attempt".into()),
            "Wait for the provider.",
            limits(Duration::from_secs(30)),
        )
        .unwrap();
    let pid = cancelled.pid();
    thread::sleep(Duration::from_millis(100));
    cancelled.cancel().unwrap();
    let outcome = cancelled.wait().unwrap();
    assert_eq!(outcome.status(), TerminalStatus::Cancelled);
    assert!(!Path::new(&format!("/proc/{pid}")).exists());
    drop(cancel_listener);
    fs::remove_dir_all(test_root).unwrap();
}
