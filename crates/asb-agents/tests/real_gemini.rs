// SPDX-License-Identifier: MIT
//! Credential-free native Gemini CLI journeys against a loopback Gemini endpoint.

use asb_agents::gemini::{AdapterError, GeminiArtifact, GeminiConfig};
use asb_protocol::{Event, Id, TerminalStatus};
use asb_runtime::ProcessLimits;
use std::fs;
use std::io;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use url::Url;

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "asb-real-gemini-{name}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
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

fn adapter(root: &Path, endpoint: Url) -> GeminiConfig {
    adapter_with_actions(root, endpoint, 4)
}

fn adapter_with_actions(root: &Path, endpoint: Url, max_actions: u32) -> GeminiConfig {
    let entry = std::env::var_os("ASB_GEMINI_BIN")
        .map(PathBuf::from)
        .expect("ASB_GEMINI_BIN must name the pinned entry bundle");
    let node = std::env::var_os("ASB_GEMINI_NODE_BIN")
        .map(PathBuf::from)
        .expect("ASB_GEMINI_NODE_BIN must name the pinned Node.js runtime");
    GeminiConfig::new(
        entry,
        node,
        root.join("workspace"),
        root.join("state"),
        endpoint,
        "fixture-model",
        4,
        max_actions,
        GeminiArtifact::LinuxX86_64V0_58_0,
    )
    .unwrap()
}

#[test]
#[ignore = "requires exact pinned Gemini CLI 0.58.0 npm package and Node.js 26.3.0 on Linux x86_64"]
fn pinned_cli_blocks_action_beyond_budget_before_side_effect() {
    let scratch = Scratch::new("budget");
    let workspace = scratch.0.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
    let server = thread::spawn(move || -> io::Result<()> {
        let mut stream = accept_bounded(&listener)?;
        let _ = read_request(&mut stream);
        let body = concat!(
            "data: {\"candidates\":[{\"content\":{\"parts\":[",
            "{\"functionCall\":{\"name\":\"write_file\",\"args\":{\"file_path\":\"first.txt\",\"content\":\"first\\n\"}}},",
            "{\"functionCall\":{\"name\":\"write_file\",\"args\":{\"file_path\":\"second.txt\",\"content\":\"second\\n\"}}}",
            "],\"role\":\"model\"},\"finishReason\":\"STOP\",\"index\":0}],",
            "\"usageMetadata\":{\"promptTokenCount\":7,\"candidatesTokenCount\":2,\"totalTokenCount\":9}}\n\n"
        );
        send_sse(&mut stream, body);
        Ok(())
    });

    let config = adapter_with_actions(&scratch.0, endpoint, 1);
    let mut running = config
        .start(
            Id("session".into()),
            Id("attempt".into()),
            "Make both requested fixture files.",
            limits(),
        )
        .unwrap();
    // `start` returns only after the CLI's SessionStart hook proves that the
    // same private hook configuration governing BeforeTool is active.
    server.join().unwrap().unwrap();
    let outcome = running.wait().unwrap();
    assert_eq!(outcome.status(), TerminalStatus::Failed);
    let side_effects = [workspace.join("first.txt"), workspace.join("second.txt")]
        .into_iter()
        .filter(|path| path.exists())
        .count();
    assert_eq!(
        side_effects, 1,
        "action max+1 must be denied before execution"
    );
    assert!(
        fs::read_dir(scratch.0.join("state"))
            .unwrap()
            .next()
            .is_none()
    );
}

#[test]
#[ignore = "requires exact pinned Gemini CLI 0.58.0 npm package and Node.js 26.3.0 on Linux x86_64"]
fn pinned_cli_denies_unsupported_shell_before_side_effect() {
    let scratch = Scratch::new("deny-shell");
    let workspace = scratch.0.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
    let server = thread::spawn(move || -> io::Result<()> {
        let mut stream = accept_bounded(&listener)?;
        let _ = read_request(&mut stream);
        let body = concat!(
            "data: {\"candidates\":[{\"content\":{\"parts\":[",
            "{\"functionCall\":{\"name\":\"run_shell_command\",\"args\":{\"command\":\"printf forbidden > forbidden.txt\"}}}",
            "],\"role\":\"model\"},\"finishReason\":\"STOP\",\"index\":0}],",
            "\"usageMetadata\":{\"promptTokenCount\":7,\"candidatesTokenCount\":2,\"totalTokenCount\":9}}\n\n"
        );
        send_sse(&mut stream, body);
        let mut second = accept_bounded(&listener)?;
        let _ = read_request(&mut second);
        let completed = concat!(
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"blocked\"}],\"role\":\"model\"},",
            "\"finishReason\":\"STOP\",\"index\":0}],\"usageMetadata\":{\"promptTokenCount\":8,",
            "\"candidatesTokenCount\":1,\"totalTokenCount\":9}}\n\n"
        );
        send_sse(&mut second, completed);
        Ok(())
    });

    let config = adapter(&scratch.0, endpoint);
    let mut running = config
        .start(
            Id("session".into()),
            Id("attempt".into()),
            "Exercise the unsupported fixture route.",
            limits(),
        )
        .unwrap();
    let result = running.wait();
    server.join().unwrap().unwrap();
    match result {
        Ok(outcome) => assert_eq!(outcome.status(), TerminalStatus::Failed),
        Err(AdapterError::InvalidEvent) => {}
        Err(error) => panic!("unexpected adapter error: {error}"),
    }
    assert!(!workspace.join("forbidden.txt").exists());
    assert!(
        fs::read_dir(scratch.0.join("state"))
            .unwrap()
            .next()
            .is_none()
    );
}

