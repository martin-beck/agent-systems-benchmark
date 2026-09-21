// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Persistent local runner service behind the typed frontend control boundary.

use super::*;
use asb_control::{
    AnalysisSummary, ArtifactMetadata, ArtifactSensitivity, AuthStatusResponse, BackendFailure,
    BoundControlResult, CONTROL_MEASUREMENT_SELECTION_V1, Capabilities, ConfigurationSnapshot,
    ConfigurationStatusRequest, ControlBackend, ControlCall, ControlEvent, ControlEventKind,
    ControlLimits, ControlResult, ControlVersion, MeasurementCatalogPublication,
    MeasurementSettingsIssue, MutationAcknowledgement, Page, PlanReference, ProviderAuthMethod,
    ProviderAvailability, ProviderCatalog, ProviderCatalogAction, ProviderCatalogEntry,
    ProviderCatalogRequest, ProviderModel, ProvisionedControlServer, PublicRunState,
    RequestDeadline, Revision, RunId, RunSummary, SettingsIssue, SettingsValidation,
};
use asb_protocol::baseline_measurement_catalog;
use asb_runtime::provider_capture::{
    ProviderCapture, ProviderCaptureError, ProviderCaptureRequest, ProviderCaptureResult,
    UnavailableProviderCapture,
};
use std::collections::{BTreeMap, BTreeSet};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::sync::atomic::AtomicU64;
use std::time::{SystemTime, UNIX_EPOCH};

const CATALOG_VERSION: u16 = 1;
const MAX_CONTROL_CONFIG_BYTES: u64 = 64 * 1024;
const MAX_CATALOG_BYTES: usize = 16 * 1024 * 1024;
static CATALOG_TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn open_artifact_beneath(
    result_root: &Path,
    run_id: &str,
    digest: &str,
) -> Result<fs::File, BackendFailure> {
    let mut directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(result_root)
        .map_err(|_| BackendFailure::Rejected)?;
    for component in ["runs", run_id, "artifacts"] {
        let path =
            PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd())).join(component);
        directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)
            .map_err(|_| BackendFailure::Rejected)?;
    }
    let path = PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd())).join(digest);
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| BackendFailure::NotFound)?;
    let metadata = file.metadata().map_err(|_| BackendFailure::Rejected)?;
    if !metadata.is_file() {
        return Err(BackendFailure::Rejected);
    }
    Ok(file)
}

