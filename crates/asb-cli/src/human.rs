// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Human-first presentation for the public command-line interface.
//!
//! Structured command results remain the authority. This module has an
//! explicit entry for every public command family and deliberately has no
//! recursive JSON fallback: adding a command requires choosing its human
//! presentation as well.

use super::{CliError, HumanErrorClass};
use serde::Deserialize;
use serde_json::{Map, Value};
use std::ffi::OsString;
use std::io::{self, Write};

const DEFAULT_WIDTH: usize = 80;
const MIN_WIDTH: usize = 40;
const MAX_WIDTH: usize = 120;

/// Authoritative public top-level command inventory. Dispatch rejects any
/// command that is not classified below; doctor and shell completion consume
/// this same registry so those discovery surfaces cannot silently drift.
pub(super) const PUBLIC_COMMANDS: &[&str] = &[
    "doctor",
    "setup",
    "capabilities",
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AuthKind {
    Setup,
    Enroll,
    Status,
    Rotate,
    Revoke,
    Unknown,
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
}

#[derive(Deserialize)]
struct CatalogView {
    #[serde(rename = "schema_version")]
    _schema_version: u16,
    ok: bool,
    command: String,
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
    result: Option<Value>,
    #[serde(default)]
    error: Option<Value>,
    #[serde(default)]
    method: Option<String>,
}

#[derive(Deserialize)]
struct ProviderPlanView {
    #[serde(rename = "schema_version")]
    _schema_version: u16,
    ok: bool,
    command: String,
    provider_profile: String,
    model: String,
    effective: Vec<Value>,
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
}

#[derive(Deserialize)]
struct CompareView {
    #[serde(rename = "schema_version")]
    _schema_version: u16,
    ok: bool,
    command: String,
    comparable: bool,
    differences: Vec<String>,
    pairs: Vec<Value>,
    unavailable_reasons: Vec<String>,
}

#[derive(Deserialize)]
struct ReportView {
    #[serde(rename = "schema_version")]
    _schema_version: u16,
    ok: bool,
    command: String,
    runs: Vec<Value>,
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

fn validate_typed_result(
    invocation: InvocationKind,
    args: &[OsString],
    captured: &[u8],
) -> io::Result<()> {
    let invalid = || {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "command result discriminator does not match its invocation",
        )
    };
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
                || value.selected_agents.len() > 9
            {
                return Err(invalid());
            }
        }
        InvocationKind::Easy(EasyKind::Lifecycle) => {
            let value: LifecycleView = decode(captured)?;
            if !value.ok
                || value.command != "easy"
                || value.operation.is_empty()
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
        InvocationKind::Tui(_) => {
            let value: TuiView = decode(captured)?;
            if value.command != "tui"
                || value.operation.is_empty()
                || (!value.ok && value.classification.as_deref().is_none_or(str::is_empty))
                || (!value.ok && value.code.as_deref().is_none_or(str::is_empty))
            {
                return Err(invalid());
            }
        }
        InvocationKind::Capabilities => {
            super::capabilities::CapabilityResponse::parse(captured).map_err(|_| invalid())?;
        }
        InvocationKind::ProviderCatalog { .. }
        | InvocationKind::Easy(EasyKind::ProviderCatalog) => {
            validate_catalog(captured, "provider-catalog")?;
        }
        InvocationKind::AdapterCatalog | InvocationKind::Easy(EasyKind::AdapterCatalog) => {
            validate_catalog(captured, "adapter-catalog")?;
        }
        InvocationKind::WorkloadCatalog => validate_catalog(captured, "workload-catalog")?,
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
        InvocationKind::Auth(_) => {
            let value: JsonRpcView = decode(captured)?;
            let submitted = args.iter().any(|arg| arg == "--socket");
            let valid_outcome = if submitted {
                value.result.is_some() != value.error.is_some() && value.method.is_none()
            } else {
                value.result.is_none()
                    && value.error.is_none()
                    && value
                        .method
                        .as_deref()
                        .is_some_and(|method| method.starts_with("auth_"))
            };
            if value.jsonrpc != "2.0" || value.id.is_null() || !valid_outcome {
                return Err(invalid());
            }
        }
        InvocationKind::ProviderPlan => {
            let value: ProviderPlanView = decode(captured)?;
            if !value.ok
                || value.command != "provider-plan"
                || value.provider_profile.is_empty()
                || value.model.is_empty()
                || value.effective.is_empty()
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
            if !matches!(value.command.as_str(), "run" | "sweep")
                || value.run_ids.len() != value.points.len()
                || value.cancelled && value.ok
                || value.points.iter().any(|point| {
                    matches!(point.decision, PointDecisionView::Inconclusive) && value.ok
                })
            {
                return Err(invalid());
            }
        }
        InvocationKind::Compare | InvocationKind::Easy(EasyKind::Compare) => {
            let value: CompareView = decode(captured)?;
            if !value.ok
                || value.command != "compare"
                || value.pairs.is_empty()
                || value.comparable
                    && (!value.differences.is_empty() || !value.unavailable_reasons.is_empty())
            {
                return Err(invalid());
            }
        }
        InvocationKind::Report | InvocationKind::Easy(EasyKind::Report) => {
            let value: ReportView = decode(captured)?;
            if !value.ok || value.command != "report" || value.runs.is_empty() {
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
            if !value.ok
                || !matches!(value.command.as_str(), "replay" | "replay-offline")
                || value.network != "denied"
                || value.agent_id.is_empty()
                || value.response_status == 0
            {
                return Err(invalid());
            }
        }
        InvocationKind::Help
        | InvocationKind::Version
        | InvocationKind::Easy(EasyKind::Help)
        | InvocationKind::Completion
        | InvocationKind::Serve
        | InvocationKind::InternalOpenCodeBatch => {}
        InvocationKind::Cli | InvocationKind::Easy(EasyKind::Unknown) => return Err(invalid()),
    }
    Ok(())
}

fn validate_catalog(captured: &[u8], expected: &str) -> io::Result<()> {
    let value: CatalogView = decode(captured)?;
    if value.ok && value.command == expected {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "catalog result discriminator does not match its invocation",
        ))
    }
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

