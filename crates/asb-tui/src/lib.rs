// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
#![forbid(unsafe_code)]
#![deny(missing_docs)]
//! Capability-driven state for the independent settings wizard.

use asb_control::{
    CancelParams, Capabilities, ControlCall, ControlEvent, ControlEventKind, ControlLimits,
    ControlResult, LaunchParams, MutationAcknowledgement, MutationParams, Page, PageParams,
    PlanReference, PublicRunState, Revision, RunSummary, validate_digest, validate_idempotency_key,
    validate_identity,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeSet;
use std::fmt;

/// Wizard settings generation.
pub const WIZARD_SETTINGS_V1: u16 = 1;
/// Maximum negotiated entries per selector.
pub const MAX_CHOICES: usize = 256;

/// Public runner-advertised identifier.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ChoiceId(pub String);

/// Explicit execution source, without an implicit default.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderSource {
    /// Preflighted live provider.
    Live,
    /// Exact compatible recording.
    Replay {
        /// Authenticated cassette digest.
        cassette_sha256: String,
    },
}

/// Choices and bounds negotiated from the runner.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WizardCatalog {
    /// Generic frontend capabilities.
    pub control: Capabilities,
    /// Agent choices.
    pub agents: Vec<ChoiceId>,
    /// Provider choices.
    pub providers: Vec<ChoiceId>,
    /// Workload choices.
    pub workloads: Vec<ChoiceId>,
    /// Platform choices.
    pub platforms: Vec<ChoiceId>,
    /// Metric choices.
    pub metrics: Vec<ChoiceId>,
    /// Compatible recording digests.
    pub recordings: Vec<String>,
    /// Whether live provider preflight succeeded.
    pub live_available: bool,
    /// Maximum repetitions.
    pub max_repetitions: u32,
    /// Maximum concurrency.
    pub max_concurrency: u32,
}

/// Complete editable wizard settings.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WizardSettings {
    /// Settings generation.
    pub schema_version: u16,
    /// Agent selection.
    pub agent: Option<ChoiceId>,
    /// Provider selection.
    pub provider: Option<ChoiceId>,
    /// Explicit execution source.
    pub source: Option<ProviderSource>,
    /// Workload selection.
    pub workload: Option<ChoiceId>,
    /// Repetition count.
    pub repetitions: u32,
    /// Concurrency.
    pub concurrency: u32,
    /// Platform selection.
    pub platform: Option<ChoiceId>,
    /// Metric selections.
    pub metrics: Vec<ChoiceId>,
    /// Non-secret credential reference digest.
    pub credential_reference_sha256: Option<String>,
}

/// Privacy-safe wizard error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WizardError {
    /// Catalog is malformed.
    InvalidCatalog,
    /// Settings are malformed, incomplete, or unsupported.
    InvalidSettings,
    /// Runner validation is unavailable.
    ValidationUnavailable,
    /// Explicit plan confirmation did not match.
    ConfirmationRequired,
}

/// Maximum event page requested by the terminal frontend.
pub const RUN_EVENT_PAGE_ITEMS: u16 = 128;

/// Observable launch lifecycle. An acknowledgement is not a terminal outcome.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunControlPhase {
    /// A validated plan may be launched after explicit confirmation.
    Ready,
    /// One launch request has been emitted and must not be duplicated.
    LaunchPending,
    /// The runner returned the durable run and exact attempt identity.
    Following,
    /// A causally fenced cancellation request awaits runner truth.
    CancelPending,
    /// The authoritative runner state is terminal.
    Terminal(PublicRunState),
}

/// Privacy-safe run-control model error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunControlError {
    /// A public identity or idempotency key is malformed.
    InvalidIdentity,
    /// The requested transition is not valid from the current phase.
    InvalidTransition,
    /// Explicit launch confirmation did not match.
    ConfirmationRequired,
    /// A response refers to another run or attempt.
    IdentityMismatch,
    /// An authoritative revision moved backward, skipped, or was compacted.
    StaleProjection,
    /// The runner did not durably accept a mutation.
    NotAcknowledged,
}

impl fmt::Display for RunControlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidIdentity => "run-control identity is invalid",
            Self::InvalidTransition => "run-control transition is invalid",
            Self::ConfirmationRequired => "explicit launch confirmation is required",
            Self::IdentityMismatch => "runner identity does not match",
            Self::StaleProjection => "runner projection requires refresh",
            Self::NotAcknowledged => "runner mutation was not acknowledged",
        })
    }
}
impl std::error::Error for RunControlError {}

/// Reconnectable frontend projection over the runner-owned lifecycle.
pub struct RunControl {
    plan_id: String,
    plan_sha256: String,
    launch_key: String,
    phase: RunControlPhase,
    summary: Option<RunSummary>,
    cursor: Option<Revision>,
    limits: ControlLimits,
}

impl RunControl {
    /// Create a launch controller for one immutable validated plan.
    pub fn new(
        plan: PlanReference,
        launch_key: String,
        limits: ControlLimits,
    ) -> Result<Self, RunControlError> {
        if validate_identity(&plan.plan_id).is_err()
            || validate_digest(&plan.plan_sha256).is_err()
            || validate_idempotency_key(&launch_key).is_err()
            || limits.validate().is_err()
        {
            return Err(RunControlError::InvalidIdentity);
        }
        Ok(Self {
            plan_id: plan.plan_id,
            plan_sha256: plan.plan_sha256,
            launch_key,
            phase: RunControlPhase::Ready,
            summary: None,
            cursor: None,
            limits,
        })
    }

    /// Restore a frontend projection after restart without repeating launch.
    pub fn reconnect(
        summary: RunSummary,
        cursor: Option<Revision>,
        limits: ControlLimits,
    ) -> Result<Self, RunControlError> {
        if !valid_summary_identity(&summary)
            || cursor.is_some_and(|revision| revision < summary.created_revision)
            || limits.validate().is_err()
        {
            return Err(RunControlError::StaleProjection);
        }
        Ok(Self {
            plan_id: String::new(),
            plan_sha256: summary.plan_sha256.clone(),
            launch_key: String::new(),
            phase: terminal_phase(summary.state).unwrap_or(RunControlPhase::Following),
            summary: Some(summary),
            cursor,
            limits,
        })
    }

    /// Current locally observed phase, always subordinate to runner responses.
    pub const fn phase(&self) -> RunControlPhase {
        self.phase
    }

    /// Latest authoritative public run summary, if launch has been acknowledged.
    pub fn summary(&self) -> Option<&RunSummary> {
        self.summary.as_ref()
    }

    /// Emit at most one launch request after exact confirmation.
    pub fn launch(&mut self, confirmation: &str) -> Result<ControlCall, RunControlError> {
        if self.phase != RunControlPhase::Ready {
            return Err(RunControlError::InvalidTransition);
        }
        if confirmation != "launch run" {
            return Err(RunControlError::ConfirmationRequired);
        }
        self.phase = RunControlPhase::LaunchPending;
        Ok(ControlCall::Launch(LaunchParams {
            idempotency_key: self.launch_key.clone(),
            plan_id: self.plan_id.clone(),
        }))
    }

    /// Bind the durable launch result before following status or events.
    pub fn accept_launch(&mut self, summary: RunSummary) -> Result<(), RunControlError> {
        if self.phase != RunControlPhase::LaunchPending || !valid_summary_identity(&summary) {
            return Err(RunControlError::InvalidTransition);
        }
        if summary.plan_sha256 != self.plan_sha256 {
            return Err(RunControlError::IdentityMismatch);
        }
        self.cursor = Some(summary.revision);
        self.phase = terminal_phase(summary.state).unwrap_or(RunControlPhase::Following);
        self.summary = Some(summary);
        Ok(())
    }

