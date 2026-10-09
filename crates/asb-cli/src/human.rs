// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Human-first presentation for the public command-line interface.
//!
//! Structured command results remain the authority. This module has an
//! explicit entry for every public command family and deliberately has no
//! recursive JSON fallback: adding a command requires choosing its human
//! presentation as well.

use super::diagnostic::Remediation;
use super::{CliError, ErrorRemediation, HumanErrorClass};
use serde::Deserialize;
use serde_json::{Map, Value};
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::Path;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const DEFAULT_WIDTH: usize = 80;
const MIN_WIDTH: usize = 40;
const MAX_WIDTH: usize = 120;

/// Out-of-band facts produced by command execution for human presentation.
///
/// This context is deliberately absent from the public JSON contract. The
/// execution owner populates it from already-validated state; the renderer
/// never reopens plans or stores while formatting a result.
#[derive(Debug, Default, Eq, PartialEq)]
pub(super) enum PresentationContext {
    #[default]
    None,
    Plan {
        sweep_available: bool,
    },
    Execution {
        run_refs: Vec<String>,
    },
    ProviderCatalog {
        setup_profiles: Vec<String>,
    },
}

impl PresentationContext {
    pub(super) fn set_plan(&mut self, sweep_available: bool) {
        *self = Self::Plan { sweep_available };
    }

    pub(super) fn set_execution(&mut self, run_refs: Vec<String>) {
        *self = Self::Execution { run_refs };
    }

    pub(super) fn set_provider_catalog(&mut self, setup_profiles: Vec<String>) {
        *self = Self::ProviderCatalog { setup_profiles };
    }
}