fn digest_open_file(mut file: fs::File) -> Result<String, BackendFailure> {
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|_| BackendFailure::Rejected)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ServiceConfig {
    schema_version: u16,
    socket_path: PathBuf,
    provisioning_socket_path: PathBuf,
    state_root: PathBuf,
    #[serde(default)]
    limits: Option<ControlLimits>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RunRecord {
    plan_id: String,
    run_id: String,
    attempt_id: String,
    state: PublicRunState,
    revision: Revision,
    plan_sha256: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct MutationRecord {
    request_sha256: String,
    target: MutationTarget,
    state: MutationState,
    result: Option<BoundControlResult>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum MutationTarget {
    CreatePlan,
    Repeat { run_id: String },
    Launch { run_id: String, attempt_id: String },
    Cancel { run_id: String, attempt_id: String },
    ConfigurationApply,
    ProviderProfileUpsert { provider_id: String },
    RecordingCampaignPlan,
    RecordingCampaignLifecycle { campaign_id: String, action: String },
    AuthEnroll { provider: String },
    AuthRotate { provider: String },
    AuthRevoke { provider: String },
}

#[derive(Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum MutationState {
    Intent,
    Committed,
    NeedsReconciliation,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    schema_version: u16,
    runner_instance_id: String,
    revision: Revision,
    plans: BTreeMap<String, PlanFile>,
    runs: BTreeMap<String, RunRecord>,
    mutations: BTreeMap<String, MutationRecord>,
    events: Vec<ControlEvent>,
    #[serde(default)]
    auth: BTreeMap<String, AuthRecord>,
    #[serde(default)]
    configuration: Option<ConfigurationRecord>,
    #[serde(default)]
    recording_campaign: Option<RecordingCampaignRecord>,
    #[serde(default = "default_provider_generation")]
    provider_generation: u64,
    #[serde(default)]
    provider_profiles: BTreeMap<String, ProviderCatalogEntry>,
    #[serde(default)]
    provider_credential_references: BTreeMap<String, String>,
}

fn default_provider_generation() -> u64 {
    1
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ConfigurationRecord {
    agent_ids: Vec<String>,
    provider_id: String,
    model_id: String,
    auth_method: asb_control::ProviderAuthMethod,
    credential_reference_sha256: Option<String>,
    generation: u64,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RecordingCampaignRecord {
    campaign_id: String,
    provider_id: String,
    model_id: String,
    agent_ids: Vec<String>,
    workload_ids: Vec<String>,
    tuple_count: u16,
    generation: u64,
    #[serde(default = "default_recording_state")]
    state: String,
    #[serde(default)]
    covered_tuple_count: u16,
    #[serde(default)]
    offline_ready: bool,
    #[serde(default = "default_recording_reason")]
    unavailable_reason: Option<String>,
    #[serde(default)]
    coverage: Vec<RecordingTupleRecord>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RecordingTupleRecord {
    agent_id: String,
    workload_id: String,
    scorer_revision: String,
    attempt_id: String,
    generation: u64,
    state: String,
    cassette_sha256: Option<String>,
    redaction_verified: bool,
    replay_verified: bool,
}

fn default_recording_state() -> String {
    "planned".to_owned()
}

fn default_recording_reason() -> Option<String> {
    Some("recording-required".to_owned())
}

fn recording_tuple_digest(
    provider_id: &str,
    model_id: &str,
    agent_id: &str,
    workload_id: &str,
    scorer_revision: &str,
    generation: u64,
) -> String {
    let mut digest = Sha256::new();
    digest.update(b"asb-recording-tuple-v1\0");
    for value in [
        provider_id,
        model_id,
        agent_id,
        workload_id,
        scorer_revision,
    ] {
        digest.update((value.len() as u64).to_le_bytes());
        digest.update(value.as_bytes());
    }
    digest.update(generation.to_le_bytes());
    format!("{:x}", digest.finalize())
}

fn recording_coverage_complete(campaign: &RecordingCampaignRecord) -> bool {
    campaign.coverage.len() == usize::from(campaign.tuple_count)
        && campaign.coverage.iter().all(|entry| {
            entry.generation == campaign.generation
                && entry.state == "complete"
                && entry.redaction_verified
                && entry.replay_verified
                && entry
                    .cassette_sha256
                    .as_deref()
                    .is_some_and(|digest| asb_control::validate_digest(digest).is_ok())
        })
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct AuthRecord {
    provider: String,
    endpoint_identity_sha256: String,
    credential_locator_sha256: String,
    generation: u64,
    status: String,
}

struct ActiveRun {
    attempt_id: String,
    cancelled: Arc<AtomicBool>,
}

struct ActiveRunLease {
    active: Arc<Mutex<BTreeMap<String, ActiveRun>>>,
    run_id: String,
}

impl ActiveRunLease {
    fn new(active: Arc<Mutex<BTreeMap<String, ActiveRun>>>, run_id: String) -> Self {
        Self { active, run_id }
    }
}

impl Drop for ActiveRunLease {
    fn drop(&mut self) {
        if let Ok(mut active) = self.active.lock() {
            active.remove(&self.run_id);
        }
    }
}

struct RunnerBackend {
    state_root: PathBuf,
    runner_instance_id: String,
    catalog: Arc<Mutex<Catalog>>,
    active: Arc<Mutex<BTreeMap<String, ActiveRun>>>,
    state_lock: Arc<fs::File>,
    provider_capture: Arc<dyn ProviderCapture>,
}

#[derive(Clone, Copy)]
enum RecordingLifecycleAction {
    Execute,
    Cancel,
    Reconcile,
    OfflineDefault,
}

impl RecordingLifecycleAction {
    fn name(self) -> &'static str {
        match self {
            Self::Execute => "execute",
            Self::Cancel => "cancel",
            Self::Reconcile => "reconcile",
            Self::OfflineDefault => "offline_default",
        }
    }
}

pub(crate) fn serve(path: &Path) -> Result<(), CliError> {
    let config = load_config(path)?;
    if config.schema_version != 1 {
        return Err(CliError::validation(
            "unsupported control service configuration",
        ));
    }
    validate_root(&config.state_root)?;
    prepare_root(&config.state_root)?;
    let state_root = fs::canonicalize(&config.state_root)
        .map_err(|_| CliError::operation("control state root cannot be resolved"))?;
    validate_service_endpoint(&config.socket_path, &state_root)?;
    validate_service_endpoint(&config.provisioning_socket_path, &state_root)?;
    let backend = open_backend(state_root)?;
    let server = ProvisionedControlServer::bind(
        &config.socket_path,
        &config.provisioning_socket_path,
        config.limits.unwrap_or_default(),
        backend,
    )
    .map_err(|_| CliError::operation("control endpoint cannot be bound"))?;
    std::panic::set_hook(Box::new(|_| {
        eprintln!("ASB control worker failed");
    }));
    server
        .serve()
        .map_err(|_| CliError::operation("control service stopped"))
}

fn validate_service_endpoint(path: &Path, state_root: &Path) -> Result<(), CliError> {
    let parent = path
        .parent()
        .ok_or_else(|| CliError::validation("control socket path is unsafe"))?;
    if !path.is_absolute()
        || fs::canonicalize(parent).ok().as_deref() != Some(parent)
        || path.starts_with(state_root)
        || state_root.starts_with(parent)
    {
        return Err(CliError::validation("control socket path is unsafe"));
    }
    Ok(())
}

fn open_backend(state_root: PathBuf) -> Result<RunnerBackend, CliError> {
    open_backend_with_capture(state_root, Arc::new(UnavailableProviderCapture))
}

fn open_backend_with_capture(
    state_root: PathBuf,
    provider_capture: Arc<dyn ProviderCapture>,
) -> Result<RunnerBackend, CliError> {
    let state_lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(state_root.join("control.lock"))
        .map_err(|_| CliError::operation("control state lock cannot be opened"))?;
    state_lock
        .try_lock()
        .map_err(|_| CliError::operation("control state root is already owned"))?;
    let mut catalog = load_or_create_catalog(&state_root)?;
    reconcile_catalog(&mut catalog)?;
    commit_catalog(&state_root, &catalog)?;
    let runner_instance_id = catalog.runner_instance_id.clone();
    Ok(RunnerBackend {
        state_root,
        runner_instance_id,
        catalog: Arc::new(Mutex::new(catalog)),
        active: Arc::new(Mutex::new(BTreeMap::new())),
        state_lock: Arc::new(state_lock),
        provider_capture,
    })
}

fn load_config(path: &Path) -> Result<ServiceConfig, CliError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| CliError::validation("control configuration is unavailable"))?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > MAX_CONTROL_CONFIG_BYTES
    {
        return Err(CliError::validation(
            "control configuration is not a bounded regular file",
        ));
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|_| CliError::validation("control configuration cannot be read"))?;
    let opened = file
        .metadata()
        .map_err(|_| CliError::validation("control configuration cannot be inspected"))?;
    if opened.dev() != metadata.dev() || opened.ino() != metadata.ino() {
        return Err(CliError::validation(
            "control configuration changed during validation",
        ));
    }
    let mut bytes = Vec::new();
    file.take(MAX_CONTROL_CONFIG_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| CliError::validation("control configuration cannot be read"))?;
    if bytes.len() as u64 > MAX_CONTROL_CONFIG_BYTES {
        return Err(CliError::validation("control configuration is too large"));
    }
    toml::from_str(
        std::str::from_utf8(&bytes)
            .map_err(|_| CliError::validation("control configuration must be UTF-8"))?,
    )
    .map_err(|_| CliError::validation("control configuration shape is invalid"))
}

fn load_or_create_catalog(root: &Path) -> Result<Catalog, CliError> {
    let path = root.join("control-catalog.json");
    if path.exists() {
        let metadata = fs::symlink_metadata(&path)
            .map_err(|_| CliError::operation("control catalog cannot be inspected"))?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.len() as usize > MAX_CATALOG_BYTES
        {
            return Err(CliError::operation("control catalog is unsafe"));
        }
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)
            .map_err(|_| CliError::operation("control catalog cannot be read"))?;
        let opened = file
            .metadata()
            .map_err(|_| CliError::operation("control catalog cannot be inspected"))?;
        if opened.dev() != metadata.dev() || opened.ino() != metadata.ino() {
            return Err(CliError::operation(
                "control catalog changed during validation",
            ));
        }
        let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
        file.take(MAX_CATALOG_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| CliError::operation("control catalog cannot be read"))?;
        if bytes.len() > MAX_CATALOG_BYTES {
            return Err(CliError::operation("control catalog exceeds its bound"));
        }
        let catalog: Catalog = serde_json::from_slice(&bytes)
            .map_err(|_| CliError::operation("control catalog is corrupt"))?;
        validate_catalog(&catalog)?;
        return Ok(catalog);
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| CliError::operation("system clock is invalid"))?
        .as_nanos();
    let identity = format!(
        "runner-{:x}",
        Sha256::digest(format!("{}:{now}", root.display()).as_bytes())
    );
    Ok(Catalog {
        schema_version: CATALOG_VERSION,
        runner_instance_id: identity[..39].to_owned(),
        revision: Revision(0),
        plans: BTreeMap::new(),
        runs: BTreeMap::new(),
        mutations: BTreeMap::new(),
        events: Vec::new(),
        auth: BTreeMap::new(),
        configuration: None,
        recording_campaign: None,
        provider_generation: 1,
        provider_profiles: BTreeMap::new(),
        provider_credential_references: BTreeMap::new(),
    })
}

fn validate_catalog(catalog: &Catalog) -> Result<(), CliError> {
    if catalog.schema_version != CATALOG_VERSION
        || asb_control::validate_identity(&catalog.runner_instance_id).is_err()
        || catalog.events.len() > 1_000_000
        || catalog.plans.len() > 100_000
        || catalog.runs.len() > 1_000_000
        || catalog.mutations.len() > 1_000_000
        || catalog.revision.0 != catalog.events.last().map_or(0, |event| event.revision.0)
    {
        return Err(CliError::operation("control catalog is invalid"));
    }
    if catalog.provider_generation == 0
        || catalog.provider_profiles.len() > 32
        || catalog.provider_credential_references.len() > 32
        || catalog
            .provider_credential_references
            .iter()
            .any(|(provider, digest)| {
                asb_control::validate_identity(provider).is_err()
                    || asb_control::validate_digest(digest).is_err()
            })
    {
        return Err(CliError::operation("control provider registry is invalid"));
    }
    for (provider_id, entry) in &catalog.provider_profiles {
        if provider_id != &entry.provider_id
            || !matches!(
                entry.availability,
                ProviderAvailability::Unavailable(ref reason) if reason == "authorization-required"
            )
        {
            return Err(CliError::operation("control provider profile is invalid"));
        }
        let mut snapshot = ProviderCatalog {
            runner_instance_id: catalog.runner_instance_id.clone(),
            generation: Revision(catalog.provider_generation),
            catalog_sha256: String::new(),
            providers: vec![entry.clone()],
            refreshed: false,
        };
        snapshot.catalog_sha256 = snapshot
            .computed_sha256()
            .map_err(|_| CliError::operation("control provider profile is invalid"))?;
        snapshot
            .validate()
            .map_err(|_| CliError::operation("control provider profile is invalid"))?;
    }
    if let Some(configuration) = &catalog.configuration {
        let snapshot = ConfigurationSnapshot {
            runner_instance_id: catalog.runner_instance_id.clone(),
            generation: Revision(configuration.generation),
            configured: true,
            agent_ids: configuration.agent_ids.clone(),
            provider_id: Some(configuration.provider_id.clone()),
            model_id: Some(configuration.model_id.clone()),
            auth_method: Some(configuration.auth_method),
            credential_reference_sha256: configuration.credential_reference_sha256.clone(),
        };
        snapshot
            .validate()
            .map_err(|_| CliError::operation("control configuration is invalid"))?;
    }
    if let Some(campaign) = &catalog.recording_campaign {
        asb_control::validate_identity(&campaign.campaign_id)
            .and_then(|_| asb_control::validate_identity(&campaign.provider_id))
            .and_then(|_| asb_control::validate_identity(&campaign.model_id))
            .map_err(|_| CliError::operation("control recording campaign identity is invalid"))?;
        if campaign.generation == 0
            || campaign.agent_ids.is_empty()
            || campaign.workload_ids.is_empty()
            || !matches!(
                campaign.state.as_str(),
                "planned"
                    | "recording"
                    | "needs_reconciliation"
                    | "complete"
                    | "cancelled"
                    | "failed"
            )
            || campaign.tuple_count
                != u16::try_from(
                    campaign
                        .agent_ids
                        .len()
                        .saturating_mul(campaign.workload_ids.len()),
                )
                .unwrap_or(0)
        {
            return Err(CliError::operation("control recording campaign is invalid"));
        }
        let mut agents = campaign.agent_ids.clone();
        let mut workloads = campaign.workload_ids.clone();
        agents.sort();
        workloads.sort();
        if agents != campaign.agent_ids
            || workloads != campaign.workload_ids
            || agents.windows(2).any(|pair| pair[0] == pair[1])
            || workloads.windows(2).any(|pair| pair[0] == pair[1])
        {
            return Err(CliError::operation(
                "control recording campaign ordering is invalid",
            ));
        }
        if campaign.coverage.len() != usize::from(campaign.tuple_count) {
            return Err(CliError::operation(
                "control recording campaign coverage is incomplete",
            ));
        }
        let mut identities = BTreeSet::new();
        let mut previous: Option<(String, String)> = None;
        for entry in &campaign.coverage {
            asb_control::validate_identity(&entry.agent_id)
                .and_then(|()| asb_control::validate_identity(&entry.workload_id))
                .and_then(|()| asb_control::validate_identity(&entry.scorer_revision))
                .and_then(|()| asb_control::validate_identity(&entry.attempt_id))
                .map_err(|_| CliError::operation("recording tuple identity is invalid"))?;
            if entry.generation != campaign.generation
                || !matches!(
                    entry.state.as_str(),
                    "ready" | "in_progress" | "complete" | "failed" | "stale"
                )
                || entry.state == "complete"
                    && (!entry.redaction_verified
                        || !entry.replay_verified
                        || entry
                            .cassette_sha256
                            .as_deref()
                            .is_none_or(|digest| asb_control::validate_digest(digest).is_err()))
                || entry.state != "complete"
                    && (entry.redaction_verified
                        || entry.replay_verified
                        || entry.cassette_sha256.is_some())
            {
                return Err(CliError::operation("recording tuple coverage is invalid"));
            }
            let identity = (entry.agent_id.clone(), entry.workload_id.clone());
            if previous.as_ref().is_some_and(|value| value >= &identity)
                || !identities.insert(identity.clone())
                || !campaign.agent_ids.contains(&entry.agent_id)
                || !campaign.workload_ids.contains(&entry.workload_id)
            {
                return Err(CliError::operation(
                    "recording tuple coverage is not an exact ordered matrix",
                ));
            }
            previous = Some(identity);
        }
        let complete = campaign
            .coverage
            .iter()
            .filter(|entry| entry.state == "complete")
            .count();
        if usize::from(campaign.covered_tuple_count) != complete
            || campaign.offline_ready
                != (campaign.state == "complete" && recording_coverage_complete(campaign))
        {
            return Err(CliError::operation(
                "recording campaign aggregate does not match tuple coverage",
            ));
        }
    }
    for (plan_id, plan) in &catalog.plans {
        asb_control::validate_identity(plan_id)
            .map_err(|_| CliError::operation("control plan identity is invalid"))?;
        validate_plan(plan)?;
    }
    for (index, event) in catalog.events.iter().enumerate() {
        if event.revision.0 != u64::try_from(index).unwrap_or(u64::MAX) + 1 {
            return Err(CliError::operation("control event revisions are invalid"));
        }
        let causal = event.run_id.is_some() && event.attempt_id.is_some();
        let valid = match event.kind {
            ControlEventKind::RunnerReady | ControlEventKind::PlanCreated => {
                event.run_id.is_none() && event.attempt_id.is_none()
            }
            _ => causal,
        };
        if !valid {
            return Err(CliError::operation("control event association is invalid"));
        }
    }
    for (run_id, run) in &catalog.runs {
        if run_id != &run.run_id
            || asb_control::validate_identity(&run.run_id).is_err()
            || asb_control::validate_identity(&run.attempt_id).is_err()
            || asb_control::validate_digest(&run.plan_sha256).is_err()
        {
            return Err(CliError::operation("control run record is invalid"));
        }
        let plan = catalog
            .plans
            .get(&run.plan_id)
            .ok_or_else(|| CliError::operation("control run plan is missing"))?;
        if plan_digest(plan)? != run.plan_sha256 {
            return Err(CliError::operation("control run plan digest is invalid"));
        }
        let latest = catalog
            .events
            .iter()
            .rev()
            .find(|event| event.run_id.as_ref().is_some_and(|id| id.0 == *run_id))
            .ok_or_else(|| CliError::operation("control run event is missing"))?;
        let state_kind_ok = match run.state {
            PublicRunState::Planned | PublicRunState::Prepared | PublicRunState::Collecting => {
                matches!(latest.kind, ControlEventKind::RunUpdated)
            }
            PublicRunState::Running => matches!(latest.kind, ControlEventKind::RunStarted),
            PublicRunState::Completed => matches!(latest.kind, ControlEventKind::RunCompleted),
            PublicRunState::Failed => matches!(latest.kind, ControlEventKind::RunFailed),
            PublicRunState::Cancelled => matches!(latest.kind, ControlEventKind::RunCancelled),
            PublicRunState::NeedsReconciliation => {
                matches!(latest.kind, ControlEventKind::ReconciliationRequired)
            }
        };
        if latest
            .attempt_id
            .as_ref()
            .is_none_or(|id| id.0 != run.attempt_id)
            || latest.revision != run.revision
            || !state_kind_ok
        {
            return Err(CliError::operation("control run event state is invalid"));
        }
    }
    let maximum_limits = ControlLimits {
        max_frame_bytes: asb_control::MAX_CONTROL_FRAME_BYTES,
        max_timeout_ms: asb_control::MAX_CONTROL_TIMEOUT_MS,
        max_page_items: asb_control::MAX_PAGE_ITEMS,
        max_in_flight: asb_control::MAX_CONTROL_IN_FLIGHT,
    };
    for (key, mutation) in &catalog.mutations {
        if asb_control::validate_digest(key).is_err()
            || asb_control::validate_digest(&mutation.request_sha256).is_err()
        {
            return Err(CliError::operation("control mutation record is invalid"));
        }
        match &mutation.target {
            MutationTarget::CreatePlan => {}
            MutationTarget::Repeat { run_id } => {
                asb_control::validate_identity(run_id)
                    .map_err(|_| CliError::operation("control mutation target is invalid"))?;
            }
            MutationTarget::Launch { run_id, attempt_id }
            | MutationTarget::Cancel { run_id, attempt_id } => {
                asb_control::validate_identity(run_id)
                    .and_then(|()| asb_control::validate_identity(attempt_id))
                    .map_err(|_| CliError::operation("control mutation target is invalid"))?;
            }
            MutationTarget::AuthEnroll { provider }
            | MutationTarget::AuthRotate { provider }
            | MutationTarget::AuthRevoke { provider } => {
                asb_control::validate_identity(provider)
                    .map_err(|_| CliError::operation("control auth mutation target is invalid"))?;
            }
            MutationTarget::ConfigurationApply => {}
            MutationTarget::ProviderProfileUpsert { provider_id } => {
                asb_control::validate_identity(provider_id).map_err(|_| {
                    CliError::operation("control provider mutation target is invalid")
                })?;
            }
            MutationTarget::RecordingCampaignPlan => {}
            MutationTarget::RecordingCampaignLifecycle {
                campaign_id,
                action,
            } => {
                asb_control::validate_identity(campaign_id)
                    .and_then(|()| asb_control::validate_identity(action))
                    .map_err(|_| {
                        CliError::operation("control recording mutation target is invalid")
                    })?;
            }
        }
        match (mutation.state, mutation.result.as_ref()) {
            (MutationState::Committed, Some(result))
                if result.request_sha256 == mutation.request_sha256
                    && asb_control::ControlSuccess::Operation(result.clone())
                        .validate(maximum_limits)
                        .is_ok() => {}
            (MutationState::Intent | MutationState::NeedsReconciliation, Some(result))
                if result.request_sha256 == mutation.request_sha256
                    && asb_control::ControlSuccess::Operation(result.clone())
                        .validate(maximum_limits)
                        .is_ok() => {}
            _ => return Err(CliError::operation("control mutation outcome is invalid")),
        }
        if let Some(result) = mutation.result.as_ref() {
            let plan_matches = |reference: &PlanReference| {
                catalog.plans.get(&reference.plan_id).is_some_and(|plan| {
                    plan_digest(plan).is_ok_and(|digest| {
                        reference.plan_sha256 == digest
                            && reference.plan_id == format!("plan-{}", &digest[..24])
                    })
                })
            };
            let target_matches = match (&mutation.target, &result.result) {
                (MutationTarget::CreatePlan, ControlResult::Plan(reference)) => {
                    plan_matches(reference)
                }
                (MutationTarget::Repeat { run_id }, ControlResult::Plan(reference)) => {
                    catalog.runs.contains_key(run_id) && plan_matches(reference)
                }
                (MutationTarget::Launch { run_id, attempt_id }, ControlResult::Launch(summary)) => {
                    catalog.runs.get(run_id).is_some_and(|run| {
                        run.run_id == *run_id
                            && run.attempt_id == *attempt_id
                            && summary.run_id.0 == *run_id
                            && summary.attempt_id.0 == *attempt_id
                            && summary.state == PublicRunState::Planned
                            && summary.plan_sha256 == run.plan_sha256
                            && summary.revision.0 > 0
                            && catalog
                                .events
                                .get(usize::try_from(summary.revision.0 - 1).unwrap_or(usize::MAX))
                                .is_some_and(|event| {
                                    event.revision == summary.revision
                                        && event.kind == ControlEventKind::RunUpdated
                                        && event.run_id.as_ref().is_some_and(|id| id.0 == *run_id)
                                        && event
                                            .attempt_id
                                            .as_ref()
                                            .is_some_and(|id| id.0 == *attempt_id)
                                })
                    })
                }
                (MutationTarget::Cancel { run_id, attempt_id }, ControlResult::Acknowledged(_)) => {
                    catalog.runs.get(run_id).is_some_and(|run| {
                        run.run_id == *run_id
                            && run.attempt_id == *attempt_id
                            && (mutation.state != MutationState::Committed
                                || run.state == PublicRunState::Cancelled)
                    })
                }
                (
                    MutationTarget::AuthEnroll { provider }
                    | MutationTarget::AuthRotate { provider }
                    | MutationTarget::AuthRevoke { provider },
                    ControlResult::Acknowledged(_),
                ) => catalog
                    .auth
                    .get(provider)
                    .is_some_and(|record| record.provider == *provider),
                (MutationTarget::ConfigurationApply, ControlResult::Configuration(snapshot)) => {
                    catalog.configuration.as_ref().is_some_and(|configuration| {
                        snapshot.configured
                            && snapshot.generation.0 == configuration.generation
                            && snapshot.provider_id.as_deref()
                                == Some(configuration.provider_id.as_str())
                    })
                }
                (
                    MutationTarget::ProviderProfileUpsert { provider_id },
                    ControlResult::ProviderProfile(snapshot),
                ) => {
                    snapshot.generation.0 == catalog.provider_generation
                        && snapshot.providers.iter().any(|entry| {
                            entry.provider_id == *provider_id
                                && catalog.provider_profiles.get(provider_id) == Some(entry)
                        })
                }
                (MutationTarget::RecordingCampaignPlan, ControlResult::RecordingCampaign(plan)) => {
                    catalog.recording_campaign.as_ref().is_some_and(|campaign| {
                        plan.campaign_id == campaign.campaign_id
                            && plan.generation.0 == campaign.generation
                            && plan.provider_id == campaign.provider_id
                            && plan.model_id == campaign.model_id
                            && plan.agent_ids == campaign.agent_ids
                            && plan.workload_ids == campaign.workload_ids
                            && plan.tuple_count == campaign.tuple_count
                    })
                }
                (
                    MutationTarget::RecordingCampaignLifecycle { campaign_id, .. },
                    ControlResult::RecordingCampaignLifecycle(lifecycle),
                ) => catalog.recording_campaign.as_ref().is_some_and(|campaign| {
                    lifecycle.campaign_id == *campaign_id
                        && lifecycle.campaign_id == campaign.campaign_id
                        && lifecycle.generation.0 == campaign.generation
                        && lifecycle.provider_id == campaign.provider_id
                        && lifecycle.model_id == campaign.model_id
                        && lifecycle.agent_ids == campaign.agent_ids
                        && lifecycle.workload_ids == campaign.workload_ids
                        && lifecycle.tuple_count == campaign.tuple_count
                        // Lifecycle mutation results are historical snapshots.  A later
                        // reconcile/cancel transition may legitimately change the current
                        // campaign state, so only immutable identity and matrix fields are
                        // matched here.
                        && lifecycle.covered_tuple_count <= lifecycle.tuple_count
                        && matches!(
                            lifecycle.state.as_str(),
                            "planned"
                                | "recording"
                                | "needs_reconciliation"
                                | "complete"
                                | "cancelled"
                                | "failed"
                        )
                }),
                _ => false,
            };
            if !target_matches {
                return Err(CliError::operation(
                    "control mutation target/result mismatch",
                ));
            }
        }
    }
    Ok(())
}

fn plan_digest(plan: &PlanFile) -> Result<String, CliError> {
    let bytes = serde_json::to_vec(plan)
        .map_err(|_| CliError::operation("control plan cannot be encoded"))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn commit_catalog(root: &Path, catalog: &Catalog) -> Result<(), CliError> {
    let bytes = serde_json::to_vec(catalog)
        .map_err(|_| CliError::operation("control catalog cannot be encoded"))?;
    if bytes.len() > MAX_CATALOG_BYTES {
        return Err(CliError::operation("control catalog exceeds its bound"));
    }
    let temporary = root.join(format!(
        ".control-catalog-{}-{}.tmp",
        std::process::id(),
        CATALOG_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .map_err(|_| CliError::operation("control catalog staging failed"))?;
    let result = (|| {
        file.write_all(&bytes)
            .map_err(|_| CliError::operation("control catalog write failed"))?;
        file.sync_all()
            .map_err(|_| CliError::operation("control catalog sync failed"))?;
        fs::rename(&temporary, root.join("control-catalog.json"))
            .map_err(|_| CliError::operation("control catalog commit failed"))?;
        fs::File::open(root)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| CliError::operation("control catalog directory sync failed"))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn commit_staged_catalog(root: &Path, live: &mut Catalog, staged: Catalog) -> Result<(), CliError> {
    validate_catalog(&staged)?;
    match commit_catalog(root, &staged) {
        Ok(()) => {
            *live = staged;
            Ok(())
        }
        Err(error) => {
            // A directory fsync can fail after the atomic rename. Reload the
            // authoritative replacement so memory cannot serve an older catalog.
            if let Ok(reloaded) = load_or_create_catalog(root) {
                *live = reloaded;
            }
            Err(error)
        }
    }
}

fn commit_analysis(root: &Path, bytes: &[u8], digest: &str) -> Result<(), CliError> {
    let directory = root.join("analyses");
    if !directory.exists() {
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .map_err(|_| CliError::operation("analysis directory cannot be created"))?;
    }
    let destination = directory.join(digest);
    if destination.exists() {
        return if digest_file(&destination)? == digest {
            Ok(())
        } else {
            Err(CliError::operation("stored analysis digest is invalid"))
        };
    }
    let temporary = directory.join(format!(
        ".analysis-{}-{}.tmp",
        std::process::id(),
        CATALOG_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .map_err(|_| CliError::operation("analysis staging failed"))?;
    let result = (|| {
        file.write_all(bytes)
            .map_err(|_| CliError::operation("analysis write failed"))?;
        file.sync_all()
            .map_err(|_| CliError::operation("analysis sync failed"))?;
        fs::rename(&temporary, &destination)
            .map_err(|_| CliError::operation("analysis commit failed"))?;
        fs::File::open(&directory)
            .and_then(|value| value.sync_all())
            .map_err(|_| CliError::operation("analysis directory sync failed"))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn reconcile_catalog(catalog: &mut Catalog) -> Result<(), CliError> {
    if let Some(campaign) = catalog.recording_campaign.as_mut()
        && campaign.state == "recording"
    {
        for entry in &mut campaign.coverage {
            if entry.state == "in_progress" {
                entry.state = "stale".to_owned();
            }
        }
        campaign.state = "needs_reconciliation".to_owned();
        campaign.covered_tuple_count = campaign
            .coverage
            .iter()
            .filter(|entry| entry.state == "complete")
            .count()
            .try_into()
            .unwrap_or(u16::MAX);
        campaign.offline_ready = false;
        campaign.unavailable_reason = Some("provider-capture-interrupted".to_owned());
    }
    let mut uncertain_runs = BTreeSet::new();
    let mut intents = Vec::new();
    for (key, mutation) in &mut catalog.mutations {
        if mutation.state == MutationState::Intent {
            intents.push((key.clone(), mutation.target.clone()));
            mutation.state = MutationState::NeedsReconciliation;
        }
        if let MutationTarget::Launch { run_id, .. } | MutationTarget::Cancel { run_id, .. } =
            &mutation.target
        {
            uncertain_runs.insert(run_id.clone());
        }
    }
    let plans = catalog.plans.clone();
    let mut transitions = Vec::new();
    for record in catalog.runs.values_mut() {
        let prior = record.state;
        let Some(plan) = plans.get(&record.plan_id) else {
            record.state = PublicRunState::NeedsReconciliation;
            if record.state != prior {
                transitions.push(record.run_id.clone());
            }
            continue;
        };
        let observed = observed_state(plan, &record.run_id, record.state);
        record.state = if matches!(
            observed,
            PublicRunState::Running | PublicRunState::Collecting
        ) || (uncertain_runs.contains(&record.run_id)
            && matches!(
                observed,
                PublicRunState::Planned
                    | PublicRunState::Prepared
                    | PublicRunState::Running
                    | PublicRunState::Collecting
            )) {
            PublicRunState::NeedsReconciliation
        } else {
            observed
        };
        if record.state != prior {
            transitions.push(record.run_id.clone());
        }
    }
    for run_id in transitions {
        let mut record = catalog
            .runs
            .get(&run_id)
            .cloned()
            .ok_or_else(|| CliError::operation("control reconciliation lost a run"))?;
        let revision = catalog
            .revision
            .0
            .checked_add(1)
            .map(Revision)
            .ok_or_else(|| CliError::operation("control revision is exhausted"))?;
        catalog.revision = revision;
        let kind = match record.state {
            PublicRunState::Completed => ControlEventKind::RunCompleted,
            PublicRunState::Cancelled => ControlEventKind::RunCancelled,
            PublicRunState::Failed => ControlEventKind::RunFailed,
            PublicRunState::NeedsReconciliation => ControlEventKind::ReconciliationRequired,
            _ => ControlEventKind::RunUpdated,
        };
        record.revision = revision;
        catalog.events.push(ControlEvent {
            revision,
            kind,
            run_id: Some(RunId(record.run_id.clone())),
            attempt_id: Some(asb_control::AttemptId(record.attempt_id.clone())),
        });
        catalog.runs.insert(run_id, record);
    }
    let mut remove = Vec::new();
    for (key, target) in intents {
        match target {
            MutationTarget::Launch { run_id, .. } => {
                let journal_exists = catalog
                    .runs
                    .get(&run_id)
                    .and_then(|record| catalog.plans.get(&record.plan_id))
                    .and_then(|plan| {
                        AtomicStore::open(&plan.result_root, StoreLimits::default())
                            .ok()
                            .and_then(|store| store.load_journal(&run_id).ok())
                    })
                    .is_some_and(|events| !events.is_empty());
                if journal_exists && let Some(mutation) = catalog.mutations.get_mut(&key) {
                    mutation.state = MutationState::Committed;
                }
            }
            MutationTarget::Cancel { run_id, .. } => {
                match catalog.runs.get(&run_id).map(|record| record.state) {
                    Some(PublicRunState::Cancelled) => {
                        if let Some(mutation) = catalog.mutations.get_mut(&key) {
                            mutation.state = MutationState::Committed;
                        }
                    }
                    Some(PublicRunState::Completed | PublicRunState::Failed) => remove.push(key),
                    _ => {}
                }
            }
            MutationTarget::AuthEnroll { .. }
            | MutationTarget::AuthRotate { .. }
            | MutationTarget::AuthRevoke { .. }
            | MutationTarget::ConfigurationApply
            | MutationTarget::ProviderProfileUpsert { .. }
            | MutationTarget::RecordingCampaignPlan
            | MutationTarget::RecordingCampaignLifecycle { .. } => {}
            MutationTarget::CreatePlan | MutationTarget::Repeat { .. } => {}
        }
    }
    for key in remove {
        catalog.mutations.remove(&key);
    }
    Ok(())
}

fn observed_state(plan: &PlanFile, run_id: &str, prior: PublicRunState) -> PublicRunState {
    if !plan.result_root.exists() {
        return if prior == PublicRunState::Planned {
            PublicRunState::Planned
        } else {
            PublicRunState::NeedsReconciliation
        };
    }
    if !plan
        .result_root
        .join("runs")
        .join(run_id)
        .join("manifest.json")
        .is_file()
    {
        return if prior == PublicRunState::Planned {
            PublicRunState::Planned
        } else {
            PublicRunState::NeedsReconciliation
        };
    }
    let Ok(store) = AtomicStore::open(&plan.result_root, StoreLimits::default()) else {
        return PublicRunState::NeedsReconciliation;
    };
    let Ok(events) = store.load_journal(run_id) else {
        return PublicRunState::NeedsReconciliation;
    };
    match events.last().map(|event| event.state) {
        None | Some(ExecutionState::Planned) => PublicRunState::Planned,
        Some(ExecutionState::Prepared) => PublicRunState::Prepared,
        Some(ExecutionState::Running) => PublicRunState::Running,
        Some(ExecutionState::Collecting) => PublicRunState::Collecting,
        Some(ExecutionState::Completed) => PublicRunState::Completed,
        Some(ExecutionState::Failed) => PublicRunState::Failed,
        Some(ExecutionState::Cancelled) => PublicRunState::Cancelled,
    }
}

impl RunnerBackend {
    fn bind(
        &self,
        call: &ControlCall,
        result: ControlResult,
    ) -> Result<BoundControlResult, BackendFailure> {
        BoundControlResult::new(call, result).map_err(|_| BackendFailure::Rejected)
    }

    fn mutation(
        &self,
        call: &ControlCall,
        key: &str,
        target: MutationTarget,
        deadline: RequestDeadline,
        build: impl FnOnce(&mut Catalog) -> Result<ControlResult, BackendFailure>,
    ) -> Result<BoundControlResult, BackendFailure> {
        let request = BoundControlResult::new(
            call,
            ControlResult::Acknowledged(MutationAcknowledgement { accepted: true }),
        )
        .map_err(|_| BackendFailure::Rejected)?;
        let key_digest = format!("{:x}", Sha256::digest(key.as_bytes()));
        let mut catalog = self
            .catalog
            .lock()
            .map_err(|_| BackendFailure::NeedsReconciliation)?;
        if let Some(record) = catalog.mutations.get(&key_digest) {
            return match (
                record.request_sha256 == request.request_sha256,
                record.state,
                record.result.as_ref(),
            ) {
                (true, MutationState::Committed, Some(result)) => Ok(result.clone()),
                _ => Err(BackendFailure::NeedsReconciliation),
            };
        }
        let mut staged = catalog.clone();
        let result = self.bind(call, build(&mut staged)?)?;
        staged.mutations.insert(
            key_digest.clone(),
            MutationRecord {
                request_sha256: result.request_sha256.clone(),
                target,
                state: MutationState::Committed,
                result: Some(result.clone()),
            },
        );
        deadline
            .check()
            .map_err(|_| BackendFailure::NeedsReconciliation)?;
        commit_staged_catalog(&self.state_root, &mut catalog, staged)
            .map_err(|_| BackendFailure::NeedsReconciliation)?;
        Ok(result)
    }

    fn append_event(
        catalog: &mut Catalog,
        kind: ControlEventKind,
        run: Option<&RunRecord>,
    ) -> Result<Revision, BackendFailure> {
        let revision = Revision(
            catalog
                .revision
                .0
                .checked_add(1)
                .ok_or(BackendFailure::Rejected)?,
        );
        catalog.revision = revision;
        catalog.events.push(ControlEvent {
            revision,
            kind,
            run_id: run.map(|value| RunId(value.run_id.clone())),
            attempt_id: run.map(|value| asb_control::AttemptId(value.attempt_id.clone())),
        });
        Ok(revision)
    }

    fn launch(
        &self,
        call: &ControlCall,
        params: &asb_control::LaunchParams,
        deadline: RequestDeadline,
    ) -> Result<BoundControlResult, BackendFailure> {
        let key_digest = format!("{:x}", Sha256::digest(params.idempotency_key.as_bytes()));
        let call_digest = BoundControlResult::new(
            call,
            ControlResult::Acknowledged(MutationAcknowledgement { accepted: true }),
        )
        .map_err(|_| BackendFailure::Rejected)?
        .request_sha256;
        let mut catalog = self
            .catalog
            .lock()
            .map_err(|_| BackendFailure::NeedsReconciliation)?;
        if let Some(record) = catalog.mutations.get(&key_digest) {
            return match (
                record.request_sha256 == call_digest,
                record.state,
                record.result.as_ref(),
            ) {
                (true, MutationState::Committed, Some(result)) => Ok(result.clone()),
                _ => Err(BackendFailure::NeedsReconciliation),
            };
        }
        let plan = catalog
            .plans
            .get(&params.plan_id)
            .cloned()
            .ok_or(BackendFailure::NotFound)?;
        if catalog.runs.contains_key(&plan.run_id) {
            return Err(BackendFailure::NeedsReconciliation);
        }
        let mut record = RunRecord {
            plan_id: params.plan_id.clone(),
            run_id: plan.run_id.clone(),
            attempt_id: format!("{}-attempt", plan.run_id),
            state: PublicRunState::Planned,
            revision: Revision(0),
            plan_sha256: plan_digest(&plan).map_err(|_| BackendFailure::Rejected)?,
        };
        let mut intent = catalog.clone();
        record.revision =
            Self::append_event(&mut intent, ControlEventKind::RunUpdated, Some(&record))?;
        intent.runs.insert(record.run_id.clone(), record.clone());
        let result = self.bind(
            call,
            ControlResult::Launch(Self::summary(&intent, &record)?),
        )?;
        intent.mutations.insert(
            key_digest.clone(),
            MutationRecord {
                request_sha256: result.request_sha256.clone(),
                target: MutationTarget::Launch {
                    run_id: record.run_id.clone(),
                    attempt_id: record.attempt_id.clone(),
                },
                state: MutationState::Intent,
                result: Some(result.clone()),
            },
        );
        deadline
            .check()
            .map_err(|_| BackendFailure::NeedsReconciliation)?;
        commit_staged_catalog(&self.state_root, &mut catalog, intent)
            .map_err(|_| BackendFailure::NeedsReconciliation)?;
        if prepare_root(&plan.result_root).is_err() || prepare_root(&plan.work_root).is_err() {
            self.quarantine_launch(&mut catalog, &key_digest, &record)?;
            return Err(BackendFailure::NeedsReconciliation);
        }
        if deadline.check().is_err() {
            self.quarantine_launch(&mut catalog, &key_digest, &record)?;
            return Err(BackendFailure::NeedsReconciliation);
        }

        let cancelled = Arc::new(AtomicBool::new(false));
        self.active
            .lock()
            .map_err(|_| BackendFailure::NeedsReconciliation)?
            .insert(
                record.run_id.clone(),
                ActiveRun {
                    attempt_id: record.attempt_id.clone(),
                    cancelled: Arc::clone(&cancelled),
                },
            );
        let catalog_state = Arc::clone(&self.catalog);
        let active = Arc::clone(&self.active);
        let state_lock = Arc::clone(&self.state_lock);
        let state_root = self.state_root.clone();
        let run_id = record.run_id.clone();
        let (start_sender, start_receiver) = std::sync::mpsc::channel();
        let worker = thread::Builder::new()
            .name(format!("asb-run-{run_id}"))
            .spawn(move || {
                let _state_lock = state_lock;
                let _active_run = ActiveRunLease::new(Arc::clone(&active), run_id.clone());
                if !start_receiver.recv().unwrap_or(false) {
                    return;
                }
                let execution = AtomicStore::open(&plan.result_root, StoreLimits::default())
                    .map(Arc::new)
                    .map_err(|_| CliError::operation("result store cannot be opened"))
                    .and_then(|store| {
                        run_point(
                            store,
                            &plan,
                            run_id.clone(),
                            plan.point.concurrency,
                            Arc::clone(&cancelled),
                        )
                    });
                if let Ok(mut catalog) = catalog_state.lock() {
                    let mut staged = catalog.clone();
                    if let Some(mut current) = staged.runs.get(&run_id).cloned() {
                        let observed = observed_state(&plan, &run_id, current.state);
                        if observed == current.state
                            && matches!(
                                observed,
                                PublicRunState::Completed
                                    | PublicRunState::Failed
                                    | PublicRunState::Cancelled
                                    | PublicRunState::NeedsReconciliation
                            )
                        {
                            return;
                        }
                        current.state = if execution.is_err()
                            && !matches!(
                                observed,
                                PublicRunState::Completed
                                    | PublicRunState::Failed
                                    | PublicRunState::Cancelled
                            ) {
                            PublicRunState::NeedsReconciliation
                        } else {
                            observed
                        };
                        let kind = match current.state {
                            PublicRunState::Completed => ControlEventKind::RunCompleted,
                            PublicRunState::Cancelled => ControlEventKind::RunCancelled,
                            PublicRunState::Failed => ControlEventKind::RunFailed,
                            _ => ControlEventKind::ReconciliationRequired,
                        };
                        if let Ok(revision) =
                            RunnerBackend::append_event(&mut staged, kind, Some(&current))
                        {
                            current.revision = revision;
                            staged.runs.insert(run_id.clone(), current);
                            let _ = commit_staged_catalog(&state_root, &mut catalog, staged);
                        }
                    }
                }
            });
        if worker.is_err() {
            if let Ok(mut active) = self.active.lock() {
                active.remove(&record.run_id);
            }
            self.quarantine_launch(&mut catalog, &key_digest, &record)?;
            return Err(BackendFailure::NeedsReconciliation);
        }

        let mut committed = catalog.clone();
        let mut started = committed
            .runs
            .get(&record.run_id)
            .cloned()
            .ok_or(BackendFailure::NeedsReconciliation)?;
        started.state = PublicRunState::Running;
        started.revision =
            Self::append_event(&mut committed, ControlEventKind::RunStarted, Some(&started))?;
        committed.runs.insert(record.run_id.clone(), started);
        committed.mutations.insert(
            key_digest.clone(),
            MutationRecord {
                request_sha256: result.request_sha256.clone(),
                target: MutationTarget::Launch {
                    run_id: record.run_id.clone(),
                    attempt_id: record.attempt_id.clone(),
                },
                state: MutationState::Committed,
                result: Some(result.clone()),
            },
        );
        if deadline.check().is_err() {
            let _ = start_sender.send(false);
            self.quarantine_launch(&mut catalog, &key_digest, &record)?;
            return Err(BackendFailure::NeedsReconciliation);
        }
        commit_staged_catalog(&self.state_root, &mut catalog, committed)
            .map_err(|_| BackendFailure::NeedsReconciliation)?;
        if deadline.check().is_err() {
            let _ = start_sender.send(false);
            self.quarantine_launch(&mut catalog, &key_digest, &record)?;
            return Err(BackendFailure::NeedsReconciliation);
        }
        if start_sender.send(true).is_err() {
            self.quarantine_launch(&mut catalog, &key_digest, &record)?;
            return Err(BackendFailure::NeedsReconciliation);
        }
        Ok(result)
    }

    fn quarantine_launch(
        &self,
        catalog: &mut Catalog,
        key_digest: &str,
        record: &RunRecord,
    ) -> Result<(), BackendFailure> {
        let mut staged = catalog.clone();
        if let Some(mutation) = staged.mutations.get_mut(key_digest) {
            mutation.state = MutationState::NeedsReconciliation;
        }
        let mut uncertain = staged
            .runs
            .get(&record.run_id)
            .cloned()
            .ok_or(BackendFailure::NeedsReconciliation)?;
        uncertain.state = PublicRunState::NeedsReconciliation;
        uncertain.revision = Self::append_event(
            &mut staged,
            ControlEventKind::ReconciliationRequired,
            Some(&uncertain),
        )?;
        staged.runs.insert(record.run_id.clone(), uncertain);
        commit_staged_catalog(&self.state_root, catalog, staged)
            .map_err(|_| BackendFailure::NeedsReconciliation)
    }

    fn summary(catalog: &Catalog, record: &RunRecord) -> Result<RunSummary, BackendFailure> {
        let created_revision = catalog
            .events
            .iter()
            .find(|event| {
                event
                    .run_id
                    .as_ref()
                    .is_some_and(|id| id.0 == record.run_id)
                    && event
                        .attempt_id
                        .as_ref()
                        .is_some_and(|id| id.0 == record.attempt_id)
            })
            .map(|event| event.revision)
            .ok_or(BackendFailure::NeedsReconciliation)?;
        Ok(RunSummary {
            run_id: RunId(record.run_id.clone()),
            attempt_id: asb_control::AttemptId(record.attempt_id.clone()),
            state: record.state,
            created_revision,
            revision: record.revision,
            plan_sha256: record.plan_sha256.clone(),
        })
    }

    fn refresh(
        &self,
        run_id: &str,
        deadline: RequestDeadline,
    ) -> Result<RunRecord, BackendFailure> {
        let mut catalog = self
            .catalog
            .lock()
            .map_err(|_| BackendFailure::NeedsReconciliation)?;
        let mut staged = catalog.clone();
        let mut record = staged
            .runs
            .get(run_id)
            .cloned()
            .ok_or(BackendFailure::NotFound)?;
        let plan = staged
            .plans
            .get(&record.plan_id)
            .cloned()
            .ok_or(BackendFailure::NeedsReconciliation)?;
        let active_present = self
            .active
            .lock()
            .map_err(|_| BackendFailure::NeedsReconciliation)?
            .contains_key(run_id);
        let state = if active_present
            && matches!(
                record.state,
                PublicRunState::Planned
                    | PublicRunState::Prepared
                    | PublicRunState::Running
                    | PublicRunState::Collecting
            ) {
            record.state
        } else {
            observed_state(&plan, run_id, record.state)
        };
        if state != record.state {
            record.state = state;
            let kind = match state {
                PublicRunState::Completed => ControlEventKind::RunCompleted,
                PublicRunState::Cancelled => ControlEventKind::RunCancelled,
                PublicRunState::Failed => ControlEventKind::RunFailed,
                PublicRunState::NeedsReconciliation => ControlEventKind::ReconciliationRequired,
                PublicRunState::Running => ControlEventKind::RunStarted,
                _ => ControlEventKind::RunUpdated,
            };
            let revision = Self::append_event(&mut staged, kind, Some(&record))?;
            record.revision = revision;
            staged.runs.insert(run_id.to_owned(), record.clone());
            deadline
                .check()
                .map_err(|_| BackendFailure::NeedsReconciliation)?;
            commit_staged_catalog(&self.state_root, &mut catalog, staged)
                .map_err(|_| BackendFailure::NeedsReconciliation)?;
            if matches!(
                state,
                PublicRunState::Completed | PublicRunState::Failed | PublicRunState::Cancelled
            ) && let Ok(mut active) = self.active.lock()
            {
                active.remove(run_id);
            }
        }
        Ok(record)
    }

    fn commit_analysis_before_deadline(
        &self,
        bytes: &[u8],
        digest: &str,
        deadline: RequestDeadline,
    ) -> Result<(), BackendFailure> {
        deadline
            .check()
            .map_err(|_| BackendFailure::NeedsReconciliation)?;
        commit_analysis(&self.state_root, bytes, digest)
            .map_err(|_| BackendFailure::NeedsReconciliation)
    }
}

impl ControlBackend for RunnerBackend {
    fn runner_instance_id(&self) -> &str {
        &self.runner_instance_id
    }

    fn oldest_revision(&self) -> Revision {
        self.catalog
            .lock()
            .ok()
            .and_then(|catalog| catalog.events.first().map(|event| event.revision))
            .unwrap_or(Revision(0))
    }

    fn latest_revision(&self) -> Revision {
        self.catalog
            .lock()
            .map_or(Revision(0), |catalog| catalog.revision)
    }

    fn execute(
        &self,
        call: &ControlCall,
        deadline: RequestDeadline,
    ) -> Result<BoundControlResult, BackendFailure> {
        deadline
            .check()
            .map_err(|_| BackendFailure::NeedsReconciliation)?;
        match call {
            ControlCall::Capabilities => self.bind(
                call,
                ControlResult::Capabilities(Capabilities {
                    validate_settings: true,
                    run_control: true,
                    repeat: true,
                    analysis: true,
                    events: true,
                }),
            ),
            ControlCall::MeasurementCatalog => self.bind(
                call,
                ControlResult::MeasurementCatalog(MeasurementCatalogPublication::built_in(
                    baseline_measurement_catalog(),
                )),
            ),
            ControlCall::ProviderCatalog(request) => {
                let catalog = self.provider_catalog(request)?;
                self.bind(call, ControlResult::ProviderCatalog(catalog))
            }
            ControlCall::ConfigurationStatus(request) => {
                let snapshot = self.configuration_status(request)?;
                self.bind(call, ControlResult::Configuration(snapshot))
            }
            ControlCall::ProviderProfileUpsert(params) => {
                self.provider_profile_upsert(call, params, deadline)
            }
            ControlCall::ConfigurationApply(params) => {
                self.configuration_apply(call, params, deadline)
            }
            ControlCall::RecordingCampaignEstimate(request) => {
                let estimate = self.recording_campaign_estimate(request)?;
                self.bind(call, ControlResult::RecordingCampaignEstimate(estimate))
            }
            ControlCall::RecordingCampaignPlan(params) => {
                self.recording_campaign_plan(call, params, deadline)
            }
            ControlCall::RecordingCampaignStatus(request) => {
                let status = self.recording_campaign_status(request)?;
                self.bind(call, ControlResult::RecordingCampaignStatus(status))
            }
            ControlCall::RecordingCampaignExecute(params) => self.recording_campaign_lifecycle(
                call,
                &params.idempotency_key,
                params.expected_generation,
                &params.runner_instance_id,
                &params.campaign_id,
                RecordingLifecycleAction::Execute,
                deadline,
            ),
            ControlCall::RecordingCampaignProgress(request) => {
                self.recording_campaign_progress(call, request)
            }
            ControlCall::RecordingCampaignCancel(params) => self.recording_campaign_lifecycle(
                call,
                &params.idempotency_key,
                params.expected_generation,
                &params.runner_instance_id,
                &params.campaign_id,
                RecordingLifecycleAction::Cancel,
                deadline,
            ),
            ControlCall::RecordingCampaignReconcile(params) => self.recording_campaign_lifecycle(
                call,
                &params.idempotency_key,
                params.expected_generation,
                &params.runner_instance_id,
                &params.campaign_id,
                RecordingLifecycleAction::Reconcile,
                deadline,
            ),
            ControlCall::RecordingCampaignOfflineDefault(params) => self
                .recording_campaign_lifecycle(
                    call,
                    &params.idempotency_key,
                    params.expected_generation,
                    &params.runner_instance_id,
                    &params.campaign_id,
                    RecordingLifecycleAction::OfflineDefault,
                    deadline,
                ),
            // Agent catalog population and package verification are deliberately
            // not inferred from the runner's local state yet. Keep the new wire
            // operation fail-closed until the authenticated catalog provider is
            // integrated, rather than exposing an empty or unverifiable list.
            ControlCall::AgentCatalog(_) => Err(BackendFailure::CapabilityUnavailable),
            ControlCall::ValidateSettings { settings } => {
                let (issue, measurement_issue) =
                    match serde_json::from_value::<PlanFile>(settings.clone()) {
                        Ok(plan) => {
                            let issue =
                                validate_plan(&plan).err().map(|error| error.settings_issue);
                            let detail = effective_measurement_selection(&plan)
                                .ok()
                                .and_then(|selection| {
                                    measurement_capabilities().ok().and_then(|capabilities| {
                                        selection
                                            .validate(
                                                &baseline_measurement_catalog(),
                                                &capabilities,
                                            )
                                            .err()
                                    })
                                })
                                .map(|error| MeasurementSettingsIssue {
                                    reason: error.reason,
                                    id: error.id,
                                })
                                .filter(|detail| {
                                    issue
                                        == Some(SettingsIssue::from_measurement_reason(
                                            detail.reason,
                                        ))
                                });
                            (issue, detail)
                        }
                        Err(_) => (Some(SettingsIssue::InvalidFormat), None),
                    };
                self.bind(
                    call,
                    ControlResult::SettingsValidation(SettingsValidation {
                        valid: issue.is_none(),
                        issues: issue.into_iter().collect(),
                        measurement_issue,
                    }),
                )
            }
            ControlCall::CreatePlan(params) => self.mutation(
                call,
                &params.idempotency_key,
                MutationTarget::CreatePlan,
                deadline,
                |catalog| {
                    let plan: PlanFile = serde_json::from_value(params.definition.clone())
                        .map_err(|_| BackendFailure::Rejected)?;
                    validate_plan(&plan).map_err(|_| BackendFailure::Rejected)?;
                    if plan.result_root.starts_with(&self.state_root)
                        || plan.work_root.starts_with(&self.state_root)
                        || self.state_root.starts_with(&plan.result_root)
                        || self.state_root.starts_with(&plan.work_root)
                    {
                        return Err(BackendFailure::Rejected);
                    }
                    let bytes = serde_json::to_vec(&plan).map_err(|_| BackendFailure::Rejected)?;
                    let digest = format!("{:x}", Sha256::digest(bytes));
                    let plan_id = format!("plan-{}", &digest[..24]);
                    catalog.plans.entry(plan_id.clone()).or_insert(plan);
                    Self::append_event(catalog, ControlEventKind::PlanCreated, None)?;
                    Ok(ControlResult::Plan(PlanReference {
                        plan_id,
                        plan_sha256: digest,
                    }))
                },
            ),
            ControlCall::Launch(params) => self.launch(call, params, deadline),
            ControlCall::Status { run_id } => {
                let record = self.refresh(&run_id.0, deadline)?;
                let catalog = self
                    .catalog
                    .lock()
                    .map_err(|_| BackendFailure::NeedsReconciliation)?;
                self.bind(
                    call,
                    ControlResult::Status(Self::summary(&catalog, &record)?),
                )
            }
            ControlCall::Cancel(params) => {
                let request = BoundControlResult::new(
                    call,
                    ControlResult::Acknowledged(MutationAcknowledgement { accepted: true }),
                )
                .map_err(|_| BackendFailure::Rejected)?;
                let key_digest = format!("{:x}", Sha256::digest(params.idempotency_key.as_bytes()));
                {
                    let mut catalog = self
                        .catalog
                        .lock()
                        .map_err(|_| BackendFailure::NeedsReconciliation)?;
                    if let Some(record) = catalog.mutations.get(&key_digest) {
                        return match (
                            record.request_sha256 == request.request_sha256,
                            record.state,
                            record.result.as_ref(),
                        ) {
                            (true, MutationState::Committed, Some(result)) => Ok(result.clone()),
                            _ => Err(BackendFailure::NeedsReconciliation),
                        };
                    }
                    // Serialize the terminal-state check and durable cancellation intent
                    // with the worker's terminal transition. All nested paths acquire
                    // catalog before active, preventing lock inversion.
                    let active = self
                        .active
                        .lock()
                        .map_err(|_| BackendFailure::NeedsReconciliation)?;
                    let run = active
                        .get(&params.run_id.0)
                        .ok_or(BackendFailure::StaleIdentity)?;
                    if run.attempt_id != params.attempt_id.0
                        || !matches!(
                            catalog
                                .runs
                                .get(&params.run_id.0)
                                .map(|record| record.state),
                            Some(
                                PublicRunState::Planned
                                    | PublicRunState::Prepared
                                    | PublicRunState::Running
                                    | PublicRunState::Collecting
                            )
                        )
                    {
                        return Err(BackendFailure::StaleIdentity);
                    }
                    let mut staged = catalog.clone();
                    staged.mutations.insert(
                        key_digest.clone(),
                        MutationRecord {
                            request_sha256: request.request_sha256.clone(),
                            target: MutationTarget::Cancel {
                                run_id: params.run_id.0.clone(),
                                attempt_id: params.attempt_id.0.clone(),
                            },
                            state: MutationState::Intent,
                            result: Some(request.clone()),
                        },
                    );
                    deadline
                        .check()
                        .map_err(|_| BackendFailure::NeedsReconciliation)?;
                    commit_staged_catalog(&self.state_root, &mut catalog, staged)
                        .map_err(|_| BackendFailure::NeedsReconciliation)?;
                    deadline
                        .check()
                        .map_err(|_| BackendFailure::NeedsReconciliation)?;
                    // The intent is durable before cancellation becomes visible, and the
                    // worker cannot commit Completed ahead of this transition.
                    run.cancelled.store(true, Ordering::SeqCst);
                }
                let terminal = loop {
                    deadline
                        .check()
                        .map_err(|_| BackendFailure::NeedsReconciliation)?;
                    let state = self.refresh(&params.run_id.0, deadline)?.state;
                    if matches!(
                        state,
                        PublicRunState::Completed
                            | PublicRunState::Failed
                            | PublicRunState::Cancelled
                    ) {
                        break state;
                    }
                    thread::sleep(Duration::from_millis(5));
                };
                let mut catalog = self
                    .catalog
                    .lock()
                    .map_err(|_| BackendFailure::NeedsReconciliation)?;
                let mut staged = catalog.clone();
                if terminal != PublicRunState::Cancelled {
                    staged.mutations.remove(&key_digest);
                    deadline
                        .check()
                        .map_err(|_| BackendFailure::NeedsReconciliation)?;
                    commit_staged_catalog(&self.state_root, &mut catalog, staged)
                        .map_err(|_| BackendFailure::NeedsReconciliation)?;
                    return Err(BackendFailure::StaleIdentity);
                }
                staged.mutations.insert(
                    key_digest.clone(),
                    MutationRecord {
                        request_sha256: request.request_sha256.clone(),
                        target: MutationTarget::Cancel {
                            run_id: params.run_id.0.clone(),
                            attempt_id: params.attempt_id.0.clone(),
                        },
                        state: MutationState::Committed,
                        result: Some(request.clone()),
                    },
                );
                deadline
                    .check()
                    .map_err(|_| BackendFailure::NeedsReconciliation)?;
                commit_staged_catalog(&self.state_root, &mut catalog, staged)
                    .map_err(|_| BackendFailure::NeedsReconciliation)?;
                Ok(request)
            }
            ControlCall::History(page) => {
                let catalog = self
                    .catalog
                    .lock()
                    .map_err(|_| BackendFailure::NeedsReconciliation)?;
                let mut values = catalog
                    .runs
                    .values()
                    .map(|record| Ok((Self::summary(&catalog, record)?.created_revision, record)))
                    .collect::<Result<Vec<_>, BackendFailure>>()?;
                values.sort_by_key(|(created_revision, _)| *created_revision);
                if let Some(after) = page.after {
                    let oldest = values
                        .first()
                        .map_or(Revision(0), |(revision, _)| *revision);
                    let latest = values.last().map_or(Revision(0), |(revision, _)| *revision);
                    if after > latest || (oldest.0 > 0 && after.0.saturating_add(1) < oldest.0) {
                        return Err(BackendFailure::StaleCursor);
                    }
                }
                let filtered = values
                    .into_iter()
                    .filter(|(created_revision, _)| {
                        page.after.is_none_or(|after| *created_revision > after)
                    })
                    .collect::<Vec<_>>();
                let has_more = filtered.len() > usize::from(page.limit);
                let items = filtered
                    .into_iter()
                    .take(usize::from(page.limit))
                    .map(|(_, record)| Self::summary(&catalog, record))
                    .collect::<Result<Vec<_>, _>>()?;
                self.bind(
                    call,
                    ControlResult::History(Page {
                        next: items.last().map(|item| item.created_revision),
                        items,
                        has_more,
                    }),
                )
            }
            ControlCall::Repeat(params) => self.mutation(
                call,
                &params.idempotency_key,
                MutationTarget::Repeat {
                    run_id: params.run_id.0.clone(),
                },
                deadline,
                |catalog| {
                    let source = catalog
                        .runs
                        .get(&params.run_id.0)
                        .ok_or(BackendFailure::NotFound)?;
                    if source.state != PublicRunState::Completed {
                        return Err(BackendFailure::Rejected);
                    }
                    let mut plan = catalog
                        .plans
                        .get(&source.plan_id)
                        .cloned()
                        .ok_or(BackendFailure::NeedsReconciliation)?;
                    let suffix = format!(
                        "{:x}",
                        Sha256::digest(
                            BoundControlResult::new(
                                call,
                                ControlResult::Acknowledged(MutationAcknowledgement {
                                    accepted: true,
                                }),
                            )
                            .map_err(|_| BackendFailure::Rejected)?
                            .request_sha256
                        )
                    );
                    plan.run_id = format!("repeat-{}", &suffix[..24]);
                    validate_plan(&plan).map_err(|_| BackendFailure::Rejected)?;
                    let digest = format!(
                        "{:x}",
                        Sha256::digest(
                            serde_json::to_vec(&plan).map_err(|_| BackendFailure::Rejected)?
                        )
                    );
                    let plan_id = format!("plan-{}", &digest[..24]);
                    catalog.plans.insert(plan_id.clone(), plan);
                    Self::append_event(catalog, ControlEventKind::PlanCreated, None)?;
                    Ok(ControlResult::Plan(PlanReference {
                        plan_id,
                        plan_sha256: digest,
                    }))
                },
            ),
            ControlCall::Analyze { run_ids } => {
                let records = run_ids
                    .iter()
                    .map(|run_id| self.refresh(&run_id.0, deadline))
                    .collect::<Result<Vec<_>, _>>()?;
                let catalog = self
                    .catalog
                    .lock()
                    .map_err(|_| BackendFailure::NeedsReconciliation)?;
                let summaries = records
                    .iter()
                    .map(|record| Self::summary(&catalog, record))
                    .collect::<Result<Vec<_>, _>>()?;
                if summaries.iter().any(|summary| {
                    !matches!(
                        summary.state,
                        PublicRunState::Completed
                            | PublicRunState::Failed
                            | PublicRunState::Cancelled
                    )
                }) {
                    return Err(BackendFailure::Rejected);
                }
                let analysis =
                    serde_json::to_vec(&summaries).map_err(|_| BackendFailure::Rejected)?;
                let digest = format!("{:x}", Sha256::digest(&analysis));
                self.commit_analysis_before_deadline(&analysis, &digest, deadline)?;
                self.bind(
                    call,
                    ControlResult::Analysis(AnalysisSummary {
                        run_count: u16::try_from(summaries.len())
                            .map_err(|_| BackendFailure::Rejected)?,
                        analysis_sha256: digest,
                    }),
                )
            }
            ControlCall::Events(page) => {
                let catalog = self
                    .catalog
                    .lock()
                    .map_err(|_| BackendFailure::NeedsReconciliation)?;
                let filtered = catalog
                    .events
                    .iter()
                    .filter(|event| page.after.is_none_or(|after| event.revision > after))
                    .cloned()
                    .collect::<Vec<_>>();
                if let Some(after) = page.after {
                    let oldest = catalog
                        .events
                        .first()
                        .map_or(Revision(0), |event| event.revision);
                    let latest = catalog.revision;
                    if after > latest || (oldest.0 > 0 && after.0.saturating_add(1) < oldest.0) {
                        return Err(BackendFailure::StaleCursor);
                    }
                }
                let has_more = filtered.len() > usize::from(page.limit);
                let items = filtered
                    .into_iter()
                    .take(usize::from(page.limit))
                    .collect::<Vec<_>>();
                self.bind(
                    call,
                    ControlResult::Events(Page {
                        next: items.last().map(|item| item.revision),
                        items,
                        has_more,
                    }),
                )
            }
            ControlCall::ArtifactMetadata { run_id, digest } => {
                let catalog = self
                    .catalog
                    .lock()
                    .map_err(|_| BackendFailure::NeedsReconciliation)?;
                let record = catalog
                    .runs
                    .get(&run_id.0)
                    .ok_or(BackendFailure::NotFound)?;
                let plan = catalog
                    .plans
                    .get(&record.plan_id)
                    .ok_or(BackendFailure::NeedsReconciliation)?;
                let file = open_artifact_beneath(&plan.result_root, &run_id.0, digest)?;
                let size_bytes = file.metadata().map_err(|_| BackendFailure::Rejected)?.len();
                if size_bytes > StoreLimits::default().max_artifact_bytes {
                    return Err(BackendFailure::Rejected);
                }
                if digest_open_file(file)? != *digest {
                    return Err(BackendFailure::NeedsReconciliation);
                }
                self.bind(
                    call,
                    ControlResult::ArtifactMetadata(ArtifactMetadata {
                        sha256: digest.clone(),
                        size_bytes,
                        sensitivity: ArtifactSensitivity::Sensitive,
                    }),
                )
            }
            ControlCall::AuthEnroll(params) => self.mutation(
                call,
                &params.idempotency_key,
                MutationTarget::AuthEnroll {
                    provider: params.provider.clone(),
                },
                deadline,
                |catalog| {
                    if let Some(existing) = catalog.auth.get(&params.provider) {
                        if existing.endpoint_identity_sha256 != params.endpoint_identity_sha256
                            || existing.credential_locator_sha256
                                != params.credential_locator_sha256
                        {
                            return Err(BackendFailure::Rejected);
                        }
                        return Ok(ControlResult::Acknowledged(MutationAcknowledgement {
                            accepted: true,
                        }));
                    }
                    catalog.auth.insert(
                        params.provider.clone(),
                        AuthRecord {
                            provider: params.provider.clone(),
                            endpoint_identity_sha256: params.endpoint_identity_sha256.clone(),
                            credential_locator_sha256: params.credential_locator_sha256.clone(),
                            generation: 1,
                            status: "active".to_owned(),
                        },
                    );
                    Ok(ControlResult::Acknowledged(MutationAcknowledgement {
                        accepted: true,
                    }))
                },
            ),
            ControlCall::AuthStatus(params) => {
                let catalog = self
                    .catalog
                    .lock()
                    .map_err(|_| BackendFailure::NeedsReconciliation)?;
                let record = catalog
                    .auth
                    .get(&params.provider)
                    .ok_or(BackendFailure::Rejected)?;
                self.bind(
                    call,
                    ControlResult::AuthStatus(AuthStatusResponse {
                        provider: record.provider.clone(),
                        endpoint_identity_sha256: record.endpoint_identity_sha256.clone(),
                        credential_locator_sha256: record.credential_locator_sha256.clone(),
                        generation: record.generation,
                        status: record.status.clone(),
                    }),
                )
            }
            ControlCall::AuthRotate(params) => self.mutation(
                call,
                &params.idempotency_key,
                MutationTarget::AuthRotate {
                    provider: params.provider.clone(),
                },
                deadline,
                |catalog| {
                    let record = catalog
                        .auth
                        .get_mut(&params.provider)
                        .ok_or(BackendFailure::Rejected)?;
                    record.credential_locator_sha256 = params.credential_locator_sha256.clone();
                    record.generation = record
                        .generation
                        .checked_add(1)
                        .ok_or(BackendFailure::Rejected)?;
                    record.status = "active".to_owned();
                    Ok(ControlResult::Acknowledged(MutationAcknowledgement {
                        accepted: true,
                    }))
                },
            ),
            ControlCall::AuthRevoke(params) => self.mutation(
                call,
                &params.idempotency_key,
                MutationTarget::AuthRevoke {
                    provider: params.provider.clone(),
                },
                deadline,
                |catalog| {
                    let record = catalog
                        .auth
                        .get_mut(&params.provider)
                        .ok_or(BackendFailure::Rejected)?;
                    record.status = "revoked".to_owned();
                    Ok(ControlResult::Acknowledged(MutationAcknowledgement {
                        accepted: true,
                    }))
                },
            ),
            // Lifecycle storage and bundle verification are not wired into the
            // runner yet. Reject every operation explicitly so no caller can
            // observe a fabricated or partially active installation.
            ControlCall::AgentInstall(_)
            | ControlCall::AgentStatus(_)
            | ControlCall::AgentCancel(_)
            | ControlCall::AgentRetry(_)
            | ControlCall::AgentRemove(_) => Err(BackendFailure::CapabilityUnavailable),
            ControlCall::Negotiate(_) => Err(BackendFailure::Rejected),
        }
    }

    fn execute_versioned(
        &self,
        call: &ControlCall,
        deadline: RequestDeadline,
        version: ControlVersion,
    ) -> Result<BoundControlResult, BackendFailure> {
        let mut result = self.execute(call, deadline)?;
        if version < CONTROL_MEASUREMENT_SELECTION_V1
            && let ControlResult::SettingsValidation(validation) = &mut result.result
        {
            validation.issues = validation
                .issues
                .iter()
                .map(|issue| issue.legacy_projection())
                .collect();
            validation.measurement_issue = None;
        }
        Ok(result)
    }
}

impl RunnerBackend {
    fn recording_campaign_estimate(
        &self,
        request: &asb_control::RecordingCampaignEstimateRequest,
    ) -> Result<asb_control::RecordingCampaignEstimate, BackendFailure> {
        if request.runner_instance_id != self.runner_instance_id {
            return Err(BackendFailure::StaleIdentity);
        }
        let configuration = self.configuration_status(&ConfigurationStatusRequest {
            runner_instance_id: self.runner_instance_id.clone(),
        })?;
        let tuple_count = request
            .agent_ids
            .len()
            .checked_mul(request.workload_ids.len())
            .and_then(|count| u16::try_from(count).ok())
            .ok_or(BackendFailure::Rejected)?;
        let provider_catalog = self.provider_catalog(&ProviderCatalogRequest {
            action: ProviderCatalogAction::Status,
            runner_instance_id: self.runner_instance_id.clone(),
            known_generation: None,
        })?;
        let provider_ok = provider_catalog.providers.iter().any(|provider| {
            provider.provider_id == request.provider_id
                && matches!(&provider.availability, ProviderAvailability::Available)
                && provider.models.iter().any(|model| {
                    model.model_id == request.model_id
                        && matches!(&model.availability, ProviderAvailability::Available)
                })
        });
        let workloads_ok = request
            .workload_ids
            .iter()
            .all(|workload| OriginalWorkloads::describe(workload).is_ok());
        let configured_selection_matches = configuration.configured
            && configuration.provider_id.as_deref() == Some(request.provider_id.as_str())
            && configuration.model_id.as_deref() == Some(request.model_id.as_str())
            && configuration.agent_ids == request.agent_ids;
        let (complete_coverage, offline_ready, unavailable_reason) = if !configuration.configured {
            (false, false, Some("configuration-required".to_owned()))
        } else if !configured_selection_matches {
            (false, false, Some("configuration-mismatch".to_owned()))
        } else if !provider_ok {
            (false, false, Some("provider-model-unavailable".to_owned()))
        } else if !workloads_ok {
            (false, false, Some("workload-unavailable".to_owned()))
        } else {
            // An estimate never fabricates cassette coverage. Recording must
            // be launched explicitly and then reconciled before offline use.
            (false, false, Some("recording-required".to_owned()))
        };
        Ok(asb_control::RecordingCampaignEstimate {
            runner_instance_id: self.runner_instance_id.clone(),
            generation: configuration.generation,
            tuple_count,
            complete_coverage,
            offline_ready,
            unavailable_reason,
        })
    }

    fn recording_campaign_plan(
        &self,
        call: &ControlCall,
        params: &asb_control::RecordingCampaignPlanParams,
        deadline: RequestDeadline,
    ) -> Result<BoundControlResult, BackendFailure> {
        if params.runner_instance_id != self.runner_instance_id {
            return Err(BackendFailure::StaleIdentity);
        }
        let configuration = self.configuration_status(&ConfigurationStatusRequest {
            runner_instance_id: self.runner_instance_id.clone(),
        })?;
        if !configuration.configured
            || configuration.generation != params.expected_generation
            || configuration.provider_id.as_deref() != Some(params.provider_id.as_str())
            || configuration.model_id.as_deref() != Some(params.model_id.as_str())
            || configuration.agent_ids != params.agent_ids
        {
            return Err(BackendFailure::StaleIdentity);
        }
        let provider_catalog = self.provider_catalog(&ProviderCatalogRequest {
            action: ProviderCatalogAction::Status,
            runner_instance_id: self.runner_instance_id.clone(),
            known_generation: None,
        })?;
        let provider = provider_catalog
            .providers
            .iter()
            .find(|provider| provider.provider_id == params.provider_id)
            .ok_or(BackendFailure::Rejected)?;
        if !matches!(&provider.availability, ProviderAvailability::Available)
            || !provider.models.iter().any(|model| {
                model.model_id == params.model_id
                    && matches!(&model.availability, ProviderAvailability::Available)
            })
            || params
                .workload_ids
                .iter()
                .any(|workload| OriginalWorkloads::describe(workload).is_err())
        {
            return Err(BackendFailure::Rejected);
        }
        let tuple_count = params
            .agent_ids
            .len()
            .checked_mul(params.workload_ids.len())
            .and_then(|value| u16::try_from(value).ok())
            .ok_or(BackendFailure::Rejected)?;
        let request_digest = BoundControlResult::new(
            call,
            ControlResult::Acknowledged(MutationAcknowledgement { accepted: true }),
        )
        .map_err(|_| BackendFailure::Rejected)?
        .request_sha256;
        let campaign_id = format!("campaign-{}", &request_digest[..24]);
        let runner_instance_id = self.runner_instance_id.clone();
        let campaign_id_for_coverage = campaign_id.clone();
        let coverage = params
            .agent_ids
            .iter()
            .flat_map(|agent_id| {
                let campaign_id = campaign_id_for_coverage.clone();
                params.workload_ids.iter().map(move |workload_id| {
                    let scorer_revision = "scorer-v1".to_owned();
                    RecordingTupleRecord {
                        agent_id: agent_id.clone(),
                        workload_id: workload_id.clone(),
                        scorer_revision,
                        attempt_id: format!("{campaign_id}-{agent_id}-{workload_id}-attempt"),
                        generation: configuration.generation.0,
                        state: "ready".to_owned(),
                        cassette_sha256: None,
                        redaction_verified: false,
                        replay_verified: false,
                    }
                })
            })
            .collect::<Vec<_>>();
        self.mutation(
            call,
            &params.idempotency_key,
            MutationTarget::RecordingCampaignPlan,
            deadline,
            |catalog| {
                catalog.recording_campaign = Some(RecordingCampaignRecord {
                    campaign_id: campaign_id.clone(),
                    provider_id: params.provider_id.clone(),
                    model_id: params.model_id.clone(),
                    agent_ids: params.agent_ids.clone(),
                    workload_ids: params.workload_ids.clone(),
                    tuple_count,
                    generation: configuration.generation.0,
                    state: "planned".to_owned(),
                    covered_tuple_count: 0,
                    offline_ready: false,
                    unavailable_reason: Some("recording-required".to_owned()),
                    coverage: coverage.clone(),
                });
                Ok(ControlResult::RecordingCampaign(
                    asb_control::RecordingCampaignPlan {
                        runner_instance_id: runner_instance_id.clone(),
                        generation: configuration.generation,
                        campaign_id: campaign_id.clone(),
                        provider_id: params.provider_id.clone(),
                        model_id: params.model_id.clone(),
                        agent_ids: params.agent_ids.clone(),
                        workload_ids: params.workload_ids.clone(),
                        tuple_count,
                        state: "planned".to_owned(),
                        offline_ready: false,
                        unavailable_reason: Some("recording-required".to_owned()),
                    },
                ))
            },
        )
    }

    fn recording_campaign_status(
        &self,
        request: &asb_control::RecordingCampaignStatusRequest,
    ) -> Result<asb_control::RecordingCampaignStatus, BackendFailure> {
        if request.runner_instance_id != self.runner_instance_id {
            return Err(BackendFailure::StaleIdentity);
        }
        let catalog = self
            .catalog
            .lock()
            .map_err(|_| BackendFailure::NeedsReconciliation)?;
        let campaign =
            catalog
                .recording_campaign
                .as_ref()
                .map(|record| asb_control::RecordingCampaignPlan {
                    runner_instance_id: self.runner_instance_id.clone(),
                    generation: Revision(record.generation),
                    campaign_id: record.campaign_id.clone(),
                    provider_id: record.provider_id.clone(),
                    model_id: record.model_id.clone(),
                    agent_ids: record.agent_ids.clone(),
                    workload_ids: record.workload_ids.clone(),
                    tuple_count: record.tuple_count,
                    state: "planned".to_owned(),
                    offline_ready: false,
                    unavailable_reason: Some("recording-required".to_owned()),
                });
        let generation = catalog
            .configuration
            .as_ref()
            .map_or(Revision(1), |configuration| {
                Revision(configuration.generation)
            });
        Ok(asb_control::RecordingCampaignStatus {
            runner_instance_id: self.runner_instance_id.clone(),
            generation,
            campaign,
        })
    }

    fn recording_campaign_progress(
        &self,
        call: &ControlCall,
        request: &asb_control::RecordingCampaignProgressRequest,
    ) -> Result<BoundControlResult, BackendFailure> {
        if request.runner_instance_id != self.runner_instance_id {
            return Err(BackendFailure::StaleIdentity);
        }
        let catalog = self
            .catalog
            .lock()
            .map_err(|_| BackendFailure::NeedsReconciliation)?;
        let record = catalog
            .recording_campaign
            .as_ref()
            .filter(|record| record.campaign_id == request.campaign_id)
            .ok_or(BackendFailure::NotFound)?;
        self.bind(
            call,
            ControlResult::RecordingCampaignLifecycle(self.lifecycle_projection(record)),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn recording_campaign_lifecycle(
        &self,
        call: &ControlCall,
        key: &str,
        expected_generation: Revision,
        runner_instance_id: &str,
        campaign_id: &str,
        action: RecordingLifecycleAction,
        deadline: RequestDeadline,
    ) -> Result<BoundControlResult, BackendFailure> {
        if runner_instance_id != self.runner_instance_id {
            return Err(BackendFailure::StaleIdentity);
        }
        if matches!(action, RecordingLifecycleAction::Execute) {
            return self.recording_campaign_execute(
                call,
                key,
                expected_generation,
                campaign_id,
                deadline,
            );
        }
        let target = MutationTarget::RecordingCampaignLifecycle {
            campaign_id: campaign_id.to_owned(),
            action: action.name().to_owned(),
        };
        let runner_instance_id = self.runner_instance_id.clone();
        self.mutation(call, key, target, deadline, |catalog| {
            let record = catalog
                .recording_campaign
                .as_mut()
                .filter(|record| record.campaign_id == campaign_id)
                .ok_or(BackendFailure::NotFound)?;
            if record.generation != expected_generation.0 {
                return Err(BackendFailure::StaleIdentity);
            }
            match action {
                RecordingLifecycleAction::Execute => unreachable!("execute handled above"),
                RecordingLifecycleAction::Cancel if !record.offline_ready => {
                    record.state = "cancelled".to_owned();
                    record.unavailable_reason = Some("recording-cancelled".to_owned());
                }
                RecordingLifecycleAction::Cancel => return Err(BackendFailure::Rejected),
                RecordingLifecycleAction::Reconcile => {
                    if record.state == "recording" {
                        for entry in &mut record.coverage {
                            if entry.state == "in_progress" {
                                entry.state = "stale".to_owned();
                            }
                        }
                        record.state = "needs_reconciliation".to_owned();
                        record.unavailable_reason = Some("provider-capture-required".to_owned());
                    }
                }
                RecordingLifecycleAction::OfflineDefault => {
                    if !record.offline_ready || record.state != "complete" {
                        return Err(BackendFailure::CapabilityUnavailable);
                    }
                }
            }
            if matches!(action, RecordingLifecycleAction::OfflineDefault) {
                record.unavailable_reason = None;
            }
            let projection = asb_control::RecordingCampaignLifecycle {
                runner_instance_id: runner_instance_id.clone(),
                generation: Revision(record.generation),
                campaign_id: record.campaign_id.clone(),
                provider_id: record.provider_id.clone(),
                model_id: record.model_id.clone(),
                agent_ids: record.agent_ids.clone(),
                workload_ids: record.workload_ids.clone(),
                tuple_count: record.tuple_count,
                covered_tuple_count: record.covered_tuple_count,
                state: record.state.clone(),
                offline_ready: record.offline_ready,
                unavailable_reason: record.unavailable_reason.clone(),
            };
            Ok(ControlResult::RecordingCampaignLifecycle(projection))
        })
    }

    fn recording_campaign_execute(
        &self,
        call: &ControlCall,
        key: &str,
        expected_generation: Revision,
        campaign_id: &str,
        deadline: RequestDeadline,
    ) -> Result<BoundControlResult, BackendFailure> {
        let campaign = self
            .catalog
            .lock()
            .map_err(|_| BackendFailure::NeedsReconciliation)?
            .recording_campaign
            .clone()
            .filter(|record| record.campaign_id == campaign_id)
            .ok_or(BackendFailure::NotFound)?;
        if campaign.generation != expected_generation.0 {
            return Err(BackendFailure::StaleIdentity);
        }
        if campaign.state != "planned" {
            return if campaign.state == "recording" {
                Err(BackendFailure::NeedsReconciliation)
            } else if campaign.state == "complete" {
                self.bind(
                    call,
                    ControlResult::RecordingCampaignLifecycle(self.lifecycle_projection(&campaign)),
                )
            } else {
                Err(BackendFailure::Rejected)
            };
        }
        let start_target = MutationTarget::RecordingCampaignLifecycle {
            campaign_id: campaign_id.to_owned(),
            action: "execute_intent".to_owned(),
        };
        let started = self.mutation(call, key, start_target, deadline, |catalog| {
            let record = catalog
                .recording_campaign
                .as_mut()
                .filter(|record| record.campaign_id == campaign_id)
                .ok_or(BackendFailure::NotFound)?;
            if record.generation != expected_generation.0 || record.state != "planned" {
                return Err(BackendFailure::StaleIdentity);
            }
            for entry in &mut record.coverage {
                entry.state = "in_progress".to_owned();
                entry.cassette_sha256 = None;
                entry.redaction_verified = false;
                entry.replay_verified = false;
            }
            record.state = "recording".to_owned();
            record.covered_tuple_count = 0;
            record.offline_ready = false;
            record.unavailable_reason = Some("provider-capture-in-progress".to_owned());
            Ok(ControlResult::RecordingCampaignLifecycle(
                self.lifecycle_projection(record),
            ))
        })?;

        let mut captures = Vec::<ProviderCaptureResult>::with_capacity(campaign.coverage.len());
        let mut failure = None;
        for entry in &campaign.coverage {
            let request = ProviderCaptureRequest {
                provider_profile_sha256: recording_tuple_digest(
                    &campaign.provider_id,
                    &campaign.model_id,
                    &entry.agent_id,
                    &entry.workload_id,
                    &entry.scorer_revision,
                    campaign.generation,
                ),
                agent_id: entry.agent_id.clone(),
                workload_id: entry.workload_id.clone(),
                scorer_revision: entry.scorer_revision.clone(),
                attempt_id: entry.attempt_id.clone(),
                generation: campaign.generation,
            };
            match self.provider_capture.capture(&request) {
                Ok(result)
                    if result.redaction_verified
                        && result.replay_verified
                        && asb_control::validate_digest(&result.cassette_sha256).is_ok() =>
                {
                    captures.push(result);
                }
                Ok(_) => {
                    failure = Some(ProviderCaptureError::Verification);
                    break;
                }
                Err(error) => {
                    failure = Some(error);
                    break;
                }
            }
        }
        let final_key = format!("{key}:capture");
        let final_target = MutationTarget::RecordingCampaignLifecycle {
            campaign_id: campaign_id.to_owned(),
            action: "execute_result".to_owned(),
        };
        let finalized = self.mutation(call, &final_key, final_target, deadline, |catalog| {
            let record = catalog
                .recording_campaign
                .as_mut()
                .filter(|record| record.campaign_id == campaign_id)
                .ok_or(BackendFailure::NotFound)?;
            if record.generation != expected_generation.0 || record.state != "recording" {
                return Err(BackendFailure::NeedsReconciliation);
            }
            if let Some(error) = failure {
                for entry in &mut record.coverage {
                    if entry.state == "in_progress" {
                        entry.state = "failed".to_owned();
                    }
                }
                record.state = "failed".to_owned();
                record.offline_ready = false;
                record.covered_tuple_count = 0;
                record.unavailable_reason = Some(
                    match error {
                        ProviderCaptureError::Unavailable => "runtime-capture-unavailable",
                        ProviderCaptureError::IdentityMismatch => {
                            "runtime-capture-identity-mismatch"
                        }
                        ProviderCaptureError::Bounds => "runtime-capture-bounds",
                        ProviderCaptureError::Verification => "runtime-capture-verification-failed",
                    }
                    .to_owned(),
                );
            } else {
                for (entry, result) in record.coverage.iter_mut().zip(captures.iter()) {
                    entry.state = "complete".to_owned();
                    entry.cassette_sha256 = Some(result.cassette_sha256.clone());
                    entry.redaction_verified = result.redaction_verified;
                    entry.replay_verified = result.replay_verified;
                }
                record.covered_tuple_count = record.tuple_count;
                record.state = "complete".to_owned();
                record.offline_ready = recording_coverage_complete(record);
                record.unavailable_reason = if record.offline_ready {
                    None
                } else {
                    Some("coverage-verification-incomplete".to_owned())
                };
            }
            Ok(ControlResult::RecordingCampaignLifecycle(
                self.lifecycle_projection(record),
            ))
        })?;
        if failure.is_some() {
            return Err(
                if matches!(failure, Some(ProviderCaptureError::Unavailable)) {
                    BackendFailure::CapabilityUnavailable
                } else {
                    BackendFailure::Rejected
                },
            );
        }
        let _ = started;
        Ok(finalized)
    }

    fn lifecycle_projection(
        &self,
        record: &RecordingCampaignRecord,
    ) -> asb_control::RecordingCampaignLifecycle {
        asb_control::RecordingCampaignLifecycle {
            runner_instance_id: self.runner_instance_id.clone(),
            generation: Revision(record.generation),
            campaign_id: record.campaign_id.clone(),
            provider_id: record.provider_id.clone(),
            model_id: record.model_id.clone(),
            agent_ids: record.agent_ids.clone(),
            workload_ids: record.workload_ids.clone(),
            tuple_count: record.tuple_count,
            covered_tuple_count: record.covered_tuple_count,
            state: record.state.clone(),
            offline_ready: record.offline_ready,
            unavailable_reason: record.unavailable_reason.clone(),
        }
    }

    fn configuration_apply(
        &self,
        call: &ControlCall,
        params: &asb_control::ConfigurationApplyParams,
        deadline: RequestDeadline,
    ) -> Result<BoundControlResult, BackendFailure> {
        let runner_instance_id = self.runner_instance_id.clone();
        let provider_catalog = self.provider_catalog(&ProviderCatalogRequest {
            action: ProviderCatalogAction::Status,
            runner_instance_id: runner_instance_id.clone(),
            known_generation: None,
        })?;
        let provider = provider_catalog
            .providers
            .iter()
            .find(|provider| provider.provider_id == params.selection.provider_id)
            .ok_or(BackendFailure::Rejected)?;
        if !matches!(&provider.availability, ProviderAvailability::Available)
            || !provider
                .auth_methods
                .contains(&params.selection.auth_method)
            || !provider.models.iter().any(|model| {
                model.model_id == params.selection.model_id
                    && matches!(&model.availability, ProviderAvailability::Available)
            })
        {
            return Err(BackendFailure::Rejected);
        }
        self.mutation(
            call,
            &params.idempotency_key,
            MutationTarget::ConfigurationApply,
            deadline,
            |catalog| {
                let actual = catalog
                    .configuration
                    .as_ref()
                    .map_or(1, |configuration| configuration.generation);
                if actual != params.expected_generation.0 {
                    return Err(BackendFailure::StaleIdentity);
                }
                let next_generation = actual.checked_add(1).ok_or(BackendFailure::Rejected)?;
                catalog.configuration = Some(ConfigurationRecord {
                    agent_ids: params.selection.agent_ids.clone(),
                    provider_id: params.selection.provider_id.clone(),
                    model_id: params.selection.model_id.clone(),
                    auth_method: params.selection.auth_method,
                    credential_reference_sha256: params
                        .selection
                        .credential_reference_sha256
                        .clone(),
                    generation: next_generation,
                });
                Ok(ControlResult::Configuration(ConfigurationSnapshot {
                    runner_instance_id: runner_instance_id.clone(),
                    generation: Revision(next_generation),
                    configured: true,
                    agent_ids: params.selection.agent_ids.clone(),
                    provider_id: Some(params.selection.provider_id.clone()),
                    model_id: Some(params.selection.model_id.clone()),
                    auth_method: Some(params.selection.auth_method),
                    credential_reference_sha256: params
                        .selection
                        .credential_reference_sha256
                        .clone(),
                }))
            },
        )
    }

    fn configuration_status(
        &self,
        request: &ConfigurationStatusRequest,
    ) -> Result<ConfigurationSnapshot, BackendFailure> {
        if request.runner_instance_id != self.runner_instance_id {
            return Err(BackendFailure::StaleIdentity);
        }
        let catalog = self
            .catalog
            .lock()
            .map_err(|_| BackendFailure::NeedsReconciliation)?;
        let Some(configuration) = &catalog.configuration else {
            return Ok(ConfigurationSnapshot {
                runner_instance_id: self.runner_instance_id.clone(),
                generation: Revision(1),
                configured: false,
                agent_ids: Vec::new(),
                provider_id: None,
                model_id: None,
                auth_method: None,
                credential_reference_sha256: None,
            });
        };
        Ok(ConfigurationSnapshot {
            runner_instance_id: self.runner_instance_id.clone(),
            generation: Revision(configuration.generation),
            configured: true,
            agent_ids: configuration.agent_ids.clone(),
            provider_id: Some(configuration.provider_id.clone()),
            model_id: Some(configuration.model_id.clone()),
            auth_method: Some(configuration.auth_method),
            credential_reference_sha256: configuration.credential_reference_sha256.clone(),
        })
    }

    fn provider_catalog(
        &self,
        request: &ProviderCatalogRequest,
    ) -> Result<ProviderCatalog, BackendFailure> {
        if matches!(request.action, ProviderCatalogAction::Refresh) {
            // Discovery and connectivity probing must be supplied by a
            // verified provider registry; never label a static projection as
            // refreshed or connected.
            return Err(BackendFailure::CapabilityUnavailable);
        }
        if request.runner_instance_id != self.runner_instance_id {
            return Err(BackendFailure::StaleIdentity);
        }
        let generation = self
            .catalog
            .lock()
            .map_err(|_| BackendFailure::NeedsReconciliation)?
            .provider_generation;
        if request
            .known_generation
            .is_some_and(|known| known.0 > generation)
        {
            return Err(BackendFailure::StaleIdentity);
        }
        let mut catalog = ProviderCatalog {
            runner_instance_id: self.runner_instance_id.clone(),
            generation: Revision(generation),
            catalog_sha256: String::new(),
            providers: vec![
                ProviderCatalogEntry {
                    provider_id: "ollama".into(),
                    display_name: "Ollama".into(),
                    auth_methods: vec![ProviderAuthMethod::LocalDaemon, ProviderAuthMethod::None],
                    models: vec![ProviderModel {
                        model_id: asb_agents::ollama::OLLAMA_MODEL.into(),
                        revision: "local-daemon".into(),
                        availability: ProviderAvailability::Unavailable(
                            "verified-daemon-unavailable".into(),
                        ),
                    }],
                    availability: ProviderAvailability::Unavailable(
                        "verified-daemon-unavailable".into(),
                    ),
                },
                ProviderCatalogEntry {
                    provider_id: "openai".into(),
                    display_name: "OpenAI".into(),
                    auth_methods: vec![ProviderAuthMethod::CredentialReference],
                    models: vec![ProviderModel {
                        model_id: asb_agents::openai::OPENAI_MODEL.into(),
                        revision: "provider-catalog-v1".into(),
                        availability: ProviderAvailability::Available,
                    }],
                    availability: ProviderAvailability::Available,
                },
            ],
            refreshed: false,
        };
        let custom = self
            .catalog
            .lock()
            .map_err(|_| BackendFailure::NeedsReconciliation)?
            .provider_profiles
            .values()
            .cloned()
            .collect::<Vec<_>>();
        catalog.providers.extend(custom);
        catalog
            .providers
            .sort_by(|left, right| left.provider_id.cmp(&right.provider_id));
        catalog.catalog_sha256 = catalog
            .computed_sha256()
            .map_err(|_| BackendFailure::Rejected)?;
        Ok(catalog)
    }

    fn provider_profile_upsert(
        &self,
        call: &ControlCall,
        params: &asb_control::ProviderProfileUpsertParams,
        deadline: RequestDeadline,
    ) -> Result<BoundControlResult, BackendFailure> {
        if params.runner_instance_id != self.runner_instance_id {
            return Err(BackendFailure::StaleIdentity);
        }
        params.validate().map_err(|_| BackendFailure::Rejected)?;
        let runner_instance_id = self.runner_instance_id.clone();
        let mut base_snapshot = self.provider_catalog(&ProviderCatalogRequest {
            action: ProviderCatalogAction::Status,
            runner_instance_id: runner_instance_id.clone(),
            known_generation: None,
        })?;
        base_snapshot
            .providers
            .retain(|entry| entry.provider_id != params.entry.provider_id);
        base_snapshot.providers.push(params.entry.clone());
        base_snapshot
            .providers
            .sort_by(|left, right| left.provider_id.cmp(&right.provider_id));
        self.mutation(
            call,
            &params.idempotency_key,
            MutationTarget::ProviderProfileUpsert {
                provider_id: params.entry.provider_id.clone(),
            },
            deadline,
            |catalog| {
                if catalog.provider_generation != params.expected_generation.0 {
                    return Err(BackendFailure::StaleIdentity);
                }
                let next_generation = catalog
                    .provider_generation
                    .checked_add(1)
                    .ok_or(BackendFailure::Rejected)?;
                catalog
                    .provider_profiles
                    .insert(params.entry.provider_id.clone(), params.entry.clone());
                if let Some(digest) = &params.credential_reference_sha256 {
                    catalog
                        .provider_credential_references
                        .insert(params.entry.provider_id.clone(), digest.clone());
                } else {
                    catalog
                        .provider_credential_references
                        .remove(&params.entry.provider_id);
                }
                catalog.provider_generation = next_generation;
                let mut snapshot = base_snapshot.clone();
                snapshot.generation = Revision(next_generation);
                snapshot.catalog_sha256 = snapshot
                    .computed_sha256()
                    .map_err(|_| BackendFailure::Rejected)?;
                Ok(ControlResult::ProviderProfile(snapshot))
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use asb_control::{ControlServer, ControlSuccess, ProviderProfileUpsertParams};
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt, symlink};
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct FixtureProviderCapture {
        calls: AtomicUsize,
    }

    impl ProviderCapture for FixtureProviderCapture {
        fn capture(
            &self,
            request: &ProviderCaptureRequest,
        ) -> Result<ProviderCaptureResult, ProviderCaptureError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            assert!(request.attempt_id.contains(&request.agent_id));
            Ok(ProviderCaptureResult {
                cassette_sha256: "a".repeat(64),
                redaction_verified: true,
                replay_verified: true,
            })
        }
    }

    struct ErrorProviderCapture {
        error: ProviderCaptureError,
    }

    impl ProviderCapture for ErrorProviderCapture {
        fn capture(
            &self,
            _request: &ProviderCaptureRequest,
        ) -> Result<ProviderCaptureResult, ProviderCaptureError> {
            Err(self.error)
        }
    }

    struct InvalidResultProviderCapture {
        result: ProviderCaptureResult,
    }

    impl ProviderCapture for InvalidResultProviderCapture {
        fn capture(
            &self,
            _request: &ProviderCaptureRequest,
        ) -> Result<ProviderCaptureResult, ProviderCaptureError> {
            Ok(self.result.clone())
        }
    }

    struct Scratch(PathBuf, fs::File, fs::File, std::ffi::OsString);

    impl std::ops::Deref for Scratch {
        type Target = Path;

        fn deref(&self) -> &Path {
            &self.0
        }
    }

    impl Scratch {
        fn new() -> Self {
            let base = test_scratch_base().unwrap();
            create_test_scratch(&base, "runner").unwrap()
        }

        fn cleanup_with_hook(&mut self, hook: impl FnOnce()) {
            let Ok(identity) = self.1.metadata() else {
                return;
            };
            let anchored = fd_path(&self.2).join(&self.3);
            let Ok(metadata) = fs::symlink_metadata(&anchored) else {
                return;
            };
            if !metadata.is_dir()
                || metadata.file_type().is_symlink()
                || metadata.dev() != identity.dev()
                || metadata.ino() != identity.ino()
            {
                return;
            }
            hook();
            let _ = remove_directory_contents(&self.1);
            let Ok(current) = fs::symlink_metadata(&anchored) else {
                return;
            };
            if current.dev() == identity.dev() && current.ino() == identity.ino() {
                let _ = fs::remove_dir(&anchored);
            }
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            self.cleanup_with_hook(|| {});
        }
    }

    fn fd_path(file: &fs::File) -> PathBuf {
        PathBuf::from(format!("/proc/self/fd/{}", file.as_raw_fd()))
    }

    fn open_directory(path: &Path) -> std::io::Result<fs::File> {
        fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(path)
    }

    fn remove_directory_contents(directory: &fs::File) -> std::io::Result<()> {
        let anchored = fd_path(directory);
        for entry in fs::read_dir(&anchored)? {
            let entry = entry?;
            let path = anchored.join(entry.file_name());
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                let child = open_directory(&path)?;
                let identity = child.metadata()?;
                remove_directory_contents(&child)?;
                let current = fs::symlink_metadata(&path)?;
                if current.dev() == identity.dev() && current.ino() == identity.ino() {
                    fs::remove_dir(&path)?;
                }
            } else {
                fs::remove_file(&path)?;
            }
        }
        Ok(())
    }

    fn contains_symlink(path: &Path) -> bool {
        let mut current = PathBuf::new();
        for component in path.components() {
            current.push(component.as_os_str());
            if fs::symlink_metadata(&current)
                .is_ok_and(|metadata| metadata.file_type().is_symlink())
            {
                return true;
            }
        }
        false
    }

    fn workspace_root() -> Result<PathBuf, &'static str> {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .ok_or("workspace root is unavailable")?
            .canonicalize()
            .map_err(|_| "workspace root is not canonical")
    }

    fn open_test_base(path: &Path) -> Result<(PathBuf, fs::File), &'static str> {
        if !path.is_absolute() || contains_symlink(path) {
            return Err("test scratch base is not an absolute real path");
        }
        let identity = open_directory(path).map_err(|_| "test scratch base is not a directory")?;
        let metadata = identity
            .metadata()
            .map_err(|_| "test scratch base identity is unavailable")?;
        if !metadata.is_dir() {
            return Err("test scratch base is not a directory");
        }
        let uid = fs::metadata("/proc/self")
            .map_err(|_| "current process identity is unavailable")?
            .uid();
        let mode = metadata.permissions().mode() & 0o7777;
        let private_owned = metadata.uid() == uid && mode & 0o022 == 0;
        let sticky_public = (metadata.uid() == 0 || metadata.uid() == uid) && mode & 0o1000 != 0;
        if !private_owned && !sticky_public {
            return Err("test scratch base ownership or mode is unsafe");
        }
        let canonical = fs::canonicalize(fd_path(&identity))
            .map_err(|_| "test scratch base is not canonical")?;
        let repository = workspace_root()?;
        if canonical.starts_with(&repository) || repository.starts_with(&canonical) {
            return Err("test scratch base overlaps the repository");
        }
        Ok((canonical, identity))
    }

    fn validate_test_base(path: &Path) -> Result<PathBuf, &'static str> {
        open_test_base(path).map(|(canonical, _)| canonical)
    }

    fn test_scratch_base() -> Result<PathBuf, &'static str> {
        let configured = std::env::var_os("ASB_TEST_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        validate_test_base(&configured)
    }

    fn validate_test_scratch(path: &Path, expected_uid: u32) -> Result<fs::Metadata, &'static str> {
        let metadata = fs::symlink_metadata(path).map_err(|_| "test scratch root is missing")?;
        if !path.is_absolute()
            || metadata.file_type().is_symlink()
            || !metadata.is_dir()
            || metadata.uid() != expected_uid
            || metadata.permissions().mode() & 0o777 != 0o700
        {
            return Err("test scratch root identity is unsafe");
        }
        let canonical = fs::canonicalize(path).map_err(|_| "test scratch root is not canonical")?;
        let repository = fs::canonicalize(env!("CARGO_MANIFEST_DIR"))
            .map_err(|_| "repository root is not canonical")?;
        if canonical != path
            || canonical.starts_with(&repository)
            || repository.starts_with(&canonical)
        {
            return Err("test scratch root aliases the repository");
        }
        Ok(metadata)
    }

    fn create_test_scratch(base: &Path, label: &str) -> Result<Scratch, &'static str> {
        let base = validate_test_base(base)?;
        let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "system clock precedes epoch")?
            .as_nanos();
        create_test_scratch_with_identity(&base, label, sequence, epoch)
    }

    fn create_test_scratch_with_identity(
        base: &Path,
        label: &str,
        sequence: u64,
        epoch: u128,
    ) -> Result<Scratch, &'static str> {
        create_test_scratch_with_identity_and_hook(base, label, sequence, epoch, || {})
    }

    fn create_test_scratch_with_identity_and_hook(
        base: &Path,
        label: &str,
        sequence: u64,
        epoch: u128,
        hook: impl FnOnce(),
    ) -> Result<Scratch, &'static str> {
        let (_, base_identity) = open_test_base(base)?;
        hook();
        for attempt in 0..128_u8 {
            let name = std::ffi::OsString::from(format!(
                ".asb-control-{label}-{}-{sequence}-{epoch}-{attempt}",
                std::process::id()
            ));
            let anchored = fd_path(&base_identity).join(&name);
            match fs::DirBuilder::new().mode(0o700).create(&anchored) {
                Ok(()) => {
                    let uid = fs::metadata("/proc/self")
                        .map_err(|_| "current process identity is unavailable")?
                        .uid();
                    let identity = open_directory(&anchored)
                        .map_err(|_| "test scratch identity cannot be retained")?;
                    let path = fs::canonicalize(fd_path(&identity))
                        .map_err(|_| "test scratch root is not canonical")?;
                    let metadata = validate_test_scratch(&path, uid)?;
                    let retained = identity
                        .metadata()
                        .map_err(|_| "test scratch identity cannot be read")?;
                    if (metadata.dev(), metadata.ino()) != (retained.dev(), retained.ino()) {
                        return Err("test scratch identity changed during creation");
                    }
                    return Ok(Scratch(path, identity, base_identity, name));
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(_) => return Err("test scratch root cannot be created"),
            }
        }
        Err("test scratch collision limit exceeded")
    }

    #[test]
    fn test_scratch_roots_are_unique_private_and_symlink_safe() {
        let base = validate_test_base(&std::env::temp_dir()).unwrap();
        let container = create_test_scratch(&base, "container").unwrap();
        assert!(validate_test_base(Path::new(".")).is_err());
        assert!(validate_test_base(&workspace_root().unwrap().join("crates")).is_err());
        let regular_file = container.join("not-a-directory");
        fs::write(&regular_file, b"unchanged").unwrap();
        assert!(validate_test_base(&regular_file).is_err());
        let repository_alias = container.join("repository-alias");
        symlink(env!("CARGO_MANIFEST_DIR"), &repository_alias).unwrap();
        assert!(validate_test_base(&repository_alias).is_err());

        let sequence = u64::MAX;
        let epoch = 1_u128;
        let collision = container.join(format!(
            ".asb-control-collision-{}-{sequence}-{epoch}-0",
            std::process::id()
        ));
        fs::create_dir(&collision).unwrap();
        fs::write(collision.join("must-remain"), b"stale").unwrap();
        let root =
            create_test_scratch_with_identity(&container, "collision", sequence, epoch).unwrap();
        assert_ne!(root.0, collision);
        assert_eq!(fs::read(collision.join("must-remain")).unwrap(), b"stale");

        let leaf_symlink = container.join(format!(
            ".asb-control-leaf-symlink-{}-{sequence}-{epoch}-0",
            std::process::id()
        ));
        symlink(env!("CARGO_MANIFEST_DIR"), &leaf_symlink).unwrap();
        let symlink_safe =
            create_test_scratch_with_identity(&container, "leaf-symlink", sequence, epoch).unwrap();
        assert_ne!(symlink_safe.0, leaf_symlink);
        assert!(
            fs::symlink_metadata(&leaf_symlink)
                .unwrap()
                .file_type()
                .is_symlink()
        );

        let metadata = fs::symlink_metadata(&root.0).unwrap();
        assert_eq!(metadata.permissions().mode() & 0o777, 0o700);
        assert!(validate_test_scratch(&root.0, metadata.uid().wrapping_add(1)).is_err());
        fs::set_permissions(&root.0, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(validate_test_scratch(&root.0, metadata.uid()).is_err());
        fs::set_permissions(&root.0, fs::Permissions::from_mode(0o700)).unwrap();

        let policy_base = container.join("policy-base");
        fs::create_dir(&policy_base).unwrap();
        fs::set_permissions(&policy_base, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(validate_test_base(&policy_base).is_err());
        fs::set_permissions(&policy_base, fs::Permissions::from_mode(0o1777)).unwrap();
        assert!(validate_test_base(&policy_base).is_ok());

        let swapped_base = container.join("swapped-base");
        let retained_base = container.join("retained-base");
        fs::create_dir(&swapped_base).unwrap();
        fs::set_permissions(&swapped_base, fs::Permissions::from_mode(0o700)).unwrap();
        let anchored = create_test_scratch_with_identity_and_hook(
            &swapped_base,
            "ancestor-swap",
            sequence,
            epoch,
            || {
                fs::rename(&swapped_base, &retained_base).unwrap();
                symlink(workspace_root().unwrap(), &swapped_base).unwrap();
            },
        )
        .unwrap();
        assert!(anchored.0.starts_with(&retained_base));
        assert!(
            !workspace_root()
                .unwrap()
                .join(anchored.0.file_name().unwrap())
                .exists()
        );

        let mut substituted = create_test_scratch(&container, "substituted").unwrap();
        let substituted_path = substituted.0.clone();
        substituted.cleanup_with_hook(|| {
            fs::remove_dir(&substituted_path).unwrap();
            fs::create_dir(&substituted_path).unwrap();
            fs::write(substituted_path.join("must-remain"), b"replacement").unwrap();
        });
        drop(substituted);
        assert_eq!(
            fs::read(substituted_path.join("must-remain")).unwrap(),
            b"replacement"
        );
    }

    #[test]
    fn parallel_scratch_creation_never_reuses_a_private_root() {
        let base = validate_test_base(&std::env::temp_dir()).unwrap();
        let roots = std::thread::scope(|scope| {
            let workers = (0..32)
                .map(|_| {
                    let base = base.clone();
                    scope.spawn(move || {
                        let scratch = create_test_scratch(&base, "parallel").unwrap();
                        let metadata = fs::symlink_metadata(&scratch.0).unwrap();
                        assert_eq!(metadata.permissions().mode() & 0o777, 0o700);
                        (scratch, metadata.dev(), metadata.ino())
                    })
                })
                .collect::<Vec<_>>();
            workers
                .into_iter()
                .map(|worker| worker.join().unwrap())
                .collect::<Vec<_>>()
        });

        let mut paths = std::collections::HashSet::new();
        let mut identities = std::collections::HashSet::new();
        for (scratch, device, inode) in roots {
            assert!(paths.insert(scratch.0.clone()));
            assert!(identities.insert((device, inode)));
        }
        assert_eq!(paths.len(), 32);
        assert_eq!(identities.len(), 32);
    }

    fn fixture_plan(root: &Path) -> PlanFile {
        fixture_plan_with_script(
            root,
            b"#!/bin/sh\nprintf '%s\\n' 'def parse_line(line):' '    if line.endswith(\"\\r\"):' '        line = line[:-1]' '    return line' > parser.py\n",
        )
    }

    fn fixture_plan_with_script(root: &Path, script: &[u8]) -> PlanFile {
        let executable = root.join("fixture-agent");
        fs::write(&executable, script).unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        let executable = fs::canonicalize(executable).unwrap();
        let executable_sha256 = digest_file(&executable).unwrap();
        let mut experiment: ExperimentManifestV1 = serde_json::from_str(include_str!(
            "../../asb-protocol/fixtures/v1/experiment-manifest.json"
        ))
        .unwrap();
        let workload = OriginalWorkloads::describe("original.bug-fix").unwrap();
        experiment.agent.binary_sha256 = executable_sha256.clone();
        experiment.workload.workload = workload.workload_id.0;
        experiment.workload.workload_revision = workload.version;
        experiment.workload.workload_sha256 = workload.content_sha256;
        experiment.workload.scorer_revision = workload.scoring_version;
        experiment.platform.architecture = std::env::consts::ARCH.to_owned();
        experiment.refresh_content_address().unwrap();
        let measurement_selection =
            super::process_measurement_selection(experiment.controls.replay.mode, 5_000_000)
                .unwrap();
        PlanFile {
            schema_version: PLAN_SCHEMA_VERSION,
            run_id: "control-real-run".into(),
            result_root: root.join("results"),
            work_root: root.join("work"),
            workload: "original.bug-fix".into(),
            agent: BatchAgent {
                executable,
                executable_sha256,
                arguments: Vec::new(),
            },
            point: PointInput {
                measured: 1,
                warmups: 0,
                concurrency: 1,
                queue: 0,
                max_failures: 0,
                timeout_ms: 5_000,
                poll_ms: 5,
                seed: 7,
                open_loop_interval_ms: None,
                sweep_max_concurrency: None,
            },
            experiment,
            measurement_selection: Some(measurement_selection),
        }
    }

    fn deadline() -> RequestDeadline {
        RequestDeadline::start(10_000).unwrap()
    }

    #[test]
    fn backend_returns_the_exact_builtin_measurement_catalog_over_v1_2() {
        let scratch = Scratch::new();
        let state = scratch.0.join("state");
        prepare_root(&state).unwrap();
        let backend = open_backend(state).unwrap();

        let publication = backend
            .execute(&ControlCall::MeasurementCatalog, deadline())
            .unwrap();
        publication
            .validate_for_call(&ControlCall::MeasurementCatalog, ControlLimits::default())
            .unwrap();
        let ControlResult::MeasurementCatalog(publication) = publication.result else {
            panic!("measurement catalog result");
        };
        assert_eq!(publication.catalog.0, baseline_measurement_catalog());

        let socket = scratch.0.join("measurement-catalog.sock");
        let mut server = ControlServer::bind(&socket, ControlLimits::default(), backend).unwrap();
        let service = thread::spawn(move || server.serve_one());
        let mut client = asb_control::ControlClient::connect_with_versions(
            &socket,
            ControlLimits::default(),
            [asb_control::CONTROL_MEASUREMENT_CATALOG_V1],
        )
        .expect("connect catalog client");
        let response = client
            .call(ControlCall::MeasurementCatalog, 5_000)
            .expect("catalog response")
            .into_result()
            .expect("successful result");
        let ControlSuccess::Operation(response) = response else {
            panic!("operation result");
        };
        let ControlResult::MeasurementCatalog(publication) = response.result else {
            panic!("measurement catalog result");
        };
        assert_eq!(publication.catalog.0, baseline_measurement_catalog());
        drop(client);
        service.join().unwrap().unwrap();
    }

    #[test]
    fn agent_catalog_is_explicitly_unavailable_until_verified_provider_is_wired() {
        let scratch = Scratch::new();
        let state = scratch.0.join("state");
        prepare_root(&state).unwrap();
        let backend = open_backend(state).unwrap();
        let call = ControlCall::AgentCatalog(asb_control::AgentCatalogRequest {
            action: asb_control::AgentCatalogAction::Status,
            runner_instance_id: backend.runner_instance_id().to_owned(),
            known_generation: None,
        });
        assert_eq!(
            backend.execute(&call, deadline()),
            Err(BackendFailure::CapabilityUnavailable)
        );
    }

    #[test]
    fn provider_catalog_exposes_models_and_auth_methods_without_credentials() {
        let scratch = Scratch::new();
        let state = scratch.0.join("state");
        prepare_root(&state).unwrap();
        let backend = open_backend(state).unwrap();
        let call = ControlCall::ProviderCatalog(asb_control::ProviderCatalogRequest {
            action: asb_control::ProviderCatalogAction::Status,
            runner_instance_id: backend.runner_instance_id().to_owned(),
            known_generation: None,
        });
        let result = backend.execute(&call, deadline()).unwrap();
        result
            .validate_for_call(&call, ControlLimits::default())
            .unwrap();
        let ControlResult::ProviderCatalog(catalog) = result.result else {
            panic!("provider catalog result");
        };
        assert_eq!(catalog.providers.len(), 2);
        assert_eq!(catalog.providers[0].provider_id, "ollama");
        assert_eq!(catalog.providers[1].provider_id, "openai");
        assert_eq!(
            catalog.providers[1].models[0].model_id,
            asb_agents::openai::OPENAI_MODEL
        );
        assert!(matches!(
            catalog.providers[0].availability,
            asb_control::ProviderAvailability::Unavailable(_)
        ));
        let encoded = serde_json::to_string(&catalog).unwrap();
        assert!(!encoded.contains("api_key"));
        assert!(!encoded.contains("sk-"));
        let refresh = ControlCall::ProviderCatalog(asb_control::ProviderCatalogRequest {
            action: asb_control::ProviderCatalogAction::Refresh,
            runner_instance_id: backend.runner_instance_id().to_owned(),
            known_generation: None,
        });
        assert_eq!(
            backend.execute(&refresh, deadline()),
            Err(BackendFailure::CapabilityUnavailable)
        );

        let socket = scratch.0.join("provider-catalog.sock");
        let mut server = ControlServer::bind(&socket, ControlLimits::default(), backend).unwrap();
        let service = thread::spawn(move || server.serve_one());
        let mut client = asb_control::ControlClient::connect_with_versions(
            &socket,
            ControlLimits::default(),
            [asb_control::CONTROL_PROVIDER_CATALOG_V1],
        )
        .expect("connect provider catalog client");
        let runner_instance_id = client.negotiated().runner_instance_id.clone();
        let response = client
            .call(
                ControlCall::ProviderCatalog(asb_control::ProviderCatalogRequest {
                    action: asb_control::ProviderCatalogAction::Status,
                    runner_instance_id,
                    known_generation: None,
                }),
                5_000,
            )
            .expect("provider catalog response")
            .into_result()
            .expect("successful result");
        assert!(
            matches!(response, ControlSuccess::Operation(operation) if matches!(operation.result, ControlResult::ProviderCatalog(_)))
        );
        drop(client);
        service.join().unwrap().unwrap();
    }

    #[test]
    fn provider_profile_upsert_persists_metadata_and_stays_unauthorized() {
        let scratch = Scratch::new();
        let state = scratch.0.join("state");
        prepare_root(&state).unwrap();
        let backend = open_backend(state.clone()).unwrap();
        let runner_instance_id = backend.runner_instance_id().to_owned();
        let entry = ProviderCatalogEntry {
            provider_id: "custom".into(),
            display_name: "Custom".into(),
            auth_methods: vec![ProviderAuthMethod::CredentialReference],
            models: vec![ProviderModel {
                model_id: "custom-model".into(),
                revision: "catalog-1".into(),
                availability: ProviderAvailability::Unavailable("authorization-required".into()),
            }],
            availability: ProviderAvailability::Unavailable("authorization-required".into()),
        };
        let call = ControlCall::ProviderProfileUpsert(ProviderProfileUpsertParams {
            idempotency_key: "profile-1".into(),
            expected_generation: Revision(1),
            runner_instance_id: runner_instance_id.clone(),
            entry,
            credential_reference_sha256: Some("c".repeat(64)),
        });
        let result = backend.execute(&call, deadline()).unwrap();
        result
            .validate_for_call(&call, ControlLimits::default())
            .unwrap();
        let ControlResult::ProviderProfile(snapshot) = &result.result else {
            panic!("provider profile result");
        };
        assert_eq!(snapshot.generation, Revision(2));
        let custom = snapshot
            .providers
            .iter()
            .find(|provider| provider.provider_id == "custom")
            .expect("custom profile");
        assert!(matches!(
            custom.availability,
            ProviderAvailability::Unavailable(_)
        ));
        assert!(matches!(
            custom.models[0].availability,
            ProviderAvailability::Unavailable(_)
        ));
        let retry = backend.execute(&call, deadline()).unwrap();
        assert_eq!(retry.request_sha256, result.request_sha256);

        let stale = ControlCall::ProviderProfileUpsert(ProviderProfileUpsertParams {
            idempotency_key: "profile-2".into(),
            expected_generation: Revision(1),
            runner_instance_id,
            entry: custom.clone(),
            credential_reference_sha256: Some("c".repeat(64)),
        });
        assert_eq!(
            backend.execute(&stale, deadline()),
            Err(BackendFailure::StaleIdentity)
        );
        let bytes = fs::read(state.join("control-catalog.json")).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("api_key"));
        assert!(!String::from_utf8_lossy(&bytes).contains("sk-"));
    }

    #[test]
    fn provider_readiness_remains_unavailable_without_verified_probe() {
        let scratch = Scratch::new();
        let state = scratch.0.join("state");
        prepare_root(&state).unwrap();
        let backend = open_backend(state).unwrap();
        let runner_instance_id = backend.runner_instance_id().to_owned();

        let enroll = ControlCall::AuthEnroll(asb_control::AuthEnrollParams {
            provider: "openai".into(),
            endpoint_identity_sha256: "a".repeat(64),
            credential_locator_sha256: "b".repeat(64),
            idempotency_key: "readiness-enroll".into(),
        });
        backend.execute(&enroll, deadline()).unwrap();

        // Enrollment is metadata-only. The returned lifecycle state is not
        // provider authorization evidence.
        let status = backend
            .execute(
                &ControlCall::AuthStatus(asb_control::AuthStatusParams {
                    provider: "openai".into(),
                }),
                deadline(),
            )
            .unwrap();
        let ControlResult::AuthStatus(status) = status.result else {
            panic!("auth status result");
        };
        assert_eq!(status.endpoint_identity_sha256, "a".repeat(64));
        assert_eq!(status.credential_locator_sha256, "b".repeat(64));
        assert_eq!(status.status, "active");

        // A static catalog and enrolled metadata cannot be upgraded into a
        // connectivity/authorization claim while the verified probe producer
        // is absent.
        let refresh = ControlCall::ProviderCatalog(asb_control::ProviderCatalogRequest {
            action: asb_control::ProviderCatalogAction::Refresh,
            runner_instance_id,
            known_generation: None,
        });
        assert_eq!(
            backend.execute(&refresh, deadline()),
            Err(BackendFailure::CapabilityUnavailable)
        );
    }

    #[test]
    fn configuration_status_is_explicitly_unconfigured_and_generation_bound() {
        let scratch = Scratch::new();
        let state = scratch.0.join("state");
        prepare_root(&state).unwrap();
        let backend = open_backend(state).unwrap();
        let call = ControlCall::ConfigurationStatus(asb_control::ConfigurationStatusRequest {
            runner_instance_id: backend.runner_instance_id().to_owned(),
        });
        let result = backend.execute(&call, deadline()).unwrap();
        result
            .validate_for_call(&call, ControlLimits::default())
            .unwrap();
        let ControlResult::Configuration(snapshot) = result.result else {
            panic!("configuration result");
        };
        assert!(!snapshot.configured);
        assert_eq!(snapshot.generation, Revision(1));
        assert!(snapshot.provider_id.is_none());
        assert!(snapshot.credential_reference_sha256.is_none());
    }

    #[test]
    fn configuration_apply_is_idempotent_and_generation_fenced() {
        let scratch = Scratch::new();
        let state = scratch.0.join("state");
        prepare_root(&state).unwrap();
        let backend = open_backend(state.clone()).unwrap();
        let runner_instance_id = backend.runner_instance_id().to_owned();
        let selection = asb_control::ConfigurationSelection {
            agent_ids: vec!["aider".into()],
            provider_id: "openai".into(),
            model_id: asb_agents::openai::OPENAI_MODEL.into(),
            auth_method: asb_control::ProviderAuthMethod::CredentialReference,
            credential_reference_sha256: Some("a".repeat(64)),
        };
        let call = ControlCall::ConfigurationApply(asb_control::ConfigurationApplyParams {
            idempotency_key: "configure-1".into(),
            expected_generation: Revision(1),
            selection,
        });
        let first = backend.execute(&call, deadline()).unwrap();
        first
            .validate_for_call(&call, ControlLimits::default())
            .unwrap();
        let second = backend.execute(&call, deadline()).unwrap();
        assert_eq!(first, second);
        let stale = ControlCall::ConfigurationApply(asb_control::ConfigurationApplyParams {
            idempotency_key: "configure-2".into(),
            expected_generation: Revision(1),
            selection: match &call {
                ControlCall::ConfigurationApply(params) => params.selection.clone(),
                _ => unreachable!(),
            },
        });
        assert_eq!(
            backend.execute(&stale, deadline()),
            Err(BackendFailure::StaleIdentity)
        );
        let status = backend
            .execute(
                &ControlCall::ConfigurationStatus(asb_control::ConfigurationStatusRequest {
                    runner_instance_id,
                }),
                deadline(),
            )
            .unwrap();
        let ControlResult::Configuration(snapshot) = status.result else {
            panic!("configuration status result");
        };
        assert!(snapshot.configured);
        assert_eq!(snapshot.generation, Revision(2));

        // Configuration is a durable default, not merely an in-memory wizard
        // draft.  A fresh backend instance must expose the same privacy-safe
        // selection and generation after restart.
        drop(backend);
        let restarted = open_backend(state).unwrap();
        let restarted_status = restarted
            .execute(
                &ControlCall::ConfigurationStatus(asb_control::ConfigurationStatusRequest {
                    runner_instance_id: restarted.runner_instance_id().to_owned(),
                }),
                deadline(),
            )
            .unwrap();
        let ControlResult::Configuration(restarted_snapshot) = restarted_status.result else {
            panic!("restarted configuration result");
        };
        assert_eq!(restarted_snapshot.generation, Revision(2));
        assert_eq!(restarted_snapshot.agent_ids, vec!["aider"]);
        assert_eq!(restarted_snapshot.provider_id.as_deref(), Some("openai"));
        assert_eq!(
            restarted_snapshot.model_id.as_deref(),
            Some(asb_agents::openai::OPENAI_MODEL)
        );
        assert_eq!(
            restarted_snapshot.credential_reference_sha256,
            Some("a".repeat(64))
        );
    }

    #[test]
    fn recording_estimate_covers_all_current_workloads_without_fabricating_offline_ready() {
        let scratch = Scratch::new();
        let state = scratch.0.join("state");
        prepare_root(&state).unwrap();
        let backend = open_backend(state).unwrap();
        backend
            .execute(
                &ControlCall::ConfigurationApply(asb_control::ConfigurationApplyParams {
                    idempotency_key: "recording-config".into(),
                    expected_generation: Revision(1),
                    selection: asb_control::ConfigurationSelection {
                        agent_ids: vec!["aider".into()],
                        provider_id: "openai".into(),
                        model_id: asb_agents::openai::OPENAI_MODEL.into(),
                        auth_method: asb_control::ProviderAuthMethod::CredentialReference,
                        credential_reference_sha256: Some("b".repeat(64)),
                    },
                }),
                deadline(),
            )
            .unwrap();
        let call =
            ControlCall::RecordingCampaignEstimate(asb_control::RecordingCampaignEstimateRequest {
                runner_instance_id: backend.runner_instance_id().to_owned(),
                provider_id: "openai".into(),
                model_id: asb_agents::openai::OPENAI_MODEL.into(),
                agent_ids: vec!["aider".into()],
                workload_ids: OriginalWorkloads::fixture_ids()
                    .iter()
                    .map(|id| (*id).to_owned())
                    .collect(),
            });
        let result = backend.execute(&call, deadline()).unwrap();
        result
            .validate_for_call(&call, ControlLimits::default())
            .unwrap();
        let ControlResult::RecordingCampaignEstimate(estimate) = result.result else {
            panic!("recording estimate result");
        };
        assert_eq!(estimate.tuple_count, 7);
        assert!(!estimate.complete_coverage);
        assert!(!estimate.offline_ready);
        assert_eq!(
            estimate.unavailable_reason.as_deref(),
            Some("recording-required")
        );

        let mismatched =
            ControlCall::RecordingCampaignEstimate(asb_control::RecordingCampaignEstimateRequest {
                runner_instance_id: backend.runner_instance_id().to_owned(),
                provider_id: "openai".into(),
                model_id: asb_agents::openai::OPENAI_MODEL.into(),
                agent_ids: vec!["codex".into()],
                workload_ids: OriginalWorkloads::fixture_ids()
                    .iter()
                    .map(|id| (*id).to_owned())
                    .collect(),
            });
        let ControlResult::RecordingCampaignEstimate(mismatch) =
            backend.execute(&mismatched, deadline()).unwrap().result
        else {
            panic!("mismatched recording estimate result");
        };
        assert_eq!(
            mismatch.unavailable_reason.as_deref(),
            Some("configuration-mismatch")
        );
    }

    #[test]
    fn recording_estimate_rejects_stale_unconfigured_and_invalid_requests() {
        let scratch = Scratch::new();
        let state = scratch.0.join("state");
        prepare_root(&state).unwrap();
        let backend = open_backend(state).unwrap();
        let runner = backend.runner_instance_id().to_owned();
        let estimate = |runner_instance_id: &str, agent_ids: Vec<&str>, workload_ids: Vec<&str>| {
            ControlCall::RecordingCampaignEstimate(asb_control::RecordingCampaignEstimateRequest {
                runner_instance_id: runner_instance_id.to_owned(),
                provider_id: "openai".into(),
                model_id: asb_agents::openai::OPENAI_MODEL.into(),
                agent_ids: agent_ids.into_iter().map(str::to_owned).collect(),
                workload_ids: workload_ids.into_iter().map(str::to_owned).collect(),
            })
        };

        assert_eq!(
            backend.execute(
                &estimate("other-runner", vec!["aider"], vec!["original.bug-fix"]),
                deadline()
            ),
            Err(BackendFailure::StaleIdentity)
        );
        let unconfigured = backend
            .execute(
                &estimate(&runner, vec!["aider"], vec!["original.bug-fix"]),
                deadline(),
            )
            .unwrap();
        let ControlResult::RecordingCampaignEstimate(unconfigured) = unconfigured.result else {
            panic!("unconfigured recording estimate result");
        };
        assert_eq!(
            unconfigured.unavailable_reason.as_deref(),
            Some("configuration-required")
        );

        backend
            .execute(
                &ControlCall::ConfigurationApply(asb_control::ConfigurationApplyParams {
                    idempotency_key: "recording-estimate-config".into(),
                    expected_generation: Revision(1),
                    selection: asb_control::ConfigurationSelection {
                        agent_ids: vec!["aider".into()],
                        provider_id: "openai".into(),
                        model_id: asb_agents::openai::OPENAI_MODEL.into(),
                        auth_method: asb_control::ProviderAuthMethod::CredentialReference,
                        credential_reference_sha256: Some("e".repeat(64)),
                    },
                }),
                deadline(),
            )
            .unwrap();
        let invalid = backend
            .execute(
                &estimate(&runner, vec!["aider"], vec!["unknown.workload"]),
                deadline(),
            )
            .unwrap();
        let ControlResult::RecordingCampaignEstimate(invalid) = invalid.result else {
            panic!("invalid recording estimate result");
        };
        assert_eq!(
            invalid.unavailable_reason.as_deref(),
            Some("workload-unavailable")
        );
    }

    #[test]
    fn recording_campaign_plan_is_durable_idempotent_and_not_offline_ready() {
        let scratch = Scratch::new();
        let state = scratch.0.join("state");
        prepare_root(&state).unwrap();
        let backend = open_backend(state.clone()).unwrap();
        let runner_instance_id = backend.runner_instance_id().to_owned();
        backend
            .execute(
                &ControlCall::ConfigurationApply(asb_control::ConfigurationApplyParams {
                    idempotency_key: "campaign-config".into(),
                    expected_generation: Revision(1),
                    selection: asb_control::ConfigurationSelection {
                        agent_ids: vec!["aider".into()],
                        provider_id: "openai".into(),
                        model_id: asb_agents::openai::OPENAI_MODEL.into(),
                        auth_method: asb_control::ProviderAuthMethod::CredentialReference,
                        credential_reference_sha256: Some("c".repeat(64)),
                    },
                }),
                deadline(),
            )
            .unwrap();
        let call = ControlCall::RecordingCampaignPlan(asb_control::RecordingCampaignPlanParams {
            idempotency_key: "campaign-plan".into(),
            expected_generation: Revision(2),
            runner_instance_id,
            provider_id: "openai".into(),
            model_id: asb_agents::openai::OPENAI_MODEL.into(),
            agent_ids: vec!["aider".into()],
            workload_ids: vec![
                "original.bug-fix".into(),
                "original.feature-addition".into(),
            ],
        });
        let first = backend.execute(&call, deadline()).unwrap();
        first
            .validate_for_call(&call, ControlLimits::default())
            .unwrap();
        let second = backend.execute(&call, deadline()).unwrap();
        assert_eq!(first, second);
        let ControlResult::RecordingCampaign(plan) = first.result else {
            panic!("campaign plan result");
        };
        assert_eq!(plan.tuple_count, 2);
        assert_eq!(plan.state, "planned");
        assert!(!plan.offline_ready);
        assert_eq!(
            plan.unavailable_reason.as_deref(),
            Some("recording-required")
        );

        drop(backend);
        let restarted = open_backend(state).unwrap();
        let status = restarted
            .execute(
                &ControlCall::ConfigurationStatus(asb_control::ConfigurationStatusRequest {
                    runner_instance_id: restarted.runner_instance_id().to_owned(),
                }),
                deadline(),
            )
            .unwrap();
        assert!(
            matches!(status.result, ControlResult::Configuration(snapshot) if snapshot.configured)
        );
        let campaign_status_call =
            ControlCall::RecordingCampaignStatus(asb_control::RecordingCampaignStatusRequest {
                runner_instance_id: restarted.runner_instance_id().to_owned(),
            });
        let campaign_status = restarted
            .execute(&campaign_status_call, deadline())
            .unwrap();
        campaign_status
            .validate_for_call(&campaign_status_call, ControlLimits::default())
            .unwrap();
        let ControlResult::RecordingCampaignStatus(campaign_status) = campaign_status.result else {
            panic!("campaign status result");
        };
        assert!(campaign_status.campaign.is_some());
    }

    #[test]
    fn recording_campaign_status_is_empty_before_a_plan() {
        let scratch = Scratch::new();
        let state = scratch.0.join("state");
        prepare_root(&state).unwrap();
        let backend = open_backend(state).unwrap();
        let call =
            ControlCall::RecordingCampaignStatus(asb_control::RecordingCampaignStatusRequest {
                runner_instance_id: backend.runner_instance_id().to_owned(),
            });

        let result = backend.execute(&call, deadline()).unwrap();
        result
            .validate_for_call(&call, ControlLimits::default())
            .unwrap();
        let ControlResult::RecordingCampaignStatus(status) = result.result else {
            panic!("campaign status result");
        };
        assert!(status.campaign.is_none());
    }

    #[test]
    fn recording_campaign_lifecycle_transitions_are_durable_and_fail_closed() {
        let scratch = Scratch::new();
        let state = scratch.0.join("state");
        prepare_root(&state).unwrap();
        let backend = open_backend(state).unwrap();
        let runner = backend.runner_instance_id().to_owned();
        backend
            .execute(
                &ControlCall::ConfigurationApply(asb_control::ConfigurationApplyParams {
                    idempotency_key: "lifecycle-config".into(),
                    expected_generation: Revision(1),
                    selection: asb_control::ConfigurationSelection {
                        agent_ids: vec!["aider".into()],
                        provider_id: "openai".into(),
                        model_id: asb_agents::openai::OPENAI_MODEL.into(),
                        auth_method: asb_control::ProviderAuthMethod::CredentialReference,
                        credential_reference_sha256: Some("d".repeat(64)),
                    },
                }),
                deadline(),
            )
            .unwrap();
        let plan = backend
            .execute(
                &ControlCall::RecordingCampaignPlan(asb_control::RecordingCampaignPlanParams {
                    idempotency_key: "lifecycle-plan".into(),
                    expected_generation: Revision(2),
                    runner_instance_id: runner.clone(),
                    provider_id: "openai".into(),
                    model_id: asb_agents::openai::OPENAI_MODEL.into(),
                    agent_ids: vec!["aider".into()],
                    workload_ids: vec!["original.bug-fix".into()],
                }),
                deadline(),
            )
            .unwrap();
        let ControlResult::RecordingCampaign(plan) = plan.result else {
            panic!("campaign plan result");
        };
        let execute = |call: ControlCall| {
            backend.execute(&call, deadline()).inspect(|result| {
                result
                    .validate_for_call(&call, ControlLimits::default())
                    .unwrap();
            })
        };
        let execute_call =
            ControlCall::RecordingCampaignExecute(asb_control::RecordingCampaignExecuteParams {
                idempotency_key: "lifecycle-execute".into(),
                expected_generation: Revision(2),
                runner_instance_id: runner.clone(),
                campaign_id: plan.campaign_id.clone(),
            });
        assert_eq!(
            execute(execute_call.clone()),
            Err(BackendFailure::CapabilityUnavailable)
        );
        assert_eq!(execute(execute_call.clone()), Err(BackendFailure::Rejected));

        let stale_runner_execute = asb_control::RecordingCampaignExecuteParams {
            idempotency_key: "lifecycle-stale-runner".into(),
            expected_generation: Revision(2),
            runner_instance_id: "other-runner".into(),
            campaign_id: plan.campaign_id.clone(),
        };
        assert_eq!(
            backend.execute(
                &ControlCall::RecordingCampaignExecute(stale_runner_execute),
                deadline()
            ),
            Err(BackendFailure::StaleIdentity)
        );
        let missing_execute = asb_control::RecordingCampaignExecuteParams {
            idempotency_key: "lifecycle-missing".into(),
            expected_generation: Revision(2),
            runner_instance_id: backend.runner_instance_id().into(),
            campaign_id: "campaign-missing".into(),
        };
        assert_eq!(
            backend.execute(
                &ControlCall::RecordingCampaignExecute(missing_execute),
                deadline()
            ),
            Err(BackendFailure::NotFound)
        );
        let stale_generation = asb_control::RecordingCampaignExecuteParams {
            idempotency_key: "lifecycle-stale-generation".into(),
            expected_generation: Revision(1),
            runner_instance_id: backend.runner_instance_id().into(),
            campaign_id: plan.campaign_id.clone(),
        };
        assert_eq!(
            backend.execute(
                &ControlCall::RecordingCampaignExecute(stale_generation),
                deadline()
            ),
            Err(BackendFailure::StaleIdentity)
        );

        let progress_call =
            ControlCall::RecordingCampaignProgress(asb_control::RecordingCampaignProgressRequest {
                runner_instance_id: runner.clone(),
                campaign_id: plan.campaign_id.clone(),
            });
        let progress = execute(progress_call).unwrap();
        assert!(matches!(
            progress.result,
            ControlResult::RecordingCampaignLifecycle(ref value) if value.state == "failed"
        ));

        let wrong_runner =
            ControlCall::RecordingCampaignProgress(asb_control::RecordingCampaignProgressRequest {
                runner_instance_id: "other-runner".into(),
                campaign_id: plan.campaign_id.clone(),
            });
        assert_eq!(
            backend.execute(&wrong_runner, deadline()),
            Err(BackendFailure::StaleIdentity)
        );
        let missing_campaign =
            ControlCall::RecordingCampaignProgress(asb_control::RecordingCampaignProgressRequest {
                runner_instance_id: backend.runner_instance_id().into(),
                campaign_id: "campaign-missing".into(),
            });
        assert_eq!(
            backend.execute(&missing_campaign, deadline()),
            Err(BackendFailure::NotFound)
        );

        let unavailable_offline = ControlCall::RecordingCampaignOfflineDefault(
            asb_control::RecordingCampaignOfflineDefaultParams {
                idempotency_key: "lifecycle-offline-before-capture".into(),
                expected_generation: Revision(2),
                runner_instance_id: backend.runner_instance_id().into(),
                campaign_id: plan.campaign_id.clone(),
            },
        );
        assert_eq!(
            backend.execute(&unavailable_offline, deadline()),
            Err(BackendFailure::CapabilityUnavailable)
        );

        let reconcile_call = ControlCall::RecordingCampaignReconcile(
            asb_control::RecordingCampaignReconcileParams {
                idempotency_key: "lifecycle-reconcile".into(),
                expected_generation: Revision(2),
                runner_instance_id: runner.clone(),
                campaign_id: plan.campaign_id.clone(),
            },
        );
        let reconciled = execute(reconcile_call.clone()).unwrap();
        assert!(matches!(
            reconciled.result,
            ControlResult::RecordingCampaignLifecycle(ref value)
                if value.state == "failed"
        ));
        // Reconciliation is idempotent once a runtime capture failure is
        // durably recorded; it never retries the provider effect.
        assert_eq!(execute(reconcile_call.clone()).unwrap(), reconciled);

        let cancel_call =
            ControlCall::RecordingCampaignCancel(asb_control::RecordingCampaignCancelParams {
                idempotency_key: "lifecycle-cancel".into(),
                expected_generation: Revision(2),
                runner_instance_id: runner,
                campaign_id: plan.campaign_id.clone(),
            });
        let cancelled = execute(cancel_call.clone()).unwrap();
        assert!(matches!(
            cancelled.result,
            ControlResult::RecordingCampaignLifecycle(ref value) if value.state == "cancelled"
        ));
        assert_eq!(execute(cancel_call.clone()).unwrap(), cancelled);

        {
            let mut catalog = backend.catalog.lock().unwrap();
            let record = catalog.recording_campaign.as_mut().unwrap();
            for entry in &mut record.coverage {
                entry.state = "complete".into();
                entry.cassette_sha256 = Some("a".repeat(64));
                entry.redaction_verified = true;
                entry.replay_verified = true;
            }
            record.state = "complete".into();
            record.covered_tuple_count = record.tuple_count;
            record.offline_ready = true;
            record.unavailable_reason = None;
            commit_catalog(&backend.state_root, &catalog).unwrap();
        }
        let cancel_complete =
            ControlCall::RecordingCampaignCancel(asb_control::RecordingCampaignCancelParams {
                idempotency_key: "lifecycle-cancel-complete".into(),
                expected_generation: Revision(2),
                runner_instance_id: backend.runner_instance_id().into(),
                campaign_id: plan.campaign_id.clone(),
            });
        assert_eq!(
            backend.execute(&cancel_complete, deadline()),
            Err(BackendFailure::Rejected)
        );
        let offline_ready = ControlCall::RecordingCampaignOfflineDefault(
            asb_control::RecordingCampaignOfflineDefaultParams {
                idempotency_key: "lifecycle-offline-ready".into(),
                expected_generation: Revision(2),
                runner_instance_id: backend.runner_instance_id().into(),
                campaign_id: plan.campaign_id.clone(),
            },
        );
        let activated = execute(offline_ready).unwrap();
        assert!(matches!(
            activated.result,
            ControlResult::RecordingCampaignLifecycle(ref value)
                if value.state == "complete" && value.offline_ready && value.unavailable_reason.is_none()
        ));
    }

    #[test]
    fn runtime_capture_callback_completes_exact_tuple_matrix() {
        let scratch = Scratch::new();
        let state = scratch.0.join("state");
        prepare_root(&state).unwrap();
        let capture = Arc::new(FixtureProviderCapture {
            calls: AtomicUsize::new(0),
        });
        let backend = open_backend_with_capture(state, capture.clone()).unwrap();
        let runner = backend.runner_instance_id().to_owned();
        backend
            .execute(
                &ControlCall::ConfigurationApply(asb_control::ConfigurationApplyParams {
                    idempotency_key: "capture-config".into(),
                    expected_generation: Revision(1),
                    selection: asb_control::ConfigurationSelection {
                        agent_ids: vec!["aider".into()],
                        provider_id: "openai".into(),
                        model_id: asb_agents::openai::OPENAI_MODEL.into(),
                        auth_method: asb_control::ProviderAuthMethod::CredentialReference,
                        credential_reference_sha256: Some("d".repeat(64)),
                    },
                }),
                deadline(),
            )
            .unwrap();
        let plan = backend
            .execute(
                &ControlCall::RecordingCampaignPlan(asb_control::RecordingCampaignPlanParams {
                    idempotency_key: "capture-plan".into(),
                    expected_generation: Revision(2),
                    runner_instance_id: runner.clone(),
                    provider_id: "openai".into(),
                    model_id: asb_agents::openai::OPENAI_MODEL.into(),
                    agent_ids: vec!["aider".into()],
                    workload_ids: vec!["original.bug-fix".into()],
                }),
                deadline(),
            )
            .unwrap();
        let ControlResult::RecordingCampaign(plan) = plan.result else {
            panic!("campaign plan result");
        };
        let execute =
            ControlCall::RecordingCampaignExecute(asb_control::RecordingCampaignExecuteParams {
                idempotency_key: "capture-execute".into(),
                expected_generation: Revision(2),
                runner_instance_id: runner,
                campaign_id: plan.campaign_id,
            });
        let result = backend.execute(&execute, deadline()).unwrap();
        assert!(matches!(
            result.result,
            ControlResult::RecordingCampaignLifecycle(ref value)
                if value.state == "complete" && value.offline_ready && value.covered_tuple_count == 1
        ));
        assert_eq!(capture.calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn runtime_capture_failures_remain_typed_and_fail_closed() {
        for (suffix, error) in [
            ("identity", ProviderCaptureError::IdentityMismatch),
            ("bounds", ProviderCaptureError::Bounds),
            ("verification", ProviderCaptureError::Verification),
        ] {
            let scratch = Scratch::new();
            let state = scratch.0.join("state");
            prepare_root(&state).unwrap();
            let backend =
                open_backend_with_capture(state, Arc::new(ErrorProviderCapture { error })).unwrap();
            let runner = backend.runner_instance_id().to_owned();
            backend
                .execute(
                    &ControlCall::ConfigurationApply(asb_control::ConfigurationApplyParams {
                        idempotency_key: format!("capture-error-config-{suffix}"),
                        expected_generation: Revision(1),
                        selection: asb_control::ConfigurationSelection {
                            agent_ids: vec!["aider".into()],
                            provider_id: "openai".into(),
                            model_id: asb_agents::openai::OPENAI_MODEL.into(),
                            auth_method: asb_control::ProviderAuthMethod::CredentialReference,
                            credential_reference_sha256: Some("d".repeat(64)),
                        },
                    }),
                    deadline(),
                )
                .unwrap();
            let plan = backend
                .execute(
                    &ControlCall::RecordingCampaignPlan(asb_control::RecordingCampaignPlanParams {
                        idempotency_key: format!("capture-error-plan-{suffix}"),
                        expected_generation: Revision(2),
                        runner_instance_id: runner.clone(),
                        provider_id: "openai".into(),
                        model_id: asb_agents::openai::OPENAI_MODEL.into(),
                        agent_ids: vec!["aider".into()],
                        workload_ids: vec!["original.bug-fix".into()],
                    }),
                    deadline(),
                )
                .unwrap();
            let ControlResult::RecordingCampaign(plan) = plan.result else {
                panic!("campaign plan result");
            };
            let execute = ControlCall::RecordingCampaignExecute(
                asb_control::RecordingCampaignExecuteParams {
                    idempotency_key: format!("capture-error-execute-{suffix}"),
                    expected_generation: Revision(2),
                    runner_instance_id: runner,
                    campaign_id: plan.campaign_id,
                },
            );
            assert_eq!(
                backend.execute(&execute, deadline()),
                Err(BackendFailure::Rejected)
            );
            let catalog = backend.catalog.lock().unwrap();
            let record = catalog.recording_campaign.as_ref().unwrap();
            assert_eq!(record.state, "failed");
            assert_eq!(record.covered_tuple_count, 0);
            assert!(!record.offline_ready);
            assert_eq!(
                record.unavailable_reason.as_deref(),
                Some(match error {
                    ProviderCaptureError::IdentityMismatch => "runtime-capture-identity-mismatch",
                    ProviderCaptureError::Bounds => "runtime-capture-bounds",
                    ProviderCaptureError::Verification => "runtime-capture-verification-failed",
                    ProviderCaptureError::Unavailable => unreachable!(),
                })
            );
        }
    }

    #[test]
    fn runtime_capture_rejects_unverified_or_malformed_results() {
        for (suffix, result) in [
            (
                "redaction",
                ProviderCaptureResult {
                    cassette_sha256: "a".repeat(64),
                    redaction_verified: false,
                    replay_verified: true,
                },
            ),
            (
                "replay",
                ProviderCaptureResult {
                    cassette_sha256: "a".repeat(64),
                    redaction_verified: true,
                    replay_verified: false,
                },
            ),
            (
                "digest",
                ProviderCaptureResult {
                    cassette_sha256: "not-a-digest".into(),
                    redaction_verified: true,
                    replay_verified: true,
                },
            ),
        ] {
            let scratch = Scratch::new();
            let state = scratch.0.join("state");
            prepare_root(&state).unwrap();
            let backend =
                open_backend_with_capture(state, Arc::new(InvalidResultProviderCapture { result }))
                    .unwrap();
            let runner = backend.runner_instance_id().to_owned();
            backend
                .execute(
                    &ControlCall::ConfigurationApply(asb_control::ConfigurationApplyParams {
                        idempotency_key: format!("capture-invalid-config-{suffix}"),
                        expected_generation: Revision(1),
                        selection: asb_control::ConfigurationSelection {
                            agent_ids: vec!["aider".into()],
                            provider_id: "openai".into(),
                            model_id: asb_agents::openai::OPENAI_MODEL.into(),
                            auth_method: asb_control::ProviderAuthMethod::CredentialReference,
                            credential_reference_sha256: Some("d".repeat(64)),
                        },
                    }),
                    deadline(),
                )
                .unwrap();
            let plan = backend
                .execute(
                    &ControlCall::RecordingCampaignPlan(asb_control::RecordingCampaignPlanParams {
                        idempotency_key: format!("capture-invalid-plan-{suffix}"),
                        expected_generation: Revision(2),
                        runner_instance_id: runner.clone(),
                        provider_id: "openai".into(),
                        model_id: asb_agents::openai::OPENAI_MODEL.into(),
                        agent_ids: vec!["aider".into()],
                        workload_ids: vec!["original.bug-fix".into()],
                    }),
                    deadline(),
                )
                .unwrap();
            let ControlResult::RecordingCampaign(plan) = plan.result else {
                panic!("campaign plan result");
            };
            let execute = ControlCall::RecordingCampaignExecute(
                asb_control::RecordingCampaignExecuteParams {
                    idempotency_key: format!("capture-invalid-execute-{suffix}"),
                    expected_generation: Revision(2),
                    runner_instance_id: runner,
                    campaign_id: plan.campaign_id,
                },
            );
            assert_eq!(
                backend.execute(&execute, deadline()),
                Err(BackendFailure::Rejected)
            );
            let catalog = backend.catalog.lock().unwrap();
            let record = catalog.recording_campaign.as_ref().unwrap();
            assert_eq!(record.state, "failed");
            assert_eq!(
                record.unavailable_reason.as_deref(),
                Some("runtime-capture-verification-failed")
            );
        }
    }

    #[test]
    fn restart_reconciles_in_progress_capture_without_retrying_provider_effect() {
        let scratch = Scratch::new();
        let state = scratch.0.join("state");
        prepare_root(&state).unwrap();
        let backend = open_backend(state.clone()).unwrap();
        {
            let mut catalog = backend.catalog.lock().unwrap();
            catalog.recording_campaign = Some(RecordingCampaignRecord {
                campaign_id: "campaign-restart".into(),
                provider_id: "openai".into(),
                model_id: "model".into(),
                agent_ids: vec!["aider".into()],
                workload_ids: vec!["workload".into()],
                tuple_count: 1,
                generation: 1,
                state: "recording".into(),
                covered_tuple_count: 0,
                offline_ready: false,
                unavailable_reason: Some("provider-capture-in-progress".into()),
                coverage: vec![RecordingTupleRecord {
                    agent_id: "aider".into(),
                    workload_id: "workload".into(),
                    scorer_revision: "scorer-v1".into(),
                    attempt_id: "capture-attempt".into(),
                    generation: 1,
                    state: "in_progress".into(),
                    cassette_sha256: None,
                    redaction_verified: false,
                    replay_verified: false,
                }],
            });
            commit_catalog(&backend.state_root, &catalog).unwrap();
        }
        drop(backend);
        let recovered = open_backend(state).unwrap();
        let campaign = recovered
            .catalog
            .lock()
            .unwrap()
            .recording_campaign
            .clone()
            .unwrap();
        assert_eq!(campaign.state, "needs_reconciliation");
        assert_eq!(campaign.coverage[0].state, "stale");
        assert!(!campaign.offline_ready);
    }

    #[test]
    fn every_agent_lifecycle_call_is_explicitly_unavailable_until_provider_is_wired() {
        use asb_control::{
            AgentCancelRequest, AgentInstallRequest, AgentLifecycleBinding, AgentRemoveRequest,
            AgentRetryRequest, AgentStatusRequest,
        };
        let scratch = Scratch::new();
        let state = scratch.0.join("state");
        prepare_root(&state).unwrap();
        let backend = open_backend(state).unwrap();
        let binding = AgentLifecycleBinding {
            agent_id: "codex".into(),
            runner_instance_id: backend.runner_instance_id().to_owned(),
            catalog_sha256: "a".repeat(64),
        };
        let calls = [
            ControlCall::AgentInstall(AgentInstallRequest {
                binding: binding.clone(),
                catalog_generation: Revision(1),
                idempotency_key: "install-1".into(),
            }),
            ControlCall::AgentStatus(AgentStatusRequest {
                binding: binding.clone(),
                operation_id: Some("operation-1".into()),
            }),
            ControlCall::AgentCancel(AgentCancelRequest {
                binding: binding.clone(),
                operation_id: "operation-1".into(),
                idempotency_key: "cancel-1".into(),
            }),
            ControlCall::AgentRetry(AgentRetryRequest {
                binding: binding.clone(),
                operation_id: "operation-1".into(),
                idempotency_key: "retry-1".into(),
            }),
            ControlCall::AgentRemove(AgentRemoveRequest {
                binding,
                idempotency_key: "remove-1".into(),
            }),
        ];
        for call in calls {
            assert_eq!(
                backend.execute(&call, deadline()),
                Err(BackendFailure::CapabilityUnavailable)
            );
        }
    }

    #[test]
    fn control_plan_identity_binds_the_exact_measurement_selection() {
        let scratch = Scratch::new();
        let state = scratch.0.join("state");
        prepare_root(&state).unwrap();
        let backend = open_backend(state).unwrap();
        let selected = fixture_plan(&scratch.0);
        let mut empty = selected.clone();
        empty.measurement_selection = Some(
            asb_protocol::MeasurementSelectionV1::new(
                &baseline_measurement_catalog(),
                Vec::new(),
                empty.experiment.controls.replay.mode,
                None,
            )
            .unwrap(),
        );

        for plan in [&selected, &empty] {
            let validation = backend
                .execute(
                    &ControlCall::ValidateSettings {
                        settings: serde_json::to_value(plan).unwrap(),
                    },
                    deadline(),
                )
                .unwrap();
            let ControlResult::SettingsValidation(validation) = validation.result else {
                panic!("settings validation result");
            };
            assert!(validation.valid);
            assert!(validation.issues.is_empty());
        }

        let mut stale_catalog = selected.clone();
        stale_catalog
            .measurement_selection
            .as_mut()
            .unwrap()
            .catalog_sha256 = "0".repeat(64);
        let validation = backend
            .execute(
                &ControlCall::ValidateSettings {
                    settings: serde_json::to_value(&stale_catalog).unwrap(),
                },
                deadline(),
            )
            .unwrap();
        let ControlResult::SettingsValidation(validation) = validation.result else {
            panic!("settings validation result");
        };
        assert!(!validation.valid);
        assert_eq!(
            validation.issues,
            vec![SettingsIssue::MeasurementCatalogDigestMismatch]
        );
        assert_eq!(
            validation.measurement_issue,
            Some(MeasurementSettingsIssue {
                reason: asb_protocol::MeasurementSelectionReason::CatalogDigestMismatch,
                id: None,
            })
        );
        let legacy_validation = backend
            .execute_versioned(
                &ControlCall::ValidateSettings {
                    settings: serde_json::to_value(&stale_catalog).unwrap(),
                },
                deadline(),
                asb_control::CONTROL_MEASUREMENT_CATALOG_V1,
            )
            .unwrap();
        let ControlResult::SettingsValidation(legacy_validation) = legacy_validation.result else {
            panic!("settings validation result");
        };
        assert_eq!(legacy_validation.issues, vec![SettingsIssue::InvalidFormat]);
        assert_eq!(legacy_validation.measurement_issue, None);

        let create = |key: &str, plan: &PlanFile| {
            let result = backend
                .execute(
                    &ControlCall::CreatePlan(asb_control::MutationParams {
                        idempotency_key: key.into(),
                        definition: serde_json::to_value(plan).unwrap(),
                    }),
                    deadline(),
                )
                .unwrap();
            let ControlResult::Plan(reference) = result.result else {
                panic!("plan result");
            };
            reference
        };
        let selected_reference = create("measurement-selected", &selected);
        let empty_reference = create("measurement-empty", &empty);
        assert_ne!(selected_reference.plan_id, empty_reference.plan_id);
        assert_ne!(selected_reference.plan_sha256, empty_reference.plan_sha256);
    }

    #[test]
    fn control_configuration_rejects_network_listener_fields_without_effects() {
        let scratch = Scratch::new();
        let config = scratch.0.join("control.toml");
        let socket = scratch.0.join("control.sock");
        let state = scratch.0.join("state");
        fs::write(
            &config,
            format!(
                "schema_version = 1\nsocket_path = {:?}\nstate_root = {:?}\ntcp_listen = \"127.0.0.1:0\"\n",
                socket, state
            ),
        )
        .unwrap();

        assert!(load_config(&config).is_err());
        assert!(!socket.exists());
        assert!(!state.exists());
    }

    #[test]
    fn control_configuration_requires_a_distinct_provisioning_endpoint() {
        let scratch = Scratch::new();
        let config = scratch.0.join("control.toml");
        let socket = scratch.0.join("control.sock");
        let state = scratch.0.join("state");
        fs::write(
            &config,
            format!(
                "schema_version = 1\nsocket_path = {:?}\nstate_root = {:?}\n",
                socket, state
            ),
        )
        .unwrap();
        assert!(load_config(&config).is_err());

        let provisioning = scratch.0.join("provision.sock");
        fs::write(
            &config,
            format!(
                "schema_version = 1\nsocket_path = {:?}\nprovisioning_socket_path = {:?}\nstate_root = {:?}\n",
                socket, provisioning, state
            ),
        )
        .unwrap();
        let parsed = load_config(&config).unwrap();
        assert_eq!(parsed.socket_path, socket);
        assert_eq!(parsed.provisioning_socket_path, provisioning);
        assert_eq!(parsed.state_root, state);
    }

    #[test]
    fn active_run_lease_retires_worker_even_when_terminal_commit_cannot_complete() {
        let active = Arc::new(Mutex::new(BTreeMap::from([(
            "run-with-uncertain-commit".into(),
            ActiveRun {
                attempt_id: "attempt-1".into(),
                cancelled: Arc::new(AtomicBool::new(false)),
            },
        )])));
        let worker_active = Arc::clone(&active);
        let terminal = thread::spawn(move || -> Result<(), CliError> {
            let _lease = ActiveRunLease::new(worker_active, "run-with-uncertain-commit".into());
            Err(CliError::operation("injected terminal catalog failure"))
        })
        .join()
        .unwrap();
        assert!(terminal.is_err());
        assert!(active.lock().unwrap().is_empty());
    }

    #[test]
    fn history_cursor_is_stable_when_an_earlier_run_changes_state() {
        let scratch = Scratch::new();
        let state = scratch.0.join("state");
        prepare_root(&state).unwrap();
        let backend = open_backend(state.clone()).unwrap();
        {
            let mut catalog = backend.catalog.lock().unwrap();
            for ordinal in 1..=2 {
                let mut plan = fixture_plan(&scratch.0);
                plan.run_id = format!("history-run-{ordinal}");
                plan.work_root = scratch.0.join(format!("work-{ordinal}"));
                plan.result_root = scratch.0.join(format!("result-{ordinal}"));
                let plan_id = format!("history-plan-{ordinal}");
                let plan_sha256 = plan_digest(&plan).unwrap();
                catalog.plans.insert(plan_id.clone(), plan);
                let mut record = RunRecord {
                    plan_id,
                    run_id: format!("history-run-{ordinal}"),
                    attempt_id: format!("history-attempt-{ordinal}"),
                    state: PublicRunState::Planned,
                    revision: Revision(0),
                    plan_sha256,
                };
                record.revision = RunnerBackend::append_event(
                    &mut catalog,
                    ControlEventKind::RunUpdated,
                    Some(&record),
                )
                .unwrap();
                catalog.runs.insert(record.run_id.clone(), record);
            }
            commit_catalog(&state, &catalog).unwrap();
        }

        let first = backend
            .execute(
                &ControlCall::History(asb_control::PageParams {
                    after: None,
                    limit: 1,
                }),
                deadline(),
            )
            .unwrap();
        let ControlResult::History(first) = first.result else {
            panic!("history result");
        };
        assert_eq!(first.items[0].run_id.0, "history-run-1");
        assert!(first.has_more);

        {
            let mut catalog = backend.catalog.lock().unwrap();
            let mut updated = catalog.runs["history-run-1"].clone();
            updated.state = PublicRunState::Completed;
            updated.revision = RunnerBackend::append_event(
                &mut catalog,
                ControlEventKind::RunCompleted,
                Some(&updated),
            )
            .unwrap();
            catalog.runs.insert(updated.run_id.clone(), updated);
            commit_catalog(&state, &catalog).unwrap();
        }
        let second = backend
            .execute(
                &ControlCall::History(asb_control::PageParams {
                    after: first.next,
                    limit: 1,
                }),
                deadline(),
            )
            .unwrap();
        let ControlResult::History(second) = second.result else {
            panic!("history result");
        };
        assert_eq!(second.items.len(), 1);
        assert_eq!(second.items[0].run_id.0, "history-run-2");
        assert!(!second.has_more);
    }

    #[test]
    fn artifact_metadata_rejects_symlinked_artifact_ancestor() {
        let scratch = Scratch::new();
        let state = scratch.0.join("state");
        prepare_root(&state).unwrap();
        let backend = open_backend(state).unwrap();
        let mut plan = fixture_plan(&scratch.0);
        plan.run_id = "artifact-run".into();
        plan.result_root = scratch.0.join("results");
        let plan_id = "artifact-plan".to_owned();
        let plan_sha256 = plan_digest(&plan).unwrap();
        {
            let mut catalog = backend.catalog.lock().unwrap();
            catalog.plans.insert(plan_id.clone(), plan);
            catalog.runs.insert(
                "artifact-run".into(),
                RunRecord {
                    plan_id,
                    run_id: "artifact-run".into(),
                    attempt_id: "artifact-attempt".into(),
                    state: PublicRunState::Completed,
                    revision: Revision(1),
                    plan_sha256,
                },
            );
        }

        let outside = scratch.0.join("outside");
        fs::create_dir_all(&outside).unwrap();
        let bytes = b"private artifact";
        let digest = format!("{:x}", Sha256::digest(bytes));
        fs::write(outside.join(&digest), bytes).unwrap();
        let run_dir = scratch.0.join("results/runs/artifact-run");
        let artifacts = run_dir.join("artifacts");
        fs::create_dir_all(&artifacts).unwrap();
        fs::write(artifacts.join(&digest), bytes).unwrap();
        let regular = backend
            .execute(
                &ControlCall::ArtifactMetadata {
                    run_id: RunId("artifact-run".into()),
                    digest: digest.clone(),
                },
                deadline(),
            )
            .unwrap();
        assert_eq!(
            regular.result,
            ControlResult::ArtifactMetadata(ArtifactMetadata {
                sha256: digest.clone(),
                size_bytes: 16,
                sensitivity: ArtifactSensitivity::Sensitive,
            })
        );
        fs::remove_dir_all(&artifacts).unwrap();
        std::os::unix::fs::symlink(&outside, run_dir.join("artifacts")).unwrap();

        assert_eq!(
            backend.execute(
                &ControlCall::ArtifactMetadata {
                    run_id: RunId("artifact-run".into()),
                    digest: digest.clone(),
                },
                deadline(),
            ),
            Err(BackendFailure::Rejected)
        );
        fs::remove_file(run_dir.join("artifacts")).unwrap();
        fs::create_dir(&artifacts).unwrap();
        std::os::unix::fs::symlink(outside.join(&digest), artifacts.join(&digest)).unwrap();
        assert_eq!(
            backend.execute(
                &ControlCall::ArtifactMetadata {
                    run_id: RunId("artifact-run".into()),
                    digest,
                },
                deadline(),
            ),
            Err(BackendFailure::NotFound)
        );
    }

    #[test]
    fn artifact_metadata_rejects_oversized_sparse_file_without_reading_it() {
        let scratch = Scratch::new();
        let state = scratch.0.join("state");
        prepare_root(&state).unwrap();
        let backend = open_backend(state).unwrap();
        let mut plan = fixture_plan(&scratch.0);
        plan.run_id = "oversized-artifact-run".into();
        plan.result_root = scratch.0.join("results");
        let plan_id = "oversized-artifact-plan".to_owned();
        let plan_sha256 = plan_digest(&plan).unwrap();
        {
            let mut catalog = backend.catalog.lock().unwrap();
            catalog.plans.insert(plan_id.clone(), plan);
            catalog.runs.insert(
                "oversized-artifact-run".into(),
                RunRecord {
                    plan_id,
                    run_id: "oversized-artifact-run".into(),
                    attempt_id: "oversized-artifact-attempt".into(),
                    state: PublicRunState::Completed,
                    revision: Revision(1),
                    plan_sha256,
                },
            );
        }

        let digest = "a".repeat(64);
        let artifacts = scratch
            .0
            .join("results/runs/oversized-artifact-run/artifacts");
        fs::create_dir_all(&artifacts).unwrap();
        let file = fs::File::create(artifacts.join(&digest)).unwrap();
        file.set_len(StoreLimits::default().max_artifact_bytes + 1)
            .unwrap();
        assert_eq!(
            backend.execute(
                &ControlCall::ArtifactMetadata {
                    run_id: RunId("oversized-artifact-run".into()),
                    digest,
                },
                deadline(),
            ),
            Err(BackendFailure::Rejected)
        );
    }

    #[test]
    fn auth_lifecycle_survives_restart_and_revoke_is_idempotent() {
        let scratch = Scratch::new();
        let state = scratch.0.join("state");
        prepare_root(&state).unwrap();
        let backend = open_backend(state.clone()).unwrap();
        let enroll = ControlCall::AuthEnroll(asb_control::AuthEnrollParams {
            provider: "gemini".into(),
            endpoint_identity_sha256: "a".repeat(64),
            credential_locator_sha256: "b".repeat(64),
            idempotency_key: "enroll-restart".into(),
        });
        let enrolled = backend.execute(&enroll, deadline()).unwrap();
        assert!(matches!(enrolled.result, ControlResult::Acknowledged(_)));
        drop(backend);

        let cutoff = Instant::now() + Duration::from_secs(2);
        let recovered = loop {
            match open_backend(state.clone()) {
                Ok(backend) => break backend,
                Err(error) if Instant::now() < cutoff => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("auth backend did not release restart lock: {error:?}"),
            }
        };
        let status = recovered
            .execute(
                &ControlCall::AuthStatus(asb_control::AuthStatusParams {
                    provider: "gemini".into(),
                }),
                deadline(),
            )
            .unwrap();
        assert!(matches!(status.result, ControlResult::AuthStatus(_)));
        let revoke = ControlCall::AuthRevoke(asb_control::AuthRevokeParams {
            provider: "gemini".into(),
            idempotency_key: "revoke-restart".into(),
        });
        let first = recovered.execute(&revoke, deadline()).unwrap();
        let second = recovered.execute(&revoke, deadline()).unwrap();
        assert_eq!(first, second);
        let status = recovered
            .execute(
                &ControlCall::AuthStatus(asb_control::AuthStatusParams {
                    provider: "gemini".into(),
                }),
                deadline(),
            )
            .unwrap();
        assert!(matches!(
            status.result,
            ControlResult::AuthStatus(ref value) if value.status == "revoked"
        ));
    }

    #[test]
    fn production_backend_runs_without_frontend_and_recovers_idempotency() {
        let scratch = Scratch::new();
        let state = scratch.0.join("state");
        prepare_root(&state).unwrap();
        let backend = open_backend(state.clone()).unwrap();
        let plan = fixture_plan(&scratch.0);
        let create_call = ControlCall::CreatePlan(asb_control::MutationParams {
            idempotency_key: "create-real".into(),
            definition: serde_json::to_value(&plan).unwrap(),
        });
        let created = backend.execute(&create_call, deadline()).unwrap();
        let ControlResult::Plan(reference) = &created.result else {
            panic!("plan result");
        };
        let launch_call = ControlCall::Launch(asb_control::LaunchParams {
            idempotency_key: "launch-real".into(),
            plan_id: reference.plan_id.clone(),
        });
        let launched = backend.execute(&launch_call, deadline()).unwrap();
        assert!(matches!(launched.result, ControlResult::Launch(_)));

        let status_call = ControlCall::Status {
            run_id: RunId(plan.run_id.clone()),
        };
        let terminal = loop {
            let status = backend.execute(&status_call, deadline()).unwrap();
            let ControlResult::Status(summary) = status.result else {
                panic!("status result");
            };
            if matches!(
                summary.state,
                PublicRunState::Completed | PublicRunState::Failed | PublicRunState::Cancelled
            ) {
                break summary;
            }
            thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(terminal.state, PublicRunState::Completed);
        let history = backend
            .execute(
                &ControlCall::History(asb_control::PageParams {
                    after: None,
                    limit: 8,
                }),
                deadline(),
            )
            .unwrap();
        assert!(matches!(
            history.result,
            ControlResult::History(Page { ref items, .. }) if items.len() == 1
        ));
        backend
            .execute(
                &ControlCall::Analyze {
                    run_ids: vec![RunId(plan.run_id.clone())],
                },
                deadline(),
            )
            .unwrap();
        backend
            .execute(
                &ControlCall::Repeat(asb_control::RepeatParams {
                    run_id: RunId(plan.run_id.clone()),
                    idempotency_key: "repeat-real".into(),
                }),
                deadline(),
            )
            .unwrap();
        drop(backend);

        let recovered = open_backend(state).unwrap();
        assert_eq!(
            recovered.execute(&launch_call, deadline()).unwrap(),
            launched
        );
        let recovered_status = recovered.execute(&status_call, deadline()).unwrap();
        assert!(matches!(
            recovered_status.result,
            ControlResult::Status(RunSummary {
                state: PublicRunState::Completed,
                ..
            })
        ));
    }

    #[test]
    fn active_worker_retains_exclusive_state_ownership_until_terminal_commit() {
        let scratch = Scratch::new();
        let state = scratch.0.join("state");
        prepare_root(&state).unwrap();
        let backend = open_backend(state.clone()).unwrap();
        let plan = fixture_plan(&scratch.0);
        let create = ControlCall::CreatePlan(asb_control::MutationParams {
            idempotency_key: "create-slow".into(),
            definition: serde_json::to_value(&plan).unwrap(),
        });
        let created = backend.execute(&create, deadline()).unwrap();
        let ControlResult::Plan(reference) = created.result else {
            panic!("plan result");
        };
        let launch = ControlCall::Launch(asb_control::LaunchParams {
            idempotency_key: "launch-slow".into(),
            plan_id: reference.plan_id,
        });
        let launched = backend.execute(&launch, deadline()).unwrap();
        drop(backend);

        assert!(open_backend(state.clone()).is_err());
        let cutoff = Instant::now() + Duration::from_secs(5);
        let recovered = loop {
            match open_backend(state.clone()) {
                Ok(backend) => break backend,
                Err(_) if Instant::now() < cutoff => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("worker did not release state ownership: {error:?}"),
            }
        };
        assert_eq!(recovered.execute(&launch, deadline()).unwrap(), launched);
        let status = recovered
            .execute(
                &ControlCall::Status {
                    run_id: RunId(plan.run_id),
                },
                deadline(),
            )
            .unwrap();
        assert!(matches!(
            status.result,
            ControlResult::Status(RunSummary {
                state: PublicRunState::Completed,
                ..
            })
        ));
    }

    #[test]
    fn state_root_is_exclusive_and_uncertain_restart_fails_closed() {
        let scratch = Scratch::new();
        let state = scratch.0.join("state");
        prepare_root(&state).unwrap();
        let backend = open_backend(state.clone()).unwrap();
        assert!(open_backend(state.clone()).is_err());
        {
            let mut catalog = backend.catalog.lock().unwrap();
            let plan = fixture_plan(&scratch.0);
            let plan_sha256 = plan_digest(&plan).unwrap();
            catalog.plans.insert("plan-uncertain".into(), plan);
            catalog.runs.insert(
                "control-real-run".into(),
                RunRecord {
                    plan_id: "plan-uncertain".into(),
                    run_id: "control-real-run".into(),
                    attempt_id: "control-real-run-attempt".into(),
                    state: PublicRunState::Running,
                    revision: Revision(1),
                    plan_sha256,
                },
            );
            catalog.revision = Revision(1);
            catalog.events.push(ControlEvent {
                revision: Revision(1),
                kind: ControlEventKind::RunStarted,
                run_id: Some(RunId("control-real-run".into())),
                attempt_id: Some(asb_control::AttemptId("control-real-run-attempt".into())),
            });
            commit_catalog(&state, &catalog).unwrap();
        }
        drop(backend);
        let recovered = open_backend(state).unwrap();
        let catalog = recovered.catalog.lock().unwrap();
        assert_eq!(
            catalog.runs["control-real-run"].state,
            PublicRunState::NeedsReconciliation
        );
    }

    #[test]
    fn catalog_rejects_unbound_committed_plan_evidence() {
        let scratch = Scratch::new();
        let state = scratch.0.join("state");
        prepare_root(&state).unwrap();
        let backend = open_backend(state).unwrap();
        let plan = fixture_plan(&scratch.0);
        let create = ControlCall::CreatePlan(asb_control::MutationParams {
            idempotency_key: "bind-create".into(),
            definition: serde_json::to_value(&plan).unwrap(),
        });
        backend.execute(&create, deadline()).unwrap();
        let catalog = backend.catalog.lock().unwrap().clone();
        let created_reference = catalog
            .mutations
            .values()
            .find_map(
                |mutation| match mutation.result.as_ref().map(|bound| &bound.result) {
                    Some(ControlResult::Plan(reference)) => Some(reference.clone()),
                    _ => None,
                },
            )
            .unwrap();

        let mut missing_plan = catalog.clone();
        let mutation = missing_plan.mutations.values_mut().next().unwrap();
        let ControlResult::Plan(reference) = &mut mutation.result.as_mut().unwrap().result else {
            panic!("plan result");
        };
        reference.plan_id = "plan-missing".into();
        assert!(validate_catalog(&missing_plan).is_err());

        let mut wrong_digest = catalog;
        let mutation = wrong_digest.mutations.values_mut().next().unwrap();
        let ControlResult::Plan(reference) = &mut mutation.result.as_mut().unwrap().result else {
            panic!("plan result");
        };
        reference.plan_sha256 = "0".repeat(64);
        assert!(validate_catalog(&wrong_digest).is_err());

        let mut launch_catalog = backend.catalog.lock().unwrap().clone();
        let run_id = plan.run_id.clone();
        let attempt_id = format!("{run_id}-attempt");
        let plan_sha256 = created_reference.plan_sha256.clone();
        let launch_revision = Revision(launch_catalog.revision.0 + 1);
        launch_catalog.revision = launch_revision;
        launch_catalog.events.push(ControlEvent {
            revision: launch_revision,
            kind: ControlEventKind::RunUpdated,
            run_id: Some(RunId(run_id.clone())),
            attempt_id: Some(asb_control::AttemptId(attempt_id.clone())),
        });
        launch_catalog.runs.insert(
            run_id.clone(),
            RunRecord {
                plan_id: created_reference.plan_id.clone(),
                run_id: run_id.clone(),
                attempt_id: attempt_id.clone(),
                state: PublicRunState::Planned,
                revision: launch_revision,
                plan_sha256: plan_sha256.clone(),
            },
        );
        let launch_call = ControlCall::Launch(asb_control::LaunchParams {
            idempotency_key: "bind-launch".into(),
            plan_id: created_reference.plan_id,
        });
        let launch_result = BoundControlResult::new(
            &launch_call,
            ControlResult::Launch(RunSummary {
                run_id: RunId(run_id.clone()),
                attempt_id: asb_control::AttemptId(attempt_id.clone()),
                state: PublicRunState::Planned,
                created_revision: launch_revision,
                revision: launch_revision,
                plan_sha256,
            }),
        )
        .unwrap();
        launch_catalog.mutations.insert(
            "1".repeat(64),
            MutationRecord {
                request_sha256: launch_result.request_sha256.clone(),
                target: MutationTarget::Launch {
                    run_id: run_id.clone(),
                    attempt_id: attempt_id.clone(),
                },
                state: MutationState::Committed,
                result: Some(launch_result),
            },
        );
        validate_catalog(&launch_catalog).unwrap();
        let mut cancel_catalog = launch_catalog.clone();
        let cancel_revision = Revision(cancel_catalog.revision.0 + 1);
        cancel_catalog.revision = cancel_revision;
        let mut cancelled = cancel_catalog.runs.get(&run_id).unwrap().clone();
        cancelled.state = PublicRunState::Cancelled;
        cancelled.revision = cancel_revision;
        cancel_catalog.runs.insert(run_id.clone(), cancelled);
        cancel_catalog.events.push(ControlEvent {
            revision: cancel_revision,
            kind: ControlEventKind::RunCancelled,
            run_id: Some(RunId(run_id.clone())),
            attempt_id: Some(asb_control::AttemptId(attempt_id.clone())),
        });
        let cancel_call = ControlCall::Cancel(asb_control::CancelParams {
            run_id: RunId(run_id.clone()),
            attempt_id: asb_control::AttemptId(attempt_id.clone()),
            idempotency_key: "bind-cancel".into(),
        });
        let cancel_result = BoundControlResult::new(
            &cancel_call,
            ControlResult::Acknowledged(MutationAcknowledgement { accepted: true }),
        )
        .unwrap();
        cancel_catalog.mutations.insert(
            "2".repeat(64),
            MutationRecord {
                request_sha256: cancel_result.request_sha256.clone(),
                target: MutationTarget::Cancel { run_id, attempt_id },
                state: MutationState::Committed,
                result: Some(cancel_result),
            },
        );
        validate_catalog(&cancel_catalog).unwrap();
        let MutationTarget::Cancel { attempt_id, .. } = &mut cancel_catalog
            .mutations
            .get_mut(&"2".repeat(64))
            .unwrap()
            .target
        else {
            panic!("cancel target");
        };
        *attempt_id = "different-attempt".into();
        assert!(validate_catalog(&cancel_catalog).is_err());

        let launch = launch_catalog.mutations.get_mut(&"1".repeat(64)).unwrap();
        let ControlResult::Launch(summary) = &mut launch.result.as_mut().unwrap().result else {
            panic!("launch result");
        };
        summary.plan_sha256 = "0".repeat(64);
        assert!(validate_catalog(&launch_catalog).is_err());
    }

    #[test]
    fn expired_mutation_deadline_prevents_catalog_commit() {
        let scratch = Scratch::new();
        let state = scratch.0.join("state");
        prepare_root(&state).unwrap();
        let backend = open_backend(state).unwrap();
        let call = ControlCall::CreatePlan(asb_control::MutationParams {
            idempotency_key: "late-create".into(),
            definition: serde_json::Value::Null,
        });
        let before = serde_json::to_vec(&*backend.catalog.lock().unwrap()).unwrap();
        let result = backend.mutation(
            &call,
            "late-create",
            MutationTarget::CreatePlan,
            RequestDeadline::start(1).unwrap(),
            |_| {
                thread::sleep(Duration::from_millis(5));
                Ok(ControlResult::Plan(PlanReference {
                    plan_id: "plan-never-committed".into(),
                    plan_sha256: "0".repeat(64),
                }))
            },
        );
        assert!(matches!(result, Err(BackendFailure::NeedsReconciliation)));
        let after = serde_json::to_vec(&*backend.catalog.lock().unwrap()).unwrap();
        assert_eq!(after, before);
    }

    #[test]
    fn expired_status_deadline_prevents_refresh_catalog_commit() {
        let scratch = Scratch::new();
        let state = scratch.0.join("state");
        prepare_root(&state).unwrap();
        let backend = open_backend(state.clone()).unwrap();
        let plan = fixture_plan(&scratch.0);
        let create = ControlCall::CreatePlan(asb_control::MutationParams {
            idempotency_key: "late-status-plan".into(),
            definition: serde_json::to_value(&plan).unwrap(),
        });
        let created = backend.execute(&create, deadline()).unwrap();
        let ControlResult::Plan(reference) = created.result else {
            panic!("plan result");
        };
        {
            let mut catalog = backend.catalog.lock().unwrap();
            let mut record = RunRecord {
                plan_id: reference.plan_id,
                run_id: plan.run_id.clone(),
                attempt_id: format!("{}-attempt", plan.run_id),
                state: PublicRunState::Running,
                revision: Revision(0),
                plan_sha256: reference.plan_sha256,
            };
            record.revision = RunnerBackend::append_event(
                &mut catalog,
                ControlEventKind::RunStarted,
                Some(&record),
            )
            .unwrap();
            catalog.runs.insert(record.run_id.clone(), record);
            commit_catalog(&state, &catalog).unwrap();
        }
        let before_memory = serde_json::to_vec(&*backend.catalog.lock().unwrap()).unwrap();
        let before_file = fs::read(state.join("control-catalog.json")).unwrap();
        let expired = RequestDeadline::start(1).unwrap();
        thread::sleep(Duration::from_millis(5));
        assert!(matches!(
            backend.refresh(&plan.run_id, expired),
            Err(BackendFailure::NeedsReconciliation)
        ));
        assert_eq!(
            serde_json::to_vec(&*backend.catalog.lock().unwrap()).unwrap(),
            before_memory
        );
        assert_eq!(
            fs::read(state.join("control-catalog.json")).unwrap(),
            before_file
        );
    }

    #[test]
    fn expired_analysis_deadline_prevents_artifact_commit() {
        let scratch = Scratch::new();
        let state = scratch.0.join("state");
        prepare_root(&state).unwrap();
        let backend = open_backend(state.clone()).unwrap();
        let bytes = br#"{"bounded":"public"}"#;
        let digest = format!("{:x}", Sha256::digest(bytes));
        let expired = RequestDeadline::start(1).unwrap();
        thread::sleep(Duration::from_millis(5));
        assert!(matches!(
            backend.commit_analysis_before_deadline(bytes, &digest, expired),
            Err(BackendFailure::NeedsReconciliation)
        ));
        assert!(!state.join("analyses").exists());
    }

    #[test]
    fn unix_frontend_disconnect_does_not_stop_real_run() {
        let scratch = Scratch::new();
        let state = scratch.0.join("state");
        prepare_root(&state).unwrap();
        let socket = scratch.0.join("control.sock");
        let backend = open_backend(state).unwrap();
        let mut server = ControlServer::bind(&socket, ControlLimits::default(), backend).unwrap();
        let service = thread::spawn(move || server.serve_connections(3));
        let plan = fixture_plan_with_script(
            &scratch.0,
            b"#!/bin/sh\n/bin/sleep 1\nprintf '%s\\n' 'def parse_line(line):' '    if line.endswith(\"\\r\"):' '        line = line[:-1]' '    return line' > parser.py\n",
        );
        let mut first =
            asb_control::ControlClient::connect(&socket, ControlLimits::default()).unwrap();
        let create = first
            .call(
                ControlCall::CreatePlan(asb_control::MutationParams {
                    idempotency_key: "wire-create".into(),
                    definition: serde_json::to_value(&plan).unwrap(),
                }),
                5_000,
            )
            .unwrap()
            .into_result()
            .unwrap();
        let ControlSuccess::Operation(created) = create else {
            panic!("operation");
        };
        let ControlResult::Plan(reference) = created.result else {
            panic!("plan");
        };
        let initial_events = first
            .call(
                ControlCall::Events(asb_control::PageParams {
                    after: None,
                    limit: 8,
                }),
                5_000,
            )
            .unwrap()
            .into_result()
            .unwrap();
        let ControlSuccess::Operation(initial_events) = initial_events else {
            panic!("operation");
        };
        let ControlResult::Events(initial_events) = initial_events.result else {
            panic!("events");
        };
        let reconnect_cursor = initial_events.next.expect("created-plan event cursor");
        drop(first);

        let launch_call = ControlCall::Launch(asb_control::LaunchParams {
            idempotency_key: "wire-launch".into(),
            plan_id: reference.plan_id,
        });
        let mut raw = std::os::unix::net::UnixStream::connect(&socket).unwrap();
        let negotiation = asb_control::ControlRequest {
            jsonrpc: asb_control::JSONRPC_VERSION.into(),
            id: asb_control::RequestId(1),
            timeout_ms: 5_000,
            call: ControlCall::Negotiate(asb_control::NegotiateParams {
                versions: std::collections::BTreeSet::from([asb_control::CONTROL_V1]),
                limits: ControlLimits::default(),
            }),
        };
        asb_control::write_frame(&mut raw, &negotiation, ControlLimits::default()).unwrap();
        let _: asb_control::ControlResponse =
            asb_control::read_frame(&mut raw, ControlLimits::default()).unwrap();
        let launch_request = asb_control::ControlRequest {
            jsonrpc: asb_control::JSONRPC_VERSION.into(),
            id: asb_control::RequestId(2),
            timeout_ms: 5_000,
            call: launch_call.clone(),
        };
        asb_control::write_frame(&mut raw, &launch_request, ControlLimits::default()).unwrap();
        raw.shutdown(std::net::Shutdown::Both).unwrap();
        drop(raw);

        let mut second =
            asb_control::ControlClient::connect(&socket, ControlLimits::default()).unwrap();
        let replayed = second
            .call(launch_call.clone(), 5_000)
            .unwrap()
            .into_result()
            .unwrap();
        assert_eq!(
            replayed,
            second
                .call(launch_call, 5_000)
                .unwrap()
                .into_result()
                .unwrap()
        );
        let resumed_events = second
            .call(
                ControlCall::Events(asb_control::PageParams {
                    after: Some(reconnect_cursor),
                    limit: 8,
                }),
                5_000,
            )
            .unwrap()
            .into_result()
            .unwrap();
        let ControlSuccess::Operation(resumed_events) = resumed_events else {
            panic!("operation");
        };
        let ControlResult::Events(resumed_events) = resumed_events.result else {
            panic!("events");
        };
        assert!(!resumed_events.items.is_empty());
        assert_eq!(
            resumed_events.items[0].revision,
            Revision(reconnect_cursor.0 + 1)
        );
        assert!(resumed_events.items.iter().any(|event| {
            event.kind == ControlEventKind::RunStarted
                && event
                    .run_id
                    .as_ref()
                    .is_some_and(|run_id| run_id.0 == plan.run_id)
        }));
        let terminal_cursor = resumed_events.next.expect("resumed event cursor");
        let history = second
            .call(
                ControlCall::History(asb_control::PageParams {
                    after: None,
                    limit: 8,
                }),
                5_000,
            )
            .unwrap()
            .into_result()
            .unwrap();
        assert!(matches!(
            history,
            ControlSuccess::Operation(BoundControlResult {
                result: ControlResult::History(Page { ref items, .. }),
                ..
            }) if items.len() == 1
        ));
        loop {
            let response = second
                .call(
                    ControlCall::Status {
                        run_id: RunId(plan.run_id.clone()),
                    },
                    5_000,
                )
                .unwrap()
                .into_result()
                .unwrap();
            let ControlSuccess::Operation(bound) = response else {
                panic!("operation");
            };
            let ControlResult::Status(summary) = bound.result else {
                panic!("status");
            };
            if matches!(
                summary.state,
                PublicRunState::Completed | PublicRunState::Failed | PublicRunState::Cancelled
            ) {
                break;
            }
            assert!(matches!(
                summary.state,
                PublicRunState::Planned
                    | PublicRunState::Prepared
                    | PublicRunState::Running
                    | PublicRunState::Collecting
            ));
            thread::sleep(Duration::from_millis(10));
        }
        let terminal_events = second
            .call(
                ControlCall::Events(asb_control::PageParams {
                    after: Some(terminal_cursor),
                    limit: 8,
                }),
                5_000,
            )
            .unwrap()
            .into_result()
            .unwrap();
        let ControlSuccess::Operation(terminal_events) = terminal_events else {
            panic!("operation");
        };
        let ControlResult::Events(terminal_events) = terminal_events.result else {
            panic!("events");
        };
        assert_eq!(
            terminal_events.items.first().map(|event| event.revision),
            Some(Revision(terminal_cursor.0 + 1))
        );
        assert!(terminal_events.items.iter().any(|event| {
            event.kind == ControlEventKind::RunCompleted
                && event
                    .run_id
                    .as_ref()
                    .is_some_and(|run_id| run_id.0 == plan.run_id)
        }));
        drop(second);
        service.join().unwrap().unwrap();
    }
}
