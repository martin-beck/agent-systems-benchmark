// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Persistent local runner service behind the typed frontend control boundary.

use super::*;
use asb_control::{
    AnalysisSummary, ArtifactMetadata, ArtifactSensitivity, BackendFailure, BoundControlResult,
    Capabilities, ControlBackend, ControlCall, ControlEvent, ControlEventKind, ControlLimits,
    ControlResult, ControlServer, MutationAcknowledgement, Page, PlanReference, PublicRunState,
    RequestDeadline, Revision, RunId, RunSummary, SettingsIssue, SettingsValidation,
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
    let socket_parent = config
        .socket_path
        .parent()
        .ok_or_else(|| CliError::validation("control socket path is unsafe"))?;
    if !config.socket_path.is_absolute()
        || fs::canonicalize(socket_parent).ok().as_deref() != Some(socket_parent)
        || config.socket_path.starts_with(&state_root)
        || state_root.starts_with(socket_parent)
    {
        return Err(CliError::validation("control socket path is unsafe"));
    }
    let backend = open_backend(state_root)?;
    let mut server = ControlServer::bind(
        &config.socket_path,
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

fn open_backend(state_root: PathBuf) -> Result<RunnerBackend, CliError> {
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
            ControlCall::ValidateSettings { settings } => {
                let valid = serde_json::from_value::<PlanFile>(settings.clone())
                    .ok()
                    .is_some_and(|plan| validate_plan(&plan).is_ok());
                self.bind(
                    call,
                    ControlResult::SettingsValidation(SettingsValidation {
                        valid,
                        issues: if valid {
                            Vec::new()
                        } else {
                            vec![SettingsIssue::InvalidFormat]
                        },
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
            ControlCall::Negotiate(_) => Err(BackendFailure::Rejected),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use asb_control::ControlSuccess;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::AtomicU64;

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            let root = std::env::var_os("ASB_TEST_ROOT")
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir)
                .join(format!(
                    "asb-control-runner-{}-{}",
                    std::process::id(),
                    TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
                ));
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&root)
                .unwrap();
            Self(fs::canonicalize(root).unwrap())
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
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
        }
    }

    fn deadline() -> RequestDeadline {
        RequestDeadline::start(10_000).unwrap()
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
