// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Contract tests for the closed human diagnostic catalog.

use asb_cli::diagnostic::{Cause, Diagnostic, Remediation, Severity, StateChange, Subject};
use serde_json::Value;
use std::collections::BTreeSet;
use std::ffi::OsString;

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