    /// Build an authoritative status refresh after reconnect.
    pub fn status_call(&self) -> Result<ControlCall, RunControlError> {
        let summary = self
            .summary
            .as_ref()
            .ok_or(RunControlError::InvalidTransition)?;
        Ok(ControlCall::Status {
            run_id: summary.run_id.clone(),
        })
    }

    /// Resume bounded public events from the last contiguous durable revision.
    pub fn events_call(&self) -> Result<ControlCall, RunControlError> {
        if self.summary.is_none() {
            return Err(RunControlError::InvalidTransition);
        }
        Ok(ControlCall::Events(PageParams {
            after: self.cursor,
            limit: RUN_EVENT_PAGE_ITEMS.min(self.limits.max_page_items),
        }))
    }

    /// Accept a non-regressing authoritative status for the exact run and attempt.
    pub fn accept_status(&mut self, next: RunSummary) -> Result<(), RunControlError> {
        let current = self
            .summary
            .as_ref()
            .ok_or(RunControlError::InvalidTransition)?;
        if next.run_id != current.run_id
            || next.attempt_id != current.attempt_id
            || next.created_revision != current.created_revision
            || next.plan_sha256 != current.plan_sha256
        {
            return Err(RunControlError::IdentityMismatch);
        }
        if next.revision < current.revision
            || (next.revision == current.revision && next != *current)
            || (next.revision > current.revision
                && !valid_status_transition(current.state, next.state))
        {
            return Err(RunControlError::StaleProjection);
        }
        self.cursor = Some(
            self.cursor
                .map_or(next.revision, |cursor| cursor.max(next.revision)),
        );
        self.phase = terminal_phase(next.state).unwrap_or_else(|| {
            if self.phase == RunControlPhase::CancelPending {
                RunControlPhase::CancelPending
            } else {
                RunControlPhase::Following
            }
        });
        self.summary = Some(next);
        Ok(())
    }

    /// Advance only through a contiguous page for this exact run and attempt.
    pub fn accept_events(&mut self, page: Page<ControlEvent>) -> Result<(), RunControlError> {
        if ControlResult::Events(page.clone())
            .validate(self.limits)
            .is_err()
        {
            return Err(RunControlError::StaleProjection);
        }
        let summary = self
            .summary
            .as_ref()
            .ok_or(RunControlError::InvalidTransition)?;
        let mut expected = match self.cursor {
            Some(Revision(u64::MAX)) if !page.items.is_empty() => {
                return Err(RunControlError::StaleProjection);
            }
            Some(revision) => revision.0.checked_add(1),
            None => None,
        };
        let mut projected = (summary.state, summary.revision);
        for (index, event) in page.items.iter().enumerate() {
            if expected.is_some_and(|value| event.revision.0 != value) {
                return Err(RunControlError::StaleProjection);
            }
            let matches_followed_run = event.run_id.as_ref() == Some(&summary.run_id)
                && event.attempt_id.as_ref() == Some(&summary.attempt_id);
            if matches_followed_run {
                if let Some(next_state) = event_state(event.kind) {
                    if !valid_status_transition(projected.0, next_state) {
                        return Err(RunControlError::StaleProjection);
                    }
                    projected = (next_state, event.revision);
                } else if terminal_phase(projected.0).is_some() {
                    return Err(RunControlError::StaleProjection);
                }
            }
            expected = event.revision.0.checked_add(1);
            if expected.is_none() && index + 1 != page.items.len() {
                return Err(RunControlError::StaleProjection);
            }
        }
        let observed = page.items.last().map(|event| event.revision);
        if page.next != observed {
            return Err(RunControlError::StaleProjection);
        }
        if let Some(next) = page.next {
            self.cursor = Some(next);
        }
        if projected != (summary.state, summary.revision) {
            self.phase = terminal_phase(projected.0).unwrap_or_else(|| {
                if self.phase == RunControlPhase::CancelPending {
                    RunControlPhase::CancelPending
                } else {
                    RunControlPhase::Following
                }
            });
            if let Some(summary) = self.summary.as_mut() {
                summary.state = projected.0;
                summary.revision = projected.1;
            }
        }
        Ok(())
    }

    /// Emit one cancellation request fenced to the exact current attempt.
    pub fn cancel(&mut self, idempotency_key: String) -> Result<ControlCall, RunControlError> {
        if self.phase != RunControlPhase::Following
            || validate_idempotency_key(&idempotency_key).is_err()
        {
            return Err(RunControlError::InvalidTransition);
        }
        let summary = self
            .summary
            .as_ref()
            .ok_or(RunControlError::InvalidTransition)?;
        self.phase = RunControlPhase::CancelPending;
        Ok(ControlCall::Cancel(CancelParams {
            run_id: summary.run_id.clone(),
            attempt_id: summary.attempt_id.clone(),
            idempotency_key,
        }))
    }

    /// Record durable mutation acceptance without inventing terminal state.
    pub fn accept_cancel(
        &mut self,
        acknowledgement: MutationAcknowledgement,
    ) -> Result<(), RunControlError> {
        if self.phase != RunControlPhase::CancelPending {
            return Err(RunControlError::InvalidTransition);
        }
        if !acknowledgement.accepted {
            return Err(RunControlError::NotAcknowledged);
        }
        Ok(())
    }
}

fn valid_summary_identity(summary: &RunSummary) -> bool {
    validate_identity(&summary.run_id.0).is_ok()
        && validate_identity(&summary.attempt_id.0).is_ok()
        && validate_digest(&summary.plan_sha256).is_ok()
        && summary.created_revision <= summary.revision
}

const fn terminal_phase(state: PublicRunState) -> Option<RunControlPhase> {
    match state {
        PublicRunState::Completed
        | PublicRunState::Failed
        | PublicRunState::Cancelled
        | PublicRunState::NeedsReconciliation => Some(RunControlPhase::Terminal(state)),
        _ => None,
    }
}

const fn event_state(kind: ControlEventKind) -> Option<PublicRunState> {
    match kind {
        ControlEventKind::RunStarted => Some(PublicRunState::Running),
        ControlEventKind::RunCompleted => Some(PublicRunState::Completed),
        ControlEventKind::RunFailed => Some(PublicRunState::Failed),
        ControlEventKind::RunCancelled => Some(PublicRunState::Cancelled),
        ControlEventKind::ReconciliationRequired => Some(PublicRunState::NeedsReconciliation),
        ControlEventKind::RunnerReady
        | ControlEventKind::PlanCreated
        | ControlEventKind::RunUpdated => None,
    }
}

fn valid_status_transition(from: PublicRunState, to: PublicRunState) -> bool {
    if from == to {
        return true;
    }
    matches!(
        (from, to),
        (
            PublicRunState::Planned,
            PublicRunState::Prepared
                | PublicRunState::Running
                | PublicRunState::Collecting
                | PublicRunState::Completed
                | PublicRunState::Failed
                | PublicRunState::Cancelled
        ) | (
            PublicRunState::Prepared,
            PublicRunState::Running
                | PublicRunState::Collecting
                | PublicRunState::Completed
                | PublicRunState::Failed
                | PublicRunState::Cancelled
        ) | (
            PublicRunState::Running,
            PublicRunState::Collecting
                | PublicRunState::Completed
                | PublicRunState::Failed
                | PublicRunState::Cancelled
        ) | (
            PublicRunState::Collecting,
            PublicRunState::Completed | PublicRunState::Failed | PublicRunState::Cancelled
        ) | (_, PublicRunState::NeedsReconciliation)
            | (
                PublicRunState::NeedsReconciliation,
                PublicRunState::Completed | PublicRunState::Failed | PublicRunState::Cancelled
            )
    )
}

impl fmt::Display for WizardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidCatalog => "negotiated catalog is invalid",
            Self::InvalidSettings => "wizard settings are invalid",
            Self::ValidationUnavailable => "runner validation is unavailable",
            Self::ConfirmationRequired => "explicit plan confirmation is required",
        })
    }
}
impl std::error::Error for WizardError {}

