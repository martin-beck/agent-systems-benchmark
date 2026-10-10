// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Disposable fresh-user project qualification for the public executable.

use serde_json::Value;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NONCE: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "asb-project-qualification-{}-{}",
            std::process::id(),
            NONCE.fetch_add(1, Ordering::Relaxed)
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

struct FreshUser {
    executable: PathBuf,
    home: PathBuf,
    path: PathBuf,
}

impl FreshUser {
    fn install(scratch: &std::path::Path, system_bin: &std::path::Path) -> Self {
        let home = scratch.join("home");
        let user_bin = home.join(".local/bin");
        fs::create_dir_all(&user_bin).unwrap();
        let executable = user_bin.join("asb");
        fs::copy(env!("CARGO_BIN_EXE_asb"), &executable).unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(executable.is_file());
        let path = std::env::join_paths([&user_bin, system_bin]).unwrap();
        Self {
            executable,
            home,
            path: PathBuf::from(path),
        }
    }

    fn json(&self, args: &[String]) -> (std::process::ExitStatus, Value, Vec<u8>) {
        let output = Command::new(&self.executable)
            .env_clear()
            .env("HOME", &self.home)
            .env("XDG_CONFIG_HOME", self.home.join(".config"))
            .env("XDG_CACHE_HOME", self.home.join(".cache"))
            .env("XDG_DATA_HOME", self.home.join(".local/share"))
            .env("LANG", "C")
            .env("LC_ALL", "C")
            .env("PATH", &self.path)
            .arg("--json")
            .args(args)
            .output()
            .unwrap();
        let value = serde_json::from_slice(&output.stdout).unwrap();
        (output.status, value, output.stderr)
    }
}

#[test]
fn disposable_fresh_user_five_tool_receipt_is_json_silent_and_recovers_safely() {
    let scratch = Scratch::new();
    let system_bin = scratch.0.join("system-bin");
    fs::create_dir_all(&system_bin).unwrap();
    let system_cargo = system_bin.join("cargo");
    fs::write(&system_cargo, "#!/bin/sh\necho cargo 1.0.0\n").unwrap();
    fs::set_permissions(&system_cargo, fs::Permissions::from_mode(0o755)).unwrap();
    let user = FreshUser::install(&scratch.0, &system_bin);
    let project = scratch.0.join("project");
    let project_text = project.display().to_string();
    let (status, init, stderr) =
        user.json(&["project".into(), "init".into(), project_text.clone()]);
    assert!(status.success());
    assert!(stderr.is_empty());
    assert_eq!(init["command"], "project init");

    for (id, kind) in [
        ("agent-fixture", "agent"),
        ("harness-fixture", "harness"),
        ("benchmark-fixture", "benchmark"),
        ("workload-fixture", "workload"),
        ("support-fixture", "support"),
    ] {
        let (status, installed, stderr) = user.json(&[
            "tool".into(),
            "install".into(),
            id.into(),
            "--kind".into(),
            kind.into(),
            "--source".into(),
            format!("fixture://{id}"),
            "--version".into(),
            "1.0.0".into(),
            "--project".into(),
            project_text.clone(),
        ]);
        assert!(status.success());
        assert!(stderr.is_empty());
        assert_eq!(installed["id"], id);
        if kind == "support" {
            continue;
        }
        let (status, selected, stderr) = user.json(&[
            "tool".into(),
            "select".into(),
            id.into(),
            "--kind".into(),
            kind.into(),
            "--project".into(),
            project_text.clone(),
        ]);
        assert!(status.success());
        assert!(stderr.is_empty());
        assert_eq!(selected["id"], id);
    }

    let (status, discovery, stderr) =
        user.json(&["tool".into(), "discover".into(), project_text.clone()]);
    assert!(status.success());
    assert!(stderr.is_empty());
    for id in [
        "agent-fixture",
        "harness-fixture",
        "benchmark-fixture",
        "workload-fixture",
        "support-fixture",
    ] {
        assert!(
            discovery["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["name"] == id)
        );
    }

    let (status, system_discovery, stderr) =
        user.json(&["tool".into(), "discover".into(), project_text.clone()]);
    assert!(status.success());
    assert!(stderr.is_empty());
    assert!(
        system_discovery["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| {
                tool["name"] == "cargo" && tool["source"] == "path" && tool["status"] == "available"
            })
    );

    let (status, generated, stderr) = user.json(&[
        "catalog".into(),
        "generate".into(),
        "--project".into(),
        project_text.clone(),
    ]);
    assert!(status.success());
    assert!(stderr.is_empty());
    for catalog in generated["catalogs"].as_array().unwrap() {
        let id = catalog["id"].as_str().unwrap();
        let kind = catalog["kind"].as_str().unwrap();
        let digest = catalog["digest_sha256"].as_str().unwrap();
        let (status, selected, stderr) = user.json(&[
            "catalog".into(),
            "select".into(),
            id.into(),
            "--kind".into(),
            kind.into(),
            "--digest-sha256".into(),
            digest.into(),
            "--project".into(),
            project_text.clone(),
        ]);
        assert!(status.success());
        assert!(stderr.is_empty());
        assert_eq!(selected["active"], true);
    }

    let (status, missing, stderr) = user.json(&[
        "tool".into(),
        "discover".into(),
        scratch.0.join("missing").display().to_string(),
    ]);
    assert!(!status.success());
    assert!(stderr.is_empty());
    assert!(missing["error"].is_object());
    assert!(!project.join("results/runs").exists());

    let plan = scratch.0.join("plan.toml");
    let result_root = project.join("results");
    let work_root = scratch.0.join("work");
    let (status, created, stderr) = user.json(&[
        "plan".into(),
        "create".into(),
        "--workload".into(),
        "original.bug-fix".into(),
        "--agent-executable".into(),
        "/usr/bin/true".into(),
        "--output".into(),
        plan.display().to_string(),
        "--result-root".into(),
        result_root.display().to_string(),
        "--work-root".into(),
        work_root.display().to_string(),
    ]);
    assert!(status.success());
    assert!(stderr.is_empty());
    assert_eq!(created["command"], "plan-create");

    let (status, run, stderr) = user.json(&[
        "run".into(),
        plan.display().to_string(),
        "--project".into(),
        project_text,
        "--local-mock".into(),
    ]);
    assert!(status.success());
    assert!(stderr.is_empty());
    assert_eq!(run["command"], "run");
    assert!(result_root.join("runs").is_dir());

    let run_id = run["run_ids"][0].as_str().unwrap();
    let (status, report, stderr) = user.json(&[
        "report".into(),
        result_root.join("runs").join(run_id).display().to_string(),
    ]);
    assert!(status.success());
    assert!(stderr.is_empty());
    assert_eq!(report["command"], "report");
}
