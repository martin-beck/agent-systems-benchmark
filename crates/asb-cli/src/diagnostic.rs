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

/// Diagnostic severity at the public command boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Severity {
    /// The request was rejected before the operation could begin.
    Error,
    /// An attempted operation did not complete.
    Failure,
    /// The operation completed or can continue with an important limitation.
    Warning,
}

/// Fine-grained stable cause at the public command boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Cause {
    /// A required parent directory does not exist.
    MissingParent,
    /// A required input is absent.
    MissingInput,
    /// The destination already exists and cannot be replaced.
    AlreadyExists,
    /// A directory was required but another object was found.
    NotDirectory,
    /// A regular file was required but another object was found.
    NotRegularFile,
    /// Access was denied by the storage boundary.
    PermissionDenied,
    /// Storage refused mutation because it is read-only or full.
    ReadOnlyStorage,
    /// A bounded storage or workspace quota was exhausted.
    ResourceExhausted,
    /// A symlink or overlapping root makes the topology unsafe.
    UnsafeTopology,
    /// A supplied path is empty or otherwise invalid.
    InvalidPath,
    /// Input bytes or shape are malformed.
    MalformedInput,
    /// Input is valid but incompatible with the selected contract.
    IncompatibleInput,
    /// An identity was valid previously but is now stale.
    StaleIdentity,
    /// A required host or product capability is unavailable.
    UnavailableCapability,
    /// A required executable or registered tool is missing.
    MissingTool,
    /// Provider authentication could not be established.
    ProviderAuthentication,
    /// A provider rejected an otherwise bounded request.
    ProviderRejection,
    /// The lifecycle owner rejected a requested state transition.
    LifecycleRejected,
    /// Transport could not complete a bounded exchange.
    TransportFailure,
    /// A bounded deadline expired.
    Timeout,
    /// The operation was cancelled before completion.
    Cancellation,
    /// The operation completed only part of its requested work.
    PartialCompletion,
    /// Durable state must be inspected before retrying.
    ReconciliationRequired,
    /// The product could not complete an attempted operation.
    UnexpectedProductFailure,
    /// No reviewed mapping can determine a narrower cause.
    UnknownCause,
}

/// Safe role of the subject named by a diagnostic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Subject {
    /// Parent directory of a destination.
    Parent,
    /// User or fixture input.
    Input,
    /// Destination or selected target.
    Target,
    /// User-supplied option.
    Option,
    /// Configuration document or selection.
    Configuration,
    /// Installed or required tool.
    Tool,
    /// External model provider.
    Provider,
    /// Host or product capability.
    Capability,
    /// Network or local transport.
    Transport,
    /// Durable benchmark run.
    Run,
    /// Individual execution attempt.
    Attempt,
    /// Result store or journal.
    Store,
    /// Versioned catalog.
    Catalog,
    /// Private execution workspace.
    Workspace,
    /// TUI or release channel.
    Channel,
    /// Checked-in or transferred artifact.
    Artifact,
    /// Opaque credential reference.
    CredentialReference,
    /// Command output destination.
    Output,
    /// Durable lifecycle state.
    State,
    /// No safe subject could be identified.
    Unknown,
}

/// Bounded phase at which a diagnostic was produced.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Phase {
    /// Parse command arguments or framing.
    Parse,
    /// Validate user input or compatibility.
    Validate,
    /// Inspect existing state or topology.
    Inspect,
    /// Prepare a bounded operation.
    Prepare,
    /// Stage a candidate before publication.
    Stage,
    /// Commit a durable state change.
    Commit,
    /// Execute the requested operation.
    Execute,
    /// Collect result or measurement evidence.
    Collect,
    /// Reconcile uncertain durable state.
    Reconcile,
    /// Remove temporary or owned state.
    Cleanup,
    /// Render a public response.
    Render,
    /// Discover an installed capability or tool.
    Discover,
    /// Establish provider authentication.
    Authenticate,
    /// Exchange bounded transport data.
    Transport,
    /// Consume a strict offline replay.
    Replay,
}

