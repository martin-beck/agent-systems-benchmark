// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
#![cfg(target_os = "linux")]
#![allow(missing_docs)]

use asb_agents::qwen_code::{QwenCodeArtifact, QwenCodeConfig};
use asb_protocol::{Event, Id, TerminalStatus};
use asb_runtime::ProcessLimits;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use url::Url;

struct RemoveDirectory(PathBuf);
impl Drop for RemoveDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn root(label: &str) -> PathBuf {
    let target = PathBuf::from(std::env::var_os("CARGO_TARGET_DIR").expect("external target root"));
    assert!(target.is_absolute());
    target.join("asb-integration-fixtures").join(format!(
        "asb-real-qwen-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

fn limits() -> ProcessLimits {
    ProcessLimits::new(
        16 * 1024 * 1024,
        1024 * 1024,
        Duration::from_secs(40),
        Duration::from_millis(300),
        Duration::from_millis(5),
    )
    .unwrap()
}

fn start_server(mode: &str, target: &Path, ready: &Path) -> (Server, u16) {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/qwen_openai.py");
    let mut child = Command::new("/usr/bin/python3")
        .arg(fixture)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("ASB_QWEN_MODE", mode)
        .env("ASB_QWEN_TARGET", target)
        .env("ASB_QWEN_READY", ready)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut line = String::new();
    BufReader::new(stdout).read_line(&mut line).unwrap();
    let port = line.trim().parse().unwrap();
    (Server(child), port)
}

fn config(root: &Path, port: u16) -> QwenCodeConfig {
    QwenCodeConfig::new(
        PathBuf::from(std::env::var_os("ASB_QWEN_EXECUTABLE").expect("Qwen launcher path")),
        PathBuf::from(std::env::var_os("ASB_QWEN_ARCHIVE").expect("Qwen archive path")),
        root.join("workspace"),
        root.join("state"),
        Url::parse(&format!("http://127.0.0.1:{port}/v1")).unwrap(),
        "fixture-model",
        QwenCodeArtifact::LinuxX86_64V0_23_0,
    )
    .unwrap()
}

#[test]
#[ignore = "requires official pinned Qwen Code 0.23.0 Linux x86_64 standalone archive"]
fn pinned_qwen_edits_streams_and_cancels() {
    assert_eq!(std::env::consts::ARCH, "x86_64");
    let scratch = root("edit");
    fs::create_dir_all(scratch.join("workspace")).unwrap();
    fs::create_dir_all(scratch.join("state")).unwrap();
    let _remove = RemoveDirectory(scratch.clone());
    let target = scratch.join("workspace/new.txt");
    let ready = scratch.join("unused-ready");
    let (_server, port) = start_server("edit", &target, &ready);
    let adapter = config(&scratch, port);
    let mut running = adapter
        .start(
            Id("session".into()),
            Id("edit".into()),
            "create the requested fixture file",
            limits(),
        )
        .unwrap();
    let outcome = running.wait().unwrap();
    assert_eq!(outcome.status(), TerminalStatus::Completed);
    assert_eq!(fs::read_to_string(&target).unwrap(), "new\n");
    assert!(outcome.events().iter().any(|event| matches!(
        &event.event,
        Event::ToolStarted { name, .. } if name == "write_file"
    )));
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
    assert!(
        fs::read_dir(scratch.join("state"))
            .unwrap()
            .next()
            .is_none()
    );

    let cancel = root("cancel");
    fs::create_dir_all(cancel.join("workspace")).unwrap();
    fs::create_dir_all(cancel.join("state")).unwrap();
    let _remove_cancel = RemoveDirectory(cancel.clone());
    let ready = cancel.join("provider-request-started");
    let (_server, port) = start_server("hang", &cancel.join("unused"), &ready);
    let adapter = config(&cancel, port);
    let mut running = adapter
        .start(
            Id("session".into()),
            Id("cancel".into()),
            "wait for the provider fixture",
            limits(),
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(25);
    while !ready.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        ready.exists(),
        "Qwen Code did not reach the provider fixture"
    );
    running.cancel().unwrap();
    let outcome = running.wait().unwrap();
    assert_eq!(outcome.status(), TerminalStatus::Cancelled);
    assert!(fs::read_dir(cancel.join("state")).unwrap().next().is_none());

    let failure = root("provider-error");
    fs::create_dir_all(failure.join("workspace")).unwrap();
    fs::create_dir_all(failure.join("state")).unwrap();
    let _remove_failure = RemoveDirectory(failure.clone());
    let (_server, port) = start_server(
        "error",
        &failure.join("unused"),
        &failure.join("unused-ready"),
    );
    let adapter = config(&failure, port);
    let mut running = adapter
        .start(
            Id("session".into()),
            Id("provider-error".into()),
            "exercise a provider failure",
            limits(),
        )
        .unwrap();
    match running.wait() {
        Ok(outcome) => assert_ne!(outcome.status(), TerminalStatus::Completed),
        Err(error) => assert!(!error.to_string().contains("private provider diagnostic")),
    }
    assert!(
        fs::read_dir(failure.join("state"))
            .unwrap()
            .next()
            .is_none()
    );
}
