// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
#![cfg(target_os = "linux")]
#![allow(missing_docs)]
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::fs;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::thread;
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
const WHEEL_SHA256: &str = "21e9479c6b858cda28c250d63066f862fc0915cf2039edb00f016cbec7f9abba";
const INTERPRETER_SHA256: &str = "1643dacd9feaedc58f3cc581e4d22577dfe25c09b10282936186ccf0f2e61118";
const PUBLIC_SENTINEL: &str = "asb-loopback-public-sentinel";
const PRIVATE_SENTINELS: [&str; 5] = [
    "asb-ambient-private",
    "asb-config-private",
    "asb-scratch-private",
    "asb-private-user",
    "asb-private-machine",
];
const MAX_CAPTURE_BYTES: usize = 1024 * 1024;
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
struct BoundedChild {
    child: Child,
    stdout: JoinHandle<Vec<u8>>,
    stderr: JoinHandle<Vec<u8>>,
}
impl BoundedChild {
    fn wait(mut self) -> Output {
        let status = self.child.wait().unwrap();
        bounded_output(status, self.stdout, self.stderr)
    }
}
fn capture(stream: impl Read + Send + 'static) -> JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        stream
            .take((MAX_CAPTURE_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .unwrap();
        bytes
    })
}
fn bounded_output(
    status: ExitStatus,
    stdout: JoinHandle<Vec<u8>>,
    stderr: JoinHandle<Vec<u8>>,
) -> Output {
    let stdout = stdout.join().unwrap();
    let stderr = stderr.join().unwrap();
    assert!(stdout.len() <= MAX_CAPTURE_BYTES, "stdout exceeded bound");
    assert!(stderr.len() <= MAX_CAPTURE_BYTES, "stderr exceeded bound");
    Output {
        status,
        stdout,
        stderr,
    }
}
fn spawn_bounded(command: &mut Command) -> BoundedChild {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    let stdout = capture(child.stdout.take().unwrap());
    let stderr = capture(child.stderr.take().unwrap());
    BoundedChild {
        child,
        stdout,
        stderr,
    }
}
fn run_bounded(command: &mut Command) -> Output {
    spawn_bounded(command).wait()
}
#[test]
#[should_panic(expected = "stdout exceeded bound")]
fn bounded_capture_rejects_oversized_output() {
    let _ = run_bounded(
        Command::new("/usr/bin/head")
            .args(["-c", "1048577", "/dev/zero"])
            .env_clear()
            .stdin(Stdio::null()),
    );
}
fn scratch_root(configured: Option<OsString>) -> PathBuf {
    configured
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}
fn scratch_at(root: PathBuf, label: &str) -> Scratch {
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
fn scratch(label: &str) -> Scratch {
    scratch_at(scratch_root(std::env::var_os("CARGO_TARGET_DIR")), label)
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
fn verify_artifacts_at(executable: &Path, wheel: &Path) -> Result<PathBuf, &'static str> {
    if !executable.is_absolute()
        || !wheel.is_absolute()
        || executable.is_symlink()
        || wheel.is_symlink()
        || !executable.is_file()
        || !wheel.is_file()
    {
        return Err("artifact path is not an absolute regular non-symlink file");
    }
    if format!(
        "{:x}",
        Sha256::digest(fs::read(wheel).map_err(|_| "wheel unreadable")?)
    ) != WHEEL_SHA256
    {
        return Err("wheel digest mismatch");
    }
    let runtime = executable
        .parent()
        .and_then(Path::parent)
        .ok_or("executable is outside a runtime root")?;
    if runtime.join("bin/openjiuwen") != executable || runtime.is_symlink() {
        return Err("entrypoint is outside the runtime root");
    }
    let interpreter = runtime.join("bin/python");
    if fs::read_link(&interpreter).ok().as_deref() != Some(Path::new("/usr/bin/python3.12"))
        || !interpreter.is_file()
        || format!(
            "{:x}",
            Sha256::digest(fs::read(&interpreter).map_err(|_| "interpreter unreadable")?)
        ) != INTERPRETER_SHA256
    {
        return Err("runtime interpreter identity mismatch");
    }
    let bytes = fs::read(executable).map_err(|_| "entrypoint unreadable")?;
    let shebang = format!("#!{}\n", interpreter.display());
    if !bytes.starts_with(shebang.as_bytes())
        || !bytes
            .windows(b"from openjiuwen.harness.cli.cli import cli".len())
            .any(|part| part == b"from openjiuwen.harness.cli.cli import cli")
    {
        return Err("entrypoint shebang or import binding mismatch");
    }
    let verifier = r#"
import base64,csv,hashlib,importlib.metadata,json,pathlib,re,sys,zipfile
root=pathlib.Path(sys.argv[1]).resolve(); wheel=pathlib.Path(sys.argv[2]).resolve(); lock=pathlib.Path(sys.argv[3])
if pathlib.Path(sys.prefix).resolve()!=root: raise SystemExit('runtime root mismatch')
site=root/'lib/python3.12/site-packages'; dist=site/'openjiuwen-0.1.17.post1.dist-info'
if not dist.is_dir(): raise SystemExit('distribution identity missing')
ep=(dist/'entry_points.txt').read_text()
if 'openjiuwen = openjiuwen.harness.cli.cli:cli' not in ep: raise SystemExit('entrypoint metadata mismatch')
expected={}
for line in lock.read_text().splitlines():
  match=re.fullmatch(r'([a-z0-9_.-]+)==([^ ]+) \\',line)
  if match: expected[re.sub(r'[-_.]+','-',match[1]).lower()]=match[2]
installed_versions={re.sub(r'[-_.]+','-',item.metadata['Name']).lower():item.version for item in importlib.metadata.distributions(path=[str(site)])}
if len(expected)!=170 or installed_versions!=expected: raise SystemExit('closed runtime inventory mismatch')
checked=0
for item in importlib.metadata.distributions(path=[str(site)]):
  record=pathlib.Path(item._path)/'RECORD'
  if not record.is_file(): raise SystemExit('package RECORD missing')
  with record.open(newline='') as f:
    for name,digest,size in csv.reader(f):
      if not digest: continue
      path=(site/name).resolve()
      if root not in path.parents or not path.is_file(): raise SystemExit('RECORD path mismatch')
      alg,value=digest.split('=',1)
      if alg!='sha256': raise SystemExit('RECORD algorithm mismatch')
      actual=base64.urlsafe_b64encode(hashlib.sha256(path.read_bytes()).digest()).rstrip(b'=').decode()
      if actual!=value or path.stat().st_size!=int(size): raise SystemExit('installed RECORD mismatch')
      checked+=1
with zipfile.ZipFile(wheel) as z:
  rows=list(csv.reader(z.read('openjiuwen-0.1.17.post1.dist-info/RECORD').decode().splitlines()))
  for name,digest,size in rows:
    if not digest or name.endswith('.dist-info/RECORD'): continue
    installed_path=(site/name).resolve()
    if not installed_path.is_file(): raise SystemExit('wheel member absent from runtime')
    alg,value=digest.split('=',1)
    actual=base64.urlsafe_b64encode(hashlib.sha256(installed_path.read_bytes()).digest()).rstrip(b'=').decode()
    if alg!='sha256' or actual!=value or installed_path.stat().st_size!=int(size): raise SystemExit('wheel/runtime identity mismatch')
print(json.dumps({'packages':len(installed_versions),'record_files':checked,'wheel_members':len(rows),'version':'0.1.17.post1'}))
"#;
    let lock =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/openjiuwen-runtime.lock");
    let output = run_bounded(
        Command::new(&interpreter)
            .args(["-I", "-c", verifier])
            .arg(runtime)
            .arg(wheel)
            .arg(lock)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .stdin(Stdio::null()),
    );
    if !output.status.success() {
        return Err("runtime RECORD attestation failed");
    }
    let receipt: Value =
        serde_json::from_slice(&output.stdout).map_err(|_| "attestation receipt malformed")?;
    if receipt["version"] != "0.1.17.post1"
        || receipt["packages"] != 170
        || receipt["record_files"].as_u64().unwrap_or(0) < 2_000
        || receipt["wheel_members"].as_u64().unwrap_or(0) < 2_000
    {
        return Err("attestation receipt incomplete");
    }
    Ok(runtime.to_path_buf())
}
fn verify_artifacts() -> (PathBuf, PathBuf) {
    assert_eq!(
        std::env::var("ASB_OPENJIUWEN_LOOPBACK_ONLY").as_deref(),
        Ok("1")
    );
    let executable =
        PathBuf::from(std::env::var_os("ASB_OPENJIUWEN_EXECUTABLE").expect("pinned executable"));
    let wheel = PathBuf::from(std::env::var_os("ASB_OPENJIUWEN_WHEEL").expect("pinned wheel"));
    verify_artifacts_at(&executable, &wheel).unwrap();
    (executable, wheel)
}
#[test]
fn artifact_attestation_rejects_arbitrary_executables() {
    assert!(verify_artifacts_at(Path::new("/bin/true"), Path::new("/bin/true")).is_err());
}
#[test]
#[ignore = "requires pinned OpenJiuwen 0.1.17.post1 wheel"]
fn artifact_attestation_rejects_mismatched_entrypoint_with_valid_wheel() {
    let wheel =
        PathBuf::from(std::env::var_os("ASB_OPENJIUWEN_WHEEL").expect("pinned OpenJiuwen wheel"));
    assert!(verify_artifacts_at(Path::new("/bin/true"), &wheel).is_err());
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
    let mut cmd = Command::new("/usr/bin/bwrap");
    let home = root.join("home");
    fs::create_dir_all(&home).unwrap();
    fs::write(
        home.join(".openjiuwen.json"),
        format!(
            r#"{{"api_base":"https://{}.invalid","api_key":"{}","user":"{}","machine":"{}"}}"#,
            PRIVATE_SENTINELS[0], PRIVATE_SENTINELS[1], PRIVATE_SENTINELS[3], PRIVATE_SENTINELS[4]
        ),
    )
    .unwrap();
    fs::write(root.join(PRIVATE_SENTINELS[2]), b"private\n").unwrap();
    cmd.args(["--unshare-pid", "--die-with-parent", "--ro-bind", "/", "/"])
        .arg("--bind")
        .arg(root)
        .arg(root)
        .args(["--proc", "/proc", "--dev", "/dev", "--"])
        .arg(executable)
        .env_clear()
        .current_dir(root.join("workspace"))
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", root.join("xdg-config"))
        .env("XDG_CACHE_HOME", root.join("xdg-cache"))
        .env("PATH", "/usr/bin:/bin")
        .env("ASB_AMBIENT_SECRET", PRIVATE_SENTINELS[0])
        .env("USER", PRIVATE_SENTINELS[3])
        .env("HOSTNAME", PRIVATE_SENTINELS[4])
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
fn terminate_group(mut child: BoundedChild) -> Output {
    let group = format!("-{}", child.child.id());
    let terminated = Command::new("/bin/kill")
        .args(["-TERM", "--", &group])
        .status()
        .unwrap();
    assert!(
        terminated.success(),
        "failed to terminate OpenJiuwen process group"
    );
    let term_deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < term_deadline
        && Command::new("/bin/kill")
            .args(["-0", "--", &group])
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success()
    {
        thread::sleep(Duration::from_millis(20));
    }
    if Command::new("/bin/kill")
        .args(["-0", "--", &group])
        .stderr(Stdio::null())
        .status()
        .unwrap()
        .success()
    {
        assert!(
            Command::new("/bin/kill")
                .args(["-KILL", "--", &group])
                .status()
                .unwrap()
                .success(),
            "failed to force termination after grace period"
        );
    }
    let status = child.child.wait().unwrap();
    let reap_deadline = Instant::now() + Duration::from_secs(2);
    let mut alive = true;
    while alive && Instant::now() < reap_deadline {
        alive = Command::new("/bin/kill")
            .args(["-0", "--", &group])
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success();
        if alive {
            thread::sleep(Duration::from_millis(20));
        }
    }
    assert!(!alive, "OpenJiuwen process group survived cancellation");
    bounded_output(status, child.stdout, child.stderr)
}
fn assert_terminal_failure(output: &Output, root: &Path, label: &str) {
    assert_safe_output(output, root);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !output.status.success() || stdout.contains("TASK_FAILED"),
        "{label} lacked terminal failure: {stdout}"
    );
}
#[test]
fn scratch_roots_reject_repository_overlap_and_public_permissions() {
    assert!(validate_scratch_root(Path::new(env!("CARGO_MANIFEST_DIR"))).is_err());
    let fallback_root = scratch_root(None);
    assert_eq!(fallback_root, std::env::temp_dir());
    let fallback = scratch_at(fallback_root, "fallback");
    assert!(fallback.0.starts_with(std::env::temp_dir()));

    let configured_root = std::env::temp_dir().join(format!(
        "asb-explicit-target-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&configured_root).unwrap();
    fs::set_permissions(&configured_root, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(
        scratch_root(Some(configured_root.clone().into_os_string())),
        configured_root
    );
    let configured = scratch_at(configured_root.clone(), "configured");
    assert!(configured.0.starts_with(&configured_root));
    drop(configured);
    fs::remove_dir_all(&configured_root).unwrap();

    let root = scratch_at(std::env::temp_dir(), "permissions");
    fs::set_permissions(&root.0, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(validate_scratch_root(&root.0).is_err());
}
fn assert_safe_output(output: &Output, root: &Path) {
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
    for sentinel in PRIVATE_SENTINELS {
        assert!(!combined.contains(sentinel), "private sentinel leaked");
    }
    assert!(
        !combined.contains(&root.display().to_string()),
        "private scratch path leaked"
    );
}
#[test]
#[ignore = "requires pinned OpenJiuwen 0.1.17.post1 closure and loopback-only network namespace"]
fn pinned_openjiuwen_edits_tools_and_reports_usage() {
    let (executable, _wheel) = verify_artifacts();
    let root = scratch("success");
    let (_server, port, receipt, target) = start_server(&root.0, "success");
    let output = run_bounded(&mut command(&executable, &root.0, port, "json"));
    assert_safe_output(&output, &root.0);
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
fn pinned_openjiuwen_emits_agent_parsed_usage() {
    let (executable, _wheel) = verify_artifacts();
    let root = scratch("usage");
    let (_server, port, receipt, _target) = start_server(&root.0, "usage");
    let output = run_bounded(&mut command(&executable, &root.0, port, "stream-json"));
    assert_safe_output(&output, &root.0);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut parsed_usage = None;
    for line in String::from_utf8(output.stdout).unwrap().lines() {
        let item: Value = serde_json::from_str(line).unwrap();
        if item["type"] == "llm_usage" {
            parsed_usage = Some(item["payload"].clone());
        }
    }
    let usage = parsed_usage.expect("agent emitted no parsed usage chunk");
    let parsed = &usage["usage_metadata"];
    assert_eq!(parsed["input_tokens"], 101);
    assert_eq!(parsed["output_tokens"], 7);
    assert_eq!(parsed["total_tokens"], 108);
    let fixture: Value = serde_json::from_str(&fs::read_to_string(receipt).unwrap()).unwrap();
    assert_eq!(fixture["positive_usage_sent"], true);
}
#[test]
#[ignore = "requires pinned OpenJiuwen 0.1.17.post1 closure and loopback-only network namespace"]
fn pinned_openjiuwen_fails_closed_on_malformed_provider() {
    let (executable, _wheel) = verify_artifacts();
    let root = scratch("malformed");
    let (_server, port, _receipt, target) = start_server(&root.0, "malformed");
    let output = run_bounded(&mut command(&executable, &root.0, port, "stream-json"));
    assert_terminal_failure(&output, &root.0, "malformed stream");
    assert!(!target.exists(), "malformed provider caused an edit");
}
#[test]
#[ignore = "requires pinned OpenJiuwen 0.1.17.post1 closure and loopback-only network namespace"]
fn pinned_openjiuwen_rejects_corrupt_tool_and_retry_exhaustion() {
    let (executable, _wheel) = verify_artifacts();
    for mode in ["tool_corrupt", "http_error"] {
        let root = scratch(mode);
        let (_server, port, receipt, target) = start_server(&root.0, mode);
        let output = run_bounded(&mut command(&executable, &root.0, port, "stream-json"));
        assert_terminal_failure(&output, &root.0, mode);
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
    let child = spawn_bounded(&mut command(&executable, &root.0, port, "json"));
    wait_file(&receipt, Duration::from_secs(15));
    let started = Instant::now();
    let output = terminate_group(child);
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(!output.status.success());
    assert_safe_output(&output, &root.0);
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
    let child = spawn_bounded(&mut command(&executable, &root.0, port, "json"));
    wait_file(&receipt, Duration::from_secs(15));
    thread::sleep(Duration::from_millis(200));
    let started = Instant::now();
    let output = terminate_group(child);
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(!output.status.success());
    assert_safe_output(&output, &root.0);
    assert!(!target.exists(), "trickled response caused an edit");
}
#[test]
#[ignore = "requires pinned OpenJiuwen 0.1.17.post1 closure and loopback-only network namespace"]
fn pinned_openjiuwen_cancellation_contains_setsid_child() {
    let (executable, _wheel) = verify_artifacts();
    let root = scratch("child-escape");
    let (_server, port, receipt, target) = start_server(&root.0, "child_escape");
    let child = spawn_bounded(&mut command(&executable, &root.0, port, "json"));
    wait_file(&receipt, Duration::from_secs(15));
    wait_file(&target.with_extension("txt.pid"), Duration::from_secs(15));
    let output = terminate_group(child);
    assert!(!output.status.success());
    assert_safe_output(&output, &root.0);
    thread::sleep(Duration::from_secs(3));
    assert!(
        !target.with_extension("txt.escape").exists(),
        "setsid child escaped the cancellation boundary"
    );
    assert!(!target.exists(), "escape scenario caused the graded edit");
}
