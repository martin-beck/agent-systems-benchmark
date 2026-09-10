// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Bounded validation for the immutable catalog publication recovery boundaries.

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
const RECOVERY_HEAD: &str = "607a3afb3a44b87f9c60b6ae3bc764570e84d5fe";
const RECOVERY_TREE: &str = "b45e1a64545cf40fa3c181e255fb3051c71e6858";
const RECOVERY_MERGE: &str = "a01f7f21f5be07dda7f18185724122c56dff1bb7";
const RECOVERY_CLASSIFICATION: &str =
    "github_verified_tree_equivalent_merge_nonmatching_dco_author_name";
const RECOVERY_AUTHOR: &str = "martin-beck";
const RECOVERY_EMAIL: &str = "martin.beck2@gmx.de";
const RECOVERY_RAW_TRAILER: &str = "Signed-off-by: Martin Beck <martin.beck2@gmx.de>";
const REQUIRED_TRAILER: &str = "Signed-off-by: martin-beck <martin.beck2@gmx.de>";

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
    first_recovery: RecoveryBoundary,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecoveryBoundary {
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
    merge_author_name: String,
    merge_author_email: String,
    raw_signed_off_by: String,
    merge_commit_dco: bool,
    publication_method: String,
    classification: String,
    required_corrective_trailer: String,
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

fn checks_are_exact(checks: &ExactHeadChecks) -> bool {
    let identities: BTreeSet<_> = checks
        .checks
        .iter()
        .map(|check| (check.name.as_str(), check.workflow.as_str()))
        .collect();
    checks.expected == 12
        && checks.successful == 12
        && checks.checks.len() == 12
        && checks
            .checks
            .iter()
            .all(|check| check.conclusion == "success")
        && identities.len() == 12
        && identities == expected_checks()
}

fn validate(bytes: &[u8]) -> bool {
    if bytes.len() > MAX_ATTESTATION_BYTES {
        return false;
    }
    let Ok(attestation) = serde_json::from_slice::<Attestation>(bytes) else {
        return false;
    };
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
        && checks_are_exact(&attestation.exact_head_checks)
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
        && attestation.first_recovery.pull_request == 133
        && attestation.first_recovery.reviewed_base_commit == MERGE
        && attestation.first_recovery.reviewed_head_commit == RECOVERY_HEAD
        && attestation.first_recovery.reviewed_head_tree == RECOVERY_TREE
        && attestation.first_recovery.reviewed_head_signature == "valid_allowed_ssh"
        && attestation.first_recovery.reviewed_head_dco
        && checks_are_exact(&attestation.first_recovery.exact_head_checks)
        && attestation.first_recovery.merge_commit == RECOVERY_MERGE
        && attestation.first_recovery.merge_parents == [MERGE, RECOVERY_HEAD]
        && attestation.first_recovery.merge_tree == RECOVERY_TREE
        && attestation.first_recovery.merge_tree == attestation.first_recovery.reviewed_head_tree
        && attestation.first_recovery.merge_signature.verified
        && attestation.first_recovery.merge_signature.reason == "valid"
        && attestation.first_recovery.merge_signature.signer == "github_web_flow"
        && attestation.first_recovery.merge_signature.verified_at == "2026-09-10T23:33:54Z"
        && attestation.first_recovery.merge_author_name == RECOVERY_AUTHOR
        && attestation.first_recovery.merge_author_email == RECOVERY_EMAIL
        && attestation.first_recovery.raw_signed_off_by == RECOVERY_RAW_TRAILER
        && attestation.first_recovery.raw_signed_off_by
            != format!("Signed-off-by: {} <{}>", RECOVERY_AUTHOR, RECOVERY_EMAIL)
        && !attestation.first_recovery.merge_commit_dco
        && attestation.first_recovery.publication_method == "merge"
        && attestation.first_recovery.classification == RECOVERY_CLASSIFICATION
        && attestation.first_recovery.required_corrective_trailer == REQUIRED_TRAILER
        && attestation.first_recovery.required_corrective_trailer
            == format!("Signed-off-by: {} <{}>", RECOVERY_AUTHOR, RECOVERY_EMAIL)
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
    for (field, replacement) in [
        ("reviewed_head_commit", Value::String("0".repeat(40))),
        ("reviewed_head_tree", Value::String("1".repeat(40))),
        ("merge_commit", Value::String("2".repeat(40))),
        ("merge_tree", Value::String("3".repeat(40))),
        ("merge_commit_dco", Value::Bool(true)),
        ("merge_author_name", Value::String("Martin Beck".into())),
        ("raw_signed_off_by", Value::String(REQUIRED_TRAILER.into())),
        (
            "required_corrective_trailer",
            Value::String(RECOVERY_RAW_TRAILER.into()),
        ),
    ] {
        let mut candidate = original.clone();
        candidate["first_recovery"][field] = replacement;
        assert!(
            !validate(&serde_json::to_vec(&candidate).unwrap()),
            "{field}"
        );
    }
    let mut candidate = original.clone();
    candidate["first_recovery"]["merge_parents"][0] = Value::String(BASE.into());
    assert!(!validate(&serde_json::to_vec(&candidate).unwrap()));
    let mut candidate = original.clone();
    candidate["first_recovery"]["merge_signature"]["verified"] = Value::Bool(false);
    assert!(!validate(&serde_json::to_vec(&candidate).unwrap()));
    let mut candidate = original.clone();
    candidate["first_recovery"]["exact_head_checks"]["checks"][0]["conclusion"] =
        Value::String("failure".into());
    assert!(!validate(&serde_json::to_vec(&candidate).unwrap()));
    let mut candidate = original.clone();
    candidate["first_recovery"]["extra"] = Value::Bool(true);
    assert!(!validate(&serde_json::to_vec(&candidate).unwrap()));
    let mut candidate = original;
    candidate["catalog_fixture"]["file_sha256"] = Value::String("0".repeat(64));
    assert!(!validate(&serde_json::to_vec(&candidate).unwrap()));
    assert!(!validate(&vec![b' '; MAX_ATTESTATION_BYTES + 1]));
}
