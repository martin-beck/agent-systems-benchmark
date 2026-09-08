// SPDX-License-Identifier: MIT
//! Bounded provenance inventory for supported agent runtime bundles.

use super::{VerifyError, bounded_nonempty, validate_hash};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

const CATALOG_SCHEMA_VERSION: u32 = 1;
const MAX_AGENTS: usize = 32;
const MAX_SOURCE_ARTIFACTS: usize = 256;
const MAX_MISSING_EVIDENCE: usize = 256;

/// Exact supported adapter roster, in canonical bytewise order.
pub const SUPPORTED_AGENT_IDS: [&str; 9] = [
    "aider",
    "codex",
    "gemini",
    "goose",
    "mini-swe",
    "opencode",
    "opendesk",
    "openhands",
    "qwen-code",
];

/// Versioned inventory of every supported agent runtime.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentRuntimeCatalog {
    /// Catalog schema version; currently exactly one.
    pub schema_version: u32,
    /// Complete canonical supported-agent roster.
    pub agents: Vec<AgentRuntimeInventory>,
}

/// One supported agent's pinned source and dependency-closure state.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentRuntimeInventory {
    /// Stable adapter identifier.
    pub agent_id: String,
    /// Exact supported upstream version.
    pub version: String,
    /// Immutable upstream Git revision when one has been established.
    pub source_revision: Option<String>,
    /// SPDX expression declared for the top-level upstream component.
    pub declared_license: String,
    /// Redistribution decision for the complete environment.
    pub redistribution: RedistributionStatus,
    /// Sorted top-level source or release artifacts already pinned by ASB.
    pub source_artifacts: Vec<SourceArtifactPin>,
    /// Whether a complete signed runtime-bundle dependency closure exists.
    pub closure: AgentClosureEvidence,
}

/// One immutable top-level source or release artifact.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SourceArtifactPin {
    /// Stable artifact role, unique within the agent inventory.
    pub name: String,
    /// Lowercase SHA-256 of the exact artifact bytes.
    pub sha256: String,
}

/// Redistribution status for the complete runtime, not only its top-level package.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RedistributionStatus {
    /// Complete license evidence permits redistribution.
    Allowed,
    /// Complete evidence forbids or restricts redistribution.
    Restricted,
    /// Complete transitive license evidence has not yet been established.
    Unverified,
}

/// Evidence that a complete transitive runtime closure does or does not exist.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentClosureEvidence {
    /// A signed runtime manifest covers every runtime component.
    Complete {
        /// SHA-256 of the exact signed runtime manifest bytes.
        manifest_sha256: String,
        /// Number of components in the complete dependency graph.
        component_count: usize,
    },
    /// The runtime is inventory-only and must not be executed as verified.
    Incomplete {
        /// Bounded public descriptions of evidence still required.
        missing_evidence: Vec<String>,
    },
}

/// Bounded structural catalog-validation result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CatalogSummary {
    /// Number of supported agents represented.
    pub agent_count: usize,
    /// Number with complete signed dependency closures.
    pub complete_count: usize,
}

