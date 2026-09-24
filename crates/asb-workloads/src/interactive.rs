// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Offline stateful/tool-use fixtures for interactive literature families.
//!
//! These adapters model the evaluator boundary without importing an upstream
//! framework or contacting a provider.  They are deliberately content-free:
//! only stable identities and bounded counters are returned as evidence.

use crate::literature::{LiteratureAdapter, LiteratureError, LiteraturePrepared};
use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::path::PathBuf;

/// Interactive workload families with distinct evaluator dimensions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InteractiveFamily {
    /// Stateful environments and task reset semantics.
    AgentBench,
    /// Tool use with simulated users and repeated-trial reliability.
    TauBench,
    /// Tool-use utility under safety attacks and defenses.
    AgentDojo,
}

impl InteractiveFamily {
    /// Resolve a stable workload ID.
    pub fn from_id(id: &str) -> Result<Self, InteractiveError> {
        match id {
            "agentbench" => Ok(Self::AgentBench),
            "tau-bench" => Ok(Self::TauBench),
            "agentdojo" => Ok(Self::AgentDojo),
            _ => Err(InteractiveError::UnsupportedFamily),
        }
    }

    /// Stable ASB workload ID.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::AgentBench => "agentbench",
            Self::TauBench => "tau-bench",
            Self::AgentDojo => "agentdojo",
        }
    }

    const fn task_revision(self) -> &'static str {
        match self {
            Self::AgentBench => "d1e4a10db08c87075c78972e48ecc182be03e2d5",
            Self::TauBench => "59a200c6d575d595120f1cb70fea53cef0632f6b",
            Self::AgentDojo => "089ed468cf3ed0322acc66b0211f26d9d90dbf60",
        }
    }
}

/// One bounded tool invocation supplied by a deterministic test agent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InteractiveToolCall {
    /// Stable tool name; arguments are intentionally not retained.
    pub name: String,
    /// Whether the local fixture policy permits this call.
    pub permitted: bool,
}

/// One simulated-user turn.  Only role and bounded content length are used.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SimulatedUserTurn {
    /// `user` or `assistant`.
    pub role: String,
    /// Bounded, non-secret turn text.
    pub content: String,
}

/// Configuration for one deterministic interactive attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InteractiveMockConfig {
    /// Exact task revision the caller prepared.
    pub task_revision: String,
    /// Revision the caller expects to grade.
    pub expected_revision: String,
    /// Scorer contract identity.
    pub scorer_revision: String,
    /// Ordered tool calls to validate.
    pub tool_calls: Vec<InteractiveToolCall>,
    /// Simulated-user transcript shape (content is never emitted in results).
    pub simulated_user: Vec<SimulatedUserTurn>,
    /// Number of deterministic repeated attempts for pass@k/pass^k.
    pub attempts: u32,
    /// Whether the environment was reset before this attempt.
    pub state_reset: bool,
    /// AgentDojo safety outcome, kept separate from useful completion.
    pub policy_violation: bool,
}

impl InteractiveMockConfig {
    /// Construct the valid fixture configuration for a family.
    #[must_use]
    pub fn for_family(family: InteractiveFamily) -> Self {
        Self {
            task_revision: family.task_revision().into(),
            expected_revision: family.task_revision().into(),
            scorer_revision: "asb-interactive-local-scorer-v1".into(),
            tool_calls: Vec::new(),
            simulated_user: vec![SimulatedUserTurn {
                role: "user".into(),
                content: "fixture request".into(),
            }],
            attempts: 1,
            state_reset: true,
            policy_violation: false,
        }
    }
}

/// Content-free result for one interactive local fixture.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InteractiveMockResult {
    workload_id: String,
    task_revision: String,
    scorer_revision: String,
    passed: bool,
    policy_violations: u32,
    pass_at_k: bool,
    pass_k: bool,
}

