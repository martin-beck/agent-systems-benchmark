// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Credential-free, non-production package-journey qualification.
//!
//! The signed-package verifier fixture lives in `asb-bundle`'s offline
//! verifier tests. This process-boundary test proves that the resulting
//! qualification journey starts from an isolated owner-only environment.

use serde_json::Value;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NONCE: AtomicU64 = AtomicU64::new(0);
use std::path::{Path, PathBuf};

struct TempRoot(PathBuf);

impl TempRoot {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "asb-package-qualification-{}-{}",
            std::process::id(),
            NONCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).expect("create qualification root");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).expect("protect root");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn invoke(root: &TempRoot, args: &[&str]) -> (bool, Value) {
    let home = root.path().join("home");
    let config = root.path().join("config");
    let data = root.path().join("data");
    let state = root.path().join("state");
    let cache = root.path().join("cache");
    for path in [&home, &config, &data, &state, &cache] {
        fs::create_dir_all(path).expect("create isolated XDG root");
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).expect("protect XDG root");
    }
    let output = Command::new(env!("CARGO_BIN_EXE_asb"))
        .env_clear()
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", &config)
        .env("XDG_DATA_HOME", &data)
        .env("XDG_STATE_HOME", &state)
        .env("XDG_CACHE_HOME", &cache)
        .env("LANG", "C")
        .env("LC_ALL", "C")
        .arg("--json")
        .args(args)
        .output()
        .expect("run asb qualification command");
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {:?}",
        output.stderr
    );
    let value: Value = serde_json::from_slice(&output.stdout).expect("structured JSON output");
    (output.status.success(), value)
}

#[test]
fn non_production_fixture_starts_owner_only_cli_journey() {
    let root = TempRoot::new();
    let (doctor_ok, doctor) = invoke(&root, &["doctor"]);
    assert!(doctor_ok);
    assert_eq!(doctor["command"], "doctor");
    let (setup_ok, setup) = invoke(&root, &["setup", "--format=json"]);
    assert!(setup_ok);
    assert_eq!(setup["command"], "setup");
    assert!(setup.is_object());
    assert!(!setup.to_string().contains("credential"));
}

#[test]
fn production_release_claim_is_not_made_by_fixture() {
    let guide = include_str!("../../../docs/workflows/self-contained-package-qualification.md");
    assert!(guide.contains("non-production qualification"));
    assert!(guide.contains("never release") && guide.contains("evidence"));
    assert!(guide.contains("signed package"));
}