/// Authoritative public top-level command inventory for dispatch, help, and
/// human-presentation exhaustiveness. Legacy doctor/completion projections are
/// independently pinned to their compatibility fixtures.
pub(super) const PUBLIC_COMMANDS: &[&str] = &[
    "doctor",
    "setup",
    "capabilities",
    "project",
    "tool",
    "provider-catalog",
    "adapter-catalog",
    "workload-catalog",
    "easy",
    "tui",
    "config",
    "auth",
    "provider-plan",
    "completion",
    "plan",
    "run",
    "sweep",
    "benchmark-live",
    "compare",
    "report",
    "serve",
    "record",
    "record-live",
    "record-campaign",
    "replay",
    "replay-offline",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CommandKind {
    Cli,
    Help,
    Version,
    Doctor,
    Setup,
    Easy,
    Tui,
    Capabilities,
    Project,
    Tool,
    ProviderCatalog,
    AdapterCatalog,
    WorkloadCatalog,
    Config,
    Auth,
    ProviderPlan,
    Completion,
    Plan,
    Run,
    Sweep,
    BenchmarkLive,
    Serve,
    Compare,
    Report,
    Record,
    RecordLive,
    RecordCampaign,
    Replay,
    ReplayOffline,
    InternalOpenCodeBatch,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum EasyKind {
    Help,
    Setup,
    Lifecycle,
    ProviderCatalog,
    AdapterCatalog,
    Plan,
    Run,
    Sweep,
    Report,
    Compare,
    Record,
    RecordLive,
    RecordCampaign,
    Replay,
    ReplayOffline,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TuiKind {
    Launch,
    Preflight,
    Status,
    Doctor,
    Install,
    Upgrade,
    Remove,
    LiveProvider,
    DynamicCatalog,
    Help,
    Version,
    Unknown,
}

impl TuiKind {
    const fn result_operation(self) -> Option<&'static str> {
        match self {
            Self::Launch => Some("launch"),
            Self::Preflight => Some("preflight"),
            Self::Status => Some("status"),
            Self::Doctor => Some("doctor"),
            Self::Install => Some("install"),
            Self::Upgrade => Some("upgrade"),
            Self::Remove => Some("remove"),
            Self::LiveProvider => Some("live_provider"),
            Self::DynamicCatalog => Some("dynamic_catalog"),
            Self::Help | Self::Version | Self::Unknown => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AuthKind {
    Setup,
    Enroll,
    Status,
    Rotate,
    Revoke,
    Unknown,
}

impl AuthKind {
    const fn request_method(self) -> Option<&'static str> {
        match self {
            Self::Enroll => Some("auth_enroll"),
            Self::Status => Some("auth_status"),
            Self::Rotate => Some("auth_rotate"),
            Self::Revoke => Some("auth_revoke"),
            Self::Setup | Self::Unknown => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum InvocationKind {
    Cli,
    Help,
    Version,
    Doctor,
    Setup,
    Easy(EasyKind),
    Tui(TuiKind),
    Capabilities,
    ProjectInit,
    Tool,
    ProviderCatalog { refresh: bool },
    AdapterCatalog,
    WorkloadCatalog,
    ConfigOpenrouter,
    Auth(AuthKind),
    ProviderPlan,
    Completion,
    Plan { create: bool },
    Run,
    Sweep,
    BenchmarkLive,
    Serve,
    Compare,
    Report,
    Record,
    RecordLive,
    RecordCampaign,
    Replay,
    ReplayOffline { local_mock: bool },
    InternalOpenCodeBatch,
}

impl InvocationKind {
    pub(super) fn classify_args(args: &[OsString]) -> Self {
        let words = args
            .iter()
            .filter_map(|arg| arg.to_str())
            .collect::<Vec<_>>();
        Self::classify_words(&words)
    }

    pub(super) fn classify_words(words: &[&str]) -> Self {
        let first = words.first().copied();
        match first {
            None | Some("--help" | "-h") => Self::Help,
            Some("--version" | "-V") => Self::Version,
            Some("doctor") => Self::Doctor,
            Some("setup") => Self::Setup,
            Some("easy") => Self::Easy(match words.get(1).copied() {
                None | Some("--help" | "-h") => EasyKind::Help,
                Some("setup") => EasyKind::Setup,
                Some(
                    "build" | "install" | "update" | "test" | "status" | "rollback" | "remove",
                ) => EasyKind::Lifecycle,
                Some("provider-catalog") => EasyKind::ProviderCatalog,
                Some("adapter-catalog") => EasyKind::AdapterCatalog,
                Some("plan") => EasyKind::Plan,
                Some("run") => EasyKind::Run,
                Some("sweep") => EasyKind::Sweep,
                Some("report") => EasyKind::Report,
                Some("compare") => EasyKind::Compare,
                Some("record") => EasyKind::Record,
                Some("record-live") => EasyKind::RecordLive,
                Some("record-campaign") => EasyKind::RecordCampaign,
                Some("replay") => EasyKind::Replay,
                Some("replay-offline") => EasyKind::ReplayOffline,
                Some(_) => EasyKind::Unknown,
            }),
            Some("tui") => Self::Tui(match words.get(1).copied() {
                None | Some("launch") => TuiKind::Launch,
                Some("preflight") => TuiKind::Preflight,
                Some("status") => TuiKind::Status,
                Some("doctor") => TuiKind::Doctor,
                Some("install") => TuiKind::Install,
                Some("upgrade") => TuiKind::Upgrade,
                Some("remove") => TuiKind::Remove,
                Some("live-provider") => TuiKind::LiveProvider,
                Some("dynamic-catalog") => TuiKind::DynamicCatalog,
                Some("--help" | "-h") => TuiKind::Help,
                Some("--version" | "-V") => TuiKind::Version,
                Some(_) => TuiKind::Unknown,
            }),
            Some("capabilities") => Self::Capabilities,
            Some("project") => Self::ProjectInit,
            Some("tool") => Self::Tool,
            Some("provider-catalog") => Self::ProviderCatalog {
                refresh: words.contains(&"--refresh"),
            },
            Some("adapter-catalog") => Self::AdapterCatalog,
            Some("workload-catalog") => Self::WorkloadCatalog,
            Some("config") if words.get(1) == Some(&"openrouter") => Self::ConfigOpenrouter,
            Some("config") => Self::Cli,
            Some("auth") => Self::Auth(match words.get(1).copied() {
                Some("setup") => AuthKind::Setup,
                Some("enroll") => AuthKind::Enroll,
                Some("status") => AuthKind::Status,
                Some("rotate") => AuthKind::Rotate,
                Some("revoke") => AuthKind::Revoke,
                _ => AuthKind::Unknown,
            }),
            Some("provider-plan") => Self::ProviderPlan,
            Some("completion") => Self::Completion,
            Some("plan") => Self::Plan {
                create: words.get(1) == Some(&"create"),
            },
            Some("run") => Self::Run,
            Some("sweep") => Self::Sweep,
            Some("benchmark-live") => Self::BenchmarkLive,
            Some("serve") => Self::Serve,
            Some("compare") => Self::Compare,
            Some("report") => Self::Report,
            Some("record") => Self::Record,
            Some("record-live") => Self::RecordLive,
            Some("record-campaign") => Self::RecordCampaign,
            Some("replay") => Self::Replay,
            Some("replay-offline") => Self::ReplayOffline {
                local_mock: words.contains(&"--local-mock"),
            },
            Some("internal-opencode-batch") => Self::InternalOpenCodeBatch,
            Some(_) => Self::Cli,
        }
    }

    const fn family(self) -> CommandKind {
        match self {
            Self::Cli => CommandKind::Cli,
            Self::Help => CommandKind::Help,
            Self::Version => CommandKind::Version,
            Self::Doctor => CommandKind::Doctor,
            Self::Setup | Self::Easy(EasyKind::Setup) => CommandKind::Setup,
            Self::Tui(_) => CommandKind::Tui,
            Self::Capabilities => CommandKind::Capabilities,
            Self::ProjectInit => CommandKind::Project,
            Self::Tool => CommandKind::Tool,
            Self::ProviderCatalog { .. } | Self::Easy(EasyKind::ProviderCatalog) => {
                CommandKind::ProviderCatalog
            }
            Self::AdapterCatalog | Self::Easy(EasyKind::AdapterCatalog) => {
                CommandKind::AdapterCatalog
            }
            Self::WorkloadCatalog => CommandKind::WorkloadCatalog,
            Self::ConfigOpenrouter => CommandKind::Config,
            Self::Auth(_) => CommandKind::Auth,
            Self::ProviderPlan => CommandKind::ProviderPlan,
            Self::Completion => CommandKind::Completion,
            Self::Plan { .. } | Self::Easy(EasyKind::Plan) => CommandKind::Plan,
            Self::Run | Self::Easy(EasyKind::Run) => CommandKind::Run,
            Self::Sweep | Self::Easy(EasyKind::Sweep) => CommandKind::Sweep,
            Self::BenchmarkLive => CommandKind::BenchmarkLive,
            Self::Serve => CommandKind::Serve,
            Self::Compare | Self::Easy(EasyKind::Compare) => CommandKind::Compare,
            Self::Report | Self::Easy(EasyKind::Report) => CommandKind::Report,
            Self::Record | Self::Easy(EasyKind::Record) => CommandKind::Record,
            Self::RecordLive | Self::Easy(EasyKind::RecordLive) => CommandKind::RecordLive,
            Self::RecordCampaign | Self::Easy(EasyKind::RecordCampaign) => {
                CommandKind::RecordCampaign
            }
            Self::Replay | Self::Easy(EasyKind::Replay) => CommandKind::Replay,
            Self::ReplayOffline { .. } | Self::Easy(EasyKind::ReplayOffline) => {
                CommandKind::ReplayOffline
            }
            Self::InternalOpenCodeBatch => CommandKind::InternalOpenCodeBatch,
            Self::Easy(EasyKind::Help | EasyKind::Lifecycle | EasyKind::Unknown) => {
                CommandKind::Easy
            }
        }
    }

    const fn is_raw(self) -> bool {
        matches!(
            self,
            Self::Help
                | Self::Version
                | Self::Completion
                | Self::Easy(EasyKind::Help)
                | Self::Tui(TuiKind::Help | TuiKind::Version)
                | Self::Serve
                | Self::Tool
                | Self::InternalOpenCodeBatch
        )
    }
}

impl CommandKind {
    fn parse(args: &[OsString]) -> io::Result<Self> {
        let family = InvocationKind::classify_args(args).family();
        if family == Self::Cli {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "public command has no human presentation",
            ))
        } else {
            Ok(family)
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Cli => "command",
            Self::Help => "help",
            Self::Version => "version",
            Self::Doctor => "doctor",
            Self::Setup => "setup",
            Self::Easy => "easy",
            Self::Tui => "tui",
            Self::Capabilities => "capabilities",
            Self::Project => "project initialization",
            Self::Tool => "tool",
            Self::ProviderCatalog => "provider catalog",
            Self::AdapterCatalog => "adapter catalog",
            Self::WorkloadCatalog => "workload catalog",
            Self::Config => "configuration",
            Self::Auth => "authentication",
            Self::ProviderPlan => "provider plan",
            Self::Completion => "completion",
            Self::Plan => "plan",
            Self::Run => "benchmark run",
            Self::Sweep => "benchmark sweep",
            Self::BenchmarkLive => "live benchmark",
            Self::Serve => "control service",
            Self::Compare => "comparison",
            Self::Report => "report",
            Self::Record => "recording",
            Self::RecordLive => "live recording",
            Self::RecordCampaign => "recording campaign",
            Self::Replay => "replay",
            Self::ReplayOffline => "offline replay",
            Self::InternalOpenCodeBatch => "internal adapter run",
        }
    }

    fn raw(self, args: &[OsString]) -> bool {
        InvocationKind::classify_args(args).is_raw()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum OutcomeKind {
    #[default]
    Succeeded,
    Warning,
    Partial,
    UserError,
    HostLimitation,
    ProductFailure,
    Cancelled,
}

#[derive(Debug, Default, Eq, PartialEq)]
struct Presentation {
    kind: OutcomeKind,
    outcome: String,
    facts: Vec<String>,
    warnings: Vec<String>,
    next: Option<NextAction>,
}

#[derive(Debug, Eq, PartialEq)]
struct NextAction {
    argv: Vec<String>,
}

impl NextAction {
    fn new(argv: impl IntoIterator<Item = impl Into<String>>) -> Option<Self> {
        let argv = argv.into_iter().map(Into::into).collect::<Vec<_>>();
        if argv.first().is_none_or(|word| word != "asb")
            || argv.iter().any(|word| {
                word.is_empty()
                    || word
                        .chars()
                        .any(|character| character.is_control() || character == '\0')
            })
        {
            return None;
        }
        Some(Self { argv })
    }

    fn render(&self) -> String {
        self.argv
            .iter()
            .map(|word| shell_quote(word))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[derive(Deserialize)]
struct DoctorView {
    #[serde(rename = "schema_version")]
    _schema_version: u16,
    ok: bool,
    command: String,
    linux: bool,
    architecture: String,
    procfs: bool,
    cgroup_v2: bool,
}

#[derive(Deserialize)]
struct SetupView {
    #[serde(rename = "schema_version")]
    _schema_version: u16,
    ok: bool,
    command: String,
    mode: String,
    persistent_change: bool,
    persisted: bool,
    selected_agents: Vec<String>,
    provider_profile: Option<String>,
    model: Option<String>,
}

#[derive(Deserialize)]
struct ProjectInitView {
    #[serde(rename = "schema_version")]
    _schema_version: u16,
    ok: bool,
    command: String,
    initialized: bool,
    recovered: bool,
    config: String,
    results: String,
    catalogs: String,
}

#[derive(Deserialize)]
struct LifecycleView {
    #[serde(rename = "schema_version")]
    _schema_version: u16,
    ok: bool,
    command: String,
    operation: String,
    channel: String,
    dry_run: bool,
    installed: bool,
    message: String,
}

#[derive(Deserialize)]
struct TuiView {
    #[serde(rename = "schema_version")]
    _schema_version: u64,
    ok: bool,
    command: String,
    operation: String,
    classification: Option<String>,
    #[serde(default)]
    code: Option<String>,
    channel: Option<String>,
    #[serde(default)]
    warnings: Vec<TuiWarningView>,
}

#[derive(Deserialize)]
enum TuiWarningView {
    #[serde(rename = "development_missing_authentication_allowed")]
    AuthenticationMissing,
    #[serde(rename = "development_missing_signatures_allowed")]
    UnsignedArtifacts,
    #[serde(rename = "development_missing_key_management_allowed")]
    KeyManagementMissing,
    #[serde(rename = "development_rustup_permission_or_ownership_findings_allowed")]
    RustupPermissions,
}

#[derive(Deserialize)]
struct CatalogEnvelope {
    #[serde(rename = "schema_version")]
    _schema_version: u16,
    ok: bool,
    command: String,
}

#[derive(Deserialize)]
struct ProviderCatalogView {
    #[serde(flatten)]
    envelope: CatalogEnvelope,
    agents: Vec<String>,
    profiles: Vec<ProviderCatalogEntryView>,
    openrouter_free_models: Vec<String>,
}

#[derive(Deserialize)]
struct ProviderCatalogEntryView {
    id: String,
    model: String,
    selectable: bool,
    unavailable_reason: Option<String>,
}

#[derive(Deserialize)]
struct AdapterCatalogView {
    #[serde(flatten)]
    envelope: CatalogEnvelope,
    adapters: Vec<AdapterCatalogEntryView>,
}

#[derive(Deserialize)]
struct AdapterCatalogEntryView {
    id: String,
    providers: Vec<String>,
    availability: String,
}

#[derive(Deserialize)]
struct WorkloadCatalogView {
    #[serde(flatten)]
    envelope: CatalogEnvelope,
    entries: Vec<WorkloadCatalogEntryView>,
}

#[derive(Deserialize)]
struct WorkloadCatalogEntryView {
    id: String,
    kind: String,
    availability: WorkloadAvailabilityView,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum WorkloadAvailabilityView {
    Available,
    FixtureOnly,
    Unavailable,
}

#[derive(Deserialize)]
struct ConfigView {
    #[serde(rename = "schema_version")]
    _schema_version: u16,
    ok: bool,
    command: String,
    provider: String,
    model: String,
}

#[derive(Deserialize)]
struct AuthSetupView {
    #[serde(rename = "schema_version")]
    _schema_version: u16,
    ok: bool,
    command: String,
    configured: bool,
    persisted: bool,
}

#[derive(Deserialize)]
struct JsonRpcView {
    jsonrpc: String,
    id: Value,
    #[serde(default)]
    result: Option<AuthResultView>,
    #[serde(default)]
    error: Option<Value>,
    #[serde(default)]
    method: Option<String>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
enum AuthResultView {
    Acknowledged(AuthAcknowledgementView),
    AuthStatus(AuthStatusValueView),
}

#[derive(Deserialize)]
struct AuthAcknowledgementView {
    accepted: bool,
}

#[derive(Deserialize)]
struct AuthStatusValueView {
    provider: String,
    endpoint_identity_sha256: String,
    credential_locator_sha256: String,
    generation: u64,
    status: String,
}

#[derive(Deserialize)]
struct ProviderPlanView {
    #[serde(rename = "schema_version")]
    _schema_version: u16,
    ok: bool,
    command: String,
    dry_run: bool,
    provider_profile: String,
    model: String,
    effective: Vec<EffectiveAgentView>,
}

#[derive(Deserialize)]
struct EffectiveAgentView {
    agent: String,
    profile_sha256: String,
    api_mode: EffectiveApiModeView,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum EffectiveApiModeView {
    ChatCompletions,
    Responses,
}

#[derive(Deserialize)]
struct PlanView {
    #[serde(rename = "schema_version")]
    _schema_version: u16,
    ok: bool,
    command: String,
    run_id: String,
    workload: String,
    agent_implementation: String,
}

#[derive(Deserialize)]
struct PlanCreateView {
    #[serde(rename = "schema_version")]
    _schema_version: u16,
    ok: bool,
    command: String,
    plan: String,
    workload: String,
    experiment_sha256: String,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PointDecisionView {
    Pass,
    Fail,
    Inconclusive,
}

#[derive(Deserialize)]
struct PointView {
    decision: PointDecisionView,
}

#[derive(Deserialize)]
struct ExecutionView {
    #[serde(rename = "schema_version")]
    _schema_version: u16,
    ok: bool,
    command: String,
    run_ids: Vec<String>,
    points: Vec<PointView>,
    cancelled: bool,
    highest_confirmed_capacity: Option<u64>,
}

#[derive(Deserialize)]
struct CompareView {
    #[serde(rename = "schema_version")]
    _schema_version: u16,
    ok: bool,
    command: String,
    comparable: bool,
    differences: Vec<String>,
    pairs: Vec<ComparePairView>,
    unavailable_reasons: Vec<String>,
}

#[derive(Deserialize)]
struct ComparePairView {
    left_run_id: String,
    right_run_id: String,
    left_agent: String,
    right_agent: String,
    comparable: bool,
    differences: Vec<String>,
}

#[derive(Deserialize)]
struct ReportView {
    #[serde(rename = "schema_version")]
    _schema_version: u16,
    ok: bool,
    command: String,
    runs: Vec<ReportRunView>,
}

#[derive(Deserialize)]
struct ReportRunView {
    run_id: String,
    attempt_id: String,
    event_count: usize,
    terminal_state: Option<String>,
    execution_sha256: String,
}

#[derive(Deserialize)]
struct RecordingView {
    #[serde(rename = "schema_version")]
    _schema_version: u16,
    cassette_id: String,
    cassette_sha256: String,
    agent_id: String,
    complete: bool,
}

#[derive(Deserialize)]
struct CampaignView {
    #[serde(rename = "schema_version")]
    _schema_version: u16,
    ok: bool,
    command: String,
    tuple_count: u16,
    complete_coverage: bool,
    offline_ready: bool,
    unavailable_reason: Option<CampaignUnavailableReasonView>,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
enum CampaignUnavailableReasonView {
    RecordingCoverageIncomplete,
}

#[derive(Deserialize)]
struct ReplayView {
    #[serde(rename = "schema_version")]
    _schema_version: u16,
    ok: bool,
    command: String,
    network: String,
    agent_id: String,
    response_status: u16,
}

#[derive(Deserialize)]
struct LocalReplayView {
    #[serde(rename = "schema_version")]
    _schema_version: u16,
    ok: bool,
    command: String,
    network: String,
    agent_id: String,
    result_digest: String,
    output_bytes: u64,
}

fn decode<T: for<'de> Deserialize<'de>>(captured: &[u8]) -> io::Result<T> {
    serde_json::from_slice(captured).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "command returned a malformed or mismatched result",
        )
    })
}

struct ValidatedOutcome {
    fields: Map<String, Value>,
}

fn validate_typed_result(
    invocation: InvocationKind,
    args: &[OsString],
    captured: &[u8],
) -> io::Result<ValidatedOutcome> {
    let invalid = || {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "command result discriminator does not match its invocation",
        )
    };
    if let Ok(value) = serde_json::from_slice::<Value>(captured)
        && let Some(version) = value.get("schema_version")
    {
        let expected = if matches!(
            invocation,
            InvocationKind::Setup | InvocationKind::Easy(EasyKind::Setup)
        ) {
            2
        } else {
            1
        };
        if version.as_u64() != Some(expected) {
            return Err(invalid());
        }
    }
    match invocation {
        InvocationKind::Doctor => {
            let value: DoctorView = decode(captured)?;
            if value.command != "doctor" || value.ok != value.linux || value.architecture.is_empty()
            {
                return Err(invalid());
            }
            let _ = (value.procfs, value.cgroup_v2);
        }
        InvocationKind::Setup | InvocationKind::Easy(EasyKind::Setup) => {
            let value: SetupView = decode(captured)?;
            if !value.ok
                || value.command != "setup"
                || !matches!(value.mode.as_str(), "preflight" | "commit")
                || value.persisted && !value.persistent_change
                || value.mode == "preflight" && value.persistent_change
                || value.mode == "commit" && !value.persistent_change
                || value.selected_agents.len() > 9
                || value.selected_agents.iter().any(|agent| !safe_id(agent))
                || value.persisted && value.selected_agents.is_empty()
                || value.persisted
                    && value
                        .provider_profile
                        .as_deref()
                        .is_none_or(|provider| !safe_id(provider))
                || value.persisted
                    && value
                        .model
                        .as_deref()
                        .is_none_or(|model| model.is_empty() || model.chars().any(char::is_control))
            {
                return Err(invalid());
            }
        }
        InvocationKind::Easy(EasyKind::Lifecycle) => {
            let value: LifecycleView = decode(captured)?;
            let expected = args.get(1).and_then(|arg| arg.to_str());
            if !value.ok
                || value.command != "easy"
                || Some(value.operation.as_str()) != expected
                || value.channel.is_empty()
                || value.message.is_empty()
                || value.dry_run && value.message != "no changes made"
                || value.operation == "test" && !value.installed
            {
                return Err(invalid());
            }
        }
        InvocationKind::Tui(TuiKind::Help | TuiKind::Version) => {}
        InvocationKind::Tui(TuiKind::Unknown) => return Err(invalid()),
        InvocationKind::Tui(tui_kind) => {
            let value: TuiView = decode(captured)?;
            if value.command != "tui"
                || Some(value.operation.as_str()) != tui_kind.result_operation()
                || (!value.ok && value.classification.as_deref().is_none_or(str::is_empty))
                || (!value.ok && value.code.as_deref().is_none_or(str::is_empty))
                || value
                    .channel
                    .as_deref()
                    .is_some_and(|channel| !safe_id(channel))
            {
                return Err(invalid());
            }
            let _warnings = &value.warnings;
        }
        InvocationKind::Capabilities => {
            super::capabilities::CapabilityResponse::parse(captured).map_err(|_| invalid())?;
        }
        InvocationKind::ProjectInit => {
            let value: ProjectInitView = decode(captured)?;
            if !value.ok
                || value.command != "project init"
                || value.initialized == value.recovered
                || value.config != ".asb/project.json"
                || value.results != "results"
                || value.catalogs != "catalogs"
            {
                return Err(invalid());
            }
        }
        InvocationKind::ProviderCatalog { .. }
        | InvocationKind::Easy(EasyKind::ProviderCatalog) => {
            let value: ProviderCatalogView = decode(captured)?;
            validate_catalog_envelope(&value.envelope, "provider-catalog")?;
            if value.agents.is_empty()
                || value.profiles.is_empty()
                || value.agents.iter().any(|item| !safe_id(item))
                || value
                    .openrouter_free_models
                    .iter()
                    .any(|model| model.is_empty() || model.chars().any(char::is_control))
                || value.profiles.iter().any(|item| {
                    !safe_id(&item.id)
                        || item.model.is_empty()
                        || item.model.chars().any(char::is_control)
                        || item
                            .unavailable_reason
                            .as_deref()
                            .is_some_and(|reason| reason.chars().any(char::is_control))
                })
                || !value.profiles.iter().any(|item| item.selectable)
            {
                return Err(invalid());
            }
        }
        InvocationKind::AdapterCatalog | InvocationKind::Easy(EasyKind::AdapterCatalog) => {
            let value: AdapterCatalogView = decode(captured)?;
            validate_catalog_envelope(&value.envelope, "adapter-catalog")?;
            if value.adapters.is_empty()
                || value.adapters.iter().any(|item| {
                    !safe_id(&item.id)
                        || item.providers.is_empty()
                        || item.providers.iter().any(|provider| !safe_id(provider))
                        || item.availability.is_empty()
                })
            {
                return Err(invalid());
            }
        }
        InvocationKind::WorkloadCatalog => {
            let value: WorkloadCatalogView = decode(captured)?;
            validate_catalog_envelope(&value.envelope, "workload-catalog")?;
            if value.entries.is_empty()
                || value.entries.iter().any(|item| {
                    let valid_availability = matches!(
                        item.availability,
                        WorkloadAvailabilityView::Available
                            | WorkloadAvailabilityView::FixtureOnly
                            | WorkloadAvailabilityView::Unavailable
                    );
                    !safe_id(&item.id) || item.kind.is_empty() || !valid_availability
                })
            {
                return Err(invalid());
            }
        }
        InvocationKind::ConfigOpenrouter => {
            let value: ConfigView = decode(captured)?;
            if !value.ok
                || value.command != "config-openrouter"
                || value.provider != "openrouter"
                || value.model.is_empty()
            {
                return Err(invalid());
            }
        }
        InvocationKind::Auth(AuthKind::Setup) => {
            let value: AuthSetupView = decode(captured)?;
            if !value.ok || value.command != "auth-setup" || value.persisted {
                return Err(invalid());
            }
            let _ = value.configured;
        }
        InvocationKind::Auth(AuthKind::Unknown) => return Err(invalid()),
        InvocationKind::Auth(auth_kind) => {
            let value: JsonRpcView = decode(captured)?;
            let submitted = args.iter().any(|arg| arg == "--socket");
            let valid_outcome = if submitted {
                let response_shape =
                    value.result.is_some() != value.error.is_some() && value.method.is_none();
                let result_matches =
                    value
                        .result
                        .as_ref()
                        .is_none_or(|result| match (auth_kind, result) {
                            (AuthKind::Status, AuthResultView::AuthStatus(status)) => {
                                safe_id(&status.provider)
                                    && safe_id(&status.status)
                                    && status.endpoint_identity_sha256.len() == 64
                                    && status.credential_locator_sha256.len() == 64
                                    && status.generation > 0
                            }
                            (
                                AuthKind::Enroll | AuthKind::Rotate | AuthKind::Revoke,
                                AuthResultView::Acknowledged(acknowledgement),
                            ) => {
                                let _accepted = acknowledgement.accepted;
                                true
                            }
                            _ => false,
                        });
                response_shape && result_matches
            } else {
                value.result.is_none()
                    && value.error.is_none()
                    && value.method.as_deref() == auth_kind.request_method()
            };
            if value.jsonrpc != "2.0" || value.id.is_null() || !valid_outcome {
                return Err(invalid());
            }
        }
        InvocationKind::ProviderPlan => {
            let value: ProviderPlanView = decode(captured)?;
            if !value.ok
                || value.command != "provider-plan"
                || !value.dry_run
                || value.provider_profile.is_empty()
                || value.model.is_empty()
                || value.effective.is_empty()
                || value.effective.iter().any(|item| {
                    !safe_id(&item.agent)
                        || item.profile_sha256.len() != 64
                        || !matches!(
                            item.api_mode,
                            EffectiveApiModeView::ChatCompletions | EffectiveApiModeView::Responses
                        )
                })
            {
                return Err(invalid());
            }
        }
        InvocationKind::Plan { create: true } => {
            let value: PlanCreateView = decode(captured)?;
            if !value.ok
                || value.command != "plan-create"
                || value.plan.is_empty()
                || value.workload.is_empty()
                || value.experiment_sha256.len() != 64
            {
                return Err(invalid());
            }
        }
        InvocationKind::Plan { create: false } | InvocationKind::Easy(EasyKind::Plan) => {
            let value: PlanView = decode(captured)?;
            if !value.ok
                || value.command != "plan"
                || value.run_id.is_empty()
                || value.workload.is_empty()
                || value.agent_implementation.is_empty()
            {
                return Err(invalid());
            }
        }
        InvocationKind::Run
        | InvocationKind::Sweep
        | InvocationKind::BenchmarkLive
        | InvocationKind::Easy(EasyKind::Run | EasyKind::Sweep) => {
            let value: ExecutionView = decode(captured)?;
            let expected = match invocation {
                InvocationKind::Sweep | InvocationKind::Easy(EasyKind::Sweep) => "sweep",
                InvocationKind::BenchmarkLive if args.iter().any(|arg| arg == "--sweep") => "sweep",
                _ => "run",
            };
            let every_point_passes = value
                .points
                .iter()
                .all(|point| matches!(point.decision, PointDecisionView::Pass));
            if value.command != expected
                || value.run_ids.len() != value.points.len()
                || (!value.cancelled && value.points.is_empty())
                || value.ok != (!value.cancelled && every_point_passes)
            {
                return Err(invalid());
            }
            let _highest_confirmed_capacity = value.highest_confirmed_capacity;
        }
        InvocationKind::Compare | InvocationKind::Easy(EasyKind::Compare) => {
            let value: CompareView = decode(captured)?;
            let every_pair_comparable = value.pairs.iter().all(|pair| pair.comparable);
            let coherent_comparable = every_pair_comparable
                && value.differences.is_empty()
                && value.unavailable_reasons.is_empty();
            if !value.ok
                || value.command != "compare"
                || value.pairs.is_empty()
                || value.pairs.iter().any(|pair| {
                    !safe_id(&pair.left_run_id)
                        || !safe_id(&pair.right_run_id)
                        || !safe_id(&pair.left_agent)
                        || !safe_id(&pair.right_agent)
                        || pair.comparable && !pair.differences.is_empty()
                        || !pair.comparable
                            && pair.differences.is_empty()
                            && value.unavailable_reasons.is_empty()
                        || pair
                            .differences
                            .iter()
                            .any(|difference| !safe_id(difference))
                        || pair
                            .differences
                            .iter()
                            .any(|difference| !value.differences.contains(difference))
                })
                || value
                    .differences
                    .iter()
                    .any(|difference| !safe_id(difference))
                || value.comparable != coherent_comparable
            {
                return Err(invalid());
            }
        }
        InvocationKind::Report | InvocationKind::Easy(EasyKind::Report) => {
            let value: ReportView = decode(captured)?;
            if !value.ok
                || value.command != "report"
                || value.runs.is_empty()
                || value.runs.iter().any(|run| {
                    let _event_count = run.event_count;
                    !safe_id(&run.run_id)
                        || !safe_id(&run.attempt_id)
                        || run.execution_sha256.len() != 64
                        || run
                            .terminal_state
                            .as_deref()
                            .is_none_or(|state| !safe_id(state))
                })
            {
                return Err(invalid());
            }
        }
        InvocationKind::Record
        | InvocationKind::RecordLive
        | InvocationKind::Easy(EasyKind::Record | EasyKind::RecordLive) => {
            let value: RecordingView = decode(captured)?;
            if value.cassette_id.is_empty()
                || value.cassette_sha256.len() != 64
                || value.agent_id.is_empty()
                || !value.complete
            {
                return Err(invalid());
            }
        }
        InvocationKind::RecordCampaign | InvocationKind::Easy(EasyKind::RecordCampaign) => {
            let value: CampaignView = decode(captured)?;
            if value.command != "record-campaign"
                || value.tuple_count == 0
                || value.ok != (value.complete_coverage && value.offline_ready)
                || value.complete_coverage == value.unavailable_reason.is_some()
            {
                return Err(invalid());
            }
        }
        InvocationKind::Easy(EasyKind::Replay | EasyKind::ReplayOffline) => {
            validate_local_replay(captured)?;
        }
        InvocationKind::ReplayOffline { local_mock: true } => {
            validate_local_replay(captured)?;
        }
        InvocationKind::Replay | InvocationKind::ReplayOffline { local_mock: false } => {
            let value: ReplayView = decode(captured)?;
            let expected = if matches!(invocation, InvocationKind::Replay) {
                "replay"
            } else {
                "replay-offline"
            };
            if !value.ok
                || value.command != expected
                || value.network != "denied"
                || value.agent_id.is_empty()
                || value.response_status == 0
            {
                return Err(invalid());
            }
        }
        InvocationKind::Help
        | InvocationKind::Version
        | InvocationKind::Tool
        | InvocationKind::Easy(EasyKind::Help)
        | InvocationKind::Completion
        | InvocationKind::Serve
        | InvocationKind::InternalOpenCodeBatch => {}
        InvocationKind::Cli | InvocationKind::Easy(EasyKind::Unknown) => return Err(invalid()),
    }
    let raw: Value = serde_json::from_slice(captured).map_err(|_| invalid())?;
    let object = raw.as_object().ok_or_else(invalid)?;
    Ok(ValidatedOutcome {
        fields: project_for_presentation(invocation, object),
    })
}

fn validate_catalog_envelope(value: &CatalogEnvelope, expected: &str) -> io::Result<()> {
    if value.ok && value.command == expected {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "catalog result discriminator does not match its invocation",
        ))
    }
}

fn safe_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._-/:".contains(character))
}

fn validate_local_replay(captured: &[u8]) -> io::Result<()> {
    let value: LocalReplayView = decode(captured)?;
    if value.ok
        && value.command == "replay-offline"
        && value.network == "denied"
        && !value.agent_id.is_empty()
        && value.result_digest.len() == 64
        && value.output_bytes > 0
    {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "local replay result does not match its invocation",
        ))
    }
}

impl Presentation {
    fn new(outcome: impl Into<String>) -> Self {
        Self {
            outcome: outcome.into(),
            ..Self::default()
        }
    }

    fn fact(&mut self, fact: impl Into<String>) {
        let fact = fact.into();
        if !fact.is_empty() {
            self.facts.push(fact);
        }
    }

    fn warning(&mut self, warning: impl Into<String>) {
        let warning = warning.into();
        if !warning.is_empty() {
            self.warnings.push(warning);
        }
    }
}

#[cfg(test)]
pub(super) fn render_success(
    args: &[OsString],
    captured: &[u8],
    details: bool,
    output: &mut dyn Write,
) -> io::Result<()> {
    render_success_with_context(args, captured, details, &PresentationContext::None, output)
}

pub(super) fn render_success_with_context(
    args: &[OsString],
    captured: &[u8],
    details: bool,
    context: &PresentationContext,
    output: &mut dyn Write,
) -> io::Result<()> {
    let kind = CommandKind::parse(args)?;
    if kind.raw(args) {
        return output.write_all(captured);
    }
    let invocation = InvocationKind::classify_args(args);
    let validated = validate_typed_result(invocation, args, captured)?;
    let mut presentation = present(invocation, args, &validated.fields, context);
    if details {
        add_details(&mut presentation, &validated.fields);
    }
    write_presentation(&presentation, output, terminal_width())
}

/// Project only fields covered by the invocation's typed validator. Unknown
/// JSON remains machine-visible but cannot silently become a human claim.
fn project_for_presentation(
    invocation: InvocationKind,
    object: &Map<String, Value>,
) -> Map<String, Value> {
    let fields: &[&str] = match invocation.family() {
        CommandKind::Doctor => &["ok", "linux", "architecture", "procfs", "cgroup_v2"],
        CommandKind::Setup => &["persisted", "provider_profile", "model", "selected_agents"],
        CommandKind::Easy => &["operation", "dry_run", "message", "channel"],
        CommandKind::Tui => &[
            "ok",
            "operation",
            "classification",
            "code",
            "channel",
            "warnings",
        ],
        CommandKind::Capabilities => &["protocol_version", "capabilities"],
        CommandKind::Project => &["initialized", "recovered", "config", "results", "catalogs"],
        CommandKind::Tool => &["ok", "command", "tools", "warnings"],
        CommandKind::ProviderCatalog => &["profiles", "openrouter_free_models", "agents"],
        CommandKind::AdapterCatalog => &["adapters"],
        CommandKind::WorkloadCatalog => &["entries"],
        CommandKind::Config => &["provider", "model"],
        CommandKind::Auth => &[
            "command",
            "configured",
            "jsonrpc",
            "id",
            "method",
            "result",
            "error",
        ],
        CommandKind::ProviderPlan => &["provider_profile", "model", "effective"],
        CommandKind::Plan => &["plan", "run_id", "workload", "agent_implementation"],
        CommandKind::Run | CommandKind::Sweep | CommandKind::BenchmarkLive => &[
            "ok",
            "run_ids",
            "points",
            "cancelled",
            "highest_confirmed_capacity",
        ],
        CommandKind::Compare => &["comparable", "pairs", "differences", "unavailable_reasons"],
        CommandKind::Report => &["runs"],
        CommandKind::Replay | CommandKind::ReplayOffline => &[
            "network",
            "agent_id",
            "response_status",
            "result_digest",
            "output_bytes",
        ],
        CommandKind::RecordCampaign => &[
            "complete_coverage",
            "offline_ready",
            "tuple_count",
            "unavailable_reason",
        ],
        CommandKind::Record
        | CommandKind::RecordLive
        | CommandKind::Cli
        | CommandKind::Help
        | CommandKind::Version
        | CommandKind::Completion
        | CommandKind::Serve
        | CommandKind::InternalOpenCodeBatch => &[],
    };
    fields
        .iter()
        .filter_map(|field| {
            object
                .get(*field)
                .cloned()
                .map(|value| ((*field).to_owned(), value))
        })
        .collect::<Map<_, _>>()
}

pub(super) fn render_error(
    args: &[OsString],
    error: &CliError,
    details: bool,
    output: &mut dyn Write,
) -> io::Result<()> {
    let kind = CommandKind::parse(args).unwrap_or(CommandKind::Cli);
    let diagnostic = error.diagnostic;
    let mut presentation = Presentation::new(format!(
        "ASB could not complete the {}: {}.",
        kind.label(),
        diagnostic.cause_explanation()
    ));
    presentation.fact(format!("Affected {}.", diagnostic.subject_label()));
    presentation.fact(format!("Detail: {}.", sentence_fragment(error.message)));
    presentation.fact(diagnostic.state_change_explanation());
    presentation.fact(format!(
        "Recovery: {}",
        diagnostic.remediation_explanation()
    ));
    match error.exit_code {
        _ if error.human_class == HumanErrorClass::UserCorrection && error.exit_code == 2 => {
            presentation.kind = OutcomeKind::UserError;
            presentation.fact("The command arguments were not accepted.");
            presentation.fact("No successful state change was reported.");
            if error.remediation == ErrorRemediation::Help {
                presentation.next = NextAction::new(["asb", "--help"]);
            }
        }
        _ if error.human_class == HumanErrorClass::UserCorrection => {
            presentation.kind = OutcomeKind::UserError;
            presentation.fact("The requested input or configuration needs correction.");
            presentation.fact("No successful state change was reported.");
            presentation.next = validation_next(args, error);
        }
        _ if error.human_class == HumanErrorClass::HostLimitation => {
            presentation.kind = OutcomeKind::HostLimitation;
            presentation.fact("A required host capability is unavailable.");
        }
        _ if error.human_class == HumanErrorClass::DependencyUnavailable => {
            presentation.kind = OutcomeKind::Warning;
            presentation.fact("A required external provider or network dependency is unavailable.");
            if error.remediation == ErrorRemediation::OpenRouterCredential {
                presentation.fact(
                    "Set OPENROUTER_API_KEY in the current shell before retrying this provider command; ASB does not store the key.",
                );
            } else {
                presentation.fact(
                    "Check provider reachability and the selected provider response before trying again.",
                );
            }
        }
        _ if error.human_class == HumanErrorClass::ProductFailure => {
            presentation.kind = OutcomeKind::ProductFailure;
            presentation.fact("ASB did not report a completed result.");
            presentation.warning(
                "Inspect any destination named in the command before retrying; no retry was run automatically.",
            );
        }
        _ => presentation.fact("ASB did not report a completed result."),
    }
    presentation.next = match diagnostic.context.remediation {
        Remediation::RunDoctor => NextAction::new(["asb", "doctor"]),
        Remediation::AuthenticateProvider
            if error.remediation == ErrorRemediation::OpenRouterCredential =>
        {
            NextAction::new([
                "asb",
                "auth",
                "setup",
                "--provider",
                "openrouter",
                "--api-key-stdin",
            ])
        }
        Remediation::CorrectInput => validation_next(args, error),
        _ => None,
    };
    if let Some(path) = &error.path {
        presentation.fact(format!(
            "Affected destination: `{}`.",
            safe_local_path(path)
        ));
    }
    if details {
        presentation.fact(format!(
            "Diagnostic code: {} (exit {}).",
            error.code, error.exit_code
        ));
        presentation.fact(format!(
            "Diagnostic cause: {:?}; subject {:?}; phase {:?}; state {:?}.",
            error.diagnostic.cause,
            error.diagnostic.context.subject,
            error.diagnostic.context.phase,
            error.diagnostic.context.state_change
        ));
    }
    write_presentation(&presentation, output, terminal_width())
}

fn safe_local_path(path: &Path) -> String {
    let value = path.to_string_lossy();
    let mut output = String::with_capacity(value.len());
    for character in value.chars() {
        if character.is_control() {
            output.push('?');
        } else {
            output.push(character);
        }
    }
    output
}

pub(super) fn render_start(args: &[OsString], output: &mut dyn Write) -> io::Result<()> {
    if InvocationKind::classify_args(args) != InvocationKind::Serve {
        return Ok(());
    }
    let valid_shape = match args {
        [_, _] => true,
        [_, _, flag] => flag == "--local-mock",
        _ => false,
    };
    if !valid_shape {
        return Ok(());
    }
    writeln!(
        output,
        "ASB is attempting to start the control service from the supplied configuration."
    )?;
    output.flush()
}

fn present(
    invocation: InvocationKind,
    args: &[OsString],
    object: &Map<String, Value>,
    context: &PresentationContext,
) -> Presentation {
    let kind = invocation.family();
    match kind {
        CommandKind::Doctor => present_doctor(object),
        CommandKind::Setup => present_setup(args, object),
        CommandKind::Easy => present_easy(args, object, context),
        CommandKind::Tui => present_tui(object),
        CommandKind::Capabilities => {
            let mut value = Presentation::new("ASB reported its supported frontend capabilities.");
            value.fact(format!(
                "Protocol version: {}.",
                number(object, "protocol_version").unwrap_or(1)
            ));
            if let Some(capabilities) = object.get("capabilities").and_then(Value::as_object) {
                let available = capabilities
                    .iter()
                    .filter_map(|(name, enabled)| {
                        enabled
                            .as_bool()
                            .filter(|enabled| *enabled)
                            .map(|_| human_code(name))
                    })
                    .collect::<Vec<_>>();
                if !available.is_empty() {
                    value.fact(format!("Available operations: {}.", available.join(", ")));
                }
            }
            value
        }
        CommandKind::Project => {
            let initialized = boolean(object, "initialized").unwrap_or(false);
            let mut value = Presentation::new(if initialized {
                "ASB initialized the project workspace."
            } else {
                "ASB recovered the existing project workspace."
            });
            fact_pair(&mut value, object, "Configuration", "config");
            fact_pair(&mut value, object, "Results directory", "results");
            fact_pair(&mut value, object, "Catalog directory", "catalogs");
            value.next = NextAction::new(["asb", "provider-catalog"]);
            value
        }
        CommandKind::Tool => {
            let mut value = Presentation::new("ASB discovered project and system tools.");
            fact_count(&mut value, object, "tools", "discovered tools");
            if let Some(warnings) = object.get("warnings").and_then(Value::as_array)
                && !warnings.is_empty()
            {
                for warning in warnings.iter().filter_map(Value::as_str).take(4) {
                    value.warning(warning_text(warning));
                }
            }
            value
        }
        CommandKind::ProviderCatalog => present_catalog(
            "ASB loaded the provider catalog.",
            object,
            "profiles",
            "provider profiles",
            "id",
            context,
        ),
        CommandKind::AdapterCatalog => present_catalog(
            "ASB loaded the coding-agent adapter catalog.",
            object,
            "adapters",
            "adapters",
            "id",
            context,
        ),
        CommandKind::WorkloadCatalog => present_catalog(
            "ASB loaded the benchmark workload catalog.",
            object,
            "entries",
            "workloads",
            "id",
            context,
        ),
        CommandKind::Config => {
            let mut value = Presentation::new("ASB saved the OpenRouter configuration.");
            fact_pair(&mut value, object, "Provider", "provider");
            fact_pair(&mut value, object, "Model", "model");
            value.next = NextAction::new(["asb", "provider-catalog"]);
            value
        }
        CommandKind::Auth => present_auth(args, object),
        CommandKind::ProviderPlan => {
            let configured = args.iter().any(|arg| arg == "--use-config");
            let mut value = Presentation::new(if configured {
                "ASB validated the configured provider selection for this command; no configuration was changed."
            } else {
                "ASB validated a provider selection preview; no configuration was changed."
            });
            fact_pair(&mut value, object, "Provider", "provider_profile");
            fact_pair(&mut value, object, "Model", "model");
            fact_count(&mut value, object, "effective", "selected agents");
            if let Some(agents) = object.get("effective").and_then(Value::as_array) {
                let selected = agents
                    .iter()
                    .take(12)
                    .filter_map(|agent| {
                        Some(format!(
                            "{} ({})",
                            display_value(string_value(agent, "agent")?),
                            human_code(string_value(agent, "api_mode")?)
                        ))
                    })
                    .collect::<Vec<_>>();
                value.fact(format!("Selected agents: {}.", selected.join(", ")));
            }
            if configured {
                value.next = NextAction::new(["asb", "workload-catalog"]);
            }
            value
        }
        CommandKind::Plan => present_plan(invocation, args, object, context),
        CommandKind::Run | CommandKind::Sweep | CommandKind::BenchmarkLive => {
            present_execution(kind, object, context)
        }
        CommandKind::Compare => present_compare(object),
        CommandKind::Report => present_report(invocation, args, object),
        CommandKind::Record | CommandKind::RecordLive => {
            let mut value = Presentation::new("ASB created and sealed the replay recording.");
            let output_index = if matches!(
                invocation,
                InvocationKind::Easy(EasyKind::Record | EasyKind::RecordLive)
            ) {
                3
            } else {
                2
            };
            if let Some(path) = args.get(output_index).and_then(|arg| arg.to_str()) {
                value.fact(format!("Cassette: {}.", display_value(path)));
            }
            value
        }
        CommandKind::RecordCampaign => present_campaign(object),
        CommandKind::Replay | CommandKind::ReplayOffline => {
            let mut value =
                Presentation::new("ASB completed the replay without provider network access.");
            fact_pair(&mut value, object, "Agent", "agent_id");
            if let Some(status) = number(object, "response_status") {
                value.fact(format!("Recorded response status: {status}."));
            }
            value
        }
        CommandKind::Cli
        | CommandKind::Help
        | CommandKind::Version
        | CommandKind::Completion
        | CommandKind::Serve
        | CommandKind::InternalOpenCodeBatch => unreachable!("raw commands bypass presentation"),
    }
}

fn present_doctor(object: &Map<String, Value>) -> Presentation {
    let ready = boolean(object, "ok").unwrap_or(false)
        && boolean(object, "procfs").unwrap_or(false)
        && boolean(object, "cgroup_v2").unwrap_or(false);
    let mut value = Presentation::new(if ready {
        "ASB found this host ready for its supported command-line workflows."
    } else {
        "ASB found host limitations that prevent supported command-line workflows."
    });
    fact_pair(&mut value, object, "Architecture", "architecture");
    let mut missing = Vec::new();
    for (key, label) in [
        ("linux", "Linux"),
        ("procfs", "procfs"),
        ("cgroup_v2", "cgroup v2"),
    ] {
        if !boolean(object, key).unwrap_or(false) {
            missing.push(label);
        }
    }
    if !missing.is_empty() {
        value.kind = OutcomeKind::HostLimitation;
        value.warning(format!("Unavailable: {}.", missing.join(", ")));
    }
    value
}

fn present_setup(args: &[OsString], object: &Map<String, Value>) -> Presentation {
    let persisted = boolean(object, "persisted").unwrap_or(false);
    let output_path = args
        .windows(2)
        .find(|pair| pair[0] == "--output")
        .and_then(|pair| pair[1].to_str());
    let mut value = Presentation::new(if persisted {
        "ASB saved the selected agents, provider, and model."
    } else if output_path.is_some() {
        "ASB created a setup selection file; the active configuration was not changed."
    } else {
        "ASB checked the available setup choices; no configuration was changed."
    });
    if let Some(path) = output_path {
        value.fact(format!("Selection file: {}.", display_value(path)));
    }
    fact_pair(&mut value, object, "Provider", "provider_profile");
    fact_pair(&mut value, object, "Model", "model");
    if let Some(agents) = object.get("selected_agents").and_then(Value::as_array) {
        let agents = agents
            .iter()
            .filter_map(Value::as_str)
            .map(display_value)
            .collect::<Vec<_>>();
        if !agents.is_empty() {
            value.fact(format!("Selected agent count: {}.", agents.len()));
            value.fact(format!("Selected agents: {}.", agents.join(", ")));
        }
    }
    value.next = if persisted {
        NextAction::new(["asb", "workload-catalog"])
    } else {
        NextAction::new(["asb", "provider-catalog"])
    };
    value
}

fn present_easy(
    args: &[OsString],
    object: &Map<String, Value>,
    context: &PresentationContext,
) -> Presentation {
    if object.contains_key("operation") {
        let operation = string(object, "operation").unwrap_or("operation");
        let dry_run = boolean(object, "dry_run").unwrap_or(false);
        let message = string(object, "message").unwrap_or("completed");
        let mut value = Presentation::new(if dry_run {
            format!("ASB checked the easy {operation} operation; no changes were made.")
        } else {
            format!("ASB completed the easy {operation} operation: {message}.")
        });
        fact_pair(&mut value, object, "Channel", "channel");
        if operation == "build" {
            let channel = string(object, "channel").unwrap_or("dev");
            value.next = NextAction::new([
                "asb".to_owned(),
                "easy".to_owned(),
                "install".to_owned(),
                "--channel".to_owned(),
                channel.to_owned(),
                "--yes".to_owned(),
            ]);
        } else if operation == "install" || operation == "update" {
            value.next = NextAction::new(["asb", "easy", "test"]);
        } else if operation == "status" && !boolean(object, "installed").unwrap_or(false) {
            value.next = NextAction::new(["asb", "easy", "install", "--channel", "dev", "--yes"]);
        }
        return value;
    }
    present_execution(
        if args.get(1).and_then(|arg| arg.to_str()) == Some("sweep") {
            CommandKind::Sweep
        } else {
            CommandKind::Run
        },
        object,
        context,
    )
}

fn present_tui(object: &Map<String, Value>) -> Presentation {
    let ok = boolean(object, "ok").unwrap_or(false);
    let operation = string(object, "operation").unwrap_or("launch");
    let mut value = Presentation::new(if ok {
        match operation {
            "install" => "ASB installed the terminal interface successfully.".to_owned(),
            "upgrade" => "ASB upgraded the terminal interface successfully.".to_owned(),
            "remove" => "ASB removed the terminal interface successfully.".to_owned(),
            "status" => "ASB verified the terminal interface installation.".to_owned(),
            "doctor" => "ASB verified that the terminal interface can run.".to_owned(),
            "preflight" => "ASB completed the terminal interface preflight checks.".to_owned(),
            "live_provider" => {
                "ASB completed the terminal interface live-provider operation.".to_owned()
            }
            "dynamic_catalog" => {
                "ASB completed the terminal interface dynamic-catalog operation.".to_owned()
            }
            "launch" => "ASB launched the terminal interface successfully.".to_owned(),
            _ => format!("ASB completed the terminal interface {operation} operation."),
        }
    } else {
        format!(
            "ASB could not complete the terminal interface {operation}: {}.",
            tui_failure_text(string(object, "code").unwrap_or("operation_failed"))
        )
    });
    if !ok {
        value.kind = match string(object, "classification") {
            Some("host_limitation") => OutcomeKind::HostLimitation,
            Some("product_failure") => OutcomeKind::ProductFailure,
            _ => OutcomeKind::Warning,
        };
    }
    fact_pair(&mut value, object, "Channel", "channel");
    if let Some(warnings) = object.get("warnings").and_then(Value::as_array) {
        for warning in warnings.iter().filter_map(Value::as_str).take(4) {
            match warning {
                "development_missing_authentication_allowed" => value.warning(
                    "Development authentication is not configured; local/mock and offline flows remain available, but no live-provider authorization is claimed.",
                ),
                "development_missing_signatures_allowed" => value.warning(
                    "Development signature verification is not configured; this result is not evidence of a production-trusted release.",
                ),
                "development_missing_key_management_allowed" => value.warning(
                    "Development key management is not configured; no production signing or key-rotation authority is available.",
                ),
                "development_rustup_permission_or_ownership_findings_allowed" => value.warning(
                    "The Rust toolchain contains group-writable or differently owned paths; restrict their permissions and ownership before relying on it.",
                ),
                other => value.warning(warning_text(other)),
            }
        }
    }
    if !ok {
        value.fact("The terminal interface was not reported ready.");
        value.next = tui_next(operation, object);
    } else if operation == "install" || operation == "upgrade" {
        value.next = NextAction::new(["asb", "tui"]);
    }
    value
}

fn present_catalog(
    outcome: &str,
    object: &Map<String, Value>,
    field: &str,
    label: &str,
    id_field: &str,
    context: &PresentationContext,
) -> Presentation {
    let mut value = Presentation::new(outcome);
    fact_count(&mut value, object, field, label);
    if field != "entries"
        && let Some(items) = object.get(field).and_then(Value::as_array)
    {
        let ids = items
            .iter()
            .filter_map(|item| string_value(item, id_field))
            .take(12)
            .map(display_value)
            .collect::<Vec<_>>();
        if !ids.is_empty() {
            value.fact(format!("Catalog {label}: {}.", ids.join(", ")));
        }
        if items.len() > ids.len() {
            value.fact(format!(
                "Showing {} of {} {label}; use asb --json {} for the complete catalog.",
                ids.len(),
                items.len(),
                if field == "profiles" {
                    "provider-catalog"
                } else {
                    "adapter-catalog"
                }
            ));
        }
    }
    if field == "profiles" {
        let profiles = object
            .get("profiles")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|profile| {
                let id = string_value(profile, "id")?;
                let model = string_value(profile, "model")?;
                let setup_supported = matches!(context, PresentationContext::ProviderCatalog { setup_profiles } if setup_profiles.iter().any(|candidate| candidate == id));
                let route = if setup_supported
                    && profile.get("selectable").and_then(Value::as_bool) == Some(true)
                {
                    "setup supported"
                } else {
                    "setup persistence unavailable"
                };
                Some(format!(
                    "{} -> {} ({route})",
                    display_value(id),
                    display_value(model)
                ))
            })
            .collect::<Vec<_>>();
        if !profiles.is_empty() {
            value.fact(format!("Provider routes: {}.", profiles.join(", ")));
        }
        if let Some(models) = object
            .get("openrouter_free_models")
            .and_then(Value::as_array)
        {
            let total = models.len();
            let models = models
                .iter()
                .filter_map(Value::as_str)
                .take(8)
                .map(display_value)
                .collect::<Vec<_>>();
            if !models.is_empty() {
                value.fact(format!("OpenRouter free models: {}.", models.join(", ")));
                if total > models.len() {
                    value.fact(format!(
                        "Showing {} of {total} OpenRouter free models; use asb --json provider-catalog for the complete list.",
                        models.len()
                    ));
                }
            } else {
                value.fact("OpenRouter free models: none discovered.");
            }
        }
        if let Some(agents) = object.get("agents").and_then(Value::as_array) {
            let agents = agents
                .iter()
                .filter_map(Value::as_str)
                .take(12)
                .map(display_value)
                .collect::<Vec<_>>();
            if !agents.is_empty() {
                value.fact(format!("Supported agents: {}.", agents.join(", ")));
            }
        }
        value.fact(
            "Choose a supported agent and a provider/model route marked setup supported; pass those exact values to asb setup.",
        );
        value.fact(
            "Command form: asb setup --agent AGENT --provider-profile PROVIDER --model MODEL --persist.",
        );
    } else if field == "entries" {
        if let Some(items) = object.get(field).and_then(Value::as_array) {
            let available = items
                .iter()
                .filter(|item| string_value(item, "availability") == Some("available"))
                .filter_map(|item| string_value(item, id_field))
                .map(display_value)
                .collect::<Vec<_>>();
            let unavailable = items.len().saturating_sub(available.len());
            if available.is_empty() {
                value.fact("Runnable workloads: none on this host.");
            } else {
                value.fact(format!("Runnable workloads: {}.", available.join(", ")));
            }
            if unavailable > 0 {
                value.fact(format!(
                    "{unavailable} additional workloads are fixture-only or unavailable; use asb --json workload-catalog for their status."
                ));
            }
        }
        value.fact(
            "Choose a listed workload and create a configured plan with: asb plan create --workload WORKLOAD --agent AGENT --agent-executable AGENT_EXECUTABLE --output PLAN --use-config.",
        );
    }
    value
}

fn present_auth(args: &[OsString], object: &Map<String, Value>) -> Presentation {
    if string(object, "command") == Some("auth-setup") {
        let configured = boolean(object, "configured").unwrap_or(false);
        let mut value = Presentation::new(if configured {
            "ASB accepted the development credential for this process; it was not persisted."
        } else {
            "ASB found no usable OpenRouter credential; offline setup remains available."
        });
        if !configured {
            value.kind = OutcomeKind::Warning;
            value.next = NextAction::new([
                "asb",
                "auth",
                "setup",
                "--provider",
                "openrouter",
                "--api-key-stdin",
            ]);
        }
        return value;
    }
    let operation = args
        .get(1)
        .and_then(|arg| arg.to_str())
        .unwrap_or("request");
    let submitted = args.iter().any(|arg| arg == "--socket");
    let rejected = object.contains_key("error");
    let typed = serde_json::from_value::<JsonRpcView>(Value::Object(object.clone())).ok();
    let not_accepted = matches!(
        typed.as_ref().and_then(|rpc| rpc.result.as_ref()),
        Some(AuthResultView::Acknowledged(AuthAcknowledgementView {
            accepted: false
        }))
    );
    let mut value = Presentation::new(if rejected {
        format!("The control service rejected the authentication {operation} request.")
    } else if not_accepted {
        format!("The control service did not accept the authentication {operation} mutation.")
    } else if submitted {
        format!("ASB completed the authentication {operation} request through the control service.")
    } else {
        format!("ASB prepared the authentication {operation} request without sending it.")
    });
    if rejected || not_accepted {
        value.kind = OutcomeKind::ProductFailure;
        value.warning("No authentication state change was reported.");
    } else if let Some(AuthResultView::AuthStatus(status)) =
        typed.as_ref().and_then(|rpc| rpc.result.as_ref())
    {
        value.fact(format!("Provider: {}.", display_value(&status.provider)));
        value.fact(format!("Lifecycle status: {}.", human_code(&status.status)));
        value.fact(format!("Generation: {}.", status.generation));
    }
    value
}

fn present_plan(
    invocation: InvocationKind,
    args: &[OsString],
    object: &Map<String, Value>,
    context: &PresentationContext,
) -> Presentation {
    let created = matches!(invocation, InvocationKind::Plan { create: true });
    let mut value = Presentation::new(if created {
        "ASB created and validated the benchmark plan."
    } else {
        "ASB validated the benchmark plan."
    });
    fact_pair(
        &mut value,
        object,
        if created { "Plan" } else { "Run" },
        if created { "plan" } else { "run_id" },
    );
    fact_pair(&mut value, object, "Workload", "workload");
    fact_pair(&mut value, object, "Agent", "agent_implementation");
    value.next = plan_next(invocation, args, context);
    value
}

fn present_execution(
    kind: CommandKind,
    object: &Map<String, Value>,
    context: &PresentationContext,
) -> Presentation {
    let cancelled = boolean(object, "cancelled").unwrap_or(false);
    let ok = boolean(object, "ok").unwrap_or(false);
    let inconclusive = object
        .get("points")
        .and_then(Value::as_array)
        .is_some_and(|points| {
            points
                .iter()
                .any(|point| string_value(point, "decision") == Some("inconclusive"))
        });
    let outcome = if cancelled {
        format!("ASB cancelled the {} before it completed.", kind.label())
    } else if inconclusive {
        format!(
            "ASB completed the {}, but the result is inconclusive.",
            kind.label()
        )
    } else if ok {
        format!("ASB completed the {} successfully.", kind.label())
    } else {
        format!(
            "ASB completed the {}, but one or more benchmark points failed.",
            kind.label()
        )
    };
    let mut value = Presentation::new(outcome);
    value.kind = if cancelled {
        OutcomeKind::Cancelled
    } else if inconclusive {
        OutcomeKind::Partial
    } else if ok {
        OutcomeKind::Succeeded
    } else {
        OutcomeKind::Warning
    };
    fact_count(&mut value, object, "run_ids", "runs");
    if let Some(run_ids) = object.get("run_ids").and_then(Value::as_array) {
        let ids = run_ids
            .iter()
            .filter_map(Value::as_str)
            .take(12)
            .map(display_value)
            .collect::<Vec<_>>();
        if !ids.is_empty() {
            value.fact(format!("Run IDs: {}.", ids.join(", ")));
        }
    }
    if let Some(points) = object.get("points").and_then(Value::as_array) {
        let decisions = points
            .iter()
            .filter_map(|point| string_value(point, "decision"))
            .map(human_code)
            .collect::<Vec<_>>();
        if !decisions.is_empty() {
            value.fact(format!("Decisions: {}.", decisions.join(", ")));
        }
    }
    if let Some(capacity) = number(object, "highest_confirmed_capacity") {
        value.fact(format!("Highest confirmed concurrency: {capacity}."));
    }
    if let PresentationContext::Execution { run_refs } = context
        && !run_refs.is_empty()
    {
        value.next = NextAction::new(
            ["asb".to_owned(), "report".to_owned()]
                .into_iter()
                .chain(run_refs.iter().cloned()),
        );
    }
    value
}

fn present_compare(object: &Map<String, Value>) -> Presentation {
    let comparable = boolean(object, "comparable").unwrap_or(false);
    let mut value = Presentation::new(if comparable {
        "ASB compared the runs successfully; their evidence is comparable."
    } else {
        "ASB compared the runs, but their evidence is not directly comparable."
    });
    if !comparable {
        value.kind = OutcomeKind::Warning;
    }
    fact_count(&mut value, object, "pairs", "run pairs");
    if let Some(pairs) = object.get("pairs").and_then(Value::as_array) {
        for pair in pairs.iter().take(8) {
            if let (Some(left), Some(right), Some(pair_comparable)) = (
                string_value(pair, "left_run_id"),
                string_value(pair, "right_run_id"),
                pair.get("comparable").and_then(Value::as_bool),
            ) {
                value.fact(format!(
                    "Pair {} to {}: {}.",
                    display_value(left),
                    display_value(right),
                    if pair_comparable {
                        "comparable"
                    } else {
                        "not comparable"
                    }
                ));
                if !pair_comparable
                    && let Some(differences) = pair.get("differences").and_then(Value::as_array)
                {
                    let reasons = differences
                        .iter()
                        .filter_map(Value::as_str)
                        .map(human_code)
                        .collect::<Vec<_>>();
                    if !reasons.is_empty() {
                        value.fact(format!("Pair differences: {}.", reasons.join(", ")));
                    }
                }
            }
        }
    }
    if !comparable {
        fact_count(&mut value, object, "differences", "material differences");
        if let Some(differences) = object.get("differences").and_then(Value::as_array) {
            let names = differences
                .iter()
                .filter_map(Value::as_str)
                .take(12)
                .map(human_code)
                .collect::<Vec<_>>();
            if !names.is_empty() {
                value.fact(format!("Differences: {}.", names.join(", ")));
            }
        }
        fact_count(
            &mut value,
            object,
            "unavailable_reasons",
            "missing evidence items",
        );
        if let Some(reasons) = object.get("unavailable_reasons").and_then(Value::as_array) {
            for reason in reasons.iter().filter_map(Value::as_str).take(8) {
                if let Some((run, code)) = reason.split_once(':') {
                    value.fact(format!(
                        "Evidence unavailable for {}: {}.",
                        display_value(run),
                        human_code(code)
                    ));
                }
            }
        }
    }
    value
}

fn present_report(
    invocation: InvocationKind,
    args: &[OsString],
    object: &Map<String, Value>,
) -> Presentation {
    let mut value = Presentation::new("ASB generated the benchmark report.");
    fact_count(&mut value, object, "runs", "runs");
    if let Some(runs) = object.get("runs").and_then(Value::as_array) {
        let completed = runs
            .iter()
            .filter(|run| string_value(run, "terminal_state") == Some("completed"))
            .count();
        value.fact(format!("Completed runs: {completed}."));
        for run in runs.iter().take(8) {
            if let Some(run) = run.as_object() {
                let id = string(run, "run_id")
                    .map(display_value)
                    .unwrap_or_else(|| "unknown".to_owned());
                let state = string(run, "terminal_state")
                    .map(human_code)
                    .unwrap_or_else(|| "not terminal".to_owned());
                let events = number(run, "event_count").unwrap_or(0);
                value.fact(format!("Run {id}: {state}; {events} events."));
            }
        }
    }
    let first_run_index = if matches!(invocation, InvocationKind::Easy(EasyKind::Report)) {
        2
    } else {
        1
    };
    let run_refs = args
        .iter()
        .skip(first_run_index)
        .filter_map(|arg| arg.to_str())
        .collect::<Vec<_>>();
    if run_refs.len() >= 2 {
        value.next = NextAction::new(
            ["asb".to_owned(), "compare".to_owned()]
                .into_iter()
                .chain(run_refs.into_iter().map(str::to_owned)),
        );
    }
    value
}

fn present_campaign(object: &Map<String, Value>) -> Presentation {
    let complete = boolean(object, "complete_coverage").unwrap_or(false)
        && boolean(object, "offline_ready").unwrap_or(false);
    let mut value = Presentation::new(if complete {
        "ASB completed the recording campaign; every requested tuple is ready for offline replay."
    } else {
        "ASB completed part of the recording campaign, but offline coverage is incomplete."
    });
    if !complete {
        value.kind = OutcomeKind::Partial;
    }
    if let Some(count) = number(object, "tuple_count") {
        value.fact(format!("Requested tuples: {count}."));
    }
    if let Some(reason) = string(object, "unavailable_reason") {
        value.warning(format!(
            "Incomplete because {}.",
            campaign_reason_text(reason)
        ));
    }
    value
}

fn tui_next(operation: &str, object: &Map<String, Value>) -> Option<NextAction> {
    let code = string(object, "code").unwrap_or_default();
    let channel = string(object, "channel").unwrap_or("dev");
    if matches!(
        code,
        "extension_not_installed" | "development_installation_invalid"
    ) {
        NextAction::new(["asb", "tui", "install", "--channel", channel])
    } else if operation != "doctor"
        && matches!(
            code,
            "trusted_tool_invalid"
                | "trusted_tool_unavailable"
                | "development_compiler_unsupported"
        )
    {
        NextAction::new(["asb", "tui", "doctor"])
    } else if operation == "launch" {
        NextAction::new(["asb", "tui", "status"])
    } else {
        None
    }
}

fn plan_next(
    invocation: InvocationKind,
    args: &[OsString],
    context: &PresentationContext,
) -> Option<NextAction> {
    let words = args
        .iter()
        .filter_map(|arg| arg.to_str())
        .collect::<Vec<_>>();
    if words.get(1) == Some(&"create") {
        let path = words
            .windows(2)
            .find(|pair| pair[0] == "--output")
            .map(|pair| pair[1])?;
        let mut next = vec!["asb".to_owned(), "plan".to_owned(), path.to_owned()];
        if words.contains(&"--use-config") {
            next.push("--use-config".to_owned());
        }
        return NextAction::new(next);
    }
    let path = words.get(
        if matches!(invocation, InvocationKind::Easy(EasyKind::Plan)) {
            2
        } else {
            1
        },
    )?;
    let operation = if matches!(
        context,
        PresentationContext::Plan {
            sweep_available: true
        }
    ) {
        "sweep"
    } else {
        "run"
    };
    let mut command = vec!["asb".to_owned(), operation.to_owned(), (*path).to_owned()];
    if let Some(index) = words
        .iter()
        .position(|word| *word == "--provider-selection")
    {
        let selection = words.get(index + 1)?;
        command.push("--provider-selection".to_owned());
        command.push((*selection).to_owned());
    } else if words.contains(&"--use-config") {
        command.push("--use-config".to_owned());
    }
    NextAction::new(command)
}

fn validation_next(args: &[OsString], error: &CliError) -> Option<NextAction> {
    if args.first().and_then(|arg| arg.to_str()) == Some("record-live")
        && error.remediation == ErrorRemediation::RecordConfirmation
    {
        let mut words = args
            .iter()
            .filter_map(|arg| arg.to_str())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        words.insert(0, "asb".to_owned());
        if !words.iter().any(|word| word == "--confirm-record") {
            words.push("--confirm-record".to_owned());
        }
        return NextAction::new(words);
    }
    None
}

fn add_details(presentation: &mut Presentation, object: &Map<String, Value>) {
    for (label, key) in [
        ("Result code", "code"),
        ("Classification", "classification"),
        ("Schema version", "schema_version"),
    ] {
        if let Some(value) = object.get(key).and_then(scalar) {
            presentation.fact(format!("{label}: {}.", display_value(&value)));
        }
    }
}

fn write_presentation(
    presentation: &Presentation,
    output: &mut dyn Write,
    width: usize,
) -> io::Result<()> {
    write_wrapped(output, &presentation.outcome, width, "")?;
    for fact in &presentation.facts {
        write_wrapped(output, fact, width, "  ")?;
    }
    for warning in &presentation.warnings {
        write_wrapped(output, &format!("Note: {warning}"), width, "")?;
    }
    if let Some(next) = &presentation.next {
        writeln!(output, "Next: {}", next.render())?;
    }
    Ok(())
}

fn write_wrapped(output: &mut dyn Write, text: &str, width: usize, prefix: &str) -> io::Result<()> {
    let available = width.saturating_sub(UnicodeWidthStr::width(prefix)).max(20);
    let mut line = String::new();
    for word in text.split_whitespace() {
        let word_width = UnicodeWidthStr::width(word);
        let line_width = UnicodeWidthStr::width(line.as_str());
        if !line.is_empty() && line_width + 1 + word_width > available {
            writeln!(output, "{prefix}{line}")?;
            line.clear();
        }
        if word_width > available {
            if !line.is_empty() {
                writeln!(output, "{prefix}{line}")?;
                line.clear();
            }
            let mut chunk = String::new();
            let mut chunk_width = 0;
            for character in word.chars() {
                let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
                if !chunk.is_empty() && chunk_width + character_width > available {
                    writeln!(output, "{prefix}{chunk}")?;
                    chunk.clear();
                    chunk_width = 0;
                }
                chunk.push(character);
                chunk_width += character_width;
                if chunk_width == available {
                    writeln!(output, "{prefix}{chunk}")?;
                    chunk.clear();
                    chunk_width = 0;
                }
            }
            line = chunk;
            continue;
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if line.is_empty() {
        Ok(())
    } else {
        writeln!(output, "{prefix}{line}")
    }
}

fn terminal_width() -> usize {
    std::env::var("COLUMNS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_WIDTH)
        .clamp(MIN_WIDTH, MAX_WIDTH)
}

fn fact_pair(presentation: &mut Presentation, object: &Map<String, Value>, label: &str, key: &str) {
    if let Some(value) = string(object, key).filter(|value| !value.is_empty()) {
        presentation.fact(format!("{label}: {}.", display_value(value)));
    }
}

fn fact_count(
    presentation: &mut Presentation,
    object: &Map<String, Value>,
    key: &str,
    label: &str,
) {
    if let Some(count) = object.get(key).and_then(Value::as_array).map(Vec::len) {
        presentation.fact(format!("{}: {count}.", title_case(label)));
    }
}

fn string<'a>(object: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    object.get(key).and_then(Value::as_str)
}

fn boolean(object: &Map<String, Value>, key: &str) -> Option<bool> {
    object.get(key).and_then(Value::as_bool)
}

fn number(object: &Map<String, Value>, key: &str) -> Option<u64> {
    object.get(key).and_then(Value::as_u64)
}

fn string_value<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.as_object()?.get(key)?.as_str()
}

fn scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn sentence_fragment(value: &str) -> String {
    let sanitized = display_value(value);
    sanitized.trim().trim_end_matches('.').to_owned()
}

fn title_case(value: &str) -> String {
    let mut value = value.to_owned();
    if let Some(first) = value.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    value
}

fn human_code(value: &str) -> String {
    display_value(value).replace(['_', '-'], " ")
}

fn warning_text(code: &str) -> &'static str {
    match code {
        "development_authentication_warning" | "development_missing_authentication_allowed" => {
            "Development authentication is not configured; local/mock and offline flows remain available, but live-provider authorization is not claimed."
        }
        "development_missing_signatures_allowed" => {
            "Development signature verification is not configured; this result is not evidence of a production-trusted release."
        }
        "development_missing_key_management_allowed" => {
            "Development key management is not configured; no production signing or key-rotation authority is available."
        }
        "development_rustup_permission_or_ownership_findings_allowed" => {
            "The development Rust toolchain contains group-writable or differently owned paths; restrict permissions and ownership before relying on it."
        }
        "development authentication, signatures, and keys are warning-only; discovery does not require them" => {
            "Tool discovery completed without production authentication, signature, or key-management services; discovery is usable, but it does not establish production trust."
        }
        "attacker_defined_claim" => {
            "The supplied claim is attacker-defined and cannot establish trusted provenance."
        }
        "development_only" => {
            "This result is development-only and must not be presented as production qualification."
        }
        _ => {
            "ASB received an unrecognized warning identity; this version cannot classify its limitation, so do not treat the result as fully qualified. Inspect the bounded machine-readable warning identity before relying on it."
        }
    }
}

fn campaign_reason_text(code: &str) -> &'static str {
    match code {
        "recording-coverage-incomplete" => {
            "the requested recording matrix did not produce a complete offline-ready cassette"
        }
        _ => "the requested recording matrix is incomplete and is not offline-ready",
    }
}

fn tui_failure_text(code: &str) -> &'static str {
    match code {
        "development_bundle_invalid" => {
            "the development bundle is malformed or incompatible with the selected channel"
        }
        "development_channel_rejected" => {
            "the selected development channel was rejected before activation"
        }
        "development_channel_unavailable" => {
            "the selected development channel could not be reached or inspected"
        }
        "development_compiler_unsupported" => {
            "the required compiler capability is unavailable or unsupported"
        }
        "development_control_failed" => "the development control exchange did not complete",
        "development_control_unavailable" => "the development control service is unavailable",
        "development_source_identity_invalid" | "dev_source_identity_invalid" => {
            "the development source identity is invalid"
        }
        "development_source_identity_mismatch" | "dev_source_identity_mismatch" => {
            "the development source identity does not match the selected artifact"
        }
        "dev_source_identity_unknown" => "the development source identity could not be established",
        "dev_source_identity_stale" => {
            "the development source identity is stale and must be reconciled"
        }
        "development_descriptor_failed" => {
            "the development descriptor could not be validated or published"
        }
        "development_descriptor_oversized" => "the development descriptor exceeds its bounded size",
        "development_filesystem_invalid" => {
            "the development filesystem layout is unsafe or invalid"
        }
        "development_installation_invalid" => {
            "the installed development artifact is invalid or incomplete"
        }
        "development_launch_failed" => {
            "the terminal interface launch failed before a completed session"
        }
        "development_launch_timeout" => {
            "the terminal interface launch exceeded its bounded deadline"
        }
        "development_metadata_failed" | "dev_metadata_failed" => {
            "development metadata could not be produced or validated"
        }
        "candidate_execution_failed" => {
            "the lifecycle candidate execution failed before a completed transition"
        }
        "candidate_request_failed" => "the lifecycle candidate request did not complete",
        "candidate_response_invalid" => {
            "the lifecycle candidate response was malformed or incompatible"
        }
        "development_operation_invalid" => "the requested terminal interface operation is invalid",
        "development_remove_failed" => {
            "the development installation could not be removed completely"
        }
        "development_source_unavailable_offline" => {
            "the development source is unavailable while offline"
        }
        "development_terminal_unavailable" => "the required terminal capability is unavailable",
        "development_host_unavailable" => "the required development host capability is unavailable",
        "candidate_rejected_lifecycle" => {
            "the lifecycle candidate rejected the requested transition; resulting state must be reconciled"
        }
        "rollback_rejected" => "the requested rollback violates the accepted lifecycle state",
        "artifact_quota_exceeded" => {
            "the bounded artifact storage quota was exhausted before publication"
        }
        "dev_workspace_quota_exceeded" => {
            "the bounded development workspace quota was exhausted before completion"
        }
        "trusted_tool_unavailable" | "host_capability_unavailable" => {
            "the trusted tool unavailable state prevents use of the required host capability"
        }
        "signature_verifier_unavailable" => "the signature-verifier capability is unavailable",
        "trusted_tool_invalid" => {
            "the trusted tool is invalid or does not satisfy the development trust contract"
        }
        "transfer_too_large" => "the transferred artifact exceeded the bounded size limit",
        "artifact_transfer_failed" => "the artifact transfer did not complete",
        "rollback_state_invalid" => "the persisted rollback state is malformed or incompatible",
        "rollback_state_failed" => "the rollback state could not be durably written",
        _ => {
            "the terminal interface reported an unclassified failure; inspect the machine-readable code"
        }
    }
}

fn display_value(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect()
}

fn shell_quote(value: &str) -> String {
    if !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_./:@%+=,-".contains(&byte))
    {
        value.to_owned()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(words: &[&str]) -> Vec<OsString> {
        words.iter().map(OsString::from).collect()
    }

    fn render(words: &[&str], value: Value, details: bool) -> String {
        let mut output = Vec::new();
        render_success(
            &args(words),
            &serde_json::to_vec(&value).unwrap(),
            details,
            &mut output,
        )
        .unwrap();
        String::from_utf8(output).unwrap()
    }

    fn render_with_context(
        words: &[&str],
        value: Value,
        context: &PresentationContext,
    ) -> (Vec<u8>, String) {
        let captured = serde_json::to_vec(&value).unwrap();
        let original = captured.clone();
        let mut output = Vec::new();
        render_success_with_context(&args(words), &captured, false, context, &mut output).unwrap();
        assert_eq!(captured, original);
        (captured, String::from_utf8(output).unwrap())
    }

    fn rejects(words: &[&str], value: Value) {
        let mut output = Vec::new();
        assert!(
            render_success(
                &args(words),
                &serde_json::to_vec(&value).unwrap(),
                false,
                &mut output,
            )
            .is_err(),
            "{words:?}"
        );
        assert!(output.is_empty(), "{words:?}");
    }

    #[test]
    fn public_command_inventory_is_explicit_and_has_no_unknown_fallback() {
        let commands: &[&[&str]] = &[
            &["doctor"],
            &["setup"],
            &["easy", "run"],
            &["tui", "launch"],
            &["capabilities"],
            &["project", "init", "/tmp/project"],
            &["tool", "discover"],
            &["provider-catalog"],
            &["adapter-catalog"],
            &["workload-catalog"],
            &["config", "openrouter"],
            &["auth", "status"],
            &["provider-plan"],
            &["completion", "bash"],
            &["plan", "x"],
            &["run", "x"],
            &["sweep", "x"],
            &["benchmark-live", "x"],
            &["serve", "x"],
            &["compare", "a", "b"],
            &["report", "a"],
            &["record", "a", "b"],
            &["record-live", "a", "b"],
            &["record-campaign", "a"],
            &["replay", "a", "b", "c"],
            &["replay-offline", "a", "b", "c"],
            &["internal-opencode-batch"],
        ];
        for command in commands {
            assert!(CommandKind::parse(&args(command)).is_ok(), "{command:?}");
        }
        let mut classified = commands
            .iter()
            .filter_map(|command| command.first().copied())
            .filter(|command| *command != "internal-opencode-batch")
            .collect::<Vec<_>>();
        classified.sort_unstable();
        classified.dedup();
        let mut registered = PUBLIC_COMMANDS.to_vec();
        registered.sort_unstable();
        assert_eq!(classified, registered);
        let mut golden_families = include_str!("../fixtures/human/public-family-output-v1.tsv")
            .lines()
            .filter(|line| !line.starts_with('#'))
            .map(|line| line.split('\t').next().unwrap())
            .collect::<Vec<_>>();
        golden_families.sort_unstable();
        assert_eq!(golden_families, registered);
        assert!(CommandKind::parse(&args(&["future-command"])).is_err());

        for operation in [
            "launch",
            "preflight",
            "status",
            "doctor",
            "install",
            "upgrade",
            "remove",
            "live-provider",
            "dynamic-catalog",
            "--help",
            "--version",
        ] {
            assert_ne!(
                InvocationKind::classify_words(&["tui", operation]),
                InvocationKind::Tui(TuiKind::Unknown),
                "tui {operation}"
            );
        }
        for operation in ["setup", "enroll", "status", "rotate", "revoke"] {
            assert_ne!(
                InvocationKind::classify_words(&["auth", operation]),
                InvocationKind::Auth(AuthKind::Unknown),
                "auth {operation}"
            );
        }
    }

    #[test]
    fn representative_result_for_every_structured_family_has_explicit_human_output() {
        let digest = "a".repeat(64);
        let cases = vec![
            (
                vec!["setup"],
                serde_json::json!({"schema_version":2,"ok":true,"command":"setup","mode":"preflight","persistent_change":false,"persisted":false,"selected_agents":[]}),
            ),
            (
                vec!["easy", "status"],
                serde_json::json!({"schema_version":1,"ok":true,"command":"easy","operation":"status","channel":"dev","dry_run":false,"installed":true,"message":"installed"}),
            ),
            (
                vec!["capabilities"],
                serde_json::json!({"protocol":"asb-cli-capabilities","protocol_version":1,"asb_version":"0.1.0","capabilities":{"analysis":true,"artifacts":true,"cancel":true,"events":true,"history":true,"launch":true,"planning":true,"repeat":true}}),
            ),
            (
                vec!["project", "init", "/tmp/project"],
                serde_json::json!({"schema_version":1,"ok":true,"command":"project init","initialized":true,"recovered":false,"config":".asb/project.json","results":"results","catalogs":"catalogs","next":["asb provider-catalog","asb project init .","asb run PLAN.toml"]}),
            ),
            (
                vec!["provider-catalog"],
                serde_json::json!({"schema_version":1,"ok":true,"command":"provider-catalog","profiles":[{"id":"openrouter","model":"free/model","selectable":true}],"agents":["codex"],"openrouter_free_models":["free/model"]}),
            ),
            (
                vec!["adapter-catalog"],
                serde_json::json!({"schema_version":1,"ok":true,"command":"adapter-catalog","adapters":[{"id":"agent.codex","providers":["openrouter"],"availability":"development"}]}),
            ),
            (
                vec!["workload-catalog"],
                serde_json::json!({"schema_version":1,"ok":true,"command":"workload-catalog","entries":[{"id":"original.bug-fix","kind":"builtin","availability":"available"}]}),
            ),
            (
                vec!["config", "openrouter"],
                serde_json::json!({"schema_version":1,"ok":true,"command":"config-openrouter","provider":"openrouter","model":"free/model"}),
            ),
            (
                vec!["auth", "setup"],
                serde_json::json!({"schema_version":1,"ok":true,"command":"auth-setup","configured":false,"persisted":false}),
            ),
            (
                vec!["provider-plan"],
                serde_json::json!({"schema_version":1,"ok":true,"command":"provider-plan","dry_run":true,"provider_profile":"openrouter","model":"free/model","effective":[{"agent":"codex","profile_sha256":"a".repeat(64),"api_mode":"responses"}]}),
            ),
            (
                vec!["plan", "plan.toml"],
                serde_json::json!({"schema_version":1,"ok":true,"command":"plan","run_id":"run-1","workload":"original.bug-fix","agent_implementation":"codex"}),
            ),
            (
                vec!["run", "plan.toml"],
                serde_json::json!({"schema_version":1,"ok":true,"command":"run","run_ids":["run-1"],"points":[{"decision":"pass"}],"cancelled":false}),
            ),
            (
                vec!["sweep", "plan.toml"],
                serde_json::json!({"schema_version":1,"ok":false,"command":"sweep","run_ids":["run-1"],"points":[{"decision":"inconclusive"}],"cancelled":false}),
            ),
            (
                vec!["benchmark-live", "plan.toml"],
                serde_json::json!({"schema_version":1,"ok":false,"command":"run","run_ids":[],"points":[],"cancelled":true}),
            ),
            (
                vec!["compare", "/a", "/b"],
                serde_json::json!({"schema_version":1,"ok":true,"command":"compare","comparable":true,"differences":[],"pairs":[{"left_run_id":"run-1","right_run_id":"run-2","left_agent":"codex","right_agent":"codex","comparable":true,"differences":[]}],"unavailable_reasons":[]}),
            ),
            (
                vec!["report", "/a"],
                serde_json::json!({"schema_version":1,"ok":true,"command":"report","runs":[{"run_id":"run-1","attempt_id":"attempt-1","terminal_state":"completed","event_count":2,"execution_sha256":"a".repeat(64)}]}),
            ),
            (
                vec!["record", "in.json", "out.json"],
                serde_json::json!({"schema_version":1,"cassette_id":"one","cassette_sha256":digest,"agent_id":"codex","complete":true}),
            ),
            (
                vec!["record-campaign", "campaign.json"],
                serde_json::json!({"schema_version":1,"ok":true,"command":"record-campaign","tuple_count":1,"complete_coverage":true,"offline_ready":true}),
            ),
            (
                vec!["replay", "cassette.json", "profile", "codex"],
                serde_json::json!({"schema_version":1,"ok":true,"command":"replay","network":"denied","agent_id":"codex","response_status":200}),
            ),
            (
                vec![
                    "replay-offline",
                    "cassette.json",
                    "profile",
                    "codex",
                    "--local-mock",
                ],
                serde_json::json!({"schema_version":1,"ok":true,"command":"replay-offline","network":"denied","agent_id":"codex","result_digest":"b".repeat(64),"output_bytes":1}),
            ),
        ];
        for (words, value) in cases {
            let output = render(&words, value, false);
            assert!(!output.is_empty(), "{words:?}");
            assert!(!output.starts_with('{'), "{words:?}");
            assert!(!output.contains("schema_version"), "{words:?}");
            assert!(!output.contains("\u{1b}"), "{words:?}");
        }
    }

    #[test]
    fn invocation_and_typed_result_discriminators_are_exactly_coupled() {
        let tui = |operation: &str| {
            serde_json::json!({
                "schema_version":1,"ok":true,"command":"tui","operation":operation,
                "classification":null,"code":"ready","channel":"dev"
            })
        };
        for (invocation, wrong) in [
            (&["tui", "launch"][..], "status"),
            (&["tui", "preflight"][..], "doctor"),
            (&["tui", "live-provider"][..], "dynamic_catalog"),
            (&["tui", "dynamic-catalog"][..], "live_provider"),
        ] {
            rejects(invocation, tui(wrong));
        }
        rejects(
            &["easy", "status"],
            serde_json::json!({"schema_version":1,"ok":true,"command":"easy","operation":"install","channel":"dev","dry_run":false,"installed":true,"message":"installed"}),
        );
        let execution = |command: &str| {
            serde_json::json!({
                "schema_version":1,"ok":true,"command":command,"run_ids":["run-1"],
                "points":[{"decision":"pass"}],"cancelled":false
            })
        };
        rejects(&["run", "plan.toml"], execution("sweep"));
        rejects(&["sweep", "plan.toml"], execution("run"));
        rejects(&["easy", "run", "plan.toml"], execution("sweep"));
        rejects(&["easy", "sweep", "plan.toml"], execution("run"));
        for malformed in [
            serde_json::json!({"schema_version":1,"ok":true,"command":"project","initialized":true,"recovered":false,"config":".asb/project.json","results":"results","catalogs":"catalogs"}),
            serde_json::json!({"schema_version":1,"ok":true,"command":"project init","initialized":true,"recovered":true,"config":".asb/project.json","results":"results","catalogs":"catalogs"}),
            serde_json::json!({"schema_version":1,"ok":true,"command":"project init","initialized":true,"recovered":false,"config":"/private/project.json","results":"results","catalogs":"catalogs"}),
        ] {
            rejects(&["project", "init", "/tmp/project"], malformed);
        }
        rejects(&["benchmark-live", "plan.toml"], execution("sweep"));
        rejects(
            &["benchmark-live", "plan.toml", "--sweep"],
            execution("run"),
        );
        for (operation, wrong_method) in [
            ("enroll", "auth_status"),
            ("status", "auth_rotate"),
            ("rotate", "auth_revoke"),
            ("revoke", "auth_enroll"),
        ] {
            rejects(
                &["auth", operation],
                serde_json::json!({"jsonrpc":"2.0","id":1,"method":wrong_method}),
            );
        }
        rejects(
            &["auth", "status", "--socket", "/tmp/control.sock"],
            serde_json::json!({"jsonrpc":"2.0","id":1,"result":{"kind":"acknowledged","value":{}}}),
        );
        rejects(
            &["auth", "rotate", "--socket", "/tmp/control.sock"],
            serde_json::json!({"jsonrpc":"2.0","id":1,"result":{"kind":"auth_status","value":{}}}),
        );
        let replay = |command: &str| {
            serde_json::json!({
                "schema_version":1,"ok":true,"command":command,"network":"denied",
                "agent_id":"codex","response_status":200
            })
        };
        rejects(&["replay", "a", "b", "c"], replay("replay-offline"));
        rejects(&["replay-offline", "a", "b", "c"], replay("replay"));
    }

    #[test]
    fn execution_context_changes_only_human_navigation_not_json_bytes() {
        let result = serde_json::json!({
            "schema_version":1,"ok":true,"command":"sweep",
            "run_ids":["run-1","run-2"],
            "points":[{"decision":"pass"},{"decision":"pass"}],
            "cancelled":false
        });
        let (without_bytes, without) = render_with_context(
            &["sweep", "/tmp/plan.toml"],
            result.clone(),
            &PresentationContext::None,
        );
        let context = PresentationContext::Execution {
            run_refs: vec![
                "/tmp/results/runs/run-1".to_owned(),
                "/tmp/results/runs/run-2".to_owned(),
            ],
        };
        let (with_bytes, with) =
            render_with_context(&["sweep", "/tmp/plan.toml"], result, &context);
        assert_eq!(with_bytes, without_bytes);
        assert!(!without.contains("Next:"));
        assert!(
            with.ends_with("Next: asb report /tmp/results/runs/run-1 /tmp/results/runs/run-2\n")
        );
    }

    #[test]
    fn retained_failed_inconclusive_and_cancelled_runs_remain_reportable() {
        let context = PresentationContext::Execution {
            run_refs: vec!["/tmp/results/runs/retained".to_owned()],
        };
        for (ok, decision, cancelled, expected) in [
            (false, "fail", false, "failed"),
            (false, "inconclusive", false, "inconclusive"),
            (false, "pass", true, "cancelled"),
        ] {
            let (_, output) = render_with_context(
                &["run", "/tmp/plan.toml"],
                serde_json::json!({
                    "schema_version":1,"ok":ok,"command":"run",
                    "run_ids":["retained"],"points":[{"decision":decision}],
                    "cancelled":cancelled
                }),
                &context,
            );
            assert!(output.contains(expected), "{output}");
            assert!(output.ends_with("Next: asb report /tmp/results/runs/retained\n"));
            assert!(!output.to_lowercase().contains("retry"));
            assert!(!output.to_lowercase().contains("recover"));
        }
    }

    #[test]
    fn terminal_results_do_not_fabricate_next_actions_or_leak_controls_and_secrets() {
        let doctor = render(
            &["doctor"],
            serde_json::json!({"schema_version":1,"ok":true,"command":"doctor","linux":true,"architecture":"x86_64\u{1b}[31m","procfs":true,"cgroup_v2":true,"api_key":"secret-value"}),
            false,
        );
        assert!(!doctor.contains("Next:"));
        assert!(!doctor.contains("secret-value"));
        assert!(!doctor.contains('\u{1b}'));
        let report = render(
            &["report", "/run"],
            serde_json::json!({"schema_version":1,"ok":true,"command":"report","runs":[{"run_id":"run-1","attempt_id":"attempt-1","terminal_state":"completed","event_count":1,"execution_sha256":"a".repeat(64)}]}),
            false,
        );
        assert!(!report.contains("Next:"));
    }

    #[test]
    fn doctor_hides_envelope_metadata_and_leads_with_outcome() {
        let output = render(
            &["doctor"],
            serde_json::json!({
                "schema_version":1,"ok":true,"command":"doctor","linux":true,
                "architecture":"x86_64","procfs":true,"cgroup_v2":true,
                "interactive_stderr":false,"commands":[],"batch_agent_boundary":"batch-stdio-v1",
                "workloads":[]
            }),
            false,
        );
        assert!(output.starts_with("ASB found this host ready"));
        for irrelevant in [
            "schema_version",
            "interactive_stderr",
            "batch_agent_boundary",
            "ok:",
        ] {
            assert!(!output.contains(irrelevant));
        }
    }

    #[test]
    fn partial_and_warning_outcomes_are_not_called_successes() {
        let compare = render(
            &["compare", "/a", "/b"],
            serde_json::json!({"schema_version":1,"ok":true,"command":"compare","comparable":false,"pairs":[{"left_run_id":"run-1","right_run_id":"run-2","left_agent":"codex","right_agent":"aider","comparable":false,"differences":["agent"]}],"differences":["agent"],"unavailable_reasons":[]}),
            false,
        );
        assert!(compare.contains("not directly comparable"));
        let campaign = render(
            &["record-campaign", "campaign.json", "--local-mock"],
            serde_json::json!({"schema_version":1,"ok":false,"command":"record-campaign","complete_coverage":false,"offline_ready":false,"tuple_count":1,"unavailable_reason":"recording-coverage-incomplete"}),
            false,
        );
        assert!(campaign.starts_with("ASB completed part"));
    }

    #[test]
    fn tui_failure_has_one_valid_secret_free_next_command() {
        let output = render(
            &["tui", "status", "--channel", "dev"],
            serde_json::json!({"schema_version":1,"ok":false,"command":"tui","operation":"status","channel":"dev","classification":"product_failure","code":"development_installation_invalid"}),
            false,
        );
        assert!(output.starts_with("ASB could not complete"));
        assert_eq!(output.matches("Next:").count(), 1);
        assert!(output.ends_with("Next: asb tui install --channel dev\n"));
        assert!(!output.contains("schema_version"));
    }

    #[test]
    fn routed_lifecycle_and_quota_failures_are_not_generic() {
        let cases = [
            ("trusted_tool_invalid", "development trust contract"),
            ("transfer_too_large", "bounded size limit"),
            ("dev_source_identity_unknown", "could not be established"),
            (
                "candidate_execution_failed",
                "before a completed transition",
            ),
            (
                "candidate_request_failed",
                "candidate request did not complete",
            ),
            (
                "candidate_response_invalid",
                "candidate response was malformed",
            ),
            (
                "artifact_transfer_failed",
                "artifact transfer did not complete",
            ),
            ("rollback_state_invalid", "rollback state is malformed"),
            (
                "rollback_state_failed",
                "rollback state could not be durably written",
            ),
            ("candidate_rejected_lifecycle", "requested transition"),
            ("rollback_rejected", "accepted lifecycle state"),
            ("dev_workspace_quota_exceeded", "quota was exhausted"),
            ("artifact_quota_exceeded", "quota was exhausted"),
        ];
        for (code, expected) in cases {
            let output = render(
                &["tui", "status", "--channel", "dev"],
                serde_json::json!({
                    "schema_version":1,"ok":false,"command":"tui",
                    "operation":"status","classification":"product_failure",
                    "code":code,"channel":"dev"
                }),
                false,
            );
            let normalized = output.split_whitespace().collect::<Vec<_>>().join(" ");
            assert!(normalized.contains(expected), "{code}: {output}");
            assert!(!output.contains("unclassified failure"), "{code}: {output}");
        }
    }

    #[test]
    fn every_tui_operation_has_truthful_success_wording() {
        let cases = [
            ("launch", "launched"),
            ("preflight", "preflight"),
            ("status", "installation"),
            ("doctor", "can run"),
            ("install", "installed"),
            ("upgrade", "upgraded"),
            ("remove", "removed"),
            ("live_provider", "live-provider"),
            ("dynamic_catalog", "dynamic-catalog"),
        ];
        for (operation, expected) in cases {
            let output = render(
                &["tui", &operation.replace('_', "-")],
                serde_json::json!({
                    "schema_version":1,"ok":true,"command":"tui",
                    "operation":operation,"classification":null,"code":"ready",
                    "channel":"dev","warnings":["development_rustup_permission_or_ownership_findings_allowed"]
                }),
                false,
            );
            assert!(output.contains(expected), "{operation}: {output}");
            assert_eq!(output.matches("group-writable").count(), 1, "{operation}");
            assert!(!output.contains("development_rustup_permission"));
        }
    }

    #[test]
    fn every_known_warning_has_explicit_consequence_text() {
        let cases = [
            ("development_authentication_warning", "live-provider"),
            (
                "development_missing_authentication_allowed",
                "live-provider",
            ),
            (
                "development_missing_signatures_allowed",
                "production-trusted",
            ),
            ("development_missing_key_management_allowed", "key-rotation"),
            (
                "development_rustup_permission_or_ownership_findings_allowed",
                "permissions",
            ),
            ("attacker_defined_claim", "attacker-defined"),
            ("development_only", "development-only"),
            (
                "development authentication, signatures, and keys are warning-only; discovery does not require them",
                "production trust",
            ),
        ];
        for (code, consequence) in cases {
            let text = warning_text(code);
            assert!(!text.contains("unrecognized"), "{code}: {text}");
            assert!(text.contains(consequence), "{code}: {text}");
        }
        let unknown = warning_text("future_warning_identity");
        assert!(unknown.contains("unrecognized warning identity"));
        assert!(unknown.contains("cannot classify its limitation"));
    }

    #[test]
    fn plan_next_command_is_shell_safe_and_context_aware() {
        let output = render(
            &["plan", "/tmp/plan with quote's.toml", "--use-config"],
            serde_json::json!({"schema_version":1,"ok":true,"command":"plan","run_id":"one","workload":"repository-navigation","agent_implementation":"codex"}),
            false,
        );
        assert!(output.contains("Next: asb run '/tmp/plan with quote'\\''s.toml' --use-config"));
    }

    #[test]
    fn details_are_bounded_and_default_is_clean() {
        let value = serde_json::json!({"ok":true,"command":"provider-catalog","profiles":[{"id":"openrouter","model":"free/model","selectable":true}],"agents":["codex"],"openrouter_free_models":["free/model"],"schema_version":1,"classification":"verified","code":"ready","secret":"must-not-render"});
        let normal = render(&["provider-catalog"], value.clone(), false);
        assert!(!normal.contains("schema_version"));
        assert!(!normal.contains("must-not-render"));
        let details = render(&["provider-catalog"], value, true);
        assert!(!details.contains("Schema version:"));
        assert!(!details.contains("Classification:"));
        assert!(!details.contains("Result code:"));
        assert!(!details.contains("must-not-render"));
    }

    #[test]
    fn width_renderer_wraps_prose_but_keeps_next_command_copyable() {
        let presentation = Presentation {
            kind: OutcomeKind::Succeeded,
            outcome: "ASB completed a deliberately long operation sentence that must wrap cleanly."
                .into(),
            facts: vec![
                "A deliberately long relevant fact also wraps without terminal styling.".into(),
            ],
            warnings: vec![],
            next: NextAction::new(["asb", "plan", "/tmp/a plan.toml"]),
        };
        let mut output = Vec::new();
        write_presentation(&presentation, &mut output, 40).unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(
            output
                .lines()
                .filter(|line| !line.starts_with("Next:"))
                .all(|line| line.len() <= 40)
        );
        assert!(output.ends_with("Next: asb plan '/tmp/a plan.toml'\n"));
        assert!(!output.contains("\u{1b}["));
    }

    #[test]
    fn wrapping_counts_unicode_characters_and_splits_long_paths_and_ids() {
        for width in [40, 80, 120] {
            let presentation = Presentation {
                kind: OutcomeKind::Succeeded,
                outcome: format!("ASB handled {}.", "\u{6e2c}".repeat(width + 5)),
                facts: vec![
                    format!("Artifact: /tmp/{}.", "a".repeat(width * 2)),
                    format!("Identifier: {}.", "\u{e9}".repeat(width + 7)),
                ],
                warnings: vec![format!(
                    "Executable: /opt/{}.",
                    "\u{5de5}\u{5177}".repeat(width)
                )],
                next: Some(NextAction::new(["asb", "plan", "/tmp/a very long plan.toml"]).unwrap()),
            };
            let mut output = Vec::new();
            write_presentation(&presentation, &mut output, width).unwrap();
            let output = String::from_utf8(output).unwrap();
            for line in output.lines().filter(|line| !line.starts_with("Next:")) {
                assert!(line.chars().count() <= width, "width={width}: {line}");
            }
            assert_eq!(output.matches("Next:").count(), 1);
        }
    }

    #[test]
    fn errors_distinguish_user_host_and_product_failures() {
        let mut user = Vec::new();
        render_error(
            &args(&["plan"]),
            &CliError::usage("missing plan"),
            false,
            &mut user,
        )
        .unwrap();
        assert!(
            String::from_utf8(user)
                .unwrap()
                .contains("arguments were not accepted")
        );
        let mut host = Vec::new();
        render_error(
            &args(&["tui"]),
            &CliError::operation_code("trusted_tool_unavailable", "tool unavailable"),
            false,
            &mut host,
        )
        .unwrap();
        assert!(
            String::from_utf8(host)
                .unwrap()
                .contains("required host capability")
        );
        let mut product = Vec::new();
        render_error(
            &args(&["run"]),
            &CliError::operation("journal write failed"),
            true,
            &mut product,
        )
        .unwrap();
        let product = String::from_utf8(product).unwrap();
        assert!(product.contains("ASB did not report a completed result"));
        assert!(product.contains("Diagnostic code: operation (exit 4)."));
        for message in [
            "ASB state is unavailable",
            "OpenRouter credential unavailable",
        ] {
            let mut output = Vec::new();
            render_error(
                &args(&["run"]),
                &CliError::operation(message),
                false,
                &mut output,
            )
            .unwrap();
            assert!(String::from_utf8(output).unwrap().contains(message));
        }
    }

    #[test]
    fn error_presentations_name_cause_state_and_recovery_without_private_text() {
        let cases = [
            (
                CliError::validation("output parent is unavailable")
                    .with_path(Path::new("/tmp/asb-results/report.json")),
                "required parent directory does not exist",
                "Create or select the named parent directory",
            ),
            (
                CliError::validation("output path is not a directory")
                    .with_path(Path::new("/tmp/asb-results/report.json")),
                "selected path is not a directory",
                "Inspect the named destination",
            ),
            (
                CliError::validation("output path is a symlink")
                    .with_path(Path::new("/tmp/asb-results/report.json")),
                "path topology is unsafe",
                "Inspect the named destination",
            ),
            (
                CliError::operation_code(
                    "provider_credential_unavailable",
                    "OpenRouter credential unavailable",
                ),
                "provider authentication could not be established",
                "Establish the provider credential",
            ),
            (
                CliError::operation_code(
                    "provider_transport_failed",
                    "OpenRouter transport failed",
                ),
                "bounded transport exchange did not complete",
                "Check the selected provider",
            ),
            (
                CliError::operation("operation timed out"),
                "bounded deadline expired",
                "Inspect durable state",
            ),
            (
                CliError::operation("operation was cancelled"),
                "operation was cancelled before completion",
                "Inspect durable state",
            ),
        ];
        for (error, cause, recovery) in cases {
            let mut output = Vec::new();
            render_error(&args(&["run"]), &error, false, &mut output).unwrap();
            let output = String::from_utf8(output).unwrap();
            let compact = output.split_whitespace().collect::<Vec<_>>().join(" ");
            assert!(compact.contains(cause), "{output}");
            assert!(compact.contains(recovery), "{output}");
            assert!(
                output.contains("No relevant state was changed.")
                    || output.contains("No external operation was started.")
                    || output.contains("resulting durable state"),
                "{output}"
            );
            assert!(!output.contains("/home/martin"), "{output}");
        }
    }

    #[test]
    fn known_error_classes_have_distinct_human_explanations() {
        let errors = [
            CliError::validation("input is missing"),
            CliError::validation("input is malformed"),
            CliError::validation("input is incompatible"),
            CliError::validation("input permission denied"),
            CliError::operation("provider request timed out"),
            CliError::operation("provider request was cancelled"),
        ];
        let outputs = errors
            .iter()
            .map(|error| {
                let mut output = Vec::new();
                render_error(&args(&["run"]), error, false, &mut output).unwrap();
                String::from_utf8(output).unwrap()
            })
            .collect::<Vec<_>>();
        for left in 0..outputs.len() {
            for right in left + 1..outputs.len() {
                assert_ne!(outputs[left], outputs[right]);
            }
        }
    }

    #[test]
    fn redirected_and_no_color_contract_never_emits_ansi() {
        let output = render(
            &["workload-catalog"],
            serde_json::json!({"schema_version":1,"ok":true,"command":"workload-catalog","entries":[{"id":"original.bug-fix","kind":"builtin","availability":"available"}]}),
            false,
        );
        assert!(!output.contains("\u{1b}["));
        assert_eq!(
            output,
            "ASB loaded the benchmark workload catalog.\n  Workloads: 1.\n  Runnable workloads: original.bug-fix.\n  Choose a listed workload and create a configured plan with: asb plan create\n  --workload WORKLOAD --agent AGENT --agent-executable AGENT_EXECUTABLE --output\n  PLAN --use-config.\n"
        );
    }

    #[test]
    fn typed_special_cases_accept_real_shapes_and_reject_mismatches() {
        let tui = render(
            &["tui", "doctor"],
            serde_json::json!({
                "schema_version":1,"ok":true,"command":"tui","operation":"doctor",
                "code":"extension_not_installed","network":"denied","channel":"dev",
                "development_only":true
            }),
            false,
        );
        assert!(tui.starts_with("ASB verified"));

        let create = render(
            &["plan", "create", "--output", "/tmp/plan.toml"],
            serde_json::json!({
                "schema_version":1,"ok":true,"command":"plan-create",
                "plan":"/tmp/plan.toml","workload":"original.bug-fix",
                "experiment_sha256":"a".repeat(64)
            }),
            false,
        );
        assert!(create.contains("Plan: /tmp/plan.toml."));
        assert!(create.ends_with("Next: asb plan /tmp/plan.toml\n"));

        let malformed = serde_json::json!({"schema_version":1,"ok":true,"command":"plan"});
        let mut output = Vec::new();
        assert!(
            render_success(
                &args(&["plan", "/tmp/plan.toml"]),
                &serde_json::to_vec(&malformed).unwrap(),
                false,
                &mut output
            )
            .is_err()
        );
        assert!(output.is_empty());

        let auth_error = render(
            &["auth", "status", "--socket", "/tmp/control.sock"],
            serde_json::json!({"jsonrpc":"2.0","id":"request-1","error":{"code":-1,"message":"rejected"}}),
            false,
        );
        assert!(auth_error.starts_with("The control service rejected"));
        assert!(!auth_error.contains("completed the authentication"));
    }

    #[test]
    fn every_easy_alias_has_an_explicit_invocation_and_family() {
        let cases = [
            ("setup", CommandKind::Setup),
            ("build", CommandKind::Easy),
            ("install", CommandKind::Easy),
            ("update", CommandKind::Easy),
            ("test", CommandKind::Easy),
            ("status", CommandKind::Easy),
            ("rollback", CommandKind::Easy),
            ("remove", CommandKind::Easy),
            ("provider-catalog", CommandKind::ProviderCatalog),
            ("adapter-catalog", CommandKind::AdapterCatalog),
            ("plan", CommandKind::Plan),
            ("run", CommandKind::Run),
            ("sweep", CommandKind::Sweep),
            ("report", CommandKind::Report),
            ("compare", CommandKind::Compare),
            ("record", CommandKind::Record),
            ("record-live", CommandKind::RecordLive),
            ("record-campaign", CommandKind::RecordCampaign),
            ("replay", CommandKind::Replay),
            ("replay-offline", CommandKind::ReplayOffline),
        ];
        for (subcommand, expected) in cases {
            let invocation = InvocationKind::classify_words(&["easy", subcommand]);
            assert_ne!(invocation, InvocationKind::Easy(EasyKind::Unknown));
            assert_eq!(invocation.family(), expected, "easy {subcommand}");
        }
    }

    #[test]
    fn structured_next_actions_reject_controls_and_reparse() {
        assert!(NextAction::new(["asb", "plan", "/tmp/a\nplan"]).is_none());
        assert!(NextAction::new(["asb", "plan", ""]).is_none());
        let action = plan_next(
            InvocationKind::Plan { create: false },
            &args(&[
                "plan",
                "/tmp/a plan.toml",
                "--provider-selection",
                "/tmp/selection's.json",
            ]),
            &PresentationContext::None,
        )
        .unwrap();
        assert_eq!(action.argv.first().map(String::as_str), Some("asb"));
        let words = action.argv[1..]
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        assert_eq!(InvocationKind::classify_words(&words), InvocationKind::Run);
        assert_eq!(action.render().matches('\n').count(), 0);
        assert!(action.render().contains("'/tmp/a plan.toml'"));
        assert!(action.render().contains("'/tmp/selection'\\''s.json'"));

        let easy = plan_next(
            InvocationKind::Easy(EasyKind::Plan),
            &args(&["easy", "plan", "/tmp/easy-plan.toml", "--use-config"]),
            &PresentationContext::None,
        )
        .unwrap();
        assert_eq!(
            easy.argv,
            ["asb", "run", "/tmp/easy-plan.toml", "--use-config"]
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        );

        let sweep = plan_next(
            InvocationKind::Plan { create: false },
            &args(&[
                "plan",
                "/tmp/sweep plan.toml",
                "--provider-selection",
                "/tmp/selection's.json",
            ]),
            &PresentationContext::Plan {
                sweep_available: true,
            },
        )
        .unwrap();
        assert_eq!(sweep.argv[1], "sweep");
        assert_eq!(
            InvocationKind::classify_words(
                &sweep.argv[1..]
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
            ),
            InvocationKind::Sweep
        );
        assert!(sweep.render().contains("'/tmp/sweep plan.toml'"));
        assert!(sweep.render().contains("'/tmp/selection'\\''s.json'"));
    }

    #[test]
    fn easy_record_reports_the_output_not_the_input() {
        let value = serde_json::json!({
            "schema_version":1,"cassette_id":"one","cassette_sha256":"a".repeat(64),
            "provider_profile_sha256":"b".repeat(64),"agent_id":"codex",
            "interactions":1,"redaction":{},"network":"denied","complete":true,"source":"fixture"
        });
        let output = render(
            &[
                "easy",
                "record",
                "/tmp/input.json",
                "/tmp/output.json",
                "--local-mock",
            ],
            value,
            false,
        );
        assert!(output.contains("Cassette: /tmp/output.json."));
        assert!(!output.contains("Cassette: /tmp/input.json."));
    }

    #[test]
    fn execution_invariants_reject_false_success_and_vacuous_results() {
        rejects(
            &["run", "plan.toml"],
            serde_json::json!({"schema_version":1,"ok":true,"command":"run","run_ids":["one"],"points":[{"decision":"fail"}],"cancelled":false}),
        );
        rejects(
            &["run", "plan.toml"],
            serde_json::json!({"schema_version":1,"ok":false,"command":"run","run_ids":["one"],"points":[{"decision":"pass"}],"cancelled":false}),
        );
        for words in [
            &["run", "plan.toml"][..],
            &["sweep", "plan.toml"][..],
            &["benchmark-live", "plan.toml"][..],
        ] {
            let command = if words[0] == "sweep" { "sweep" } else { "run" };
            rejects(
                words,
                serde_json::json!({"schema_version":1,"ok":true,"command":command,"run_ids":[],"points":[],"cancelled":false}),
            );
        }
        let cancelled = render(
            &["run", "plan.toml"],
            serde_json::json!({"schema_version":1,"ok":false,"command":"run","run_ids":[],"points":[],"cancelled":true}),
            false,
        );
        assert!(cancelled.contains("cancelled"));
        assert!(!cancelled.contains("Next:"));
    }

    #[test]
    fn auth_results_are_operation_typed_truthful_and_private() {
        let digest = "a".repeat(64);
        let status = render(
            &["auth", "status", "--socket", "/tmp/control.sock"],
            serde_json::json!({"jsonrpc":"2.0","id":1,"result":{"kind":"auth_status","value":{"provider":"openrouter","endpoint_identity_sha256":digest,"credential_locator_sha256":"b".repeat(64),"generation":2,"status":"enrolled"}}}),
            false,
        );
        assert!(status.contains("Provider: openrouter."));
        assert!(status.contains("Lifecycle status: enrolled."));
        assert!(status.contains("Generation: 2."));
        assert!(!status.contains(&"a".repeat(64)));
        for operation in ["enroll", "rotate", "revoke"] {
            let accepted = render(
                &["auth", operation, "--socket", "/tmp/control.sock"],
                serde_json::json!({"jsonrpc":"2.0","id":1,"result":{"kind":"acknowledged","value":{"accepted":true}}}),
                false,
            );
            assert!(accepted.contains("completed"));
            let refused = render(
                &["auth", operation, "--socket", "/tmp/control.sock"],
                serde_json::json!({"jsonrpc":"2.0","id":1,"result":{"kind":"acknowledged","value":{"accepted":false}}}),
                false,
            );
            assert!(refused.contains("did not accept"));
            assert!(!refused.contains("success"));
            let error = render(
                &["auth", operation, "--socket", "/tmp/control.sock"],
                serde_json::json!({"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"rejected"}}),
                false,
            );
            assert!(error.contains("rejected"));
        }
    }

    #[test]
    fn provider_plan_lists_typed_agents_and_rejects_malformed_entries() {
        let value = serde_json::json!({
            "schema_version":1,"ok":true,"command":"provider-plan","dry_run":true,
            "provider_profile":"openrouter","model":"free/model",
            "effective":[
                {"agent":"codex","profile_sha256":"a".repeat(64),"api_mode":"responses"},
                {"agent":"opendesk","profile_sha256":"b".repeat(64),"api_mode":"chat_completions"}
            ]
        });
        let output = render(&["provider-plan"], value, false);
        assert!(output.contains("codex (responses)"));
        assert!(output.contains("opendesk (chat completions)"));
        rejects(
            &["provider-plan"],
            serde_json::json!({"schema_version":1,"ok":true,"command":"provider-plan","dry_run":true,"provider_profile":"openrouter","model":"free/model","effective":[{}]}),
        );
    }

    #[test]
    fn details_sanitize_controls_and_schema_versions_fail_closed() {
        let value = serde_json::json!({
            "schema_version":1,"ok":true,"command":"provider-catalog",
            "profiles":[{"id":"openrouter","model":"free/model","selectable":true}],
            "agents":["codex"],"openrouter_free_models":[],
            "classification":"verified\n\u{1b}[31m","code":"ready\tbad"
        });
        let output = render(&["provider-catalog"], value, true);
        assert!(!output.contains('\u{1b}'));
        assert!(!output.contains("\n[31m"));
        rejects(
            &["provider-catalog"],
            serde_json::json!({"schema_version":0,"ok":true,"command":"provider-catalog","profiles":[{"id":"openrouter","model":"free/model","selectable":true}],"agents":["codex"],"openrouter_free_models":[]}),
        );
        rejects(
            &["setup"],
            serde_json::json!({"schema_version":1,"ok":true,"command":"setup","mode":"preflight","persistent_change":false,"persisted":false,"selected_agents":[]}),
        );
        for invalid in [
            serde_json::json!({"schema_version":2,"ok":true,"command":"setup","mode":"preflight","persistent_change":true,"persisted":false,"selected_agents":[],"provider_profile":null,"model":null}),
            serde_json::json!({"schema_version":2,"ok":true,"command":"setup","mode":"commit","persistent_change":false,"persisted":false,"selected_agents":[],"provider_profile":null,"model":null}),
            serde_json::json!({"schema_version":2,"ok":true,"command":"setup","mode":"commit","persistent_change":true,"persisted":true,"selected_agents":["codex"],"provider_profile":null,"model":"free/model"}),
            serde_json::json!({"schema_version":2,"ok":true,"command":"setup","mode":"commit","persistent_change":true,"persisted":true,"selected_agents":["codex"],"provider_profile":"openrouter","model":null}),
        ] {
            rejects(&["setup"], invalid);
        }
    }

    #[test]
    fn compare_global_and_pair_outcomes_are_consistent() {
        let pair = serde_json::json!({
            "left_run_id":"one","right_run_id":"two",
            "left_agent":"codex","right_agent":"codex",
            "comparable":false,"differences":[]
        });
        rejects(
            &["compare", "/one", "/two"],
            serde_json::json!({"schema_version":1,"ok":true,"command":"compare","comparable":true,"pairs":[pair.clone()],"differences":[],"unavailable_reasons":[]}),
        );
        rejects(
            &["compare", "/one", "/two"],
            serde_json::json!({"schema_version":1,"ok":true,"command":"compare","comparable":false,"pairs":[pair],"differences":[],"unavailable_reasons":[]}),
        );
    }

    #[test]
    fn typed_presentation_boundary_rejects_injectable_display_fields() {
        rejects(
            &["tui", "status"],
            serde_json::json!({
                "schema_version":1,"ok":true,"command":"tui","operation":"status",
                "classification":null,"code":"ready","channel":"dev",
                "warnings":["attacker_defined_claim"]
            }),
        );
        rejects(
            &["record-campaign", "campaign.json", "--local-mock"],
            serde_json::json!({
                "schema_version":1,"ok":false,"command":"record-campaign",
                "tuple_count":1,"complete_coverage":false,"offline_ready":false,
                "unavailable_reason":"attacker-defined-claim"
            }),
        );
        rejects(
            &["run", "plan.toml"],
            serde_json::json!({
                "schema_version":1,"ok":true,"command":"run","run_ids":["run-1"],
                "points":[{"decision":"pass"}],"cancelled":false,
                "highest_confirmed_capacity":"unbounded"
            }),
        );
        let output = render(
            &["run", "plan.toml"],
            serde_json::json!({
                "schema_version":1,"ok":true,"command":"run","run_ids":["run-1"],
                "points":[{"decision":"pass"}],"cancelled":false,
                "highest_confirmed_capacity":1,"attacker_claim":"trusted"
            }),
            false,
        );
        assert!(output.contains("Highest confirmed concurrency: 1."));
        assert!(!output.contains("attacker"));
        assert!(!output.contains("trusted"));

        for (words, value) in [
            (
                &["doctor"][..],
                serde_json::json!({
                    "schema_version":1,"ok":true,"command":"doctor","linux":true,
                    "architecture":"x86_64","procfs":true,"cgroup_v2":true,
                    "classification":"attacker_claim","code":"trusted"
                }),
            ),
            (
                &["setup"][..],
                serde_json::json!({
                    "schema_version":2,"ok":true,"command":"setup","mode":"preflight",
                    "persistent_change":false,"persisted":false,"selected_agents":[],
                    "provider_profile":null,"model":null,
                    "classification":"attacker_claim","code":"trusted"
                }),
            ),
        ] {
            let output = render(words, value, true);
            assert!(!output.contains("attacker"));
            assert!(!output.contains("trusted"));
            assert!(!output.contains("Classification:"));
            assert!(!output.contains("Result code:"));
        }
    }
}