/// Stable keyboard-first wizard step.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WizardStep {
    /// Agent selection.
    Agent,
    /// Provider selection.
    Provider,
    /// Explicit source selection.
    Source,
    /// Workload selection.
    Workload,
    /// Resource settings.
    Resources,
    /// Platform and metrics.
    Evidence,
    /// Dry-run and final review.
    Review,
}

/// Keyboard command shared by interactive and plain modes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WizardCommand {
    /// Advance one step.
    Next,
    /// Return one step.
    Previous,
    /// Reset all settings.
    Reset,
    /// Toggle contextual help.
    Help,
}

/// Lossless, launch-free wizard state.
pub struct Wizard {
    catalog: WizardCatalog,
    current: WizardSettings,
    history: Vec<WizardSettings>,
    step: WizardStep,
    help_visible: bool,
}

impl Wizard {
    /// Construct from negotiated support only.
    pub fn new(catalog: WizardCatalog) -> Result<Self, WizardError> {
        validate_catalog(&catalog)?;
        Ok(Self {
            catalog,
            current: WizardSettings {
                schema_version: WIZARD_SETTINGS_V1,
                ..WizardSettings::default()
            },
            history: Vec::new(),
            step: WizardStep::Agent,
            help_visible: false,
        })
    }
    /// Current renderable settings.
    pub fn settings(&self) -> &WizardSettings {
        &self.current
    }
    /// Current navigation step.
    pub const fn step(&self) -> WizardStep {
        self.step
    }
    /// Apply a keyboard command without external effects.
    pub fn command(&mut self, command: WizardCommand) {
        match command {
            WizardCommand::Next => self.step = next_step(self.step),
            WizardCommand::Previous => self.step = previous_step(self.step),
            WizardCommand::Reset => self.reset(),
            WizardCommand::Help => self.help_visible = !self.help_visible,
        }
    }
    /// Search negotiated choices with bounded ASCII case folding.
    pub fn search(&self, values: &[ChoiceId], query: &str) -> Result<Vec<ChoiceId>, WizardError> {
        if query.len() > 128 || !query.is_ascii() || values.len() > MAX_CHOICES {
            return Err(WizardError::InvalidSettings);
        }
        let query = query.to_ascii_lowercase();
        Ok(values
            .iter()
            .filter(|value| value.0.to_ascii_lowercase().contains(&query))
            .cloned()
            .collect())
    }
    /// Render a deterministic privacy-safe plain-text snapshot.
    pub fn render_plain(&self, width: usize) -> Result<String, WizardError> {
        if !(24..=240).contains(&width) {
            return Err(WizardError::InvalidSettings);
        }
        let source = match &self.current.source {
            None => "not selected",
            Some(ProviderSource::Live) => "live",
            Some(ProviderSource::Replay { .. }) => "matching replay",
        };
        let help = if self.help_visible {
            "\nKeys: next, previous, reset, help"
        } else {
            ""
        };
        Ok(format!(
            "ASB settings | {:?}\nAgent: {}\nProvider: {}\nSource: {}\nWorkload: {}\nRepetitions: {}  Concurrency: {}\nNo run starts from this screen.{}",
            self.step,
            selected(&self.current.agent),
            selected(&self.current.provider),
            source,
            selected(&self.current.workload),
            self.current.repetitions,
            self.current.concurrency,
            help
        ))
    }
    /// Apply a validated edit.
    pub fn apply(&mut self, next: WizardSettings) -> Result<(), WizardError> {
        validate(&self.catalog, &next, false)?;
        self.history.push(self.current.clone());
        self.current = next;
        Ok(())
    }
    /// Restore the prior edit.
    pub fn back(&mut self) -> bool {
        if let Some(prior) = self.history.pop() {
            self.current = prior;
            true
        } else {
            false
        }
    }
    /// Reset without losing backtracking.
    pub fn reset(&mut self) {
        self.history.push(self.current.clone());
        self.current = WizardSettings {
            schema_version: WIZARD_SETTINGS_V1,
            ..WizardSettings::default()
        };
    }
    /// Import bounded closed JSON.
    pub fn import_json(&mut self, bytes: &[u8]) -> Result<(), WizardError> {
        if bytes.is_empty() || bytes.len() > 65_536 {
            return Err(WizardError::InvalidSettings);
        }
        let value = serde_json::from_slice(bytes).map_err(|_| WizardError::InvalidSettings)?;
        self.apply(value)
    }
    /// Export without credential values, which are not representable.
    pub fn export_json(&self) -> Result<Vec<u8>, WizardError> {
        serde_json::to_vec_pretty(&self.current).map_err(|_| WizardError::InvalidSettings)
    }
    /// Build a validation-only request.
    pub fn dry_run(&self) -> Result<ControlCall, WizardError> {
        if !self.catalog.control.validate_settings {
            return Err(WizardError::ValidationUnavailable);
        }
        validate(&self.catalog, &self.current, true)?;
        Ok(ControlCall::ValidateSettings {
            settings: json!(self.current),
        })
    }
    /// Build plan creation after exact confirmation. This cannot launch a run.
    pub fn confirm_plan(&self, confirmation: &str) -> Result<ControlCall, WizardError> {
        validate(&self.catalog, &self.current, true)?;
        if !self.catalog.control.run_control {
            return Err(WizardError::InvalidSettings);
        }
        if confirmation != "create plan" {
            return Err(WizardError::ConfirmationRequired);
        }
        Ok(ControlCall::CreatePlan(MutationParams {
            idempotency_key: "asb-tui-create-plan-v1".into(),
            definition: json!(self.current),
        }))
    }
}

fn selected(value: &Option<ChoiceId>) -> &str {
    value
        .as_ref()
        .map_or("not selected", |choice| choice.0.as_str())
}
const fn next_step(step: WizardStep) -> WizardStep {
    match step {
        WizardStep::Agent => WizardStep::Provider,
        WizardStep::Provider => WizardStep::Source,
        WizardStep::Source => WizardStep::Workload,
        WizardStep::Workload => WizardStep::Resources,
        WizardStep::Resources => WizardStep::Evidence,
        WizardStep::Evidence | WizardStep::Review => WizardStep::Review,
    }
}
const fn previous_step(step: WizardStep) -> WizardStep {
    match step {
        WizardStep::Agent | WizardStep::Provider => WizardStep::Agent,
        WizardStep::Source => WizardStep::Provider,
        WizardStep::Workload => WizardStep::Source,
        WizardStep::Resources => WizardStep::Workload,
        WizardStep::Evidence => WizardStep::Resources,
        WizardStep::Review => WizardStep::Evidence,
    }
}

fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
fn choices(values: &[ChoiceId]) -> bool {
    !values.is_empty()
        && values.len() <= MAX_CHOICES
        && values.iter().all(|v| {
            !v.0.is_empty()
                && v.0.len() <= 128
                && v.0
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        })
        && values.iter().collect::<BTreeSet<_>>().len() == values.len()
}
fn validate_catalog(c: &WizardCatalog) -> Result<(), WizardError> {
    if !choices(&c.agents)
        || !choices(&c.providers)
        || !choices(&c.workloads)
        || !choices(&c.platforms)
        || !choices(&c.metrics)
        || c.recordings.len() > MAX_CHOICES
        || !c.recordings.iter().all(|v| digest(v))
        || c.recordings.iter().collect::<BTreeSet<_>>().len() != c.recordings.len()
        || c.max_repetitions == 0
        || c.max_concurrency == 0
    {
        Err(WizardError::InvalidCatalog)
    } else {
        Ok(())
    }
}
fn validate(c: &WizardCatalog, s: &WizardSettings, complete: bool) -> Result<(), WizardError> {
    let supported = s.schema_version == WIZARD_SETTINGS_V1
        && s.repetitions <= c.max_repetitions
        && s.concurrency <= c.max_concurrency
        && s.agent.as_ref().is_none_or(|v| c.agents.contains(v))
        && s.provider.as_ref().is_none_or(|v| c.providers.contains(v))
        && s.workload.as_ref().is_none_or(|v| c.workloads.contains(v))
        && s.platform.as_ref().is_none_or(|v| c.platforms.contains(v))
        && s.metrics.iter().all(|v| c.metrics.contains(v))
        && s.metrics.iter().collect::<BTreeSet<_>>().len() == s.metrics.len()
        && s.credential_reference_sha256
            .as_ref()
            .is_none_or(|v| digest(v))
        && match &s.source {
            None => true,
            Some(ProviderSource::Live) => c.live_available,
            Some(ProviderSource::Replay { cassette_sha256 }) => {
                c.recordings.contains(cassette_sha256)
            }
        };
    let filled = s.agent.is_some()
        && s.provider.is_some()
        && s.source.is_some()
        && s.workload.is_some()
        && s.repetitions > 0
        && s.concurrency > 0
        && s.platform.is_some();
    if supported && (!complete || filled) {
        Ok(())
    } else {
        Err(WizardError::InvalidSettings)
    }
}

