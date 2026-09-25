// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Stable terminal and automation interface for Agent Systems Benchmark.

pub mod capabilities;
mod control;
mod provider_launch;
mod tui;
pub mod tui_handoff;

use asb_agents::all_agents_provider::{
    ALL_AGENTS_PROVIDER_SELECTION_V1, AllAgentsProviderKind, AllAgentsProviderPlan,
    AllAgentsProviderSelection, EffectiveApiMode, SelectedAgent, resolve_openai_selection,
    resolve_openrouter_selection,
};
use asb_agents::openai::OpenAiProfile;
use asb_agents::openrouter::OpenRouterProfile;
use asb_agents::provider_launch::{
    LaunchPolicy, ProviderLaunchProjection, ProviderLaunchRecord, ProviderLaunchV1,
    RuntimeBundleIdentity, credential_target_for_provider_agent,
};
use asb_analysis::{ComparisonField, compare_experiments};
use asb_config::{ConfigStore, Configuration, OpenRouterFreeModelConfig};
use asb_metrics::LinuxCollector;
use asb_protocol::{
    ExperimentManifestV1, Id, MeasurementArchitecture, MeasurementExecutionCapabilities,
    MeasurementOperatingSystem, MeasurementPlatformFeature, MeasurementSelectionError,
    MeasurementSelectionReason, MeasurementSelectionV1, baseline_measurement_catalog,
};
use asb_replay::{
    CassetteLimits, ExecutionSource, RecordingCapture, RecordingDescriptor, RecordingIndex,
    SourceChoice, seal_recording,
};
use asb_runtime::launch_factory::{
    LaunchAuthorityError, LiveProviderAttempt, LiveProviderAttemptFactory, ReplayLaunchAuthority,
};
use asb_runtime::live_service::{
    LiveProviderRuntimeDispatchSource, LiveProviderRuntimeScheduler, LocalProviderMockBackend,
};
use asb_runtime::sandbox::SandboxProcess;
use asb_runtime::scheduler::{
    AttemptOutcome, CapacityDecision, CapacityPoint, LoadModel, MissReason, PointPlan, Scheduler,
    StopReason, SystemClock, capacity_order, highest_confirmed_capacity,
};
use asb_runtime::{ProcessError, ProcessLimits, RunningProcess, Termination};
use asb_store::{
    AtomicStore, ExecutionState, JOURNAL_SCHEMA_VERSION, JournalEvent, MANIFEST_SCHEMA_VERSION,
    RunManifest, StoreLimits,
};
use asb_workloads::{
    CatalogKind, OriginalWorkloads, PreparedWorkloadChoice, WorkloadEvaluation, describe_workload,
    prepare_workload, workload_catalog,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use signal_hook::consts::{SIGINT, SIGTERM};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{self, IsTerminal, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const OUTPUT_SCHEMA_VERSION: u16 = 1;
const LEGACY_PLAN_SCHEMA_VERSION: u16 = 1;
const PLAN_SCHEMA_VERSION: u16 = 2;
const MAX_PLAN_BYTES: u64 = 1024 * 1024;
const MAX_PROVIDER_SELECTION_BYTES: u64 = 64 * 1024;
const MAX_EXECUTABLE_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_ID_BYTES: usize = 128;
const MAX_CAPTURE_BYTES: usize = 16 * 1024 * 1024;
const MAX_TIMEOUT_MS: u64 = 24 * 60 * 60 * 1000;
const MAX_BUSY_SPAWN_RETRIES: u8 = 3;
const MAX_POINT_ATTEMPTS: u32 = 4_096;
const MAX_OPTIONAL_METRIC_VALUES_PER_ATTEMPT: u64 = 1_000_000;
const PROVIDER_CATALOG_VERSION: u16 = 1;
const MAX_SELECTED_AGENTS: usize = 9;

/// Run the command-line interface with process standard streams.
#[must_use]
pub fn entry(args: Vec<OsString>) -> ExitCode {
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    ExitCode::from(run(&args, &mut stdout, &mut stderr))
}

/// Execute one CLI request with injected output streams.
pub fn run(args: &[OsString], stdout: &mut dyn Write, stderr: &mut dyn Write) -> u8 {
    match dispatch(args, stdout, stderr, None, None) {
        Ok(exit_code) => exit_code,
        Err(error) => {
            let envelope = ErrorEnvelope {
                schema_version: OUTPUT_SCHEMA_VERSION,
                ok: false,
                command: command_name(args),
                error,
            };
            if write_json(stdout, &envelope).is_err() {
                let _ = writeln!(stderr, "ASB could not write structured error output");
            }
            envelope.error.exit_code
        }
    }
}

/// Execute a strict-replay CLI request with runtime-issued launch authority.
///
/// The ordinary argument-only entry point intentionally cannot execute replay;
/// runtime supervision must inject this opaque, one-shot authority.
pub fn run_with_replay_authority(
    args: &[OsString],
    authority: ReplayLaunchAuthority,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> u8 {
    match dispatch(args, stdout, stderr, Some(authority), None) {
        Ok(exit_code) => exit_code,
        Err(error) => {
            let envelope = ErrorEnvelope {
                schema_version: OUTPUT_SCHEMA_VERSION,
                ok: false,
                command: command_name(args),
                error,
            };
            if write_json(stdout, &envelope).is_err() {
                let _ = writeln!(stderr, "ASB could not write structured error output");
            }
            envelope.error.exit_code
        }
    }
}

/// Execute one live-provider run or sweep with a runtime-issued opaque attempt.
/// The CLI cannot construct this capability or select its endpoint.
pub fn run_with_live_provider_attempt(
    args: &[OsString],
    attempt: LiveProviderAttempt,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> u8 {
    let attempt = Arc::new(Mutex::new(Some(attempt)));
    let factory = LiveProviderAttemptFactory::from_fn(move |_, _| {
        attempt
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
            .ok_or(LaunchAuthorityError::InvalidLaunchInput)
    });
    match dispatch(args, stdout, stderr, None, Some(factory)) {
        Ok(exit_code) => exit_code,
        Err(error) => {
            let envelope = ErrorEnvelope {
                schema_version: OUTPUT_SCHEMA_VERSION,
                ok: false,
                command: command_name(args),
                error,
            };
            let _ = write_json(stdout, &envelope);
            envelope.error.exit_code
        }
    }
}

/// Execute one live-provider run or sweep using a runtime-owned factory that
/// issues one opaque capability for each admitted scheduler attempt.
pub fn run_with_live_provider_factory(
    args: &[OsString],
    factory: LiveProviderAttemptFactory,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> u8 {
    match dispatch(args, stdout, stderr, None, Some(factory)) {
        Ok(exit_code) => exit_code,
        Err(error) => {
            let envelope = ErrorEnvelope {
                schema_version: OUTPUT_SCHEMA_VERSION,
                ok: false,
                command: command_name(args),
                error,
            };
            let _ = write_json(stdout, &envelope);
            envelope.error.exit_code
        }
    }
}

/// Execute a live-provider run or sweep using the runtime-owned scheduler
/// composition boundary. The CLI receives only the opaque per-attempt
/// factory; provider, lease, relay, namespace, credential, and backend
/// authority remain inside `asb-runtime`.
pub fn run_with_runtime_live_provider_scheduler(
    args: &[OsString],
    scheduler: LiveProviderRuntimeScheduler,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> u8 {
    run_with_live_provider_factory(args, scheduler.into_factory(), stdout, stderr)
}

/// Execute a live-provider run or sweep from the runtime-owned authenticated
/// dispatch source. The CLI receives no provider, credential, policy, relay,
/// namespace, tool, or launch authority inputs.
pub fn run_with_runtime_live_provider_source(
    args: &[OsString],
    source: LiveProviderRuntimeDispatchSource,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> u8 {
    run_with_runtime_live_provider_scheduler(args, source.into_scheduler(), stdout, stderr)
}

fn dispatch(
    args: &[OsString],
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
    mut replay_authority: Option<ReplayLaunchAuthority>,
    live_factory: Option<LiveProviderAttemptFactory>,
) -> Result<u8, CliError> {
    let words = unicode_args(args)?;
    match words.as_slice() {
        [] => write_help(stdout).map(|()| 0),
        [word] if matches!(word.as_str(), "--help" | "-h") => write_help(stdout).map(|()| 0),
        [word] if matches!(word.as_str(), "--version" | "-V") => {
            writeln!(stdout, "asb {}", env!("CARGO_PKG_VERSION"))
                .map(|()| 0)
                .map_err(output_error)
        }
        [command] if command == "doctor" => doctor(stdout).map(|()| 0),
        [command] if command == "setup" => setup(&[], stdout).map(|()| 0),
        [command, setup_args @ ..] if command == "setup" => setup(setup_args, stdout).map(|()| 0),
        [command, easy_args @ ..] if command == "easy" => guided_local(easy_args, stdout, stderr),
        [command, tui_args @ ..] if command == "tui" => tui::dispatch(tui_args, stdout),
        [command, format, value]
            if command == "capabilities" && format == "--format" && value == "json" =>
        {
            write_json(stdout, &capabilities::CapabilityResponse::control_v1()).map(|()| 0)
        }
        [command] if command == "provider-catalog" => provider_catalog(stdout).map(|()| 0),
        [command] if command == "workload-catalog" => workload_catalog_output(stdout).map(|()| 0),
        [command, operation] if command == "config" && operation == "openrouter" => {
            configure_openrouter(stdout).map(|()| 0)
        }
        [command, auth_args @ ..] if command == "auth" => auth(auth_args, stdout),
        [command, selection @ ..] if command == "provider-plan" => {
            provider_plan(selection, stdout).map(|()| 0)
        }
        [command, shell] if command == "completion" => completion(shell, stdout).map(|()| 0),
        [command, path] if command == "plan" => plan(Path::new(path), stdout).map(|()| 0),
        [command, path, flag] if command == "plan" && flag == "--use-config" => {
            plan_with_config(Path::new(path), stdout).map(|()| 0)
        }
        [command, path, flag, selection] if command == "plan" && flag == "--provider-selection" => {
            plan_with_selection(Path::new(path), Path::new(selection), stdout).map(|()| 0)
        }
        [command, path] if command == "run" => {
            execute(Path::new(path), false, false, None, stdout, stderr)
        }
        [command, path, flag, selection] if command == "run" && flag == "--provider-selection" => {
            execute_with_selection(
                Path::new(path),
                Path::new(selection),
                false,
                false,
                stdout,
                stderr,
            )
        }
        [command, path, flag] if command == "run" && flag == "--use-config" => {
            execute_with_config(Path::new(path), false, false, stdout, stderr)
        }
        [command, path, flag] if command == "run" && flag == "--live-provider" => {
            execute(Path::new(path), false, true, live_factory, stdout, stderr)
        }
        [command, path] if command == "sweep" => {
            execute(Path::new(path), true, false, None, stdout, stderr)
        }
        [command, path, flag, selection]
            if command == "sweep" && flag == "--provider-selection" =>
        {
            execute_with_selection(
                Path::new(path),
                Path::new(selection),
                true,
                false,
                stdout,
                stderr,
            )
        }
        [command, path, flag] if command == "sweep" && flag == "--use-config" => {
            execute_with_config(Path::new(path), true, false, stdout, stderr)
        }
        [command, path, flag] if command == "sweep" && flag == "--live-provider" => {
            execute(Path::new(path), true, true, live_factory, stdout, stderr)
        }
        [command, path] if command == "serve" => control::serve(Path::new(path)).map(|()| 0),
        [command, runs @ ..] if command == "compare" && runs.len() >= 2 => {
            compare(runs, stdout).map(|()| 0)
        }
        [command, runs @ ..] if command == "report" && !runs.is_empty() => {
            report(runs, stdout).map(|()| 0)
        }
        [command, input, output] if command == "record" => {
            record(Path::new(input), Path::new(output), stdout).map(|()| 0)
        }
        [command, manifest] if command == "record-campaign" => {
            record_campaign(Path::new(manifest), stdout).map(|()| 0)
        }
        [command, cassette, profile, agent] if command == "replay" => replay(
            Path::new(cassette),
            profile,
            agent,
            replay_authority.take(),
            stdout,
        )
        .map(|()| 0),
        _ => Err(CliError::usage("unsupported arguments; use asb --help")),
    }
}

fn unicode_args(args: &[OsString]) -> Result<Vec<String>, CliError> {
    args.iter()
        .map(|arg| {
            arg.clone()
                .into_string()
                .map_err(|_| CliError::usage("arguments must be valid UTF-8"))
        })
        .collect()
}

/// Run the catalog/config-driven local qualification fixture through the
/// ordinary run/sweep dispatcher. This command deliberately has no live or
/// endpoint option: the explicit `--local-mock --use-config` pair is the
/// complete authority surface.
fn guided_local(
    args: &[String],
    output: &mut dyn Write,
    progress: &mut dyn Write,
) -> Result<u8, CliError> {
    if args.len() != 4 {
        return Err(CliError::usage(
            "easy requires run|sweep PATH --use-config --local-mock",
        ));
    }
    if args[1].is_empty() || !args[2..].iter().any(|flag| flag == "--use-config") {
        return Err(CliError::validation(
            "easy requires an experiment path and --use-config",
        ));
    }
    if !args[2..].iter().any(|flag| flag == "--local-mock")
        || args[2..]
            .iter()
            .any(|flag| !matches!(flag.as_str(), "--use-config" | "--local-mock"))
    {
        return Err(CliError::usage(
            "easy accepts only --use-config and --local-mock",
        ));
    }
    let store = ConfigStore::from_environment()
        .map_err(|_| CliError::operation("ASB configuration location is unavailable"))?;
    guided_local_at(args, &store, output, progress)
}

fn guided_local_at(
    args: &[String],
    store: &ConfigStore,
    output: &mut dyn Write,
    progress: &mut dyn Write,
) -> Result<u8, CliError> {
    let sweep = match args[0].as_str() {
        "run" => false,
        "sweep" => true,
        _ => return Err(CliError::usage("easy requires run or sweep")),
    };
    execute_inner_from_source(
        Path::new(&args[1]),
        SelectionSource::Config(&store),
        sweep,
        false,
        None,
        true,
        output,
        progress,
    )
}

/// Emit a bounded, credential-free authenticated control request.
fn auth(args: &[String], stdout: &mut dyn Write) -> Result<u8, CliError> {
    let usage =
        || CliError::usage("auth requires enroll|status|rotate|revoke and named digest options");
    let operation = args.first().ok_or_else(usage)?;
    let value = |name: &str| -> Result<String, CliError> {
        args.windows(2)
            .find(|pair| pair[0] == name)
            .map(|pair| pair[1].clone())
            .ok_or_else(|| CliError::usage("auth missing required option"))
    };
    let provider = value("--provider")?;
    let call = match operation.as_str() {
        "enroll" => asb_control::ControlCall::AuthEnroll(asb_control::AuthEnrollParams {
            provider,
            endpoint_identity_sha256: value("--endpoint-digest")?,
            credential_locator_sha256: value("--credential-digest")?,
            idempotency_key: value("--idempotency-key")?,
        }),
        "status" => {
            asb_control::ControlCall::AuthStatus(asb_control::AuthStatusParams { provider })
        }
        "rotate" => asb_control::ControlCall::AuthRotate(asb_control::AuthRotateParams {
            provider,
            credential_locator_sha256: value("--credential-digest")?,
            idempotency_key: value("--idempotency-key")?,
        }),
        "revoke" => asb_control::ControlCall::AuthRevoke(asb_control::AuthRevokeParams {
            provider,
            idempotency_key: value("--idempotency-key")?,
        }),
        _ => return Err(usage()),
    };
    let request = asb_control::ControlRequest {
        jsonrpc: "2.0".to_owned(),
        id: asb_control::RequestId(0),
        timeout_ms: 300_000,
        call,
    };
    if let Some(socket) = args
        .windows(2)
        .find(|pair| pair[0] == "--socket")
        .map(|pair| pair[1].as_str())
    {
        let mut client = asb_control::ControlClient::connect_with_versions(
            Path::new(socket),
            asb_control::ControlLimits::default(),
            asb_control::SUPPORTED_CONTROL_VERSIONS,
        )
        .map_err(|_| CliError::operation("auth control service connection failed"))?;
        let timeout_ms = client.negotiated().limits.max_timeout_ms;
        let response = client
            .call(request.call, timeout_ms)
            .map_err(|_| CliError::operation("auth control service request failed"))?;
        return write_json(stdout, &response).map(|()| 0);
    }
    write_json(stdout, &request).map(|()| 0)
}

fn command_name(args: &[OsString]) -> &'static str {
    match args.first().and_then(|value| value.to_str()) {
        Some("doctor") => "doctor",
        Some("setup") => "setup",
        Some("easy") => "easy",
        Some("tui") => "tui",
        Some("capabilities") => "capabilities",
        Some("provider-catalog") => "provider-catalog",
        Some("workload-catalog") => "workload-catalog",
        Some("config") => "config",
        Some("auth") => "auth",
        Some("provider-plan") => "provider-plan",
        Some("completion") => "completion",
        Some("plan") => "plan",
        Some("run") => "run",
        Some("sweep") => "sweep",
        Some("compare") => "compare",
        Some("report") => "report",
        Some("serve") => "serve",
        Some("record") => "record",
        Some("replay") => "replay",
        _ => "cli",
    }
}

fn write_help(output: &mut dyn Write) -> Result<(), CliError> {
    writeln!(
        output,
        "Agent Systems Benchmark (ASB)\n\nUsage:\n  asb doctor\n  asb setup [--format=json]\n  asb easy run|sweep EXPERIMENT.toml --use-config --local-mock\n  asb capabilities --format json\n  asb tui [launch]\n  asb tui install [--offline] [--dry-run] [--launch]\n  asb tui upgrade [--offline] [--dry-run] [--launch]\n  asb tui status|doctor|remove\n  asb tui --version\n  asb provider-catalog\n  asb provider-plan --catalog-sha256 SHA256 --provider-profile openai|openrouter --agent AGENT --agent AGENT --credential-reference-sha256 SHA256 > selection.json\n  asb plan EXPERIMENT.toml --provider-selection selection.json\n  asb run EXPERIMENT.toml --provider-selection selection.json\n  asb sweep EXPERIMENT.toml --provider-selection selection.json\n  asb compare RUN...\n  asb report RUN...\n  asb completion bash\n  asb serve CONTROL.toml\n\nStructured command results are JSON on stdout; progress is on stderr.\nThe optional frontend is independently verified and installed under rootless XDG state; ASB contains no frontend rendering code. The capability probe is deterministic and side-effect-free. Provider planning is a side-effect-free dry run and never launches an agent or contacts a provider. The saved selection is content-pinned and must match the experiment agent, provider, model, and additional-settings identity."
    )
    .map_err(output_error)?;
    writeln!(output, "  asb record CAPTURE.json CASSETTE.json\n  asb record-campaign MANIFEST.json\n  asb replay CASSETTE.json PROVIDER_PROFILE_SHA256 AGENT")
        .map_err(output_error)
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct SetupOutput {
    schema_version: u16,
    ok: bool,
    command: &'static str,
    mode: &'static str,
    steps: &'static [&'static str],
    persistent_change: bool,
    provider_contact: bool,
    provider_profile: Option<String>,
    model: Option<String>,
}

/// Emit a side-effect-free setup checklist. Interactive mutation is a later
/// phase; this contract gives scripts a stable, explicit preflight surface.
fn setup(args: &[String], output: &mut dyn Write) -> Result<(), CliError> {
    let mut provider_profile = None;
    let mut model = None;
    let mut output_path = None;
    let mut index = 0;
    while index < args.len() {
        let value = args[index].as_str();
        let target = match value {
            "--provider-profile" => &mut provider_profile,
            "--model" => &mut model,
            "--output" => &mut output_path,
            "--format=json" => {
                index += 1;
                continue;
            }
            _ => {
                return Err(CliError::usage(
                    "setup accepts --provider-profile, --model, --output, and --format=json",
                ));
            }
        };
        index += 1;
        let argument = args
            .get(index)
            .ok_or_else(|| CliError::usage("setup option is missing a value"))?;
        if argument.is_empty() || argument.len() > MAX_ID_BYTES {
            return Err(CliError::validation(
                "setup option value is empty or too long",
            ));
        }
        *target = Some(argument.clone());
        index += 1;
    }
    if provider_profile.is_some() != model.is_some() {
        return Err(CliError::validation(
            "provider profile and model must be selected together",
        ));
    }
    if let (Some(profile), Some(model_name)) = (&provider_profile, &model) {
        let compatible = match profile.as_str() {
            "openai" => model_name.starts_with("gpt-") || model_name.starts_with("o1"),
            "gemini" => model_name.starts_with("gemini-"),
            "ollama" => ["llama", "mistral", "qwen", "phi"]
                .iter()
                .any(|prefix| model_name.starts_with(prefix)),
            _ => false,
        };
        if !compatible {
            return Err(CliError::validation(
                "provider profile and model are incompatible or unsupported",
            ));
        }
    }
    let contract = SetupOutput {
        schema_version: OUTPUT_SCHEMA_VERSION,
        ok: true,
        command: "setup",
        mode: if output_path.is_some() {
            "commit"
        } else {
            "preflight"
        },
        steps: &[
            "detect-installed-agents",
            "select-provider-and-model",
            "configure-authentication",
            "confirm-persistence",
        ],
        persistent_change: output_path.is_some(),
        provider_contact: false,
        provider_profile,
        model,
    };
    if let Some(path) = output_path {
        let encoded = serde_json::to_vec(&contract)
            .map_err(|_| CliError::operation("setup configuration cannot be encoded"))?;
        write_atomic_private(Path::new(&path), &encoded)?;
    }
    write_json(output, &contract)
}

fn completion(shell: &str, output: &mut dyn Write) -> Result<(), CliError> {
    if shell != "bash" {
        return Err(CliError::usage("only bash completion is supported"));
    }
    writeln!(
        output,
        "complete -W 'doctor setup capabilities provider-catalog workload-catalog provider-plan plan run sweep compare report record record-campaign replay completion serve tui easy --help --version' asb"
    )
    .map_err(output_error)
}

#[derive(Serialize)]
struct ReplayWorkflowOutput {
    schema_version: u16,
    ok: bool,
    command: &'static str,
    source: ExecutionSource,
    network: &'static str,
    /// Replay is deterministic evidence, not a fresh model-quality measurement.
    fresh_model_quality: bool,
    provider_profile_sha256: String,
    agent_id: String,
    cassette_sha256: String,
    response_status: u16,
    response_sha256: String,
}

fn record(input: &Path, output: &Path, stdout: &mut dyn Write) -> Result<(), CliError> {
    let bytes = read_bounded_json(input, MAX_CAPTURE_BYTES, "recording capture")?;
    let capture: RecordingCapture = serde_json::from_slice(&bytes)
        .map_err(|_| CliError::validation("recording capture syntax or shape is invalid"))?;
    let artifact =
        seal_recording(capture, Default::default(), CassetteLimits::default()).map_err(|_| {
            CliError::validation("recording capture is incomplete, unsafe, or unapproved")
        })?;
    let encoded = serde_json::to_vec(&artifact.cassette)
        .map_err(|_| CliError::operation("recording cassette cannot be encoded"))?;
    if encoded.len() > MAX_CAPTURE_BYTES {
        return Err(CliError::validation(
            "recording cassette exceeds its byte limit",
        ));
    }
    write_atomic_private(output, &encoded)?;
    write_json(stdout, &artifact.metadata)
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordingCampaignManifest {
    schema_version: u16,
    provider_profile_sha256: String,
    agent_ids: Vec<String>,
    workload_ids: Vec<String>,
    entries: Vec<RecordingCampaignEntry>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordingCampaignEntry {
    workload_id: String,
    capture_path: PathBuf,
    cassette_path: PathBuf,
}

#[derive(Serialize)]
struct RecordingCampaignOutput {
    schema_version: u16,
    ok: bool,
    command: &'static str,
    campaign_id: String,
    tuple_count: u16,
    complete_coverage: bool,
    offline_ready: bool,
    unavailable_reason: Option<&'static str>,
    recordings: Vec<asb_replay::RecordingMetadata>,
}

fn record_campaign(input: &Path, stdout: &mut dyn Write) -> Result<(), CliError> {
    let bytes = read_bounded_json(input, MAX_CAPTURE_BYTES, "recording campaign manifest")?;
    let manifest: RecordingCampaignManifest = serde_json::from_slice(&bytes)
        .map_err(|_| CliError::validation("recording campaign manifest is invalid"))?;
    if manifest.schema_version != asb_replay::RECORDING_WORKFLOW_SCHEMA_VERSION
        || manifest.agent_ids.is_empty()
        || manifest.workload_ids.is_empty()
        || manifest.agent_ids.len() > MAX_SELECTED_AGENTS
        || manifest.workload_ids.len() > MAX_SELECTED_AGENTS
        || manifest.entries.len() > asb_replay::MAX_RECORDING_CAMPAIGN_TUPLES
    {
        return Err(CliError::validation(
            "recording campaign bounds are invalid",
        ));
    }
    if !valid_sha256(&manifest.provider_profile_sha256)
        || manifest.agent_ids.windows(2).any(|pair| pair[0] >= pair[1])
        || manifest
            .workload_ids
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
        || manifest
            .agent_ids
            .iter()
            .any(|agent| parse_agent(agent).is_err())
    {
        return Err(CliError::validation(
            "recording campaign identities are invalid",
        ));
    }
    let workloads = manifest
        .workload_ids
        .iter()
        .map(|id| {
            describe_workload(id)
                .map(|workload| (id.clone(), workload.scoring_version))
                .map_err(|_| CliError::validation("recording campaign workload is unavailable"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let campaign = asb_replay::RecordingCampaign {
        schema_version: manifest.schema_version,
        provider_profile_sha256: manifest.provider_profile_sha256.clone(),
        agent_ids: manifest.agent_ids.clone(),
        workloads,
        max_tuples: u16::try_from(asb_replay::MAX_RECORDING_CAMPAIGN_TUPLES).unwrap_or(u16::MAX),
        cost_per_tuple_minor: 0,
    };
    let tuples = campaign
        .expand()
        .map_err(|_| CliError::validation("recording campaign matrix is invalid"))?;
    let expected = tuples
        .iter()
        .map(|tuple| (tuple.agent_id.clone(), tuple.workload_id.clone()))
        .collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    let mut recordings = Vec::with_capacity(manifest.entries.len());
    for entry in manifest.entries {
        if !manifest.workload_ids.contains(&entry.workload_id) {
            return Err(CliError::validation(
                "recording campaign entry workload is not selected",
            ));
        }
        let capture_bytes = read_bounded_json(
            &entry.capture_path,
            MAX_CAPTURE_BYTES,
            "recording campaign capture",
        )?;
        let capture: RecordingCapture = serde_json::from_slice(&capture_bytes)
            .map_err(|_| CliError::validation("recording campaign capture is invalid"))?;
        if capture.provider_profile_sha256 != manifest.provider_profile_sha256
            || !manifest.agent_ids.contains(&capture.agent_id)
            || !seen.insert((capture.agent_id.clone(), entry.workload_id.clone()))
        {
            return Err(CliError::validation(
                "recording campaign coverage is duplicate or mismatched",
            ));
        }
        let artifact = seal_recording(capture, Default::default(), CassetteLimits::default())
            .map_err(|_| CliError::validation("recording campaign capture cannot be sealed"))?;
        let encoded = serde_json::to_vec(&artifact.cassette)
            .map_err(|_| CliError::operation("recording campaign cassette cannot be encoded"))?;
        if encoded.len() > MAX_CAPTURE_BYTES {
            return Err(CliError::validation(
                "recording campaign cassette is too large",
            ));
        }
        write_atomic_private(&entry.cassette_path, &encoded)?;
        recordings.push(artifact.metadata);
    }
    let complete = seen == expected;
    let campaign_id = format!(
        "campaign-{}",
        &format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&campaign).unwrap_or_default())
        )[..24]
    );
    write_json(
        stdout,
        &RecordingCampaignOutput {
            schema_version: OUTPUT_SCHEMA_VERSION,
            ok: complete,
            command: "record-campaign",
            campaign_id,
            tuple_count: u16::try_from(expected.len()).unwrap_or(u16::MAX),
            complete_coverage: complete,
            offline_ready: complete,
            unavailable_reason: (!complete).then_some("recording-coverage-incomplete"),
            recordings,
        },
    )
}

fn replay(
    cassette_path: &Path,
    provider_profile_sha256: &str,
    agent_id: &str,
    authority: Option<ReplayLaunchAuthority>,
    stdout: &mut dyn Write,
) -> Result<(), CliError> {
    let bytes = read_bounded_json(cassette_path, MAX_CAPTURE_BYTES, "recording cassette")?;
    let cassette = asb_replay::decode_cassette(&bytes, CassetteLimits::default())
        .map_err(|_| CliError::validation("recording cassette is corrupt or incomplete"))?;
    let mut runtime_context = authority
        .ok_or_else(|| CliError::validation("runtime replay authority is required"))?
        .consume_for(&cassette.integrity.digest)
        .map_err(|_| CliError::validation("runtime replay authority does not match cassette"))?;
    let descriptor = RecordingDescriptor {
        provider_profile_sha256: provider_profile_sha256.to_owned(),
        agent_id: agent_id.to_owned(),
        cassette_sha256: cassette.integrity.digest.clone(),
    };
    let mut index = RecordingIndex::new();
    index.insert(descriptor, &cassette).map_err(|_| {
        CliError::validation("recording cassette is not compatible with this selection")
    })?;
    let source = asb_replay::choose_source(
        &index,
        provider_profile_sha256,
        agent_id,
        false,
        Some(&SourceChoice::Replay {
            cassette_sha256: cassette.integrity.digest.clone(),
        }),
    )
    .map_err(|_| CliError::validation("recording cassette is not an exact compatible replay"))?;
    let interaction = cassette
        .contents
        .interactions
        .first()
        .ok_or_else(|| CliError::validation("recording cassette has no replay interaction"))?;
    let route = asb_replay::ReplayRoute {
        session_id: interaction.session_id.clone(),
        attempt_id: interaction.attempt_id.clone(),
        dialect: interaction.dialect,
    };
    let request = asb_replay::ReplayHttpRequest {
        method: interaction.request.method.clone(),
        path: interaction.request.path.clone(),
        headers: interaction.request.headers.clone(),
        body: asb_replay::canonical_json_bytes(&interaction.request.body)
            .map_err(|_| CliError::validation("recording request body is not canonical"))?,
    };
    let encoded_request =
        asb_replay::encode_dispatch_request(&asb_replay::ReplayDispatchRequest { route, request })
            .map_err(|_| CliError::validation("replay operation request is not bounded"))?;
    let service =
        asb_replay::StrictReplayService::new(cassette.clone(), asb_replay::ReplayLimits::default())
            .map_err(|_| {
                CliError::validation("recording cassette cannot initialize strict replay")
            })?;
    let mut operation = runtime_context
        .issue_operation()
        .map_err(|_| CliError::operation("runtime replay operation could not be issued"))?;
    let encoded_response = operation
        .dispatch(
            "replay-operation-1".into(),
            encoded_request,
            move |payload| {
                let request = asb_replay::decode_dispatch_request(&payload)
                    .map_err(|_| asb_runtime::ReplayTransportError::InvalidEnvelope)?;
                let response = service
                    .handle(&request.route, request.request)
                    .map_err(|_| asb_runtime::ReplayTransportError::InvalidEnvelope)?;
                asb_replay::encode_dispatch_response(&response)
                    .map_err(|_| asb_runtime::ReplayTransportError::InvalidEnvelope)
            },
        )
        .map_err(|_| CliError::operation("strict replay operation failed; no provider fallback"))?;
    let response = asb_replay::decode_dispatch_response(&encoded_response)
        .map_err(|_| CliError::operation("strict replay response was malformed"))?;
    let response_sha256 = response
        .segments
        .iter()
        .fold(Sha256::new(), |mut digest, segment| {
            digest.update(segment);
            digest
        })
        .finalize();
    let mut child = runtime_context
        .spawn_owned()
        .map_err(|_| CliError::validation("runtime replay child could not be supervised"))?;
    let output = child
        .wait()
        .map_err(|_| CliError::validation("runtime replay child did not terminate cleanly"))?;
    if output.exit_code != Some(0) {
        return Err(CliError::validation("runtime replay child failed closed"));
    }
    write_json(
        stdout,
        &ReplayWorkflowOutput {
            schema_version: asb_replay::RECORDING_WORKFLOW_SCHEMA_VERSION,
            ok: true,
            command: "replay",
            source,
            network: "denied",
            fresh_model_quality: false,
            provider_profile_sha256: provider_profile_sha256.to_owned(),
            agent_id: agent_id.to_owned(),
            cassette_sha256: cassette.integrity.digest,
            response_status: response.status,
            response_sha256: format!("{response_sha256:x}"),
        },
    )
}

fn read_bounded_json(
    path: &Path,
    maximum: usize,
    label: &'static str,
) -> Result<Vec<u8>, CliError> {
    // Open and validate the same descriptor that is read.  A separate
    // symlink_metadata/metadata check leaves a replacement window in which a
    // validated path can be swapped before File::open; O_NOFOLLOW makes the
    // final component fail closed instead.
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| CliError::validation(label))?;
    let metadata = file.metadata().map_err(|_| CliError::validation(label))?;
    if !metadata.is_file() || metadata.len() > maximum as u64 {
        return Err(CliError::validation(label));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take((maximum as u64).saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| CliError::validation(label))?;
    if bytes.len() > maximum {
        return Err(CliError::validation(label));
    }
    Ok(bytes)
}

fn write_atomic_private(path: &Path, bytes: &[u8]) -> Result<(), CliError> {
    if let Ok(metadata) = fs::symlink_metadata(path)
        && metadata.file_type().is_symlink()
    {
        return Err(CliError::validation(
            "workflow output cannot replace a symlink",
        ));
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let temporary = parent.join(format!(".asb-record-{}.tmp", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .map_err(|_| CliError::operation("workflow output cannot be staged"))?;
    if file.write_all(bytes).is_err() || file.sync_all().is_err() {
        let _ = fs::remove_file(&temporary);
        return Err(CliError::operation("workflow output cannot be written"));
    }
    drop(file);
    fs::rename(&temporary, path).map_err(|_| {
        let _ = fs::remove_file(&temporary);
        CliError::operation("workflow output cannot be installed")
    })
}

#[derive(Serialize)]
struct DoctorOutput<'a> {
    schema_version: u16,
    ok: bool,
    command: &'static str,
    linux: bool,
    architecture: &'a str,
    procfs: bool,
    cgroup_v2: bool,
    interactive_stderr: bool,
    commands: &'a [&'a str],
    batch_agent_boundary: &'static str,
    workloads: &'static [&'static str],
}

fn doctor(output: &mut dyn Write) -> Result<(), CliError> {
    write_json(
        output,
        &DoctorOutput {
            schema_version: OUTPUT_SCHEMA_VERSION,
            ok: cfg!(target_os = "linux"),
            command: "doctor",
            linux: cfg!(target_os = "linux"),
            architecture: std::env::consts::ARCH,
            procfs: Path::new("/proc/self/stat").is_file(),
            cgroup_v2: Path::new("/sys/fs/cgroup/cgroup.controllers").is_file(),
            interactive_stderr: io::stderr().is_terminal(),
            commands: &[
                "doctor",
                "setup",
                "capabilities",
                "provider-catalog",
                "provider-plan",
                "plan",
                "run",
                "sweep",
                "compare",
                "report",
                "record",
                "replay",
                "completion",
                "serve",
            ],
            batch_agent_boundary: "batch-stdio-v1",
            workloads: OriginalWorkloads::fixture_ids(),
        },
    )
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct ProviderCatalogOutput {
    schema_version: u16,
    ok: bool,
    command: &'static str,
    catalog_version: u16,
    catalog_sha256: String,
    agents: &'static [&'static str],
    profiles: [ProviderCatalogEntry; 3],
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct ProviderCatalogEntry {
    id: &'static str,
    model: &'static str,
    credential_source: &'static str,
    selectable: bool,
    unavailable_reason: Option<&'static str>,
}

const AGENT_IDS: [&str; 9] = [
    "opencode",
    "opendesk",
    "aider",
    "codex",
    "gemini",
    "qwen_code",
    "goose",
    "mini_swe",
    "openhands",
];

fn provider_catalog_digest() -> String {
    let mut digest = Sha256::new();
    digest.update(b"asb-cli-provider-catalog-v1\0");
    for agent in AGENT_IDS {
        digest.update(agent.as_bytes());
        digest.update([0]);
    }
    digest.update(b"openai\0");
    digest.update(asb_agents::openai::OPENAI_MODEL.as_bytes());
    digest.update(b"\0environment\0selectable\0openrouter\0");
    digest.update(asb_agents::openrouter::OPENROUTER_MODEL.as_bytes());
    digest.update(b"\0environment\0selectable\0ollama\0");
    digest.update(asb_agents::ollama::OLLAMA_MODEL.as_bytes());
    digest.update(b"\0none\0requires-verified-daemon\0");
    format!("{:x}", digest.finalize())
}

fn workload_catalog_output(output: &mut dyn Write) -> Result<(), CliError> {
    let entries = workload_catalog()
        .into_iter()
        .map(|entry| {
            json!({
                "id": entry.id,
                "kind": match entry.kind {
                    CatalogKind::Builtin => "builtin",
                    CatalogKind::Literature => "literature",
                    CatalogKind::Methodology => "methodology",
                },
                "source": entry.source,
                "source_revision": entry.source_revision,
                "license": entry.license,
                "evaluator": entry.evaluator,
                "capability_tags": entry.capability_tags,
                "adaptation": entry.adaptation,
                "platform": entry.platform,
                "availability": entry.availability,
                "evidence": entry.evidence,
            })
        })
        .collect::<Vec<_>>();
    write_json(
        output,
        &json!({
            "schema_version": OUTPUT_SCHEMA_VERSION,
            "ok": true,
            "command": "workload-catalog",
            "entries": entries,
        }),
    )
}

fn provider_catalog(output: &mut dyn Write) -> Result<(), CliError> {
    write_json(
        output,
        &ProviderCatalogOutput {
            schema_version: OUTPUT_SCHEMA_VERSION,
            ok: true,
            command: "provider-catalog",
            catalog_version: PROVIDER_CATALOG_VERSION,
            catalog_sha256: provider_catalog_digest(),
            agents: &AGENT_IDS,
            profiles: [
                ProviderCatalogEntry {
                    id: "openai",
                    model: asb_agents::openai::OPENAI_MODEL,
                    credential_source: "environment",
                    selectable: true,
                    unavailable_reason: None,
                },
                ProviderCatalogEntry {
                    id: "openrouter",
                    model: asb_agents::openrouter::OPENROUTER_MODEL,
                    credential_source: "environment",
                    selectable: true,
                    unavailable_reason: None,
                },
                ProviderCatalogEntry {
                    id: "ollama",
                    model: asb_agents::ollama::OLLAMA_MODEL,
                    credential_source: "none",
                    selectable: false,
                    unavailable_reason: Some("verified local daemon evidence is unavailable"),
                },
            ],
        },
    )
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct ConfigOutput {
    schema_version: u16,
    ok: bool,
    command: &'static str,
    provider: &'static str,
    model: &'static str,
    model_snapshot: &'static str,
    credential_environment: &'static str,
    credential_reference_sha256: String,
    generation: u64,
}

/// Persist the credential-free, pinned OpenRouter user configuration.
fn configure_openrouter(output: &mut dyn Write) -> Result<(), CliError> {
    let store = ConfigStore::from_environment()
        .map_err(|_| CliError::operation("ASB configuration location is unavailable"))?;
    configure_openrouter_at(&store, output)
}

fn configure_openrouter_at(store: &ConfigStore, output: &mut dyn Write) -> Result<(), CliError> {
    let mut config = store
        .load()
        .map_err(|_| CliError::validation("ASB configuration is unavailable or malformed"))?
        .unwrap_or_else(Configuration::empty);
    let credential_reference = asb_agents::openrouter::openrouter_credential_reference()
        .map_err(|_| CliError::operation("OpenRouter credential reference cannot be prepared"))?;
    let profile = OpenRouterProfile::new(&credential_reference)
        .map_err(|_| CliError::operation("OpenRouter profile cannot be prepared"))?;
    let generation = config
        .openrouter_free_model
        .as_ref()
        .map_or(0, |value| value.enrollment.generation)
        .checked_add(1)
        .ok_or_else(|| CliError::validation("OpenRouter configuration generation overflow"))?;
    let selection = OpenRouterFreeModelConfig::enroll(
        asb_agents::openrouter::OPENROUTER_MODEL.to_owned(),
        asb_agents::openrouter::OPENROUTER_MODEL_SNAPSHOT_DATE.to_owned(),
        profile.provider_profile().endpoint.identity_sha256.clone(),
        generation,
    )
    .map_err(|_| CliError::operation("OpenRouter configuration cannot be validated"))?;
    let result = ConfigOutput {
        schema_version: asb_config::CONFIG_SCHEMA_VERSION,
        ok: true,
        command: "config-openrouter",
        provider: "openrouter",
        model: asb_agents::openrouter::OPENROUTER_MODEL,
        model_snapshot: asb_agents::openrouter::OPENROUTER_MODEL_SNAPSHOT,
        credential_environment: asb_agents::openrouter::OPENROUTER_API_KEY_ENV,
        credential_reference_sha256: selection.credential.locator_sha256.clone(),
        generation: selection.enrollment.generation,
    };
    config.openrouter_free_model = Some(selection);
    store
        .save(&config)
        .map_err(|_| CliError::operation("OpenRouter configuration cannot be persisted"))?;
    write_json(output, &result)
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ProviderPlanOutput {
    schema_version: u16,
    ok: bool,
    command: String,
    dry_run: bool,
    catalog_sha256: String,
    selection_sha256: String,
    provider_profile: String,
    provider_profile_sha256: String,
    model: String,
    credential_source: String,
    credential_reference_sha256: String,
    effective: Vec<EffectiveCliAgent>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct EffectiveCliAgent {
    agent: String,
    profile_sha256: String,
    api_mode: EffectiveApiMode,
}

fn provider_plan(args: &[String], output: &mut dyn Write) -> Result<(), CliError> {
    let store = ConfigStore::from_environment()
        .map_err(|_| CliError::operation("ASB configuration location is unavailable"))?;
    provider_plan_at(args, output, &store)
}

fn provider_plan_at(
    args: &[String],
    output: &mut dyn Write,
    store: &ConfigStore,
) -> Result<(), CliError> {
    let mut catalog_sha256 = None;
    let mut provider = None;
    let mut credential_reference_sha256 = None;
    let mut use_config = false;
    let mut agents = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        if flag == "--use-config" {
            if use_config {
                return Err(CliError::usage(
                    "provider-plan option was supplied more than once",
                ));
            }
            use_config = true;
            index += 1;
            continue;
        }
        let value = args
            .get(index + 1)
            .ok_or_else(|| CliError::usage("provider-plan options require values"))?;
        match flag {
            "--catalog-sha256" if catalog_sha256.replace(value.as_str()).is_none() => {}
            "--provider-profile" if provider.replace(value.as_str()).is_none() => {}
            "--credential-reference-sha256"
                if credential_reference_sha256
                    .replace(value.as_str())
                    .is_none() => {}
            "--agent" if agents.len() < MAX_SELECTED_AGENTS => agents.push(parse_agent(value)?),
            "--catalog-sha256" | "--provider-profile" | "--credential-reference-sha256" => {
                return Err(CliError::usage(
                    "provider-plan option was supplied more than once",
                ));
            }
            "--agent" => {
                return Err(CliError::validation("selected agent set exceeds its bound"));
            }
            _ => return Err(CliError::usage("unsupported provider-plan option")),
        }
        index += 2;
    }
    let expected_catalog = provider_catalog_digest();
    if catalog_sha256 != Some(expected_catalog.as_str()) {
        return Err(CliError::validation(
            "provider catalog identity is stale or absent",
        ));
    }
    let config = use_config
        .then(|| load_openrouter_config_at(store))
        .transpose()?;
    let provider = provider
        .or_else(|| config.as_ref().map(|_| "openrouter"))
        .ok_or_else(|| CliError::validation("provider profile is absent"))?;
    if use_config && provider != "openrouter" {
        return Err(CliError::validation(
            "--use-config only supports openrouter",
        ));
    }
    if let Some(config) = &config {
        if let Some(requested) = credential_reference_sha256
            && requested != config.credential.locator_sha256
        {
            return Err(CliError::validation(
                "configured credential reference does not match",
            ));
        }
        credential_reference_sha256 = Some(config.credential.locator_sha256.as_str());
    }
    if provider == "ollama" {
        return Err(CliError::validation(
            "provider profile is advertised but unavailable without verified daemon evidence",
        ));
    }
    let (provider_kind, plan, model) =
        provider_plan_for(provider, agents, credential_reference_sha256)?;
    let effective = plan
        .effective()
        .iter()
        .map(|item| EffectiveCliAgent {
            agent: agent_id(item.agent).to_owned(),
            profile_sha256: item.profile_sha256.clone(),
            api_mode: item.api_mode,
        })
        .collect::<Vec<_>>();
    let canonical_selection = AllAgentsProviderSelection {
        schema_version: ALL_AGENTS_PROVIDER_SELECTION_V1,
        provider: provider_kind,
        agents: effective
            .iter()
            .map(|item| parse_agent(&item.agent))
            .collect::<Result<Vec<_>, _>>()?,
    };
    let selection_sha256 =
        provider_selection_digest(&expected_catalog, &canonical_selection, &effective)?;
    write_json(
        output,
        &ProviderPlanOutput {
            schema_version: OUTPUT_SCHEMA_VERSION,
            ok: true,
            command: "provider-plan".to_owned(),
            dry_run: true,
            catalog_sha256: expected_catalog,
            selection_sha256,
            provider_profile: provider.to_owned(),
            provider_profile_sha256: plan.profile_sha256().to_owned(),
            model: model.to_owned(),
            credential_source: "environment".to_owned(),
            credential_reference_sha256: credential_reference_sha256
                .expect("provider profile resolution validated the credential reference identity")
                .to_owned(),
            effective,
        },
    )
}

fn load_openrouter_config_at(store: &ConfigStore) -> Result<OpenRouterFreeModelConfig, CliError> {
    let config = store
        .load()
        .map_err(|_| CliError::validation("ASB configuration is unavailable or malformed"))?
        .ok_or_else(|| CliError::validation("OpenRouter configuration is absent"))?;
    let selection = config
        .openrouter_free_model
        .ok_or_else(|| CliError::validation("OpenRouter configuration is absent"))?;
    let profile = OpenRouterProfile::new(selection.credential.locator_sha256.clone())
        .map_err(|_| CliError::validation("OpenRouter credential reference is invalid"))?;
    if selection.model != asb_agents::openrouter::OPENROUTER_MODEL
        || selection.model_snapshot != asb_agents::openrouter::OPENROUTER_MODEL_SNAPSHOT
        || selection.model_snapshot_date != asb_agents::openrouter::OPENROUTER_MODEL_SNAPSHOT_DATE
        || selection.endpoint_identity_sha256 != profile.provider_profile().endpoint.identity_sha256
    {
        return Err(CliError::validation(
            "OpenRouter configuration identity is stale",
        ));
    }
    Ok(selection)
}

fn provider_plan_from_selection(
    config: &OpenRouterFreeModelConfig,
    agent: &str,
) -> Result<ProviderPlanOutput, CliError> {
    let selected = vec![parse_agent(agent)?];
    let credential = config.credential.locator_sha256.as_str();
    let (provider_kind, plan, model) = provider_plan_for("openrouter", selected, Some(credential))?;
    let effective = plan
        .effective()
        .iter()
        .map(|item| EffectiveCliAgent {
            agent: agent_id(item.agent).to_owned(),
            profile_sha256: item.profile_sha256.clone(),
            api_mode: item.api_mode,
        })
        .collect::<Vec<_>>();
    let selection = AllAgentsProviderSelection {
        schema_version: ALL_AGENTS_PROVIDER_SELECTION_V1,
        provider: provider_kind,
        agents: effective
            .iter()
            .map(|item| parse_agent(&item.agent))
            .collect::<Result<Vec<_>, _>>()?,
    };
    let catalog = provider_catalog_digest();
    let selection_sha256 = provider_selection_digest(&catalog, &selection, &effective)?;
    let output = ProviderPlanOutput {
        schema_version: OUTPUT_SCHEMA_VERSION,
        ok: true,
        command: "provider-plan".into(),
        dry_run: true,
        catalog_sha256: catalog,
        selection_sha256,
        provider_profile: "openrouter".into(),
        provider_profile_sha256: plan.profile_sha256().into(),
        model: model.into(),
        credential_source: "environment".into(),
        credential_reference_sha256: credential.into(),
        effective,
    };
    validate_provider_selection(&output)?;
    Ok(output)
}

fn provider_plan_for(
    provider: &str,
    agents: Vec<SelectedAgent>,
    credential_reference_sha256: Option<&str>,
) -> Result<(AllAgentsProviderKind, AllAgentsProviderPlan, &'static str), CliError> {
    let provider_kind = match provider {
        "openai" => AllAgentsProviderKind::OpenAi,
        "openrouter" => AllAgentsProviderKind::OpenRouter,
        _ => return Err(CliError::validation("unknown provider profile")),
    };
    let credential_reference_sha256 = credential_reference_sha256
        .ok_or_else(|| CliError::validation("credential reference identity is absent"))?;
    let selection = AllAgentsProviderSelection {
        schema_version: ALL_AGENTS_PROVIDER_SELECTION_V1,
        provider: provider_kind,
        agents,
    };
    let (plan, model) = match selection.provider {
        AllAgentsProviderKind::OpenAi => {
            let profile = OpenAiProfile::new(credential_reference_sha256)
                .map_err(|_| CliError::validation("credential reference identity is invalid"))?;
            let plan = resolve_openai_selection(&selection, &profile).map_err(|_| {
                CliError::validation("provider profile is incompatible with selected agents")
            })?;
            (plan, asb_agents::openai::OPENAI_MODEL)
        }
        AllAgentsProviderKind::OpenRouter => {
            let profile = OpenRouterProfile::new(credential_reference_sha256)
                .map_err(|_| CliError::validation("credential reference identity is invalid"))?;
            let plan = resolve_openrouter_selection(&selection, &profile).map_err(|_| {
                CliError::validation("provider profile is incompatible with selected agents")
            })?;
            (plan, asb_agents::openrouter::OPENROUTER_MODEL)
        }
        AllAgentsProviderKind::Ollama => {
            return Err(CliError::validation(
                "provider profile is advertised but unavailable without verified daemon evidence",
            ));
        }
    };
    Ok((selection.provider, plan, model))
}

fn provider_selection_digest(
    catalog_sha256: &str,
    selection: &AllAgentsProviderSelection,
    effective: &[EffectiveCliAgent],
) -> Result<String, CliError> {
    let canonical = serde_json::to_vec(&(
        PROVIDER_CATALOG_VERSION,
        catalog_sha256,
        selection,
        effective,
    ))
    .map_err(|_| CliError::operation("provider plan cannot be encoded"))?;
    let mut digest = Sha256::new();
    digest.update(b"asb-cli-provider-plan-v1\0");
    digest.update(canonical);
    Ok(format!("{:x}", digest.finalize()))
}

const fn agent_id(agent: SelectedAgent) -> &'static str {
    match agent {
        SelectedAgent::OpenCode => "opencode",
        SelectedAgent::OpenDesk => "opendesk",
        SelectedAgent::Aider => "aider",
        SelectedAgent::Codex => "codex",
        SelectedAgent::Gemini => "gemini",
        SelectedAgent::QwenCode => "qwen_code",
        SelectedAgent::Goose => "goose",
        SelectedAgent::MiniSwe => "mini_swe",
        SelectedAgent::OpenHands => "openhands",
    }
}

fn parse_agent(value: &str) -> Result<SelectedAgent, CliError> {
    match value {
        "opencode" => Ok(SelectedAgent::OpenCode),
        "opendesk" => Ok(SelectedAgent::OpenDesk),
        "aider" => Ok(SelectedAgent::Aider),
        "codex" => Ok(SelectedAgent::Codex),
        "gemini" => Ok(SelectedAgent::Gemini),
        "qwen_code" => Ok(SelectedAgent::QwenCode),
        "goose" => Ok(SelectedAgent::Goose),
        "mini_swe" => Ok(SelectedAgent::MiniSwe),
        "openhands" => Ok(SelectedAgent::OpenHands),
        _ => Err(CliError::validation("unknown selected agent")),
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PlanFile {
    schema_version: u16,
    run_id: String,
    result_root: PathBuf,
    work_root: PathBuf,
    workload: String,
    agent: BatchAgent,
    point: PointInput,
    experiment: ExperimentManifestV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    measurement_selection: Option<MeasurementSelectionV1>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct BatchAgent {
    executable: PathBuf,
    executable_sha256: String,
    #[serde(default)]
    arguments: Vec<String>,
}

#[derive(Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct PointInput {
    measured: u32,
    #[serde(default)]
    warmups: u32,
    concurrency: u32,
    #[serde(default)]
    queue: u32,
    #[serde(default)]
    max_failures: u32,
    timeout_ms: u64,
    #[serde(default = "default_poll_ms")]
    poll_ms: u64,
    #[serde(default)]
    seed: u64,
    #[serde(default)]
    open_loop_interval_ms: Option<u64>,
    #[serde(default)]
    sweep_max_concurrency: Option<u32>,
}

const fn default_poll_ms() -> u64 {
    5
}

impl PointInput {
    fn build(self, concurrency: u32) -> Result<PointPlan, CliError> {
        if self
            .measured
            .checked_add(self.warmups)
            .is_none_or(|total| total > MAX_POINT_ATTEMPTS)
        {
            return Err(CliError::validation(
                "point attempt evidence exceeds the durable CLI bound",
            ));
        }
        if self.timeout_ms == 0 || self.timeout_ms > MAX_TIMEOUT_MS {
            return Err(CliError::validation(
                "point timeout is outside the supported bound",
            ));
        }
        let model = match self.open_loop_interval_ms {
            Some(0) => return Err(CliError::validation("open-loop interval must be positive")),
            Some(value) => LoadModel::OpenLoop {
                inter_arrival: Duration::from_millis(value),
            },
            None => LoadModel::ClosedLoop,
        };
        PointPlan::new(
            model,
            self.measured,
            self.warmups,
            concurrency,
            self.queue,
            self.max_failures,
            Duration::from_millis(self.timeout_ms),
            Duration::from_millis(self.poll_ms),
            self.seed,
        )
        .map_err(|_| CliError::validation("invalid capacity-point settings"))
    }
}

#[derive(Serialize)]
struct PlanOutput<'a> {
    schema_version: u16,
    ok: bool,
    command: &'static str,
    plan_schema_version: u16,
    run_id: &'a str,
    experiment_sha256: &'a str,
    workload: &'a str,
    agent_implementation: &'a str,
    agent_executable_sha256: &'a str,
    measured: u32,
    warmups: u32,
    concurrency: u32,
    sweep_max_concurrency: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    measurement_catalog_schema_version: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    measurement_catalog_sha256: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    measurement_selection_sha256: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    measurement_mode: Option<asb_protocol::ReplayMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    measurement_sample_interval_ns: Option<Option<u64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    measurement_selected_ids: Option<&'a [String]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider_selection_sha256: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider_profile_sha256: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider_profile: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider_model: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    credential_source: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    effective_agents: Option<&'a [EffectiveCliAgent]>,
}

fn plan(path: &Path, output: &mut dyn Write) -> Result<(), CliError> {
    render_plan(path, None, output)
}

fn plan_with_selection(
    path: &Path,
    selection_path: &Path,
    output: &mut dyn Write,
) -> Result<(), CliError> {
    render_plan(path, Some(selection_path), output)
}

fn plan_with_config(path: &Path, output: &mut dyn Write) -> Result<(), CliError> {
    let store = ConfigStore::from_environment()
        .map_err(|_| CliError::operation("ASB configuration location is unavailable"))?;
    plan_with_config_at(path, output, &store)
}

fn plan_with_config_at(
    path: &Path,
    output: &mut dyn Write,
    store: &ConfigStore,
) -> Result<(), CliError> {
    let plan = load_and_validate(path)?;
    let selection = selection_for_plan_at(&plan, store)?;
    validate_selection_binding(&plan, &selection)?;
    render_plan_values(&plan, Some(&selection), output)
}

fn render_plan(
    path: &Path,
    selection_path: Option<&Path>,
    output: &mut dyn Write,
) -> Result<(), CliError> {
    let (plan, selection) = load_plan_and_selection(path, selection_path)?;
    render_plan_values(&plan, selection.as_ref(), output)
}

fn render_plan_values(
    plan: &PlanFile,
    selection: Option<&ProviderPlanOutput>,
    output: &mut dyn Write,
) -> Result<(), CliError> {
    let measurement_selection = effective_measurement_selection(plan)?;
    write_json(
        output,
        &PlanOutput {
            schema_version: OUTPUT_SCHEMA_VERSION,
            ok: true,
            command: "plan",
            plan_schema_version: plan.schema_version,
            run_id: &plan.run_id,
            experiment_sha256: &plan.experiment.experiment_sha256,
            workload: &plan.workload,
            agent_implementation: &plan.experiment.agent.implementation,
            agent_executable_sha256: &plan.agent.executable_sha256,
            measured: plan.point.measured,
            warmups: plan.point.warmups,
            concurrency: plan.point.concurrency,
            sweep_max_concurrency: plan.point.sweep_max_concurrency,
            measurement_catalog_schema_version: (plan.schema_version == PLAN_SCHEMA_VERSION)
                .then_some(measurement_selection.catalog_schema_version),
            measurement_catalog_sha256: (plan.schema_version == PLAN_SCHEMA_VERSION)
                .then_some(measurement_selection.catalog_sha256.as_str()),
            measurement_selection_sha256: (plan.schema_version == PLAN_SCHEMA_VERSION)
                .then_some(measurement_selection.selection_sha256.as_str()),
            measurement_mode: (plan.schema_version == PLAN_SCHEMA_VERSION)
                .then_some(measurement_selection.mode),
            measurement_sample_interval_ns: (plan.schema_version == PLAN_SCHEMA_VERSION)
                .then_some(measurement_selection.sample_interval_ns),
            measurement_selected_ids: (plan.schema_version == PLAN_SCHEMA_VERSION)
                .then_some(measurement_selection.selected_ids.as_slice()),
            provider_selection_sha256: selection.map(|value| value.selection_sha256.as_str()),
            provider_profile_sha256: selection.map(|value| value.provider_profile_sha256.as_str()),
            provider_profile: selection.map(|value| value.provider_profile.as_str()),
            provider_model: selection.map(|value| value.model.as_str()),
            credential_source: selection.map(|value| value.credential_source.as_str()),
            effective_agents: selection.map(|value| value.effective.as_slice()),
        },
    )
}

fn selection_for_plan_at(
    plan: &PlanFile,
    store: &ConfigStore,
) -> Result<ProviderPlanOutput, CliError> {
    let config = load_openrouter_config_at(store)?;
    provider_plan_from_selection(&config, &plan.experiment.agent.implementation)
}

fn load_plan_and_selection(
    path: &Path,
    selection_path: Option<&Path>,
) -> Result<(PlanFile, Option<ProviderPlanOutput>), CliError> {
    let plan = load_and_validate(path)?;
    let selection = selection_path.map(load_provider_selection).transpose()?;
    if let Some(selection) = &selection {
        validate_selection_binding(&plan, selection)?;
    }
    Ok((plan, selection))
}

fn load_provider_selection(path: &Path) -> Result<ProviderPlanOutput, CliError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| CliError::validation("provider selection is unavailable"))?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > MAX_PROVIDER_SELECTION_BYTES
    {
        return Err(CliError::validation(
            "provider selection is not a bounded regular file",
        ));
    }
    let file = fs::File::open(path)
        .map_err(|_| CliError::validation("provider selection cannot be opened"))?;
    let opened = file
        .metadata()
        .map_err(|_| CliError::validation("provider selection metadata cannot be read"))?;
    if !opened.is_file()
        || opened.len() > MAX_PROVIDER_SELECTION_BYTES
        || opened.dev() != metadata.dev()
        || opened.ino() != metadata.ino()
    {
        return Err(CliError::validation(
            "provider selection changed during validation",
        ));
    }
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
    file.take(MAX_PROVIDER_SELECTION_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| CliError::validation("provider selection cannot be read"))?;
    if bytes.len() as u64 > MAX_PROVIDER_SELECTION_BYTES {
        return Err(CliError::validation(
            "provider selection exceeds its byte limit",
        ));
    }
    let selection: ProviderPlanOutput = serde_json::from_slice(&bytes)
        .map_err(|_| CliError::validation("provider selection syntax or shape is invalid"))?;
    validate_provider_selection(&selection)?;
    Ok(selection)
}

fn validate_provider_selection(value: &ProviderPlanOutput) -> Result<(), CliError> {
    if value.schema_version != OUTPUT_SCHEMA_VERSION
        || !value.ok
        || value.command != "provider-plan"
        || !value.dry_run
        || value.catalog_sha256 != provider_catalog_digest()
        || value.credential_source != "environment"
        || !valid_sha256(&value.credential_reference_sha256)
        || !valid_sha256(&value.provider_profile_sha256)
        || value.effective.is_empty()
        || value.effective.len() > MAX_SELECTED_AGENTS
    {
        return Err(CliError::validation(
            "provider selection identity is invalid",
        ));
    }
    let agents = value
        .effective
        .iter()
        .map(|item| parse_agent(&item.agent))
        .collect::<Result<Vec<_>, _>>()?;
    let (provider_kind, expected) = match value.provider_profile.as_str() {
        "openai" => {
            if value.model != asb_agents::openai::OPENAI_MODEL {
                return Err(CliError::validation(
                    "provider selection identity is invalid",
                ));
            }
            let verification_profile = OpenAiProfile::new(&value.credential_reference_sha256)
                .map_err(|_| CliError::operation("provider verifier profile cannot be created"))?;
            let selection = AllAgentsProviderSelection {
                schema_version: ALL_AGENTS_PROVIDER_SELECTION_V1,
                provider: AllAgentsProviderKind::OpenAi,
                agents,
            };
            let expected = resolve_openai_selection(&selection, &verification_profile)
                .map_err(|_| CliError::validation("provider selection is incompatible"))?;
            (AllAgentsProviderKind::OpenAi, expected)
        }
        "openrouter" => {
            if value.model != asb_agents::openrouter::OPENROUTER_MODEL {
                return Err(CliError::validation(
                    "provider selection identity is invalid",
                ));
            }
            let verification_profile = OpenRouterProfile::new(&value.credential_reference_sha256)
                .map_err(|_| {
                CliError::operation("provider verifier profile cannot be created")
            })?;
            let selection = AllAgentsProviderSelection {
                schema_version: ALL_AGENTS_PROVIDER_SELECTION_V1,
                provider: AllAgentsProviderKind::OpenRouter,
                agents,
            };
            let expected = resolve_openrouter_selection(&selection, &verification_profile)
                .map_err(|_| CliError::validation("provider selection is incompatible"))?;
            (AllAgentsProviderKind::OpenRouter, expected)
        }
        _ => {
            return Err(CliError::validation(
                "provider selection identity is invalid",
            ));
        }
    };
    let canonical_selection = AllAgentsProviderSelection {
        schema_version: ALL_AGENTS_PROVIDER_SELECTION_V1,
        provider: provider_kind,
        agents: value
            .effective
            .iter()
            .map(|item| parse_agent(&item.agent))
            .collect::<Result<Vec<_>, _>>()?,
    };
    if expected.profile_sha256() != value.provider_profile_sha256
        || value
            .effective
            .iter()
            .zip(expected.effective())
            .any(|(actual, expected)| {
                actual.agent != agent_id(expected.agent)
                    || actual.api_mode != expected.api_mode
                    || actual.profile_sha256 != expected.profile_sha256
            })
        || provider_selection_digest(
            &value.catalog_sha256,
            &canonical_selection,
            &value.effective,
        )? != value.selection_sha256
    {
        return Err(CliError::validation(
            "provider selection content address is invalid",
        ));
    }
    Ok(())
}

fn validate_selection_binding(
    plan: &PlanFile,
    selection: &ProviderPlanOutput,
) -> Result<(), CliError> {
    let selected = selection
        .effective
        .iter()
        .any(|item| item.agent == plan.experiment.agent.implementation);
    if !selected
        || plan.experiment.model.provider != selection.provider_profile
        || plan.experiment.model.model != selection.model
        || plan
            .experiment
            .model
            .settings
            .additional_settings_sha256
            .as_deref()
            != Some(selection.provider_profile_sha256.as_str())
    {
        return Err(CliError::validation(
            "provider selection does not match the experiment identity",
        ));
    }
    Ok(())
}

fn build_provider_launch(
    plan: &PlanFile,
    selection: &ProviderPlanOutput,
    run_id: &str,
    attempt_id: &str,
) -> Result<ProviderLaunchRecord, CliError> {
    let selected = selection
        .effective
        .iter()
        .find(|value| value.agent == plan.experiment.agent.implementation)
        .ok_or_else(|| {
            CliError::validation("provider selection does not include the launched agent")
        })?;
    let selected_agent = parse_agent(&selected.agent)?;
    let (projection, endpoint_sha256) = match selection.provider_profile.as_str() {
        "openai" => {
            let profile = OpenAiProfile::new(&selection.credential_reference_sha256)
                .map_err(|_| CliError::validation("provider credential reference is invalid"))?;
            let projection =
                ProviderLaunchProjection::openai(&profile, selected_agent).map_err(|_| {
                    CliError::validation("selected adapter has no exact provider route")
                })?;
            (
                projection,
                profile.provider_profile().endpoint.identity_sha256.clone(),
            )
        }
        "openrouter" => {
            let profile = OpenRouterProfile::new(&selection.credential_reference_sha256)
                .map_err(|_| CliError::validation("provider credential reference is invalid"))?;
            let projection = ProviderLaunchProjection::openrouter(&profile, selected_agent)
                .map_err(|_| {
                    CliError::validation("selected adapter has no exact provider route")
                })?;
            (
                projection,
                profile.provider_profile().endpoint.identity_sha256.clone(),
            )
        }
        _ => return Err(CliError::validation("provider selection is incompatible")),
    };
    let input = ProviderLaunchV1 {
        schema_version: asb_agents::provider_launch::PROVIDER_LAUNCH_V1,
        catalog_sha256: selection.catalog_sha256.clone(),
        selection_sha256: selection.selection_sha256.clone(),
        provider_profile_sha256: selection.provider_profile_sha256.clone(),
        endpoint_sha256,
        agent: selected.agent.clone(),
        adapter: selected.agent.clone(),
        api_mode: projection.api_mode(),
        provider: projection.provider().to_owned(),
        model: projection.model().to_owned(),
        settings_sha256: projection.settings_sha256().to_owned(),
        // The v1 plan carries one verified executable identity. Until the
        // runtime-bundle manifest is part of the plan, the executable's
        // content address is the fail-closed bundle identity as well.
        runtime: RuntimeBundleIdentity {
            bundle_sha256: plan.agent.executable_sha256.clone(),
            executable_sha256: plan.agent.executable_sha256.clone(),
        },
        credential: projection.credential().clone(),
        workload_sha256: plan.experiment.workload.workload_sha256.clone(),
        run_id: run_id.to_owned(),
        attempt_id: attempt_id.to_owned(),
        policy: LaunchPolicy {
            max_stdout_bytes: MAX_CAPTURE_BYTES as u64,
            max_stderr_bytes: MAX_CAPTURE_BYTES as u64,
            timeout_ms: plan.point.timeout_ms,
            max_environment_entries: 8,
            max_argv_entries: 1,
        },
    };
    ProviderLaunchRecord::bind(input, &projection)
        .map_err(|_| CliError::validation("provider-aware launch binding is invalid"))
}

fn load_and_validate(path: &Path) -> Result<PlanFile, CliError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| CliError::validation("experiment plan is unavailable"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > MAX_PLAN_BYTES {
        return Err(CliError::validation(
            "experiment plan is not a bounded regular file",
        ));
    }
    let file = fs::File::open(path)
        .map_err(|_| CliError::validation("experiment plan cannot be opened"))?;
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
    file.take(MAX_PLAN_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| CliError::validation("experiment plan cannot be read"))?;
    if bytes.len() as u64 > MAX_PLAN_BYTES {
        return Err(CliError::validation(
            "experiment plan exceeds its byte limit",
        ));
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| CliError::validation("experiment plan must be UTF-8"))?;
    let plan: PlanFile = toml::from_str(text)
        .map_err(|_| CliError::validation("experiment plan syntax or shape is invalid"))?;
    validate_plan(&plan)?;
    Ok(plan)
}

fn process_measurement_selection(
    mode: asb_protocol::ReplayMode,
    interval_ns: u64,
) -> Result<MeasurementSelectionV1, CliError> {
    MeasurementSelectionV1::new(
        &baseline_measurement_catalog(),
        [
            "process.cpu.system_time",
            "process.cpu.user_time",
            "process.faults.major",
            "process.faults.minor",
            "process.io.read",
            "process.io.write",
            "process.memory.resident",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        mode,
        Some(interval_ns),
    )
    .map_err(|_| CliError::validation("legacy measurement selection cannot be migrated"))
}

fn legacy_measurement_selection(plan: &PlanFile) -> Result<MeasurementSelectionV1, CliError> {
    let interval = plan
        .point
        .poll_ms
        .checked_mul(1_000_000)
        .ok_or_else(|| CliError::validation("legacy measurement cadence overflows"))?;
    process_measurement_selection(plan.experiment.controls.replay.mode, interval)
}

fn effective_measurement_selection(plan: &PlanFile) -> Result<MeasurementSelectionV1, CliError> {
    match (plan.schema_version, &plan.measurement_selection) {
        (LEGACY_PLAN_SCHEMA_VERSION, None) => legacy_measurement_selection(plan),
        (PLAN_SCHEMA_VERSION, Some(selection)) => Ok(selection.clone()),
        (LEGACY_PLAN_SCHEMA_VERSION, Some(_)) => Err(CliError::validation(
            "legacy experiment plans cannot declare measurement_selection",
        )),
        (PLAN_SCHEMA_VERSION, None) => Err(CliError::validation(
            "experiment plan v2 requires measurement_selection",
        )),
        _ => Err(CliError::validation("unsupported experiment plan version")),
    }
}

fn measurement_capabilities() -> Result<MeasurementExecutionCapabilities, CliError> {
    let architecture = match std::env::consts::ARCH {
        "x86_64" => MeasurementArchitecture::X86_64,
        "aarch64" => MeasurementArchitecture::Aarch64,
        _ => {
            return Err(CliError::validation_issue(
                "unsupported measurement architecture",
                asb_control::SettingsIssue::MeasurementPlatformUnsupported,
            ));
        }
    };
    let mut features = BTreeSet::new();
    if Path::new("/proc").is_dir() {
        features.insert(MeasurementPlatformFeature::Procfs);
    }
    if Path::new("/sys/fs/cgroup/cgroup.controllers").is_file() {
        features.insert(MeasurementPlatformFeature::CgroupV2);
    }
    Ok(MeasurementExecutionCapabilities {
        operating_system: MeasurementOperatingSystem::Linux,
        architecture,
        features,
        privileged: false,
        process_target: true,
        // The current runner owns only a PID. ASB's own cgroup is not an attempt target.
        cgroup_target: false,
    })
}

fn validate_measurement_selection(plan: &PlanFile) -> Result<MeasurementSelectionV1, CliError> {
    let selection = effective_measurement_selection(plan)?;
    if selection.mode != plan.experiment.controls.replay.mode {
        return Err(CliError::validation_issue(
            "measurement selection mode does not match experiment",
            asb_control::SettingsIssue::MeasurementModeUnsupported,
        ));
    }
    selection
        .validate(
            &baseline_measurement_catalog(),
            &measurement_capabilities()?,
        )
        .map_err(measurement_selection_cli_error)?;
    if let Some(interval_ns) = selection.sample_interval_ns {
        let duration_ns = plan
            .point
            .timeout_ms
            .checked_mul(1_000_000)
            .ok_or_else(|| {
                CliError::validation_issue(
                    "measurement schedule overflows",
                    asb_control::SettingsIssue::MeasurementCadenceCapacityExceeded,
                )
            })?;
        let slots = duration_ns
            .checked_div(interval_ns)
            .and_then(|value| value.checked_add(2))
            .ok_or_else(|| {
                CliError::validation_issue(
                    "measurement schedule is invalid",
                    asb_control::SettingsIssue::MeasurementInvalidCadence,
                )
            })?;
        let values = slots
            .checked_mul(u64::try_from(selection.selected_ids.len()).unwrap_or(u64::MAX))
            .ok_or_else(|| {
                CliError::validation_issue(
                    "measurement schedule exceeds its bound",
                    asb_control::SettingsIssue::MeasurementCadenceCapacityExceeded,
                )
            })?;
        if values > MAX_OPTIONAL_METRIC_VALUES_PER_ATTEMPT {
            return Err(CliError::validation_issue(
                "measurement schedule exceeds its evidence bound",
                asb_control::SettingsIssue::MeasurementCadenceCapacityExceeded,
            ));
        }
    }
    let identity_bytes = selection.selected_ids.iter().try_fold(0_u64, |total, id| {
        total.checked_add(
            u64::try_from(id.len())
                .unwrap_or(u64::MAX)
                .saturating_add(3),
        )
    });
    let attempts = plan
        .point
        .measured
        .checked_add(plan.point.warmups)
        .map(u64::from);
    let estimated_point_bytes = identity_bytes
        .and_then(|bytes| bytes.checked_mul(4))
        .and_then(|bytes| bytes.checked_add(2_048))
        .zip(attempts)
        .and_then(|(per_attempt, attempts)| per_attempt.checked_mul(attempts));
    if estimated_point_bytes.is_none_or(|bytes| {
        bytes
            > StoreLimits::default()
                .max_event_bytes
                .saturating_sub(64 * 1024)
    }) {
        return Err(CliError::validation(
            "measurement selection exceeds the terminal evidence bound",
        ));
    }
    Ok(selection)
}

fn measurement_selection_cli_error(error: MeasurementSelectionError) -> CliError {
    use MeasurementSelectionReason as Reason;
    use asb_control::SettingsIssue;
    let (message, settings_issue) = match error.reason {
        Reason::UnsupportedSchemaVersion => (
            "measurement selection schema version is unsupported",
            SettingsIssue::MeasurementUnsupportedSchemaVersion,
        ),
        Reason::CatalogGenerationMismatch => (
            "measurement catalog generation does not match",
            SettingsIssue::MeasurementCatalogGenerationMismatch,
        ),
        Reason::CatalogDigestMismatch => (
            "measurement catalog digest does not match",
            SettingsIssue::MeasurementCatalogDigestMismatch,
        ),
        Reason::SelectionDigestMismatch => (
            "measurement selection digest does not match",
            SettingsIssue::MeasurementSelectionDigestMismatch,
        ),
        Reason::TooManyMeasurements => (
            "measurement selection contains too many identities",
            SettingsIssue::MeasurementTooMany,
        ),
        Reason::NonCanonicalOrder => (
            "measurement identities are not in canonical order",
            SettingsIssue::MeasurementNonCanonicalOrder,
        ),
        Reason::DuplicateId => (
            "measurement selection contains a duplicate identity",
            SettingsIssue::MeasurementDuplicateId,
        ),
        Reason::UnknownId => (
            "measurement selection contains an unknown identity",
            SettingsIssue::MeasurementUnknownId,
        ),
        Reason::SourceUnqualified => (
            "measurement source is not qualified",
            SettingsIssue::MeasurementSourceUnqualified,
        ),
        Reason::ModeUnsupported => (
            "measurement execution mode is unsupported",
            SettingsIssue::MeasurementModeUnsupported,
        ),
        Reason::PlatformUnsupported => (
            "measurement platform is unsupported",
            SettingsIssue::MeasurementPlatformUnsupported,
        ),
        Reason::PermissionRequired => (
            "measurement permission is unavailable",
            SettingsIssue::MeasurementPermissionRequired,
        ),
        Reason::TargetScopeUnavailable => (
            "measurement target scope is unavailable",
            SettingsIssue::MeasurementTargetScopeUnavailable,
        ),
        Reason::InvalidCadence => (
            "measurement cadence is invalid",
            SettingsIssue::MeasurementInvalidCadence,
        ),
        Reason::CadenceTooFast => (
            "measurement cadence is too fast",
            SettingsIssue::MeasurementCadenceTooFast,
        ),
        Reason::CadenceCapacityExceeded => (
            "measurement cadence exceeds its capacity",
            SettingsIssue::MeasurementCadenceCapacityExceeded,
        ),
        Reason::WireTooLarge | Reason::InvalidWire | Reason::Serialization => (
            "measurement selection encoding is invalid",
            SettingsIssue::InvalidFormat,
        ),
    };
    CliError::validation_issue(message, settings_issue)
}

fn validate_plan(plan: &PlanFile) -> Result<(), CliError> {
    if !matches!(
        plan.schema_version,
        LEGACY_PLAN_SCHEMA_VERSION | PLAN_SCHEMA_VERSION
    ) {
        return Err(CliError::validation("unsupported experiment plan version"));
    }
    validate_id(&plan.run_id)?;
    if plan.run_id.len() > MAX_ID_BYTES - 16 {
        return Err(CliError::validation(
            "run identity is too long for sweep points",
        ));
    }
    validate_root(&plan.result_root)?;
    validate_root(&plan.work_root)?;
    if plan.result_root == plan.work_root
        || plan.result_root.starts_with(&plan.work_root)
        || plan.work_root.starts_with(&plan.result_root)
    {
        return Err(CliError::validation(
            "result and work roots must be disjoint",
        ));
    }
    validate_agent(&plan.agent)?;
    plan.experiment
        .validate()
        .map_err(|_| CliError::validation("experiment identity is invalid"))?;
    if plan.experiment.agent.binary_sha256 != plan.agent.executable_sha256 {
        return Err(CliError::validation(
            "agent executable identity does not match experiment",
        ));
    }
    if plan.experiment.platform.architecture != std::env::consts::ARCH {
        return Err(CliError::validation(
            "experiment architecture does not match this host",
        ));
    }
    validate_measurement_selection(plan)?;
    let workload = describe_workload(&plan.workload)
        .map_err(|_| CliError::validation("unknown or invalid workload"))?;
    if workload.workload_id.0 != plan.experiment.workload.workload
        || workload.version != plan.experiment.workload.workload_revision
        || workload.content_sha256 != plan.experiment.workload.workload_sha256
        || workload.scoring_version != plan.experiment.workload.scorer_revision
    {
        return Err(CliError::validation(
            "workload identity does not match experiment",
        ));
    }
    plan.point.build(plan.point.concurrency)?;
    process_limits(plan.point)?;
    if let Some(maximum) = plan.point.sweep_max_concurrency {
        capacity_order(maximum, plan.point.seed)
            .map_err(|_| CliError::validation("invalid sweep concurrency bound"))?;
    }
    Ok(())
}

fn validate_id(value: &str) -> Result<(), CliError> {
    if value.is_empty()
        || value.len() > MAX_ID_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(CliError::validation("run identity is invalid"));
    }
    Ok(())
}

fn validate_root(path: &Path) -> Result<(), CliError> {
    if !path.is_absolute() {
        return Err(CliError::validation("run roots must be absolute"));
    }
    let existing = if path.exists() {
        path
    } else {
        path.parent()
            .ok_or_else(|| CliError::validation("run root has no parent"))?
    };
    let metadata = fs::symlink_metadata(existing)
        .map_err(|_| CliError::validation("run root parent is unavailable"))?;
    if metadata.file_type().is_symlink()
        || !metadata.is_dir()
        || fs::canonicalize(existing).ok().as_deref() != Some(existing)
    {
        return Err(CliError::validation("run root topology is unsafe"));
    }
    Ok(())
}

fn validate_agent(agent: &BatchAgent) -> Result<(), CliError> {
    if !agent.executable.is_absolute()
        || fs::canonicalize(&agent.executable).ok().as_deref() != Some(&agent.executable)
    {
        return Err(CliError::validation(
            "agent executable path is not exact and absolute",
        ));
    }
    let metadata = fs::symlink_metadata(&agent.executable)
        .map_err(|_| CliError::validation("agent executable is unavailable"))?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() == 0
        || metadata.len() > MAX_EXECUTABLE_BYTES
        || metadata.permissions().mode() & 0o111 == 0
    {
        return Err(CliError::validation(
            "agent executable is not a bounded regular file",
        ));
    }
    if !valid_sha256(&agent.executable_sha256)
        || digest_file(&agent.executable)? != agent.executable_sha256
    {
        return Err(CliError::validation(
            "agent executable digest does not match",
        ));
    }
    if !agent.arguments.is_empty() {
        return Err(CliError::validation(
            "agent arguments are not provenance-pinned; use a pinned wrapper executable",
        ));
    }
    Ok(())
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn digest_file(path: &Path) -> Result<String, CliError> {
    let mut file = fs::File::open(path)
        .map_err(|_| CliError::validation("agent executable cannot be opened"))?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|_| CliError::validation("agent executable cannot be read"))?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn process_limits(point: PointInput) -> Result<ProcessLimits, CliError> {
    ProcessLimits::new(
        MAX_CAPTURE_BYTES,
        MAX_CAPTURE_BYTES,
        Duration::from_millis(point.timeout_ms),
        Duration::from_secs(1),
        Duration::from_millis(point.poll_ms),
    )
    .map_err(|_| CliError::validation("invalid process limits"))
}

#[derive(Clone, Debug, Serialize)]
struct AttemptSummary {
    input_id: u32,
    phase: &'static str,
    outcome: &'static str,
    grade_passed: bool,
    failed_check_count: usize,
    termination: &'static str,
    exit_code: Option<i32>,
    signal: Option<i32>,
    elapsed_ns: u64,
    spawn_retry_count: u8,
    metric_sample_count: usize,
    metric_available_count: u64,
    metric_unavailable_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    metric_scheduled_collections: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metric_completed_collections: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metric_lost_collections: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metric_collection_time_ns: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metric_max_collection_time_ns: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metric_requested_ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metric_collected_ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metric_unavailable_ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metric_omitted_ids: Option<Vec<String>>,
    stdout_bytes: u64,
    stderr_bytes: u64,
    output_truncated: bool,
}

struct PreparedGuard(Option<PreparedWorkloadChoice>);

impl PreparedGuard {
    fn new(workload: PreparedWorkloadChoice) -> Self {
        Self(Some(workload))
    }

    fn cleanup(&mut self) -> Result<(), CliError> {
        if let Some(workload) = self.0.take() {
            workload
                .cleanup()
                .map_err(|_| CliError::operation("attempt workspace cleanup failed"))?;
        }
        Ok(())
    }
}

impl PreparedGuard {
    fn workspace(&self) -> PathBuf {
        self.0
            .as_ref()
            .expect("prepared workload remains guarded")
            .workspace()
    }
    fn prompt(&self) -> String {
        self.0
            .as_ref()
            .expect("prepared workload remains guarded")
            .prompt()
    }
    fn evaluate(&self) -> Result<WorkloadEvaluation, String> {
        self.0
            .as_ref()
            .expect("prepared workload remains guarded")
            .evaluate()
    }
}

impl Drop for PreparedGuard {
    fn drop(&mut self) {
        if let Some(workload) = self.0.take() {
            let _ = workload.cleanup();
        }
    }
}

#[derive(Clone, Serialize)]
struct PointOutput {
    concurrency: u32,
    decision: &'static str,
    stop_reason: &'static str,
    admission_stop_reason: &'static str,
    admitted: usize,
    missed: usize,
    completed: usize,
    failed: usize,
    timed_out: usize,
    cancelled: usize,
    infrastructure_failures: usize,
    attempts: Vec<AttemptSummary>,
    scheduler_attempts: Vec<SchedulerAttemptEvidence>,
    attempt_failures: Vec<AttemptFailureEvidence>,
}

#[derive(Clone, Serialize)]
struct SchedulerAttemptEvidence {
    input_id: u32,
    phase: &'static str,
    scheduled_at_ns: u64,
    started_at_ns: Option<u64>,
    finished_at_ns: Option<u64>,
    outcome: Option<&'static str>,
    queue_delay_ns: u64,
    missed: bool,
    miss_reason: Option<String>,
}

#[derive(Clone, Serialize)]
struct AttemptFailureEvidence {
    input_id: u32,
    phase: &'static str,
    code: &'static str,
    message: &'static str,
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ExecutionDefinition {
    plan_schema_version: u16,
    workload: String,
    agent_executable_sha256: String,
    batch_protocol: String,
    requested_point: PointInput,
    executed_concurrency: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    measurement_catalog_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    measurement_selection_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    provider_selection_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    provider_launch_sha256: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredRunDefinition {
    experiment: ExperimentManifestV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    measurement_selection: Option<MeasurementSelectionV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    provider_selection: Option<ProviderPlanOutput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    provider_launch: Option<ProviderLaunchRecord>,
    execution: ExecutionDefinition,
    execution_sha256: String,
}

fn execution_definition(
    plan: &PlanFile,
    concurrency: u32,
    selection: Option<&ProviderPlanOutput>,
    launch: Option<&ProviderLaunchRecord>,
) -> ExecutionDefinition {
    ExecutionDefinition {
        plan_schema_version: plan.schema_version,
        workload: plan.workload.clone(),
        agent_executable_sha256: plan.agent.executable_sha256.clone(),
        batch_protocol: "batch-stdio-v1".to_owned(),
        requested_point: plan.point,
        executed_concurrency: concurrency,
        measurement_catalog_sha256: plan
            .measurement_selection
            .as_ref()
            .map(|value| value.catalog_sha256.clone()),
        measurement_selection_sha256: plan
            .measurement_selection
            .as_ref()
            .map(|value| value.selection_sha256.clone()),
        provider_selection_sha256: selection.map(|value| value.selection_sha256.clone()),
        provider_launch_sha256: launch.map(|value| value.launch_sha256.clone()),
    }
}

fn execution_digest(execution: &ExecutionDefinition) -> Result<String, CliError> {
    let bytes = serde_json::to_vec(execution)
        .map_err(|_| CliError::operation("execution definition cannot be encoded"))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn validate_stored_definition(definition: &StoredRunDefinition) -> Result<(), CliError> {
    definition
        .experiment
        .validate()
        .map_err(|_| CliError::validation("run experiment is invalid"))?;
    if let Some(selection) = &definition.provider_selection {
        validate_provider_selection(selection)?;
        let synthetic_plan = PlanFile {
            schema_version: definition.execution.plan_schema_version,
            run_id: "stored-validation".to_owned(),
            result_root: PathBuf::from("/stored-validation-results"),
            work_root: PathBuf::from("/stored-validation-work"),
            workload: definition.execution.workload.clone(),
            agent: BatchAgent {
                executable: PathBuf::from("/stored-validation-agent"),
                executable_sha256: definition.execution.agent_executable_sha256.clone(),
                arguments: Vec::new(),
            },
            point: definition.execution.requested_point,
            experiment: definition.experiment.clone(),
            measurement_selection: definition.measurement_selection.clone(),
        };
        validate_selection_binding(&synthetic_plan, selection)?;
    }
    if let Some(launch) = &definition.provider_launch {
        launch
            .validate()
            .map_err(|_| CliError::validation("stored provider launch is invalid"))?;
        let selection = definition.provider_selection.as_ref();
        if selection.is_none()
            || definition.execution.provider_launch_sha256.as_deref()
                != Some(launch.launch_sha256.as_str())
            || launch.input.run_id.is_empty()
            || selection.is_some_and(|value| {
                launch.input.catalog_sha256 != value.catalog_sha256
                    || launch.input.selection_sha256 != value.selection_sha256
                    || launch.input.provider_profile_sha256 != value.provider_profile_sha256
                    || launch.input.provider != value.provider_profile
                    || launch.input.model != value.model
            })
            || launch.input.agent != definition.experiment.agent.implementation
            || launch.input.workload_sha256 != definition.experiment.workload.workload_sha256
            || launch.input.runtime.executable_sha256
                != definition.execution.agent_executable_sha256
        {
            return Err(CliError::validation(
                "stored provider launch is not bound to its execution",
            ));
        }
    }
    let workload = describe_workload(&definition.execution.workload)
        .map_err(|_| CliError::validation("stored run workload is invalid"))?;
    let measurement_selection = match definition.execution.plan_schema_version {
        LEGACY_PLAN_SCHEMA_VERSION => {
            let expected = legacy_measurement_selection(&PlanFile {
                schema_version: LEGACY_PLAN_SCHEMA_VERSION,
                run_id: "stored-validation".to_owned(),
                result_root: PathBuf::from("/stored-validation-results"),
                work_root: PathBuf::from("/stored-validation-work"),
                workload: definition.execution.workload.clone(),
                agent: BatchAgent {
                    executable: PathBuf::from("/stored-validation-agent"),
                    executable_sha256: definition.execution.agent_executable_sha256.clone(),
                    arguments: Vec::new(),
                },
                point: definition.execution.requested_point,
                experiment: definition.experiment.clone(),
                measurement_selection: None,
            })?;
            if definition
                .measurement_selection
                .as_ref()
                .is_some_and(|selection| selection != &expected)
            {
                return Err(CliError::validation(
                    "stored legacy measurement migration is invalid",
                ));
            }
            expected
        }
        PLAN_SCHEMA_VERSION => definition
            .measurement_selection
            .clone()
            .ok_or_else(|| CliError::validation("stored measurement selection is absent"))?,
        _ => return Err(CliError::validation("run execution definition is invalid")),
    };
    let stored_architecture = match definition.experiment.platform.architecture.as_str() {
        "x86_64" => MeasurementArchitecture::X86_64,
        "aarch64" => MeasurementArchitecture::Aarch64,
        _ => {
            return Err(CliError::validation(
                "stored measurement architecture is invalid",
            ));
        }
    };
    let stored_capabilities = MeasurementExecutionCapabilities {
        operating_system: MeasurementOperatingSystem::Linux,
        architecture: stored_architecture,
        features: BTreeSet::from([
            MeasurementPlatformFeature::Procfs,
            MeasurementPlatformFeature::CgroupV2,
        ]),
        privileged: false,
        process_target: true,
        cgroup_target: false,
    };
    if measurement_selection.mode != definition.experiment.controls.replay.mode
        || measurement_selection
            .validate(&baseline_measurement_catalog(), &stored_capabilities)
            .is_err()
    {
        return Err(CliError::validation(
            "stored measurement selection is invalid",
        ));
    }
    if definition.execution.plan_schema_version == PLAN_SCHEMA_VERSION
        && (definition.execution.measurement_catalog_sha256.as_deref()
            != Some(measurement_selection.catalog_sha256.as_str())
            || definition.execution.measurement_selection_sha256.as_deref()
                != Some(measurement_selection.selection_sha256.as_str()))
    {
        return Err(CliError::validation(
            "stored measurement selection is not bound to its execution",
        ));
    }
    if !matches!(
        definition.execution.plan_schema_version,
        LEGACY_PLAN_SCHEMA_VERSION | PLAN_SCHEMA_VERSION
    ) || definition.execution.batch_protocol != "batch-stdio-v1"
        || !valid_sha256(&definition.execution.agent_executable_sha256)
        || definition.execution.agent_executable_sha256 != definition.experiment.agent.binary_sha256
        || workload.workload_id.0 != definition.experiment.workload.workload
        || workload.version != definition.experiment.workload.workload_revision
        || workload.content_sha256 != definition.experiment.workload.workload_sha256
        || workload.scoring_version != definition.experiment.workload.scorer_revision
        || definition
            .execution
            .requested_point
            .build(definition.execution.executed_concurrency)
            .is_err()
        || execution_digest(&definition.execution)? != definition.execution_sha256
        || definition.execution.provider_selection_sha256.as_deref()
            != definition
                .provider_selection
                .as_ref()
                .map(|value| value.selection_sha256.as_str())
        || definition.execution.provider_launch_sha256.as_deref()
            != definition
                .provider_launch
                .as_ref()
                .map(|value| value.launch_sha256.as_str())
    {
        return Err(CliError::validation("run execution definition is invalid"));
    }
    Ok(())
}

#[derive(Serialize)]
struct ExecuteOutput {
    schema_version: u16,
    ok: bool,
    command: &'static str,
    run_ids: Vec<String>,
    points: Vec<PointOutput>,
    highest_confirmed_capacity: Option<u32>,
    cancelled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    measurement_catalog_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    measurement_selection_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider_selection_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider_profile_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider_launch_sha256: Option<String>,
}

fn execute(
    path: &Path,
    sweep: bool,
    live_provider: bool,
    live_factory: Option<LiveProviderAttemptFactory>,
    output: &mut dyn Write,
    progress: &mut dyn Write,
) -> Result<u8, CliError> {
    execute_inner(
        path,
        None,
        sweep,
        live_provider,
        live_factory,
        output,
        progress,
    )
}

fn execute_with_selection(
    path: &Path,
    selection_path: &Path,
    sweep: bool,
    live_provider: bool,
    output: &mut dyn Write,
    progress: &mut dyn Write,
) -> Result<u8, CliError> {
    execute_inner(
        path,
        Some(selection_path),
        sweep,
        live_provider,
        None,
        output,
        progress,
    )
}

fn execute_with_config(
    path: &Path,
    sweep: bool,
    live_provider: bool,
    output: &mut dyn Write,
    progress: &mut dyn Write,
) -> Result<u8, CliError> {
    let store = ConfigStore::from_environment()
        .map_err(|_| CliError::operation("ASB configuration location is unavailable"))?;
    execute_inner_from_source(
        path,
        SelectionSource::Config(&store),
        sweep,
        live_provider,
        None,
        true,
        output,
        progress,
    )
}

fn execute_inner(
    path: &Path,
    selection_path: Option<&Path>,
    sweep: bool,
    live_provider: bool,
    live_factory: Option<LiveProviderAttemptFactory>,
    output: &mut dyn Write,
    progress: &mut dyn Write,
) -> Result<u8, CliError> {
    execute_inner_from_source(
        path,
        SelectionSource::Path(selection_path),
        sweep,
        live_provider,
        live_factory,
        false,
        output,
        progress,
    )
}

enum SelectionSource<'a> {
    Path(Option<&'a Path>),
    Config(&'a ConfigStore),
}

#[allow(clippy::too_many_arguments)]
fn execute_inner_from_source(
    path: &Path,
    source: SelectionSource<'_>,
    sweep: bool,
    live_provider: bool,
    live_factory: Option<LiveProviderAttemptFactory>,
    local_mock: bool,
    output: &mut dyn Write,
    progress: &mut dyn Write,
) -> Result<u8, CliError> {
    let (plan, selection) = match source {
        SelectionSource::Path(selection_path) => load_plan_and_selection(path, selection_path)?,
        SelectionSource::Config(store) => {
            let plan = load_and_validate(path)?;
            let selection = selection_for_plan_at(&plan, store)?;
            validate_selection_binding(&plan, &selection)?;
            (plan, Some(selection))
        }
    };
    if live_provider && selection.is_none() {
        return Err(CliError::validation(
            "--live-provider requires an explicit provider selection",
        ));
    }
    if live_provider && live_factory.is_none() {
        return Err(CliError::validation(
            "live provider requires a runtime-issued attempt factory",
        ));
    }
    if plan.experiment.controls.replay.mode == asb_protocol::ReplayMode::Replay {
        return Err(CliError::validation(
            "run cannot use a replay plan without an explicit strict cassette execution",
        ));
    }
    let measurement_selection = effective_measurement_selection(&plan)?;
    if sweep && plan.point.sweep_max_concurrency.is_none() {
        return Err(CliError::validation("sweep requires sweep_max_concurrency"));
    }
    prepare_root(&plan.result_root)?;
    prepare_root(&plan.work_root)?;
    let store = Arc::new(
        AtomicStore::open(&plan.result_root, StoreLimits::default())
            .map_err(|_| CliError::operation("result store cannot be opened"))?,
    );
    let cancelled = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(SIGINT, Arc::clone(&cancelled))
        .map_err(|_| CliError::operation("SIGINT handler cannot be installed"))?;
    signal_hook::flag::register(SIGTERM, Arc::clone(&cancelled))
        .map_err(|_| CliError::operation("SIGTERM handler cannot be installed"))?;
    let concurrencies = if sweep {
        capacity_order(
            plan.point.sweep_max_concurrency.unwrap_or(0),
            plan.point.seed,
        )
        .map_err(|_| CliError::validation("invalid sweep concurrency bound"))?
    } else {
        vec![plan.point.concurrency]
    };
    let mut run_ids = Vec::with_capacity(concurrencies.len());
    let mut points = Vec::with_capacity(concurrencies.len());
    let mut capacity = Vec::with_capacity(concurrencies.len());
    let mut provider_launch_sha256 = None;
    for concurrency in concurrencies {
        if cancelled.load(Ordering::SeqCst) {
            break;
        }
        let run_id = if sweep {
            format!("{}-c{concurrency}", plan.run_id)
        } else {
            plan.run_id.clone()
        };
        let _ = writeln!(progress, "starting {run_id}");
        let point = run_point_with_selection(
            Arc::clone(&store),
            &plan,
            run_id.clone(),
            concurrency,
            selection.as_ref(),
            Arc::clone(&cancelled),
            live_provider,
            live_factory.clone(),
            local_mock,
        )?;
        if provider_launch_sha256.is_none() {
            provider_launch_sha256 = selection.as_ref().map(|value| {
                build_provider_launch(&plan, value, &run_id, &format!("{run_id}-attempt"))
                    .expect("launch was validated before execution")
                    .launch_sha256
            });
        }
        let decision = match point.decision {
            "pass" => CapacityDecision::Pass,
            "fail" => CapacityDecision::Fail,
            _ => CapacityDecision::Inconclusive,
        };
        capacity.push(CapacityPoint {
            concurrency,
            decision,
        });
        run_ids.push(run_id);
        points.push(point);
    }
    let exit_code = execution_exit_code(
        cancelled.load(Ordering::SeqCst),
        points.iter().map(|point| point.decision),
    );
    write_json(
        output,
        &ExecuteOutput {
            schema_version: OUTPUT_SCHEMA_VERSION,
            ok: exit_code == 0,
            command: if sweep { "sweep" } else { "run" },
            run_ids,
            points,
            highest_confirmed_capacity: highest_confirmed_capacity(&capacity),
            cancelled: cancelled.load(Ordering::SeqCst),
            measurement_catalog_sha256: (plan.schema_version == PLAN_SCHEMA_VERSION)
                .then_some(measurement_selection.catalog_sha256),
            measurement_selection_sha256: (plan.schema_version == PLAN_SCHEMA_VERSION)
                .then_some(measurement_selection.selection_sha256),
            provider_selection_sha256: selection
                .as_ref()
                .map(|value| value.selection_sha256.clone()),
            provider_profile_sha256: selection
                .as_ref()
                .map(|value| value.provider_profile_sha256.clone()),
            provider_launch_sha256,
        },
    )?;
    Ok(exit_code)
}

fn execution_exit_code<'a>(cancelled: bool, decisions: impl Iterator<Item = &'a str>) -> u8 {
    if cancelled {
        130
    } else {
        let decisions = decisions.collect::<Vec<_>>();
        if decisions.contains(&"inconclusive") {
            6
        } else if decisions.contains(&"fail") {
            5
        } else {
            0
        }
    }
}

fn run_point(
    store: Arc<AtomicStore>,
    plan: &PlanFile,
    run_id: String,
    concurrency: u32,
    cancelled: Arc<AtomicBool>,
) -> Result<PointOutput, CliError> {
    run_point_with_selection(
        store,
        plan,
        run_id,
        concurrency,
        None,
        cancelled,
        false,
        None,
        false,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_point_with_selection(
    store: Arc<AtomicStore>,
    plan: &PlanFile,
    run_id: String,
    concurrency: u32,
    selection: Option<&ProviderPlanOutput>,
    cancelled: Arc<AtomicBool>,
    live_provider: bool,
    live_factory: Option<LiveProviderAttemptFactory>,
    local_mock: bool,
) -> Result<PointOutput, CliError> {
    let started = Instant::now();
    let attempt_id = format!("{run_id}-attempt");
    // Bind and verify provider selection before creating any durable run or
    // preparing a work root. A metadata-only selection is never launchable.
    let launch = selection
        .map(|value| build_provider_launch(plan, value, &run_id, &attempt_id))
        .transpose()?;
    let measurement_selection = validate_measurement_selection(plan)?;
    let execution = execution_definition(plan, concurrency, selection, launch.as_ref());
    let execution_sha256 = execution_digest(&execution)?;
    let manifest = RunManifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        run_id: Id(run_id.clone()),
        attempt_id: Id(attempt_id.clone()),
        definition: serde_json::to_value(&StoredRunDefinition {
            experiment: plan.experiment.clone(),
            measurement_selection: Some(measurement_selection.clone()),
            provider_selection: selection.cloned(),
            provider_launch: launch.clone(),
            execution,
            execution_sha256: execution_sha256.clone(),
        })
        .map_err(|_| CliError::operation("experiment cannot be encoded"))?,
    };
    store
        .create_run(&manifest)
        .map_err(|_| CliError::operation("run identity already exists or cannot be stored"))?;
    append_state(
        &store,
        &run_id,
        &attempt_id,
        0,
        started,
        ExecutionState::Planned,
        json!({}),
    )?;
    append_state(
        &store,
        &run_id,
        &attempt_id,
        1,
        started,
        ExecutionState::Prepared,
        json!({}),
    )?;
    append_state(
        &store,
        &run_id,
        &attempt_id,
        2,
        started,
        ExecutionState::Running,
        json!({}),
    )?;
    let summaries = Arc::new(Mutex::new(Vec::<AttemptSummary>::new()));
    let attempt_failures = Arc::new(Mutex::new(Vec::<AttemptFailureEvidence>::new()));
    let plan_owned = plan.clone();
    let launch_owned = launch.clone();
    let measurement_selection_owned = measurement_selection.clone();
    let work_root = plan.work_root.clone();
    let run_for_attempt = run_id.clone();
    let summaries_for_attempt = Arc::clone(&summaries);
    let failures_for_attempt = Arc::clone(&attempt_failures);
    let cancelled_for_attempt = Arc::clone(&cancelled);
    let live_factory = live_factory.map(Arc::new);
    let point_plan = plan.point.build(concurrency)?;
    let result =
        Scheduler::new(SystemClock::start()).run_with_context(point_plan, move |context| {
            if cancelled_for_attempt.load(Ordering::SeqCst) {
                return AttemptOutcome::Cancelled;
            }
            let attempt = if live_provider {
                let Some(factory) = live_factory.as_ref() else {
                    return AttemptOutcome::InfrastructureFailure;
                };
                match factory.acquire(context.input_id(), context.is_warmup()) {
                    Ok(attempt) => Some(attempt),
                    Err(_) => return AttemptOutcome::InfrastructureFailure,
                }
            } else {
                None
            };
            let summary = if local_mock {
                let backend = match LocalProviderMockBackend::provision() {
                    Ok(backend) => backend,
                    Err(_) => return AttemptOutcome::InfrastructureFailure,
                };
                let mock_attempt_id = context.input_id().saturating_add(1);
                let mut mock_attempt = match backend.issue_attempt(mock_attempt_id) {
                    Ok(attempt) => attempt,
                    Err(_) => return AttemptOutcome::InfrastructureFailure,
                };
                let request = format!(
                    "{}:{}:{}",
                    run_for_attempt,
                    context.input_id(),
                    if context.is_warmup() {
                        "warmup"
                    } else {
                        "measured"
                    }
                );
                let response = if cancelled_for_attempt.load(Ordering::SeqCst) {
                    mock_attempt.cancel();
                    Err(())
                } else {
                    mock_attempt
                        .execute_default(request.as_bytes())
                        .map_err(|_| ())
                };
                match response {
                    Ok(response) => Ok(Some(local_mock_attempt_summary(
                        context.input_id(),
                        context.is_warmup(),
                        response.response_sha256(),
                    ))),
                    Err(()) => Ok(None),
                }
            } else {
                run_attempt(
                    &plan_owned,
                    &work_root,
                    &run_for_attempt,
                    context.input_id(),
                    context.is_warmup(),
                    launch_owned.as_ref(),
                    &measurement_selection_owned,
                    Arc::clone(&cancelled_for_attempt),
                    live_provider,
                    attempt,
                )
            };
            let outcome = match summary.as_ref() {
                Ok(None) => AttemptOutcome::Cancelled,
                Ok(Some(value)) => match value.outcome {
                    "completed" => AttemptOutcome::Completed,
                    "failed" => AttemptOutcome::Failed,
                    "timed_out" => AttemptOutcome::TimedOut,
                    "cancelled" => AttemptOutcome::Cancelled,
                    _ => AttemptOutcome::InfrastructureFailure,
                },
                Err(error) => {
                    failures_for_attempt
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .push(AttemptFailureEvidence {
                            input_id: context.input_id(),
                            phase: if context.is_warmup() {
                                "warmup"
                            } else {
                                "measured"
                            },
                            code: error.code,
                            message: error.message,
                        });
                    AttemptOutcome::InfrastructureFailure
                }
            };
            if let Ok(Some(summary)) = summary {
                summaries_for_attempt
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .push(summary);
            }
            outcome
        });
    append_state(
        &store,
        &run_id,
        &attempt_id,
        3,
        started,
        ExecutionState::Collecting,
        json!({}),
    )?;
    let mut attempts = summaries
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    attempts.sort_by_key(|item| (item.phase != "warmup", item.input_id));
    let measured = || result.measured().filter_map(|item| item.outcome());
    let completed = measured()
        .filter(|outcome| *outcome == AttemptOutcome::Completed)
        .count();
    let failed = measured()
        .filter(|outcome| *outcome == AttemptOutcome::Failed)
        .count();
    let timed_out = measured()
        .filter(|outcome| *outcome == AttemptOutcome::TimedOut)
        .count();
    let cancelled_count = measured()
        .filter(|outcome| *outcome == AttemptOutcome::Cancelled)
        .count();
    let infrastructure_failures = measured()
        .filter(|outcome| *outcome == AttemptOutcome::InfrastructureFailure)
        .count();
    let admitted = result
        .measured()
        .filter(|item| item.outcome().is_some())
        .count();
    let decision = if result.contaminated() || infrastructure_failures > 0 {
        "inconclusive"
    } else if result.missed_arrivals() == 0
        && failed == 0
        && timed_out == 0
        && cancelled_count == 0
        && completed == plan.point.measured as usize
    {
        "pass"
    } else {
        "fail"
    };
    let terminal = if cancelled.load(Ordering::SeqCst) || cancelled_count > 0 {
        ExecutionState::Cancelled
    } else if decision == "pass" {
        ExecutionState::Completed
    } else {
        ExecutionState::Failed
    };
    let scheduler_attempts = result
        .attempts()
        .iter()
        .map(|attempt| {
            let outcome = attempt.outcome();
            let (started_at_ns, finished_at_ns, queue_delay_ns) =
                match (attempt.started_at(), attempt.finished_at()) {
                    (Some(started), Some(finished)) => (
                        Some(duration_ns(started)),
                        Some(duration_ns(finished)),
                        duration_ns(attempt.queue_delay()),
                    ),
                    (None, None) => (None, None, 0),
                    _ if outcome == Some(AttemptOutcome::InfrastructureFailure) => (None, None, 0),
                    (started, finished) => (
                        started.map(duration_ns),
                        finished.map(duration_ns),
                        duration_ns(attempt.queue_delay()),
                    ),
                };
            SchedulerAttemptEvidence {
                input_id: attempt.input_id(),
                phase: if attempt.is_warmup() {
                    "warmup"
                } else {
                    "measured"
                },
                scheduled_at_ns: duration_ns(attempt.scheduled_at()),
                started_at_ns,
                finished_at_ns,
                outcome: outcome.map(attempt_outcome_name),
                queue_delay_ns,
                missed: attempt.missed(),
                miss_reason: attempt.miss_reason().map(miss_reason_name),
            }
        })
        .collect();
    let attempt_failures = attempt_failures
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    let point = PointOutput {
        concurrency,
        decision,
        stop_reason: stop_reason_name(result.stop_reason()),
        admission_stop_reason: stop_reason_name(result.admission_stop_reason()),
        admitted,
        missed: result.missed_arrivals(),
        completed,
        failed,
        timed_out,
        cancelled: cancelled_count,
        infrastructure_failures,
        attempts,
        scheduler_attempts,
        attempt_failures,
    };
    append_state(
        &store,
        &run_id,
        &attempt_id,
        4,
        started,
        terminal,
        json!({"execution_sha256": execution_sha256, "point": &point}),
    )?;
    Ok(point)
}

fn local_mock_attempt_summary(
    input_id: u32,
    warmup: bool,
    response_sha256: &str,
) -> AttemptSummary {
    debug_assert_eq!(response_sha256.len(), 64);
    AttemptSummary {
        input_id,
        phase: if warmup { "warmup" } else { "measured" },
        outcome: "completed",
        grade_passed: true,
        failed_check_count: 0,
        termination: "exited",
        exit_code: Some(0),
        signal: None,
        elapsed_ns: 0,
        spawn_retry_count: 0,
        metric_sample_count: 0,
        metric_available_count: 0,
        metric_unavailable_count: 0,
        metric_scheduled_collections: Some(0),
        metric_completed_collections: Some(0),
        metric_lost_collections: Some(0),
        metric_collection_time_ns: Some(0),
        metric_max_collection_time_ns: Some(0),
        metric_requested_ids: Some(Vec::new()),
        metric_collected_ids: Some(Vec::new()),
        metric_unavailable_ids: Some(Vec::new()),
        metric_omitted_ids: Some(Vec::new()),
        stdout_bytes: 0,
        stderr_bytes: 0,
        output_truncated: false,
    }
}

#[allow(clippy::too_many_arguments)]
fn run_attempt(
    plan: &PlanFile,
    work_root: &Path,
    run_id: &str,
    input_id: u32,
    warmup: bool,
    launch: Option<&ProviderLaunchRecord>,
    measurement_selection: &MeasurementSelectionV1,
    cancelled: Arc<AtomicBool>,
    live_provider: bool,
    live_attempt: Option<LiveProviderAttempt>,
) -> Result<Option<AttemptSummary>, CliError> {
    if cancelled.load(Ordering::SeqCst) {
        return Ok(None);
    }
    let phase = if warmup { "warmup" } else { "measured" };
    let attempt_root = work_root.join(format!("{run_id}-{phase}-{input_id}"));
    let mut prepared = PreparedGuard::new(
        prepare_workload(&plan.workload, &attempt_root)
            .map_err(|_| CliError::operation("workload preparation failed"))?,
    );
    let private = attempt_root.join(".asb-private");
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&private)
        .map_err(|_| CliError::operation("private attempt state cannot be created"))?;
    let prompt_path = private.join("prompt");
    let mut prompt = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&prompt_path)
        .map_err(|_| CliError::operation("prompt descriptor cannot be created"))?;
    fs::remove_file(&prompt_path)
        .map_err(|_| CliError::operation("prompt descriptor cannot be unlinked"))?;
    prompt
        .write_all(prepared.prompt().as_bytes())
        .and_then(|()| prompt.flush())
        .and_then(|()| prompt.seek(SeekFrom::Start(0)).map(|_| ()))
        .map_err(|_| CliError::operation("prompt descriptor cannot be prepared"))?;
    let home = private.join("home");
    let temporary = private.join("tmp");
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&home)
        .and_then(|()| fs::DirBuilder::new().mode(0o700).create(&temporary))
        .map_err(|_| CliError::operation("private agent directories cannot be created"))?;
    if cancelled.load(Ordering::SeqCst) {
        return Ok(None);
    }
    let agent_snapshot = snapshot_agent(&plan.agent, &private)?;
    if cancelled.load(Ordering::SeqCst) {
        return Ok(None);
    }
    let limits = process_limits(plan.point)?;
    let (mut process, spawn_retry_count) = spawn_verified_agent(
        plan,
        &prepared.workspace(),
        &home,
        &temporary,
        &agent_snapshot,
        &prompt,
        limits,
        launch,
        live_provider,
        live_attempt,
    )?;
    let collector = LinuxCollector::host();
    let selected_ids = measurement_selection
        .selected_ids
        .iter()
        .cloned()
        .map(Id)
        .collect::<Vec<_>>();
    let mut metric = collector
        .collect_process_selected(process.pid(), 0, &selected_ids)
        .map_err(|_| CliError::validation("validated process measurement selection drifted"))?;
    let measurement_interval_ns = measurement_selection.sample_interval_ns;
    let mut next_measurement_slot = 1_u64;
    let mut metric_scheduled_collections = u64::from(measurement_interval_ns.is_some());
    let mut metric_completed_collections = metric_scheduled_collections;
    let mut metric_lost_collections = 0_u64;
    let mut metric_collection_time_ns = if measurement_interval_ns.is_some() {
        metric.evidence().collection_time_ns()
    } else {
        0
    };
    let mut metric_max_collection_time_ns = metric_collection_time_ns;
    let mut collected_metric_ids = BTreeSet::new();
    let mut unavailable_metric_ids = BTreeSet::new();
    for sample in metric.samples() {
        match sample.value {
            asb_protocol::MetricValue::Available { .. } => {
                collected_metric_ids.insert(sample.descriptor.metric_id.0.clone());
            }
            asb_protocol::MetricValue::Unavailable { .. } => {
                unavailable_metric_ids.insert(sample.descriptor.metric_id.0.clone());
            }
        }
    }
    let process_started = Instant::now();
    while !process
        .leader_has_exited()
        .map_err(|_| CliError::operation("agent process cannot be observed"))?
    {
        if cancelled.load(Ordering::SeqCst) {
            process
                .cancel()
                .map_err(|_| CliError::operation("agent process cannot be cancelled"))?;
            break;
        }
        if process_started.elapsed() >= Duration::from_millis(plan.point.timeout_ms) {
            break;
        }
        thread::sleep(Duration::from_millis(plan.point.poll_ms));
        if !process
            .leader_has_exited()
            .map_err(|_| CliError::operation("agent process cannot be observed"))?
        {
            let elapsed = process_started.elapsed();
            let elapsed_ns = u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX);
            if measurement_interval_ns.is_some_and(|interval| {
                elapsed_ns >= interval.saturating_mul(next_measurement_slot)
            }) {
                let interval = measurement_interval_ns.expect("validated non-empty cadence");
                let due_slot = elapsed_ns / interval;
                let due_collections = due_slot
                    .saturating_sub(next_measurement_slot)
                    .saturating_add(1);
                metric_scheduled_collections =
                    metric_scheduled_collections.saturating_add(due_collections);
                metric_lost_collections =
                    metric_lost_collections.saturating_add(due_collections.saturating_sub(1));
                metric = collector
                    .collect_process_selected(process.pid(), elapsed_ns, &selected_ids)
                    .map_err(|_| {
                        CliError::validation("validated process measurement selection drifted")
                    })?;
                metric_completed_collections = metric_completed_collections.saturating_add(1);
                metric_collection_time_ns = metric_collection_time_ns
                    .saturating_add(metric.evidence().collection_time_ns());
                metric_max_collection_time_ns =
                    metric_max_collection_time_ns.max(metric.evidence().collection_time_ns());
                for sample in metric.samples() {
                    match sample.value {
                        asb_protocol::MetricValue::Available { .. } => {
                            collected_metric_ids.insert(sample.descriptor.metric_id.0.clone());
                        }
                        asb_protocol::MetricValue::Unavailable { .. } => {
                            unavailable_metric_ids.insert(sample.descriptor.metric_id.0.clone());
                        }
                    }
                }
                next_measurement_slot = due_slot.saturating_add(1);
            }
        }
    }
    let evidence = process
        .wait()
        .map_err(|_| CliError::operation("agent process did not yield terminal evidence"))?;
    let grade = prepared
        .evaluate()
        .map_err(|_| CliError::operation("workload evaluation failed"))?;
    let termination = termination_name(evidence.termination);
    let outcome = match evidence.termination {
        Termination::Cancelled => "cancelled",
        Termination::TimedOut => "timed_out",
        Termination::Exited if evidence.exit_code == Some(0) && grade.passed => "completed",
        Termination::Exited => "failed",
    };
    let observed_ids = collected_metric_ids
        .iter()
        .chain(&unavailable_metric_ids)
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let metric_omitted_ids = measurement_selection
        .selected_ids
        .iter()
        .filter(|id| !observed_ids.contains(id.as_str()))
        .cloned()
        .collect();
    let summary = AttemptSummary {
        input_id,
        phase,
        outcome,
        grade_passed: grade.passed,
        failed_check_count: grade.failed_check_count,
        termination,
        exit_code: evidence.exit_code,
        signal: evidence.signal,
        elapsed_ns: u64::try_from(evidence.elapsed.as_nanos()).unwrap_or(u64::MAX),
        spawn_retry_count,
        metric_sample_count: metric.samples().len(),
        metric_available_count: metric.evidence().available_values(),
        metric_unavailable_count: metric.evidence().unavailable_values(),
        metric_scheduled_collections: (plan.schema_version == PLAN_SCHEMA_VERSION)
            .then_some(metric_scheduled_collections),
        metric_completed_collections: (plan.schema_version == PLAN_SCHEMA_VERSION)
            .then_some(metric_completed_collections),
        metric_lost_collections: (plan.schema_version == PLAN_SCHEMA_VERSION)
            .then_some(metric_lost_collections),
        metric_collection_time_ns: (plan.schema_version == PLAN_SCHEMA_VERSION)
            .then_some(metric_collection_time_ns),
        metric_max_collection_time_ns: (plan.schema_version == PLAN_SCHEMA_VERSION)
            .then_some(metric_max_collection_time_ns),
        metric_requested_ids: (plan.schema_version == PLAN_SCHEMA_VERSION)
            .then_some(measurement_selection.selected_ids.clone()),
        metric_collected_ids: (plan.schema_version == PLAN_SCHEMA_VERSION)
            .then_some(collected_metric_ids.into_iter().collect()),
        metric_unavailable_ids: (plan.schema_version == PLAN_SCHEMA_VERSION)
            .then_some(unavailable_metric_ids.into_iter().collect()),
        metric_omitted_ids: (plan.schema_version == PLAN_SCHEMA_VERSION)
            .then_some(metric_omitted_ids),
        stdout_bytes: evidence.stdout.total_bytes,
        stderr_bytes: evidence.stderr.total_bytes,
        output_truncated: evidence.stdout.truncated || evidence.stderr.truncated,
    };
    prepared.cleanup()?;
    Ok(Some(summary))
}

#[allow(clippy::too_many_arguments)]
fn spawn_verified_agent(
    plan: &PlanFile,
    workspace: &Path,
    home: &Path,
    temporary: &Path,
    agent_snapshot: &Path,
    prompt: &fs::File,
    limits: ProcessLimits,
    launch: Option<&ProviderLaunchRecord>,
    live_provider: bool,
    live_attempt: Option<LiveProviderAttempt>,
) -> Result<(AgentProcess, u8), CliError> {
    if live_provider {
        let mut attempt = live_attempt.ok_or_else(|| {
            CliError::validation("live provider requires a runtime-issued attempt")
        })?;
        let process = attempt
            .spawn()
            .map_err(|_| CliError::operation("runtime live-provider spawn failed"))?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| CliError::operation("runtime clock is unavailable"))?
            .as_millis() as u64;
        let relay_worker = attempt
            .start_relay(now)
            .map_err(|_| CliError::operation("runtime live relay cannot start"))?;
        return Ok((
            AgentProcess::Live {
                process,
                _attempt: attempt,
                relay_worker: Some(relay_worker),
            },
            0,
        ));
    }
    if let Some(launch) = launch {
        launch
            .validate()
            .map_err(|_| CliError::validation("provider-aware launch changed before spawn"))?;
        if launch.input.runtime.executable_sha256 != plan.agent.executable_sha256 {
            return Err(CliError::validation(
                "provider-aware executable identity changed before spawn",
            ));
        }
    }
    for retry in 0..=MAX_BUSY_SPAWN_RETRIES {
        let stdin = prompt
            .try_clone()
            .map_err(|_| CliError::operation("prompt descriptor cannot be duplicated"))?;
        let mut command = Command::new(agent_snapshot);
        command
            .args(&plan.agent.arguments)
            .current_dir(workspace)
            .env_clear()
            .env("HOME", home)
            .env("TMPDIR", temporary)
            .env("PATH", "/usr/bin:/bin")
            .env("ASB_BATCH_PROTOCOL", "batch-stdio-v1")
            .stdin(Stdio::from(stdin));
        if let Some(launch) = launch {
            let credential_target =
                credential_target_for_provider_agent(&launch.input.provider, &launch.input.adapter)
                    .ok_or_else(|| {
                        CliError::validation("provider adapter credential target is unsupported")
                    })?;
            command
                .env(provider_launch::LAUNCH_VERSION_ENV, "1")
                .env(provider_launch::LAUNCH_DIGEST_ENV, &launch.launch_sha256)
                .env(provider_launch::PROVIDER_ENV, &launch.input.provider)
                .env(provider_launch::ADAPTER_ENV, &launch.input.adapter)
                .env(provider_launch::MODEL_ENV, &launch.input.model)
                .env(
                    provider_launch::API_MODE_ENV,
                    format!("{:?}", launch.input.api_mode).to_lowercase(),
                )
                .env(
                    provider_launch::PROFILE_DIGEST_ENV,
                    &launch.input.provider_profile_sha256,
                )
                .env(
                    provider_launch::SETTINGS_DIGEST_ENV,
                    &launch.input.settings_sha256,
                )
                .env(
                    provider_launch::ENDPOINT_DIGEST_ENV,
                    &launch.input.endpoint_sha256,
                )
                .env(
                    provider_launch::CREDENTIAL_REFERENCE_ENV,
                    &launch.input.credential.reference_sha256,
                )
                .env(provider_launch::CREDENTIAL_TARGET_ENV, credential_target);
        }
        match RunningProcess::spawn(command, limits) {
            Ok(process) => return Ok((AgentProcess::Direct(process), retry)),
            Err(ProcessError::Spawn(error)) if should_retry_busy(error.raw_os_error(), retry) => {
                thread::sleep(Duration::from_millis(1));
            }
            Err(error) => return Err(spawn_error(error)),
        }
    }
    unreachable!("the bounded retry loop returns on its final iteration")
}

#[allow(clippy::large_enum_variant)]
enum AgentProcess {
    Direct(RunningProcess),
    Live {
        process: SandboxProcess,
        _attempt: LiveProviderAttempt,
        relay_worker:
            Option<std::thread::JoinHandle<Result<(), asb_runtime::live_relay::LiveRelayError>>>,
    },
}

impl AgentProcess {
    fn pid(&self) -> u32 {
        match self {
            Self::Direct(process) => process.pid(),
            Self::Live { process, .. } => process.pid(),
        }
    }
    fn leader_has_exited(&self) -> Result<bool, CliError> {
        match self {
            Self::Direct(process) => process
                .leader_has_exited()
                .map_err(|_| CliError::operation("agent process cannot be observed")),
            Self::Live { process, .. } => process
                .leader_has_exited()
                .map_err(|_| CliError::operation("agent process cannot be observed")),
        }
    }
    fn cancel(&mut self) -> Result<(), CliError> {
        match self {
            Self::Direct(process) => process
                .cancel()
                .map_err(|_| CliError::operation("agent process cannot be cancelled")),
            Self::Live {
                process,
                _attempt,
                relay_worker,
            } => {
                _attempt.revoke();
                process
                    .cancel()
                    .map_err(|_| CliError::operation("agent process cannot be cancelled"))?;
                if let Some(worker) = relay_worker.take() {
                    worker
                        .join()
                        .map_err(|_| CliError::operation("live relay worker panicked"))?
                        .map_err(|_| CliError::operation("live relay forwarding failed"))?;
                }
                Ok(())
            }
        }
    }
    fn wait(&mut self) -> Result<asb_runtime::ProcessOutput, CliError> {
        match self {
            Self::Direct(process) => process
                .wait()
                .cloned()
                .map_err(|_| CliError::operation("agent process did not yield terminal evidence")),
            Self::Live {
                process,
                _attempt,
                relay_worker,
            } => {
                let result = process.wait().map_err(|_| {
                    CliError::operation("agent process did not yield terminal evidence")
                });
                _attempt.revoke();
                if let Some(worker) = relay_worker.take() {
                    worker
                        .join()
                        .map_err(|_| CliError::operation("live relay worker panicked"))?
                        .map_err(|_| CliError::operation("live relay forwarding failed"))?;
                }
                result
            }
        }
    }
}

fn should_retry_busy(raw_os_error: Option<i32>, retries_completed: u8) -> bool {
    raw_os_error == Some(libc::ETXTBSY) && retries_completed < MAX_BUSY_SPAWN_RETRIES
}

fn spawn_error(error: ProcessError) -> CliError {
    match error {
        ProcessError::Spawn(error) if error.raw_os_error() == Some(libc::ETXTBSY) => {
            CliError::operation("agent snapshot is unexpectedly busy")
        }
        ProcessError::Spawn(error) if error.raw_os_error() == Some(libc::EAGAIN) => {
            CliError::operation("agent process limit prevented start")
        }
        ProcessError::Spawn(error) if error.raw_os_error() == Some(libc::EACCES) => {
            CliError::operation("agent snapshot execution was denied")
        }
        _ => CliError::operation("agent process cannot be started"),
    }
}

fn snapshot_agent(agent: &BatchAgent, private: &Path) -> Result<PathBuf, CliError> {
    let mut source = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&agent.executable)
        .map_err(|_| CliError::operation("agent executable cannot be reopened safely"))?;
    let metadata = source
        .metadata()
        .map_err(|_| CliError::operation("agent executable metadata cannot be verified"))?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_EXECUTABLE_BYTES {
        return Err(CliError::operation(
            "agent executable changed after validation",
        ));
    }
    let snapshot = private.join("agent-snapshot");
    let mut destination = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&snapshot)
        .map_err(|_| CliError::operation("agent snapshot cannot be created"))?;
    let mut digest = Sha256::new();
    let mut copied = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = source
            .read(&mut buffer)
            .map_err(|_| CliError::operation("agent executable cannot be snapshotted"))?;
        if count == 0 {
            break;
        }
        copied = copied
            .checked_add(count as u64)
            .ok_or_else(|| CliError::operation("agent executable size overflow"))?;
        if copied > MAX_EXECUTABLE_BYTES {
            return Err(CliError::operation(
                "agent executable changed after validation",
            ));
        }
        digest.update(&buffer[..count]);
        destination
            .write_all(&buffer[..count])
            .map_err(|_| CliError::operation("agent snapshot cannot be written"))?;
    }
    destination
        .sync_all()
        .map_err(|_| CliError::operation("agent snapshot cannot be synchronized"))?;
    if copied != metadata.len() || format!("{:x}", digest.finalize()) != agent.executable_sha256 {
        return Err(CliError::operation(
            "agent executable changed after validation",
        ));
    }
    drop(destination);
    drop(source);
    fs::set_permissions(&snapshot, fs::Permissions::from_mode(0o500))
        .map_err(|_| CliError::operation("agent snapshot cannot be made executable"))?;
    Ok(snapshot)
}

fn append_state(
    store: &AtomicStore,
    run_id: &str,
    attempt_id: &str,
    sequence: u64,
    started: Instant,
    state: ExecutionState,
    evidence: Value,
) -> Result<(), CliError> {
    store
        .append(
            run_id,
            &JournalEvent {
                schema_version: JOURNAL_SCHEMA_VERSION,
                sequence,
                attempt_id: Id(attempt_id.to_owned()),
                monotonic_offset_ns: u64::try_from(started.elapsed().as_nanos())
                    .unwrap_or(u64::MAX),
                state,
                evidence,
            },
        )
        .map_err(|_| CliError::operation("run state cannot be committed"))
}

fn prepare_root(path: &Path) -> Result<(), CliError> {
    if path.exists() {
        return Ok(());
    }
    fs::DirBuilder::new()
        .mode(0o700)
        .create(path)
        .map_err(|_| CliError::operation("run root cannot be created"))
}

fn termination_name(value: Termination) -> &'static str {
    match value {
        Termination::Exited => "exited",
        Termination::Cancelled => "cancelled",
        Termination::TimedOut => "timed_out",
    }
}

fn stop_reason_name(value: StopReason) -> &'static str {
    match value {
        StopReason::Completed => "completed",
        StopReason::FailureLimit => "failure_limit",
        StopReason::DurationLimit => "duration_limit",
        StopReason::Contaminated => "contaminated",
    }
}

fn attempt_outcome_name(value: AttemptOutcome) -> &'static str {
    match value {
        AttemptOutcome::Completed => "completed",
        AttemptOutcome::Failed => "failed",
        AttemptOutcome::TimedOut => "timed_out",
        AttemptOutcome::Cancelled => "cancelled",
        AttemptOutcome::InfrastructureFailure => "infrastructure_failure",
    }
}

fn miss_reason_name(value: MissReason) -> String {
    match value {
        MissReason::Backpressure => "backpressure".to_owned(),
        MissReason::PointStopped(reason) => format!("point_stopped:{}", stop_reason_name(reason)),
    }
}

fn duration_ns(value: Duration) -> u64 {
    u64::try_from(value.as_nanos()).unwrap_or(u64::MAX)
}

#[derive(Serialize)]
struct ComparisonOutput {
    schema_version: u16,
    ok: bool,
    command: &'static str,
    comparable: bool,
    differences: Vec<String>,
}

fn compare(runs: &[String], output: &mut dyn Write) -> Result<(), CliError> {
    let manifests = runs
        .iter()
        .map(|path| load_run_definition(Path::new(path)))
        .collect::<Result<Vec<_>, _>>()?;
    let baseline = &manifests[0];
    let mut differences = Vec::new();
    for candidate in &manifests[1..] {
        let report = compare_experiments(&baseline.experiment, &candidate.experiment)
            .map_err(|_| CliError::validation("stored experiments cannot be compared"))?;
        for field in report.differences() {
            let name = comparison_field_name(*field).to_owned();
            if !differences.contains(&name) {
                differences.push(name);
            }
        }
        if baseline.execution.measurement_selection_sha256
            != candidate.execution.measurement_selection_sha256
            && !differences
                .iter()
                .any(|name| name == "measurement_selection")
        {
            differences.push("measurement_selection".to_owned());
        }
        if baseline.execution != candidate.execution
            && !differences.iter().any(|name| name == "execution")
        {
            differences.push("execution".to_owned());
        }
    }
    write_json(
        output,
        &ComparisonOutput {
            schema_version: OUTPUT_SCHEMA_VERSION,
            ok: true,
            command: "compare",
            comparable: differences.is_empty(),
            differences,
        },
    )
}

fn load_run_definition(path: &Path) -> Result<StoredRunDefinition, CliError> {
    let (store_root, run_id) = parse_run_ref(path)?;
    let store = AtomicStore::open(store_root, StoreLimits::default())
        .map_err(|_| CliError::validation("run store cannot be opened"))?;
    let manifest = store
        .load_manifest(run_id)
        .map_err(|_| CliError::validation("run manifest cannot be loaded"))?;
    let definition: StoredRunDefinition = serde_json::from_value(manifest.definition)
        .map_err(|_| CliError::validation("run definition has an invalid shape"))?;
    validate_stored_definition(&definition)?;
    Ok(definition)
}

fn parse_run_ref(path: &Path) -> Result<(&Path, &str), CliError> {
    if !path.is_absolute() || fs::canonicalize(path).ok().as_deref() != Some(path) {
        return Err(CliError::validation(
            "run reference must be an exact absolute directory",
        ));
    }
    let run_id = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| CliError::validation("run reference identity is invalid"))?;
    validate_id(run_id)?;
    let runs = path
        .parent()
        .filter(|value| value.file_name().is_some_and(|name| name == "runs"))
        .ok_or_else(|| CliError::validation("run reference is outside a store"))?;
    let root = runs
        .parent()
        .ok_or_else(|| CliError::validation("run reference has no store root"))?;
    Ok((root, run_id))
}

fn comparison_field_name(value: ComparisonField) -> &'static str {
    match value {
        ComparisonField::Agent => "agent",
        ComparisonField::Model => "model",
        ComparisonField::ModelSettings => "model_settings",
        ComparisonField::ToolPolicy => "tool_policy",
        ComparisonField::Workload => "workload",
        ComparisonField::Scorer => "scorer",
        ComparisonField::Image => "image",
        ComparisonField::Dependencies => "dependencies",
        ComparisonField::Kernel => "kernel",
        ComparisonField::Distribution => "distribution",
        ComparisonField::Architecture => "architecture",
        ComparisonField::CpuModel => "cpu_model",
        ComparisonField::LogicalCpuCount => "logical_cpu_count",
        ComparisonField::NumaNodeCount => "numa_node_count",
        ComparisonField::ScalingGovernor => "scaling_governor",
        ComparisonField::CacheState => "cache_state",
        ComparisonField::LoadPolicy => "load_policy",
        ComparisonField::RetrySettings => "retry_settings",
        ComparisonField::ReplayMode => "replay_mode",
        ComparisonField::ReplayCassette => "replay_cassette",
        ComparisonField::ReplayPacing => "replay_pacing",
        ComparisonField::CancellationTimeout => "cancellation_timeout",
    }
}

#[derive(Serialize)]
struct ReportOutput {
    schema_version: u16,
    ok: bool,
    command: &'static str,
    runs: Vec<ReportRun>,
}

#[derive(Serialize)]
struct ReportRun {
    run_id: String,
    attempt_id: String,
    event_count: usize,
    terminal_state: Option<&'static str>,
    execution_sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    measurement_catalog_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    measurement_selection_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    measurement_mode: Option<asb_protocol::ReplayMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    measurement_sample_interval_ns: Option<Option<u64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    measurement_selected_ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider_selection_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider_profile_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider_launch_sha256: Option<String>,
    point: Option<Value>,
}

fn report(runs: &[String], output: &mut dyn Write) -> Result<(), CliError> {
    let mut reports = Vec::with_capacity(runs.len());
    for path in runs {
        let (root, run_id) = parse_run_ref(Path::new(path))?;
        let store = AtomicStore::open(root, StoreLimits::default())
            .map_err(|_| CliError::validation("run store cannot be opened"))?;
        let manifest = store
            .load_manifest(run_id)
            .map_err(|_| CliError::validation("run manifest cannot be loaded"))?;
        let journal = store
            .load_journal(run_id)
            .map_err(|_| CliError::validation("run journal cannot be loaded"))?;
        let definition: StoredRunDefinition = serde_json::from_value(manifest.definition)
            .map_err(|_| CliError::validation("run definition has an invalid shape"))?;
        validate_stored_definition(&definition)?;
        let point = validate_terminal_point(journal.last(), &definition)?;
        let exposes_measurement_selection =
            definition.execution.plan_schema_version == PLAN_SCHEMA_VERSION;
        let public_measurement_selection = if exposes_measurement_selection {
            definition.measurement_selection.as_ref()
        } else {
            None
        };
        reports.push(ReportRun {
            run_id: manifest.run_id.0,
            attempt_id: manifest.attempt_id.0,
            event_count: journal.len(),
            terminal_state: journal.last().map(|event| state_name(event.state)),
            execution_sha256: definition.execution_sha256,
            measurement_catalog_sha256: public_measurement_selection
                .map(|value| value.catalog_sha256.clone()),
            measurement_selection_sha256: public_measurement_selection
                .map(|value| value.selection_sha256.clone()),
            measurement_mode: public_measurement_selection.map(|value| value.mode),
            measurement_sample_interval_ns: public_measurement_selection
                .map(|value| value.sample_interval_ns),
            measurement_selected_ids: public_measurement_selection
                .map(|value| value.selected_ids.clone()),
            provider_selection_sha256: definition
                .provider_selection
                .as_ref()
                .map(|value| value.selection_sha256.clone()),
            provider_profile_sha256: definition
                .provider_selection
                .as_ref()
                .map(|value| value.provider_profile_sha256.clone()),
            provider_launch_sha256: definition
                .provider_launch
                .as_ref()
                .map(|value| value.launch_sha256.clone()),
            point,
        });
    }
    write_json(
        output,
        &ReportOutput {
            schema_version: OUTPUT_SCHEMA_VERSION,
            ok: true,
            command: "report",
            runs: reports,
        },
    )
}

fn validate_terminal_point(
    event: Option<&JournalEvent>,
    definition: &StoredRunDefinition,
) -> Result<Option<Value>, CliError> {
    let Some(event) = event else {
        return Ok(None);
    };
    if !matches!(
        event.state,
        ExecutionState::Completed | ExecutionState::Failed | ExecutionState::Cancelled
    ) {
        return Ok(None);
    }
    let evidence = event
        .evidence
        .as_object()
        .filter(|object| object.len() == 2)
        .ok_or_else(|| CliError::validation("terminal point evidence has an invalid shape"))?;
    if evidence.get("execution_sha256").and_then(Value::as_str)
        != Some(&definition.execution_sha256)
    {
        return Err(CliError::validation(
            "terminal point evidence does not match execution",
        ));
    }
    let point = evidence
        .get("point")
        .and_then(Value::as_object)
        .ok_or_else(|| CliError::validation("terminal point evidence has an invalid shape"))?;
    const KEYS: &[&str] = &[
        "concurrency",
        "decision",
        "stop_reason",
        "admission_stop_reason",
        "admitted",
        "missed",
        "completed",
        "failed",
        "timed_out",
        "cancelled",
        "infrastructure_failures",
        "attempts",
        "scheduler_attempts",
        "attempt_failures",
    ];
    if point.len() != KEYS.len() || KEYS.iter().any(|key| !point.contains_key(*key)) {
        return Err(CliError::validation(
            "terminal point evidence has an invalid shape",
        ));
    }
    let concurrency = point
        .get("concurrency")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok());
    let decision = point.get("decision").and_then(Value::as_str);
    let valid_terminal = matches!(
        (event.state, decision),
        (ExecutionState::Completed, Some("pass"))
            | (ExecutionState::Failed, Some("fail" | "inconclusive"))
            | (ExecutionState::Cancelled, Some("fail" | "inconclusive"))
    );
    let total = definition
        .execution
        .requested_point
        .measured
        .checked_add(definition.execution.requested_point.warmups)
        .ok_or_else(|| CliError::validation("terminal point evidence count overflow"))?
        as usize;
    let attempts = point.get("attempts").and_then(Value::as_array);
    let scheduler = point.get("scheduler_attempts").and_then(Value::as_array);
    let failures = point.get("attempt_failures").and_then(Value::as_array);
    let numeric = [
        "admitted",
        "missed",
        "completed",
        "failed",
        "timed_out",
        "cancelled",
        "infrastructure_failures",
    ];
    let expected_measurement_selection =
        if definition.execution.plan_schema_version == PLAN_SCHEMA_VERSION {
            definition.measurement_selection.as_ref()
        } else {
            None
        };
    if concurrency != Some(definition.execution.executed_concurrency)
        || !valid_terminal
        || !matches!(
            point.get("stop_reason").and_then(Value::as_str),
            Some("completed" | "failure_limit" | "duration_limit" | "contaminated")
        )
        || !matches!(
            point.get("admission_stop_reason").and_then(Value::as_str),
            Some("completed" | "failure_limit" | "duration_limit" | "contaminated")
        )
        || numeric.iter().any(|key| {
            point
                .get(*key)
                .and_then(Value::as_u64)
                .is_none_or(|value| value > total as u64)
        })
        || attempts.is_none_or(|items| items.len() > total)
        || scheduler.is_none_or(|items| items.len() != total)
        || failures.is_none_or(|items| items.len() > total)
        || attempts.is_some_and(|items| {
            items.iter().any(|item| {
                !valid_attempt_evidence(
                    item,
                    expected_measurement_selection,
                    definition.execution.requested_point.timeout_ms,
                )
            })
        })
        || scheduler.is_some_and(|items| items.iter().any(|item| !valid_scheduler_evidence(item)))
        || failures.is_some_and(|items| items.iter().any(|item| !valid_failure_evidence(item)))
        || attempts
            .zip(failures)
            .is_none_or(|(attempts, failures)| attempts.len() + failures.len() > total)
    {
        return Err(CliError::validation(
            "terminal point evidence is inconsistent",
        ));
    }
    if !valid_point_relationships(point, definition) {
        return Err(CliError::validation(
            "terminal point evidence relationships are inconsistent",
        ));
    }
    Ok(evidence.get("point").cloned())
}

fn evidence_identity(value: &Value) -> Option<(bool, u32)> {
    let object = value.as_object()?;
    let warmup = match object.get("phase")?.as_str()? {
        "warmup" => true,
        "measured" => false,
        _ => return None,
    };
    let input_id = u32::try_from(object.get("input_id")?.as_u64()?).ok()?;
    Some((warmup, input_id))
}

fn valid_point_relationships(point: &Map<String, Value>, definition: &StoredRunDefinition) -> bool {
    let Some(scheduler) = point.get("scheduler_attempts").and_then(Value::as_array) else {
        return false;
    };
    let mut scheduler_by_id = BTreeMap::new();
    for item in scheduler {
        let Some(identity @ (warmup, input_id)) = evidence_identity(item) else {
            return false;
        };
        let limit = if warmup {
            definition.execution.requested_point.warmups
        } else {
            definition.execution.requested_point.measured
        };
        if input_id >= limit || scheduler_by_id.insert(identity, item).is_some() {
            return false;
        }
        let Some(object) = item.as_object() else {
            return false;
        };
        let missed = object.get("missed").and_then(Value::as_bool) == Some(true);
        let started = object.get("started_at_ns").and_then(Value::as_u64);
        let finished = object.get("finished_at_ns").and_then(Value::as_u64);
        let outcome = object.get("outcome").and_then(Value::as_str);
        let miss_reason = object.get("miss_reason").and_then(Value::as_str);
        let scheduled = object.get("scheduled_at_ns").and_then(Value::as_u64);
        let queue_delay = object.get("queue_delay_ns").and_then(Value::as_u64);
        if missed {
            if started.is_some()
                || finished.is_some()
                || outcome.is_some()
                || miss_reason.is_none()
                || queue_delay != Some(0)
            {
                return false;
            }
        } else {
            if outcome.is_none() || miss_reason.is_some() {
                return false;
            }
            match (scheduled, started, finished) {
                (Some(scheduled), Some(started), Some(finished)) => {
                    if started < scheduled
                        || finished < started
                        || queue_delay != Some(started - scheduled)
                    {
                        return false;
                    }
                }
                (Some(_), None, None)
                    if outcome == Some("infrastructure_failure") && queue_delay == Some(0) => {}
                _ => {
                    return false;
                }
            }
        }
    }
    if scheduler_by_id.len()
        != (definition.execution.requested_point.warmups
            + definition.execution.requested_point.measured) as usize
    {
        return false;
    }

    let Some(attempts) = point.get("attempts").and_then(Value::as_array) else {
        return false;
    };
    let mut attempts_by_id = BTreeMap::new();
    for item in attempts {
        let Some(identity) = evidence_identity(item) else {
            return false;
        };
        if !scheduler_by_id.contains_key(&identity)
            || attempts_by_id.insert(identity, item).is_some()
        {
            return false;
        }
        let Some(object) = item.as_object() else {
            return false;
        };
        let outcome = object.get("outcome").and_then(Value::as_str);
        let termination = object.get("termination").and_then(Value::as_str);
        let completed = termination == Some("exited")
            && object.get("exit_code").and_then(Value::as_i64) == Some(0)
            && object.get("grade_passed").and_then(Value::as_bool) == Some(true);
        if !matches!(
            (outcome, termination),
            (Some("completed"), Some("exited"))
                | (Some("failed"), Some("exited"))
                | (Some("timed_out"), Some("timed_out"))
                | (Some("cancelled"), Some("cancelled"))
        ) || (outcome == Some("completed")) != completed
        {
            return false;
        }
    }
    let Some(failures) = point.get("attempt_failures").and_then(Value::as_array) else {
        return false;
    };
    let mut failures_by_id = BTreeMap::new();
    for item in failures {
        let Some(identity) = evidence_identity(item) else {
            return false;
        };
        if !scheduler_by_id.contains_key(&identity)
            || failures_by_id.insert(identity, item).is_some()
        {
            return false;
        }
    }

    for (identity, scheduler) in &scheduler_by_id {
        let object = scheduler.as_object().expect("shape was validated");
        let outcome = object.get("outcome").and_then(Value::as_str);
        let attempt = attempts_by_id.get(identity);
        let failure = failures_by_id.get(identity);
        if outcome.is_none() {
            if attempt.is_some() || failure.is_some() {
                return false;
            }
        } else if outcome == Some("infrastructure_failure") {
            if attempt.is_some() || failure.is_none() {
                return false;
            }
        } else if failure.is_some()
            || attempt
                .is_none_or(|attempt| attempt.get("outcome").and_then(Value::as_str) != outcome)
        {
            return false;
        }
    }

    let measured = scheduler_by_id.iter().filter(|((warmup, _), _)| !warmup);
    let count = |expected: Option<&str>| {
        measured
            .clone()
            .filter(|(_, item)| item.get("outcome").and_then(Value::as_str) == expected)
            .count() as u64
    };
    let admitted = measured
        .clone()
        .filter(|(_, item)| item.get("outcome").is_some_and(|value| !value.is_null()))
        .count() as u64;
    let missed = measured
        .clone()
        .filter(|(_, item)| item.get("missed").and_then(Value::as_bool) == Some(true))
        .count() as u64;
    let completed = count(Some("completed"));
    let failed = count(Some("failed"));
    let timed_out = count(Some("timed_out"));
    let cancelled = count(Some("cancelled"));
    let infrastructure_failures = count(Some("infrastructure_failure"));
    for (key, expected) in [
        ("admitted", admitted),
        ("missed", missed),
        ("completed", completed),
        ("failed", failed),
        ("timed_out", timed_out),
        ("cancelled", cancelled),
        ("infrastructure_failures", infrastructure_failures),
    ] {
        if point.get(key).and_then(Value::as_u64) != Some(expected) {
            return false;
        }
    }
    let stop_reason = point.get("stop_reason").and_then(Value::as_str);
    let expected_decision = if infrastructure_failures > 0 || stop_reason == Some("contaminated") {
        "inconclusive"
    } else if missed == 0
        && failed == 0
        && timed_out == 0
        && cancelled == 0
        && completed == u64::from(definition.execution.requested_point.measured)
    {
        "pass"
    } else {
        "fail"
    };
    point.get("decision").and_then(Value::as_str) == Some(expected_decision)
}

fn exact_keys(object: &Map<String, Value>, keys: &[&str]) -> bool {
    object.len() == keys.len() && keys.iter().all(|key| object.contains_key(*key))
}

fn optional_i64(value: Option<&Value>) -> bool {
    value.is_some_and(|value| value.is_null() || value.as_i64().is_some())
}

fn valid_measurement_evidence_relationships(
    object: &Map<String, Value>,
    selection: &MeasurementSelectionV1,
    timeout_ms: u64,
) -> bool {
    let ids = |key| {
        object
            .get(key)
            .and_then(Value::as_array)
            .and_then(|values| {
                let ids = values
                    .iter()
                    .map(Value::as_str)
                    .collect::<Option<Vec<_>>>()?;
                (ids.len() <= asb_protocol::MAX_MEASUREMENTS
                    && ids.windows(2).all(|pair| pair[0] < pair[1]))
                .then_some(ids)
            })
    };
    let Some(requested) = ids("metric_requested_ids") else {
        return false;
    };
    let Some(collected) = ids("metric_collected_ids") else {
        return false;
    };
    let Some(unavailable) = ids("metric_unavailable_ids") else {
        return false;
    };
    let Some(omitted) = ids("metric_omitted_ids") else {
        return false;
    };
    if requested
        != selection
            .selected_ids
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
    {
        return false;
    }
    let requested = requested.into_iter().collect::<BTreeSet<_>>();
    let collected = collected.into_iter().collect::<BTreeSet<_>>();
    let unavailable = unavailable.into_iter().collect::<BTreeSet<_>>();
    let omitted = omitted.into_iter().collect::<BTreeSet<_>>();
    let expected_omitted = requested
        .difference(&collected.union(&unavailable).copied().collect())
        .copied()
        .collect::<BTreeSet<_>>();
    let Some(scheduled) = object["metric_scheduled_collections"].as_u64() else {
        return false;
    };
    let Some(completed) = object["metric_completed_collections"].as_u64() else {
        return false;
    };
    let Some(lost) = object["metric_lost_collections"].as_u64() else {
        return false;
    };
    let Some(expected_total) = completed.checked_add(lost) else {
        return false;
    };
    let Some(total_collection_time_ns) = object["metric_collection_time_ns"].as_u64() else {
        return false;
    };
    let Some(max_collection_time_ns) = object["metric_max_collection_time_ns"].as_u64() else {
        return false;
    };
    let Some(maximum_scheduled) = selection.sample_interval_ns.map_or(Some(0), |interval_ns| {
        timeout_ms
            .checked_mul(1_000_000)
            .and_then(|duration_ns| duration_ns.checked_div(interval_ns))
            .and_then(|slots| slots.checked_add(2))
    }) else {
        return false;
    };
    let within_value_bound = scheduled
        .checked_mul(u64::try_from(requested.len()).unwrap_or(u64::MAX))
        .is_some_and(|values| values <= MAX_OPTIONAL_METRIC_VALUES_PER_ATTEMPT);
    let sample_counts_match = object["metric_available_count"]
        .as_u64()
        .zip(object["metric_unavailable_count"].as_u64())
        .and_then(|(available, unavailable)| available.checked_add(unavailable))
        .zip(object["metric_sample_count"].as_u64())
        .is_some_and(|(classified, samples)| {
            classified == samples && samples <= requested.len() as u64
        });
    collected.is_subset(&requested)
        && unavailable.is_subset(&requested)
        && omitted == expected_omitted
        && scheduled == expected_total
        && scheduled <= maximum_scheduled
        && within_value_bound
        && sample_counts_match
        && max_collection_time_ns <= total_collection_time_ns
        && (completed > 0 || (total_collection_time_ns == 0 && max_collection_time_ns == 0))
        && (requested.is_empty()
            == (scheduled == 0
                && completed == 0
                && lost == 0
                && total_collection_time_ns == 0
                && max_collection_time_ns == 0))
}

fn valid_attempt_evidence(
    value: &Value,
    measurement_selection: Option<&MeasurementSelectionV1>,
    timeout_ms: u64,
) -> bool {
    const LEGACY_KEYS: &[&str] = &[
        "input_id",
        "phase",
        "outcome",
        "grade_passed",
        "failed_check_count",
        "termination",
        "exit_code",
        "signal",
        "elapsed_ns",
        "spawn_retry_count",
        "metric_sample_count",
        "metric_available_count",
        "metric_unavailable_count",
        "stdout_bytes",
        "stderr_bytes",
        "output_truncated",
    ];
    const SELECTION_KEYS: &[&str] = &[
        "input_id",
        "phase",
        "outcome",
        "grade_passed",
        "failed_check_count",
        "termination",
        "exit_code",
        "signal",
        "elapsed_ns",
        "spawn_retry_count",
        "metric_sample_count",
        "metric_available_count",
        "metric_unavailable_count",
        "metric_scheduled_collections",
        "metric_completed_collections",
        "metric_lost_collections",
        "metric_collection_time_ns",
        "metric_max_collection_time_ns",
        "metric_requested_ids",
        "metric_collected_ids",
        "metric_unavailable_ids",
        "metric_omitted_ids",
        "stdout_bytes",
        "stderr_bytes",
        "output_truncated",
    ];
    let expected_keys = if measurement_selection.is_some() {
        SELECTION_KEYS
    } else {
        LEGACY_KEYS
    };
    let Some(object) = value
        .as_object()
        .filter(|object| exact_keys(object, expected_keys))
    else {
        return false;
    };
    object
        .get("input_id")
        .and_then(Value::as_u64)
        .is_some_and(|value| value <= u32::MAX as u64)
        && matches!(
            object.get("phase").and_then(Value::as_str),
            Some("warmup" | "measured")
        )
        && matches!(
            object.get("outcome").and_then(Value::as_str),
            Some("completed" | "failed" | "timed_out" | "cancelled")
        )
        && object
            .get("grade_passed")
            .and_then(Value::as_bool)
            .is_some()
        && matches!(
            object.get("termination").and_then(Value::as_str),
            Some("exited" | "timed_out" | "cancelled")
        )
        && optional_i64(object.get("exit_code"))
        && optional_i64(object.get("signal"))
        && object
            .get("spawn_retry_count")
            .and_then(Value::as_u64)
            .is_some_and(|value| value <= MAX_BUSY_SPAWN_RETRIES as u64)
        && [
            "failed_check_count",
            "elapsed_ns",
            "metric_sample_count",
            "metric_available_count",
            "metric_unavailable_count",
            "stdout_bytes",
            "stderr_bytes",
        ]
        .iter()
        .all(|key| object.get(*key).and_then(Value::as_u64).is_some())
        && object
            .get("output_truncated")
            .and_then(Value::as_bool)
            .is_some()
        && measurement_selection.is_none_or(|selection| {
            valid_measurement_evidence_relationships(object, selection, timeout_ms)
        })
}

fn valid_scheduler_evidence(value: &Value) -> bool {
    const KEYS: &[&str] = &[
        "input_id",
        "phase",
        "scheduled_at_ns",
        "started_at_ns",
        "finished_at_ns",
        "outcome",
        "queue_delay_ns",
        "missed",
        "miss_reason",
    ];
    let Some(object) = value.as_object().filter(|object| exact_keys(object, KEYS)) else {
        return false;
    };
    let optional_u64 = |key| {
        object
            .get(key)
            .is_some_and(|value| value.is_null() || value.as_u64().is_some())
    };
    object
        .get("input_id")
        .and_then(Value::as_u64)
        .is_some_and(|value| value <= u32::MAX as u64)
        && matches!(
            object.get("phase").and_then(Value::as_str),
            Some("warmup" | "measured")
        )
        && object
            .get("scheduled_at_ns")
            .and_then(Value::as_u64)
            .is_some()
        && optional_u64("started_at_ns")
        && optional_u64("finished_at_ns")
        && object.get("outcome").is_some_and(|value| {
            value.is_null()
                || matches!(
                    value.as_str(),
                    Some(
                        "completed"
                            | "failed"
                            | "timed_out"
                            | "cancelled"
                            | "infrastructure_failure"
                    )
                )
        })
        && object
            .get("queue_delay_ns")
            .and_then(Value::as_u64)
            .is_some()
        && object.get("missed").and_then(Value::as_bool).is_some()
        && object.get("miss_reason").is_some_and(|value| {
            value.is_null()
                || matches!(
                    value.as_str(),
                    Some(
                        "backpressure"
                            | "point_stopped:completed"
                            | "point_stopped:failure_limit"
                            | "point_stopped:duration_limit"
                            | "point_stopped:contaminated"
                    )
                )
        })
}

fn valid_failure_evidence(value: &Value) -> bool {
    const KEYS: &[&str] = &["input_id", "phase", "code", "message"];
    let Some(object) = value.as_object().filter(|object| exact_keys(object, KEYS)) else {
        return false;
    };
    object
        .get("input_id")
        .and_then(Value::as_u64)
        .is_some_and(|value| value <= u32::MAX as u64)
        && matches!(
            object.get("phase").and_then(Value::as_str),
            Some("warmup" | "measured")
        )
        && object.get("code").and_then(Value::as_str) == Some("operation")
        && object
            .get("message")
            .and_then(Value::as_str)
            .is_some_and(|message| !message.is_empty() && message.len() <= 128)
}

fn state_name(value: ExecutionState) -> &'static str {
    match value {
        ExecutionState::Planned => "planned",
        ExecutionState::Prepared => "prepared",
        ExecutionState::Running => "running",
        ExecutionState::Collecting => "collecting",
        ExecutionState::Completed => "completed",
        ExecutionState::Failed => "failed",
        ExecutionState::Cancelled => "cancelled",
    }
}

