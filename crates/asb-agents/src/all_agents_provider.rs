// SPDX-License-Identifier: MIT
//! Atomic provider-profile selection for a complete chosen agent set.

use crate::ollama::{OllamaAgent, OllamaApiMode, OllamaError, VerifiedOllamaProfile};
use crate::openai::{OpenAiAgent, OpenAiApiMode, OpenAiProfile, OpenAiProfileError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;

/// Current all-agent provider-selection contract generation.
pub const ALL_AGENTS_PROVIDER_SELECTION_V1: u16 = 1;

/// Stable built-in agent identity used by all-agent provider selection.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectedAgent {
    /// OpenCode.
    OpenCode,
    /// OpenDesk.
    OpenDesk,
    /// aider.
    Aider,
    /// Codex.
    Codex,
    /// Gemini CLI.
    Gemini,
    /// Qwen Code.
    QwenCode,
    /// Goose.
    Goose,
    /// mini-SWE-agent.
    MiniSwe,
    /// OpenHands.
    OpenHands,
}

/// The single provider profile selected for the complete agent set.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AllAgentsProviderKind {
    /// Pinned public OpenAI profile.
    OpenAi,
    /// Pinned, locally verified Ollama profile.
    Ollama,
}

/// Versioned configuration selecting one provider for all chosen agents.
///
/// Per-agent overrides are intentionally not representable in this contract.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AllAgentsProviderSelection {
    /// Contract generation.
    pub schema_version: u16,
    /// One profile family applied to every selected agent.
    pub provider: AllAgentsProviderKind,
    /// Complete chosen agent set; order is not semantically significant.
    pub agents: Vec<SelectedAgent>,
}

/// Provider API route selected for an agent.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectiveApiMode {
    /// OpenAI-compatible chat completions.
    ChatCompletions,
    /// OpenAI-compatible Responses API.
    Responses,
}

/// A field whose exact application failed during preflight.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnsupportedProviderField {
    /// The adapter has no exact route for the chosen provider profile.
    ProviderRoute,
}

/// Deterministic credential-free incompatibility report for one agent.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderPreflightIssue {
    /// Incompatible selected agent.
    pub agent: SelectedAgent,
    /// Complete unsupported field set.
    pub unsupported_fields: BTreeSet<UnsupportedProviderField>,
}

/// Effective provider settings for one selected agent.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EffectiveAgentProvider {
    /// Selected agent.
    pub agent: SelectedAgent,
    /// Exact common credential-free profile identity.
    pub profile_sha256: String,
    /// Exact provider API route.
    pub api_mode: EffectiveApiMode,
}

/// Constructor-controlled complete preflight result.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AllAgentsProviderPlan {
    /// Canonically ordered settings for the complete selected set.
    effective: Vec<EffectiveAgentProvider>,
}

impl AllAgentsProviderPlan {
    /// Borrow the complete, canonically ordered effective settings.
    pub fn effective(&self) -> &[EffectiveAgentProvider] {
        &self.effective
    }

    /// Exact common profile identity, present because empty selections are rejected.
    pub fn profile_sha256(&self) -> &str {
        &self.effective[0].profile_sha256
    }
}

/// Atomic all-agent preflight failure; no partial plan is returned.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AllAgentsProviderError {
    /// The selection contract generation is unsupported.
    UnsupportedVersion(u16),
    /// The supplied profile family differs from the configured family.
    WrongProvider,
    /// At least one agent must be selected.
    EmptySelection,
    /// Duplicate identities make the requested set ambiguous.
    DuplicateAgent(SelectedAgent),
    /// One or more adapters cannot preserve the selected profile.
    Incompatible(Vec<ProviderPreflightIssue>),
    /// The common provider profile itself is invalid or changed.
    ProfileMismatch,
}