/// Version of the multi-agent/shared-provider selection contract.
pub const MULTI_AGENT_PROVIDER_SELECTION_V1: u16 = 1;

/// A runner-advertised provider profile and the agents for which it is qualified.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SharedProviderChoice {
    /// Safe display and selection identifier.
    pub id: ChoiceId,
    /// Credential-free digest of the complete provider profile.
    pub profile_sha256: String,
    /// Complete set of agents qualified for this profile.
    pub compatible_agents: Vec<ChoiceId>,
}

/// Negotiated catalog used by the multi-agent/shared-provider selector.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MultiAgentCatalog {
    /// Generic frontend capabilities.
    pub control: Capabilities,
    /// Runner-advertised agent identifiers.
    pub agents: Vec<ChoiceId>,
    /// Runner-advertised provider profiles and compatibility sets.
    pub providers: Vec<SharedProviderChoice>,
    /// Workload choices.
    pub workloads: Vec<ChoiceId>,
    /// Platform choices.
    pub platforms: Vec<ChoiceId>,
    /// Metric choices.
    pub metrics: Vec<ChoiceId>,
    /// Compatible recording digests.
    pub recordings: Vec<String>,
    /// Whether live provider preflight succeeded.
    pub live_available: bool,
    /// Maximum repetitions.
    pub max_repetitions: u32,
    /// Maximum concurrency.
    pub max_concurrency: u32,
}

/// Complete multi-agent settings with exactly one shared provider profile.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MultiAgentSettings {
    /// Contract generation.
    pub schema_version: u16,
    /// Selected agents; order is not semantically significant.
    pub agents: Vec<ChoiceId>,
    /// One provider profile applied to every selected agent.
    pub provider: Option<ChoiceId>,
    /// Explicit execution source.
    pub source: Option<ProviderSource>,
    /// Workload selection.
    pub workload: Option<ChoiceId>,
    /// Repetition count.
    pub repetitions: u32,
    /// Concurrency.
    pub concurrency: u32,
    /// Platform selection.
    pub platform: Option<ChoiceId>,
    /// Metric selections.
    pub metrics: Vec<ChoiceId>,
    /// Non-secret credential reference digest.
    pub credential_reference_sha256: Option<String>,
}

/// The privacy-safe review shown before plan creation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MultiAgentReview {
    /// Canonically ordered selected agents.
    pub agents: Vec<ChoiceId>,
    /// Selected provider identifier.
    pub provider: ChoiceId,
    /// Credential-free provider profile digest.
    pub provider_profile_sha256: String,
    /// Selected workload.
    pub workload: ChoiceId,
    /// Selected platform.
    pub platform: ChoiceId,
    /// Repetition count.
    pub repetitions: u32,
    /// Concurrency.
    pub concurrency: u32,
}

/// Side-effect-free state for selecting several agents and one common profile.
#[derive(Debug)]
pub struct MultiAgentWizard {
    catalog: MultiAgentCatalog,
    current: MultiAgentSettings,
    history: Vec<MultiAgentSettings>,
}

impl MultiAgentWizard {
    /// Construct a selector from one freshly negotiated runner catalog.
    pub fn new(catalog: MultiAgentCatalog) -> Result<Self, WizardError> {
        validate_multi_catalog(&catalog)?;
        Ok(Self {
            catalog,
            current: MultiAgentSettings {
                schema_version: MULTI_AGENT_PROVIDER_SELECTION_V1,
                ..MultiAgentSettings::default()
            },
            history: Vec::new(),
        })
    }

    /// Borrow the current editable settings.
    pub fn settings(&self) -> &MultiAgentSettings {
        &self.current
    }

    /// Search only runner-advertised agents using bounded ASCII folding.
    pub fn search_agents(&self, query: &str) -> Result<Vec<ChoiceId>, WizardError> {
        self.search(&self.catalog.agents, query)
    }

    /// Search only runner-advertised provider profiles.
    pub fn search_providers(&self, query: &str) -> Result<Vec<ChoiceId>, WizardError> {
        let values = self
            .catalog
            .providers
            .iter()
            .map(|provider| provider.id.clone())
            .collect::<Vec<_>>();
        self.search(&values, query)
    }

    /// Return providers compatible with every currently selected agent.
    pub fn compatible_providers(&self) -> Vec<ChoiceId> {
        self.catalog
            .providers
            .iter()
            .filter(|provider| {
                self.current
                    .agents
                    .iter()
                    .all(|agent| provider.compatible_agents.contains(agent))
            })
            .map(|provider| provider.id.clone())
            .collect()
    }

    /// Select a non-empty, duplicate-free set of runner-advertised agents.
    pub fn select_agents(&mut self, agents: Vec<ChoiceId>) -> Result<(), WizardError> {
        let mut next = self.current.clone();
        next.agents = agents;
        if let Some(provider) = &next.provider {
            let choice =
                provider_choice(&self.catalog, provider).ok_or(WizardError::InvalidSettings)?;
            if !next
                .agents
                .iter()
                .all(|agent| choice.compatible_agents.contains(agent))
            {
                next.provider = None;
            }
        }
        validate_multi(&self.catalog, &next, false)?;
        self.history.push(self.current.clone());
        self.current = next;
        Ok(())
    }

    /// Toggle one advertised agent while preserving set semantics.
    pub fn toggle_agent(&mut self, agent: ChoiceId) -> Result<(), WizardError> {
        let mut agents = self.current.agents.clone();
        if let Some(index) = agents.iter().position(|value| value == &agent) {
            agents.remove(index);
        } else {
            agents.push(agent);
        }
        self.select_agents(agents)
    }

    /// Clear all selected agents and the now-unbound provider.
    pub fn clear_agents(&mut self) {
        self.history.push(self.current.clone());
        self.current.agents.clear();
        self.current.provider = None;
    }

    /// Select one provider only when it supports the complete agent set.
    pub fn select_provider(&mut self, provider: ChoiceId) -> Result<(), WizardError> {
        let choice =
            provider_choice(&self.catalog, &provider).ok_or(WizardError::InvalidSettings)?;
        if !self
            .current
            .agents
            .iter()
            .all(|agent| choice.compatible_agents.contains(agent))
        {
            return Err(WizardError::InvalidSettings);
        }
        let mut next = self.current.clone();
        next.provider = Some(provider);
        validate_multi(&self.catalog, &next, false)?;
        self.history.push(self.current.clone());
        self.current = next;
        Ok(())
    }

    /// Apply workload, source, resource, and evidence settings atomically.
    pub fn apply(&mut self, next: MultiAgentSettings) -> Result<(), WizardError> {
        validate_multi(&self.catalog, &next, false)?;
        self.history.push(self.current.clone());
        self.current = next;
        Ok(())
    }

