// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Bounded validation for the immutable PR #131 publication boundary.

use std::collections::BTreeSet;

use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

const ATTESTATION: &[u8] =
    include_bytes!("../../../docs/attestations/measurement-catalog-pr131-merge.json");
const CATALOG: &[u8] = include_bytes!("../fixtures/v1/measurement-catalog.json");
const MAX_ATTESTATION_BYTES: usize = 32 * 1024;
const BASE: &str = "58d0da27736d6c22ca7c43f76ade497165b29919";
const HEAD: &str = "78c63febc0ce6c4724bf4d14121e9ae178e2a020";
const TREE: &str = "9303272ac7070a742249722c0f9e13568c3ed660";
const MERGE: &str = "0c65159d70ee728e21c7936663a90bea49ab0366";
const FIXTURE_PATH: &str = "crates/asb-protocol/fixtures/v1/measurement-catalog.json";
const CLASSIFICATION: &str = "github_verified_tree_equivalent_merge_missing_matching_dco_trailer";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Attestation {
    schema_version: u64,
    pull_request: u64,
    reviewed_base_commit: String,
    reviewed_head_commit: String,
    reviewed_head_tree: String,
    reviewed_head_signature: String,
    reviewed_head_dco: bool,
    exact_head_checks: ExactHeadChecks,
    merge_commit: String,
    merge_parents: [String; 2],
    merge_tree: String,
    merge_signature: MergeSignature,
    merge_commit_dco: bool,
    publication_method: String,
    classification: String,
    catalog_fixture: CatalogFixture,
    remediation: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExactHeadChecks {
    expected: usize,
    successful: usize,
    checks: Vec<Check>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Check {
    name: String,
    workflow: String,
    conclusion: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MergeSignature {
    verified: bool,
    reason: String,
    signer: String,
    verified_at: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogFixture {
    path: String,
    file_sha256: String,
    catalog_sha256: String,
}

fn expected_checks() -> BTreeSet<(&'static str, &'static str)> {
    BTreeSet::from([
        (
            "AWQ v0.1.0 shadow evidence",
            "Agent Workflow Quality shadow",
        ),
        ("Bounded fuzz regressions", "Fault assurance"),
        (
            "Emulated aarch64 (x86_64 host)",
            "Emulated aarch64 portability",
        ),
        (
            "Exact Huawei 2026 and SPDX MIT headers",
            "Huawei MIT source headers",
        ),
        ("Kani bounded proofs", "Formal assurance"),
        ("Loom and state models (ubuntu-24.04)", "Formal assurance"),
        ("Matcher and SLO mutation sentinels", "Fault assurance"),
        (
            "Platform evidence (ubuntu-24.04)",
            "Hosted portability and native qualification",
        ),
        ("Policy, coverage, and supply chain", "Repository quality"),
        ("Retained faults (ubuntu-24.04)", "Fault assurance"),
        ("Rust checks (ubuntu-24.04)", "Rust verification"),
        ("TLC and Alloy recovery models", "Formal assurance"),
    ])
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn valid_hash(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate(bytes: &[u8]) -> bool {
    if bytes.len() > MAX_ATTESTATION_BYTES {
        return false;
    }
    let Ok(attestation) = serde_json::from_slice::<Attestation>(bytes) else {
        return false;
    };
    let checks: BTreeSet<_> = attestation
        .exact_head_checks
        .checks
        .iter()
        .map(|check| (check.name.as_str(), check.workflow.as_str()))
        .collect();
    let catalog: Value = match serde_json::from_slice(CATALOG) {
        Ok(value) => value,
        Err(_) => return false,
    };
    attestation.schema_version == 1
        && attestation.pull_request == 131
        && valid_hash(&attestation.reviewed_base_commit)
        && valid_hash(&attestation.reviewed_head_commit)
        && valid_hash(&attestation.reviewed_head_tree)
        && valid_hash(&attestation.merge_commit)
        && attestation
            .merge_parents
            .iter()
            .all(|parent| valid_hash(parent))
        && valid_hash(&attestation.merge_tree)
        && attestation.reviewed_base_commit == BASE
        && attestation.reviewed_head_commit == HEAD
        && attestation.reviewed_head_tree == TREE
        && attestation.reviewed_head_signature == "valid_allowed_ssh"
        && attestation.reviewed_head_dco
        && attestation.exact_head_checks.expected == 12
        && attestation.exact_head_checks.successful == 12
        && attestation.exact_head_checks.checks.len() == 12
        && attestation
            .exact_head_checks
            .checks
            .iter()
            .all(|check| check.conclusion == "success")
        && checks.len() == 12
        && checks == expected_checks()
        && attestation.merge_commit == MERGE
        && attestation.merge_parents == [BASE, HEAD]
        && attestation.merge_tree == TREE
        && attestation.merge_tree == attestation.reviewed_head_tree
        && attestation.merge_signature.verified
        && attestation.merge_signature.reason == "valid"
        && attestation.merge_signature.signer == "github_web_flow"
        && attestation.merge_signature.verified_at == "2026-09-10T23:05:45Z"
        && !attestation.merge_commit_dco
        && attestation.publication_method == "merge"
        && attestation.classification == CLASSIFICATION
        && attestation.catalog_fixture.path == FIXTURE_PATH
        && attestation.catalog_fixture.file_sha256 == sha256(CATALOG)
        && attestation.catalog_fixture.catalog_sha256
            == catalog["catalog_sha256"].as_str().unwrap_or_default()
        && attestation.remediation
            == "preserve protected history and publish this bounded attestation through a separately reviewed GitHub merge commit whose actual multiline message ends with the matching Signed-off-by trailer"
}

#[test]
fn reviewed_catalog_merge_boundary_is_exact_and_bounded() {
    assert!(validate(ATTESTATION));
}

#[test]
fn malformed_or_false_publication_claims_fail_closed() {
    let original: Value = serde_json::from_slice(ATTESTATION).unwrap();
    let mutations = [
        ("unexpected", Value::Bool(true)),
        ("reviewed_head_commit", Value::String("0".repeat(39))),
        ("merge_tree", Value::String("0".repeat(40))),
        ("merge_commit_dco", Value::Bool(true)),
        ("reviewed_head_dco", Value::Bool(false)),
        ("publication_method", Value::String("squash".into())),
    ];
    for (field, replacement) in mutations {
        let mut candidate = original.clone();
        candidate[field] = replacement;
        assert!(
            !validate(&serde_json::to_vec(&candidate).unwrap()),
            "{field}"
        );
    }
    let mut candidate = original.clone();
    candidate["merge_signature"]["verified"] = Value::Bool(false);
    assert!(!validate(&serde_json::to_vec(&candidate).unwrap()));
    let mut candidate = original.clone();
    candidate["exact_head_checks"]["checks"][0]["conclusion"] = Value::String("failure".into());
    assert!(!validate(&serde_json::to_vec(&candidate).unwrap()));
    let mut candidate = original;
    candidate["catalog_fixture"]["file_sha256"] = Value::String("0".repeat(64));
    assert!(!validate(&serde_json::to_vec(&candidate).unwrap()));
    assert!(!validate(&vec![b' '; MAX_ATTESTATION_BYTES + 1]));
}
