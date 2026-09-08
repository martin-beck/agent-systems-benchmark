// SPDX-License-Identifier: MIT
#![forbid(unsafe_code)]
#![deny(missing_docs)]
//! Capability-driven state for the independent settings wizard.

use asb_control::{Capabilities, ControlCall, MutationParams};
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

#[cfg(test)]
mod tests {
    use super::*;
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
}
