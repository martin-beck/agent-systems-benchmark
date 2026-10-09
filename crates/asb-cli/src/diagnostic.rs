// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! The closed, privacy-safe diagnostic vocabulary used by the CLI boundary.
//!
//! A diagnostic is deliberately separate from the machine error envelope.  The
//! latter is a compatibility contract and must not gain host paths, operating
//! system text, credentials, prompts, or provider payloads.  The vocabulary is
//! nevertheless typed at the point where an error crosses the public boundary,
//! so human presentation can explain the affected kind of thing and the safe
//! next action without parsing free-form prose.

#![allow(missing_docs)]

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Severity {
    /// The request was rejected before the operation could begin.
    Error,
    /// An attempted operation did not complete.
    Failure,
    /// The operation completed or can continue with an important limitation.
    Warning,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Cause {
    MissingParent,
    MissingInput,
    AlreadyExists,
    NotDirectory,
    NotRegularFile,
    PermissionDenied,
    ReadOnlyStorage,
    UnsafeTopology,
    InvalidPath,
    MalformedInput,
    IncompatibleInput,
    StaleIdentity,
    UnavailableCapability,
    MissingTool,
    ProviderAuthentication,
    ProviderRejection,
    TransportFailure,
    Timeout,
    Cancellation,
    PartialCompletion,
    ReconciliationRequired,
    UnexpectedProductFailure,
    UnknownCause,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Subject {
    Parent,
    Input,
    Target,
    Option,
    Configuration,
    Tool,
    Provider,
    Capability,
    Transport,
    Run,
    Attempt,
    Store,
    Catalog,
    Workspace,
    Channel,
    Artifact,
    CredentialReference,
    Output,
    State,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Phase {
    Parse,
    Validate,
    Inspect,
    Prepare,
    Stage,
    Commit,
    Execute,
    Collect,
    Reconcile,
    Cleanup,
    Render,
    Discover,
    Authenticate,
    Transport,
    Replay,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum StateChange {
    NotStarted,
    Unchanged,
    Created,
    Updated,
    PartiallyCompleted,
    Completed,
    RolledBack,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Remediation {
    None,
    CorrectInput,
    CreateParent,
    CheckDestination,
    CheckPermissions,
    InstallTool,
    AuthenticateProvider,
    CheckProvider,
    Retry,
    Reconcile,
    RunDoctor,
    UseOfflineReplay,
}

/// Bounded context safe to use in human output.
///
/// Values are static catalog labels, never operating-system error text.  A
/// caller that has a dynamic user value must first normalize it to a bounded
/// display label and pass that label through its command-specific renderer;
/// this type intentionally cannot carry arbitrary runtime strings into the
/// machine envelope.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Context {
    pub subject: Subject,
    pub operation: &'static str,
    pub phase: Phase,
    pub state_change: StateChange,
    pub remediation: Remediation,
}

impl Context {
    const fn new(
        subject: Subject,
        operation: &'static str,
        phase: Phase,
        state_change: StateChange,
        remediation: Remediation,
    ) -> Self {
        Self {
            subject,
            operation,
            phase,
            state_change,
            remediation,
        }
    }
}

/// Typed public diagnostic identity and safe rendering context.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Diagnostic {
    pub code: &'static str,
    pub severity: Severity,
    pub cause: Cause,
    pub context: Context,
}

impl Diagnostic {
    /// Resolve every public code through the closed catalog.
    ///
    /// Unknown values are not silently accepted as a known generic error. They
    /// are explicitly marked `UnknownCause`, which makes an unreviewed new
    /// producer visible to tests and human detail output.
    pub fn for_code(code: &'static str, message: &'static str, severity: Severity) -> Self {
        let (cause, context) = classify(code, message);
        Self {
            code,
            severity,
            cause,
            context,
        }
    }

    pub fn is_catalogued(self) -> bool {
        self.cause != Cause::UnknownCause
    }

    /// Construct a typed warning for a result that completed with bounded
    /// partial coverage. This is used by successful result presenters rather
    /// than by rejected command errors.
    pub const fn partial_result(
        code: &'static str,
        subject: Subject,
        operation: &'static str,
    ) -> Self {
        Self {
            code,
            severity: Severity::Warning,
            cause: Cause::PartialCompletion,
            context: Context::new(
                subject,
                operation,
                Phase::Collect,
                StateChange::PartiallyCompleted,
                Remediation::Reconcile,
            ),
        }
    }

    /// Construct a typed warning for a completed operation with a known
    /// unavailable capability or dependency.
    pub const fn unavailable_warning(
        code: &'static str,
        subject: Subject,
        operation: &'static str,
    ) -> Self {
        Self {
            code,
            severity: Severity::Warning,
            cause: Cause::UnavailableCapability,
            context: Context::new(
                subject,
                operation,
                Phase::Inspect,
                StateChange::Completed,
                Remediation::RunDoctor,
            ),
        }
    }
}

fn classify(code: &'static str, message: &'static str) -> (Cause, Context) {
    // Stable operation codes from the provider and routed-TUI boundaries.
    match code {
        "provider_credential_unavailable" => {
            return (
                Cause::ProviderAuthentication,
                Context::new(
                    Subject::CredentialReference,
                    "authenticate",
                    Phase::Authenticate,
                    StateChange::NotStarted,
                    Remediation::AuthenticateProvider,
                ),
            );
        }
        "provider_transport_unavailable"
        | "provider_transport_failed"
        | "transfer_unavailable"
        | "transfer_failed"
        | "development_control_unavailable"
        | "development_channel_unavailable" => {
            return (
                Cause::TransportFailure,
                Context::new(
                    Subject::Transport,
                    "communicate",
                    Phase::Transport,
                    StateChange::NotStarted,
                    Remediation::CheckProvider,
                ),
            );
        }
        "provider_catalog_unavailable" | "provider_catalog_http_error" => {
            return (
                Cause::TransportFailure,
                Context::new(
                    Subject::Catalog,
                    "load_catalog",
                    Phase::Transport,
                    StateChange::Unchanged,
                    Remediation::Retry,
                ),
            );
        }
        "provider_http_error" | "provider_invalid_status" | "provider_response_too_large" => {
            return (
                Cause::ProviderRejection,
                Context::new(
                    Subject::Provider,
                    "request_provider",
                    Phase::Execute,
                    StateChange::Unchanged,
                    Remediation::CheckProvider,
                ),
            );
        }
        "trusted_tool_unavailable"
        | "host_capability_unavailable"
        | "signature_verifier_unavailable" => {
            return (
                Cause::UnavailableCapability,
                Context::new(
                    Subject::Capability,
                    "probe_capability",
                    Phase::Inspect,
                    StateChange::NotStarted,
                    Remediation::RunDoctor,
                ),
            );
        }
        "candidate_timeout" | "dev_command_timeout" | "development_launch_timeout" => {
            return (
                Cause::Timeout,
                Context::new(
                    Subject::Attempt,
                    "execute",
                    Phase::Execute,
                    StateChange::Unknown,
                    Remediation::Reconcile,
                ),
            );
        }
        "candidate_rejected_lifecycle" | "development_channel_rejected" | "rollback_rejected" => {
            return (
                Cause::ProviderRejection,
                Context::new(
                    Subject::State,
                    "change_state",
                    Phase::Validate,
                    StateChange::Unchanged,
                    Remediation::CorrectInput,
                ),
            );
        }
        "dev_source_identity_stale" => {
            return (
                Cause::StaleIdentity,
                Context::new(
                    Subject::Artifact,
                    "verify_identity",
                    Phase::Validate,
                    StateChange::Unchanged,
                    Remediation::Reconcile,
                ),
            );
        }
        "dev_source_identity_invalid"
        | "dev_source_identity_mismatch"
        | "manifest_digest_mismatch" => {
            return (
                Cause::IncompatibleInput,
                Context::new(
                    Subject::Artifact,
                    "verify_identity",
                    Phase::Validate,
                    StateChange::Unchanged,
                    Remediation::CorrectInput,
                ),
            );
        }
        "artifact_quota_exceeded" | "dev_workspace_quota_exceeded" => {
            return (
                Cause::ReadOnlyStorage,
                Context::new(
                    Subject::Workspace,
                    "write_workspace",
                    Phase::Stage,
                    StateChange::Unchanged,
                    Remediation::CheckDestination,
                ),
            );
        }
        _ => {}
    }

    if message_contains(message, "parent is unavailable")
        || message_contains(message, "has no parent")
        || message_contains(message, "parent cannot")
    {
        return (
            Cause::MissingParent,
            Context::new(
                Subject::Parent,
                "prepare_destination",
                Phase::Prepare,
                StateChange::NotStarted,
                Remediation::CreateParent,
            ),
        );
    }
    if message_contains(message, "is absent")
        || message_contains(message, "is missing")
        || message_contains(message, "missing ")
        || message_contains(message, "not installed")
        || message_contains(message, "no exact")
    {
        let subject = if message_contains(message, "tool") {
            Subject::Tool
        } else {
            Subject::Input
        };
        return (
            if subject == Subject::Tool {
                Cause::MissingTool
            } else {
                Cause::MissingInput
            },
            Context::new(
                subject,
                "inspect_input",
                Phase::Inspect,
                StateChange::NotStarted,
                if subject == Subject::Tool {
                    Remediation::InstallTool
                } else {
                    Remediation::CorrectInput
                },
            ),
        );
    }
    if message_contains(message, "already exists") || message_contains(message, "conflicting") {
        return (
            Cause::AlreadyExists,
            Context::new(
                Subject::Target,
                "create_target",
                Phase::Validate,
                StateChange::Unchanged,
                Remediation::CheckDestination,
            ),
        );
    }
    if message_contains(message, "symlink")
        || message_contains(message, "unsafe")
        || message_contains(message, "topology")
    {
        return (
            Cause::UnsafeTopology,
            Context::new(
                Subject::Target,
                "inspect_topology",
                Phase::Inspect,
                StateChange::Unchanged,
                Remediation::CheckDestination,
            ),
        );
    }
    if message_contains(message, "not a directory") {
        return (
            Cause::NotDirectory,
            Context::new(
                Subject::Target,
                "inspect_target",
                Phase::Inspect,
                StateChange::Unchanged,
                Remediation::CheckDestination,
            ),
        );
    }
    if message_contains(message, "regular file") || message_contains(message, "file cannot") {
        return (
            Cause::NotRegularFile,
            Context::new(
                Subject::Input,
                "inspect_input",
                Phase::Inspect,
                StateChange::Unchanged,
                Remediation::CheckDestination,
            ),
        );
    }
    if message_contains(message, "permission") || message_contains(message, "denied") {
        return (
            Cause::PermissionDenied,
            Context::new(
                Subject::Target,
                "access_target",
                Phase::Inspect,
                StateChange::Unchanged,
                Remediation::CheckPermissions,
            ),
        );
    }
    if message_contains(message, "read-only") || message_contains(message, "readonly") {
        return (
            Cause::ReadOnlyStorage,
            Context::new(
                Subject::Store,
                "write_store",
                Phase::Commit,
                StateChange::Unchanged,
                Remediation::CheckDestination,
            ),
        );
    }
    if message_contains(message, "path")
        && (message_contains(message, "invalid") || message_contains(message, "empty"))
    {
        return (
            Cause::InvalidPath,
            Context::new(
                Subject::Input,
                "validate_path",
                Phase::Validate,
                StateChange::NotStarted,
                Remediation::CorrectInput,
            ),
        );
    }
    if message_contains(message, "timeout") {
        return (
            Cause::Timeout,
            Context::new(
                Subject::Attempt,
                "execute",
                Phase::Execute,
                StateChange::Unknown,
                Remediation::Reconcile,
            ),
        );
    }
    if message_contains(message, "cancel") {
        return (
            Cause::Cancellation,
            Context::new(
                Subject::Attempt,
                "execute",
                Phase::Execute,
                StateChange::Unknown,
                Remediation::Reconcile,
            ),
        );
    }
    if message_contains(message, "reconcil") || message_contains(message, "recovery") {
        return (
            Cause::ReconciliationRequired,
            Context::new(
                Subject::State,
                "reconcile",
                Phase::Reconcile,
                StateChange::Unknown,
                Remediation::Reconcile,
            ),
        );
    }
    if message_contains(message, "invalid")
        || message_contains(message, "malformed")
        || message_contains(message, "syntax")
        || message_contains(message, "shape")
        || message_contains(message, "UTF-8")
        || message_contains(message, "canonical")
    {
        return (
            Cause::MalformedInput,
            Context::new(
                Subject::Input,
                "validate_input",
                Phase::Validate,
                StateChange::NotStarted,
                Remediation::CorrectInput,
            ),
        );
    }
    if message_contains(message, "incompatible")
        || message_contains(message, "unsupported")
        || message_contains(message, "mismatch")
    {
        return (
            Cause::IncompatibleInput,
            Context::new(
                Subject::Input,
                "validate_compatibility",
                Phase::Validate,
                StateChange::NotStarted,
                Remediation::CorrectInput,
            ),
        );
    }
    if message_contains(message, "failed")
        || message_contains(message, "cannot")
        || message_contains(message, "unavailable")
    {
        return (
            Cause::UnexpectedProductFailure,
            Context::new(
                Subject::Unknown,
                "operate",
                Phase::Execute,
                StateChange::Unknown,
                Remediation::Retry,
            ),
        );
    }
    (
        Cause::UnknownCause,
        Context::new(
            Subject::Unknown,
            "unknown",
            Phase::Validate,
            StateChange::Unknown,
            Remediation::None,
        ),
    )
}

fn message_contains(message: &str, needle: &str) -> bool {
    message
        .as_bytes()
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::{Cause, Diagnostic, Remediation, Severity, StateChange, Subject};

    #[test]
    fn required_fine_grained_causes_remain_distinct() {
        let cases = [
            (
                "project configuration parent is unavailable",
                Cause::MissingParent,
            ),
            ("tool ID is not installed", Cause::MissingTool),
            ("tool installation path is a symlink", Cause::UnsafeTopology),
            ("tool project path is invalid", Cause::InvalidPath),
            (
                "recording cassette syntax or shape is invalid",
                Cause::MalformedInput,
            ),
            (
                "provider selection is incompatible",
                Cause::IncompatibleInput,
            ),
            ("dev_source_identity_stale", Cause::StaleIdentity),
            ("trusted_tool_unavailable", Cause::UnavailableCapability),
            (
                "provider_credential_unavailable",
                Cause::ProviderAuthentication,
            ),
            ("provider_http_error", Cause::ProviderRejection),
            ("provider_transport_failed", Cause::TransportFailure),
            ("candidate_timeout", Cause::Timeout),
            ("operation cancelled", Cause::Cancellation),
            (
                "control reconciliation lost a run",
                Cause::ReconciliationRequired,
            ),
        ];
        for (message, expected) in cases {
            let diagnostic = Diagnostic::for_code(message, message, Severity::Failure);
            assert_eq!(diagnostic.cause, expected, "{message}");
        }
    }

    #[test]
    fn unknown_producers_are_explicitly_unclassified() {
        let diagnostic =
            Diagnostic::for_code("new_unreviewed_code", "new message", Severity::Error);
        assert_eq!(diagnostic.cause, Cause::UnknownCause);
        assert!(!diagnostic.is_catalogued());
        assert_eq!(diagnostic.context.state_change, StateChange::Unknown);
        assert_eq!(diagnostic.context.remediation, Remediation::None);
        assert_eq!(diagnostic.context.subject, Subject::Unknown);
    }

    #[test]
    fn context_contains_no_dynamic_or_private_error_text() {
        let diagnostic = Diagnostic::for_code(
            "tool source file cannot be read",
            "tool source file cannot be read",
            Severity::Failure,
        );
        assert_eq!(diagnostic.context.subject, Subject::Input);
        assert_eq!(diagnostic.context.operation, "inspect_input");
        assert_eq!(diagnostic.context.state_change, StateChange::Unchanged);
    }

    #[test]
    fn successful_partial_and_unavailable_results_have_explicit_warning_semantics() {
        let partial = Diagnostic::partial_result("recording_partial", Subject::Run, "record");
        assert_eq!(partial.severity, Severity::Warning);
        assert_eq!(partial.cause, Cause::PartialCompletion);
        assert_eq!(
            partial.context.state_change,
            StateChange::PartiallyCompleted
        );

        let unavailable = Diagnostic::unavailable_warning(
            "capability_unavailable",
            Subject::Capability,
            "doctor",
        );
        assert_eq!(unavailable.severity, Severity::Warning);
        assert_eq!(unavailable.cause, Cause::UnavailableCapability);
        assert_eq!(unavailable.context.state_change, StateChange::Completed);
    }
}
