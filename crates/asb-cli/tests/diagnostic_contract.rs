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

/// Extract literal producers that cross the ordinary public CLI error
/// boundary.  These used to share only the broad `usage`, `validation`, and
/// `operation` identities, which allowed a newly-added public failure to
/// inherit a prose-based fallback silently.  Keeping this source inventory
/// executable makes every literal producer either receive a reviewed cause or
/// fail this contract.
fn literal_cli_error_messages(source: &'static str) -> BTreeSet<&'static str> {
    [
        "CliError::usage(\"",
        "CliError::validation(\"",
        "CliError::operation(\"",
        "CliError::validation_with_remediation(\"",
    ]
    .into_iter()
    .flat_map(|prefix| {
        source
            .split(prefix)
            .skip(1)
            .map(|remainder| remainder.split('\"').next().expect("closed Rust literal"))
    })
    .collect()
}

fn assert_literal_cli_producers_are_specific(source: &'static str) -> Result<(), String> {
    let fallback = literal_cli_error_messages(source)
        .into_iter()
        .filter(|message| {
            Diagnostic::for_cli_literal(message, Severity::Failure).cause == Cause::UnknownCause
        })
        .collect::<Vec<_>>();
    if fallback.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "ordinary CLI producers need reviewed typed causes: {}",
            fallback.join(" | ")
        ))
    }
}

/// Validate the semantic obligations that must accompany a public diagnostic.
/// The inputs deliberately model the human line, stream and exit separately
/// from the stable machine envelope: changing any one of them must not hide a
/// generic, unsafe, or privacy-bearing presentation behind a green snapshot.
fn validate_public_diagnostic(
    diagnostic: Diagnostic,
    human: &str,
    warning: bool,
    stream: &str,
    exit: u8,
) -> Result<(), String> {
    if !diagnostic.is_catalogued() {
        return Err("uncatalogued public producer".into());
    }
    if diagnostic.context.subject == Subject::Unknown
        || diagnostic.context.operation == "unknown"
        || diagnostic.context.remediation == Remediation::None
    {
        return Err("diagnostic lacks safe subject, operation, or recovery".into());
    }
    if human.trim().is_empty()
        || ["failed", "unavailable", "invalid"]
            .iter()
            .any(|placeholder| human.trim().eq_ignore_ascii_case(placeholder))
    {
        return Err("generic human explanation".into());
    }
    if warning && !human.contains("not") {
        return Err("warning has no operator-visible consequence".into());
    }
    if human.contains("curl ") || human.contains("sh -c") || human.contains("; rm ") {
        return Err("unsafe suggested command".into());
    }
    if [
        "/home/",
        "/srv/",
        "credential payload",
        "private provider diagnostic",
    ]
    .iter()
    .any(|private| human.contains(private))
    {
        return Err("private material in human diagnostic".into());
    }
    let expected_exit = match diagnostic.severity {
        Severity::Error => 3,
        Severity::Failure => 4,
        Severity::Warning => 0,
    };
    if stream != "stdout" || exit != expected_exit {
        return Err("wrong public stream or exit meaning".into());
    }
    Ok(())
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
fn ordinary_cli_producers_are_mechanically_specific() {
    assert_literal_cli_producers_are_specific(include_str!("../src/lib.rs"))
        .expect("every ordinary CLI producer must have a specific reviewed cause");
}

#[test]
fn controlled_generic_cli_producer_is_rejected() {
    let defect = r#"CliError::operation("future unclassified public condition")"#;
    let error = assert_literal_cli_producers_are_specific(defect)
        .expect_err("the completeness gate must reject a generic CLI producer");
    assert!(error.contains("future unclassified public condition"));
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
        validate_public_diagnostic(diagnostic, &rendered, false, "stdout", 4)
            .unwrap_or_else(|error| panic!("{code}: {error}"));
    }
}

#[test]
fn controlled_diagnostic_quality_defects_are_rejected() {
    let valid = Diagnostic::for_code(
        "provider_transport_failed",
        "provider_transport_failed",
        Severity::Failure,
    );
    let rendered = format!(
        "{} {} {}",
        valid.cause_explanation(),
        valid.state_change_explanation(),
        valid.remediation_explanation(),
    );
    assert!(validate_public_diagnostic(valid, &rendered, false, "stdout", 4).is_ok());

    let uncatalogued = Diagnostic::for_code(
        "future_public_code",
        "future_public_code",
        Severity::Failure,
    );
    assert!(
        validate_public_diagnostic(uncatalogued, &rendered, false, "stdout", 4)
            .unwrap_err()
            .contains("uncatalogued")
    );
    assert!(
        validate_public_diagnostic(valid, "failed", false, "stdout", 4)
            .unwrap_err()
            .contains("generic")
    );
    assert!(
        validate_public_diagnostic(
            valid,
            "/home/private credential payload",
            false,
            "stdout",
            4
        )
        .unwrap_err()
        .contains("private")
    );
    assert!(
        validate_public_diagnostic(
            valid,
            "Run curl https://example.invalid",
            false,
            "stdout",
            4
        )
        .unwrap_err()
        .contains("unsafe")
    );
    assert!(
        validate_public_diagnostic(valid, &rendered, false, "stderr", 4)
            .unwrap_err()
            .contains("stream")
    );
    assert!(
        validate_public_diagnostic(valid, &rendered, false, "stdout", 3)
            .unwrap_err()
            .contains("exit")
    );
    let warning =
        Diagnostic::unavailable_warning("capability_unavailable", Subject::Capability, "doctor");
    assert!(
        validate_public_diagnostic(warning, "Capability unavailable.", true, "stdout", 0)
            .unwrap_err()
            .contains("consequence")
    );
}
