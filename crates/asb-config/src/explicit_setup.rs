//! Explicit, portable setup configuration and configuration-bound plans.
//!
//! This contract is deliberately independent of the process, network and
//! filesystem layers.  The CLI owns the explicitly supplied paths; this
//! module only validates bounded data and computes stable bindings.
#![allow(missing_docs)]

use crate::{
    CredentialReference, sha256_hex, validate_list, validate_name, validate_sha256, validate_text,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;
use thiserror::Error;

/// Version of the explicit setup contract.
pub const EXPLICIT_SETUP_SCHEMA_VERSION: u16 = 1;
/// Version of a plan bound to an explicit setup document.
pub const CONFIG_BOUND_PLAN_SCHEMA_VERSION: u16 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SetupAgent {
    pub id: String,
    pub executable: String,
    pub adapter: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SetupProvider {
    pub id: String,
    pub adapter: String,
    pub credential: CredentialReference,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SetupModel {
    pub id: String,
    pub provider: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SetupWorkload {
    pub id: String,
    pub revision: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunBounds {
    pub repetitions: u32,
    pub max_attempts: u32,
    pub timeout_seconds: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionPolicy {
    pub results_root: String,
    pub live: bool,
    pub network: bool,
    pub cost_acknowledged: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RegistrySnapshots {
    pub provider: String,
    pub workload: String,
    pub measurement: String,
}

/// The sole portable setup authority.  It contains references and digests,
/// never credential values or an implicit home/XDG location.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExplicitSetupConfigV1 {
    pub schema_version: u16,
    pub config_id: String,
    pub agents: Vec<SetupAgent>,
    pub providers: Vec<SetupProvider>,
    pub models: Vec<SetupModel>,
    pub workloads: Vec<SetupWorkload>,
    pub monitors: Vec<String>,
    pub measures: Vec<String>,
    pub run: RunBounds,
    pub execution: ExecutionPolicy,
    pub registries: RegistrySnapshots,
}

/// A plan's immutable binding to one explicit configuration.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigBoundPlanV1 {
    pub schema_version: u16,
    pub plan_id: String,
    pub config_id: String,
    pub config_sha256: String,
    pub workloads: Vec<String>,
    pub agents: Vec<String>,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum ExplicitSetupError {
    #[error("invalid explicit setup field: {0}")]
    Invalid(String),
    #[error("configuration and plan do not match")]
    BindingMismatch,
    #[error("configuration encoding failed: {0}")]
    Encoding(String),
}

impl ExplicitSetupConfigV1 {
    pub fn validate(&self) -> Result<(), ExplicitSetupError> {
        if self.schema_version != EXPLICIT_SETUP_SCHEMA_VERSION {
            return Err(ExplicitSetupError::Invalid("schema_version".into()));
        }
        validate_name(&self.config_id, "config_id")
            .map_err(|_| ExplicitSetupError::Invalid("config_id".into()))?;
        if self.agents.is_empty()
            || self.providers.is_empty()
            || self.models.is_empty()
            || self.workloads.is_empty()
        {
            return Err(ExplicitSetupError::Invalid("selection".into()));
        }
        for a in &self.agents {
            validate_name(&a.id, "agent.id")
                .map_err(|_| ExplicitSetupError::Invalid("agent.id".into()))?;
            validate_text(&a.executable, "agent.executable")
                .map_err(|_| ExplicitSetupError::Invalid("agent.executable".into()))?;
            validate_text(&a.adapter, "agent.adapter")
                .map_err(|_| ExplicitSetupError::Invalid("agent.adapter".into()))?;
        }
        if self
            .agents
            .iter()
            .map(|a| &a.id)
            .collect::<BTreeSet<_>>()
            .len()
            != self.agents.len()
            || self
                .providers
                .iter()
                .map(|p| &p.id)
                .collect::<BTreeSet<_>>()
                .len()
                != self.providers.len()
            || self
                .models
                .iter()
                .map(|m| &m.id)
                .collect::<BTreeSet<_>>()
                .len()
                != self.models.len()
            || self
                .workloads
                .iter()
                .map(|w| &w.id)
                .collect::<BTreeSet<_>>()
                .len()
                != self.workloads.len()
        {
            return Err(ExplicitSetupError::Invalid("duplicate selection id".into()));
        }
        for p in &self.providers {
            validate_name(&p.id, "provider.id")
                .map_err(|_| ExplicitSetupError::Invalid("provider.id".into()))?;
            validate_text(&p.adapter, "provider.adapter")
                .map_err(|_| ExplicitSetupError::Invalid("provider.adapter".into()))?;
            p.credential
                .validate()
                .map_err(|_| ExplicitSetupError::Invalid("provider.credential".into()))?;
        }
        for m in &self.models {
            validate_name(&m.id, "model.id")
                .map_err(|_| ExplicitSetupError::Invalid("model.id".into()))?;
            if !self.providers.iter().any(|p| p.id == m.provider) {
                return Err(ExplicitSetupError::Invalid("model.provider".into()));
            }
        }
        for w in &self.workloads {
            validate_name(&w.id, "workload.id")
                .map_err(|_| ExplicitSetupError::Invalid("workload.id".into()))?;
            validate_text(&w.revision, "workload.revision")
                .map_err(|_| ExplicitSetupError::Invalid("workload.revision".into()))?;
        }
        validate_list(&self.monitors, "monitors")
            .map_err(|_| ExplicitSetupError::Invalid("monitors".into()))?;
        validate_list(&self.measures, "measures")
            .map_err(|_| ExplicitSetupError::Invalid("measures".into()))?;
        if self.run.repetitions == 0 || self.run.max_attempts == 0 || self.run.timeout_seconds == 0
        {
            return Err(ExplicitSetupError::Invalid("run bounds".into()));
        }
        validate_text(&self.execution.results_root, "execution.results_root")
            .map_err(|_| ExplicitSetupError::Invalid("execution.results_root".into()))?;
        let root = Path::new(&self.execution.results_root);
        if root.is_absolute()
            || root
                .components()
                .any(|component| component.as_os_str() == "..")
            || self.execution.results_root.contains('\\')
            || self.execution.results_root.starts_with('~')
        {
            return Err(ExplicitSetupError::Invalid("execution.results_root".into()));
        }
        if (self.execution.live || self.execution.network) && !self.execution.cost_acknowledged {
            return Err(ExplicitSetupError::Invalid(
                "execution.cost_acknowledged".into(),
            ));
        }
        for digest in [
            &self.registries.provider,
            &self.registries.workload,
            &self.registries.measurement,
        ] {
            validate_sha256(digest, "registry digest")
                .map_err(|_| ExplicitSetupError::Invalid("registry digest".into()))?;
        }
        Ok(())
    }

    pub fn digest(&self) -> Result<String, ExplicitSetupError> {
        self.validate()?;
        serde_json::to_vec(self)
            .map(|bytes| sha256_hex(&bytes))
            .map_err(|e| ExplicitSetupError::Encoding(e.to_string()))
    }

    pub fn bind_plan(
        &self,
        plan_id: String,
        workloads: Vec<String>,
        agents: Vec<String>,
    ) -> Result<ConfigBoundPlanV1, ExplicitSetupError> {
        self.validate()?;
        validate_name(&plan_id, "plan_id")
            .map_err(|_| ExplicitSetupError::Invalid("plan_id".into()))?;
        if workloads.is_empty()
            || workloads
                .iter()
                .any(|id| !self.workloads.iter().any(|w| &w.id == id))
        {
            return Err(ExplicitSetupError::Invalid("plan workloads".into()));
        }
        if agents.is_empty()
            || agents
                .iter()
                .any(|id| !self.agents.iter().any(|a| &a.id == id))
        {
            return Err(ExplicitSetupError::Invalid("plan agents".into()));
        }
        Ok(ConfigBoundPlanV1 {
            schema_version: CONFIG_BOUND_PLAN_SCHEMA_VERSION,
            plan_id,
            config_id: self.config_id.clone(),
            config_sha256: self.digest()?,
            workloads,
            agents,
        })
    }
}

impl ConfigBoundPlanV1 {
    pub fn validate_against(
        &self,
        config: &ExplicitSetupConfigV1,
    ) -> Result<(), ExplicitSetupError> {
        if self.schema_version != CONFIG_BOUND_PLAN_SCHEMA_VERSION
            || self.config_id != config.config_id
            || self.config_sha256 != config.digest()?
        {
            return Err(ExplicitSetupError::BindingMismatch);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CredentialReferenceKind, sha256_hex};

    fn sample() -> ExplicitSetupConfigV1 {
        ExplicitSetupConfigV1 {
            schema_version: 1,
            config_id: "demo".into(),
            agents: vec![SetupAgent {
                id: "agent".into(),
                executable: "agent-bin".into(),
                adapter: "native".into(),
            }],
            providers: vec![SetupProvider {
                id: "mock".into(),
                adapter: "local-mock".into(),
                credential: crate::CredentialReference {
                    kind: CredentialReferenceKind::Helper,
                    locator_sha256: sha256_hex(b"mock"),
                },
            }],
            models: vec![SetupModel {
                id: "model".into(),
                provider: "mock".into(),
            }],
            workloads: vec![SetupWorkload {
                id: "workload".into(),
                revision: "fixture-v1".into(),
            }],
            monitors: vec!["wall_time".into()],
            measures: vec!["score".into()],
            run: RunBounds {
                repetitions: 1,
                max_attempts: 2,
                timeout_seconds: 30,
            },
            execution: ExecutionPolicy {
                results_root: "results".into(),
                live: false,
                network: false,
                cost_acknowledged: false,
            },
            registries: RegistrySnapshots {
                provider: sha256_hex(b"provider"),
                workload: sha256_hex(b"workload"),
                measurement: sha256_hex(b"measurement"),
            },
        }
    }

    #[test]
    fn explicit_config_round_trips_and_binds_plan() {
        let config = sample();
        let bytes = serde_json::to_vec(&config).expect("encode");
        let decoded: ExplicitSetupConfigV1 = serde_json::from_slice(&bytes).expect("decode");
        assert_eq!(config, decoded);
        let plan = config
            .bind_plan("plan".into(), vec!["workload".into()], vec!["agent".into()])
            .expect("bind");
        assert!(plan.validate_against(&config).is_ok());
    }

    #[test]
    fn binding_rejects_changed_config_and_unknown_fields() {
        let config = sample();
        let plan = config
            .bind_plan("plan".into(), vec!["workload".into()], vec!["agent".into()])
            .expect("bind");
        let mut changed = config.clone();
        changed.measures.push("energy".into());
        assert_eq!(
            plan.validate_against(&changed),
            Err(ExplicitSetupError::BindingMismatch)
        );
        let hostile = br#"{"schema_version":1,"config_id":"demo","agents":[],"providers":[],"models":[],"workloads":[],"monitors":[],"measures":[],"run":{"repetitions":1,"max_attempts":1,"timeout_seconds":1},"execution":{"results_root":"r","live":false,"network":false,"cost_acknowledged":false},"registries":{"provider":"x","workload":"x","measurement":"x"},"secret":"no"}"#;
        assert!(serde_json::from_slice::<ExplicitSetupConfigV1>(hostile).is_err());
    }

    #[test]
    fn live_execution_requires_cost_acknowledgement() {
        let mut config = sample();
        config.execution.live = true;
        assert!(
            matches!(config.validate(), Err(ExplicitSetupError::Invalid(field)) if field == "execution.cost_acknowledged")
        );
    }

    #[test]
    fn absolute_and_parent_result_roots_are_rejected() {
        for root in ["/tmp/results", "../results", "~/.asb/results"] {
            let mut config = sample();
            config.execution.results_root = root.into();
            assert!(
                matches!(config.validate(), Err(ExplicitSetupError::Invalid(field)) if field == "execution.results_root")
            );
        }
    }
}