fn read_request(stream: &mut TcpStream) -> Vec<u8> {
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 8192];
    let header_end = loop {
        let count = stream.read(&mut buffer).unwrap();
        assert!(count > 0, "request ended before headers");
        bytes.extend_from_slice(&buffer[..count]);
        assert!(
            bytes.len() <= 4 * 1024 * 1024,
            "loopback request exceeded bound"
        );
        if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let headers = std::str::from_utf8(&bytes[..header_end]).unwrap();
    assert!(headers.starts_with("POST /v1beta/models/fixture-model:streamGenerateContent"));
    let mut public_key_count = 0;
    for line in headers.lines().skip(1) {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.eq_ignore_ascii_case("x-goog-api-key") {
            assert_eq!(value.trim(), "asb-credential-free");
            public_key_count += 1;
        }
        assert!(!name.eq_ignore_ascii_case("authorization"));
        assert!(!name.eq_ignore_ascii_case("cookie"));
    }
    assert_eq!(
        public_key_count, 1,
        "exact public sentinel API key required"
    );
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().unwrap())
        })
        .unwrap();
    while bytes.len() - header_end < content_length {
        let count = stream.read(&mut buffer).unwrap();
        assert!(count > 0, "request ended before body");
        bytes.extend_from_slice(&buffer[..count]);
        assert!(
            bytes.len() <= 4 * 1024 * 1024,
            "loopback request exceeded bound"
        );
    }
    bytes
}

fn send_sse(stream: &mut TcpStream, body: &str) {
    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(response.as_bytes()).unwrap();
    stream.flush().unwrap();
}

fn accept_bounded(listener: &TcpListener) -> io::Result<TcpStream> {
    listener.set_nonblocking(true)?;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match listener.accept() {
            Ok((stream, _)) => return Ok(stream),
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Gemini did not connect",
                ));
            }
            Err(error) => return Err(error),
        }
    }
}

#[test]
#[ignore = "requires exact pinned Gemini CLI 0.58.0 npm package and Node.js 26.3.0 on Linux x86_64"]
fn pinned_cli_edits_with_tool_usage_and_cleans_state() {
    let scratch = Scratch::new("edit");
    let workspace = scratch.0.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    fs::write(workspace.join("target.txt"), b"before\n").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
    let server = thread::spawn(move || -> io::Result<()> {
        let mut first = accept_bounded(&listener)?;
        let request = read_request(&mut first);
        assert!(request.windows(10).any(|window| window == b"target.txt"));
        let first_body = concat!(
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"functionCall\":{\"name\":\"write_file\",\"args\":{\"file_path\":\"target.txt\",\"content\":\"after\\n\"}}}],\"role\":\"model\"},\"finishReason\":\"STOP\",\"index\":0}],",
            "\"usageMetadata\":{\"promptTokenCount\":7,\"candidatesTokenCount\":2,\"totalTokenCount\":9}}\n\n"
        );
        send_sse(&mut first, first_body);
        let mut second = accept_bounded(&listener)?;
        let _ = read_request(&mut second);
        let second_body = concat!(
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"done\"}],\"role\":\"model\"},\"finishReason\":\"STOP\",\"index\":0}],",
            "\"usageMetadata\":{\"promptTokenCount\":8,\"candidatesTokenCount\":1,\"totalTokenCount\":9}}\n\n"
        );
        send_sse(&mut second, second_body);
        Ok(())
    });

    let config = adapter(&scratch.0, endpoint);
    let mut running = config
        .start(
            Id("session".into()),
            Id("attempt".into()),
            "Replace target.txt with the requested fixture content.",
            limits(),
        )
        .unwrap();
    let outcome = running.wait().unwrap();
    server.join().unwrap().unwrap();
    assert_eq!(outcome.status(), TerminalStatus::Completed);
    assert_eq!(fs::read(workspace.join("target.txt")).unwrap(), b"after\n");
    assert!(
        outcome
            .events()
            .iter()
            .any(|event| matches!(event.event, Event::ToolStarted { .. }))
    );
    assert!(
        outcome
            .events()
            .iter()
            .any(|event| matches!(event.event, Event::ToolFinished { success: true, .. }))
    );
    assert!(
        outcome
            .events()
            .iter()
            .any(|event| matches!(event.event, Event::Usage(_)))
    );
    assert!(
        fs::read_dir(scratch.0.join("state"))
            .unwrap()
            .next()
            .is_none()
    );
}

#[test]
#[ignore = "requires exact pinned Gemini CLI 0.58.0 npm package and Node.js 26.3.0 on Linux x86_64"]
fn pinned_cli_cancellation_reaps_process_and_cleans_state() {
    let scratch = Scratch::new("cancel");
    fs::create_dir_all(scratch.0.join("workspace")).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
    let (seen_tx, seen_rx) = mpsc::sync_channel(1);
    let server = thread::spawn(move || -> io::Result<()> {
        let mut stream = accept_bounded(&listener)?;
        let _ = read_request(&mut stream);
        seen_tx.send(()).unwrap();
        thread::sleep(Duration::from_secs(2));
        Ok(())
    });
    let config = adapter(&scratch.0, endpoint);
    let mut running = config
        .start(
            Id("session".into()),
            Id("attempt".into()),
            "Wait for the provider response.",
            limits(),
        )
        .unwrap();
    let seen = seen_rx.recv_timeout(Duration::from_secs(15));
    running.cancel().unwrap();
    running.cancel().unwrap();
    let outcome = running.wait().unwrap();
    server.join().unwrap().unwrap();
    seen.unwrap();
    assert_eq!(outcome.status(), TerminalStatus::Cancelled);
    assert!(
        fs::read_dir(scratch.0.join("state"))
            .unwrap()
            .next()
            .is_none()
    );
}
