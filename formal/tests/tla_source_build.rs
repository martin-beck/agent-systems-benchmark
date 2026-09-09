// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Closed deterministic TLA+ source-build provenance contract.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;

const MANIFEST: &str = include_str!("../tla-provenance/source-build.toml");
const POSITIVE: &str = include_str!("../tla-provenance/fixtures/source-positive.json");
const MUTATIONS: &str = include_str!("../tla-provenance/fixtures/source-mutations.json");
const VERIFY: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tla-provenance/verify.sh");
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let serial = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("asb-tla-provenance-{}-{serial}", std::process::id()));
        fs::create_dir(&path).expect("create private test scratch");
        Self(path)
    }

    fn manifest(&self, contents: &str) -> PathBuf {
        let path = self.0.join("source-build.toml");
        fs::write(&path, contents).expect("write test manifest");
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn manifest_only(path: &Path) -> std::process::Output {
    Command::new(VERIFY)
        .args(["--manifest-only", path.to_str().expect("UTF-8 test path")])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run manifest verifier")
}

#[test]
fn exact_manifest_matches_public_build_receipt() {
    let expected: Value = serde_json::from_str(POSITIVE).expect("positive fixture JSON");
    for (key, value) in expected.as_object().expect("positive fixture object") {
        let rendered = match value {
            Value::String(text) => format!("{key} = \"{text}\""),
            Value::Number(number) => format!("{key} = {number}"),
            _ => panic!("unsupported positive fixture value"),
        };
        assert!(MANIFEST.contains(&rendered), "missing exact receipt field: {key}");
    }
    assert_eq!(MANIFEST.matches("[[dependency]]").count(), 31);
    let scratch = Scratch::new();
    let output = manifest_only(&scratch.manifest(MANIFEST));
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}

#[test]
fn every_declared_provenance_mutation_fails_closed() {
    let mutations: Value = serde_json::from_str(MUTATIONS).expect("mutation fixture JSON");
    for mutation in mutations.as_array().expect("mutation fixture array") {
        let name = mutation["name"].as_str().expect("mutation name");
        let from = mutation["from"].as_str().expect("mutation source");
        let to = mutation["to"].as_str().expect("mutation replacement");
        assert_eq!(MANIFEST.matches(from).count(), 1, "mutation selector must be unique: {name}");
        let changed = MANIFEST.replacen(from, to, 1);
        let scratch = Scratch::new();
        let output = manifest_only(&scratch.manifest(&changed));
        assert!(!output.status.success(), "mutation unexpectedly accepted: {name}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.starts_with("TLA provenance verification failed:"), "unbounded error for {name}");
        assert!(!stderr.contains("/home/"), "private path leaked for {name}");
    }
}

#[test]
fn scripts_preserve_offline_create_new_boundary() {
    let build = include_str!("../tla-provenance/build.sh");
    let verify = include_str!("../tla-provenance/verify.sh");
    for required in [
        "--network none",
        "--read-only",
        "--pull never",
        "mkdir -- \"$lock\"",
        "trap cleanup EXIT",
        "trap cancel HUP INT TERM",
        "mv -n -- \"$output.partial\" \"$output\"",
        "1980-01-01T00:00:02Z",
    ] {
        assert!(build.contains(required), "missing build boundary: {required}");
    }
    for required in ["dependency closure", "embedded license evidence", "output archive contains a symlink"] {
        assert!(verify.contains(required), "missing verifier boundary: {required}");
    }
}
