// SPDX-License-Identifier: MIT
#![cfg(target_os = "linux")]
#![allow(missing_docs)]

use asb_agents::mini_swe::{
    MiniSweArtifact, MiniSweConfig, RetryObservation, RetryUnavailableReason, WHEEL_SHA256,
};
use asb_protocol::{Event, Id, TerminalStatus};
use asb_runtime::ProcessLimits;
use serde_json::{Value, json};
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use url::Url;

struct RemoveDirectory(PathBuf);

impl Drop for RemoveDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn root(label: &str) -> PathBuf {
    let target = PathBuf::from(std::env::var_os("CARGO_TARGET_DIR").expect("external target root"));
    assert!(target.is_absolute());
    target.join("asb-integration-fixtures").join(format!(
        "asb-real-mini_swe-{label}-{}-{}",
        std::process::id(),
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
        Duration::from_millis(300),
        Duration::from_millis(5),
    )
    .unwrap()
}

fn read_request(stream: &mut TcpStream) -> Value {
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
    assert!(head.starts_with("POST /v1/chat/completions HTTP/1.1\r\n"));
    let length = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().unwrap())
        })
        .unwrap();
    let already = bytes.len();
    assert!(already <= head_end + length);
    bytes.resize(head_end + length, 0);
    stream.read_exact(&mut bytes[already..]).unwrap();
    serde_json::from_slice(&bytes[head_end..]).unwrap()
}

fn respond_with_status(stream: &mut TcpStream, status: &str, value: &Value) {
    let body = serde_json::to_vec(value).unwrap();
    write!(
        stream,
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    )
    .unwrap();
    stream.write_all(&body).unwrap();
    stream.flush().unwrap();
}

fn respond(stream: &mut TcpStream, value: &Value) {
    respond_with_status(stream, "200 OK", value);
}

fn accept_before(listener: &TcpListener, timeout: Duration) -> Option<TcpStream> {
    listener.set_nonblocking(true).unwrap();
    let deadline = Instant::now() + timeout;
    loop {
        match listener.accept() {
            Ok((stream, _)) => return Some(stream),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return None;
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("fixture accept failed: {error}"),
        }
    }
}

