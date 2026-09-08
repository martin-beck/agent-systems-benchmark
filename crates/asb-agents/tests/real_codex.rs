// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
#![cfg(target_os = "linux")]
#![allow(missing_docs)]
use asb_agents::codex::{CodexArtifact, CodexConfig, TESTED_LINUX_X86_64_SHA256};
use asb_protocol::{Event, Id, TerminalStatus};
use asb_runtime::ProcessLimits;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use url::Url;

struct Cleanup(PathBuf);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn root() -> PathBuf {
    let target = PathBuf::from(std::env::var_os("CARGO_TARGET_DIR").expect("external target"));
    assert!(target.is_absolute());
    target.join("asb-integration-fixtures").join(format!(
        "codex-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}
fn fixture(root: &Path, mode: &str) -> (ChildGuard, Url) {
    let port = root.join(format!("{mode}.port"));
    let child = Command::new("python3")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/codex_responses.py"))
        .args([
            port.as_os_str(),
            root.join("work").as_os_str(),
            mode.as_ref(),
        ])
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !port.exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    let port = fs::read_to_string(port).unwrap();
    (
        ChildGuard(child),
        Url::parse(&format!("http://127.0.0.1:{}/v1", port.trim())).unwrap(),
    )
}
fn adapter(binary: &Path, root: &Path, endpoint: Url) -> CodexConfig {
    CodexConfig::new(
        binary,
        root.join("work"),
        root.join("state"),
        endpoint,
        "fixture-model",
        CodexArtifact::LinuxX86_64V0_153_4,
    )
    .unwrap()
}
fn process_is_live(pid: &str) -> bool {
    let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    stat.rsplit_once(") ")
        .and_then(|(_, fields)| fields.bytes().next())
        .is_some_and(|state| state != b'Z')
}
fn fixture_child_pid() -> Option<String> {
    fs::read_dir("/proc")
        .ok()?
        .filter_map(Result::ok)
        .find_map(|entry| {
            let pid = entry.file_name().into_string().ok()?;
            let command = fs::read(entry.path().join("cmdline")).ok()?;
            command
                .windows(b"asb-codex-ar0304-child".len())
                .any(|part| part == b"asb-codex-ar0304-child")
                .then_some(pid)
        })
}
#[test]
#[ignore = "requires exact pinned Codex 0.153.4 Linux x86_64 executable"]
fn pinned_binary_executes_fixture_and_cancels_child() {
    assert_eq!(std::env::consts::ARCH, "x86_64");
    let binary = PathBuf::from(std::env::var_os("ASB_CODEX_BIN").expect("binary"));
    assert_eq!(
        std::env::var("ASB_CODEX_SHA256").unwrap(),
        TESTED_LINUX_X86_64_SHA256
    );
    let root = root();
    fs::create_dir_all(root.join("work")).unwrap();
    let _cleanup = Cleanup(root.clone());
    let (mut server, endpoint) = fixture(&root, "journey");
    let mut attempt = adapter(&binary, &root, endpoint)
        .start(
            Id("s".into()),
            Id("a".into()),
            "Use the fixture tool once.",
            ProcessLimits::default(),
        )
        .unwrap();
    let outcome = attempt.wait().unwrap();
    assert!(server.0.wait().unwrap().success());
    assert_eq!(outcome.status(), TerminalStatus::Completed);
    assert_eq!(
        fs::read_to_string(root.join("work/tool-output.txt")).unwrap(),
        "fixture-tool"
    );
    assert!(outcome.events().windows(2).any(|events| matches!((&events[0].event,&events[1].event),
        (Event::ToolStarted { tool_call_id:a,name },Event::ToolFinished { tool_call_id:b,success:true })
        if a==b && name=="command_execution")));
    let (mut cancel_server, endpoint) = fixture(&root, "cancel");
    let mut cancelled = adapter(&binary, &root, endpoint)
        .start(
            Id("cs".into()),
            Id("ca".into()),
            "Run the fixture command.",
            ProcessLimits::default(),
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let child_pid = loop {
        if let Some(pid) = fixture_child_pid() {
            break pid;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    };
    cancelled.cancel().unwrap();
    let result = cancelled.wait().unwrap();
    assert_eq!(result.status(), TerminalStatus::Cancelled);
    cancel_server.0.kill().unwrap();
    let _ = cancel_server.0.wait();
    let deadline = Instant::now() + Duration::from_secs(5);
    while process_is_live(&child_pid) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!process_is_live(&child_pid));
    assert!(fs::read_dir(root.join("state")).unwrap().next().is_none());
}
