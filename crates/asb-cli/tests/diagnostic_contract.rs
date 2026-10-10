// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Contract tests for the closed human diagnostic catalog.

use asb_cli::diagnostic::{
    CATALOGUED_CODES, Cause, Diagnostic, Remediation, Severity, StateChange, Subject,
};
use proc_macro2::{Delimiter, Group, TokenStream, TokenTree};
use serde_json::Value;
use std::collections::BTreeSet;
use std::ffi::OsString;
use syn::spanned::Spanned;
use syn::visit::Visit;
use syn::{Expr, ExprCall, ExprPath, File, ItemType, Lit, Path, TypePath, UseRename};

#[derive(Default)]
struct DiagnosticAstGate {
    aliases: Vec<String>,
    qualified_paths: Vec<String>,
    constructor_paths: BTreeSet<String>,
    constructor_calls: Vec<(String, usize, Option<String>)>,
}

impl<'ast> Visit<'ast> for DiagnosticAstGate {
    fn visit_item_type(&mut self, item: &'ast ItemType) {
        if let syn::Type::Path(TypePath { path, .. }) = item.ty.as_ref()
            && path.segments.last().is_some_and(|segment| {
                matches!(
                    segment.ident.to_string().as_str(),
                    "CliError" | "RouterError"
                )
            })
        {
            self.aliases.push(item.ident.to_string());
        }
        syn::visit::visit_item_type(self, item);
    }

    fn visit_use_rename(&mut self, rename: &'ast UseRename) {
        // A renamed import can hide a public error type from the closed
        // inventory, regardless of the chosen replacement name.
        if matches!(
            rename.ident.to_string().as_str(),
            "CliError" | "RouterError"
        ) {
            self.aliases.push(rename.rename.to_string());
        }
        syn::visit::visit_use_rename(self, rename);
    }

    fn visit_expr_path(&mut self, expression: &'ast ExprPath) {
        if expression.qself.is_some()
            && expression
                .qself
                .as_ref()
                .is_some_and(|qself| is_diagnostic_type(qself.ty.as_ref()))
        {
            self.qualified_paths
                .push("qualified diagnostic path".into());
        }
        if expression.qself.is_none()
            && let Some(constructor) = diagnostic_constructor(&expression.path)
        {
            self.constructor_paths.insert(constructor.to_owned());
        }
        syn::visit::visit_expr_path(self, expression);
    }

    fn visit_expr_call(&mut self, call: &'ast ExprCall) {
        if let Expr::Path(expression) = call.func.as_ref()
            && expression.qself.is_none()
            && let Some(constructor) = diagnostic_constructor(&expression.path)
        {
            let literal = call.args.first().and_then(|argument| match argument {
                Expr::Lit(literal) => match &literal.lit {
                    Lit::Str(value) => Some(value.value()),
                    _ => None,
                },
                _ => None,
            });
            self.constructor_calls.push((
                constructor.to_owned(),
                call.span().start().line,
                literal,
            ));
        }
        syn::visit::visit_expr_call(self, call);
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        visit_macro_token_groups(self, mac.tokens.clone());
        syn::visit::visit_macro(self, mac);
    }
}

fn visit_macro_token_groups(gate: &mut DiagnosticAstGate, tokens: TokenStream) {
    if let Ok(expression) = syn::parse2::<syn::Expr>(tokens.clone()) {
        gate.visit_expr(&expression);
    }
    // Macro payloads are token streams rather than expressions.  In particular,
    // a statement-form payload such as `let _ = RouterError::policy(...);` is
    // neither an `Expr` nor a brace-delimited `Block` by itself.  Parse both a
    // single statement and the same stream in an explicit block context so
    // multi-statement macro payloads receive the ordinary syn visitor walk.
    if let Ok(statement) = syn::parse2::<syn::Stmt>(tokens.clone()) {
        gate.visit_stmt(&statement);
    }
    let block_context = TokenStream::from(TokenTree::Group(Group::new(
        Delimiter::Brace,
        tokens.clone(),
    )));
    if let Ok(block) = syn::parse2::<syn::Block>(block_context) {
        gate.visit_block(&block);
    }
    for token in tokens {
        if let TokenTree::Group(group) = token {
            let stream = group.stream();
            if let Ok(block) = syn::parse2::<syn::Block>(stream.clone()) {
                gate.visit_block(&block);
            }
            visit_macro_token_groups(gate, stream);
        }
    }
}