impl fmt::Display for AllAgentsProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedVersion(version) => {
                write!(
                    formatter,
                    "unsupported all-agent provider selection version: {version}"
                )
            }
            Self::WrongProvider => formatter.write_str("configured provider profile mismatch"),
            Self::EmptySelection => formatter.write_str("selected agent set is empty"),
            Self::DuplicateAgent(agent) => write!(formatter, "duplicate selected agent: {agent:?}"),
            Self::Incompatible(issues) => write!(
                formatter,
                "provider profile is incompatible with {} selected agent(s)",
                issues.len()
            ),
            Self::ProfileMismatch => {
                formatter.write_str("provider profile changed during preflight")
            }
        }
    }
}

impl std::error::Error for AllAgentsProviderError {}

/// Preflight one public OpenAI profile for the complete selected set atomically.
pub fn preflight_openai_all(
    profile: &OpenAiProfile,
    selected: &[SelectedAgent],
) -> Result<AllAgentsProviderPlan, AllAgentsProviderError> {
    preflight(
        selected,
        profile.provider_profile().settings_sha256.as_str(),
        |agent| {
            profile
                .translate(openai_agent(agent), profile.provider_profile())
                .map(|route| match route.api_mode() {
                    OpenAiApiMode::ChatCompletions => EffectiveApiMode::ChatCompletions,
                    OpenAiApiMode::Responses => EffectiveApiMode::Responses,
                })
                .map_err(|error| match error {
                    OpenAiProfileError::UnsupportedAgent(_) => PreflightFailure::UnsupportedRoute,
                    _ => PreflightFailure::ProfileMismatch,
                })
        },
    )
}

/// Resolve a versioned OpenAI selection without permitting mixed profiles.
pub fn resolve_openai_selection(
    selection: &AllAgentsProviderSelection,
    profile: &OpenAiProfile,
) -> Result<AllAgentsProviderPlan, AllAgentsProviderError> {
    validate_selection(selection, AllAgentsProviderKind::OpenAi)?;
    preflight_openai_all(profile, &selection.agents)
}

/// Preflight one already verified Ollama profile for the complete selected set atomically.
pub fn preflight_ollama_all(
    profile: &VerifiedOllamaProfile,
    selected: &[SelectedAgent],
) -> Result<AllAgentsProviderPlan, AllAgentsProviderError> {
    preflight(
        selected,
        profile.provider_profile().settings_sha256.as_str(),
        |agent| {
            profile
                .translate(ollama_agent(agent))
                .map(|route| match route.api_mode() {
                    OllamaApiMode::ChatCompletions => EffectiveApiMode::ChatCompletions,
                    OllamaApiMode::Responses => EffectiveApiMode::Responses,
                })
                .map_err(|error| match error {
                    OllamaError::UnsupportedAgent(_) => PreflightFailure::UnsupportedRoute,
                    _ => PreflightFailure::ProfileMismatch,
                })
        },
    )
}

/// Resolve a versioned Ollama selection without permitting mixed profiles.
pub fn resolve_ollama_selection(
    selection: &AllAgentsProviderSelection,
    profile: &VerifiedOllamaProfile,
) -> Result<AllAgentsProviderPlan, AllAgentsProviderError> {
    validate_selection(selection, AllAgentsProviderKind::Ollama)?;
    preflight_ollama_all(profile, &selection.agents)
}

