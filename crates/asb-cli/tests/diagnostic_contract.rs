// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Contract tests for the closed human diagnostic catalog.

use asb_cli::diagnostic::{
    CATALOGUED_CODES, Cause, Diagnostic, Remediation, Severity, StateChange, Subject,
};
use serde_json::Value;
use std::collections::BTreeSet;
use std::ffi::OsString;

/// Consume Rust trivia between a constructor path and its call syntax.  The
/// source inventory is intentionally lexical (not a snapshot), so comments
/// must not create a second spelling that bypasses the public boundary.
fn skip_rust_trivia(mut source: &str) -> &str {
    loop {
        source = source.trim_start();
        if let Some(line) = source.strip_prefix("//") {
            source = line.split_once('\n').map_or("", |(_, remainder)| remainder);
            continue;
        }
        if let Some(block) = source.strip_prefix("/*") {
            source = block
                .split_once("*/")
                .map_or("", |(_, remainder)| remainder);
            continue;
        }
        return source;
    }
}

/// Extract the only two constructors used by the routed TUI boundary.  The
/// source is a checked-in, closed Rust implementation, so this is a
/// deterministic inventory rather than a snapshot of a test run.  A new
/// `RouterError::policy("...")` or `RouterError::operation("...")` producer
/// cannot pass this contract until the reviewed typed catalogue covers it.
fn routed_tui_codes(source: &'static str) -> BTreeSet<&'static str> {
    ["RouterError::policy", "RouterError::operation"]
        .into_iter()
        .flat_map(|prefix| {
            source.match_indices(prefix).filter_map(|(offset, _)| {
                let remainder = skip_rust_trivia(&source[offset + prefix.len()..]);
                if !remainder.starts_with('(') {
                    return None;
                }
                let argument = skip_rust_trivia(&remainder[1..]);
                Some(
                    argument
                        .strip_prefix('"')
                        .and_then(|literal| literal.split('"').next())
                        .unwrap_or("<nonliteral routed producer>"),
                )
            })
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
fn bare_cli_error_producers(source: &'static str) -> BTreeSet<&'static str> {
    // Deliberately match the constructor token rather than its argument
    // spelling: a producer may place `(` and a literal on later lines, or
    // pass an identifier.  Both shapes must fail before a new diagnostic can
    // inherit a prose-derived identity.
    [
        "CliError::usage",
        "CliError::validation",
        "CliError::operation",
        "CliError::validation_with_remediation",
    ]
    .into_iter()
    .filter(|constructor| {
        source.match_indices(constructor).any(|(offset, _)| {
            skip_rust_trivia(&source[offset + constructor.len()..]).starts_with('(')
        })
    })
    .collect()
}

fn assert_no_bare_cli_producers(source: &'static str) -> Result<(), String> {
    let bare = bare_cli_error_producers(source)
        .into_iter()
        .collect::<Vec<_>>();
    if bare.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "bare CLI producer bypasses the typed legacy catalog: {}",
            bare.join(" | ")
        ))
    }
}

fn assert_legacy_call_sites_are_catalogued(
    file: &'static str,
    source: &'static str,
    catalog: &'static str,
) -> Result<(), String> {
    let mut missing = Vec::new();
    for constructor in [
        "CliError::legacy_usage",
        "CliError::legacy_validation",
        "CliError::legacy_validation_with_remediation",
        "CliError::legacy_operation",
    ] {
        for (offset, _) in source.match_indices(constructor) {
            let remainder = skip_rust_trivia(&source[offset + constructor.len()..]);
            if remainder.starts_with('(') {
                let line = source[..offset]
                    .bytes()
                    .filter(|byte| *byte == b'\n')
                    .count()
                    + 1;
                let entry = format!("(\"{file}\", {line})");
                if !catalog.contains(&entry) {
                    missing.push(entry);
                }
            }
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "legacy CLI producer lacks a reviewed call-site identity: {}",
            missing.join(" | ")
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
    let multiline = "RouterError::policy(\n    \"future_multiline_router_error\",\n)";
    let error = assert_catalogued_routed_codes(multiline)
        .expect_err("the completeness gate must reject a multiline routed producer");
    assert!(error.contains("future_multiline_router_error"));
    let commented =
        "RouterError::policy /* inventory bypass */ (\n    \"future_commented_router_error\",\n)";
    let error = assert_catalogued_routed_codes(commented)
        .expect_err("the completeness gate must reject a comment-separated routed producer");
    assert!(error.contains("future_commented_router_error"));
}

#[test]
fn ordinary_cli_producers_are_mechanically_closed() {
    let catalog = include_str!("../src/diagnostic_legacy_catalog.rs");
    for (file, source) in [
        (
            "crates/asb-cli/src/control.rs",
            include_str!("../src/control.rs"),
        ),
        (
            "crates/asb-cli/src/human.rs",
            include_str!("../src/human.rs"),
        ),
        ("crates/asb-cli/src/lib.rs", include_str!("../src/lib.rs")),
        ("crates/asb-cli/src/tui.rs", include_str!("../src/tui.rs")),
    ] {
        assert_no_bare_cli_producers(source)
            .expect("every ordinary CLI producer must cross the typed boundary");
        assert_legacy_call_sites_are_catalogued(file, source, catalog)
            .expect("every quarantined legacy producer needs a reviewed identity");
    }
}

#[test]
fn controlled_bare_and_dynamic_cli_producers_are_rejected() {
    let bare = r#"CliError::operation("future workload unavailable")"#;
    let error = assert_no_bare_cli_producers(bare)
        .expect_err("the completeness gate must reject a bare keyword-looking producer");
    assert!(error.contains("CliError::operation"));
    let multiline = "CliError::operation(\n    \"future workload unavailable\",\n)";
    let error = assert_no_bare_cli_producers(multiline)
        .expect_err("the completeness gate must reject a multiline bare producer");
    assert!(error.contains("CliError::operation"));
    let dynamic = "let message = dynamic_message();\nCliError::legacy_operation(message)";
    let error = assert_legacy_call_sites_are_catalogued(
        "crates/asb-cli/src/lib.rs",
        dynamic,
        include_str!("../src/diagnostic_legacy_catalog.rs"),
    )
    .expect_err("a dynamic legacy producer must have an exact reviewed call-site identity");
    assert!(error.contains("crates/asb-cli/src/lib.rs\", 2"));
    let multiline_legacy =
        "let message = dynamic_message();\nCliError::legacy_operation\n(\n    message,\n)";
    let error = assert_legacy_call_sites_are_catalogued(
        "crates/asb-cli/src/lib.rs",
        multiline_legacy,
        include_str!("../src/diagnostic_legacy_catalog.rs"),
    )
    .expect_err("a multiline legacy producer must have an exact reviewed call-site identity");
    assert!(error.contains("crates/asb-cli/src/lib.rs\", 2"));
    let commented_legacy =
        "let message = dynamic_message();\nCliError::legacy_operation /* bypass */ (message)";
    let error = assert_legacy_call_sites_are_catalogued(
        "crates/asb-cli/src/lib.rs",
        commented_legacy,
        include_str!("../src/diagnostic_legacy_catalog.rs"),
    )
    .expect_err("a comment-separated legacy producer must have an exact reviewed identity");
    assert!(error.contains("crates/asb-cli/src/lib.rs\", 2"));
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
