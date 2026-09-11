// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Bounded validation for the immutable PR #140 publication boundary.

use std::collections::BTreeSet;
use std::io::{self, Read};
use std::path::Path;
use std::process::{Child, Command, Stdio};

use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

const ATTESTATION: &[u8] =
    include_bytes!("../../../docs/attestations/measurement-catalog-pr140-merge.json");
const MAX_ATTESTATION_BYTES: usize = 32 * 1024;
const MAX_SOURCE_BYTES: usize = 256 * 1024;
const BASE: &str = "23530dfc808650a8f3019c87a1c69fe3d0d654b5";
const HEAD: &str = "8e8d37e8c7ca7a4f673e8ae84d7395d512d45197";
const TREE: &str = "733ecc72586e1d395b45962083fa60ff6a5c258b";
const MERGE: &str = "252f746e903555c2dc626fadfa1a75bb76913144";
const SOURCE_PATH: &str = "contracts/v1/catalog.json";
const SOURCE_SHA256: &str = "83cbf015f92b9f6d0d0d2c79c27dacfd18fd2f280afd4e79961d0d8e795cb33d";
const AUTHOR: &str = "martin-beck";
const EMAIL: &str = "martin.beck2@gmx.de";
const RAW_TRAILER: &str = "Signed-off-by: Martin Beck <martin.beck2@gmx.de>";
const REQUIRED_TRAILER: &str = "Signed-off-by: martin-beck <martin.beck2@gmx.de>";
const REVIEWED_MESSAGE: &str = "feat(control): publish measurement catalog v1.2\n\nSigned-off-by: Martin Beck <martin.beck2@gmx.de>";
const MERGE_MESSAGE: &str = "feat(control): publish measurement catalog v1.2 (#140)\n\nPublish the bounded exact-version measurement catalog control contract while preserving immutable v1.0 behavior and keeping all frontend implementation outside ASB.\n\nSigned-off-by: Martin Beck <martin.beck2@gmx.de>";
const CLASSIFICATION: &str = "github_verified_tree_equivalent_merge_nonmatching_dco_author_name";
const QUALITY_FAILURE: &str = "repository policy: 252f746e903555c2dc626fadfa1a75bb76913144 lacks a matching Signed-off-by trailer";
const REMEDIATION: &str = "preserve protected history and publish this bounded attestation through a separately reviewed GitHub merge commit whose actual multiline message ends with the exact matching lowercase GitHub-author trailer";
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
    raw_signed_off_by: String,
    merge_commit_dco: bool,
    publication_method: String,
    classification: String,
    catalog_source: SourceIdentity,
    post_merge_workflows: Vec<WorkflowRun>,
    repository_quality_failure: String,
    formal_lockfile_qualification: FormalQualification,
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
struct WorkflowRun {
    workflow: String,
    run: u64,
    conclusion: String,
    url: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FormalQualification {
    workflow: String,
    run: u64,
    job: String,
    job_url: String,
    step: String,
    conclusion: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MergeRecipe {
    command: String,
    body_encoding: String,
    required_final_trailer: String,
    forbidden_publication: Vec<String>,
}

fn expected_checks() -> BTreeSet<(&'static str, &'static str, u64)> {
    BTreeSet::from([
        (
            "AWQ v0.1.0 shadow evidence",
            "Agent Workflow Quality shadow",
            34_556_818_762,
        ),
        (
            "Bounded fuzz regressions",
            "Fault assurance",
            34_556_818_753,
        ),
        (
            "Emulated aarch64 (x86_64 host)",
            "Emulated aarch64 portability",
            34_556_818_744,
        ),
        (
            "Exact Huawei 2026 and SPDX MIT headers",
            "Huawei MIT source headers",
            34_556_818_849,
        ),
        ("Kani bounded proofs", "Formal assurance", 34_556_818_741),
        (
            "Loom and state models (ubuntu-24.04)",
            "Formal assurance",
            34_556_818_741,
        ),
        (
            "Matcher and SLO mutation sentinels",
            "Fault assurance",
            34_556_818_753,
        ),
        (
            "Platform evidence (ubuntu-24.04)",
            "Hosted portability and native qualification",
            34_556_818_749,
        ),
        (
            "Policy, coverage, and supply chain",
            "Repository quality",
            34_556_818_774,
        ),
        (
            "Retained faults (ubuntu-24.04)",
            "Fault assurance",
            34_556_818_753,
        ),
        (
            "Rust checks (ubuntu-24.04)",
            "Rust verification",
            34_556_818_733,
        ),
        (
            "TLC and Alloy recovery models",
            "Formal assurance",
            34_556_818_741,
        ),
    ])
}

fn expected_workflows() -> BTreeSet<(&'static str, u64, &'static str)> {
    BTreeSet::from([
        ("Emulated aarch64 portability", 34_557_143_665, "success"),
        ("Fault assurance", 34_557_143_652, "success"),
        ("Formal assurance", 34_557_143_678, "success"),
        (
            "Hosted portability and native qualification",
            34_557_143_668,
            "success",
        ),
        ("Huawei MIT source headers", 34_557_143_650, "success"),
        ("Repository quality", 34_557_143_666, "failure"),
        ("Rust verification", 34_557_143_667, "failure"),
    ])
}

fn valid_hash(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn run_id(url: &str) -> Option<u64> {
    url.strip_prefix("https://github.com/martin-beck/agent-systems-benchmark/actions/runs/")?
        .parse()
        .ok()
}

fn checks_are_exact(checks: &ExactHeadChecks) -> bool {
    let identities: BTreeSet<_> = checks
        .checks
        .iter()
        .filter_map(|check| {
            Some((
                check.name.as_str(),
                check.workflow.as_str(),
                run_id(&check.run_url)?,
            ))
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

fn workflows_are_exact(workflows: &[WorkflowRun]) -> bool {
    let identities: BTreeSet<_> = workflows
        .iter()
        .map(|run| (run.workflow.as_str(), run.run, run.conclusion.as_str()))
        .collect();
    workflows.len() == 7
        && identities.len() == 7
        && identities == expected_workflows()
        && workflows
            .iter()
            .all(|run| run_id(&run.url) == Some(run.run))
}

fn publication_is_compliant(method: &str, body: &str) -> bool {
    method == "merge"
        && body.contains("\n\n")
        && !body.contains("\\n")
        && body.lines().last() == Some(REQUIRED_TRAILER)
        && body
            .lines()
            .filter(|line| line.starts_with("Signed-off-by:"))
            .count()
            == 1
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

fn historical_source(root: &Path, revision: &str, path: &str) -> Option<Vec<u8>> {
    let mut child = Command::new("git")
        .args(["show", &format!("{revision}:{path}")])
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
        "duplicate_trailers",
        "escaped_literal_newlines",
        "missing_trailer",
        "rebase",
        "squash",
        "title_cased_trailer",
    ]);
    attestation.schema_version == 1
        && attestation.pull_request == 140
        && attestation.pull_request_url
            == "https://github.com/martin-beck/agent-systems-benchmark/pull/140"
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
        && attestation.merge_signature.verified_at == "2026-09-11T03:05:51Z"
        && attestation.merge_author_name == AUTHOR
        && attestation.merge_author_email == EMAIL
        && attestation.merge_raw_message == MERGE_MESSAGE
        && attestation.raw_signed_off_by == RAW_TRAILER
        && !attestation.merge_commit_dco
        && attestation.raw_signed_off_by != REQUIRED_TRAILER
        && attestation.publication_method == "merge"
        && attestation.classification == CLASSIFICATION
        && attestation.catalog_source.path == SOURCE_PATH
        && attestation.catalog_source.file_sha256 == SOURCE_SHA256
        && attestation.catalog_source.file_sha256 == format!("{:x}", Sha256::digest(source))
        && workflows_are_exact(&attestation.post_merge_workflows)
        && attestation.repository_quality_failure == QUALITY_FAILURE
        && attestation.formal_lockfile_qualification.workflow == "Formal assurance"
        && attestation.formal_lockfile_qualification.run == 34_557_143_678
        && attestation.formal_lockfile_qualification.job == "Kani bounded proofs"
        && attestation.formal_lockfile_qualification.job_url
            == "https://github.com/martin-beck/agent-systems-benchmark/actions/runs/34557143678/job/103132091596"
        && attestation.formal_lockfile_qualification.step
            == "Require the committed dependency graph"
        && attestation.formal_lockfile_qualification.conclusion == "success"
        && attestation.remediation == REMEDIATION
        && attestation.corrective_merge_recipe.command == MERGE_COMMAND
        && attestation.corrective_merge_recipe.body_encoding == "actual_multiline_text"
        && attestation.corrective_merge_recipe.required_final_trailer == REQUIRED_TRAILER
        && attestation
            .corrective_merge_recipe
            .forbidden_publication
            .len()
            == 7
        && attestation
            .corrective_merge_recipe
            .forbidden_publication
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>()
            == forbidden
}

#[test]
fn reviewed_measurement_catalog_merge_boundary_is_exact_and_bounded() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source = historical_source(&root, MERGE, SOURCE_PATH)
        .expect("reviewed catalog source must remain in history");
    assert!(validate(ATTESTATION, &source));
    assert!(!validate(ATTESTATION, b"substituted catalog"));
    assert!(historical_source(&root, "0", SOURCE_PATH).is_none());
    assert!(historical_source(&root, MERGE, "missing-object").is_none());
}

#[test]
fn historical_catalog_reads_are_bounded_and_fail_closed() {
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
fn corrective_publication_requires_one_exact_author_matching_trailer() {
    let valid = format!("Attest PR #140\n\n{REQUIRED_TRAILER}");
    assert!(publication_is_compliant("merge", &valid));
    assert!(!publication_is_compliant("squash", &valid));
    assert!(!publication_is_compliant("rebase", &valid));
    assert!(!publication_is_compliant("merge", "default merge message"));
    assert!(!publication_is_compliant(
        "merge",
        &format!("Attest PR #140\\n\\n{REQUIRED_TRAILER}")
    ));
    assert!(!publication_is_compliant(
        "merge",
        &format!("Attest PR #140\n\n{RAW_TRAILER}")
    ));
    assert!(!publication_is_compliant(
        "merge",
        "Attest PR #140\n\nNo trailer"
    ));
    assert!(!publication_is_compliant(
        "merge",
        &format!("Attest PR #140\n\n{REQUIRED_TRAILER}\n{REQUIRED_TRAILER}")
    ));
}

#[test]
fn malformed_or_false_measurement_catalog_claims_fail_closed() {
    let original: Value = serde_json::from_slice(ATTESTATION).unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source = historical_source(&root, MERGE, SOURCE_PATH).unwrap();
    for (field, replacement) in [
        ("unexpected", Value::Bool(true)),
        ("schema_version", Value::from(2)),
        ("pull_request", Value::from(141)),
        (
            "pull_request_url",
            Value::String("https://example.invalid/140".into()),
        ),
        ("reviewed_base_commit", Value::String("0".repeat(40))),
        ("reviewed_head_commit", Value::String("1".repeat(40))),
        ("reviewed_head_tree", Value::String("2".repeat(40))),
        ("reviewed_head_signature", Value::String("unknown".into())),
        ("reviewed_head_dco", Value::Bool(false)),
        (
            "reviewed_head_raw_message",
            Value::String("substituted".into()),
        ),
        ("merge_commit", Value::String("3".repeat(40))),
        ("merge_tree", Value::String("4".repeat(40))),
        ("merge_author_name", Value::String("Martin Beck".into())),
        (
            "merge_author_email",
            Value::String("other@example.invalid".into()),
        ),
        ("merge_raw_message", Value::String("substituted".into())),
        ("raw_signed_off_by", Value::String(REQUIRED_TRAILER.into())),
        ("merge_commit_dco", Value::Bool(true)),
        ("publication_method", Value::String("squash".into())),
        ("classification", Value::String("compliant".into())),
        ("repository_quality_failure", Value::String("none".into())),
        ("remediation", Value::String("rewrite history".into())),
    ] {
        let mut candidate = original.clone();
        candidate[field] = replacement;
        assert!(
            !validate(&serde_json::to_vec(&candidate).unwrap(), &source),
            "{field}"
        );
    }

    let mutations: &[(&str, &str, Value)] = &[
        ("/merge_parents/0", "parent", Value::String(HEAD.into())),
        (
            "/merge_signature/verified",
            "signature verified",
            Value::Bool(false),
        ),
        (
            "/merge_signature/reason",
            "signature reason",
            Value::String("unknown".into()),
        ),
        (
            "/merge_signature/signer",
            "signature signer",
            Value::String("ambient".into()),
        ),
        (
            "/merge_signature/verified_at",
            "signature time",
            Value::String("later".into()),
        ),
        (
            "/catalog_source/path",
            "catalog path",
            Value::String("other".into()),
        ),
        (
            "/catalog_source/file_sha256",
            "catalog digest",
            Value::String("0".repeat(64)),
        ),
        (
            "/exact_head_checks/expected",
            "check expected",
            Value::from(11),
        ),
        (
            "/exact_head_checks/successful",
            "check successful",
            Value::from(11),
        ),
        (
            "/exact_head_checks/checks/0/name",
            "check name",
            Value::String("other".into()),
        ),
        (
            "/exact_head_checks/checks/0/workflow",
            "check workflow",
            Value::String("other".into()),
        ),
        (
            "/exact_head_checks/checks/0/conclusion",
            "check conclusion",
            Value::String("failure".into()),
        ),
        (
            "/exact_head_checks/checks/0/run_url",
            "check URL",
            Value::String("https://example.invalid".into()),
        ),
        (
            "/post_merge_workflows/0/workflow",
            "workflow name",
            Value::String("other".into()),
        ),
        (
            "/post_merge_workflows/0/run",
            "workflow run",
            Value::from(1),
        ),
        (
            "/post_merge_workflows/0/conclusion",
            "workflow conclusion",
            Value::String("failure".into()),
        ),
        (
            "/post_merge_workflows/0/url",
            "workflow URL",
            Value::String("https://example.invalid".into()),
        ),
        (
            "/formal_lockfile_qualification/workflow",
            "formal workflow",
            Value::String("other".into()),
        ),
        (
            "/formal_lockfile_qualification/run",
            "formal run",
            Value::from(1),
        ),
        (
            "/formal_lockfile_qualification/job",
            "formal job",
            Value::String("other".into()),
        ),
        (
            "/formal_lockfile_qualification/job_url",
            "formal URL",
            Value::String("https://example.invalid".into()),
        ),
        (
            "/formal_lockfile_qualification/step",
            "formal step",
            Value::String("skipped".into()),
        ),
        (
            "/formal_lockfile_qualification/conclusion",
            "formal conclusion",
            Value::String("failure".into()),
        ),
        (
            "/corrective_merge_recipe/command",
            "recipe command",
            Value::String("gh pr merge <pr> --squash".into()),
        ),
        (
            "/corrective_merge_recipe/body_encoding",
            "body encoding",
            Value::String("escaped_literal_newlines".into()),
        ),
        (
            "/corrective_merge_recipe/required_final_trailer",
            "required trailer",
            Value::String(RAW_TRAILER.into()),
        ),
    ];
    for (pointer, name, replacement) in mutations {
        let mut candidate = original.clone();
        *candidate.pointer_mut(pointer).unwrap() = replacement.clone();
        assert!(
            !validate(&serde_json::to_vec(&candidate).unwrap(), &source),
            "{name}"
        );
    }

    for pointer in [
        "/exact_head_checks/checks",
        "/post_merge_workflows",
        "/corrective_merge_recipe/forbidden_publication",
    ] {
        let mut missing = original.clone();
        missing
            .pointer_mut(pointer)
            .unwrap()
            .as_array_mut()
            .unwrap()
            .pop();
        assert!(
            !validate(&serde_json::to_vec(&missing).unwrap(), &source),
            "missing {pointer}"
        );
        let mut duplicate = original.clone();
        let array = duplicate
            .pointer_mut(pointer)
            .unwrap()
            .as_array_mut()
            .unwrap();
        array.push(array[0].clone());
        assert!(
            !validate(&serde_json::to_vec(&duplicate).unwrap(), &source),
            "duplicate {pointer}"
        );
    }

    let mut nested_unknown = original;
    nested_unknown["merge_signature"]["unexpected"] = Value::Bool(true);
    assert!(!validate(
        &serde_json::to_vec(&nested_unknown).unwrap(),
        &source
    ));
    assert!(!validate(&vec![b' '; MAX_ATTESTATION_BYTES + 1], &source));
}
