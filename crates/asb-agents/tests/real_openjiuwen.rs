// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
#![cfg(target_os = "linux")]
#![allow(missing_docs)]
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
const WHEEL_SHA256: &str = "21e9479c6b858cda28c250d63066f862fc0915cf2039edb00f016cbec7f9abba";
const PUBLIC_SENTINEL: &str = "asb-loopback-public-sentinel";
struct Scratch(PathBuf);
impl Drop for Scratch {
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
fn scratch(label: &str) -> Scratch {
    let root = PathBuf::from(std::env::var_os("CARGO_TARGET_DIR").expect("external target root"));
    assert!(root.is_absolute());
    let path = root.join("asb-integration-fixtures").join(format!(
        "asb-real-openjiuwen-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    validate_scratch_root(&path).unwrap();
    Scratch(path)
}
fn validate_scratch_root(path: &Path) -> Result<(), &'static str> {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if !path.is_absolute() || path.starts_with(repository) || path.is_symlink() {
        return Err("scratch root overlaps or aliases the repository");
    }
    let mode = fs::metadata(path)
        .map_err(|_| "scratch root is missing")?
        .permissions()
        .mode();
    if mode & 0o077 != 0 {
        return Err("scratch root is not private");
    }
    Ok(())
}
fn verify_artifacts() -> (PathBuf, PathBuf) {
    assert_eq!(
        std::env::var("ASB_OPENJIUWEN_LOOPBACK_ONLY").as_deref(),
        Ok("1")
    );
    let executable =
        PathBuf::from(std::env::var_os("ASB_OPENJIUWEN_EXECUTABLE").expect("pinned executable"));
    let wheel = PathBuf::from(std::env::var_os("ASB_OPENJIUWEN_WHEEL").expect("pinned wheel"));
    assert!(executable.is_absolute() && executable.is_file());
    assert!(wheel.is_absolute() && wheel.is_file());
    assert_eq!(
        format!("{:x}", Sha256::digest(fs::read(&wheel).unwrap())),
        WHEEL_SHA256
    );
    (executable, wheel)
}
fn wait_file(path: &Path, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while !path.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(path.exists(), "timed out waiting for {}", path.display());
}
fn start_server(root: &Path, mode: &str) -> (Server, u16, PathBuf, PathBuf) {
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/openjiuwen-loopback.py");
    let events =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/openjiuwen-events.ndjson");
    let port_file = root.join("port");
    let receipt = root.join("receipt.json");
    let target = root.join("workspace/live-edit.txt");
    fs::create_dir_all(root.join("workspace")).unwrap();
    let child = Command::new("/usr/bin/python3")
        .arg(fixture)
        .arg("--port-file")
        .arg(&port_file)
        .arg("--receipt")
        .arg(&receipt)
        .arg("--target")
        .arg(&target)
        .arg("--events")
        .arg(events)
        .args(["--mode", mode])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    wait_file(&port_file, Duration::from_secs(5));
    let port = fs::read_to_string(&port_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    (Server(child), port, receipt, target)
}
fn command(executable: &Path, root: &Path, port: u16, output_format: &str) -> Command {
    let mut cmd = Command::new(executable);
    let home = root.join("home");
    fs::create_dir_all(&home).unwrap();
    cmd.env_clear()
        .current_dir(root.join("workspace"))
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", root.join("xdg-config"))
        .env("XDG_CACHE_HOME", root.join("xdg-cache"))
        .env("PATH", "/usr/bin:/bin")
        .args([
            "--provider",
            "OpenAI",
            "--model",
            "asb-loopback",
            "--api-key",
            PUBLIC_SENTINEL,
        ])
        .arg("--api-base")
        .arg(format!("http://127.0.0.1:{port}/v1"))
        .arg("--workspace")
        .arg(root.join("workspace"))
        .args([
            "run",
            "--output-format",
            output_format,
            "ASB_OPENJIUWEN_PROMPT: edit live-edit.txt using a tool",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .current_dir(root.join("workspace"));
    cmd
}
fn terminate_group(child: Child) -> Output {
    let group = format!("-{}", child.id());
    let terminated = Command::new("/bin/kill")
        .args(["-TERM", "--", &group])
        .status()
        .unwrap();
    assert!(
        terminated.success(),
        "failed to terminate OpenJiuwen process group"
    );
    let output = child.wait_with_output().unwrap();
    let alive = Command::new("/bin/kill")
        .args(["-0", "--", &group])
        .status()
        .unwrap();
    assert!(
        !alive.success(),
        "OpenJiuwen process group survived cancellation"
    );
    output
}
fn assert_terminal_failure(output: &Output, label: &str) {
    assert_safe_output(output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !output.status.success() || stdout.contains("TASK_FAILED"),
        "{label} lacked terminal failure: {stdout}"
    );
}
#[test]
fn scratch_roots_reject_repository_overlap_and_public_permissions() {
    assert!(validate_scratch_root(Path::new(env!("CARGO_MANIFEST_DIR"))).is_err());
    let root = scratch("permissions");
    fs::set_permissions(&root.0, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(validate_scratch_root(&root.0).is_err());
}
fn assert_safe_output(output: &Output) {
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !combined.contains(PUBLIC_SENTINEL),
        "public sentinel leaked"
    );
    assert!(
        !combined.to_ascii_lowercase().contains("authorization"),
        "authorization metadata leaked"
    );
}
#[test]
#[ignore = "requires pinned OpenJiuwen 0.1.17.post1 closure and loopback-only network namespace"]
fn pinned_openjiuwen_edits_tools_and_reports_usage() {
    let (executable, _wheel) = verify_artifacts();
    let root = scratch("success");
    fs::create_dir_all(root.0.join("home")).unwrap();
    fs::write(
        root.0.join("home/.openjiuwen.json"),
        r#"{"api_base":"https://ambient.invalid","api_key":"ambient-private"}"#,
    )
    .unwrap();
    let (_server, port, receipt, target) = start_server(&root.0, "success");
    let output = command(&executable, &root.0, port, "json")
        .output()
        .unwrap();
    assert_safe_output(&output);
    assert!(
        output.status.success(),
        "stderr: {}; receipt: {}",
        String::from_utf8_lossy(&output.stderr),
        fs::read_to_string(&receipt).unwrap_or_default()
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        fs::read_to_string(&target).unwrap_or_else(|error| panic!(
            "missing edit ({error}); receipt={}; stdout={stdout}; stderr={}",
            fs::read_to_string(&receipt).unwrap_or_default(),
            String::from_utf8_lossy(&output.stderr)
        )),
        "ASB_OPENJIUWEN_EDIT\n"
    );
    assert!(stdout.contains("ASB_OPENJIUWEN_DONE"));
    assert!(
        stdout.contains("chunks"),
        "missing structured execution evidence: {stdout}"
    );
    let value: Value = serde_json::from_str(&fs::read_to_string(receipt).unwrap()).unwrap();
    assert!(value["requests"].as_u64().unwrap() >= 2);
    assert_eq!(value["tool_result_seen"], true);
    assert_eq!(value["positive_usage_sent"], true);
    for key in [
        "loopback",
        "path",
        "authorization",
        "model",
        "stream",
        "prompt",
    ] {
        assert_eq!(value[key], true);
    }
}
#[test]
#[ignore = "requires pinned OpenJiuwen 0.1.17.post1 closure and loopback-only network namespace"]
fn pinned_openjiuwen_fails_closed_on_malformed_provider() {
    let (executable, _wheel) = verify_artifacts();
    let root = scratch("malformed");
    let (_server, port, _receipt, target) = start_server(&root.0, "malformed");
    let output = command(&executable, &root.0, port, "stream-json")
        .output()
        .unwrap();
    assert_terminal_failure(&output, "malformed stream");
    assert!(!target.exists(), "malformed provider caused an edit");
}
#[test]
#[ignore = "requires pinned OpenJiuwen 0.1.17.post1 closure and loopback-only network namespace"]
fn pinned_openjiuwen_rejects_corrupt_tool_and_retry_exhaustion() {
    let (executable, _wheel) = verify_artifacts();
    for mode in ["tool_corrupt", "http_error"] {
        let root = scratch(mode);
        let (_server, port, receipt, target) = start_server(&root.0, mode);
        let output = command(&executable, &root.0, port, "stream-json")
            .output()
            .unwrap();
        assert_terminal_failure(&output, mode);
        assert!(!target.exists(), "{mode} caused an edit");
        let value: Value = serde_json::from_str(&fs::read_to_string(receipt).unwrap()).unwrap();
        assert!(
            value["requests"].as_u64().unwrap() <= 8,
            "{mode} exceeded retry bound"
        );
    }
}
#[test]
#[ignore = "requires pinned OpenJiuwen 0.1.17.post1 closure and loopback-only network namespace"]
fn pinned_openjiuwen_cancellation_leaves_no_edit() {
    let (executable, _wheel) = verify_artifacts();
    let root = scratch("cancel");
    let (_server, port, receipt, target) = start_server(&root.0, "delay");
    let child = command(&executable, &root.0, port, "json").spawn().unwrap();
    wait_file(&receipt, Duration::from_secs(15));
    let started = Instant::now();
    let output = terminate_group(child);
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(!output.status.success());
    assert_safe_output(&output);
    assert!(!target.exists());
    thread::sleep(Duration::from_millis(300));
    assert!(!target.exists(), "cancelled request caused a delayed edit");
}
#[test]
#[ignore = "requires pinned OpenJiuwen 0.1.17.post1 closure and loopback-only network namespace"]
fn pinned_openjiuwen_cancels_trickled_output_and_reaps_group() {
    let (executable, _wheel) = verify_artifacts();
    let root = scratch("trickle");
    let (_server, port, receipt, target) = start_server(&root.0, "trickle");
    let child = command(&executable, &root.0, port, "json").spawn().unwrap();
    wait_file(&receipt, Duration::from_secs(15));
    thread::sleep(Duration::from_millis(200));
    let started = Instant::now();
    let output = terminate_group(child);
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(!output.status.success());
    assert_safe_output(&output);
    assert!(!target.exists(), "trickled response caused an edit");
}