#[derive(Serialize)]
struct ErrorEnvelope {
    schema_version: u16,
    ok: bool,
    command: &'static str,
    error: CliError,
}

#[derive(Clone, Copy, Debug, Serialize)]
struct CliError {
    code: &'static str,
    message: &'static str,
    exit_code: u8,
    #[serde(skip)]
    settings_issue: asb_control::SettingsIssue,
}

impl CliError {
    const fn usage(message: &'static str) -> Self {
        Self {
            code: "usage",
            message,
            exit_code: 2,
            settings_issue: asb_control::SettingsIssue::InvalidFormat,
        }
    }

    const fn validation(message: &'static str) -> Self {
        Self {
            code: "validation",
            message,
            exit_code: 3,
            settings_issue: asb_control::SettingsIssue::InvalidFormat,
        }
    }

    const fn validation_issue(
        message: &'static str,
        settings_issue: asb_control::SettingsIssue,
    ) -> Self {
        Self {
            code: "validation",
            message,
            exit_code: 3,
            settings_issue,
        }
    }

    const fn operation(message: &'static str) -> Self {
        Self {
            code: "operation",
            message,
            exit_code: 4,
            settings_issue: asb_control::SettingsIssue::InvalidFormat,
        }
    }
}

fn output_error(_: io::Error) -> CliError {
    CliError::operation("structured output cannot be written")
}

