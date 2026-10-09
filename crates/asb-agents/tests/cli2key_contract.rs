// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
#![allow(missing_docs)]

//! Hosted acceptance coverage for the development-only cli2key contract.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_owned()
}

#[test]
fn credential_free_fake_proves_models_and_one_responses_exchange() {
    let root = workspace_root();
    let output = Command::new("python3")
        .arg(root.join("tools/cli2key-spike/cli2key_spike.py"))
        .arg("--fake")
        .current_dir(&root)
        .output()
        .expect("run cli2key credential-free fake");
    assert!(
        output.status.success(),
        "cli2key fake failed with a bounded diagnostic: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).expect("fake report JSON");
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["evidence_class"], "synthetic");
    assert_eq!(report["classification"], "development-only-unofficial");
    assert_eq!(report["models_status"], "passed");
    assert_eq!(report["models_count"], 1);
    assert_eq!(report["responses_status"], "passed");
    assert_eq!(report["response_shape_valid"], true);
    assert_eq!(
        report["bridge_revision"],
        "da5d271db08cca1f4666c1b035dbfb6c6f9b8f47"
    );
    let encoded = String::from_utf8(output.stdout).expect("UTF-8 report");
    for forbidden in ["Authorization", "synthetic-private-input", "oauth_token"] {
        assert!(!encoded.contains(forbidden));
    }
}

#[test]
fn contract_freezes_key_authority_and_methodology_boundaries() {
    let root = workspace_root();
    let contract: Value = serde_json::from_slice(
        &std::fs::read(root.join("config/cli2key-bridge-v1.json")).expect("read contract"),
    )
    .expect("contract JSON");
    assert_eq!(contract["bridge"]["version"], "v0.1.10");
    assert_eq!(contract["bridge"]["license"], "MIT");
    assert_eq!(
        contract["bridge"]["archive_sha256"],
        "db256f8b8b9392835fb1277cadcff6ded3da99a6831f3ff902d5f7721cc7d5bb"
    );
    assert_eq!(
        contract["credential_workflow"]["local_client_key_is_platform_api_key"],
        false
    );
    assert_eq!(
        contract["credential_workflow"]["asb_reads_codex_auth_files"],
        false
    );
    let prohibitions = contract["prohibitions"]
        .as_array()
        .expect("prohibition list")
        .iter()
        .map(|item| item.as_str().expect("prohibition string"))
        .collect::<Vec<_>>()
        .join(" ");
    assert!(prohibitions.contains("app-server"));
    assert!(prohibitions.contains("nested-agent"));
    assert!(prohibitions.contains("production readiness"));
}

#[test]
fn live_spike_requires_explicit_opt_in_without_exposing_environment() {
    let root = workspace_root();
    let output = Command::new("python3")
        .arg(root.join("tools/cli2key-spike/cli2key_spike.py"))
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .current_dir(&root)
        .output()
        .expect("run cli2key opt-in negative");
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).expect("UTF-8 diagnostic"),
        "cli2key spike failed: opt_in_required\n"
    );
}
