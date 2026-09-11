// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Bounded validation for the immutable PR #136 publication boundary.

use std::collections::BTreeSet;
use std::io::{self, Read};
use std::path::Path;
use std::process::{Child, Command, Stdio};

use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

const ATTESTATION: &[u8] =
    include_bytes!("../../../docs/attestations/gemini-readiness-pr136-merge.json");
const MAX_ATTESTATION_BYTES: usize = 32 * 1024;
const MAX_SOURCE_BYTES: usize = 256 * 1024;
const BASE: &str = "44eb1b48cb79b789252eff1cc798980d868c908c";
const HEAD: &str = "55648d5a29f4c29e9525eb7f2a890ac5232d7b5a";
const TREE: &str = "e7eb2b713de8abaf4af5d75f8882622740c7c3d8";
const MERGE: &str = "2ecb876b82a91a8103926b298c42ad49ce8dd143";
const SOURCE_PATH: &str = "crates/asb-agents/src/gemini.rs";
const SOURCE_SHA256: &str = "fdffabc78b6b76da0dd3e22729cab756925bc7c0edb68178845b11df04e03b17";
const AUTHOR: &str = "martin-beck";
const EMAIL: &str = "martin.beck2@gmx.de";
const REQUIRED_TRAILER: &str = "Signed-off-by: martin-beck <martin.beck2@gmx.de>";
const REVIEWED_MESSAGE: &str = "fix: publish Gemini hook readiness atomically\n\nSigned-off-by: Martin Beck <martin.beck2@gmx.de>";
const MERGE_MESSAGE: &str = "Merge pull request #136 from martin-beck/fix/gemini-hook-readiness-race\n\nfix: publish Gemini hook readiness atomically";
const CLASSIFICATION: &str = "github_verified_tree_equivalent_merge_missing_matching_dco_trailer";
const REMEDIATION: &str = "preserve protected history and publish this bounded attestation through a separately reviewed GitHub merge commit with exact reviewed-tree equality and a matching lowercase GitHub-author trailer";
const MERGE_COMMAND: &str = "gh pr merge <pr> --merge --subject <subject> --body \"$merge_body\"";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Attestation {
    schema_version: u64,
    pull_request: u64,
    pull_request_url: String,
    reviewed_base_commit: String,
    reviewed_head_commit: String,
    reviewed_head_tree: String,
    reviewed_head_signature: String,
    reviewed_head_dco: bool,
    reviewed_head_raw_message: String,
    exact_head_checks: ExactHeadChecks,
    merge_commit: String,
    merge_parents: [String; 2],
    merge_tree: String,
    merge_signature: MergeSignature,
    merge_author_name: String,
    merge_author_email: String,
    merge_raw_message: String,
    raw_signed_off_by: Option<String>,
    merge_commit_dco: bool,
    publication_method: String,
    classification: String,
    gemini_source: SourceIdentity,
    post_merge_repository_quality: FailedRun,
    remediation: String,
    corrective_merge_recipe: MergeRecipe,
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
    run_url: String,
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
struct SourceIdentity {
    path: String,
    file_sha256: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FailedRun {
    run: u64,
    url: String,
    conclusion: String,
    error: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MergeRecipe {
    command: String,
    body_encoding: String,
    required_final_trailer: String,
    forbidden_publication: Vec<String>,
}

fn expected_checks() -> BTreeSet<(&'static str, &'static str, &'static str)> {
    BTreeSet::from([
        (
            "AWQ v0.1.0 shadow evidence",
            "Agent Workflow Quality shadow",
            "34550056870",
        ),
        ("Bounded fuzz regressions", "Fault assurance", "34550056784"),
        (
            "Emulated aarch64 (x86_64 host)",
            "Emulated aarch64 portability",
            "34550056779",
        ),
        (
            "Exact Huawei 2026 and SPDX MIT headers",
            "Huawei MIT source headers",
            "34550056810",
        ),
        ("Kani bounded proofs", "Formal assurance", "34550056658"),
        (
            "Loom and state models (ubuntu-24.04)",
            "Formal assurance",
            "34550056658",
        ),
        (
            "Matcher and SLO mutation sentinels",
            "Fault assurance",
            "34550056784",
        ),
        (
            "Platform evidence (ubuntu-24.04)",
            "Hosted portability and native qualification",
            "34550056599",
        ),
        (
            "Policy, coverage, and supply chain",
            "Repository quality",
            "34550056667",
        ),
        (
            "Retained faults (ubuntu-24.04)",
            "Fault assurance",
            "34550056784",
        ),
        (
            "Rust checks (ubuntu-24.04)",
            "Rust verification",
            "34550056630",
        ),
        (
            "TLC and Alloy recovery models",
            "Formal assurance",
            "34550056658",
        ),
    ])
}

fn valid_hash(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn checks_are_exact(checks: &ExactHeadChecks) -> bool {
    let prefix = "https://github.com/martin-beck/agent-systems-benchmark/actions/runs/";
    let identities: BTreeSet<_> = checks
        .checks
        .iter()
        .map(|check| {
            (
                check.name.as_str(),
                check.workflow.as_str(),
                check.run_url.strip_prefix(prefix).unwrap_or_default(),
            )
        })
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

fn read_bounded(reader: impl Read, limit: usize) -> io::Result<Option<Vec<u8>>> {
    let mut bytes = Vec::with_capacity(limit.saturating_add(1));
    reader
        .take(limit.saturating_add(1) as u64)
        .read_to_end(&mut bytes)?;
    Ok((bytes.len() <= limit).then_some(bytes))
}

fn terminate_and_reap(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn historical_source(root: &Path) -> Option<Vec<u8>> {
    let mut child = Command::new("git")
        .args(["show", &format!("{MERGE}:{SOURCE_PATH}")])
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let Some(stdout) = child.stdout.take() else {
        terminate_and_reap(&mut child);
        return None;
    };
    match read_bounded(stdout, MAX_SOURCE_BYTES) {
        Ok(Some(bytes)) => match child.wait() {
            Ok(status) if status.success() => Some(bytes),
            Ok(_) => None,
            Err(_) => {
                terminate_and_reap(&mut child);
                None
            }
        },
        Ok(None) | Err(_) => {
            terminate_and_reap(&mut child);
            None
        }
    }
}

fn validate(bytes: &[u8], source: &[u8]) -> bool {
    if bytes.len() > MAX_ATTESTATION_BYTES {
        return false;
    }
    let Ok(attestation) = serde_json::from_slice::<Attestation>(bytes) else {
        return false;
    };
    let forbidden = BTreeSet::from([
        "default_merge_message",
        "rebase",
        "squash",
        "escaped_literal_newlines",
        "title_cased_trailer",
    ]);
    attestation.schema_version == 1
        && attestation.pull_request == 136
        && attestation.pull_request_url
            == "https://github.com/martin-beck/agent-systems-benchmark/pull/136"
        && [
            &attestation.reviewed_base_commit,
            &attestation.reviewed_head_commit,
            &attestation.reviewed_head_tree,
            &attestation.merge_commit,
            &attestation.merge_tree,
        ]
        .iter()
        .all(|value| valid_hash(value))
        && attestation
            .merge_parents
            .iter()
            .all(|value| valid_hash(value))
        && attestation.reviewed_base_commit == BASE
        && attestation.reviewed_head_commit == HEAD
        && attestation.reviewed_head_tree == TREE
        && attestation.reviewed_head_signature == "valid_allowed_ssh"
        && attestation.reviewed_head_dco
        && attestation.reviewed_head_raw_message == REVIEWED_MESSAGE
        && checks_are_exact(&attestation.exact_head_checks)
        && attestation.merge_commit == MERGE
        && attestation.merge_parents == [BASE, HEAD]
        && attestation.merge_tree == TREE
        && attestation.merge_tree == attestation.reviewed_head_tree
        && attestation.merge_signature.verified
        && attestation.merge_signature.reason == "valid"
        && attestation.merge_signature.signer == "github_web_flow"
        && attestation.merge_signature.verified_at == "2026-09-11T01:23:40Z"
        && attestation.merge_author_name == AUTHOR
        && attestation.merge_author_email == EMAIL
        && attestation.merge_raw_message == MERGE_MESSAGE
        && attestation.raw_signed_off_by.is_none()
        && !attestation.merge_commit_dco
        && attestation.publication_method == "merge"
        && attestation.classification == CLASSIFICATION
        && attestation.gemini_source.path == SOURCE_PATH
        && attestation.gemini_source.file_sha256 == SOURCE_SHA256
        && attestation.gemini_source.file_sha256 == format!("{:x}", Sha256::digest(source))
        && attestation.post_merge_repository_quality.run == 34_550_483_000
        && attestation.post_merge_repository_quality.url
            == "https://github.com/martin-beck/agent-systems-benchmark/actions/runs/34550483000"
        && attestation.post_merge_repository_quality.conclusion == "failure"
        && attestation.post_merge_repository_quality.error
            == "2ecb876b82a91a8103926b298c42ad49ce8dd143 lacks a matching Signed-off-by trailer"
        && attestation.remediation == REMEDIATION
        && attestation.corrective_merge_recipe.command == MERGE_COMMAND
        && attestation.corrective_merge_recipe.body_encoding == "actual_multiline_text"
        && attestation.corrective_merge_recipe.required_final_trailer == REQUIRED_TRAILER
        && attestation
            .corrective_merge_recipe
            .forbidden_publication
            .len()
            == 5
        && attestation
            .corrective_merge_recipe
            .forbidden_publication
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>()
            == forbidden
}

#[test]
fn reviewed_gemini_merge_boundary_is_exact_and_bounded() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source = historical_source(&root).expect("reviewed source blob must remain in history");
    assert!(validate(ATTESTATION, &source));
    assert!(!validate(ATTESTATION, b"substituted source"));
}

#[test]
fn historical_source_reads_are_bounded_and_fail_closed() {
    let oversized = vec![b'x'; MAX_SOURCE_BYTES + 1];
    assert!(
        read_bounded(oversized.as_slice(), MAX_SOURCE_BYTES)
            .unwrap()
            .is_none()
    );

    struct FailedReader;
    impl Read for FailedReader {
        fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("injected read failure"))
        }
    }
    assert!(read_bounded(FailedReader, MAX_SOURCE_BYTES).is_err());
}

#[test]
fn malformed_or_false_gemini_publication_claims_fail_closed() {
    let original: Value = serde_json::from_slice(ATTESTATION).unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source = historical_source(&root).expect("reviewed source blob must remain in history");
    for (field, replacement) in [
        ("unexpected", Value::Bool(true)),
        ("reviewed_base_commit", Value::String("0".repeat(39))),
        ("reviewed_head_commit", Value::String("0".repeat(40))),
        ("reviewed_head_tree", Value::String("1".repeat(40))),
        ("reviewed_head_signature", Value::String("unknown".into())),
        ("reviewed_head_dco", Value::Bool(false)),
        ("merge_commit", Value::String("2".repeat(40))),
        ("merge_tree", Value::String("3".repeat(40))),
        ("merge_commit_dco", Value::Bool(true)),
        ("publication_method", Value::String("squash".into())),
    ] {
        let mut candidate = original.clone();
        candidate[field] = replacement;
        assert!(
            !validate(&serde_json::to_vec(&candidate).unwrap(), &source),
            "{field}"
        );
    }

    let mut candidates = Vec::new();
    let mut candidate = original.clone();
    candidate["merge_parents"][0] = Value::String(HEAD.into());
    candidates.push(candidate);
    let mut candidate = original.clone();
    candidate["merge_signature"]["verified"] = Value::Bool(false);
    candidates.push(candidate);
    let mut candidate = original.clone();
    candidate["gemini_source"]["file_sha256"] = Value::String("0".repeat(64));
    candidates.push(candidate);
    let mut candidate = original.clone();
    candidate["exact_head_checks"]["checks"]
        .as_array_mut()
        .unwrap()
        .pop();
    candidates.push(candidate);
    let mut candidate = original.clone();
    candidate["exact_head_checks"]["checks"][1] =
        candidate["exact_head_checks"]["checks"][0].clone();
    candidates.push(candidate);
    let mut candidate = original.clone();
    candidate["exact_head_checks"]["checks"][0]["run_url"] =
        Value::String("https://example.invalid/run/1".into());
    candidates.push(candidate);
    let mut candidate = original.clone();
    candidate["post_merge_repository_quality"]["conclusion"] = Value::String("success".into());
    candidates.push(candidate);
    let mut candidate = original.clone();
    candidate["corrective_merge_recipe"]["body_encoding"] =
        Value::String("escaped_literal_newlines".into());
    candidates.push(candidate);
    let mut candidate = original.clone();
    candidate["corrective_merge_recipe"]["required_final_trailer"] =
        Value::String("Signed-off-by: Martin Beck <martin.beck2@gmx.de>".into());
    candidates.push(candidate);
    let mut candidate = original.clone();
    candidate["corrective_merge_recipe"]["command"] =
        Value::String("gh pr merge <pr> --squash".into());
    candidates.push(candidate);
    let mut candidate = original;
    candidate["corrective_merge_recipe"]["forbidden_publication"] =
        Value::Array(vec![Value::String("rebase".into())]);
    candidates.push(candidate);

    for candidate in candidates {
        assert!(!validate(&serde_json::to_vec(&candidate).unwrap(), &source));
    }
    assert!(!validate(&vec![b' '; MAX_ATTESTATION_BYTES + 1], &source));
}