fn write_json(output: &mut dyn Write, value: &impl Serialize) -> Result<(), CliError> {
    serde_json::to_writer(&mut *output, value)
        .map_err(|_| CliError::operation("structured output cannot be encoded"))?;
    writeln!(output).map_err(output_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::AtomicU64;

    static NONCE: AtomicU64 = AtomicU64::new(0);

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "asb-cli-{name}-{}-{}",
                std::process::id(),
                NONCE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&path)
                .unwrap();
            Self(fs::canonicalize(path).unwrap())
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn executable(root: &Path) -> (PathBuf, String) {
        let path = root.join("fixture-agent");
        fs::write(
            &path,
            b"#!/bin/sh\ntest ! -e ../.asb-private/prompt || exit 97\nif [ \"${ASB_PROVIDER_LAUNCH_V1:-}\" = 1 ]; then\n  test \"${ASB_PROVIDER_LAUNCH_SHA256:-}\" != \"\" || exit 98\n  test \"${ASB_PROVIDER:-}\" = openai || test \"${ASB_PROVIDER:-}\" = openrouter || exit 99\n  test \"${ASB_PROVIDER_MODEL:-}\" != \"\" || exit 100\n  test \"${ASB_PROVIDER_PROFILE_SHA256:-}\" != \"\" || exit 101\n  test \"${ASB_PROVIDER_CREDENTIAL_TARGET:-}\" = CODEX_API_KEY || test \"${ASB_PROVIDER_CREDENTIAL_TARGET:-}\" = OPENROUTER_API_KEY || exit 102\nfi\nprintf '%s\\n' 'def parse_line(line):' '    if line.endswith(\"\\r\"):' '        line = line[:-1]' '    return line' > parser.py\n",
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        let path = fs::canonicalize(path).unwrap();
        let digest = digest_file(&path).unwrap();
        (path, digest)
    }

    fn experiment(binary_sha256: &str) -> ExperimentManifestV1 {
        let mut value: ExperimentManifestV1 = serde_json::from_str(include_str!(
            "../../asb-protocol/fixtures/v1/experiment-manifest.json"
        ))
        .unwrap();
        let workload = OriginalWorkloads::describe("original.bug-fix").unwrap();
        value.agent.binary_sha256 = binary_sha256.to_owned();
        value.workload.workload = workload.workload_id.0;
        value.workload.workload_revision = workload.version;
        value.workload.workload_sha256 = workload.content_sha256;
        value.workload.scorer_revision = workload.scoring_version;
        value.platform.architecture = std::env::consts::ARCH.to_owned();
        // Generic command fixtures exercise the provider/live execution path.
        // Replay plans now require an explicit runtime-issued cassette authority;
        // dedicated replay tests construct that mode deliberately.
        value.controls.replay.mode = asb_protocol::ReplayMode::Live;
        value.controls.replay.cassette_sha256 = None;
        value.refresh_content_address().unwrap();
        value
    }

    #[test]
    fn cli_dispatch_rejects_methodology_only_plan_before_execution() {
        let scratch = Scratch::new("methodology-dispatch");
        let (_, mut plan) = plan_fixture(&scratch.0, "methodology");
        plan.workload = "agentops".into();
        let error = validate_plan(&plan).expect_err("methodology-only workload must fail closed");
        assert_eq!(error.message, "unknown or invalid workload");
    }

    fn plan_fixture(root: &Path, run_id: &str) -> (PathBuf, PlanFile) {
        let (executable, executable_sha256) = executable(root);
        let experiment = experiment(&executable_sha256);
        let measurement_selection =
            process_measurement_selection(experiment.controls.replay.mode, 2_000_000).unwrap();
        let plan = PlanFile {
            schema_version: PLAN_SCHEMA_VERSION,
            run_id: run_id.to_owned(),
            result_root: root.join("results"),
            work_root: root.join("work"),
            workload: "original.bug-fix".into(),
            agent: BatchAgent {
                executable,
                executable_sha256: executable_sha256.clone(),
                arguments: Vec::new(),
            },
            point: PointInput {
                measured: 1,
                warmups: 0,
                concurrency: 1,
                queue: 0,
                max_failures: 0,
                timeout_ms: 5_000,
                poll_ms: 2,
                seed: 7,
                open_loop_interval_ms: None,
                sweep_max_concurrency: Some(2),
            },
            experiment,
            measurement_selection: Some(measurement_selection),
        };
        let path = root.join(format!("{run_id}.toml"));
        fs::write(&path, toml::to_string(&plan).unwrap()).unwrap();
        (path, plan)
    }

    #[test]
    fn help_doctor_and_usage_are_stable_and_escape_free() {
        let mut output = Vec::new();
        let mut diagnostic = Vec::new();
        assert_eq!(run(&[], &mut output, &mut diagnostic), 0);
        assert!(!output.contains(&0x1b));
        output.clear();
        assert_eq!(run(&["doctor".into()], &mut output, &mut diagnostic), 0);
        let doctor: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(doctor["schema_version"], OUTPUT_SCHEMA_VERSION);
        assert_eq!(doctor["command"], "doctor");
        assert!(!output.contains(&0x1b));
        output.clear();
        assert_eq!(run(&["unknown".into()], &mut output, &mut diagnostic), 2);
        let error: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(error["error"]["code"], "usage");
    }

    #[test]
    fn guided_local_requires_explicit_mock_and_config_flags() {
        let missing_mock = vec![
            "run".into(),
            "experiment.toml".into(),
            "--use-config".into(),
            "--live-provider".into(),
        ];
        assert!(guided_local(&missing_mock, &mut Vec::new(), &mut Vec::new()).is_err());

        let unknown = vec![
            "sweep".into(),
            "experiment.toml".into(),
            "--use-config".into(),
            "--local-mock".into(),
            "--endpoint".into(),
        ];
        assert!(guided_local(&unknown, &mut Vec::new(), &mut Vec::new()).is_err());

        let absent_config = vec![
            "run".into(),
            "experiment.toml".into(),
            "--use-config".into(),
            "--local-mock".into(),
        ];
        let result = guided_local(&absent_config, &mut Vec::new(), &mut Vec::new());
        assert!(result.is_err());
    }

    #[test]
    fn guided_local_delegates_catalog_bound_run_to_mock() {
        let scratch = Scratch::new("guided-local-positive");
        let store = ConfigStore::new(scratch.0.join("config.json"));
        configure_openrouter_at(&store, &mut Vec::new()).unwrap();
        let (plan_path, mut fixture) = plan_fixture(&scratch.0, "guided-local");
        fixture.experiment.agent.implementation = "codex".into();
        let configured = load_openrouter_config_at(&store).unwrap();
        let configured_plan = provider_plan_from_selection(&configured, "codex").unwrap();
        fixture.experiment.model.provider = "openrouter".into();
        fixture.experiment.model.model = asb_agents::openrouter::OPENROUTER_MODEL.into();
        fixture.experiment.model.settings.additional_settings_sha256 =
            Some(configured_plan.provider_profile_sha256);
        fixture.experiment.refresh_content_address().unwrap();
        fs::write(&plan_path, toml::to_string(&fixture).unwrap()).unwrap();

        let args = vec![
            "run".into(),
            plan_path.to_string_lossy().into_owned(),
            "--use-config".into(),
            "--local-mock".into(),
        ];
        let mut output = Vec::new();
        assert_eq!(
            guided_local_at(&args, &store, &mut output, &mut Vec::new()).unwrap(),
            0
        );
        let result: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(result["ok"], true);
        assert_eq!(result["points"][0]["decision"], "pass");
        assert!(result["provider_launch_sha256"].is_string());
    }

    #[test]
    fn setup_preflight_is_machine_readable_and_side_effect_free() {
        let mut output = Vec::new();
        assert_eq!(run(&["setup".into()], &mut output, &mut Vec::new()), 0);
        let value: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(value["command"], "setup");
        assert_eq!(value["mode"], "preflight");
        assert_eq!(value["persistent_change"], false);
        assert_eq!(value["provider_contact"], false);
        assert_eq!(value["steps"].as_array().unwrap().len(), 4);
    }

    #[test]
    fn setup_rejects_unknown_options() {
        let mut output = Vec::new();
        assert_eq!(
            run(
                &["setup".into(), "--unknown".into()],
                &mut output,
                &mut Vec::new()
            ),
            2
        );
        let value: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(value["error"]["code"], "usage");
    }

    #[test]
    fn setup_requires_complete_provider_selection_and_does_not_commit_on_error() {
        let scratch = Scratch::new("setup");
        let destination = scratch.0.join("config.json");
        let args: Vec<OsString> = [
            "setup",
            "--provider-profile",
            "openai",
            "--output",
            destination.to_str().unwrap(),
        ]
        .into_iter()
        .map(OsString::from)
        .collect();
        let mut output = Vec::new();
        assert_ne!(run(&args, &mut output, &mut Vec::new()), 0);
        assert!(!destination.exists());
        let value: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(value["error"]["code"], "validation");
    }

    #[test]
    fn setup_commits_only_validated_selection_atomically() {
        let scratch = Scratch::new("setup-valid");
        let destination = scratch.0.join("config.json");
        let args: Vec<OsString> = [
            "setup",
            "--provider-profile",
            "openai",
            "--model",
            "gpt-4o-mini",
            "--output",
            destination.to_str().unwrap(),
        ]
        .into_iter()
        .map(OsString::from)
        .collect();
        let mut output = Vec::new();
        assert_eq!(run(&args, &mut output, &mut Vec::new()), 0);
        let persisted: Value = serde_json::from_slice(&fs::read(&destination).unwrap()).unwrap();
        assert_eq!(persisted["mode"], "commit");
        assert_eq!(persisted["provider_profile"], "openai");
        assert_eq!(persisted["model"], "gpt-4o-mini");
    }

    #[test]
    fn openrouter_config_enrollment_loads_and_plans_without_a_secret() {
        let scratch = Scratch::new("openrouter-config");
        let store = ConfigStore::new(scratch.0.join("config.json"));
        let mut output = Vec::new();
        configure_openrouter_at(&store, &mut output).unwrap();
        let result: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(result["command"], "config-openrouter");
        assert_eq!(result["credential_environment"], "OPENROUTER_API_KEY");
        assert!(!String::from_utf8_lossy(&fs::read(store.path()).unwrap()).contains("secret"));
        let loaded = load_openrouter_config_at(&store).unwrap();
        assert_eq!(loaded.enrollment.generation, 1);
        let plan = provider_plan_from_selection(&loaded, "codex").unwrap();
        assert_eq!(plan.provider_profile, "openrouter");
        assert_eq!(plan.model, asb_agents::openrouter::OPENROUTER_MODEL);
        let mut second = Vec::new();
        configure_openrouter_at(&store, &mut second).unwrap();
        assert_eq!(
            load_openrouter_config_at(&store)
                .unwrap()
                .enrollment
                .generation,
            2
        );
    }

    #[test]
    fn openrouter_config_load_rejects_absent_stale_and_unknown_agents() {
        let scratch = Scratch::new("openrouter-config-negative");
        let store = ConfigStore::new(scratch.0.join("config.json"));
        assert!(load_openrouter_config_at(&store).is_err());
        let mut output = Vec::new();
        configure_openrouter_at(&store, &mut output).unwrap();
        let mut config = store.load().unwrap().unwrap();
        config
            .openrouter_free_model
            .as_mut()
            .unwrap()
            .endpoint_identity_sha256 = "e".repeat(64);
        fs::write(store.path(), serde_json::to_vec(&config).unwrap()).unwrap();
        fs::set_permissions(store.path(), fs::Permissions::from_mode(0o600)).unwrap();
        assert!(load_openrouter_config_at(&store).is_err());
        let valid = OpenRouterFreeModelConfig::enroll(
            asb_agents::openrouter::OPENROUTER_MODEL.into(),
            asb_agents::openrouter::OPENROUTER_MODEL_SNAPSHOT_DATE.into(),
            "f".repeat(64),
            1,
        )
        .unwrap();
        assert!(provider_plan_from_selection(&valid, "unknown-agent").is_err());
    }

    #[test]
    fn provider_plan_config_dispatch_covers_valid_duplicate_and_mismatch_options() {
        let scratch = Scratch::new("provider-plan-config");
        let store = ConfigStore::new(scratch.0.join("config.json"));
        configure_openrouter_at(&store, &mut Vec::new()).unwrap();
        let selection = load_openrouter_config_at(&store).unwrap();
        let catalog = provider_catalog_digest();
        let valid = vec![
            "--use-config".into(),
            "--catalog-sha256".into(),
            catalog.clone(),
            "--agent".into(),
            "codex".into(),
        ];
        let mut output = Vec::new();
        provider_plan_at(&valid, &mut output, &store).unwrap();
        let value: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(value["provider_profile"], "openrouter");
        assert_eq!(value["model"], asb_agents::openrouter::OPENROUTER_MODEL);

        let duplicate = vec![
            "--use-config".into(),
            "--use-config".into(),
            "--catalog-sha256".into(),
            catalog.clone(),
        ];
        assert!(provider_plan_at(&duplicate, &mut Vec::new(), &store).is_err());
        let mismatch = vec![
            "--use-config".into(),
            "--catalog-sha256".into(),
            catalog.clone(),
            "--provider-profile".into(),
            "openai".into(),
        ];
        assert!(provider_plan_at(&mismatch, &mut Vec::new(), &store).is_err());
        let credential_mismatch = vec![
            "--use-config".into(),
            "--catalog-sha256".into(),
            catalog,
            "--credential-reference-sha256".into(),
            "a".repeat(64),
        ];
        assert!(provider_plan_at(&credential_mismatch, &mut Vec::new(), &store).is_err());
        assert_eq!(selection.provider, "openrouter");
    }

    #[test]
    fn plan_and_run_config_sources_bind_selection_before_execution() {
        let scratch = Scratch::new("plan-config-source");
        let store = ConfigStore::new(scratch.0.join("config.json"));
        configure_openrouter_at(&store, &mut Vec::new()).unwrap();
        let (plan_path, mut fixture) = plan_fixture(&scratch.0, "config-source");
        fixture.experiment.agent.implementation = "codex".into();
        let configured = load_openrouter_config_at(&store).unwrap();
        let configured_plan = provider_plan_from_selection(&configured, "codex").unwrap();
        fixture.experiment.model.provider = "openrouter".into();
        fixture.experiment.model.model = asb_agents::openrouter::OPENROUTER_MODEL.into();
        fixture.experiment.model.settings.additional_settings_sha256 =
            Some(configured_plan.provider_profile_sha256.clone());
        fixture.experiment.refresh_content_address().unwrap();
        fs::write(&plan_path, toml::to_string(&fixture).unwrap()).unwrap();
        let mut planned = Vec::new();
        plan_with_config_at(&plan_path, &mut planned, &store).unwrap();
        let plan: Value = serde_json::from_slice(&planned).unwrap();
        assert_eq!(plan["provider_profile"], "openrouter");
        assert_eq!(
            plan["provider_model"],
            asb_agents::openrouter::OPENROUTER_MODEL
        );

        let mut local_run = Vec::new();
        let local_exit = execute_inner_from_source(
            &plan_path,
            SelectionSource::Config(&store),
            false,
            false,
            None,
            true,
            &mut local_run,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(local_exit, 0);
        let local_output: Value = serde_json::from_slice(&local_run).unwrap();
        assert_eq!(local_output["ok"], true);
        assert_eq!(local_output["points"][0]["decision"], "pass");
        assert!(local_output["provider_launch_sha256"].is_string());

        let mut sweep_plan = fixture.clone();
        sweep_plan.run_id = "config-local-sweep".into();
        sweep_plan.result_root = scratch.0.join("config-local-sweep-results");
        sweep_plan.work_root = scratch.0.join("config-local-sweep-work");
        let sweep_path = scratch.0.join("config-local-sweep.toml");
        fs::write(&sweep_path, toml::to_string(&sweep_plan).unwrap()).unwrap();
        let mut local_sweep = Vec::new();
        let sweep_exit = execute_inner_from_source(
            &sweep_path,
            SelectionSource::Config(&store),
            true,
            false,
            None,
            true,
            &mut local_sweep,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(sweep_exit, 0);
        let sweep_output: Value = serde_json::from_slice(&local_sweep).unwrap();
        assert_eq!(sweep_output["ok"], true);
        assert_eq!(sweep_output["points"].as_array().unwrap().len(), 2);

        let mut stale = store.load().unwrap().unwrap();
        stale.openrouter_free_model.as_mut().unwrap().model = "stale-model".into();
        fs::write(store.path(), serde_json::to_vec(&stale).unwrap()).unwrap();
        fs::set_permissions(store.path(), fs::Permissions::from_mode(0o600)).unwrap();
        assert!(plan_with_config_at(&plan_path, &mut Vec::new(), &store).is_err());
        assert!(
            execute_inner_from_source(
                &plan_path,
                SelectionSource::Config(&store),
                false,
                false,
                None,
                false,
                &mut Vec::new(),
                &mut Vec::new(),
            )
            .is_err()
        );
    }

    #[test]
    fn live_provider_requires_explicit_selection_before_any_run_effect() {
        let scratch = Scratch::new("live-provider-gate");
        let (plan_path, _) = plan_fixture(&scratch.0, "live-gate");
        let error = execute_inner_from_source(
            &plan_path,
            SelectionSource::Path(None),
            false,
            true,
            None,
            false,
            &mut Vec::new(),
            &mut Vec::new(),
        )
        .unwrap_err();
        assert_eq!(
            error.message,
            "--live-provider requires an explicit provider selection"
        );
        assert!(!scratch.0.join("results").exists());
    }

    #[test]
    fn live_factory_is_invoked_once_for_each_scheduler_attempt() {
        let _scratch = Scratch::new("live-factory-attempts");
        let calls = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&calls);
        let factory = LiveProviderAttemptFactory::from_fn(move |input_id, warmup| {
            observed
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push((input_id, warmup));
            Err(LaunchAuthorityError::InvalidLaunchInput)
        });
        assert!(factory.acquire(0, true).is_err());
        assert!(factory.acquire(0, false).is_err());
        assert!(factory.acquire(1, false).is_err());
        let calls = calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(calls.len(), 3);
        assert!(calls.contains(&(0, true)));
        assert!(calls.contains(&(0, false)));
        assert!(calls.contains(&(1, false)));
    }

    #[test]
    fn setup_rejects_cross_provider_model_without_contacting_provider() {
        let args: Vec<OsString> = [
            "setup",
            "--provider-profile",
            "gemini",
            "--model",
            "gpt-4o-mini",
        ]
        .into_iter()
        .map(OsString::from)
        .collect();
        let mut output = Vec::new();
        assert_ne!(run(&args, &mut output, &mut Vec::new()), 0);
        let value: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(value["error"]["code"], "validation");
        assert!(value.to_string().len() < 512);
    }

    #[test]
    fn auth_dispatch_emits_bounded_typed_request_without_secret_material() {
        let args: Vec<OsString> = [
            "auth",
            "enroll",
            "--provider",
            "gemini",
            "--endpoint-digest",
            "a".repeat(64).as_str(),
            "--credential-digest",
            "b".repeat(64).as_str(),
            "--idempotency-key",
            "enroll-1",
        ]
        .into_iter()
        .map(OsString::from)
        .collect();
        let mut output = Vec::new();
        assert_eq!(run(&args, &mut output, &mut Vec::new()), 0);
        let request: asb_control::ControlRequest = serde_json::from_slice(&output).unwrap();
        assert!(matches!(
            request.call,
            asb_control::ControlCall::AuthEnroll(_)
        ));
        assert!(!output.windows(7).any(|window| window == b"secret!"));
    }

    #[test]
    fn auth_dispatch_rejects_missing_options_without_output() {
        let args: Vec<OsString> = ["auth", "rotate", "--provider", "ollama"]
            .into_iter()
            .map(OsString::from)
            .collect();
        let mut output = Vec::new();
        assert_ne!(run(&args, &mut output, &mut Vec::new()), 0);
        let error: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(error["ok"], false);
        assert!(error.to_string().len() < 1024);
    }

    fn provider_args(catalog: &str, provider: &str, agents: &[&str]) -> Vec<OsString> {
        let mut args = vec![
            "provider-plan".into(),
            "--catalog-sha256".into(),
            catalog.into(),
            "--provider-profile".into(),
            provider.into(),
            "--credential-reference-sha256".into(),
            "a".repeat(64).into(),
        ];
        for agent in agents {
            args.push("--agent".into());
            args.push((*agent).into());
        }
        args
    }

    fn run_json(args: &[OsString]) -> (u8, Value) {
        let mut output = Vec::new();
        let mut diagnostic = Vec::new();
        let exit = run(args, &mut output, &mut diagnostic);
        assert!(diagnostic.is_empty());
        (exit, serde_json::from_slice(&output).unwrap())
    }

    fn run_json_with_progress(args: &[OsString]) -> (u8, Value) {
        let mut output = Vec::new();
        let mut diagnostic = Vec::new();
        let exit = run(args, &mut output, &mut diagnostic);
        assert!(
            String::from_utf8(diagnostic)
                .unwrap()
                .starts_with("starting ")
        );
        (exit, serde_json::from_slice(&output).unwrap())
    }

    fn provider_selection_fixture(
        root: &Path,
        name: &str,
        provider: &str,
        agents: &[&str],
    ) -> (PathBuf, Value) {
        let args = provider_args(&provider_catalog_digest(), provider, agents);
        let mut output = Vec::new();
        let mut diagnostic = Vec::new();
        assert_eq!(run(&args, &mut output, &mut diagnostic), 0);
        assert!(diagnostic.is_empty());
        let value = serde_json::from_slice(&output).unwrap();
        let path = root.join(name);
        fs::write(&path, output).unwrap();
        (path, value)
    }

    fn bind_openai_selection(plan: &mut PlanFile, selection: &Value, agent: &str) {
        bind_provider_selection(plan, selection, agent, "openai");
    }

    fn bind_provider_selection(
        plan: &mut PlanFile,
        selection: &Value,
        agent: &str,
        provider: &str,
    ) {
        let model = match provider {
            "openai" => asb_agents::openai::OPENAI_MODEL,
            "openrouter" => asb_agents::openrouter::OPENROUTER_MODEL,
            _ => unreachable!("unsupported test provider"),
        };
        plan.experiment.agent.implementation = agent.to_owned();
        plan.experiment.model.provider = provider.to_owned();
        plan.experiment.model.model = model.to_owned();
        plan.experiment.model.settings.additional_settings_sha256 = Some(
            selection["provider_profile_sha256"]
                .as_str()
                .unwrap()
                .to_owned(),
        );
        plan.experiment.refresh_content_address().unwrap();
    }

    #[test]
    fn workload_catalog_is_stable_and_marks_methodology_unavailable() {
        let (exit, catalog) = run_json(&["workload-catalog".into()]);
        assert_eq!(exit, 0);
        assert_eq!(catalog["command"], "workload-catalog");
        let entries = catalog["entries"].as_array().unwrap();
        assert!(
            entries
                .iter()
                .any(|entry| entry["id"] == "original.bug-fix")
        );
        for id in ["harbor", "inspect-ai", "hal", "agentops"] {
            let entry = entries.iter().find(|entry| entry["id"] == id).unwrap();
            assert_eq!(entry["kind"], "methodology");
            assert_eq!(entry["availability"], "unavailable");
        }
    }

    #[test]
    fn provider_catalog_and_multi_agent_plan_are_stable_and_secret_free() {
        let (exit, catalog) = run_json(&["provider-catalog".into()]);
        assert_eq!(exit, 0);
        assert_eq!(catalog["catalog_version"], PROVIDER_CATALOG_VERSION);
        assert_eq!(catalog["profiles"][0]["id"], "openai");
        assert_eq!(catalog["profiles"][0]["selectable"], true);
        assert_eq!(catalog["profiles"][1]["id"], "openrouter");
        assert_eq!(catalog["profiles"][1]["selectable"], true);
        assert_eq!(catalog["profiles"][2]["id"], "ollama");
        assert_eq!(catalog["profiles"][2]["selectable"], false);
        let catalog_sha256 = catalog["catalog_sha256"].as_str().unwrap();
        assert_eq!(catalog_sha256.len(), 64);

        let args = provider_args(catalog_sha256, "openai", &["codex", "opendesk"]);
        let (exit, plan) = run_json(&args);
        assert_eq!(exit, 0);
        assert_eq!(plan["command"], "provider-plan");
        assert_eq!(plan["dry_run"], true);
        assert_eq!(plan["catalog_sha256"], catalog_sha256);
        assert_eq!(plan["provider_profile"], "openai");
        assert_eq!(plan["model"], asb_agents::openai::OPENAI_MODEL);
        assert_eq!(plan["credential_source"], "environment");
        assert_eq!(plan["credential_reference_sha256"], "a".repeat(64));
        assert_eq!(plan["effective"][0]["agent"], "opendesk");
        assert_eq!(plan["effective"][0]["api_mode"], "chat_completions");
        assert_eq!(plan["effective"][1]["agent"], "codex");
        assert_eq!(plan["effective"][1]["api_mode"], "responses");
        assert!(
            plan["effective"]
                .as_array()
                .unwrap()
                .iter()
                .all(|item| item["profile_sha256"] == plan["provider_profile_sha256"])
        );
        assert_eq!(plan["selection_sha256"].as_str().unwrap().len(), 64);
        let encoded = serde_json::to_string(&plan).unwrap();
        assert!(!encoded.contains("api_key"));
        assert!(!encoded.contains("authorization"));
        assert_eq!(run_json(&args), (exit, plan));
    }

    #[test]
    fn openrouter_catalog_and_plan_are_stable_secret_free_and_canonical() {
        let (exit, catalog) = run_json(&["provider-catalog".into()]);
        assert_eq!(exit, 0);
        assert_eq!(catalog["profiles"][1]["id"], "openrouter");
        assert_eq!(catalog["profiles"][1]["selectable"], true);
        assert_eq!(
            catalog["profiles"][1]["model"],
            asb_agents::openrouter::OPENROUTER_MODEL
        );
        let catalog_sha256 = catalog["catalog_sha256"].as_str().unwrap();

        let args = provider_args(catalog_sha256, "openrouter", &["codex", "opendesk"]);
        let (exit, plan) = run_json(&args);
        assert_eq!(exit, 0);
        assert_eq!(plan["command"], "provider-plan");
        assert_eq!(plan["dry_run"], true);
        assert_eq!(plan["catalog_sha256"], catalog_sha256);
        assert_eq!(plan["provider_profile"], "openrouter");
        assert_eq!(plan["model"], asb_agents::openrouter::OPENROUTER_MODEL);
        assert_eq!(plan["credential_source"], "environment");
        assert_eq!(plan["credential_reference_sha256"], "a".repeat(64));
        assert_eq!(plan["effective"][0]["agent"], "opendesk");
        assert_eq!(plan["effective"][0]["api_mode"], "chat_completions");
        assert_eq!(plan["effective"][1]["agent"], "codex");
        assert_eq!(plan["effective"][1]["api_mode"], "responses");
        assert!(
            plan["effective"]
                .as_array()
                .unwrap()
                .iter()
                .all(|item| item["profile_sha256"] == plan["provider_profile_sha256"])
        );
        assert_eq!(plan["selection_sha256"].as_str().unwrap().len(), 64);
        let encoded = serde_json::to_string(&plan).unwrap();
        assert!(!encoded.contains("api_key"));
        assert!(!encoded.contains("authorization"));
        assert_eq!(run_json(&args), (exit, plan));
    }

    #[test]
    fn openrouter_selection_drives_plan_and_run_with_exact_binding() {
        let scratch = Scratch::new("openrouter-consumption");
        let (selection_path, selection) =
            provider_selection_fixture(&scratch.0, "selection.json", "openrouter", &["codex"]);
        let (plan_path, mut plan) = plan_fixture(&scratch.0, "openrouter-run");
        bind_provider_selection(&mut plan, &selection, "codex", "openrouter");
        fs::write(&plan_path, toml::to_string(&plan).unwrap()).unwrap();

        let selection_arg = selection_path.as_os_str().to_owned();
        let plan_arg = plan_path.as_os_str().to_owned();
        let (exit, rendered) = run_json(&[
            "plan".into(),
            plan_arg.clone(),
            "--provider-selection".into(),
            selection_arg.clone(),
        ]);
        assert_eq!(exit, 0, "plan failed: {rendered}");
        assert_eq!(
            rendered["provider_selection_sha256"],
            selection["selection_sha256"]
        );
        assert_eq!(rendered["provider_profile"], "openrouter");
        assert_eq!(
            rendered["provider_model"],
            asb_agents::openrouter::OPENROUTER_MODEL
        );

        let (exit, executed) = run_json_with_progress(&[
            "run".into(),
            plan_arg,
            "--provider-selection".into(),
            selection_arg,
        ]);
        assert_eq!(exit, 0, "run failed: {executed}");
        assert_eq!(executed["run_ids"][0], "openrouter-run");
        assert_eq!(
            executed["provider_launch_sha256"].as_str().unwrap().len(),
            64
        );
        assert!(plan.result_root.join("runs/openrouter-run").is_dir());
    }

    #[test]
    fn openrouter_plan_rejects_mixed_agents_and_missing_credential_reference() {
        let catalog = provider_catalog_digest();
        for agents in [
            Vec::<&str>::new(),
            vec!["codex", "codex"],
            vec!["codex", "gemini"],
        ] {
            let (exit, error) = run_json(&provider_args(&catalog, "openrouter", &agents));
            assert_eq!(exit, 3);
            assert_eq!(
                error["error"]["message"],
                "provider profile is incompatible with selected agents"
            );
        }

        let mut missing_credential = provider_args(&catalog, "openrouter", &["codex"]);
        missing_credential.splice(5..7, []);
        let (exit, error) = run_json(&missing_credential);
        assert_eq!(exit, 3);
        assert_eq!(
            error["error"]["message"],
            "credential reference identity is absent"
        );
    }

    #[test]
    fn openrouter_selection_rejects_tampered_model_and_experiment_mismatch() {
        let scratch = Scratch::new("openrouter-negative");
        let (selection_path, mut selection) =
            provider_selection_fixture(&scratch.0, "selection.json", "openrouter", &["codex"]);
        let (plan_path, mut plan) = plan_fixture(&scratch.0, "openrouter-negative");
        bind_provider_selection(&mut plan, &selection, "codex", "openrouter");
        fs::write(&plan_path, toml::to_string(&plan).unwrap()).unwrap();
        let plan_arg = plan_path.as_os_str().to_owned();

        selection["model"] = Value::String(asb_agents::openai::OPENAI_MODEL.to_owned());
        fs::write(&selection_path, serde_json::to_vec(&selection).unwrap()).unwrap();
        let (exit, error) = run_json(&[
            "plan".into(),
            plan_arg.clone(),
            "--provider-selection".into(),
            selection_path.as_os_str().to_owned(),
        ]);
        assert_eq!(exit, 3);
        assert_eq!(
            error["error"]["message"],
            "provider selection identity is invalid"
        );
        assert!(!plan.result_root.exists());

        let (_, clean) =
            provider_selection_fixture(&scratch.0, "clean.json", "openrouter", &["codex"]);
        fs::write(&selection_path, serde_json::to_vec(&clean).unwrap()).unwrap();
        plan.experiment.model.model = asb_agents::openai::OPENAI_MODEL.to_owned();
        plan.experiment.refresh_content_address().unwrap();
        fs::write(&plan_path, toml::to_string(&plan).unwrap()).unwrap();
        let (exit, error) = run_json(&[
            "plan".into(),
            plan_arg,
            "--provider-selection".into(),
            selection_path.as_os_str().to_owned(),
        ]);
        assert_eq!(exit, 3);
        assert_eq!(
            error["error"]["message"],
            "provider selection does not match the experiment identity"
        );
        assert!(!plan.result_root.exists());
    }

    #[test]
    fn provider_plan_rejects_stale_unknown_duplicate_and_incompatible_input() {
        let catalog = provider_catalog_digest();
        for (agents, expected_message) in [
            (
                Vec::<&str>::new(),
                "provider profile is incompatible with selected agents",
            ),
            (
                vec!["codex", "codex"],
                "provider profile is incompatible with selected agents",
            ),
            (
                vec!["codex", "gemini"],
                "provider profile is incompatible with selected agents",
            ),
        ] {
            let (exit, error) = run_json(&provider_args(&catalog, "openai", &agents));
            assert_eq!(exit, 3);
            assert_eq!(error["error"]["message"], expected_message);
        }

        let mut stale = provider_args(&"0".repeat(64), "openai", &["codex"]);
        let (exit, error) = run_json(&stale);
        assert_eq!(exit, 3);
        assert_eq!(
            error["error"]["message"],
            "provider catalog identity is stale or absent"
        );
        stale[2] = catalog.clone().into();
        stale[4] = "unknown".into();
        let (exit, error) = run_json(&stale);
        assert_eq!(exit, 3);
        assert_eq!(error["error"]["message"], "unknown provider profile");

        let mut unavailable = provider_args(&catalog, "openai", &["codex"]);
        unavailable[4] = "ollama".into();
        let (exit, error) = run_json(&unavailable);
        assert_eq!(exit, 3);
        assert_eq!(
            error["error"]["message"],
            "provider profile is advertised but unavailable without verified daemon evidence"
        );

        let mut unknown_agent = provider_args(&catalog, "openai", &["codex"]);
        unknown_agent.extend(["--agent".into(), "not-an-agent".into()]);
        let (exit, error) = run_json(&unknown_agent);
        assert_eq!(exit, 3);
        assert_eq!(error["error"]["message"], "unknown selected agent");
    }

    #[test]
    fn provider_plan_bounds_options_and_has_no_filesystem_effect() {
        let scratch = Scratch::new("provider-plan");
        let before = fs::read_dir(&scratch.0).unwrap().count();
        let catalog = provider_catalog_digest();
        let mut too_many = provider_args(&catalog, "openai", &AGENT_IDS);
        too_many.extend(["--agent".into(), "codex".into()]);
        let (exit, error) = run_json(&too_many);
        assert_eq!(exit, 3);
        assert_eq!(
            error["error"]["message"],
            "selected agent set exceeds its bound"
        );

        let mut repeated = provider_args(&catalog, "openai", &["codex"]);
        repeated.extend(["--provider-profile".into(), "openai".into()]);
        let (exit, error) = run_json(&repeated);
        assert_eq!(exit, 2);
        assert_eq!(
            error["error"]["message"],
            "provider-plan option was supplied more than once"
        );

        let mut missing_value = provider_args(&catalog, "openai", &["codex"]);
        missing_value.push("--agent".into());
        let (exit, error) = run_json(&missing_value);
        assert_eq!(exit, 2);
        assert_eq!(
            error["error"]["message"],
            "provider-plan options require values"
        );
        assert_eq!(fs::read_dir(&scratch.0).unwrap().count(), before);
    }

    #[test]
    fn provider_selection_manifest_drives_plan_run_sweep_compare_and_report() {
        let scratch = Scratch::new("provider-consumption");
        let (selection_path, selection) = provider_selection_fixture(
            &scratch.0,
            "selection.json",
            "openai",
            &["codex", "opendesk"],
        );
        let (plan_path, mut plan) = plan_fixture(&scratch.0, "provider-run");
        bind_openai_selection(&mut plan, &selection, "codex");
        fs::write(&plan_path, toml::to_string(&plan).unwrap()).unwrap();

        let selection_arg = selection_path.as_os_str().to_owned();
        let plan_arg = plan_path.as_os_str().to_owned();
        let (exit, rendered) = run_json(&[
            "plan".into(),
            plan_arg.clone(),
            "--provider-selection".into(),
            selection_arg.clone(),
        ]);
        assert_eq!(exit, 0);
        assert_eq!(
            rendered["provider_selection_sha256"],
            selection["selection_sha256"]
        );
        assert_eq!(
            rendered["provider_profile_sha256"],
            selection["provider_profile_sha256"]
        );

        let (exit, executed) = run_json_with_progress(&[
            "run".into(),
            plan_arg,
            "--provider-selection".into(),
            selection_arg,
        ]);
        assert_eq!(exit, 0);
        let first_run = plan.result_root.join("runs/provider-run");
        assert_eq!(
            executed["provider_launch_sha256"].as_str().unwrap().len(),
            64
        );
        let (_, report) = run_json(&["report".into(), first_run.as_os_str().to_owned()]);
        assert_eq!(
            report["runs"][0]["provider_selection_sha256"],
            selection["selection_sha256"]
        );
        assert_eq!(
            report["runs"][0]["provider_launch_sha256"]
                .as_str()
                .unwrap()
                .len(),
            64
        );
        assert_eq!(executed["run_ids"][0], "provider-run");

        let (alternate_path, alternate) =
            provider_selection_fixture(&scratch.0, "alternate.json", "openai", &["codex", "aider"]);
        let alternate_plan_path = scratch.0.join("alternate.toml");
        plan.run_id = "provider-alternate".to_owned();
        plan.result_root = scratch.0.join("alternate-results");
        plan.work_root = scratch.0.join("alternate-work");
        bind_openai_selection(&mut plan, &alternate, "codex");
        fs::write(&alternate_plan_path, toml::to_string(&plan).unwrap()).unwrap();
        assert_eq!(
            run_json_with_progress(&[
                "run".into(),
                alternate_plan_path.as_os_str().to_owned(),
                "--provider-selection".into(),
                alternate_path.as_os_str().to_owned(),
            ])
            .0,
            0
        );
        let second_run = plan.result_root.join("runs/provider-alternate");
        let (exit, comparison) = run_json(&[
            "compare".into(),
            first_run.as_os_str().to_owned(),
            second_run.as_os_str().to_owned(),
        ]);
        assert_eq!(exit, 0);
        assert_eq!(comparison["comparable"], false);
        assert_eq!(comparison["differences"], json!(["execution"]));

        plan.run_id = "provider-sweep".to_owned();
        plan.result_root = scratch.0.join("sweep-results");
        plan.work_root = scratch.0.join("sweep-work");
        fs::write(&alternate_plan_path, toml::to_string(&plan).unwrap()).unwrap();
        let (exit, sweep) = run_json_with_progress(&[
            "sweep".into(),
            alternate_plan_path.as_os_str().to_owned(),
            "--provider-selection".into(),
            alternate_path.as_os_str().to_owned(),
        ]);
        assert_eq!(exit, 0);
        assert_eq!(sweep["points"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn provider_selection_import_fails_closed_before_run_effects() {
        let scratch = Scratch::new("provider-import-negative");
        let (selection_path, mut selection) = provider_selection_fixture(
            &scratch.0,
            "selection.json",
            "openai",
            &["codex", "opendesk"],
        );
        let (plan_path, mut plan) = plan_fixture(&scratch.0, "provider-negative");
        bind_openai_selection(&mut plan, &selection, "codex");
        fs::write(&plan_path, toml::to_string(&plan).unwrap()).unwrap();
        selection["effective"][0]["profile_sha256"] = Value::String("0".repeat(64));
        fs::write(&selection_path, serde_json::to_vec(&selection).unwrap()).unwrap();
        let (exit, error) = run_json(&[
            "run".into(),
            plan_path.as_os_str().to_owned(),
            "--provider-selection".into(),
            selection_path.as_os_str().to_owned(),
        ]);
        assert_eq!(exit, 3);
        assert_eq!(
            error["error"]["message"],
            "provider selection content address is invalid"
        );
        assert!(!plan.result_root.exists());

        let (_, clean) =
            provider_selection_fixture(&scratch.0, "clean.json", "openai", &["codex", "opendesk"]);
        fs::write(&selection_path, serde_json::to_vec(&clean).unwrap()).unwrap();
        plan.experiment.agent.implementation = "aider".to_owned();
        plan.experiment.refresh_content_address().unwrap();
        fs::write(&plan_path, toml::to_string(&plan).unwrap()).unwrap();
        assert_eq!(
            run_json(&[
                "run".into(),
                plan_path.as_os_str().to_owned(),
                "--provider-selection".into(),
                selection_path.as_os_str().to_owned(),
            ])
            .0,
            3
        );
        assert!(!plan.result_root.exists());

        bind_openai_selection(&mut plan, &clean, "codex");
        fs::write(&plan_path, toml::to_string(&plan).unwrap()).unwrap();
        let mut stale = clean.clone();
        stale["catalog_sha256"] = Value::String("0".repeat(64));
        fs::write(&selection_path, serde_json::to_vec(&stale).unwrap()).unwrap();
        assert_eq!(
            run_json(&[
                "plan".into(),
                plan_path.as_os_str().to_owned(),
                "--provider-selection".into(),
                selection_path.as_os_str().to_owned(),
            ])
            .0,
            3
        );

        let mut unknown = clean.clone();
        unknown["unexpected"] = json!(true);
        fs::write(&selection_path, serde_json::to_vec(&unknown).unwrap()).unwrap();
        assert_eq!(
            run_json(&[
                "plan".into(),
                plan_path.as_os_str().to_owned(),
                "--provider-selection".into(),
                selection_path.as_os_str().to_owned(),
            ])
            .0,
            3
        );

        let link = scratch.0.join("selection-link.json");
        std::os::unix::fs::symlink(&selection_path, &link).unwrap();
        assert_eq!(
            run_json(&[
                "plan".into(),
                plan_path.as_os_str().to_owned(),
                "--provider-selection".into(),
                link.as_os_str().to_owned(),
            ])
            .0,
            3
        );

        let oversized = scratch.0.join("oversized-selection.json");
        let file = fs::File::create(&oversized).unwrap();
        file.set_len(MAX_PROVIDER_SELECTION_BYTES + 1).unwrap();
        assert_eq!(
            run_json(&[
                "plan".into(),
                plan_path.as_os_str().to_owned(),
                "--provider-selection".into(),
                oversized.as_os_str().to_owned(),
            ])
            .0,
            3
        );
        assert!(!plan.result_root.exists());
    }

    #[test]
    fn bash_completion_is_stable_and_rejects_unknown_shells() {
        let mut output = Vec::new();
        let mut diagnostic = Vec::new();
        assert_eq!(
            run(
                &["completion".into(), "bash".into()],
                &mut output,
                &mut diagnostic
            ),
            0
        );
        let completion = String::from_utf8(output).unwrap();
        assert!(completion.contains(
            "provider-catalog workload-catalog provider-plan plan run sweep compare report"
        ));
        assert!(!completion.contains('\u{1b}'));
        assert_eq!(run_json(&["completion".into(), "zsh".into()]).0, 2);
    }

    #[test]
    fn invalid_plan_fails_before_run_roots_or_process_effects() {
        let scratch = Scratch::new("invalid");
        let (path, mut plan) = plan_fixture(&scratch.0, "invalid");
        plan.agent.executable_sha256 = "0".repeat(64);
        fs::write(&path, toml::to_string(&plan).unwrap()).unwrap();
        let mut output = Vec::new();
        let mut diagnostic = Vec::new();
        assert_eq!(
            run(
                &["run".into(), path.as_os_str().to_owned()],
                &mut output,
                &mut diagnostic,
            ),
            3
        );
        assert!(!plan.result_root.exists());
        assert!(!plan.work_root.exists());
        assert_eq!(
            serde_json::from_slice::<Value>(&output).unwrap()["error"]["code"],
            "validation"
        );
        plan.agent.executable_sha256 = digest_file(&plan.agent.executable).unwrap();
        plan.point.measured = MAX_POINT_ATTEMPTS;
        plan.point.warmups = 1;
        fs::write(&path, toml::to_string(&plan).unwrap()).unwrap();
        output.clear();
        assert_eq!(
            run(
                &["run".into(), path.as_os_str().to_owned()],
                &mut output,
                &mut diagnostic,
            ),
            3
        );
        assert!(!plan.result_root.exists());
        assert!(!plan.work_root.exists());
    }

    #[test]
    fn run_replay_mode_fails_closed_before_creating_execution_roots() {
        let scratch = Scratch::new("replay-run");
        let (path, mut plan) = plan_fixture(&scratch.0, "replay-run");
        plan.experiment.controls.replay.mode = asb_protocol::ReplayMode::Replay;
        plan.experiment.controls.replay.cassette_sha256 = Some("a".repeat(64));
        plan.experiment.refresh_content_address().unwrap();
        fs::write(&path, toml::to_string(&plan).unwrap()).unwrap();
        let mut output = Vec::new();
        let mut diagnostic = Vec::new();
        assert_eq!(
            run(
                &["run".into(), path.as_os_str().to_owned()],
                &mut output,
                &mut diagnostic,
            ),
            3
        );
        assert!(!plan.result_root.exists());
        assert!(!plan.work_root.exists());
        let error: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(error["error"]["code"], "validation");
    }

    #[test]
    fn real_run_persists_reports_and_comparison_without_raw_content() {
        let scratch = Scratch::new("run");
        let (first_path, mut first) = plan_fixture(&scratch.0, "run-one");
        first.point.measured = 2;
        first.point.warmups = 2;
        first.point.concurrency = 2;
        fs::write(&first_path, toml::to_string(&first).unwrap()).unwrap();
        let mut output = Vec::new();
        let mut diagnostic = Vec::new();
        let exit = run(
            &["run".into(), first_path.as_os_str().to_owned()],
            &mut output,
            &mut diagnostic,
        );
        assert_eq!(exit, 0, "{}", String::from_utf8_lossy(&output));
        let result: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(result["points"][0]["decision"], "pass");
        assert_eq!(result["points"][0]["completed"], 2);
        assert_eq!(result["points"][0]["attempts"][0]["phase"], "warmup");
        assert_eq!(result["points"][0]["attempts"][1]["phase"], "warmup");
        assert_eq!(result["points"][0]["attempts"][2]["phase"], "measured");
        assert_eq!(result["points"][0]["attempts"][3]["phase"], "measured");
        assert_eq!(
            result["measurement_selection_sha256"],
            first
                .measurement_selection
                .as_ref()
                .unwrap()
                .selection_sha256
        );
        let first_attempt = &result["points"][0]["attempts"][0];
        assert_eq!(
            first_attempt["metric_requested_ids"]
                .as_array()
                .unwrap()
                .len(),
            7
        );
        assert_eq!(
            first_attempt["metric_scheduled_collections"].as_u64(),
            first_attempt["metric_completed_collections"]
                .as_u64()
                .zip(first_attempt["metric_lost_collections"].as_u64())
                .map(|(completed, lost)| completed + lost)
        );
        assert_eq!(
            result["points"][0]["scheduler_attempts"]
                .as_array()
                .unwrap()
                .len(),
            4
        );
        assert!(!String::from_utf8_lossy(&output).contains("parse_line"));
        assert!(fs::read_dir(&first.work_root).unwrap().next().is_none());

        let run_ref = first.result_root.join("runs/run-one");
        output.clear();
        assert_eq!(
            run(
                &["report".into(), run_ref.as_os_str().to_owned()],
                &mut output,
                &mut diagnostic,
            ),
            0
        );
        let report: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(report["runs"][0]["terminal_state"], "completed");
        assert_eq!(report["runs"][0]["event_count"], 5);
        assert_eq!(report["runs"][0]["point"]["concurrency"], 2);
        assert_eq!(report["runs"][0]["point"]["decision"], "pass");
        assert_eq!(
            report["runs"][0]["measurement_selection_sha256"],
            first
                .measurement_selection
                .as_ref()
                .unwrap()
                .selection_sha256
        );
        assert_eq!(
            report["runs"][0]["execution_sha256"]
                .as_str()
                .unwrap()
                .len(),
            64
        );

        let second_root = scratch.0.join("second");
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&second_root)
            .unwrap();
        let second_root = fs::canonicalize(second_root).unwrap();
        let (second_path, mut second) = plan_fixture(&second_root, "run-two");
        second.measurement_selection = Some(
            MeasurementSelectionV1::new(
                &baseline_measurement_catalog(),
                Vec::new(),
                second.experiment.controls.replay.mode,
                None,
            )
            .unwrap(),
        );
        fs::write(&second_path, toml::to_string(&second).unwrap()).unwrap();
        output.clear();
        assert_eq!(
            run(
                &["run".into(), second_path.as_os_str().to_owned()],
                &mut output,
                &mut diagnostic,
            ),
            0
        );
        output.clear();
        assert_eq!(
            run(
                &[
                    "compare".into(),
                    run_ref.as_os_str().to_owned(),
                    second.result_root.join("runs/run-two").into_os_string(),
                ],
                &mut output,
                &mut diagnostic,
            ),
            0
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&output).unwrap()["comparable"],
            false
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&output).unwrap()["differences"],
            json!(["measurement_selection", "execution"])
        );
    }

    #[test]
    fn plan_bounds_and_sensitive_arguments_fail_closed() {
        assert!(validate_id("").is_err());
        assert!(validate_id("../escape").is_err());
        let scratch = Scratch::new("bounds");
        let (_, mut plan) = plan_fixture(&scratch.0, "bounded");
        plan.agent.arguments.push("--api-key=private".into());
        assert!(validate_plan(&plan).is_err());
        plan.agent.arguments.clear();
        plan.point.open_loop_interval_ms = Some(0);
        assert!(validate_plan(&plan).is_err());
        plan.point.open_loop_interval_ms = None;
        plan.point.timeout_ms = 0;
        assert!(validate_plan(&plan).is_err());
    }

    #[test]
    fn attempt_timeout_is_bounded_and_cleans_the_private_workspace() {
        let scratch = Scratch::new("timeout");
        let (_, mut plan) = plan_fixture(&scratch.0, "timeout");
        let executable = scratch.0.join("hanging-agent");
        fs::write(&executable, b"#!/bin/sh\nexec /bin/sleep 30\n").unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        plan.agent.executable = fs::canonicalize(executable).unwrap();
        plan.agent.executable_sha256 = digest_file(&plan.agent.executable).unwrap();
        plan.experiment.agent.binary_sha256 = plan.agent.executable_sha256.clone();
        plan.experiment.refresh_content_address().unwrap();
        plan.point.timeout_ms = 20;
        prepare_root(&plan.work_root).unwrap();
        let started = Instant::now();
        let summary = run_attempt(
            &plan,
            &plan.work_root,
            "timeout",
            0,
            false,
            None,
            plan.measurement_selection.as_ref().unwrap(),
            Arc::new(AtomicBool::new(false)),
            false,
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(summary.outcome, "timed_out");
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(!plan.work_root.join("timeout-measured-0").exists());
    }

    #[test]
    fn executable_replacement_after_validation_never_launches_changed_bytes() {
        let scratch = Scratch::new("replacement");
        let (_, plan) = plan_fixture(&scratch.0, "replacement");
        validate_plan(&plan).unwrap();
        let marker = scratch.0.join("changed-agent-ran");
        fs::write(
            &plan.agent.executable,
            format!("#!/bin/sh\n: > '{}'\n", marker.display()),
        )
        .unwrap();
        prepare_root(&plan.work_root).unwrap();
        let error = run_attempt(
            &plan,
            &plan.work_root,
            "replacement",
            0,
            false,
            None,
            plan.measurement_selection.as_ref().unwrap(),
            Arc::new(AtomicBool::new(false)),
            false,
            None,
        )
        .unwrap_err();
        assert_eq!(error.message, "agent executable changed after validation");
        assert!(!marker.exists());
        assert!(fs::read_dir(&plan.work_root).unwrap().next().is_none());
    }

    #[test]
    fn maximum_point_evidence_is_durable_and_report_validated() {
        let scratch = Scratch::new("maximum-evidence");
        let (_, mut plan) = plan_fixture(&scratch.0, "maximum-evidence");
        plan.point.measured = MAX_POINT_ATTEMPTS;
        plan.point.warmups = 0;
        plan.point.concurrency = 1;
        plan.point.sweep_max_concurrency = None;
        let selected_ids = plan
            .measurement_selection
            .as_ref()
            .unwrap()
            .selected_ids
            .clone();
        let selected_count = selected_ids.len();
        let maximum_scheduled = plan.point.timeout_ms * 1_000_000
            / plan
                .measurement_selection
                .as_ref()
                .unwrap()
                .sample_interval_ns
                .unwrap()
            + 2;
        let attempts = (0..MAX_POINT_ATTEMPTS)
            .map(|input_id| AttemptSummary {
                input_id,
                phase: "measured",
                outcome: "completed",
                grade_passed: true,
                failed_check_count: 0,
                termination: "exited",
                exit_code: Some(0),
                signal: None,
                elapsed_ns: u64::MAX,
                spawn_retry_count: MAX_BUSY_SPAWN_RETRIES,
                metric_sample_count: selected_count,
                metric_available_count: selected_count as u64,
                metric_unavailable_count: 0,
                metric_scheduled_collections: Some(maximum_scheduled),
                metric_completed_collections: Some(maximum_scheduled),
                metric_lost_collections: Some(0),
                metric_collection_time_ns: Some(u64::MAX),
                metric_max_collection_time_ns: Some(u64::MAX),
                metric_requested_ids: Some(selected_ids.clone()),
                metric_collected_ids: Some(selected_ids.clone()),
                metric_unavailable_ids: Some(Vec::new()),
                metric_omitted_ids: Some(Vec::new()),
                stdout_bytes: u64::MAX,
                stderr_bytes: u64::MAX,
                output_truncated: true,
            })
            .collect::<Vec<_>>();
        let scheduler_attempts = (0..MAX_POINT_ATTEMPTS)
            .map(|input_id| SchedulerAttemptEvidence {
                input_id,
                phase: "measured",
                scheduled_at_ns: u64::MAX,
                started_at_ns: Some(u64::MAX),
                finished_at_ns: Some(u64::MAX),
                outcome: Some("completed"),
                queue_delay_ns: 0,
                missed: false,
                miss_reason: None,
            })
            .collect::<Vec<_>>();
        let point = PointOutput {
            concurrency: 1,
            decision: "pass",
            stop_reason: "completed",
            admission_stop_reason: "completed",
            admitted: MAX_POINT_ATTEMPTS as usize,
            missed: 0,
            completed: MAX_POINT_ATTEMPTS as usize,
            failed: 0,
            timed_out: 0,
            cancelled: 0,
            infrastructure_failures: 0,
            attempts,
            scheduler_attempts,
            attempt_failures: Vec::new(),
        };
        let execution = execution_definition(&plan, 1, None, None);
        let execution_sha256 = execution_digest(&execution).unwrap();
        let evidence = json!({"execution_sha256": execution_sha256, "point": &point});
        assert!(
            serde_json::to_vec(&JournalEvent {
                schema_version: JOURNAL_SCHEMA_VERSION,
                sequence: 4,
                attempt_id: Id("maximum-evidence-attempt".into()),
                monotonic_offset_ns: u64::MAX,
                state: ExecutionState::Completed,
                evidence: evidence.clone(),
            })
            .unwrap()
            .len() as u64
                <= StoreLimits::default().max_event_bytes
        );
        let store = AtomicStore::open(&plan.result_root, StoreLimits::default()).unwrap();
        store
            .create_run(&RunManifest {
                schema_version: MANIFEST_SCHEMA_VERSION,
                run_id: Id(plan.run_id.clone()),
                attempt_id: Id("maximum-evidence-attempt".into()),
                definition: serde_json::to_value(StoredRunDefinition {
                    experiment: plan.experiment.clone(),
                    measurement_selection: plan.measurement_selection.clone(),
                    provider_selection: None,
                    provider_launch: None,
                    execution,
                    execution_sha256: execution_sha256.clone(),
                })
                .unwrap(),
            })
            .unwrap();
        let started = Instant::now();
        for (sequence, state) in [
            ExecutionState::Planned,
            ExecutionState::Prepared,
            ExecutionState::Running,
            ExecutionState::Collecting,
        ]
        .into_iter()
        .enumerate()
        {
            append_state(
                &store,
                &plan.run_id,
                "maximum-evidence-attempt",
                sequence as u64,
                started,
                state,
                json!({}),
            )
            .unwrap();
        }
        append_state(
            &store,
            &plan.run_id,
            "maximum-evidence-attempt",
            4,
            started,
            ExecutionState::Completed,
            evidence,
        )
        .unwrap();
        let run_ref = fs::canonicalize(plan.result_root.join("runs/maximum-evidence")).unwrap();
        let mut output = Vec::new();
        report(&[run_ref.to_string_lossy().into_owned()], &mut output).unwrap();
        let report: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(
            report["runs"][0]["point"]["scheduler_attempts"]
                .as_array()
                .unwrap()
                .len(),
            MAX_POINT_ATTEMPTS as usize
        );
        let definition = load_run_definition(&run_ref).unwrap();
        let event = store.load_journal(&plan.run_id).unwrap().pop().unwrap();
        assert!(validate_terminal_point(Some(&event), &definition).is_ok());

        let mut contradictory = event.clone();
        contradictory.evidence["point"]["admitted"] = json!(0);
        assert!(validate_terminal_point(Some(&contradictory), &definition).is_err());

        let mut duplicate_identity = event.clone();
        duplicate_identity.evidence["point"]["scheduler_attempts"][1] =
            duplicate_identity.evidence["point"]["scheduler_attempts"][0].clone();
        assert!(validate_terminal_point(Some(&duplicate_identity), &definition).is_err());

        let mut missed_with_outcome = event.clone();
        missed_with_outcome.evidence["point"]["scheduler_attempts"][0]["missed"] = json!(true);
        missed_with_outcome.evidence["point"]["scheduler_attempts"][0]["miss_reason"] =
            json!("backpressure");
        assert!(validate_terminal_point(Some(&missed_with_outcome), &definition).is_err());

        let mut mismatched_attempt = event.clone();
        mismatched_attempt.evidence["point"]["attempts"][0]["outcome"] = json!("failed");
        assert!(validate_terminal_point(Some(&mismatched_attempt), &definition).is_err());

        let miss_first_scheduler = |event: &mut JournalEvent| {
            let scheduler = &mut event.evidence["point"]["scheduler_attempts"][0];
            scheduler["started_at_ns"] = Value::Null;
            scheduler["finished_at_ns"] = Value::Null;
            scheduler["outcome"] = Value::Null;
            scheduler["queue_delay_ns"] = json!(0);
            scheduler["missed"] = json!(true);
            scheduler["miss_reason"] = json!("backpressure");
            event.evidence["point"]["admitted"] = json!(MAX_POINT_ATTEMPTS - 1);
            event.evidence["point"]["missed"] = json!(1);
            event.evidence["point"]["completed"] = json!(MAX_POINT_ATTEMPTS - 1);
            event.evidence["point"]["decision"] = json!("fail");
            event.state = ExecutionState::Failed;
        };
        let mut out_of_plan_attempt = event.clone();
        miss_first_scheduler(&mut out_of_plan_attempt);
        let attempts = out_of_plan_attempt.evidence["point"]["attempts"]
            .as_array_mut()
            .unwrap();
        let mut forged_attempt = attempts.remove(0);
        forged_attempt["input_id"] = json!(MAX_POINT_ATTEMPTS);
        attempts.push(forged_attempt);
        assert!(validate_terminal_point(Some(&out_of_plan_attempt), &definition).is_err());

        let mut out_of_plan_failure = event.clone();
        miss_first_scheduler(&mut out_of_plan_failure);
        out_of_plan_failure.evidence["point"]["attempts"]
            .as_array_mut()
            .unwrap()
            .remove(0);
        out_of_plan_failure.evidence["point"]["attempt_failures"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "input_id": MAX_POINT_ATTEMPTS,
                "phase": "measured",
                "code": "operation",
                "message": "bounded failure"
            }));
        assert!(validate_terminal_point(Some(&out_of_plan_failure), &definition).is_err());

        let mut reversed_timestamps = event.clone();
        reversed_timestamps.evidence["point"]["scheduler_attempts"][0]["started_at_ns"] = json!(0);
        reversed_timestamps.evidence["point"]["scheduler_attempts"][0]["finished_at_ns"] = json!(0);
        assert!(validate_terminal_point(Some(&reversed_timestamps), &definition).is_err());

        let mut half_timestamp = event.clone();
        half_timestamp.evidence["point"]["scheduler_attempts"][0]["outcome"] =
            json!("infrastructure_failure");
        half_timestamp.evidence["point"]["scheduler_attempts"][0]["finished_at_ns"] = Value::Null;
        half_timestamp.evidence["point"]["attempts"]
            .as_array_mut()
            .unwrap()
            .remove(0);
        half_timestamp.evidence["point"]["attempt_failures"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "input_id": 0,
                "phase": "measured",
                "code": "operation",
                "message": "bounded failure"
            }));
        half_timestamp.evidence["point"]["completed"] = json!(MAX_POINT_ATTEMPTS - 1);
        half_timestamp.evidence["point"]["infrastructure_failures"] = json!(1);
        half_timestamp.evidence["point"]["decision"] = json!("inconclusive");
        half_timestamp.evidence["point"]["stop_reason"] = json!("contaminated");
        half_timestamp.evidence["point"]["admission_stop_reason"] = json!("contaminated");
        half_timestamp.state = ExecutionState::Failed;
        assert!(validate_terminal_point(Some(&half_timestamp), &definition).is_err());

        let mut substituted_selection = event.clone();
        for key in ["metric_requested_ids", "metric_collected_ids"] {
            substituted_selection.evidence["point"]["attempts"][0][key]
                .as_array_mut()
                .unwrap()
                .pop();
        }
        assert!(validate_terminal_point(Some(&substituted_selection), &definition).is_err());

        let mut overflowing_counts = event.clone();
        let attempt = &mut overflowing_counts.evidence["point"]["attempts"][0];
        attempt["metric_scheduled_collections"] = json!(u64::MAX);
        attempt["metric_completed_collections"] = json!(u64::MAX);
        attempt["metric_lost_collections"] = json!(u64::MAX);
        assert!(validate_terminal_point(Some(&overflowing_counts), &definition).is_err());

        let mut excessive_schedule = event.clone();
        let attempt = &mut excessive_schedule.evidence["point"]["attempts"][0];
        attempt["metric_scheduled_collections"] = json!(maximum_scheduled + 1);
        attempt["metric_completed_collections"] = json!(maximum_scheduled + 1);
        attempt["metric_lost_collections"] = json!(0);
        assert!(validate_terminal_point(Some(&excessive_schedule), &definition).is_err());

        let mut missing_collection_time = event.clone();
        missing_collection_time.evidence["point"]["attempts"][0]["metric_collection_time_ns"] =
            Value::Null;
        assert!(validate_terminal_point(Some(&missing_collection_time), &definition).is_err());

        let mut wrong_execution = event;
        wrong_execution.evidence["execution_sha256"] = Value::String("0".repeat(64));
        assert!(validate_terminal_point(Some(&wrong_execution), &definition).is_err());
    }

    #[test]
    fn plan_sweep_and_failed_run_exercise_public_outcomes() {
        let scratch = Scratch::new("commands");
        let (path, mut plan) = plan_fixture(&scratch.0, "commands");
        let mut output = Vec::new();
        let mut diagnostic = Vec::new();
        assert_eq!(
            run(
                &["plan".into(), path.as_os_str().to_owned()],
                &mut output,
                &mut diagnostic,
            ),
            0
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&output).unwrap()["command"],
            "plan"
        );
        output.clear();
        assert_eq!(
            run(
                &["sweep".into(), path.as_os_str().to_owned()],
                &mut output,
                &mut diagnostic,
            ),
            0
        );
        let sweep: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(sweep["points"].as_array().unwrap().len(), 2);
        assert_eq!(sweep["highest_confirmed_capacity"], 2);

        let failing = fs::canonicalize("/bin/false").unwrap();
        plan.run_id = "failed".into();
        plan.result_root = scratch.0.join("failed-results");
        plan.work_root = scratch.0.join("failed-work");
        plan.agent.executable = failing;
        plan.agent.executable_sha256 = digest_file(&plan.agent.executable).unwrap();
        plan.experiment.agent.binary_sha256 = plan.agent.executable_sha256.clone();
        plan.experiment.refresh_content_address().unwrap();
        fs::write(&path, toml::to_string(&plan).unwrap()).unwrap();
        output.clear();
        assert_eq!(
            run(
                &["run".into(), path.as_os_str().to_owned()],
                &mut output,
                &mut diagnostic,
            ),
            5
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&output).unwrap()["points"][0]["decision"],
            "fail"
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&output).unwrap()["ok"],
            false
        );
    }

    #[test]
    fn enum_names_and_comparison_fields_are_complete() {
        assert_eq!(execution_exit_code(false, ["pass"].into_iter()), 0);
        assert_eq!(execution_exit_code(false, ["pass", "fail"].into_iter()), 5);
        assert_eq!(
            execution_exit_code(false, ["fail", "inconclusive"].into_iter()),
            6
        );
        assert_eq!(execution_exit_code(true, ["pass"].into_iter()), 130);
        assert!(should_retry_busy(Some(libc::ETXTBSY), 0));
        assert!(!should_retry_busy(
            Some(libc::ETXTBSY),
            MAX_BUSY_SPAWN_RETRIES
        ));
        assert!(!should_retry_busy(Some(libc::EACCES), 0));
        assert_eq!(termination_name(Termination::Exited), "exited");
        assert_eq!(termination_name(Termination::Cancelled), "cancelled");
        for (state, expected) in [
            (ExecutionState::Planned, "planned"),
            (ExecutionState::Prepared, "prepared"),
            (ExecutionState::Running, "running"),
            (ExecutionState::Collecting, "collecting"),
            (ExecutionState::Completed, "completed"),
            (ExecutionState::Failed, "failed"),
            (ExecutionState::Cancelled, "cancelled"),
        ] {
            assert_eq!(state_name(state), expected);
        }
        for (reason, expected) in [
            (StopReason::Completed, "completed"),
            (StopReason::FailureLimit, "failure_limit"),
            (StopReason::DurationLimit, "duration_limit"),
            (StopReason::Contaminated, "contaminated"),
        ] {
            assert_eq!(stop_reason_name(reason), expected);
        }
        assert_eq!(attempt_outcome_name(AttemptOutcome::Cancelled), "cancelled");
        assert_eq!(miss_reason_name(MissReason::Backpressure), "backpressure");
        let fields = [
            ComparisonField::Agent,
            ComparisonField::Model,
            ComparisonField::ModelSettings,
            ComparisonField::ToolPolicy,
            ComparisonField::Workload,
            ComparisonField::Scorer,
            ComparisonField::Image,
            ComparisonField::Dependencies,
            ComparisonField::Kernel,
            ComparisonField::Distribution,
            ComparisonField::Architecture,
            ComparisonField::CpuModel,
            ComparisonField::LogicalCpuCount,
            ComparisonField::NumaNodeCount,
            ComparisonField::ScalingGovernor,
            ComparisonField::CacheState,
            ComparisonField::LoadPolicy,
            ComparisonField::RetrySettings,
            ComparisonField::ReplayMode,
            ComparisonField::ReplayCassette,
            ComparisonField::ReplayPacing,
            ComparisonField::CancellationTimeout,
        ];
        let names = fields.map(comparison_field_name);
        assert_eq!(names.len(), 22);
        assert_eq!(names[0], "agent");
        assert_eq!(names[21], "cancellation_timeout");
    }

    #[test]
    fn topology_version_identity_and_output_failures_are_bounded() {
        let scratch = Scratch::new("validation");
        let (_, mut plan) = plan_fixture(&scratch.0, "validation");
        plan.schema_version = PLAN_SCHEMA_VERSION + 1;
        assert!(validate_plan(&plan).is_err());
        plan.schema_version = PLAN_SCHEMA_VERSION;
        plan.run_id = "x".repeat(MAX_ID_BYTES);
        assert!(validate_plan(&plan).is_err());
        plan.run_id = "validation".into();
        plan.work_root.clone_from(&plan.result_root);
        assert!(validate_plan(&plan).is_err());
        plan.work_root = scratch.0.join("work");
        plan.experiment.agent.binary_sha256 = "0".repeat(64);
        plan.experiment.refresh_content_address().unwrap();
        assert!(validate_plan(&plan).is_err());
        assert!(validate_root(Path::new("relative")).is_err());
        assert!(parse_run_ref(Path::new("relative")).is_err());

        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("injected"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let mut broken = Broken;
        let mut diagnostic = Vec::new();
        assert_eq!(run(&["unknown".into()], &mut broken, &mut diagnostic), 2);
        assert!(
            String::from_utf8(diagnostic)
                .unwrap()
                .contains("could not write")
        );
    }

    #[test]
    fn measurement_selection_migrates_v1_and_binds_v2_execution() {
        let scratch = Scratch::new("measurement-selection");
        let (_, mut plan) = plan_fixture(&scratch.0, "measurement-selection");
        let original = plan.measurement_selection.clone().unwrap();
        validate_plan(&plan).unwrap();
        let execution = execution_definition(&plan, 1, None, None);
        assert_eq!(
            execution.measurement_selection_sha256.as_deref(),
            Some(original.selection_sha256.as_str())
        );

        let empty = MeasurementSelectionV1::new(
            &baseline_measurement_catalog(),
            Vec::new(),
            plan.experiment.controls.replay.mode,
            None,
        )
        .unwrap();
        plan.measurement_selection = Some(empty);
        validate_plan(&plan).unwrap();
        let empty_execution = execution_definition(&plan, 1, None, None);
        assert_ne!(
            execution_digest(&execution).unwrap(),
            execution_digest(&empty_execution).unwrap()
        );

        plan.schema_version = LEGACY_PLAN_SCHEMA_VERSION;
        plan.measurement_selection = None;
        validate_plan(&plan).unwrap();
        let migrated = effective_measurement_selection(&plan).unwrap();
        assert_eq!(migrated.selected_ids.len(), 7);
        assert_eq!(migrated.sample_interval_ns, Some(2_000_000));
        assert!(
            !toml::to_string(&plan)
                .unwrap()
                .contains("measurement_selection")
        );

        plan.measurement_selection = Some(original);
        assert!(validate_plan(&plan).is_err());
    }

    #[test]
    fn measurement_selection_rejects_stale_scope_mode_and_unbounded_schedule() {
        let scratch = Scratch::new("measurement-selection-negative");
        let (_, mut plan) = plan_fixture(&scratch.0, "measurement-selection-negative");

        let mut stale = plan.measurement_selection.clone().unwrap();
        stale.catalog_sha256 = "0".repeat(64);
        plan.measurement_selection = Some(stale);
        assert!(validate_plan(&plan).is_err());

        plan.measurement_selection = Some(
            MeasurementSelectionV1::new(
                &baseline_measurement_catalog(),
                vec!["cgroup.cpu.usage".into()],
                plan.experiment.controls.replay.mode,
                Some(1_000_000),
            )
            .unwrap(),
        );
        assert!(validate_plan(&plan).is_err());

        let mismatched_mode = match plan.experiment.controls.replay.mode {
            asb_protocol::ReplayMode::Live => asb_protocol::ReplayMode::Replay,
            asb_protocol::ReplayMode::Replay => asb_protocol::ReplayMode::Live,
        };
        plan.measurement_selection =
            Some(process_measurement_selection(mismatched_mode, 2_000_000).unwrap());
        assert!(validate_plan(&plan).is_err());

        plan.measurement_selection = Some(
            process_measurement_selection(plan.experiment.controls.replay.mode, 1_000_000).unwrap(),
        );
        plan.point.timeout_ms = MAX_TIMEOUT_MS;
        assert!(validate_plan(&plan).is_err());
    }

    #[test]
    fn record_and_replay_commands_are_explicit_private_and_network_denied() {
        let scratch = Scratch::new("record-replay");
        let capture_path = scratch.0.join("capture.json");
        let cassette_path = scratch.0.join("cassette.json");
        let cassette = asb_replay::decode_cassette(
            include_bytes!("../../asb-replay/fixtures/v1/buffered.json"),
            asb_replay::CassetteLimits::default(),
        )
        .unwrap();
        let capture = asb_replay::RecordingCapture {
            schema_version: asb_replay::RECORDING_WORKFLOW_SCHEMA_VERSION,
            provider_profile_sha256: "a".repeat(64),
            agent_id: "codex".into(),
            network: asb_replay::NetworkConsequence::LoopbackOnly,
            estimated_cost_minor: 0,
            confirmation: asb_replay::RecordingConfirmation {
                record: true,
                network: true,
                cost: false,
            },
            contents: cassette.contents,
        };
        fs::write(&capture_path, serde_json::to_vec(&capture).unwrap()).unwrap();
        let mut record_output = Vec::new();
        let mut diagnostics = Vec::new();
        assert_eq!(
            run(
                &[
                    "record".into(),
                    capture_path.as_os_str().to_owned(),
                    cassette_path.as_os_str().to_owned(),
                ],
                &mut record_output,
                &mut diagnostics,
            ),
            0
        );
        assert!(cassette_path.is_file());
        let metadata: Value = serde_json::from_slice(&record_output).unwrap();
        let digest = metadata["cassette_sha256"].as_str().unwrap().to_owned();
        assert_eq!(metadata["source"], "live_recording");
        let mut replay_output = Vec::new();
        assert_eq!(
            run(
                &[
                    "replay".into(),
                    cassette_path.as_os_str().to_owned(),
                    "a".repeat(64).into(),
                    "codex".into(),
                ],
                &mut replay_output,
                &mut diagnostics,
            ),
            3
        );
        let replay: Value = serde_json::from_slice(&replay_output).unwrap();
        assert_eq!(replay["ok"], false);
        assert_eq!(
            replay["error"]["message"],
            "runtime replay authority is required"
        );
        let _ = digest;
    }

    #[test]
    fn record_campaign_requires_exact_matrix_before_offline_ready() {
        let scratch = Scratch::new("record-campaign");
        let capture_path = scratch.0.join("capture.json");
        let cassette_path = scratch.0.join("cassette.json");
        let manifest_path = scratch.0.join("campaign.json");
        let cassette = asb_replay::decode_cassette(
            include_bytes!("../../asb-replay/fixtures/v1/buffered.json"),
            asb_replay::CassetteLimits::default(),
        )
        .unwrap();
        let capture = asb_replay::RecordingCapture {
            schema_version: asb_replay::RECORDING_WORKFLOW_SCHEMA_VERSION,
            provider_profile_sha256: "a".repeat(64),
            agent_id: "codex".into(),
            network: asb_replay::NetworkConsequence::LoopbackOnly,
            estimated_cost_minor: 0,
            confirmation: asb_replay::RecordingConfirmation {
                record: true,
                network: true,
                cost: false,
            },
            contents: cassette.contents,
        };
        fs::write(&capture_path, serde_json::to_vec(&capture).unwrap()).unwrap();
        let manifest = serde_json::json!({
            "schema_version": asb_replay::RECORDING_WORKFLOW_SCHEMA_VERSION,
            "provider_profile_sha256": "a".repeat(64),
            "agent_ids": ["codex"],
            "workload_ids": ["original.bug-fix"],
            "entries": [{
                "workload_id": "original.bug-fix",
                "capture_path": capture_path,
                "cassette_path": cassette_path
            }]
        });
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let mut output = Vec::new();
        let mut diagnostics = Vec::new();
        assert_eq!(
            run(
                &[
                    "record-campaign".into(),
                    manifest_path.as_os_str().to_owned()
                ],
                &mut output,
                &mut diagnostics,
            ),
            0
        );
        let result: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(result["complete_coverage"], true);
        assert_eq!(result["offline_ready"], true);
        assert_eq!(result["tuple_count"], 1);
        assert!(cassette_path.is_file());
    }
}