/// State-change result associated with a diagnostic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum StateChange {
    /// No external operation was started.
    NotStarted,
    /// The operation left state unchanged.
    Unchanged,
    /// A new state or artifact was created.
    Created,
    /// Existing state was updated.
    Updated,
    /// Only part of the requested work completed.
    PartiallyCompleted,
    /// The requested state change completed.
    Completed,
    /// A staged change was rolled back.
    RolledBack,
    /// The state could not be determined safely.
    Unknown,
}

/// Bounded operator action appropriate to the diagnostic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Remediation {
    /// No safe automatic next action is known.
    None,
    /// Correct the supplied input or option.
    CorrectInput,
    /// Create or select the required parent.
    CreateParent,
    /// Inspect the destination before retrying.
    CheckDestination,
    /// Check access rights without exposing private error text.
    CheckPermissions,
    /// Install or register the missing tool.
    InstallTool,
    /// Establish provider authentication through the supported flow.
    AuthenticateProvider,
    /// Check bounded provider reachability or response.
    CheckProvider,
    /// Retry the bounded operation.
    Retry,
    /// Reconcile durable state before retrying.
    Reconcile,
    /// Run the bounded diagnostic command.
    RunDoctor,
    /// Select an already sealed offline replay.
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
    /// Safe resource role, never a raw path or OS error.
    pub subject: Subject,
    /// Stable operation label from the catalog.
    pub operation: &'static str,
    /// Bounded operation phase.
    pub phase: Phase,
    /// Durable state-change result.
    pub state_change: StateChange,
    /// Validated operator remediation.
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
    /// Stable producer code retained for detail presentation.
    pub code: &'static str,
    /// Error, failure, or warning semantics.
    pub severity: Severity,
    /// Fine-grained reviewed cause.
    pub cause: Cause,
    /// Safe context needed by human presentation.
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

    /// Return whether this code resolved to a reviewed non-fallback cause.
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

    /// Return a concise, cause-specific explanation suitable for human output.
    ///
    /// These phrases are deliberately stable and do not include operating
    /// system text.  The producer message remains available as bounded detail,
    /// while this explanation prevents a known cause from collapsing into a
    /// generic "failed" or "unavailable" sentence.
    pub const fn cause_explanation(self) -> &'static str {
        match self.cause {
            Cause::MissingParent => "the required parent directory does not exist",
            Cause::MissingInput => "the required input is missing",
            Cause::AlreadyExists => "the destination already exists and was not replaced",
            Cause::NotDirectory => "the selected path is not a directory",
            Cause::NotRegularFile => "the selected path is not a regular file",
            Cause::PermissionDenied => "access to the affected resource was denied",
            Cause::ReadOnlyStorage => "the destination storage refused the requested change",
            Cause::ResourceExhausted => "the bounded storage or workspace quota was exhausted",
            Cause::UnsafeTopology => "the path topology is unsafe for this operation",
            Cause::InvalidPath => "the supplied path is invalid",
            Cause::MalformedInput => "the input is malformed",
            Cause::IncompatibleInput => "the input is incompatible with this command",
            Cause::StaleIdentity => "the selected identity is stale",
            Cause::UnavailableCapability => "a required host or product capability is unavailable",
            Cause::MissingTool => "the required tool is not installed or registered",
            Cause::ProviderAuthentication => "provider authentication could not be established",
            Cause::ProviderRejection => "the provider rejected the bounded request",
            Cause::LifecycleRejected => "the lifecycle owner rejected the requested transition",
            Cause::TransportFailure => "the bounded transport exchange did not complete",
            Cause::Timeout => "the bounded deadline expired",
            Cause::Cancellation => "the operation was cancelled before completion",
            Cause::PartialCompletion => "only part of the requested work completed",
            Cause::ReconciliationRequired => "durable state must be reconciled before retrying",
            Cause::UnexpectedProductFailure => "ASB could not complete the attempted operation",
            Cause::UnknownCause => {
                "the cause is not yet classified by the reviewed diagnostic catalog"
            }
        }
    }

    /// Return a safe subject label for human output.
    pub const fn subject_label(self) -> &'static str {
        match self.context.subject {
            Subject::Parent => "parent directory",
            Subject::Input => "input",
            Subject::Target => "target",
            Subject::Option => "option",
            Subject::Configuration => "configuration",
            Subject::Tool => "tool",
            Subject::Provider => "provider",
            Subject::Capability => "capability",
            Subject::Transport => "transport",
            Subject::Run => "run",
            Subject::Attempt => "attempt",
            Subject::Store => "result store",
            Subject::Catalog => "catalog",
            Subject::Workspace => "workspace",
            Subject::Channel => "channel",
            Subject::Artifact => "artifact",
            Subject::CredentialReference => "credential reference",
            Subject::Output => "output",
            Subject::State => "durable state",
            Subject::Unknown => "affected resource",
        }
    }

    /// Return a safe statement of what changed, if anything.
    pub const fn state_change_explanation(self) -> &'static str {
        match self.context.state_change {
            StateChange::NotStarted => "No external operation was started.",
            StateChange::Unchanged => "No relevant state was changed.",
            StateChange::Created => "The requested state was created.",
            StateChange::Updated => "Existing state was updated.",
            StateChange::PartiallyCompleted => "Only part of the requested state was produced.",
            StateChange::Completed => "The requested state change completed.",
            StateChange::RolledBack => "The staged state change was rolled back.",
            StateChange::Unknown => "The resulting durable state is not safe to assume.",
        }
    }

    /// Return the concrete correction associated with the reviewed class.
    pub const fn remediation_explanation(self) -> &'static str {
        match self.context.remediation {
            Remediation::None => {
                "Do not retry automatically; inspect the reported operation and correct the underlying condition."
            }
            Remediation::CorrectInput => "Correct the named input or option and rerun the command.",
            Remediation::CreateParent => {
                "Create or select the named parent directory, then rerun the command."
            }
            Remediation::CheckDestination => {
                "Inspect the named destination and correct its type or unsafe topology before retrying."
            }
            Remediation::CheckPermissions => {
                "Check access to the named destination without exposing private operating-system details."
            }
            Remediation::InstallTool => {
                "Install or register the named tool through the supported ASB tool workflow."
            }
            Remediation::AuthenticateProvider => {
                "Establish the provider credential through the supported ASB authentication flow."
            }
            Remediation::CheckProvider => {
                "Check the selected provider and bounded transport availability; no request was retried automatically."
            }
            Remediation::Retry => {
                "Retry only after confirming that the bounded operation is safe and the condition has cleared."
            }
            Remediation::Reconcile => {
                "Inspect durable state with the supported doctor/reconciliation workflow before retrying."
            }
            Remediation::RunDoctor => {
                "Run `asb doctor` to inspect the required capability before retrying."
            }
            Remediation::UseOfflineReplay => {
                "Select an already sealed offline replay instead of contacting the provider."
            }
        }
    }
}