impl InteractiveMockResult {
    /// Stable workload identity.
    #[must_use]
    pub fn workload_id(&self) -> &str {
        &self.workload_id
    }
    /// Exact task revision graded.
    #[must_use]
    pub fn task_revision(&self) -> &str {
        &self.task_revision
    }
    /// Local scorer identity.
    #[must_use]
    pub fn scorer_revision(&self) -> &str {
        &self.scorer_revision
    }
    /// Useful completion result, independent of policy violations.
    #[must_use]
    pub const fn passed(&self) -> bool {
        self.passed
    }
    /// Number of safety-policy violations observed.
    #[must_use]
    pub const fn policy_violations(&self) -> u32 {
        self.policy_violations
    }
    /// Whether at least one of k attempts passed.
    #[must_use]
    pub const fn pass_at_k(&self) -> bool {
        self.pass_at_k
    }
    /// Whether all k attempts passed (empirical pass^k).
    #[must_use]
    pub const fn pass_k(&self) -> bool {
        self.pass_k
    }
}

/// Fail-closed interactive fixture errors.
#[derive(Debug)]
pub enum InteractiveError {
    /// The ID is not one of the three supported interactive families.
    UnsupportedFamily,
    /// Prepared task and requested revision differ.
    StaleRevision,
    /// Tool policy rejected a call.
    UnsafeToolCall,
    /// Simulated-user turns are malformed or exceed bounds.
    MalformedSimulatedUser,
    /// The scorer contract is not the pinned local scorer.
    ScorerMismatch,
    /// The environment was not reset between attempts.
    ResetLeakage,
    /// Repeated-attempt count is outside the bounded contract.
    InvalidAttemptCount,
    /// Underlying literature fixture failed.
    Literature(LiteratureError),
}

impl fmt::Display for InteractiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::UnsupportedFamily => "unsupported interactive workload family",
            Self::StaleRevision => "interactive task revision is stale",
            Self::UnsafeToolCall => "interactive tool call violates local policy",
            Self::MalformedSimulatedUser => "simulated user is malformed",
            Self::ScorerMismatch => "interactive scorer contract does not match",
            Self::ResetLeakage => "interactive environment was not reset",
            Self::InvalidAttemptCount => "interactive attempt count is outside the bound",
            Self::Literature(error) => return error.fmt(f),
        })
    }
}

impl std::error::Error for InteractiveError {}

impl From<LiteratureError> for InteractiveError {
    fn from(value: LiteratureError) -> Self {
        Self::Literature(value)
    }
}

/// Prepared, private state for an interactive local fixture.
#[derive(Debug)]
pub struct InteractivePrepared {
    family: InteractiveFamily,
    literature: LiteraturePrepared,
}

impl InteractivePrepared {
    /// Agent-writable workspace path.
    #[must_use]
    pub fn workspace(&self) -> PathBuf {
        self.literature.workspace()
    }

    /// Run a bounded local interactive evaluation.
    pub fn run_local_mock(
        &self,
        config: InteractiveMockConfig,
    ) -> Result<InteractiveMockResult, InteractiveError> {
        if config.task_revision != self.family.task_revision()
            || config.expected_revision != config.task_revision
        {
            return Err(InteractiveError::StaleRevision);
        }
        if config.scorer_revision != "asb-interactive-local-scorer-v1" {
            return Err(InteractiveError::ScorerMismatch);
        }
        if config.attempts == 0 || config.attempts > 16 {
            return Err(InteractiveError::InvalidAttemptCount);
        }
        if !config.state_reset {
            return Err(InteractiveError::ResetLeakage);
        }
        validate_user(&config.simulated_user)?;
        for call in &config.tool_calls {
            if !call.permitted || !safe_tool_name(&call.name) {
                return Err(InteractiveError::UnsafeToolCall);
            }
        }
        let passed = fs::read_to_string(self.workspace().join("mock-answer.txt"))
            .map(|text| text.lines().next().unwrap_or_default().trim() == "ASB-LOCAL-MOCK-OK")
            .unwrap_or(false)
            && !config.policy_violation;
        Ok(InteractiveMockResult {
            workload_id: self.family.id().into(),
            task_revision: self.family.task_revision().into(),
            scorer_revision: config.scorer_revision,
            passed,
            policy_violations: u32::from(config.policy_violation),
            pass_at_k: passed,
            pass_k: passed,
        })
    }

    /// Reset the stateful fixture and remove agent-created state.
    pub fn reset(&self) -> Result<(), InteractiveError> {
        self.literature.reset().map_err(InteractiveError::from)
    }

    /// Remove the exact owned fixture root.
    pub fn cleanup(self) -> Result<(), InteractiveError> {
        self.literature.cleanup().map_err(InteractiveError::from)
    }
}

