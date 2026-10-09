// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Contract tests for the closed human diagnostic catalog.

use asb_cli::diagnostic::{
    CATALOGUED_CODES, Cause, Diagnostic, Remediation, Severity, StateChange, Subject,
};
use serde_json::Value;
use std::collections::BTreeSet;
use std::ffi::OsString;

/// Extract the only two constructors used by the routed TUI boundary.  The
/// source is a checked-in, closed Rust implementation, so this is a
/// deterministic inventory rather than a snapshot of a test run.  A new
/// `RouterError::policy("...")` or `RouterError::operation("...")` producer
/// cannot pass this contract until the reviewed typed catalogue covers it.
fn routed_tui_codes(source: &'static str) -> BTreeSet<&'static str> {
    ["RouterError::policy(\"", "RouterError::operation(\""]
        .into_iter()
        .flat_map(|prefix| {
            source
                .split(prefix)
                .skip(1)
                .map(|remainder| remainder.split('"').next().expect("closed Rust literal"))
        })
        .collect()
}

fn assert_catalogued_routed_codes(source: &'static str) -> Result<(), String> {
    let catalogued = CATALOGUED_CODES
        .iter()
        .map(|(code, _)| *code)
        .collect::<BTreeSet<_>>();
    let missing = routed_tui_codes(source)
        .difference(&catalogued)
        .copied()
        .collect::<Vec<_>>();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "routed public diagnostic producers missing reviewed catalogue entries: {}",
            missing.join(", ")
        ))
    }
}

#[test]
fn known_causes_do_not_collapse_at_the_public_boundary() {
    let cases = [
        (
            "project configuration parent is unavailable",
            Cause::MissingParent,
        ),
        ("tool installation path is a symlink", Cause::UnsafeTopology),
        (
            "provider_credential_unavailable",
            Cause::ProviderAuthentication,
        ),
        ("provider_transport_failed", Cause::TransportFailure),
        ("candidate_timeout", Cause::Timeout),
        (
            "control reconciliation lost a run",
            Cause::ReconciliationRequired,
        ),
    ];
    for (code, cause) in cases {
        assert_eq!(
            Diagnostic::for_code(code, code, Severity::Failure).cause,
            cause,
            "catalog mapping for {code}"
        );
    }
}

#[test]
fn unknown_public_producer_is_explicit_and_safe() {
    let diagnostic = Diagnostic::for_code(
        "future_public_producer",
        "future message containing /private/host and credential payload",
        Severity::Error,
    );
    assert_eq!(diagnostic.cause, Cause::UnknownCause);
    assert!(!diagnostic.is_catalogued());
    assert_eq!(diagnostic.context.subject, Subject::Unknown);
    assert_eq!(diagnostic.context.state_change, StateChange::Unknown);
    assert_eq!(diagnostic.context.remediation, Remediation::None);
    assert!(!diagnostic.context.operation.contains("private"));
}

#[test]
fn machine_error_json_contract_does_not_gain_diagnostic_context() {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit = asb_cli::run(
        &[
            OsString::from("--json"),
            OsString::from("run"),
            OsString::from("/definitely/missing/asb-plan.toml"),
        ],
        &mut stdout,
        &mut stderr,
    );
    assert_eq!(exit, 3);
    assert!(stderr.is_empty());
    let value: Value = serde_json::from_slice(&stdout).expect("valid error envelope");
    let error = value["error"].as_object().expect("error object");
    assert_eq!(
        error.keys().map(String::as_str).collect::<BTreeSet<_>>(),
        ["code", "exit_code", "message"].into_iter().collect()
    );
    assert!(
        !stdout
            .windows("credential".len())
            .any(|window| window == b"credential")
    );
    assert!(
        !stdout
            .windows("/definitely".len())
            .any(|window| window == b"/definitely")
    );
}

#[test]
fn routed_tui_producers_are_mechanically_catalogued() {
    assert_catalogued_routed_codes(include_str!("../src/tui.rs"))
        .expect("every routed TUI producer must have a reviewed typed diagnostic");
}

#[test]
fn controlled_uncatalogued_routed_producer_is_rejected() {
    let defect = r#"RouterError::policy("future_uncatalogued_router_error")"#;
    let error = assert_catalogued_routed_codes(defect)
        .expect_err("the completeness gate must reject an uncatalogued producer");
    assert!(error.contains("future_uncatalogued_router_error"));
}

#[test]
fn catalogued_diagnostics_are_actionable_and_privacy_safe() {
    for &(code, expected_cause) in CATALOGUED_CODES {
        let diagnostic = Diagnostic::for_code(code, code, Severity::Failure);
        assert_eq!(diagnostic.cause, expected_cause, "{code}");
        assert_ne!(diagnostic.cause, Cause::UnknownCause, "{code}");
        assert_ne!(diagnostic.context.subject, Subject::Unknown, "{code}");
        assert_ne!(diagnostic.context.operation, "unknown", "{code}");
        assert_ne!(diagnostic.context.remediation, Remediation::None, "{code}");

        let rendered = format!(
            "{} {} {} {}",
            diagnostic.cause_explanation(),
            diagnostic.subject_label(),
            diagnostic.state_change_explanation(),
            diagnostic.remediation_explanation(),
        );
        assert!(
            !rendered.contains("not yet classified"),
            "{code}: {rendered}"
        );
        assert!(!rendered.contains("/home/"), "{code}: {rendered}");
        assert!(
            !rendered.contains("credential payload"),
            "{code}: {rendered}"
        );
        assert!(
            !rendered.contains("private provider diagnostic"),
            "{code}: {rendered}"
        );
    }
}