fn is_diagnostic_type(ty: &syn::Type) -> bool {
    matches!(ty, syn::Type::Path(TypePath { path, .. }) if path.segments.last().is_some_and(|segment| matches!(segment.ident.to_string().as_str(), "CliError" | "RouterError")))
}

fn diagnostic_constructor(path: &Path) -> Option<&'static str> {
    let segments = path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect::<Vec<_>>();
    let [.., error, constructor] = segments.as_slice() else {
        return None;
    };
    match (error.as_str(), constructor.as_str()) {
        ("RouterError", "policy") => Some("RouterError::policy"),
        ("RouterError", "operation") => Some("RouterError::operation"),
        ("CliError", "usage") => Some("CliError::usage"),
        ("CliError", "validation") => Some("CliError::validation"),
        ("CliError", "operation") => Some("CliError::operation"),
        ("CliError", "validation_with_remediation") => {
            Some("CliError::validation_with_remediation")
        }
        ("CliError", "legacy_usage") => Some("CliError::legacy_usage"),
        ("CliError", "legacy_validation") => Some("CliError::legacy_validation"),
        ("CliError", "legacy_validation_with_remediation") => {
            Some("CliError::legacy_validation_with_remediation")
        }
        ("CliError", "legacy_operation") => Some("CliError::legacy_operation"),
        _ => None,
    }
}

fn diagnostic_ast(source: &str) -> Result<DiagnosticAstGate, String> {
    // Production inputs are modules.  Controlled defects include expression
    // fragments, so parse those in a function body without falling back to a
    // text scanner.
    let parsed: File = syn::parse_file(source)
        .or_else(|_| syn::parse_file(&format!("fn diagnostic_ast_fixture() {{ {source}; }}")))
        .map_err(|error| error.to_string())?;
    let mut gate = DiagnosticAstGate::default();
    gate.visit_file(&parsed);
    Ok(gate)
}

fn assert_ast_is_direct_and_unaliased(source: &str) -> Result<DiagnosticAstGate, String> {
    let gate = diagnostic_ast(source)?;
    if !gate.aliases.is_empty() {
        return Err(format!(
            "diagnostic error type aliases are forbidden by the closed inventory: {}",
            gate.aliases.join(", ")
        ));
    }
    if !gate.qualified_paths.is_empty() {
        return Err(
            "qualified diagnostic error paths are forbidden by the closed inventory".into(),
        );
    }
    let called = gate
        .constructor_calls
        .iter()
        .map(|(constructor, _, _)| constructor)
        .collect::<BTreeSet<_>>();
    if let Some(function_item) = gate
        .constructor_paths
        .iter()
        .find(|constructor| !called.contains(constructor))
    {
        return Err(format!(
            "diagnostic constructor is used as a function item rather than a direct call: {function_item}"
        ));
    }
    Ok(gate)
}

