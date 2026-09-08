// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Isolated compilation and contract checks for the OpenHands adapter.
use asb_agents::openhands::{
    OpenHandsArtifact, OpenHandsConfig, SDK_WHEEL_SHA256, SUPPORTED_VERSION,
    TESTED_ENVIRONMENT_SHA256, UPSTREAM_ARCHIVE_SHA256, UPSTREAM_REVISION, UPSTREAM_TREE,
};

use asb_protocol::{Event, Id, TerminalStatus};
use asb_runtime::ProcessLimits;
use serde_json::{Value, json};
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use url::Url;

#[test]
fn manifest_records_exact_maintained_sdk_pin() {
    let config = OpenHandsConfig::new(
        "/usr/bin/python3.12",
        "/tmp/openhands-sdk.whl",
        "/tmp/site-packages",
        "/tmp/workspace",
        "/tmp/state",
        Url::parse("http://127.0.0.1:8080/v1").unwrap(),
        "asb-loopback",
        8,
        OpenHandsArtifact::LinuxX86_64V1_17_0,
    )
    .unwrap();
    let manifest = config.manifest();
    assert_eq!(manifest.extension_id.0, "agent.openhands");
    assert!(manifest.implementation_version.contains("1.17.0"));
    assert_eq!(SUPPORTED_VERSION, "1.17.0");
    assert_eq!(
        UPSTREAM_REVISION,
        "aabf40723d308da0d5f9063008c6793cc86df282"
    );
    assert_eq!(UPSTREAM_TREE, "850dd602d64b8d19560e63c2d9a4d44c48db82f4");
    for digest in [
        SDK_WHEEL_SHA256,
        UPSTREAM_ARCHIVE_SHA256,
        TESTED_ENVIRONMENT_SHA256,
    ] {
        assert_eq!(digest.len(), 64);
        assert!(digest.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }

    let freeze = include_str!("fixtures/openhands-sdk-1.17.0.freeze.txt");
    let packages = freeze.lines().collect::<std::collections::BTreeSet<_>>();
    assert_eq!(packages.len(), 120);
    assert!(packages.contains("openhands-sdk==1.17.0"));
    assert!(packages.contains("agent-client-protocol==0.8.1"));
    assert!(packages.contains("lmnr==0.7.24"));
    assert!(!freeze.contains("lmnr-claude-code-proxy"));
    assert!(!freeze.to_ascii_lowercase().contains("proprietary"));
}

struct RemoveDirectory(PathBuf);

impl Drop for RemoveDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn fixture_root() -> PathBuf {
    let target = PathBuf::from(std::env::var_os("CARGO_TARGET_DIR").expect("external target"));
    assert!(target.is_absolute());
    target.join("asb-integration-fixtures").join(format!(
        "asb-real-openhands-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

fn read_request(stream: &mut TcpStream) -> (Value, String) {
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    let mut bytes = Vec::new();
    let head_end = loop {
        let mut chunk = [0_u8; 4096];
        let count = stream.read(&mut chunk).unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&chunk[..count]);
        assert!(bytes.len() <= 4 * 1024 * 1024);
        if let Some(position) = bytes.windows(4).position(|value| value == b"\r\n\r\n") {
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
    let authorization = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("authorization")
                .then(|| value.trim().to_owned())
        })
        .unwrap();
    let already = bytes.len();
    bytes.resize(head_end + length, 0);
    stream.read_exact(&mut bytes[already..]).unwrap();
    (
        serde_json::from_slice(&bytes[head_end..]).unwrap(),
        authorization,
    )
}

fn respond(stream: &mut TcpStream, value: Value) {
    let body = serde_json::to_vec(&value).unwrap();
    write!(
        stream,
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    )
    .unwrap();
    stream.write_all(&body).unwrap();
}

#[test]
#[ignore = "requires pinned OpenHands SDK 1.17.0 Python environment on Linux x86_64"]
fn pinned_sdk_edits_with_bounded_confirmation_and_usage() {
    assert_eq!(std::env::consts::ARCH, "x86_64");
    let root = fixture_root();
    let _remove = RemoveDirectory(root.clone());
    fs::create_dir_all(root.join("workspace")).unwrap();
    fs::create_dir_all(root.join("state")).unwrap();
    fs::write(
        root.join("workspace/.env"),
        "OPENAI_API_KEY=hostile\nOPENAI_BASE_URL=https://example.invalid/v1\n",
    )
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        for (index, name, arguments) in [
            (
                1,
                "write",
                json!({"path":"result.txt","content":"openhands-ok"}),
            ),
            (2, "finish", json!({"message":"done"})),
        ] {
            let (mut stream, _) = listener.accept().unwrap();
            let (request, authorization) = read_request(&mut stream);
            assert_eq!(authorization, "Bearer asb-credential-free");
            assert_eq!(request["model"], "asb-loopback");
            let call = json!({
                "id":format!("call-{index}"),"type":"function",
                "function":{"name":name,"arguments":serde_json::to_string(&arguments).unwrap()}
            });
            respond(
                &mut stream,
                json!({
                    "id":format!("resp-{index}"),"object":"chat.completion",
                    "created":1,"model":"asb-loopback",
                    "choices":[{"index":0,"message":{"role":"assistant","content":null,
                        "tool_calls":[call]},"finish_reason":"tool_calls"}],
                    "usage":{"prompt_tokens":10,"completion_tokens":4,"total_tokens":14}
                }),
            );
        }
    });
    let config = OpenHandsConfig::new(
        std::env::var_os("ASB_OPENHANDS_PYTHON").unwrap(),
        std::env::var_os("ASB_OPENHANDS_SDK_WHEEL").unwrap(),
        std::env::var_os("ASB_OPENHANDS_SITE_PACKAGES").unwrap(),
        root.join("workspace"),
        root.join("state"),
        Url::parse(&format!("http://{address}/v1")).unwrap(),
        "asb-loopback",
        1,
        OpenHandsArtifact::LinuxX86_64V1_17_0,
    )
    .unwrap();
    let limits = ProcessLimits::new(
        8 * 1024 * 1024,
        1024 * 1024,
        Duration::from_secs(60),
        Duration::from_millis(500),
        Duration::from_millis(10),
    )
    .unwrap();
    let mut running = config
        .start(
            Id("session".into()),
            Id("attempt".into()),
            "Write the exact text openhands-ok into result.txt, then finish.",
            limits,
        )
        .unwrap();
    let outcome = running.wait().unwrap();
    server.join().unwrap();
    assert_eq!(outcome.status(), TerminalStatus::Completed);
    assert_eq!(outcome.exit_code(), Some(0));
    assert_eq!(
        fs::read_to_string(root.join("workspace/result.txt")).unwrap(),
        "openhands-ok"
    );
    assert!(
        outcome
            .events()
            .iter()
            .any(|event| matches!(event.event, Event::ToolFinished { success: true, .. }))
    );
    assert!(matches!(
        outcome.events().last().unwrap().event,
        Event::Completed
    ));
    assert!(fs::read_dir(root.join("state")).unwrap().next().is_none());
}