pub(super) fn render_success(
    args: &[OsString],
    captured: &[u8],
    details: bool,
    output: &mut dyn Write,
) -> io::Result<()> {
    let kind = CommandKind::parse(args)?;
    if kind.raw(args) {
        return output.write_all(captured);
    }
    let invocation = InvocationKind::classify_args(args);
    validate_typed_result(invocation, args, captured)?;
    let value: Value = serde_json::from_slice(captured).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{} returned no structured human result", kind.label()),
        )
    })?;
    let object = value.as_object().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{} returned an invalid result shape", kind.label()),
        )
    })?;
    let mut presentation = present(invocation, args, object);
    if details {
        add_details(&mut presentation, object);
    }
    write_presentation(&presentation, output, terminal_width())
}

pub(super) fn render_error(
    args: &[OsString],
    error: &CliError,
    details: bool,
    output: &mut dyn Write,
) -> io::Result<()> {
    let kind = CommandKind::parse(args).unwrap_or(CommandKind::Cli);
    let mut presentation = Presentation::new(format!(
        "ASB could not complete the {}: {}.",
        kind.label(),
        sentence_fragment(error.message)
    ));
    match error.exit_code {
        _ if error.human_class == HumanErrorClass::UserCorrection && error.exit_code == 2 => {
            presentation.kind = OutcomeKind::UserError;
            presentation.fact("The command arguments were not accepted.");
            presentation.fact("No successful state change was reported.");
            presentation.next = NextAction::new(["asb", "--help"]);
        }
        _ if error.human_class == HumanErrorClass::UserCorrection => {
            presentation.kind = OutcomeKind::UserError;
            presentation.fact("The requested input or configuration needs correction.");
            presentation.fact("No successful state change was reported.");
            presentation.next = validation_next(args, error);
        }
        _ if matches!(
            error.human_class,
            HumanErrorClass::HostLimitation | HumanErrorClass::DependencyUnavailable
        ) =>
        {
            presentation.kind = if error.human_class == HumanErrorClass::HostLimitation {
                OutcomeKind::HostLimitation
            } else {
                OutcomeKind::Warning
            };
            presentation.fact(if error.human_class == HumanErrorClass::HostLimitation {
                "A required host capability is unavailable."
            } else {
                "A required external dependency is unavailable."
            });
            presentation.next = NextAction::new(["asb", "doctor"]);
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
    if details {
        presentation.fact(format!(
            "Diagnostic code: {} (exit {}).",
            error.code, error.exit_code
        ));
    }
    write_presentation(&presentation, output, terminal_width())
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
    let path = args
        .get(1)
        .and_then(|arg| arg.to_str())
        .map(shell_quote)
        .unwrap_or_else(|| "the supplied configuration".to_owned());
    writeln!(
        output,
        "ASB is attempting to start the control service from {path}."
    )?;
    output.flush()
}

fn present(
    invocation: InvocationKind,
    args: &[OsString],
    object: &Map<String, Value>,
) -> Presentation {
    let kind = invocation.family();
    match kind {
        CommandKind::Doctor => present_doctor(object),
        CommandKind::Setup => present_setup(args, object),
        CommandKind::Easy => present_easy(args, object),
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
        CommandKind::ProviderCatalog => present_catalog(
            "ASB loaded the provider catalog.",
            object,
            "profiles",
            "provider profiles",
            "id",
        ),
        CommandKind::AdapterCatalog => present_catalog(
            "ASB loaded the coding-agent adapter catalog.",
            object,
            "adapters",
            "adapters",
            "id",
        ),
        CommandKind::WorkloadCatalog => present_catalog(
            "ASB loaded the benchmark workload catalog.",
            object,
            "entries",
            "workloads",
            "id",
        ),
        CommandKind::Config => {
            let mut value = Presentation::new("ASB saved the OpenRouter configuration.");
            fact_pair(&mut value, object, "Provider", "provider");
            fact_pair(&mut value, object, "Model", "model");
            value.next = NextAction::new(["asb", "provider-plan", "--use-config"]);
            value
        }
        CommandKind::Auth => present_auth(args, object),
        CommandKind::ProviderPlan => {
            let mut value = Presentation::new("ASB prepared the provider selection.");
            fact_pair(&mut value, object, "Provider", "provider_profile");
            fact_pair(&mut value, object, "Model", "model");
            fact_count(&mut value, object, "effective", "selected agents");
            value
        }
        CommandKind::Plan => present_plan(invocation, args, object),
        CommandKind::Run | CommandKind::Sweep | CommandKind::BenchmarkLive => {
            present_execution(kind, object)
        }
        CommandKind::Compare => present_compare(object),
        CommandKind::Report => present_report(object),
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
    fact_count(&mut value, object, "selected_agents", "selected agents");
    value.next = if persisted {
        NextAction::new(["asb", "provider-plan", "--use-config"])
    } else {
        NextAction::new(["asb", "provider-catalog"])
    };
    value
}

fn present_easy(args: &[OsString], object: &Map<String, Value>) -> Presentation {
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
            human_code(string(object, "code").unwrap_or("operation failed"))
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
            value.warning(display_value(warning));
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
) -> Presentation {
    let mut value = Presentation::new(outcome);
    fact_count(&mut value, object, field, label);
    if let Some(items) = object.get(field).and_then(Value::as_array) {
        let ids = items
            .iter()
            .filter_map(|item| string_value(item, id_field))
            .take(12)
            .map(display_value)
            .collect::<Vec<_>>();
        if !ids.is_empty() {
            value.fact(format!("Catalog {label}: {}.", ids.join(", ")));
        }
    }
    if field == "profiles" {
        let selectable = object
            .get("profiles")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|profile| profile.get("selectable").and_then(Value::as_bool) == Some(true))
            .filter_map(|profile| string_value(profile, "id"))
            .take(12)
            .map(display_value)
            .collect::<Vec<_>>();
        if !selectable.is_empty() {
            value.fact(format!("Selectable providers: {}.", selectable.join(", ")));
        }
        if let Some(models) = object
            .get("openrouter_free_models")
            .and_then(Value::as_array)
        {
            let models = models
                .iter()
                .filter_map(Value::as_str)
                .take(8)
                .map(display_value)
                .collect::<Vec<_>>();
            if !models.is_empty() {
                value.fact(format!("OpenRouter free models: {}.", models.join(", ")));
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
    let mut value = Presentation::new(if rejected {
        format!("The control service rejected the authentication {operation} request.")
    } else if submitted {
        format!("ASB completed the authentication {operation} request through the control service.")
    } else {
        format!("ASB prepared the authentication {operation} request without sending it.")
    });
    if rejected {
        value.kind = OutcomeKind::ProductFailure;
        value.warning("No authentication state change was reported.");
    }
    value
}

fn present_plan(
    invocation: InvocationKind,
    args: &[OsString],
    object: &Map<String, Value>,
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
    value.next = plan_next(invocation, args);
    value
}

fn present_execution(kind: CommandKind, object: &Map<String, Value>) -> Presentation {
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
    }
    value
}

fn present_report(object: &Map<String, Value>) -> Presentation {
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
        value.warning(format!("Incomplete because {}.", human_code(reason)));
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

fn plan_next(invocation: InvocationKind, args: &[OsString]) -> Option<NextAction> {
    let words = args
        .iter()
        .filter_map(|arg| arg.to_str())
        .collect::<Vec<_>>();
    if words.get(1) == Some(&"create") {
        let path = words
            .windows(2)
            .find(|pair| pair[0] == "--output")
            .map(|pair| pair[1])?;
        return NextAction::new(["asb", "plan", path]);
    }
    let path = words.get(
        if matches!(invocation, InvocationKind::Easy(EasyKind::Plan)) {
            2
        } else {
            1
        },
    )?;
    let mut command = vec!["asb".to_owned(), "run".to_owned(), (*path).to_owned()];
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
        && error.message.contains("--confirm-record")
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
    NextAction::new(["asb", "--help"])
}

fn add_details(presentation: &mut Presentation, object: &Map<String, Value>) {
    for (label, key) in [
        ("Result code", "code"),
        ("Classification", "classification"),
        ("Schema version", "schema_version"),
    ] {
        if let Some(value) = object.get(key).and_then(scalar) {
            presentation.fact(format!("{label}: {value}."));
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
    let available = width.saturating_sub(prefix.len()).max(20);
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.len() + 1 + word.len() > available {
            writeln!(output, "{prefix}{line}")?;
            line.clear();
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    writeln!(output, "{prefix}{line}")
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
    let mut value = sanitized.trim().trim_end_matches('.').to_owned();
    if let Some(first) = value.get_mut(0..1) {
        first.make_ascii_lowercase();
    }
    value
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

    #[test]
    fn public_command_inventory_is_explicit_and_has_no_unknown_fallback() {
        let commands: &[&[&str]] = &[
            &["doctor"],
            &["setup"],
            &["easy", "run"],
            &["tui", "launch"],
            &["capabilities"],
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
                serde_json::json!({"schema_version":1,"ok":true,"command":"setup","mode":"preflight","persistent_change":false,"persisted":false,"selected_agents":[]}),
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
                vec!["provider-catalog"],
                serde_json::json!({"schema_version":1,"ok":true,"command":"provider-catalog","profiles":[],"agents":[],"openrouter_free_models":[]}),
            ),
            (
                vec!["adapter-catalog"],
                serde_json::json!({"schema_version":1,"ok":true,"command":"adapter-catalog","adapters":[]}),
            ),
            (
                vec!["workload-catalog"],
                serde_json::json!({"schema_version":1,"ok":true,"command":"workload-catalog","entries":[]}),
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
                serde_json::json!({"schema_version":1,"ok":true,"command":"provider-plan","provider_profile":"openrouter","model":"free/model","effective":[{}]}),
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
                serde_json::json!({"schema_version":1,"ok":true,"command":"compare","comparable":true,"differences":[],"pairs":[{}],"unavailable_reasons":[]}),
            ),
            (
                vec!["report", "/a"],
                serde_json::json!({"schema_version":1,"ok":true,"command":"report","runs":[{"run_id":"run-1","terminal_state":"completed","event_count":2}]}),
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
            serde_json::json!({"schema_version":1,"ok":true,"command":"report","runs":[{"run_id":"run-1","terminal_state":"completed","event_count":1}]}),
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
            serde_json::json!({"schema_version":1,"ok":true,"command":"compare","comparable":false,"pairs":[{}],"differences":["agent"],"unavailable_reasons":[]}),
            false,
        );
        assert!(compare.contains("not directly comparable"));
        let campaign = render(
            &["record-campaign", "campaign.json", "--local-mock"],
            serde_json::json!({"schema_version":1,"ok":false,"command":"record-campaign","complete_coverage":false,"offline_ready":false,"tuple_count":1,"unavailable_reason":"missing_tuple"}),
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
                    "channel":"dev","warnings":["one warning"]
                }),
                false,
            );
            assert!(output.contains(expected), "{operation}: {output}");
            assert_eq!(output.matches("one warning").count(), 1, "{operation}");
        }
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
        let value = serde_json::json!({"ok":true,"command":"provider-catalog","profiles":[],"schema_version":1,"classification":"verified","code":"ready","secret":"must-not-render"});
        let normal = render(&["provider-catalog"], value.clone(), false);
        assert!(!normal.contains("schema_version"));
        assert!(!normal.contains("must-not-render"));
        let details = render(&["provider-catalog"], value, true);
        assert!(details.contains("Schema version: 1."));
        assert!(details.contains("Classification: verified."));
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
    }

    #[test]
    fn redirected_and_no_color_contract_never_emits_ansi() {
        let output = render(
            &["workload-catalog"],
            serde_json::json!({"schema_version":1,"ok":true,"command":"workload-catalog","entries":[]}),
            false,
        );
        assert!(!output.contains("\u{1b}["));
        assert_eq!(
            output,
            "ASB loaded the benchmark workload catalog.\n  Workloads: 0.\n"
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
        )
        .unwrap();
        assert_eq!(
            easy.argv,
            ["asb", "run", "/tmp/easy-plan.toml", "--use-config"]
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        );
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
}
