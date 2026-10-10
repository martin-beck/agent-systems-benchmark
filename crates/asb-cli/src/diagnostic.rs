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

    /// Classify an ordinary CLI literal at its public boundary.  The CLI has a
    /// large established producer surface, so constructors must not reuse the
    /// broad machine-envelope families (`usage`, `validation`, `operation`) as
    /// their human diagnostic identity.  This resolver returns a closed
    /// semantic identity and deliberately leaves an unrecognised future
    /// literal uncatalogued for the mechanical contract test to reject.
    pub fn for_cli_literal(message: &'static str, severity: Severity) -> Self {
        let (cause, context) = classify_cli_literal(message);
        Self {
            code: cli_cause_code(cause),
            severity,
            cause,
            context,
        }
    }

    /// Resolve a quarantined legacy CLI producer through the checked-in,
    /// call-site-specific catalog.  `Location` is captured at the producer,
    /// so a new literal, multiline expression, or dynamic message cannot
    /// inherit a reviewed identity merely because its prose contains familiar
    /// words.
    #[track_caller]
    pub fn for_legacy_cli_callsite(severity: Severity) -> Self {
        let location = std::panic::Location::caller();
        let (code, cause) = legacy_cli_catalog(location.file(), location.line())
            .unwrap_or(("cli_unreviewed_legacy_producer", Cause::UnknownCause));
        Self {
            code,
            severity,
            cause,
            context: legacy_cli_context(cause),
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

const fn cli_cause_code(cause: Cause) -> &'static str {
    match cause {
        Cause::MissingParent => "cli_missing_parent",
        Cause::MissingInput => "cli_missing_input",
        Cause::AlreadyExists => "cli_already_exists",
        Cause::NotDirectory => "cli_not_directory",
        Cause::NotRegularFile => "cli_not_regular_file",
        Cause::PermissionDenied => "cli_permission_denied",
        Cause::ReadOnlyStorage => "cli_read_only_storage",
        Cause::ResourceExhausted => "cli_resource_exhausted",
        Cause::UnsafeTopology => "cli_unsafe_topology",
        Cause::InvalidPath => "cli_invalid_path",
        Cause::MalformedInput => "cli_malformed_input",
        Cause::IncompatibleInput => "cli_incompatible_input",
        Cause::StaleIdentity => "cli_stale_identity",
        Cause::UnavailableCapability => "cli_unavailable_capability",
        Cause::MissingTool => "cli_missing_tool",
        Cause::ProviderAuthentication => "cli_provider_authentication",
        Cause::ProviderRejection => "cli_provider_rejection",
        Cause::LifecycleRejected => "cli_lifecycle_rejected",
        Cause::TransportFailure => "cli_transport_failure",
        Cause::Timeout => "cli_timeout",
        Cause::Cancellation => "cli_cancellation",
        Cause::PartialCompletion => "cli_partial_completion",
        Cause::ReconciliationRequired => "cli_reconciliation_required",
        Cause::UnexpectedProductFailure => "cli_product_failure",
        Cause::UnknownCause => "cli_unclassified",
    }
}

const fn legacy_cli_context(cause: Cause) -> Context {
    match cause {
        Cause::MissingInput | Cause::MalformedInput | Cause::IncompatibleInput => Context::new(
            Subject::Input,
            "validate_cli_input",
            Phase::Validate,
            StateChange::NotStarted,
            Remediation::CorrectInput,
        ),
        Cause::MissingParent => Context::new(
            Subject::Parent,
            "inspect_cli_parent",
            Phase::Inspect,
            StateChange::NotStarted,
            Remediation::CreateParent,
        ),
        Cause::InvalidPath
        | Cause::AlreadyExists
        | Cause::NotDirectory
        | Cause::NotRegularFile
        | Cause::UnsafeTopology => Context::new(
            Subject::Target,
            "inspect_cli_target",
            Phase::Inspect,
            StateChange::NotStarted,
            Remediation::CheckDestination,
        ),
        Cause::PermissionDenied | Cause::ReadOnlyStorage | Cause::ResourceExhausted => {
            Context::new(
                Subject::Workspace,
                "prepare_cli_workspace",
                Phase::Prepare,
                StateChange::NotStarted,
                Remediation::CheckPermissions,
            )
        }
        Cause::ProviderAuthentication | Cause::ProviderRejection | Cause::TransportFailure => {
            Context::new(
                Subject::Provider,
                "communicate_provider",
                Phase::Transport,
                StateChange::NotStarted,
                Remediation::CheckProvider,
            )
        }
        Cause::Timeout | Cause::Cancellation | Cause::PartialCompletion => Context::new(
            Subject::Attempt,
            "execute_benchmark",
            Phase::Execute,
            StateChange::Unknown,
            Remediation::Reconcile,
        ),
        Cause::ReconciliationRequired | Cause::StaleIdentity | Cause::LifecycleRejected => {
            Context::new(
                Subject::State,
                "reconcile_state",
                Phase::Reconcile,
                StateChange::Unknown,
                Remediation::Reconcile,
            )
        }
        Cause::MissingTool | Cause::UnavailableCapability => Context::new(
            Subject::Capability,
            "inspect_capability",
            Phase::Discover,
            StateChange::NotStarted,
            Remediation::RunDoctor,
        ),
        Cause::UnexpectedProductFailure => Context::new(
            Subject::Workspace,
            "execute_cli_operation",
            Phase::Execute,
            StateChange::Unknown,
            Remediation::Reconcile,
        ),
        Cause::UnknownCause => Context::new(
            Subject::Unknown,
            "unknown",
            Phase::Inspect,
            StateChange::Unknown,
            Remediation::None,
        ),
    }
}

include!("diagnostic_legacy_catalog.rs");

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
        "dev_metadata_failed" => {
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
        "artifact_digest_mismatch" => {
            return (
                Cause::IncompatibleInput,
                Context::new(
                    Subject::Artifact,
                    "validate_digest",
                    Phase::Validate,
                    StateChange::Unchanged,
                    Remediation::CorrectInput,
                ),
            );
        }
        "artifact_invalid" | "artifact_set_incomplete" | "artifact_size_mismatch" => {
            return (
                Cause::MalformedInput,
                Context::new(
                    Subject::Artifact,
                    "validate_artifact",
                    Phase::Validate,
                    StateChange::Unchanged,
                    Remediation::CorrectInput,
                ),
            );
        }
        "artifact_too_large" => {
            return (
                Cause::ResourceExhausted,
                Context::new(
                    Subject::Artifact,
                    "receive_artifact",
                    Phase::Transport,
                    StateChange::Unchanged,
                    Remediation::CorrectInput,
                ),
            );
        }
        "cached_input_invalid" => {
            return (
                Cause::MalformedInput,
                Context::new(
                    Subject::Input,
                    "validate_cached_input",
                    Phase::Validate,
                    StateChange::Unchanged,
                    Remediation::CorrectInput,
                ),
            );
        }
        "cached_input_unavailable" => {
            return (
                Cause::MissingInput,
                Context::new(
                    Subject::Input,
                    "load_cached_input",
                    Phase::Inspect,
                    StateChange::NotStarted,
                    Remediation::CorrectInput,
                ),
            );
        }
        "channel_invalid" => {
            return (
                Cause::IncompatibleInput,
                Context::new(
                    Subject::Channel,
                    "validate_channel",
                    Phase::Validate,
                    StateChange::Unchanged,
                    Remediation::CorrectInput,
                ),
            );
        }
        "channel_unavailable" | "compatible_release_unavailable" => {
            return (
                Cause::UnavailableCapability,
                Context::new(
                    Subject::Channel,
                    "inspect_channel",
                    Phase::Inspect,
                    StateChange::NotStarted,
                    Remediation::RunDoctor,
                ),
            );
        }
        "component_invalid" => {
            return (
                Cause::MalformedInput,
                Context::new(
                    Subject::Configuration,
                    "validate_component",
                    Phase::Validate,
                    StateChange::Unchanged,
                    Remediation::CorrectInput,
                ),
            );
        }
        "dev_artifact_invalid" => {
            return (
                Cause::MalformedInput,
                Context::new(
                    Subject::Artifact,
                    "validate_development_artifact",
                    Phase::Validate,
                    StateChange::Unchanged,
                    Remediation::CorrectInput,
                ),
            );
        }
        "dev_cleanup_failed" => {
            return (
                Cause::UnexpectedProductFailure,
                Context::new(
                    Subject::Artifact,
                    "cleanup_development_artifact",
                    Phase::Commit,
                    StateChange::Unknown,
                    Remediation::Reconcile,
                ),
            );
        }
        "dev_command_failed" => {
            return (
                Cause::UnexpectedProductFailure,
                Context::new(
                    Subject::Attempt,
                    "execute_command",
                    Phase::Execute,
                    StateChange::Unknown,
                    Remediation::Reconcile,
                ),
            );
        }
        "dev_command_unavailable" => {
            return (
                Cause::UnavailableCapability,
                Context::new(
                    Subject::Tool,
                    "execute_command",
                    Phase::Inspect,
                    StateChange::NotStarted,
                    Remediation::RunDoctor,
                ),
            );
        }
        "dev_workspace_unavailable" => {
            return (
                Cause::UnavailableCapability,
                Context::new(
                    Subject::Workspace,
                    "inspect_workspace",
                    Phase::Inspect,
                    StateChange::NotStarted,
                    Remediation::RunDoctor,
                ),
            );
        }
        "dev_workspace_unsafe" => {
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
        "environment_path_invalid" => {
            return (
                Cause::InvalidPath,
                Context::new(
                    Subject::Input,
                    "validate_environment_path",
                    Phase::Validate,
                    StateChange::NotStarted,
                    Remediation::CorrectInput,
                ),
            );
        }
        "environment_terminal_invalid" => {
            return (
                Cause::IncompatibleInput,
                Context::new(
                    Subject::Capability,
                    "validate_terminal",
                    Phase::Validate,
                    StateChange::NotStarted,
                    Remediation::CorrectInput,
                ),
            );
        }
        "extension_not_installed" => {
            return (
                Cause::MissingTool,
                Context::new(
                    Subject::Tool,
                    "install_extension",
                    Phase::Prepare,
                    StateChange::NotStarted,
                    Remediation::InstallTool,
                ),
            );
        }
        "installation_verification_failed" => {
            return (
                Cause::UnexpectedProductFailure,
                Context::new(
                    Subject::Artifact,
                    "verify_installation",
                    Phase::Validate,
                    StateChange::Unknown,
                    Remediation::Reconcile,
                ),
            );
        }
        "license_policy_rejected" => {
            return (
                Cause::IncompatibleInput,
                Context::new(
                    Subject::Artifact,
                    "evaluate_license",
                    Phase::Validate,
                    StateChange::Unchanged,
                    Remediation::CorrectInput,
                ),
            );
        }
        "license_report_incomplete" | "license_report_invalid" | "license_report_missing" => {
            return (
                Cause::MalformedInput,
                Context::new(
                    Subject::Artifact,
                    "validate_license_report",
                    Phase::Validate,
                    StateChange::Unchanged,
                    Remediation::CorrectInput,
                ),
            );
        }
        "lifecycle_busy" => {
            return (
                Cause::LifecycleRejected,
                Context::new(
                    Subject::State,
                    "coordinate_lifecycle",
                    Phase::Prepare,
                    StateChange::Unknown,
                    Remediation::Reconcile,
                ),
            );
        }
        "manifest_invalid" => {
            return (
                Cause::MalformedInput,
                Context::new(
                    Subject::Artifact,
                    "validate_manifest",
                    Phase::Validate,
                    StateChange::Unchanged,
                    Remediation::CorrectInput,
                ),
            );
        }
        "offline_artifact_unavailable" => {
            return (
                Cause::MissingInput,
                Context::new(
                    Subject::Artifact,
                    "load_offline_artifact",
                    Phase::Inspect,
                    StateChange::NotStarted,
                    Remediation::UseOfflineReplay,
                ),
            );
        }
        "output_unavailable" => {
            return (
                Cause::UnavailableCapability,
                Context::new(
                    Subject::Target,
                    "write_output",
                    Phase::Commit,
                    StateChange::NotStarted,
                    Remediation::CheckDestination,
                ),
            );
        }
        "provenance_invalid" | "provenance_missing" => {
            return (
                Cause::MalformedInput,
                Context::new(
                    Subject::Artifact,
                    "validate_provenance",
                    Phase::Validate,
                    StateChange::Unchanged,
                    Remediation::CorrectInput,
                ),
            );
        }
        "redirect_rejected" => {
            return (
                Cause::IncompatibleInput,
                Context::new(
                    Subject::Transport,
                    "validate_redirect",
                    Phase::Validate,
                    StateChange::NotStarted,
                    Remediation::CorrectInput,
                ),
            );
        }
        "release_invalid" => {
            return (
                Cause::MalformedInput,
                Context::new(
                    Subject::Channel,
                    "validate_release",
                    Phase::Validate,
                    StateChange::Unchanged,
                    Remediation::CorrectInput,
                ),
            );
        }
        "sbom_invalid" | "sbom_missing" => {
            return (
                Cause::MalformedInput,
                Context::new(
                    Subject::Artifact,
                    "validate_sbom",
                    Phase::Validate,
                    StateChange::Unchanged,
                    Remediation::CorrectInput,
                ),
            );
        }
        "signature_invalid" => {
            return (
                Cause::IncompatibleInput,
                Context::new(
                    Subject::Artifact,
                    "verify_signature",
                    Phase::Validate,
                    StateChange::Unchanged,
                    Remediation::CorrectInput,
                ),
            );
        }
        "state_path_invalid" => {
            return (
                Cause::InvalidPath,
                Context::new(
                    Subject::State,
                    "validate_state_path",
                    Phase::Validate,
                    StateChange::Unchanged,
                    Remediation::CorrectInput,
                ),
            );
        }
        "state_unavailable" => {
            return (
                Cause::UnavailableCapability,
                Context::new(
                    Subject::State,
                    "inspect_state",
                    Phase::Inspect,
                    StateChange::Unknown,
                    Remediation::RunDoctor,
                ),
            );
        }
        "state_write_failed" => {
            return (
                Cause::UnexpectedProductFailure,
                Context::new(
                    Subject::State,
                    "write_state",
                    Phase::Commit,
                    StateChange::Unknown,
                    Remediation::Reconcile,
                ),
            );
        }
        "synthetic_interruption" => {
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
        "system_clock_invalid" => {
            return (
                Cause::UnavailableCapability,
                Context::new(
                    Subject::Capability,
                    "inspect_clock",
                    Phase::Inspect,
                    StateChange::NotStarted,
                    Remediation::RunDoctor,
                ),
            );
        }
        "trust_anchor_invalid" => {
            return (
                Cause::IncompatibleInput,
                Context::new(
                    Subject::CredentialReference,
                    "validate_trust_anchor",
                    Phase::Validate,
                    StateChange::Unchanged,
                    Remediation::CorrectInput,
                ),
            );
        }
        "unexpected_document" => {
            return (
                Cause::MalformedInput,
                Context::new(
                    Subject::Artifact,
                    "parse_document",
                    Phase::Validate,
                    StateChange::Unchanged,
                    Remediation::CorrectInput,
                ),
            );
        }
        "unexpected_range" => {
            return (
                Cause::MalformedInput,
                Context::new(
                    Subject::Option,
                    "validate_range",
                    Phase::Validate,
                    StateChange::NotStarted,
                    Remediation::CorrectInput,
                ),
            );
        }
        "xdg_root_invalid" => {
            return (
                Cause::InvalidPath,
                Context::new(
                    Subject::Workspace,
                    "validate_xdg_root",
                    Phase::Validate,
                    StateChange::NotStarted,
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

fn classify_cli_literal(message: &'static str) -> (Cause, Context) {
    let classified = classify("cli_literal", message);
    if classified.0 != Cause::UnknownCause
        && !(classified.0 == Cause::UnexpectedProductFailure
            && classified.1.subject == Subject::Unknown)
    {
        return classified;
    }

    let context = |subject, operation, phase, remediation| {
        Context::new(
            subject,
            operation,
            phase,
            StateChange::NotStarted,
            remediation,
        )
    };
    if message_contains(message, "requires")
        || message_contains(message, "is missing")
        || message_contains(message, "has no ")
        || message_contains(message, "lacks ")
    {
        return (
            Cause::MissingInput,
            context(
                Subject::Input,
                "validate_input",
                Phase::Validate,
                Remediation::CorrectInput,
            ),
        );
    }
    if message_contains(message, "must be")
        || message_contains(message, "option")
        || message_contains(message, "arguments")
        || message_contains(message, "supplied twice")
        || message_contains(message, "out of range")
    {
        return (
            Cause::MalformedInput,
            context(
                Subject::Option,
                "validate_option",
                Phase::Validate,
                Remediation::CorrectInput,
            ),
        );
    }
    if message_contains(message, "overflow") || message_contains(message, "exceeds its bound") {
        return (
            Cause::ResourceExhausted,
            context(
                Subject::Input,
                "bound_input",
                Phase::Validate,
                Remediation::CorrectInput,
            ),
        );
    }
    if message_contains(message, "stale") || message_contains(message, "changed before") {
        return (
            Cause::StaleIdentity,
            context(
                Subject::State,
                "verify_identity",
                Phase::Validate,
                Remediation::Reconcile,
            ),
        );
    }
    if message_contains(message, "provider") || message_contains(message, "OpenRouter") {
        return (
            Cause::TransportFailure,
            context(
                Subject::Provider,
                "communicate",
                Phase::Transport,
                Remediation::CheckProvider,
            ),
        );
    }
    if message_contains(message, "configuration") || message_contains(message, "selection") {
        return (
            Cause::UnexpectedProductFailure,
            context(
                Subject::Configuration,
                "configure",
                Phase::Commit,
                Remediation::CorrectInput,
            ),
        );
    }
    if message_contains(message, "recording")
        || message_contains(message, "replay")
        || message_contains(message, "cassette")
        || message_contains(message, "experiment")
        || message_contains(message, "plan")
    {
        return (
            Cause::UnexpectedProductFailure,
            context(
                Subject::Artifact,
                "validate_artifact",
                Phase::Validate,
                Remediation::Reconcile,
            ),
        );
    }
    if message_contains(message, "workload") || message_contains(message, "evaluation") {
        return (
            Cause::UnavailableCapability,
            context(
                Subject::Capability,
                "prepare_workload",
                Phase::Prepare,
                Remediation::RunDoctor,
            ),
        );
    }
    if message_contains(message, "auth") || message_contains(message, "relay") {
        return (
            Cause::TransportFailure,
            context(
                Subject::Transport,
                "communicate",
                Phase::Transport,
                Remediation::CheckProvider,
            ),
        );
    }
    if message_contains(message, "signal")
        || message_contains(message, "SIGINT")
        || message_contains(message, "SIGTERM")
        || message_contains(message, "clock")
        || message_contains(message, "adapter")
        || message_contains(message, "rust")
    {
        return (
            Cause::UnavailableCapability,
            context(
                Subject::Capability,
                "probe_capability",
                Phase::Inspect,
                Remediation::RunDoctor,
            ),
        );
    }
    if message_contains(message, "API key")
        || message_contains(message, "api-key")
        || message_contains(message, "--json")
    {
        return (
            Cause::MalformedInput,
            context(
                Subject::Option,
                "validate_option",
                Phase::Validate,
                Remediation::CorrectInput,
            ),
        );
    }
    if message_contains(message, "workspace") {
        return (
            Cause::UnexpectedProductFailure,
            context(
                Subject::Workspace,
                "prepare_workspace",
                Phase::Cleanup,
                Remediation::Reconcile,
            ),
        );
    }
    if message_contains(message, "result store") {
        return (
            Cause::UnavailableCapability,
            context(
                Subject::Store,
                "open_store",
                Phase::Inspect,
                Remediation::RunDoctor,
            ),
        );
    }
    if message_contains(message, "rollback target") {
        return (
            Cause::MissingInput,
            context(
                Subject::State,
                "select_rollback",
                Phase::Inspect,
                Remediation::CorrectInput,
            ),
        );
    }
    if message_contains(message, "completion") || message_contains(message, "project init") {
        return (
            Cause::IncompatibleInput,
            context(
                Subject::Option,
                "validate_command",
                Phase::Validate,
                Remediation::CorrectInput,
            ),
        );
    }
    if message_contains(message, "redaction") || message_contains(message, "local/mock owner") {
        return (
            Cause::UnavailableCapability,
            context(
                Subject::Capability,
                "probe_capability",
                Phase::Inspect,
                Remediation::RunDoctor,
            ),
        );
    }
    if message_contains(message, "easy build") || message_contains(message, "execution definition")
    {
        return (
            Cause::UnexpectedProductFailure,
            context(
                Subject::Artifact,
                "prepare_artifact",
                Phase::Prepare,
                Remediation::Reconcile,
            ),
        );
    }
    if message_contains(message, "tool") {
        return (
            Cause::UnavailableCapability,
            context(
                Subject::Tool,
                "inspect_tool",
                Phase::Inspect,
                Remediation::InstallTool,
            ),
        );
    }
    if message_contains(message, "output") || message_contains(message, "directory") {
        return (
            Cause::UnexpectedProductFailure,
            context(
                Subject::Output,
                "write_output",
                Phase::Commit,
                Remediation::CheckDestination,
            ),
        );
    }
    if message_contains(message, "agent")
        || message_contains(message, "process")
        || message_contains(message, "prompt")
    {
        return (
            Cause::UnexpectedProductFailure,
            context(
                Subject::Attempt,
                "execute",
                Phase::Execute,
                Remediation::Reconcile,
            ),
        );
    }
    if message_contains(message, "state")
        || message_contains(message, "journal")
        || message_contains(message, "run ")
    {
        return (
            Cause::UnexpectedProductFailure,
            context(
                Subject::State,
                "operate",
                Phase::Commit,
                Remediation::Reconcile,
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
    ("dev_metadata_failed", Cause::UnexpectedProductFailure),
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
    ("artifact_digest_mismatch", Cause::IncompatibleInput),
    ("artifact_invalid", Cause::MalformedInput),
    ("artifact_set_incomplete", Cause::MalformedInput),
    ("artifact_size_mismatch", Cause::MalformedInput),
    ("artifact_too_large", Cause::ResourceExhausted),
    ("cached_input_invalid", Cause::MalformedInput),
    ("cached_input_unavailable", Cause::MissingInput),
    ("channel_invalid", Cause::IncompatibleInput),
    ("channel_unavailable", Cause::UnavailableCapability),
    (
        "compatible_release_unavailable",
        Cause::UnavailableCapability,
    ),
    ("component_invalid", Cause::MalformedInput),
    ("dev_artifact_invalid", Cause::MalformedInput),
    ("dev_cleanup_failed", Cause::UnexpectedProductFailure),
    ("dev_command_failed", Cause::UnexpectedProductFailure),
    ("dev_command_unavailable", Cause::UnavailableCapability),
    ("dev_workspace_unavailable", Cause::UnavailableCapability),
    ("dev_workspace_unsafe", Cause::UnsafeTopology),
    ("environment_path_invalid", Cause::InvalidPath),
    ("environment_terminal_invalid", Cause::IncompatibleInput),
    ("extension_not_installed", Cause::MissingTool),
    (
        "installation_verification_failed",
        Cause::UnexpectedProductFailure,
    ),
    ("license_policy_rejected", Cause::IncompatibleInput),
    ("license_report_incomplete", Cause::MalformedInput),
    ("license_report_invalid", Cause::MalformedInput),
    ("license_report_missing", Cause::MalformedInput),
    ("lifecycle_busy", Cause::LifecycleRejected),
    ("manifest_invalid", Cause::MalformedInput),
    ("offline_artifact_unavailable", Cause::MissingInput),
    ("output_unavailable", Cause::UnavailableCapability),
    ("provenance_invalid", Cause::MalformedInput),
    ("provenance_missing", Cause::MalformedInput),
    ("redirect_rejected", Cause::IncompatibleInput),
    ("release_invalid", Cause::MalformedInput),
    ("sbom_invalid", Cause::MalformedInput),
    ("sbom_missing", Cause::MalformedInput),
    ("signature_invalid", Cause::IncompatibleInput),
    ("state_path_invalid", Cause::InvalidPath),
    ("state_unavailable", Cause::UnavailableCapability),
    ("state_write_failed", Cause::UnexpectedProductFailure),
    ("synthetic_interruption", Cause::Cancellation),
    ("system_clock_invalid", Cause::UnavailableCapability),
    ("trust_anchor_invalid", Cause::IncompatibleInput),
    ("unexpected_document", Cause::MalformedInput),
    ("unexpected_range", Cause::MalformedInput),
    ("xdg_root_invalid", Cause::InvalidPath),
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
        CATALOGUED_CODES, Cause, Context, Diagnostic, LEGACY_CLI_CATALOG, Remediation, Severity,
        StateChange, Subject, cli_cause_code, legacy_cli_catalog, legacy_cli_context,
    };
    use std::hint::black_box;

    #[test]
    fn every_legacy_producer_location_resolves_to_its_reviewed_identity() {
        for &(file, line, expected_code, expected_cause) in LEGACY_CLI_CATALOG {
            assert_eq!(
                legacy_cli_catalog(file, line),
                Some((expected_code, expected_cause)),
                "legacy diagnostic catalog entry {file}:{line}"
            );
        }
        assert_eq!(legacy_cli_catalog("unknown.rs", 0), None);
    }

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
                cause: black_box(cause),
                context: Context::new(
                    Subject::Target,
                    "test",
                    super::Phase::Execute,
                    StateChange::Unknown,
                    Remediation::None,
                ),
            };
            assert!(!black_box(diagnostic).cause_explanation().is_empty());
            assert!(!black_box(diagnostic).subject_label().is_empty());
            assert!(!black_box(diagnostic).state_change_explanation().is_empty());
            assert!(!black_box(diagnostic).remediation_explanation().is_empty());
            assert!(black_box(cli_cause_code(cause)).starts_with("cli_"));
            assert!(!legacy_cli_context(black_box(cause)).operation.is_empty());
        }
    }

    #[test]
    fn public_rendering_covers_every_closed_context_dimension_at_runtime() {
        let subjects = [
            Subject::Parent,
            Subject::Input,
            Subject::Target,
            Subject::Option,
            Subject::Configuration,
            Subject::Tool,
            Subject::Provider,
            Subject::Capability,
            Subject::Transport,
            Subject::Run,
            Subject::Attempt,
            Subject::Store,
            Subject::Catalog,
            Subject::Workspace,
            Subject::Channel,
            Subject::Artifact,
            Subject::CredentialReference,
            Subject::Output,
            Subject::State,
            Subject::Unknown,
        ];
        let states = [
            StateChange::NotStarted,
            StateChange::Unchanged,
            StateChange::Created,
            StateChange::Updated,
            StateChange::PartiallyCompleted,
            StateChange::Completed,
            StateChange::RolledBack,
            StateChange::Unknown,
        ];
        let remediations = [
            Remediation::None,
            Remediation::CorrectInput,
            Remediation::CreateParent,
            Remediation::CheckDestination,
            Remediation::CheckPermissions,
            Remediation::InstallTool,
            Remediation::AuthenticateProvider,
            Remediation::CheckProvider,
            Remediation::Retry,
            Remediation::Reconcile,
            Remediation::RunDoctor,
            Remediation::UseOfflineReplay,
        ];

        for subject in subjects {
            let diagnostic = Diagnostic {
                code: "runtime-subject",
                severity: Severity::Failure,
                cause: Cause::UnexpectedProductFailure,
                context: Context::new(
                    black_box(subject),
                    "render_subject",
                    super::Phase::Inspect,
                    StateChange::Unchanged,
                    Remediation::None,
                ),
            };
            assert!(!black_box(diagnostic).subject_label().is_empty());
        }
        for state_change in states {
            let diagnostic = Diagnostic {
                code: "runtime-state",
                severity: Severity::Failure,
                cause: Cause::UnexpectedProductFailure,
                context: Context::new(
                    Subject::State,
                    "render_state",
                    super::Phase::Inspect,
                    black_box(state_change),
                    Remediation::None,
                ),
            };
            assert!(!black_box(diagnostic).state_change_explanation().is_empty());
        }
        for remediation in remediations {
            let diagnostic = Diagnostic {
                code: "runtime-remediation",
                severity: Severity::Failure,
                cause: Cause::UnexpectedProductFailure,
                context: Context::new(
                    Subject::State,
                    "render_remediation",
                    super::Phase::Inspect,
                    StateChange::Unchanged,
                    black_box(remediation),
                ),
            };
            assert!(!black_box(diagnostic).remediation_explanation().is_empty());
        }
    }

    #[test]
    fn ordinary_cli_literals_select_their_specific_runtime_contexts() {
        let cases = [
            ("requires input", Cause::MissingInput, Subject::Input),
            ("option value", Cause::MalformedInput, Subject::Option),
            ("overflow", Cause::ResourceExhausted, Subject::Input),
            ("stale selection", Cause::StaleIdentity, Subject::State),
            ("provider route", Cause::TransportFailure, Subject::Provider),
            (
                "configuration selection",
                Cause::UnexpectedProductFailure,
                Subject::Configuration,
            ),
            (
                "recording artifact",
                Cause::UnexpectedProductFailure,
                Subject::Artifact,
            ),
            (
                "workload profile",
                Cause::UnavailableCapability,
                Subject::Capability,
            ),
            ("auth relay", Cause::TransportFailure, Subject::Transport),
            (
                "signal adapter",
                Cause::UnavailableCapability,
                Subject::Capability,
            ),
            ("API key", Cause::MalformedInput, Subject::Option),
            (
                "workspace root",
                Cause::UnexpectedProductFailure,
                Subject::Workspace,
            ),
            ("result store", Cause::UnavailableCapability, Subject::Store),
            ("rollback target", Cause::MissingInput, Subject::State),
            (
                "completion request",
                Cause::IncompatibleInput,
                Subject::Option,
            ),
            (
                "redaction policy",
                Cause::UnavailableCapability,
                Subject::Capability,
            ),
            (
                "easy build",
                Cause::UnexpectedProductFailure,
                Subject::Artifact,
            ),
            ("tool catalog", Cause::UnavailableCapability, Subject::Tool),
            (
                "output destination",
                Cause::UnexpectedProductFailure,
                Subject::Output,
            ),
            (
                "agent process",
                Cause::UnexpectedProductFailure,
                Subject::Attempt,
            ),
            (
                "state journal",
                Cause::UnexpectedProductFailure,
                Subject::State,
            ),
            ("opaque literal", Cause::UnknownCause, Subject::Unknown),
        ];

        for (message, expected_cause, expected_subject) in cases {
            let diagnostic =
                Diagnostic::for_cli_literal(black_box(message), black_box(Severity::Failure));
            assert_eq!(diagnostic.cause, expected_cause, "{message}");
            assert_eq!(diagnostic.context.subject, expected_subject, "{message}");
            assert!(diagnostic.is_catalogued() || expected_cause == Cause::UnknownCause);
        }
    }

    #[test]
    fn unreviewed_codes_preserve_specific_runtime_message_classification() {
        let cases = [
            (
                "parent is unavailable",
                Cause::MissingParent,
                Subject::Parent,
            ),
            ("tool is not installed", Cause::MissingTool, Subject::Tool),
            ("input is missing", Cause::MissingInput, Subject::Input),
            (
                "target already exists",
                Cause::AlreadyExists,
                Subject::Target,
            ),
            ("unsafe symlink", Cause::UnsafeTopology, Subject::Target),
            (
                "target is not a directory",
                Cause::NotDirectory,
                Subject::Target,
            ),
            (
                "input regular file cannot be opened",
                Cause::NotRegularFile,
                Subject::Input,
            ),
            ("access denied", Cause::PermissionDenied, Subject::Target),
            ("read-only store", Cause::ReadOnlyStorage, Subject::Store),
            ("path is invalid", Cause::InvalidPath, Subject::Input),
            ("request timed out", Cause::Timeout, Subject::Attempt),
            ("operation cancelled", Cause::Cancellation, Subject::Attempt),
            (
                "reconciliation required",
                Cause::ReconciliationRequired,
                Subject::State,
            ),
            ("malformed syntax", Cause::MalformedInput, Subject::Input),
            (
                "incompatible selection",
                Cause::IncompatibleInput,
                Subject::Input,
            ),
            ("deadline expired", Cause::Timeout, Subject::Attempt),
            (
                "operation failed",
                Cause::UnexpectedProductFailure,
                Subject::Unknown,
            ),
            ("opaque prose", Cause::UnknownCause, Subject::Unknown),
        ];

        for (message, expected_cause, expected_subject) in cases {
            let diagnostic = Diagnostic::for_code(
                black_box("runtime_unreviewed_code"),
                black_box(message),
                black_box(Severity::Failure),
            );
            assert_eq!(diagnostic.cause, expected_cause, "{message}");
            assert_eq!(diagnostic.context.subject, expected_subject, "{message}");
        }
    }

    #[test]
    fn every_catalogued_producer_has_an_explicit_cause_and_context() {
        for &(code, expected_cause) in CATALOGUED_CODES {
            let diagnostic = Diagnostic::for_code(
                black_box(code),
                black_box(code),
                black_box(Severity::Failure),
            );
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