    /// Restore the previous complete edit.
    pub fn back(&mut self) -> bool {
        if let Some(prior) = self.history.pop() {
            self.current = prior;
            true
        } else {
            false
        }
    }

    /// Produce a bounded review only after all required choices are complete.
    pub fn review(&self) -> Result<MultiAgentReview, WizardError> {
        validate_multi(&self.catalog, &self.current, true)?;
        let provider = self
            .current
            .provider
            .clone()
            .ok_or(WizardError::InvalidSettings)?;
        let choice =
            provider_choice(&self.catalog, &provider).ok_or(WizardError::InvalidSettings)?;
        Ok(MultiAgentReview {
            agents: canonical_agents(&self.current.agents),
            provider,
            provider_profile_sha256: choice.profile_sha256.clone(),
            workload: self
                .current
                .workload
                .clone()
                .ok_or(WizardError::InvalidSettings)?,
            platform: self
                .current
                .platform
                .clone()
                .ok_or(WizardError::InvalidSettings)?,
            repetitions: self.current.repetitions,
            concurrency: self.current.concurrency,
        })
    }

    /// Render a deterministic, digest-free plain-text review.
    pub fn render_plain(&self, width: usize) -> Result<String, WizardError> {
        if !(24..=240).contains(&width) {
            return Err(WizardError::InvalidSettings);
        }
        let provider = self
            .current
            .provider
            .as_ref()
            .map_or("not selected", |value| value.0.as_str());
        let agents = if self.current.agents.is_empty() {
            "none".to_owned()
        } else {
            canonical_agents(&self.current.agents)
                .into_iter()
                .map(|agent| agent.0)
                .collect::<Vec<_>>()
                .join(",")
        };
        Ok(format!(
            "ASB multi-agent settings | Agents: {agents}\nProvider: {provider}\nCompatible providers: {}\nExplicit review required; no run starts from this screen.",
            self.compatible_providers().len()
        ))
    }

    /// Build a validation-only request without creating or launching a run.
    pub fn dry_run(&self) -> Result<ControlCall, WizardError> {
        if !self.catalog.control.validate_settings {
            return Err(WizardError::ValidationUnavailable);
        }
        validate_multi(&self.catalog, &self.current, true)?;
        Ok(ControlCall::ValidateSettings {
            settings: json!(self.current),
        })
    }

    /// Build plan creation only after exact explicit confirmation.
    pub fn confirm_plan(&self, confirmation: &str) -> Result<ControlCall, WizardError> {
        let review = self.review()?;
        if !self.catalog.control.run_control {
            return Err(WizardError::InvalidSettings);
        }
        if confirmation != "create plan" {
            return Err(WizardError::ConfirmationRequired);
        }
        Ok(ControlCall::CreatePlan(MutationParams {
            idempotency_key: "asb-tui-multi-agent-provider-plan-v1".into(),
            definition: json!({"selection": self.current, "review": review}),
        }))
    }

    fn search(&self, values: &[ChoiceId], query: &str) -> Result<Vec<ChoiceId>, WizardError> {
        if query.len() > 128 || !query.is_ascii() {
            return Err(WizardError::InvalidSettings);
        }
        let query = query.to_ascii_lowercase();
        Ok(values
            .iter()
            .filter(|value| value.0.to_ascii_lowercase().contains(&query))
            .cloned()
            .collect())
    }
}

fn canonical_agents(agents: &[ChoiceId]) -> Vec<ChoiceId> {
    let mut result = agents.to_vec();
    result.sort();
    result
}

fn provider_choice<'a>(
    catalog: &'a MultiAgentCatalog,
    id: &ChoiceId,
) -> Option<&'a SharedProviderChoice> {
    catalog.providers.iter().find(|provider| provider.id == *id)
}

fn validate_multi_catalog(catalog: &MultiAgentCatalog) -> Result<(), WizardError> {
    if !choices(&catalog.agents)
        || !choices(&catalog.workloads)
        || !choices(&catalog.platforms)
        || !choices(&catalog.metrics)
        || catalog.recordings.len() > MAX_CHOICES
        || !catalog.recordings.iter().all(|value| digest(value))
        || catalog.recordings.iter().collect::<BTreeSet<_>>().len() != catalog.recordings.len()
        || catalog.max_repetitions == 0
        || catalog.max_concurrency == 0
        || catalog.providers.is_empty()
        || catalog.providers.len() > MAX_CHOICES
        || catalog
            .providers
            .iter()
            .map(|provider| &provider.id)
            .collect::<BTreeSet<_>>()
            .len()
            != catalog.providers.len()
    {
        return Err(WizardError::InvalidCatalog);
    }
    if catalog.providers.iter().any(|provider| {
        !digest(&provider.profile_sha256)
            || !choices(&provider.compatible_agents)
            || provider
                .compatible_agents
                .iter()
                .any(|agent| !catalog.agents.contains(agent))
    }) {
        return Err(WizardError::InvalidCatalog);
    }
    Ok(())
}