fn validate_selection(
    selection: &AllAgentsProviderSelection,
    expected: AllAgentsProviderKind,
) -> Result<(), AllAgentsProviderError> {
    if selection.schema_version != ALL_AGENTS_PROVIDER_SELECTION_V1 {
        return Err(AllAgentsProviderError::UnsupportedVersion(
            selection.schema_version,
        ));
    }
    if selection.provider != expected {
        return Err(AllAgentsProviderError::WrongProvider);
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum PreflightFailure {
    UnsupportedRoute,
    ProfileMismatch,
}

fn preflight(
    selected: &[SelectedAgent],
    profile_sha256: &str,
    mut resolve: impl FnMut(SelectedAgent) -> Result<EffectiveApiMode, PreflightFailure>,
) -> Result<AllAgentsProviderPlan, AllAgentsProviderError> {
    if selected.is_empty() {
        return Err(AllAgentsProviderError::EmptySelection);
    }
    let ordered = selected.iter().copied().collect::<BTreeSet<_>>();
    if ordered.len() != selected.len() {
        let mut seen = BTreeSet::new();
        let duplicate = selected
            .iter()
            .copied()
            .find(|agent| !seen.insert(*agent))
            .expect("set length proved a duplicate");
        return Err(AllAgentsProviderError::DuplicateAgent(duplicate));
    }

    let mut effective = Vec::with_capacity(ordered.len());
    let mut issues = Vec::new();
    for agent in ordered {
        match resolve(agent) {
            Ok(api_mode) => effective.push(EffectiveAgentProvider {
                agent,
                profile_sha256: profile_sha256.to_owned(),
                api_mode,
            }),
            Err(PreflightFailure::UnsupportedRoute) => issues.push(ProviderPreflightIssue {
                agent,
                unsupported_fields: BTreeSet::from([UnsupportedProviderField::ProviderRoute]),
            }),
            Err(PreflightFailure::ProfileMismatch) => {
                return Err(AllAgentsProviderError::ProfileMismatch);
            }
        }
    }
    if issues.is_empty() {
        Ok(AllAgentsProviderPlan { effective })
    } else {
        Err(AllAgentsProviderError::Incompatible(issues))
    }
}

const fn openai_agent(agent: SelectedAgent) -> OpenAiAgent {
    match agent {
        SelectedAgent::OpenCode => OpenAiAgent::OpenCode,
        SelectedAgent::OpenDesk => OpenAiAgent::OpenDesk,
        SelectedAgent::Aider => OpenAiAgent::Aider,
        SelectedAgent::Codex => OpenAiAgent::Codex,
        SelectedAgent::Gemini => OpenAiAgent::Gemini,
        SelectedAgent::QwenCode => OpenAiAgent::QwenCode,
        SelectedAgent::Goose => OpenAiAgent::Goose,
        SelectedAgent::MiniSwe => OpenAiAgent::MiniSwe,
        SelectedAgent::OpenHands => OpenAiAgent::OpenHands,
    }
}

const fn ollama_agent(agent: SelectedAgent) -> OllamaAgent {
    match agent {
        SelectedAgent::OpenCode => OllamaAgent::OpenCode,
        SelectedAgent::OpenDesk => OllamaAgent::OpenDesk,
        SelectedAgent::Aider => OllamaAgent::Aider,
        SelectedAgent::Codex => OllamaAgent::Codex,
        SelectedAgent::Gemini => OllamaAgent::Gemini,
        SelectedAgent::QwenCode => OllamaAgent::QwenCode,
        SelectedAgent::Goose => OllamaAgent::Goose,
        SelectedAgent::MiniSwe => OllamaAgent::MiniSwe,
        SelectedAgent::OpenHands => OllamaAgent::OpenHands,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ollama::{OLLAMA_MODEL_SHA256, OllamaProfile};
    use url::Url;

    const COMPATIBLE: [SelectedAgent; 8] = [
        SelectedAgent::OpenCode,
        SelectedAgent::OpenDesk,
        SelectedAgent::Aider,
        SelectedAgent::Codex,
        SelectedAgent::QwenCode,
        SelectedAgent::Goose,
        SelectedAgent::MiniSwe,
        SelectedAgent::OpenHands,
    ];

    #[test]
    fn openai_selection_is_complete_canonical_and_identical() {
        let profile = OpenAiProfile::new("a".repeat(64)).unwrap();
        let mut reversed = COMPATIBLE;
        reversed.reverse();
        let plan = preflight_openai_all(&profile, &reversed).unwrap();
        assert_eq!(plan.effective().len(), COMPATIBLE.len());
        assert!(
            plan.effective()
                .windows(2)
                .all(|pair| pair[0].agent < pair[1].agent)
        );
        assert!(
            plan.effective()
                .iter()
                .all(|item| item.profile_sha256 == plan.profile_sha256())
        );
        assert_eq!(
            plan.effective()
                .iter()
                .find(|item| item.agent == SelectedAgent::Codex)
                .unwrap()
                .api_mode,
            EffectiveApiMode::Responses
        );
        assert_eq!(
            serde_json::to_vec(&plan).unwrap(),
            serde_json::to_vec(&plan).unwrap()
        );
    }

    #[test]
    fn unsupported_agent_rejects_the_whole_set_with_exact_fields() {
        let profile = OpenAiProfile::new("a".repeat(64)).unwrap();
        let mut selected = COMPATIBLE.to_vec();
        selected.push(SelectedAgent::Gemini);
        let error = preflight_openai_all(&profile, &selected).unwrap_err();
        assert_eq!(
            error,
            AllAgentsProviderError::Incompatible(vec![ProviderPreflightIssue {
                agent: SelectedAgent::Gemini,
                unsupported_fields: BTreeSet::from([UnsupportedProviderField::ProviderRoute]),
            }])
        );
    }

    #[test]
    fn empty_and_ambiguous_selections_fail_closed() {
        let profile = OpenAiProfile::new("a".repeat(64)).unwrap();
        assert_eq!(
            preflight_openai_all(&profile, &[]),
            Err(AllAgentsProviderError::EmptySelection)
        );
        assert_eq!(
            preflight_openai_all(&profile, &[SelectedAgent::Codex, SelectedAgent::Codex]),
            Err(AllAgentsProviderError::DuplicateAgent(SelectedAgent::Codex))
        );
    }

    #[test]
    fn versioned_selection_rejects_stale_mixed_and_override_shapes() {
        let profile = OpenAiProfile::new("a".repeat(64)).unwrap();
        let selection = AllAgentsProviderSelection {
            schema_version: ALL_AGENTS_PROVIDER_SELECTION_V1,
            provider: AllAgentsProviderKind::OpenAi,
            agents: COMPATIBLE.to_vec(),
        };
        assert_eq!(
            resolve_openai_selection(&selection, &profile)
                .unwrap()
                .effective()
                .len(),
            COMPATIBLE.len()
        );

        let mut stale = selection.clone();
        stale.schema_version += 1;
        assert_eq!(
            resolve_openai_selection(&stale, &profile),
            Err(AllAgentsProviderError::UnsupportedVersion(2))
        );
        let mut mixed = selection.clone();
        mixed.provider = AllAgentsProviderKind::Ollama;
        assert_eq!(
            resolve_openai_selection(&mixed, &profile),
            Err(AllAgentsProviderError::WrongProvider)
        );

        let mut serialized = serde_json::to_value(&selection).unwrap();
        serialized["per_agent_overrides"] = serde_json::json!({"codex": "other"});
        assert!(serde_json::from_value::<AllAgentsProviderSelection>(serialized).is_err());
    }

    #[test]
    fn verified_ollama_uses_the_same_atomic_boundary() {
        let profile = OllamaProfile::new(Url::parse("http://127.0.0.1:11434/").unwrap()).unwrap();
        let tags = format!(
            r#"{{"models":[{{"name":"qwen3-coder:30b","model":"qwen3-coder:30b","digest":"{OLLAMA_MODEL_SHA256}","size":18556700761,"details":{{"format":"gguf","family":"qwen3moe","quantization_level":"Q4_K_M","context_length":262144}}}}]}}"#
        );
        let verified = profile
            .verify_probe(br#"{"version":"0.33.1"}"#, tags.as_bytes())
            .unwrap();
        let plan = preflight_ollama_all(&verified, &COMPATIBLE).unwrap();
        assert_eq!(plan.effective().len(), COMPATIBLE.len());
        assert!(
            plan.effective()
                .iter()
                .all(|item| item.profile_sha256 == plan.profile_sha256())
        );
    }
}