/// Validate exact roster, ordering, pins, bounds, and closure evidence.
pub fn validate_agent_runtime_catalog(
    catalog: &AgentRuntimeCatalog,
) -> Result<CatalogSummary, VerifyError> {
    if catalog.schema_version != CATALOG_SCHEMA_VERSION {
        return Err(VerifyError::Metadata(
            "unsupported agent runtime catalog version".into(),
        ));
    }
    if catalog.agents.len() > MAX_AGENTS
        || catalog.agents.len() != SUPPORTED_AGENT_IDS.len()
        || !catalog
            .agents
            .iter()
            .map(|agent| agent.agent_id.as_str())
            .eq(SUPPORTED_AGENT_IDS)
    {
        return Err(VerifyError::Metadata(
            "agent runtime catalog does not equal canonical supported roster".into(),
        ));
    }

    let mut complete_count = 0;
    for agent in &catalog.agents {
        validate_identifier("agent id", &agent.agent_id)?;
        bounded_nonempty("agent version", &agent.version)?;
        bounded_nonempty("declared license", &agent.declared_license)?;
        if let Some(revision) = &agent.source_revision
            && (revision.len() != 40 || !is_lower_hex(revision))
        {
            return Err(VerifyError::Metadata(
                "source revision is not a lowercase Git object id".into(),
            ));
        }
        if agent.source_artifacts.is_empty() || agent.source_artifacts.len() > MAX_SOURCE_ARTIFACTS
        {
            return Err(VerifyError::Metadata(
                "source artifact inventory is empty or oversized".into(),
            ));
        }
        let mut prior = None;
        let mut names = BTreeSet::new();
        for artifact in &agent.source_artifacts {
            validate_identifier("source artifact name", &artifact.name)?;
            validate_hash(&artifact.sha256)?;
            if prior.is_some_and(|value: &str| value >= artifact.name.as_str())
                || !names.insert(artifact.name.as_str())
            {
                return Err(VerifyError::Metadata(
                    "source artifacts are not strictly sorted and unique".into(),
                ));
            }
            prior = Some(&artifact.name);
        }
        match &agent.closure {
            AgentClosureEvidence::Complete {
                manifest_sha256,
                component_count,
            } => {
                validate_hash(manifest_sha256)?;
                if *component_count == 0 || agent.redistribution == RedistributionStatus::Unverified
                {
                    return Err(VerifyError::Metadata(
                        "complete closure lacks components or redistribution evidence".into(),
                    ));
                }
                complete_count += 1;
            }
            AgentClosureEvidence::Incomplete { missing_evidence } => {
                if missing_evidence.is_empty() || missing_evidence.len() > MAX_MISSING_EVIDENCE {
                    return Err(VerifyError::Metadata(
                        "incomplete closure lacks bounded missing evidence".into(),
                    ));
                }
                for item in missing_evidence {
                    bounded_nonempty("missing evidence", item)?;
                }
            }
        }
    }
    Ok(CatalogSummary {
        agent_count: catalog.agents.len(),
        complete_count,
    })
}

/// Require every supported agent to have a complete signed dependency closure.
pub fn require_complete_agent_catalog(
    catalog: &AgentRuntimeCatalog,
) -> Result<CatalogSummary, VerifyError> {
    let summary = validate_agent_runtime_catalog(catalog)?;
    if summary.complete_count != summary.agent_count {
        return Err(VerifyError::Metadata(
            "agent runtime catalog contains incomplete dependency closures".into(),
        ));
    }
    Ok(summary)
}

fn validate_identifier(name: &str, value: &str) -> Result<(), VerifyError> {
    bounded_nonempty(name, value)?;
    if value
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        Ok(())
    } else {
        Err(VerifyError::Metadata(format!("{name} is not canonical")))
    }
}

fn is_lower_hex(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> AgentRuntimeCatalog {
        serde_json::from_str(include_str!("../fixtures/agents/v1/catalog.json")).unwrap()
    }

    #[test]
    fn released_adapter_roster_is_complete_but_runtime_closures_fail_closed() {
        let catalog = fixture();
        let summary = validate_agent_runtime_catalog(&catalog).unwrap();
        assert_eq!(summary.agent_count, SUPPORTED_AGENT_IDS.len());
        assert_eq!(summary.complete_count, 0);
        assert!(require_complete_agent_catalog(&catalog).is_err());
    }

    #[test]
    fn malformed_or_overstated_catalogs_are_rejected() {
        let mut missing_agent = fixture();
        missing_agent.agents.pop();
        assert!(validate_agent_runtime_catalog(&missing_agent).is_err());

        let mut reordered = fixture();
        reordered.agents.swap(0, 1);
        assert!(validate_agent_runtime_catalog(&reordered).is_err());

        let mut bad_hash = fixture();
        bad_hash.agents[0].source_artifacts[0].sha256 = "A".repeat(64);
        assert!(validate_agent_runtime_catalog(&bad_hash).is_err());

        let mut false_complete = fixture();
        false_complete.agents[0].closure = AgentClosureEvidence::Complete {
            manifest_sha256: "a".repeat(64),
            component_count: 1,
        };
        assert!(validate_agent_runtime_catalog(&false_complete).is_err());

        let mut empty_gap = fixture();
        empty_gap.agents[0].closure = AgentClosureEvidence::Incomplete {
            missing_evidence: Vec::new(),
        };
        assert!(validate_agent_runtime_catalog(&empty_gap).is_err());
    }
}