#[test]
#[ignore = "requires exact mini-SWE-agent 2.4.6 wheel and pinned CPython 3.12 Linux x86_64 runtime"]
fn pinned_mini_swe_edits_fixture_and_cancels() {
    assert_eq!(std::env::consts::ARCH, "x86_64");
    let python = PathBuf::from(std::env::var_os("ASB_MINI_SWE_PYTHON").expect("Python path"));
    let wheel = PathBuf::from(std::env::var_os("ASB_MINI_SWE_WHEEL").expect("wheel path"));
    assert_eq!(
        std::env::var("ASB_MINI_SWE_WHEEL_SHA256").unwrap(),
        WHEEL_SHA256
    );

    let scratch = root("edit");
    fs::create_dir_all(scratch.join("workspace")).unwrap();
    fs::write(scratch.join("workspace/target.txt"), "old\n").unwrap();
    let _remove = RemoveDirectory(scratch.clone());

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let saw_prompt = Arc::new(AtomicBool::new(false));
    let server_saw_prompt = Arc::clone(&saw_prompt);
    let requests = Arc::new(AtomicUsize::new(0));
    let server_requests = Arc::clone(&requests);
    let server = thread::spawn(move || {
        let Some(mut stream) = accept_before(&listener, Duration::from_secs(35)) else {
            return false;
        };
        let request = read_request(&mut stream);
        server_requests.fetch_add(1, Ordering::SeqCst);
        let contains_instruction = request["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|message| message["content"].as_str())
            .any(|content| content.contains("replace old with new"));
        server_saw_prompt.store(contains_instruction, Ordering::SeqCst);
        respond(
            &mut stream,
            &json!({
                "id":"asb-edit","object":"chat.completion","created":1,"model":"gpt-4o-mini",
                "choices":[{"index":0,"message":{"role":"assistant","content":null,
                  "tool_calls":[{"id":"call-edit","type":"function","function":{"name":"bash","arguments":"{\"command\":\"printf 'new\\n' > target.txt\"}"}}]},"finish_reason":"tool_calls"}],
                "usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}
            }),
        );
        let Some(mut stream) = accept_before(&listener, Duration::from_secs(35)) else {
            return false;
        };
        let request = read_request(&mut stream);
        server_requests.fetch_add(1, Ordering::SeqCst);
        let saw_tool_result = request["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| message["role"] == "tool");
        server_saw_prompt.fetch_and(saw_tool_result, Ordering::SeqCst);
        respond(
            &mut stream,
            &json!({
                "id": "asb-fixture",
                "object": "chat.completion",
                "created": 1,
                "model": "gpt-4o-mini",
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": null,
                        "tool_calls":[{"id":"call-submit","type":"function","function":{"name":"bash","arguments":"{\"command\":\"echo COMPLETE_TASK_AND_SUBMIT_FINAL_OUTPUT\"}"}}]
                    },
                    "finish_reason": "tool_calls"
                }],
                "usage": {"prompt_tokens": 10, "completion_tokens": 10, "total_tokens": 20}
            }),
        );
        true
    });
    let config = MiniSweConfig::new(
        &python,
        &wheel,
        scratch.join("workspace"),
        scratch.join("state"),
        Url::parse(&format!("http://{address}/v1")).unwrap(),
        "gpt-4o-mini",
        MiniSweArtifact::LinuxX86_64V2_4_6,
    )
    .unwrap();
    let mut running = config
        .start(
            Id("session".into()),
            Id("attempt".into()),
            "replace old with new in target.txt",
            limits(Duration::from_secs(30)),
        )
        .unwrap();
    let outcome = running.wait().unwrap();
    assert!(
        server.join().unwrap(),
        "mini_swe never reached provider fixture"
    );
    assert_eq!(outcome.status(), TerminalStatus::Completed);
    assert_eq!(outcome.exit_code(), Some(0));
    assert!(matches!(
        outcome.events().last().unwrap().event,
        Event::Completed
    ));
    assert!(saw_prompt.load(Ordering::SeqCst));
    assert_eq!(requests.load(Ordering::SeqCst), 2);
    assert!(
        outcome
            .events()
            .iter()
            .any(|event| matches!(&event.event, Event::ToolFinished { success: true, .. }))
    );
    assert_eq!(
        outcome.retry_observation(),
        RetryObservation::Unavailable {
            reason: RetryUnavailableReason::UnstructuredBatchDiagnostics,
        }
    );
    assert_eq!(
        fs::read_to_string(scratch.join("workspace/target.txt")).unwrap(),
        "new\n"
    );
    assert!(!scratch.join("workspace/.git").exists());
    assert!(
        fs::read_dir(scratch.join("state"))
            .unwrap()
            .next()
            .is_none()
    );

    let cancel_root = root("cancel");
    fs::create_dir_all(cancel_root.join("workspace")).unwrap();
    fs::write(cancel_root.join("workspace/target.txt"), "old\n").unwrap();
    let _remove_cancel = RemoveDirectory(cancel_root.clone());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let request_started = Arc::new(AtomicBool::new(false));
    let server_request_started = Arc::clone(&request_started);
    let server = thread::spawn(move || {
        let Some(mut stream) = accept_before(&listener, Duration::from_secs(35)) else {
            return false;
        };
        let _ = read_request(&mut stream);
        server_request_started.store(true, Ordering::SeqCst);
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut byte = [0_u8; 1];
        let _ = stream.read(&mut byte);
        true
    });
    let config = MiniSweConfig::new(
        python,
        wheel,
        cancel_root.join("workspace"),
        cancel_root.join("state"),
        Url::parse(&format!("http://{address}/v1")).unwrap(),
        "gpt-4o-mini",
        MiniSweArtifact::LinuxX86_64V2_4_6,
    )
    .unwrap();
    let mut running = config
        .start(
            Id("session".into()),
            Id("cancel".into()),
            "replace old with new in target.txt",
            limits(Duration::from_secs(30)),
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(35);
    while !request_started.load(Ordering::SeqCst) {
        assert!(
            Instant::now() < deadline,
            "mini_swe never started cancel request"
        );
        thread::sleep(Duration::from_millis(10));
    }
    let command_line = fs::read(format!("/proc/{}/cmdline", running.pid())).unwrap();
    assert!(
        !command_line
            .windows(b"replace old with new".len())
            .any(|window| window == b"replace old with new")
    );
    running.cancel().unwrap();
    let outcome = running.wait().unwrap();
    assert!(
        server.join().unwrap(),
        "mini_swe never reached provider fixture"
    );
    assert_eq!(outcome.status(), TerminalStatus::Cancelled);
    assert_eq!(
        fs::read_to_string(cancel_root.join("workspace/target.txt")).unwrap(),
        "old\n"
    );
    assert!(
        fs::read_dir(cancel_root.join("state"))
            .unwrap()
            .next()
            .is_none()
    );
}