/// Extract the only two constructors used by the routed TUI boundary.  The
/// source is a checked-in, closed Rust implementation, so this is a
/// deterministic inventory rather than a snapshot of a test run.  A new
/// `RouterError::policy("...")` or `RouterError::operation("...")` producer
/// cannot pass this contract until the reviewed typed catalogue covers it.
fn assert_catalogued_routed_codes(source: &'static str) -> Result<(), String> {
    let gate = assert_ast_is_direct_and_unaliased(source)?;
    let catalogued = CATALOGUED_CODES
        .iter()
        .map(|(code, _)| *code)
        .collect::<BTreeSet<_>>();
    let routed = gate
        .constructor_calls
        .iter()
        .filter(|(constructor, _, _)| constructor.starts_with("RouterError::"))
        .map(|(_, _, literal)| literal.as_deref().unwrap_or("<nonliteral routed producer>"))
        .collect::<BTreeSet<_>>();
    let missing = routed.difference(&catalogued).copied().collect::<Vec<_>>();
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
fn bare_cli_error_producers(source: &'static str) -> Result<BTreeSet<String>, String> {
    // Deliberately match the constructor token rather than its argument
    // spelling: a producer may place `(` and a literal on later lines, or
    // pass an identifier.  Both shapes must fail before a new diagnostic can
    // inherit a prose-derived identity.
    Ok(assert_ast_is_direct_and_unaliased(source)?
        .constructor_calls
        .into_iter()
        .filter_map(|(constructor, _, _)| {
            matches!(
                constructor.as_str(),
                "CliError::usage"
                    | "CliError::validation"
                    | "CliError::operation"
                    | "CliError::validation_with_remediation"
            )
            .then_some(constructor)
        })
        .collect())
}

fn assert_no_bare_cli_producers(source: &'static str) -> Result<(), String> {
    let bare = bare_cli_error_producers(source)?
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
    let constructors = [
        "CliError::legacy_usage",
        "CliError::legacy_validation",
        "CliError::legacy_validation_with_remediation",
        "CliError::legacy_operation",
    ];
    let gate = assert_ast_is_direct_and_unaliased(source)?;
    let mut missing = Vec::new();
    for (constructor, line, _) in gate.constructor_calls {
        if constructors.contains(&constructor.as_str()) {
            let entry = format!("(\"{file}\", {line},");
            if !catalog.contains(&entry) {
                missing.push(entry);
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
    let nested = "RouterError::policy /* outer /* inner */ outer */ (\n    \"future_nested_router_error\",\n)";
    let error = assert_catalogued_routed_codes(nested)
        .expect_err("the completeness gate must reject a nested-comment routed producer");
    assert!(error.contains("future_nested_router_error"));
    assert_catalogued_routed_codes(
        "// RouterError::policy(\"comment_only_uncatalogued_router_error\")\n/* RouterError::operation(\"comment_only_uncatalogued_router_error\") */",
    )
    .expect("comments are not public diagnostic producers");
    let function_item = "let produce = RouterError::policy;\nproduce(\"future_item_router_error\")";
    let error = assert_catalogued_routed_codes(function_item)
        .expect_err("the completeness gate must reject a routed function item");
    assert!(error.contains("function item"));
    let alias = "use crate::RouterError as Error;\nError::policy(\"future_alias_router_error\")";
    let error = assert_catalogued_routed_codes(alias)
        .expect_err("the completeness gate must reject a routed error-type alias");
    assert!(error.contains("aliases are forbidden"), "{error}");
    let multiline_type_alias = "type Error =\n crate::RouterError;\nError::policy(\"future_multiline_alias_router_error\")";
    let error = assert_catalogued_routed_codes(multiline_type_alias)
        .expect_err("the completeness gate must reject a multiline routed type alias");
    assert!(error.contains("aliases are forbidden"), "{error}");
    let trivia_after_type = "type /* boundary */ Error = crate::RouterError;\nError::policy(\"future_type_trivia_router_error\")";
    let error = assert_catalogued_routed_codes(trivia_after_type)
        .expect_err("the completeness gate must reject routed type-token trivia");
    assert!(error.contains("aliases are forbidden"));
    let type_alias = "type Error = RouterError;\nError::policy(\"future_type_alias_router_error\")";
    let error = assert_catalogued_routed_codes(type_alias)
        .expect_err("the completeness gate must reject a routed type alias");
    assert!(error.contains("aliases are forbidden"));
    let trivia_after_type = "type /* boundary */ Error = crate::CliError;\nError::legacy_operation(\"future_type_trivia_legacy_error\")";
    let error = assert_legacy_call_sites_are_catalogued(
        "crates/asb-cli/src/lib.rs",
        trivia_after_type,
        include_str!("../src/diagnostic_legacy_catalog.rs"),
    )
    .expect_err("the completeness gate must reject legacy type-token trivia");
    assert!(error.contains("aliases are forbidden"));
    let commented_type_alias = "type Error = /* alias boundary */ RouterError;\nError::policy(\"future_commented_type_alias_router_error\")";
    let error = assert_catalogued_routed_codes(commented_type_alias)
        .expect_err("the completeness gate must reject a comment-separated routed type alias");
    assert!(error.contains("aliases are forbidden"));
    let namespaced_type_alias = "type Error = crate /* namespace */ ::RouterError;\nError::policy(\"future_namespaced_type_alias_router_error\")";
    let error = assert_catalogued_routed_codes(namespaced_type_alias)
        .expect_err("the completeness gate must reject a namespaced routed type alias");
    assert!(error.contains("aliases are forbidden"));
    let qualified = "<RouterError>::policy(\"future_qualified_router_error\")";
    let error = assert_catalogued_routed_codes(qualified)
        .expect_err("the completeness gate must reject a qualified routed path");
    assert!(error.contains("qualified diagnostic error paths"));
    let post_close_trivia =
        "<crate::RouterError> /* path boundary */ ::policy(\"future_post_close_router_error\")";
    let error = assert_catalogued_routed_codes(post_close_trivia)
        .expect_err("the completeness gate must reject post-close routed trivia");
    assert!(error.contains("qualified diagnostic error paths"));
    let commented_qualified = "< /* path boundary */ RouterError /* path boundary */ >::policy(\"future_commented_qualified_router_error\")";
    let error = assert_catalogued_routed_codes(commented_qualified)
        .expect_err("the completeness gate must reject a comment-separated qualified routed path");
    assert!(error.contains("qualified diagnostic error paths"));
    let namespaced_qualified = "<crate /* namespace */ ::RouterError>::policy(\"future_namespaced_qualified_router_error\")";
    let error = assert_catalogued_routed_codes(namespaced_qualified)
        .expect_err("the completeness gate must reject a namespaced qualified routed path");
    assert!(error.contains("qualified diagnostic error paths"));
    let regular_qualified = "crate::RouterError::policy(\"future_regular_qualified_router_error\")";
    let error = assert_catalogued_routed_codes(regular_qualified)
        .expect_err("the completeness gate must reject a qualified routed constructor");
    assert!(error.contains("future_regular_qualified_router_error"));
    let macro_producer =
        "emit_error! { crate::RouterError::policy(\"future_macro_router_error\") };";
    let error = assert_catalogued_routed_codes(macro_producer)
        .expect_err("the completeness gate must inspect macro diagnostic producers");
    assert!(error.contains("future_macro_router_error"));
    let macro_statement_producer = "emit_error! { let _ = crate::RouterError::policy(\"future_macro_statement_router_error\"); };";
    let error = assert_catalogued_routed_codes(macro_statement_producer)
        .expect_err("the completeness gate must inspect statement-form macro diagnostic producers");
    assert!(error.contains("future_macro_statement_router_error"));
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
    let nested_legacy = "let message = dynamic_message();\nCliError::legacy_operation /* outer /* inner */ outer */ (message)";
    let error = assert_legacy_call_sites_are_catalogued(
        "crates/asb-cli/src/lib.rs",
        nested_legacy,
        include_str!("../src/diagnostic_legacy_catalog.rs"),
    )
    .expect_err("a nested-comment legacy producer must have an exact reviewed identity");
    assert!(error.contains("crates/asb-cli/src/lib.rs\", 2"));
    let function_item =
        "let produce = CliError::legacy_operation;\nproduce(\"future_item_legacy_error\")";
    let error = assert_legacy_call_sites_are_catalogued(
        "crates/asb-cli/src/lib.rs",
        function_item,
        include_str!("../src/diagnostic_legacy_catalog.rs"),
    )
    .expect_err("the completeness gate must reject a legacy function item");
    assert!(error.contains("function item"));
    let alias =
        "use crate::CliError as Error;\nError::legacy_operation(\"future_alias_legacy_error\")";
    let error = assert_legacy_call_sites_are_catalogued(
        "crates/asb-cli/src/lib.rs",
        alias,
        include_str!("../src/diagnostic_legacy_catalog.rs"),
    )
    .expect_err("the completeness gate must reject a legacy error-type alias");
    assert!(error.contains("aliases are forbidden"));
    let multiline_type_alias = "type Error =\n crate::CliError;\nError::legacy_operation(\"future_multiline_alias_legacy_error\")";
    let error = assert_legacy_call_sites_are_catalogued(
        "crates/asb-cli/src/lib.rs",
        multiline_type_alias,
        include_str!("../src/diagnostic_legacy_catalog.rs"),
    )
    .expect_err("the completeness gate must reject a multiline legacy type alias");
    assert!(error.contains("aliases are forbidden"));
    let type_alias =
        "type Error = CliError;\nError::legacy_operation(\"future_type_alias_legacy_error\")";
    let error = assert_legacy_call_sites_are_catalogued(
        "crates/asb-cli/src/lib.rs",
        type_alias,
        include_str!("../src/diagnostic_legacy_catalog.rs"),
    )
    .expect_err("the completeness gate must reject a legacy type alias");
    assert!(error.contains("aliases are forbidden"));
    let commented_type_alias = "type Error = /* alias boundary */ CliError;\nError::legacy_operation(\"future_commented_type_alias_legacy_error\")";
    let error = assert_legacy_call_sites_are_catalogued(
        "crates/asb-cli/src/lib.rs",
        commented_type_alias,
        include_str!("../src/diagnostic_legacy_catalog.rs"),
    )
    .expect_err("the completeness gate must reject a comment-separated legacy type alias");
    assert!(error.contains("aliases are forbidden"));
    let namespaced_type_alias = "type Error = super /* namespace */ ::CliError;\nError::legacy_operation(\"future_namespaced_type_alias_legacy_error\")";
    let error = assert_legacy_call_sites_are_catalogued(
        "crates/asb-cli/src/lib.rs",
        namespaced_type_alias,
        include_str!("../src/diagnostic_legacy_catalog.rs"),
    )
    .expect_err("the completeness gate must reject a namespaced legacy type alias");
    assert!(error.contains("aliases are forbidden"));
    let qualified = "<CliError>::legacy_operation(\"future_qualified_legacy_error\")";
    let error = assert_legacy_call_sites_are_catalogued(
        "crates/asb-cli/src/lib.rs",
        qualified,
        include_str!("../src/diagnostic_legacy_catalog.rs"),
    )
    .expect_err("the completeness gate must reject a qualified legacy path");
    assert!(error.contains("qualified diagnostic error paths"));
    let commented_qualified = "< /* path boundary */ CliError /* path boundary */ >::legacy_operation(\"future_commented_qualified_legacy_error\")";
    let error = assert_legacy_call_sites_are_catalogued(
        "crates/asb-cli/src/lib.rs",
        commented_qualified,
        include_str!("../src/diagnostic_legacy_catalog.rs"),
    )
    .expect_err("the completeness gate must reject a comment-separated qualified legacy path");
    assert!(error.contains("qualified diagnostic error paths"));
    let namespaced_qualified = "<self /* namespace */ ::CliError>::legacy_operation(\"future_namespaced_qualified_legacy_error\")";
    let error = assert_legacy_call_sites_are_catalogued(
        "crates/asb-cli/src/lib.rs",
        namespaced_qualified,
        include_str!("../src/diagnostic_legacy_catalog.rs"),
    )
    .expect_err("the completeness gate must reject a namespaced qualified legacy path");
    assert!(error.contains("qualified diagnostic error paths"));
    let post_close_trivia = "<crate::CliError> /* path boundary */ ::legacy_operation(\"future_post_close_legacy_error\")";
    let error = assert_legacy_call_sites_are_catalogued(
        "crates/asb-cli/src/lib.rs",
        post_close_trivia,
        include_str!("../src/diagnostic_legacy_catalog.rs"),
    )
    .expect_err("the completeness gate must reject post-close legacy trivia");
    assert!(error.contains("qualified diagnostic error paths"));
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
