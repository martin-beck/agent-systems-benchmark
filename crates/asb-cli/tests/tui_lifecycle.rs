// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Process-boundary lifecycle-router checks. The ASB binary never renders a TUI.

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static NONCE: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "asb-tui-router-e2e-{}-{}",
            std::process::id(),
            NONCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&path)
            .unwrap();
        Self(fs::canonicalize(path).unwrap())
    }

    fn command(&self, operation: &str) -> Output {
        Command::new(env!("CARGO_BIN_EXE_asb"))
            .args(["tui", operation])
            .env_clear()
            .env("HOME", self.0.join("home"))
            .env("XDG_DATA_HOME", self.0.join("data"))
            .env("XDG_STATE_HOME", self.0.join("state"))
            .env("XDG_CACHE_HOME", self.0.join("cache"))
            .output()
            .unwrap()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn response(output: &Output) -> Value {
    assert!(output.stderr.is_empty());
    serde_json::from_slice(&output.stdout).unwrap()
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[test]
fn absent_local_lifecycle_is_stable_network_free_and_non_creating() {
    let scratch = Scratch::new();
    for (operation, exit, ok) in [
        ("status", 3, false),
        ("doctor", 0, true),
        ("remove", 3, false),
        ("remove", 3, false),
        ("launch", 3, false),
    ] {
        let output = scratch.command(operation);
        assert_eq!(output.status.code(), Some(exit));
        let value = response(&output);
        assert_eq!(value["operation"], operation);
        assert_eq!(value["ok"], ok);
        assert_eq!(value["code"], "extension_not_installed");
        assert_eq!(value["network"], "denied");
    }
    for root in ["data", "state", "cache", "home"] {
        assert!(!Path::new(&scratch.0).join(root).exists());
    }
}

#[test]
fn router_version_and_help_are_explicit_and_non_interactive() {
    let version = Command::new(env!("CARGO_BIN_EXE_asb"))
        .args(["tui", "--version"])
        .output()
        .unwrap();
    assert!(version.status.success());
    assert_eq!(
        String::from_utf8(version.stdout).unwrap(),
        format!("asb tui-router {}\n", env!("CARGO_PKG_VERSION"))
    );
    let help = Command::new(env!("CARGO_BIN_EXE_asb"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(help.status.success());
    let text = String::from_utf8(help.stdout).unwrap();
    assert!(text.contains("asb tui install [--offline] [--dry-run] [--launch]"));
    assert!(text.contains("asb tui status|doctor|remove"));
}

#[cfg(target_arch = "x86_64")]
#[test]
fn launch_keeps_lifecycle_json_separate_from_controlling_terminal() {
    let scratch = Scratch::new();
    let install = scratch.0.join("data/asb/extensions/asb-tui");
    let state = scratch.0.join("state/asb/extensions/asb-tui");
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&install)
        .unwrap();
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&state)
        .unwrap();
    let candidate = include_bytes!("../fixtures/tui/interactive-candidate.sh");
    let channel = include_bytes!("../fixtures/tui/authenticated-active-channel.json");
    let channel_signature = include_bytes!("../fixtures/tui/authenticated-active-channel.json.sig");
    let manifest = include_bytes!("../fixtures/tui/authenticated-active-manifest.json");
    let signature = include_bytes!("../fixtures/tui/authenticated-active-manifest.json.sig");
    let executable_sha256 = digest(candidate);
    let manifest_sha256 = digest(manifest);
    let executable = install
        .join("versions")
        .join(&executable_sha256)
        .join("asb-tui");
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(executable.parent().unwrap())
        .unwrap();
    fs::write(&executable, candidate).unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(
        install.join("active.json"),
        serde_json::to_vec(&serde_json::json!({
            "schema_version":1,
            "release":"v0.0.1",
            "executable_sha256":executable_sha256,
            "source_commit":"1".repeat(40),
            "source_tree":"2".repeat(40),
            "target":format!("{}-unknown-linux-gnu", std::env::consts::ARCH),
            "bundle":format!("asb-tui-v1-linux-{}", std::env::consts::ARCH),
            "asb_version":"0.1.0",
            "protocol_version":1,
            "coordinator_version":"v0.3.5",
            "coordinator_commit":"510817b93feb80dde13e5a6c61d657954fae2346",
            "quality_version":"v0.23.0",
            "quality_commit":"8a9f056b7fc7926b9465a0f7a09225d4da1c572a",
            "classification":"verified_extension"
        }))
        .unwrap(),
    )
    .unwrap();
    let authenticated = state.join("manifests").join(&manifest_sha256);
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&authenticated)
        .unwrap();
    fs::write(authenticated.join("channel.json"), channel).unwrap();
    fs::write(authenticated.join("channel.json.sig"), channel_signature).unwrap();
    fs::write(authenticated.join("manifest.json"), manifest).unwrap();
    fs::write(authenticated.join("manifest.json.sig"), signature).unwrap();
    fs::write(
        state.join("accepted.json"),
        serde_json::to_vec(&serde_json::json!({
            "schema_version":1,
            "release":"v0.0.1",
            "manifest_sha256":manifest_sha256
        }))
        .unwrap(),
    )
    .unwrap();
    let runner_sentinel = scratch.0.join("runner-owned-state");
    fs::write(&runner_sentinel, b"runner-owns-this\n").unwrap();

    let command = format!("{} tui launch", env!("CARGO_BIN_EXE_asb"));
    let mut child = Command::new("/usr/bin/script")
        .args(["-q", "-e", "-c", &command, "/dev/null"])
        .env("HOME", scratch.0.join("home"))
        .env("XDG_DATA_HOME", scratch.0.join("data"))
        .env("XDG_STATE_HOME", scratch.0.join("state"))
        .env("XDG_CACHE_HOME", scratch.0.join("cache"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"q\n").unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let terminal = String::from_utf8(output.stdout).unwrap();
    assert!(terminal.contains("ASB_TUI_RENDERED"));
    let routed = terminal
        .lines()
        .find_map(|line| serde_json::from_str::<Value>(line.trim_end_matches(char::from(13))).ok())
        .unwrap();
    assert_eq!(routed["code"], "frontend_exited");
    assert_eq!(routed["network"], "denied");
    assert_eq!(fs::read(&runner_sentinel).unwrap(), b"runner-owns-this\n");

    fs::write(authenticated.join("channel.json"), b"{}").unwrap();
    let rejected = scratch.command("launch");
    assert_eq!(rejected.status.code(), Some(3));
    assert_eq!(
        response(&rejected)["code"],
        "installation_verification_failed"
    );
    fs::write(authenticated.join("channel.json"), channel).unwrap();

    let marker = scratch.0.join("unverified-executed");
    let hostile = format!("#!/bin/sh\ntouch {}\n", marker.display());
    let hostile_sha256 = digest(hostile.as_bytes());
    let hostile_path = install
        .join("versions")
        .join(&hostile_sha256)
        .join("asb-tui");
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(hostile_path.parent().unwrap())
        .unwrap();
    fs::write(&hostile_path, hostile).unwrap();
    fs::set_permissions(&hostile_path, fs::Permissions::from_mode(0o700)).unwrap();
    let mut substituted: Value =
        serde_json::from_slice(&fs::read(install.join("active.json")).unwrap()).unwrap();
    substituted["executable_sha256"] = hostile_sha256.into();
    fs::write(
        install.join("active.json"),
        serde_json::to_vec(&substituted).unwrap(),
    )
    .unwrap();
    let rejected = scratch.command("launch");
    assert_eq!(rejected.status.code(), Some(3));
    assert_eq!(
        response(&rejected)["code"],
        "installation_verification_failed"
    );
    assert!(!marker.exists());
}

#[test]
fn copied_lifecycle_and_channel_schemas_reject_widening() {
    let response_schema: Value = serde_json::from_str(include_str!(
        "../schema/tui/v1/lifecycle-response.schema.json"
    ))
    .unwrap();
    let response_validator = jsonschema::validator_for(&response_schema).unwrap();
    let absent = serde_json::json!({
        "schema_version":1,"classification":"source_only_unverified",
        "ok":false,"code":"extension_not_installed",
        "installed":false,"verified":false
    });
    assert!(response_validator.is_valid(&absent));
    let mut widened = absent.clone();
    widened["terminal_output"] = "not lifecycle JSON".into();
    assert!(!response_validator.is_valid(&widened));
    let incomplete_installed = serde_json::json!({
        "schema_version":1,"classification":"source_only_unverified",
        "ok":false,"code":"installation_verification_failed",
        "installed":true,"verified":false
    });
    assert!(!response_validator.is_valid(&incomplete_installed));

    let channel_schema: Value =
        serde_json::from_str(include_str!("../schema/tui/v1/channel-index.schema.json")).unwrap();
    let channel_validator = jsonschema::validator_for(&channel_schema).unwrap();
    let mut channel = serde_json::json!({
        "schema_version":1,"issued_unix":1,"expires_unix":2,
        "releases":[{
            "release":"v1.2.3","target":"x86_64-unknown-linux-gnu",
            "asb_version":"0.1.0","protocol_version":1,
            "manifest_url":"https://github.com/martin-beck/asb-tui/releases/download/v1.2.3/manifest.json",
            "manifest_signature_url":"https://github.com/martin-beck/asb-tui/releases/download/v1.2.3/manifest.json.sig",
            "manifest_sha256":"a".repeat(64)
        }]
    });
    assert!(channel_validator.is_valid(&channel));
    channel["releases"][0]["manifest_url"] =
        "https://github.com/martin-beck/asb-tui/releases/latest/manifest.json".into();
    assert!(!channel_validator.is_valid(&channel));
}