fn classify(code: &'static str, message: &'static str) -> (Cause, Context) {
    // Stable operation codes from the provider and routed-TUI boundaries.
    match code {
        "development_filesystem_invalid" | "development_installation_invalid" => {
            return (
                Cause::UnsafeTopology,
                Context::new(
                    Subject::Workspace,
                    "inspect_workspace",
                    Phase::Inspect,
                    StateChange::Unchanged,
                    Remediation::CheckDestination,
                ),
            );
        }
        "development_source_identity_invalid"
        | "development_source_identity_mismatch"
        | "development_bundle_invalid"
        | "development_channel_rejected" => {
            return (
                Cause::IncompatibleInput,
                Context::new(
                    Subject::Artifact,
                    "validate_development_artifact",
                    Phase::Validate,
                    StateChange::Unchanged,
                    Remediation::CorrectInput,
                ),
            );
        }
        "dev_source_identity_unknown" => {
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
        "development_source_unavailable_offline"
        | "development_control_unavailable"
        | "development_channel_unavailable"
        | "development_control_failed" => {
            return (
                Cause::TransportFailure,
                Context::new(
                    Subject::Transport,
                    "communicate",
                    Phase::Transport,
                    StateChange::Unchanged,
                    Remediation::CheckProvider,
                ),
            );
        }
        "development_terminal_unavailable"
        | "development_compiler_unsupported"
        | "development_host_unavailable" => {
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
        "development_launch_timeout" => {
            return (
                Cause::Timeout,
                Context::new(
                    Subject::Attempt,
                    "launch",
                    Phase::Execute,
                    StateChange::Unknown,
                    Remediation::Reconcile,
                ),
            );
        }
        "development_descriptor_oversized" => {
            return (
                Cause::MalformedInput,
                Context::new(
                    Subject::Artifact,
                    "validate_descriptor",
                    Phase::Validate,
                    StateChange::Unchanged,
                    Remediation::CorrectInput,
                ),
            );
        }
        "development_operation_invalid" => {
            return (
                Cause::InvalidPath,
                Context::new(
                    Subject::Option,
                    "validate_operation",
                    Phase::Validate,
                    StateChange::NotStarted,
                    Remediation::CorrectInput,
                ),
            );
        }
        "development_launch_failed"
        | "development_remove_failed"
        | "development_descriptor_failed"
        | "development_metadata_failed" => {
            return (
                Cause::UnexpectedProductFailure,
                Context::new(
                    Subject::Artifact,
                    "operate",
                    Phase::Execute,
                    StateChange::Unknown,
                    Remediation::Reconcile,
                ),
            );
        }
        "candidate_execution_failed" => {
            return (
                Cause::UnexpectedProductFailure,
                Context::new(
                    Subject::Attempt,
                    "execute_candidate",
                    Phase::Execute,
                    StateChange::Unknown,
                    Remediation::Reconcile,
                ),
            );
        }
        "candidate_request_failed" | "artifact_transfer_failed" => {
            return (
                Cause::TransportFailure,
                Context::new(
                    Subject::Transport,
                    "communicate",
                    Phase::Transport,
                    StateChange::Unknown,
                    Remediation::Reconcile,
                ),
            );
        }
        "candidate_response_invalid" => {
            return (
                Cause::MalformedInput,
                Context::new(
                    Subject::Artifact,
                    "validate_response",
                    Phase::Validate,
                    StateChange::Unchanged,
                    Remediation::CorrectInput,
                ),
            );
        }
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
        | "transfer_failed" => {
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
        "trusted_tool_invalid" => {
            return (
                Cause::IncompatibleInput,
                Context::new(
                    Subject::Tool,
                    "validate_tool",
                    Phase::Validate,
                    StateChange::Unchanged,
                    Remediation::CorrectInput,
                ),
            );
        }
        "transfer_too_large" => {
            return (
                Cause::MalformedInput,
                Context::new(
                    Subject::Artifact,
                    "receive_transfer",
                    Phase::Transport,
                    StateChange::Unchanged,
                    Remediation::CorrectInput,
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
        "candidate_timeout" | "dev_command_timeout" => {
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
        "candidate_rejected_lifecycle" => {
            return (
                Cause::LifecycleRejected,
                Context::new(
                    Subject::State,
                    "change_state",
                    Phase::Commit,
                    StateChange::Unknown,
                    Remediation::Reconcile,
                ),
            );
        }
        "rollback_rejected" => {
            return (
                Cause::LifecycleRejected,
                Context::new(
                    Subject::State,
                    "rollback",
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
                Cause::ResourceExhausted,
                Context::new(
                    Subject::Workspace,
                    "write_workspace",
                    Phase::Stage,
                    StateChange::Unchanged,
                    Remediation::CheckDestination,
                ),
            );
        }
        "rollback_state_invalid" => {
            return (
                Cause::MalformedInput,
                Context::new(
                    Subject::State,
                    "validate_rollback_state",
                    Phase::Validate,
                    StateChange::Unchanged,
                    Remediation::CorrectInput,
                ),
            );
        }
        "rollback_state_failed" => {
            return (
                Cause::UnexpectedProductFailure,
                Context::new(
                    Subject::State,
                    "write_rollback_state",
                    Phase::Commit,
                    StateChange::Unknown,
                    Remediation::Reconcile,
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
        || message_contains(message, "not found")
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
    if message_contains(message, "timeout") || message_contains(message, "timed out") {
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
        || message_contains(message, "corrupt")
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
        || message_contains(message, "rejected")
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
    if message_contains(message, "deadline") {
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

/// Stable routed producers with an explicit reviewed catalog mapping.
///
/// Keeping this inventory beside the resolver makes omissions test-visible;
/// future public producers must add a row and a semantic fixture rather than
/// silently falling through to a generic message.
pub const CATALOGUED_CODES: &[(&str, Cause)] = &[
    ("development_filesystem_invalid", Cause::UnsafeTopology),
    ("development_installation_invalid", Cause::UnsafeTopology),
    (
        "development_source_identity_invalid",
        Cause::IncompatibleInput,
    ),
    (
        "development_source_identity_mismatch",
        Cause::IncompatibleInput,
    ),
    ("dev_source_identity_unknown", Cause::IncompatibleInput),
    ("development_bundle_invalid", Cause::IncompatibleInput),
    ("development_channel_rejected", Cause::IncompatibleInput),
    (
        "development_source_unavailable_offline",
        Cause::TransportFailure,
    ),
    ("development_control_unavailable", Cause::TransportFailure),
    ("development_channel_unavailable", Cause::TransportFailure),
    ("development_control_failed", Cause::TransportFailure),
    (
        "development_terminal_unavailable",
        Cause::UnavailableCapability,
    ),
    (
        "development_compiler_unsupported",
        Cause::UnavailableCapability,
    ),
    ("development_host_unavailable", Cause::UnavailableCapability),
    ("development_launch_timeout", Cause::Timeout),
    ("development_descriptor_oversized", Cause::MalformedInput),
    ("development_operation_invalid", Cause::InvalidPath),
    ("development_launch_failed", Cause::UnexpectedProductFailure),
    ("development_remove_failed", Cause::UnexpectedProductFailure),
    (
        "development_descriptor_failed",
        Cause::UnexpectedProductFailure,
    ),
    (
        "development_metadata_failed",
        Cause::UnexpectedProductFailure,
    ),
    (
        "candidate_execution_failed",
        Cause::UnexpectedProductFailure,
    ),
    ("candidate_request_failed", Cause::TransportFailure),
    ("candidate_response_invalid", Cause::MalformedInput),
    (
        "provider_credential_unavailable",
        Cause::ProviderAuthentication,
    ),
    ("provider_transport_unavailable", Cause::TransportFailure),
    ("provider_transport_failed", Cause::TransportFailure),
    ("transfer_unavailable", Cause::TransportFailure),
    ("transfer_failed", Cause::TransportFailure),
    ("provider_catalog_unavailable", Cause::TransportFailure),
    ("provider_catalog_http_error", Cause::TransportFailure),
    ("provider_http_error", Cause::ProviderRejection),
    ("provider_invalid_status", Cause::ProviderRejection),
    ("provider_response_too_large", Cause::ProviderRejection),
    ("trusted_tool_unavailable", Cause::UnavailableCapability),
    ("host_capability_unavailable", Cause::UnavailableCapability),
    (
        "signature_verifier_unavailable",
        Cause::UnavailableCapability,
    ),
    ("trusted_tool_invalid", Cause::IncompatibleInput),
    ("transfer_too_large", Cause::MalformedInput),
    ("artifact_transfer_failed", Cause::TransportFailure),
    ("candidate_timeout", Cause::Timeout),
    ("dev_command_timeout", Cause::Timeout),
    ("candidate_rejected_lifecycle", Cause::LifecycleRejected),
    ("rollback_rejected", Cause::LifecycleRejected),
    ("dev_source_identity_stale", Cause::StaleIdentity),
    ("dev_source_identity_invalid", Cause::IncompatibleInput),
    ("dev_source_identity_mismatch", Cause::IncompatibleInput),
    ("manifest_digest_mismatch", Cause::IncompatibleInput),
    ("artifact_quota_exceeded", Cause::ResourceExhausted),
    ("dev_workspace_quota_exceeded", Cause::ResourceExhausted),
    ("rollback_state_invalid", Cause::MalformedInput),
    ("rollback_state_failed", Cause::UnexpectedProductFailure),
];

fn message_contains(message: &str, needle: &str) -> bool {
    message
        .as_bytes()
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::{
        CATALOGUED_CODES, Cause, Context, Diagnostic, Remediation, Severity, StateChange, Subject,
    };

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

    #[test]
    fn every_catalogue_cause_has_specific_subject_state_and_recovery_text() {
        let causes = [
            Cause::MissingParent,
            Cause::MissingInput,
            Cause::AlreadyExists,
            Cause::NotDirectory,
            Cause::NotRegularFile,
            Cause::PermissionDenied,
            Cause::ReadOnlyStorage,
            Cause::ResourceExhausted,
            Cause::UnsafeTopology,
            Cause::InvalidPath,
            Cause::MalformedInput,
            Cause::IncompatibleInput,
            Cause::StaleIdentity,
            Cause::UnavailableCapability,
            Cause::MissingTool,
            Cause::ProviderAuthentication,
            Cause::ProviderRejection,
            Cause::LifecycleRejected,
            Cause::TransportFailure,
            Cause::Timeout,
            Cause::Cancellation,
            Cause::PartialCompletion,
            Cause::ReconciliationRequired,
            Cause::UnexpectedProductFailure,
            Cause::UnknownCause,
        ];
        for cause in causes {
            let diagnostic = Diagnostic {
                code: "catalog-test",
                severity: Severity::Failure,
                cause,
                context: Context::new(
                    Subject::Target,
                    "test",
                    super::Phase::Execute,
                    StateChange::Unknown,
                    Remediation::None,
                ),
            };
            assert!(!diagnostic.cause_explanation().is_empty());
            assert!(!diagnostic.subject_label().is_empty());
            assert!(!diagnostic.state_change_explanation().is_empty());
            assert!(!diagnostic.remediation_explanation().is_empty());
        }
    }

    #[test]
    fn every_catalogued_producer_has_an_explicit_cause_and_context() {
        for &(code, expected_cause) in CATALOGUED_CODES {
            let diagnostic = Diagnostic::for_code(code, code, Severity::Failure);
            assert_eq!(
                diagnostic.cause, expected_cause,
                "catalog mapping for {code}"
            );
            assert!(diagnostic.is_catalogued(), "catalogued producer {code}");
            assert_ne!(diagnostic.context.subject, Subject::Unknown, "{code}");
            assert_ne!(diagnostic.context.operation, "unknown", "{code}");
            assert!(!diagnostic.cause_explanation().is_empty(), "{code}");
            assert!(!diagnostic.remediation_explanation().is_empty(), "{code}");
        }
    }

    #[test]
    fn lifecycle_and_resource_failures_keep_their_fine_grained_causes() {
        let lifecycle = Diagnostic::for_code(
            "candidate_rejected_lifecycle",
            "candidate_rejected_lifecycle",
            Severity::Failure,
        );
        assert_eq!(lifecycle.cause, Cause::LifecycleRejected);
        assert_ne!(lifecycle.cause, Cause::ProviderRejection);
        assert_eq!(lifecycle.context.remediation, Remediation::Reconcile);

        let quota = Diagnostic::for_code(
            "dev_workspace_quota_exceeded",
            "dev_workspace_quota_exceeded",
            Severity::Error,
        );
        assert_eq!(quota.cause, Cause::ResourceExhausted);
        assert_ne!(quota.cause, Cause::ReadOnlyStorage);
        assert_eq!(quota.context.subject, Subject::Workspace);
    }

    #[test]
    fn routed_development_codes_do_not_collapse_to_unknown_cause() {
        for code in [
            "development_filesystem_invalid",
            "development_installation_invalid",
            "development_bundle_invalid",
            "development_channel_rejected",
            "development_control_unavailable",
            "development_launch_timeout",
            "development_descriptor_oversized",
            "development_launch_failed",
            "development_remove_failed",
        ] {
            let diagnostic = Diagnostic::for_code(code, code, Severity::Failure);
            assert!(diagnostic.is_catalogued(), "{code}");
        }
    }
}