fn validate_multi(
    catalog: &MultiAgentCatalog,
    settings: &MultiAgentSettings,
    complete: bool,
) -> Result<(), WizardError> {
    if settings.schema_version != MULTI_AGENT_PROVIDER_SELECTION_V1
        || settings.agents.len() > MAX_CHOICES
        || settings.agents.iter().collect::<BTreeSet<_>>().len() != settings.agents.len()
        || settings
            .agents
            .iter()
            .any(|agent| !catalog.agents.contains(agent))
        || settings
            .provider
            .as_ref()
            .is_some_and(|provider| provider_choice(catalog, provider).is_none())
        || settings.repetitions > catalog.max_repetitions
        || settings.concurrency > catalog.max_concurrency
        || settings
            .workload
            .as_ref()
            .is_some_and(|value| !catalog.workloads.contains(value))
        || settings
            .platform
            .as_ref()
            .is_some_and(|value| !catalog.platforms.contains(value))
        || settings
            .metrics
            .iter()
            .any(|value| !catalog.metrics.contains(value))
        || settings.metrics.iter().collect::<BTreeSet<_>>().len() != settings.metrics.len()
        || settings
            .credential_reference_sha256
            .as_ref()
            .is_some_and(|value| !digest(value))
        || !(match &settings.source {
            None => true,
            Some(ProviderSource::Live) => catalog.live_available,
            Some(ProviderSource::Replay { cassette_sha256 }) => {
                catalog.recordings.contains(cassette_sha256)
            }
        })
    {
        return Err(WizardError::InvalidSettings);
    }
    if let Some(provider) = &settings.provider {
        let choice = provider_choice(catalog, provider).ok_or(WizardError::InvalidSettings)?;
        if settings
            .agents
            .iter()
            .any(|agent| !choice.compatible_agents.contains(agent))
        {
            return Err(WizardError::InvalidSettings);
        }
    }
    let complete_selection = !settings.agents.is_empty()
        && settings.provider.is_some()
        && settings.source.is_some()
        && settings.workload.is_some()
        && settings.repetitions > 0
        && settings.concurrency > 0
        && settings.platform.is_some();
    if complete && !complete_selection {
        Err(WizardError::InvalidSettings)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use asb_control::{AttemptId, RunId};
    fn plan_reference() -> PlanReference {
        PlanReference {
            plan_id: "plan:1".into(),
            plan_sha256: "c".repeat(64),
        }
    }
    fn control_limits(max_page_items: u16) -> ControlLimits {
        ControlLimits {
            max_frame_bytes: 65_536,
            max_timeout_ms: 1_000,
            max_page_items,
            max_in_flight: 4,
        }
    }
    fn run_summary(revision: u64, state: PublicRunState) -> RunSummary {
        RunSummary {
            run_id: RunId("run-1".into()),
            attempt_id: AttemptId("attempt-1".into()),
            state,
            created_revision: Revision(4),
            revision: Revision(revision),
            plan_sha256: "c".repeat(64),
        }
    }
    fn catalog() -> WizardCatalog {
        WizardCatalog {
            control: Capabilities {
                validate_settings: true,
                run_control: true,
                repeat: false,
                analysis: false,
                events: true,
            },
            agents: vec![ChoiceId("codex".into())],
            providers: vec![ChoiceId("openai".into())],
            workloads: vec![ChoiceId("bug-fix".into())],
            platforms: vec![ChoiceId("linux".into())],
            metrics: vec![ChoiceId("wall-time".into())],
            recordings: vec!["a".repeat(64)],
            live_available: true,
            max_repetitions: 10,
            max_concurrency: 4,
        }
    }
    fn settings() -> WizardSettings {
        WizardSettings {
            schema_version: 1,
            agent: Some(ChoiceId("codex".into())),
            provider: Some(ChoiceId("openai".into())),
            source: Some(ProviderSource::Replay {
                cassette_sha256: "a".repeat(64),
            }),
            workload: Some(ChoiceId("bug-fix".into())),
            repetitions: 3,
            concurrency: 1,
            platform: Some(ChoiceId("linux".into())),
            metrics: vec![ChoiceId("wall-time".into())],
            credential_reference_sha256: Some("b".repeat(64)),
        }
    }

    fn multi_catalog() -> MultiAgentCatalog {
        MultiAgentCatalog {
            control: catalog().control,
            agents: vec![
                ChoiceId("codex".into()),
                ChoiceId("aider".into()),
                ChoiceId("gemini".into()),
            ],
            providers: vec![
                SharedProviderChoice {
                    id: ChoiceId("openai-shared".into()),
                    profile_sha256: "c".repeat(64),
                    compatible_agents: vec![ChoiceId("aider".into()), ChoiceId("codex".into())],
                },
                SharedProviderChoice {
                    id: ChoiceId("gemini-only".into()),
                    profile_sha256: "d".repeat(64),
                    compatible_agents: vec![ChoiceId("gemini".into())],
                },
            ],
            workloads: vec![ChoiceId("bug-fix".into())],
            platforms: vec![ChoiceId("linux".into())],
            metrics: vec![ChoiceId("wall-time".into())],
            recordings: vec!["a".repeat(64)],
            live_available: true,
            max_repetitions: 10,
            max_concurrency: 4,
        }
    }

    fn multi_settings() -> MultiAgentSettings {
        MultiAgentSettings {
            schema_version: MULTI_AGENT_PROVIDER_SELECTION_V1,
            agents: vec![ChoiceId("codex".into()), ChoiceId("aider".into())],
            provider: Some(ChoiceId("openai-shared".into())),
            source: Some(ProviderSource::Live),
            workload: Some(ChoiceId("bug-fix".into())),
            repetitions: 2,
            concurrency: 1,
            platform: Some(ChoiceId("linux".into())),
            metrics: vec![ChoiceId("wall-time".into())],
            credential_reference_sha256: Some("b".repeat(64)),
        }
    }

    #[test]
    fn multi_agent_selection_requires_one_common_provider_and_is_canonical() {
        let mut wizard = MultiAgentWizard::new(multi_catalog()).unwrap();
        assert_eq!(
            wizard.search_agents("CODE").unwrap(),
            vec![ChoiceId("codex".into())]
        );
        wizard
            .select_agents(vec![ChoiceId("aider".into()), ChoiceId("codex".into())])
            .unwrap();
        assert_eq!(
            wizard.compatible_providers(),
            vec![ChoiceId("openai-shared".into())]
        );
        wizard
            .select_provider(ChoiceId("openai-shared".into()))
            .unwrap();
        wizard.apply(multi_settings()).unwrap();
        let review = wizard.review().unwrap();
        assert_eq!(
            review.agents,
            vec![ChoiceId("aider".into()), ChoiceId("codex".into())]
        );
        assert_eq!(review.provider_profile_sha256, "c".repeat(64));
        let rendered = wizard.render_plain(80).unwrap();
        assert!(rendered.contains("aider,codex"));
        assert!(!rendered.contains(&"c".repeat(64)));
        assert!(matches!(
            wizard.confirm_plan("create plan"),
            Ok(ControlCall::CreatePlan(_))
        ));
    }

    #[test]
    fn multi_agent_selection_rejects_incompatible_and_stale_inputs() {
        let mut wizard = MultiAgentWizard::new(multi_catalog()).unwrap();
        wizard
            .select_agents(vec![ChoiceId("codex".into())])
            .unwrap();
        assert_eq!(
            wizard.select_provider(ChoiceId("gemini-only".into())),
            Err(WizardError::InvalidSettings)
        );
        assert_eq!(
            wizard.select_agents(vec![ChoiceId("unknown".into())]),
            Err(WizardError::InvalidSettings)
        );
        assert!(wizard.search_providers("\u{00e9}").is_err());
        wizard.toggle_agent(ChoiceId("codex".into())).unwrap();
        assert!(wizard.settings().agents.is_empty());
        assert!(wizard.back());
        assert_eq!(wizard.settings().agents, vec![ChoiceId("codex".into())]);
    }

    #[test]
    fn multi_agent_catalog_rejects_unbound_provider_compatibility() {
        let mut catalog = multi_catalog();
        catalog.providers[0]
            .compatible_agents
            .push(ChoiceId("not-advertised".into()));
        assert_eq!(
            MultiAgentWizard::new(catalog).unwrap_err(),
            WizardError::InvalidCatalog
        );
    }
    #[test]
    fn dry_run_and_create_are_explicit_and_never_launch() {
        let mut w = Wizard::new(catalog()).unwrap();
        w.apply(settings()).unwrap();
        assert!(matches!(
            w.dry_run(),
            Ok(ControlCall::ValidateSettings { .. })
        ));
        assert_eq!(
            w.confirm_plan("yes").unwrap_err(),
            WizardError::ConfirmationRequired
        );
        assert!(matches!(
            w.confirm_plan("create plan"),
            Ok(ControlCall::CreatePlan(_))
        ));
    }
    #[test]
    fn back_reset_and_import_are_lossless() {
        let mut w = Wizard::new(catalog()).unwrap();
        w.apply(settings()).unwrap();
        let json = w.export_json().unwrap();
        w.reset();
        assert!(w.back());
        assert_eq!(w.settings(), &settings());
        w.reset();
        w.import_json(&json).unwrap();
        assert_eq!(w.settings(), &settings());
    }
    #[test]
    fn stale_unadvertised_and_secret_shaped_inputs_fail_closed() {
        let mut w = Wizard::new(catalog()).unwrap();
        let mut s = settings();
        s.source = Some(ProviderSource::Replay {
            cassette_sha256: "c".repeat(64),
        });
        assert_eq!(w.apply(s).unwrap_err(), WizardError::InvalidSettings);
        let secret = br#"{"schema_version":1,"agent":"codex","provider":"openai","source":"live","workload":"bug-fix","repetitions":1,"concurrency":1,"platform":"linux","metrics":[],"credential_reference_sha256":null,"credential":"secret"}"#;
        assert_eq!(
            w.import_json(secret).unwrap_err(),
            WizardError::InvalidSettings
        );
    }
    #[test]
    fn keyboard_search_and_plain_snapshot_are_stable() {
        let mut w = Wizard::new(catalog()).unwrap();
        w.command(WizardCommand::Next);
        w.command(WizardCommand::Help);
        assert_eq!(
            w.search(&[ChoiceId("codex".into())], "CODE").unwrap(),
            vec![ChoiceId("codex".into())]
        );
        assert_eq!(
            w.render_plain(80).unwrap(),
            "ASB settings | Provider\nAgent: not selected\nProvider: not selected\nSource: not selected\nWorkload: not selected\nRepetitions: 0  Concurrency: 0\nNo run starts from this screen.\nKeys: next, previous, reset, help"
        );
        w.command(WizardCommand::Previous);
        assert_eq!(w.step(), WizardStep::Agent);
        assert_eq!(
            w.render_plain(12).unwrap_err(),
            WizardError::InvalidSettings
        );
        assert!(
            w.search(&[ChoiceId("codex".into())], &"x".repeat(129))
                .is_err()
        );
        assert!(w.search(&[ChoiceId("codex".into())], "códex").is_err());
    }
    #[test]
    fn snapshots_hide_digests_and_duplicates_fail() {
        let mut w = Wizard::new(catalog()).unwrap();
        let mut duplicate = settings();
        duplicate.metrics.push(ChoiceId("wall-time".into()));
        assert_eq!(
            w.apply(duplicate).unwrap_err(),
            WizardError::InvalidSettings
        );
        w.apply(settings()).unwrap();
        let rendered = w.render_plain(80).unwrap();
        assert!(!rendered.contains(&"a".repeat(64)));
        assert!(!rendered.contains(&"b".repeat(64)));
        assert!(rendered.contains("matching replay"));
        assert!(w.import_json(&vec![b'x'; 65_537]).is_err());
    }

    #[test]
    fn launch_is_confirmed_once_and_returns_durable_identity_before_following() {
        let mut control =
            RunControl::new(plan_reference(), "launch:1".into(), control_limits(128)).unwrap();
        assert_eq!(
            control.launch("yes").unwrap_err(),
            RunControlError::ConfirmationRequired
        );
        assert!(matches!(
            control.launch("launch run").unwrap(),
            ControlCall::Launch(LaunchParams { plan_id, .. }) if plan_id == "plan:1"
        ));
        assert_eq!(
            control.launch("launch run").unwrap_err(),
            RunControlError::InvalidTransition
        );
        let mut wrong_plan = run_summary(5, PublicRunState::Running);
        wrong_plan.plan_sha256 = "d".repeat(64);
        assert_eq!(
            control.accept_launch(wrong_plan).unwrap_err(),
            RunControlError::IdentityMismatch
        );
        control
            .accept_launch(run_summary(5, PublicRunState::Running))
            .unwrap();
        assert_eq!(control.phase(), RunControlPhase::Following);
        assert!(matches!(
            control.status_call(),
            Ok(ControlCall::Status { .. })
        ));
        assert_eq!(
            control.events_call().unwrap(),
            ControlCall::Events(PageParams {
                after: Some(Revision(5)),
                limit: RUN_EVENT_PAGE_ITEMS,
            })
        );
    }

    #[test]
    fn cancellation_acknowledgement_never_fabricates_terminal_state() {
        let mut control =
            RunControl::new(plan_reference(), "launch-1".into(), control_limits(128)).unwrap();
        control.launch("launch run").unwrap();
        control
            .accept_launch(run_summary(5, PublicRunState::Running))
            .unwrap();
        assert!(matches!(
            control.cancel("cancel-1".into()).unwrap(),
            ControlCall::Cancel(CancelParams { run_id, attempt_id, .. })
                if run_id == RunId("run-1".into())
                    && attempt_id == AttemptId("attempt-1".into())
        ));
        control
            .accept_cancel(MutationAcknowledgement { accepted: true })
            .unwrap();
        assert_eq!(control.phase(), RunControlPhase::CancelPending);
        control
            .accept_status(run_summary(6, PublicRunState::Completed))
            .unwrap();
        assert_eq!(
            control.phase(),
            RunControlPhase::Terminal(PublicRunState::Completed)
        );
        assert_eq!(
            control.summary().map(|summary| summary.state),
            Some(PublicRunState::Completed)
        );
    }

    #[test]
    fn reconnect_rejects_event_loss_stale_status_and_other_attempts() {
        let mut control =
            RunControl::new(plan_reference(), "launch-1".into(), control_limits(128)).unwrap();
        control.launch("launch run").unwrap();
        control
            .accept_launch(run_summary(5, PublicRunState::Running))
            .unwrap();
        let event = ControlEvent {
            revision: Revision(7),
            kind: ControlEventKind::RunUpdated,
            run_id: Some(RunId("run-1".into())),
            attempt_id: Some(AttemptId("attempt-1".into())),
        };
        assert_eq!(
            control
                .accept_events(Page {
                    items: vec![event],
                    next: Some(Revision(7)),
                    has_more: false,
                })
                .unwrap_err(),
            RunControlError::StaleProjection
        );
        let mut stale = run_summary(4, PublicRunState::Running);
        stale.created_revision = Revision(3);
        assert_eq!(
            control.accept_status(stale).unwrap_err(),
            RunControlError::IdentityMismatch
        );
        let mut other = run_summary(6, PublicRunState::Running);
        other.attempt_id = AttemptId("attempt-2".into());
        assert_eq!(
            control.accept_status(other).unwrap_err(),
            RunControlError::IdentityMismatch
        );
        let same_revision_change = run_summary(5, PublicRunState::Collecting);
        assert_eq!(
            control.accept_status(same_revision_change).unwrap_err(),
            RunControlError::StaleProjection
        );
        let mut drifted_plan = run_summary(6, PublicRunState::Running);
        drifted_plan.plan_sha256 = "d".repeat(64);
        assert_eq!(
            control.accept_status(drifted_plan).unwrap_err(),
            RunControlError::IdentityMismatch
        );
        control
            .accept_events(Page {
                items: vec![ControlEvent {
                    revision: Revision(6),
                    kind: ControlEventKind::RunUpdated,
                    run_id: Some(RunId("run-1".into())),
                    attempt_id: Some(AttemptId("attempt-1".into())),
                }],
                next: Some(Revision(6)),
                has_more: false,
            })
            .unwrap();
    }

    #[test]
    fn restart_resumes_without_launch_and_terminal_events_are_not_optimistic() {
        let summary = run_summary(5, PublicRunState::Running);
        let mut control =
            RunControl::reconnect(summary, Some(Revision(5)), control_limits(128)).unwrap();
        assert_eq!(
            control.launch("launch run").unwrap_err(),
            RunControlError::InvalidTransition
        );
        control
            .accept_events(Page {
                items: vec![
                    ControlEvent {
                        revision: Revision(6),
                        kind: ControlEventKind::RunCancelled,
                        run_id: Some(RunId("run-1".into())),
                        attempt_id: Some(AttemptId("attempt-1".into())),
                    },
                    ControlEvent {
                        revision: Revision(7),
                        kind: ControlEventKind::RunUpdated,
                        run_id: Some(RunId("run-2".into())),
                        attempt_id: Some(AttemptId("attempt-2".into())),
                    },
                ],
                next: Some(Revision(7)),
                has_more: true,
            })
            .unwrap();
        assert_eq!(
            control.phase(),
            RunControlPhase::Terminal(PublicRunState::Cancelled)
        );
        assert_eq!(control.summary().unwrap().revision, Revision(6));
        assert_eq!(
            control.events_call().unwrap(),
            ControlCall::Events(PageParams {
                after: Some(Revision(7)),
                limit: RUN_EVENT_PAGE_ITEMS,
            })
        );
    }

    #[test]
    fn event_pages_and_cancellation_races_remain_bounded_and_fail_closed() {
        let mut control = RunControl::reconnect(
            run_summary(5, PublicRunState::Running),
            Some(Revision(5)),
            control_limits(128),
        )
        .unwrap();
        control
            .accept_events(Page {
                items: Vec::new(),
                next: None,
                has_more: false,
            })
            .unwrap();
        assert!(matches!(
            control.events_call(),
            Ok(ControlCall::Events(PageParams {
                after: Some(Revision(5)),
                ..
            }))
        ));
        assert_eq!(
            control
                .accept_events(Page {
                    items: (0..=RUN_EVENT_PAGE_ITEMS)
                        .map(|offset| ControlEvent {
                            revision: Revision(6 + u64::from(offset)),
                            kind: ControlEventKind::RunUpdated,
                            run_id: Some(RunId("run-1".into())),
                            attempt_id: Some(AttemptId("attempt-1".into())),
                        })
                        .collect(),
                    next: Some(Revision(6 + u64::from(RUN_EVENT_PAGE_ITEMS))),
                    has_more: false,
                })
                .unwrap_err(),
            RunControlError::StaleProjection
        );
        control.cancel("cancel-1".into()).unwrap();
        assert_eq!(
            control.cancel("cancel-2".into()).unwrap_err(),
            RunControlError::InvalidTransition
        );
        assert_eq!(
            control
                .accept_cancel(MutationAcknowledgement { accepted: false })
                .unwrap_err(),
            RunControlError::NotAcknowledged
        );
        control
            .accept_status(run_summary(6, PublicRunState::Running))
            .unwrap();
        assert_eq!(control.phase(), RunControlPhase::CancelPending);
        let mut exhausted = RunControl::reconnect(
            run_summary(u64::MAX, PublicRunState::Running),
            Some(Revision(u64::MAX)),
            control_limits(128),
        )
        .unwrap();
        assert_eq!(
            exhausted
                .accept_events(Page {
                    items: vec![ControlEvent {
                        revision: Revision(u64::MAX),
                        kind: ControlEventKind::RunUpdated,
                        run_id: Some(RunId("run-1".into())),
                        attempt_id: Some(AttemptId("attempt-1".into())),
                    }],
                    next: Some(Revision(u64::MAX)),
                    has_more: false,
                })
                .unwrap_err(),
            RunControlError::StaleProjection
        );
    }

    #[test]
    fn lifecycle_regressions_and_reconciliation_are_not_presented_as_running() {
        let mut control = RunControl::reconnect(
            run_summary(5, PublicRunState::Running),
            Some(Revision(5)),
            control_limits(128),
        )
        .unwrap();
        assert_eq!(
            control
                .accept_status(run_summary(6, PublicRunState::Prepared))
                .unwrap_err(),
            RunControlError::StaleProjection
        );
        control
            .accept_events(Page {
                items: vec![ControlEvent {
                    revision: Revision(6),
                    kind: ControlEventKind::ReconciliationRequired,
                    run_id: Some(RunId("run-1".into())),
                    attempt_id: Some(AttemptId("attempt-1".into())),
                }],
                next: Some(Revision(6)),
                has_more: false,
            })
            .unwrap();
        assert_eq!(
            control.phase(),
            RunControlPhase::Terminal(PublicRunState::NeedsReconciliation)
        );
        assert_eq!(
            control.cancel("cancel-2".into()).unwrap_err(),
            RunControlError::InvalidTransition
        );
    }

    #[test]
    fn snapshots_events_and_negotiated_pages_preserve_runner_truth() {
        let mut running = RunControl::reconnect(
            run_summary(5, PublicRunState::Running),
            Some(Revision(5)),
            control_limits(128),
        )
        .unwrap();
        running
            .accept_status(run_summary(7, PublicRunState::Completed))
            .unwrap();
        assert_eq!(
            running.phase(),
            RunControlPhase::Terminal(PublicRunState::Completed)
        );

        let mut prepared = RunControl::reconnect(
            run_summary(5, PublicRunState::Prepared),
            Some(Revision(5)),
            control_limits(7),
        )
        .unwrap();
        assert_eq!(
            prepared.events_call().unwrap(),
            ControlCall::Events(PageParams {
                after: Some(Revision(5)),
                limit: 7,
            })
        );
        for malformed in [
            ControlEvent {
                revision: Revision(6),
                kind: ControlEventKind::RunCompleted,
                run_id: None,
                attempt_id: None,
            },
            ControlEvent {
                revision: Revision(6),
                kind: ControlEventKind::RunnerReady,
                run_id: Some(RunId("run-1".into())),
                attempt_id: Some(AttemptId("attempt-1".into())),
            },
        ] {
            assert_eq!(
                prepared
                    .accept_events(Page {
                        items: vec![malformed],
                        next: Some(Revision(6)),
                        has_more: false,
                    })
                    .unwrap_err(),
                RunControlError::StaleProjection
            );
            assert!(matches!(
                prepared.events_call(),
                Ok(ControlCall::Events(PageParams {
                    after: Some(Revision(5)),
                    ..
                }))
            ));
        }
        let too_large = (0_u64..8)
            .map(|offset| ControlEvent {
                revision: Revision(6 + offset),
                kind: ControlEventKind::RunUpdated,
                run_id: Some(RunId("run-2".into())),
                attempt_id: Some(AttemptId("attempt-2".into())),
            })
            .collect();
        assert_eq!(
            prepared
                .accept_events(Page {
                    items: too_large,
                    next: Some(Revision(13)),
                    has_more: false,
                })
                .unwrap_err(),
            RunControlError::StaleProjection
        );
        prepared
            .accept_status(run_summary(8, PublicRunState::Completed))
            .unwrap();
        assert_eq!(
            prepared.phase(),
            RunControlPhase::Terminal(PublicRunState::Completed)
        );
    }

    #[test]
    fn post_terminal_corruption_converges_only_at_a_newer_revision() {
        let terminal = run_summary(5, PublicRunState::Completed);
        let mut status_control =
            RunControl::reconnect(terminal.clone(), Some(Revision(5)), control_limits(128))
                .unwrap();
        assert_eq!(
            status_control
                .accept_status(run_summary(5, PublicRunState::NeedsReconciliation))
                .unwrap_err(),
            RunControlError::StaleProjection
        );
        status_control
            .accept_status(run_summary(6, PublicRunState::NeedsReconciliation))
            .unwrap();
        assert_eq!(
            status_control.phase(),
            RunControlPhase::Terminal(PublicRunState::NeedsReconciliation)
        );

        let mut event_control =
            RunControl::reconnect(terminal, Some(Revision(5)), control_limits(128)).unwrap();
        for kind in [ControlEventKind::RunFailed, ControlEventKind::RunUpdated] {
            assert_eq!(
                event_control
                    .accept_events(Page {
                        items: vec![ControlEvent {
                            revision: Revision(6),
                            kind,
                            run_id: Some(RunId("run-1".into())),
                            attempt_id: Some(AttemptId("attempt-1".into())),
                        }],
                        next: Some(Revision(6)),
                        has_more: false,
                    })
                    .unwrap_err(),
                RunControlError::StaleProjection
            );
            assert_eq!(
                event_control.summary().unwrap().state,
                PublicRunState::Completed
            );
            assert!(matches!(
                event_control.events_call(),
                Ok(ControlCall::Events(PageParams {
                    after: Some(Revision(5)),
                    ..
                }))
            ));
        }
        event_control
            .accept_events(Page {
                items: vec![ControlEvent {
                    revision: Revision(6),
                    kind: ControlEventKind::ReconciliationRequired,
                    run_id: Some(RunId("run-1".into())),
                    attempt_id: Some(AttemptId("attempt-1".into())),
                }],
                next: Some(Revision(6)),
                has_more: false,
            })
            .unwrap();
        assert_eq!(
            event_control.phase(),
            RunControlPhase::Terminal(PublicRunState::NeedsReconciliation)
        );
        assert_eq!(
            event_control.summary().unwrap().state,
            PublicRunState::NeedsReconciliation
        );
        event_control
            .accept_events(Page {
                items: vec![ControlEvent {
                    revision: Revision(7),
                    kind: ControlEventKind::RunFailed,
                    run_id: Some(RunId("run-1".into())),
                    attempt_id: Some(AttemptId("attempt-1".into())),
                }],
                next: Some(Revision(7)),
                has_more: false,
            })
            .unwrap();
        assert_eq!(
            event_control.phase(),
            RunControlPhase::Terminal(PublicRunState::Failed)
        );
    }
}