/// Adapter for AgentBench, tau-bench, and AgentDojo local fixtures.
#[derive(Clone, Copy, Debug, Default)]
pub struct InteractiveAdapter;

impl InteractiveAdapter {
    /// Prepare one offline fixture; no upstream source is acquired.
    pub fn prepare(
        id: &str,
        root: impl Into<PathBuf>,
    ) -> Result<InteractivePrepared, InteractiveError> {
        let family = InteractiveFamily::from_id(id)?;
        Ok(InteractivePrepared {
            family,
            literature: LiteratureAdapter::prepare(id, root)?,
        })
    }
}

fn validate_user(turns: &[SimulatedUserTurn]) -> Result<(), InteractiveError> {
    if turns.is_empty() || turns.len() > 32 {
        return Err(InteractiveError::MalformedSimulatedUser);
    }
    let mut roles = BTreeSet::new();
    for turn in turns {
        if !matches!(turn.role.as_str(), "user" | "assistant")
            || turn.content.is_empty()
            || turn.content.len() > 4096
        {
            return Err(InteractiveError::MalformedSimulatedUser);
        }
        roles.insert(turn.role.as_str());
    }
    if !roles.contains("user") {
        return Err(InteractiveError::MalformedSimulatedUser);
    }
    Ok(())
}

fn safe_tool_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name.is_ascii()
        && !name.contains("network")
        && !name.contains("delete")
        && !name.contains("shell")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "asb-interactive-{label}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn three_families_run_offline_and_keep_safety_separate() {
        for family in [
            InteractiveFamily::AgentBench,
            InteractiveFamily::TauBench,
            InteractiveFamily::AgentDojo,
        ] {
            let prepared = InteractiveAdapter::prepare(family.id(), root(family.id())).unwrap();
            fs::write(
                prepared.workspace().join("mock-answer.txt"),
                "ASB-LOCAL-MOCK-OK",
            )
            .unwrap();
            let mut config = InteractiveMockConfig::for_family(family);
            if family == InteractiveFamily::TauBench {
                config.tool_calls.push(InteractiveToolCall {
                    name: "lookup".into(),
                    permitted: true,
                });
            }
            if family == InteractiveFamily::AgentDojo {
                config.policy_violation = true;
                let result = prepared.run_local_mock(config).unwrap();
                assert!(!result.passed());
                assert_eq!(result.policy_violations(), 1);
            } else {
                let result = prepared.run_local_mock(config).unwrap();
                assert!(result.passed());
                assert!(result.pass_at_k());
                assert!(result.pass_k());
            }
            prepared.cleanup().unwrap();
        }
    }

    #[test]
    fn interactive_negative_contracts_fail_closed() {
        let prepared = InteractiveAdapter::prepare("tau-bench", root("negative")).unwrap();
        let mut config = InteractiveMockConfig::for_family(InteractiveFamily::TauBench);
        config.expected_revision = "stale".into();
        assert!(matches!(
            prepared.run_local_mock(config),
            Err(InteractiveError::StaleRevision)
        ));
        let mut config = InteractiveMockConfig::for_family(InteractiveFamily::TauBench);
        config.tool_calls.push(InteractiveToolCall {
            name: "network.fetch".into(),
            permitted: true,
        });
        assert!(matches!(
            prepared.run_local_mock(config),
            Err(InteractiveError::UnsafeToolCall)
        ));
        let mut config = InteractiveMockConfig::for_family(InteractiveFamily::TauBench);
        config.simulated_user = vec![SimulatedUserTurn {
            role: "system".into(),
            content: "bad".into(),
        }];
        assert!(matches!(
            prepared.run_local_mock(config),
            Err(InteractiveError::MalformedSimulatedUser)
        ));
        let mut config = InteractiveMockConfig::for_family(InteractiveFamily::TauBench);
        config.scorer_revision = "upstream".into();
        assert!(matches!(
            prepared.run_local_mock(config),
            Err(InteractiveError::ScorerMismatch)
        ));
        let mut config = InteractiveMockConfig::for_family(InteractiveFamily::TauBench);
        config.state_reset = false;
        assert!(matches!(
            prepared.run_local_mock(config),
            Err(InteractiveError::ResetLeakage)
        ));
        prepared.cleanup().unwrap();
    }
}
