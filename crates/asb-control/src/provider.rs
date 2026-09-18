// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Renderer-neutral provider, authentication, and model catalog types.

use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    ProtocolError, Revision, validate_digest, validate_idempotency_key, validate_identity,
};

const MAX_PROVIDERS: usize = 32;
const MAX_MODELS_PER_PROVIDER: usize = 64;
const MAX_AUTH_METHODS: usize = 8;
const MAX_CATALOG_STRING_BYTES: usize = 128;

/// Read-only provider catalog operation requested by a frontend.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderCatalogAction {
    /// Return the current verified snapshot without external effects.
    Status,
    /// Refresh provider discovery and connectivity status.
    Refresh,
}

/// Request for a runner-generation-bound provider catalog operation.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderCatalogRequest {
    /// Operation to perform.
    pub action: ProviderCatalogAction,
    /// Generation identity received during protocol negotiation.
    pub runner_instance_id: String,
    /// Last catalog generation observed by the frontend, if any.
    pub known_generation: Option<Revision>,
}

/// Authentication method supported by a provider profile.
#[derive(
    Clone, Copy, Debug, Deserialize, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ProviderAuthMethod {
    /// Resolve a credential from an environment or OS credential helper.
    CredentialReference,
    /// Use an explicitly provisioned local daemon without a secret in ASB.
    LocalDaemon,
    /// Provider requires no credential.
    None,
}

/// Public availability state for a provider or model.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(
    tag = "status",
    content = "reason",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ProviderAvailability {
    /// The profile can be selected after authentication is configured.
    Available,
    /// The profile is visible for explanation but cannot be selected.
    Unavailable(String),
}

/// One selectable model exposed by a provider.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderModel {
    /// Stable provider-owned model identifier.
    pub model_id: String,
    /// Public model revision or capability identity.
    pub revision: String,
    /// Explicit model availability.
    pub availability: ProviderAvailability,
}

/// One provider profile with its supported models and authentication methods.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderCatalogEntry {
    /// Stable provider profile identifier.
    pub provider_id: String,
    /// Human-readable, non-secret display name.
    pub display_name: String,
    /// Supported authentication methods, in canonical order.
    pub auth_methods: Vec<ProviderAuthMethod>,
    /// Models supported by this profile, in canonical model-id order.
    pub models: Vec<ProviderModel>,
    /// Whether the profile itself may currently be selected.
    pub availability: ProviderAvailability,
}

/// Authenticated provider/model catalog returned by the runner.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderCatalog {
    /// Runner instance/generation identity from negotiation.
    pub runner_instance_id: String,
    /// Monotonic provider-catalog generation.
    pub generation: Revision,
    /// SHA-256 over the canonical provider snapshot.
    pub catalog_sha256: String,
    /// Complete provider profile catalog, including unavailable entries.
    pub providers: Vec<ProviderCatalogEntry>,
    /// True only when this response performed a refresh.
    pub refreshed: bool,
}

/// Read-only request for the runner's current setup state.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigurationStatusRequest {
    /// Runner identity received during protocol negotiation.
    pub runner_instance_id: String,
}

/// Privacy-safe setup projection consumed by independent frontends.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigurationSnapshot {
    /// Runner instance identity.
    pub runner_instance_id: String,
    /// Monotonic setup generation used for later mutation fencing.
    pub generation: Revision,
    /// Whether a complete selectable configuration is present.
    pub configured: bool,
    /// Selected agent IDs, sorted and without executable paths.
    pub agent_ids: Vec<String>,
    /// Selected provider profile, if configured.
    pub provider_id: Option<String>,
    /// Selected provider model, if configured.
    pub model_id: Option<String>,
    /// Selected authentication method, if configured.
    pub auth_method: Option<ProviderAuthMethod>,
    /// Digest of a credential reference, never the credential itself.
    pub credential_reference_sha256: Option<String>,
}

/// Idempotent setup mutation submitted by an independent frontend.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigurationApplyParams {
    /// Retry-safe mutation identity.
    pub idempotency_key: String,
    /// Generation returned by the last status read.
    pub expected_generation: Revision,
    /// Complete replacement selection; partial updates are not accepted.
    pub selection: ConfigurationSelection,
}

/// Provider/model/auth selection stored by the runner without secret values.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigurationSelection {
    /// Selected agents, sorted by canonical ID.
    pub agent_ids: Vec<String>,
    /// Selected provider profile.
    pub provider_id: String,
    /// Selected model from the provider catalog.
    pub model_id: String,
    /// Authentication route selected by the user.
    pub auth_method: ProviderAuthMethod,
    /// Digest of an external credential reference, never its value.
    pub credential_reference_sha256: Option<String>,
}

/// Read-only bounded recording-campaign estimate request.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecordingCampaignEstimateRequest {
    /// Runner identity received during negotiation.
    pub runner_instance_id: String,
    /// Provider profile selected for the campaign.
    pub provider_id: String,
    /// Model selected for the campaign.
    pub model_id: String,
    /// Selected agent IDs.
    pub agent_ids: Vec<String>,
    /// Selected workload IDs.
    pub workload_ids: Vec<String>,
}

/// Explicit estimate and offline-readiness projection for recording campaigns.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecordingCampaignEstimate {
    /// Runner identity.
    pub runner_instance_id: String,
    /// Configuration generation used for this estimate.
    pub generation: Revision,
    /// Number of agent x workload tuples requested.
    pub tuple_count: u16,
    /// Whether every requested tuple has current complete coverage.
    pub complete_coverage: bool,
    /// Whether the selected configuration may run offline immediately.
    pub offline_ready: bool,
    /// Stable explanation when recording or offline replay is unavailable.
    pub unavailable_reason: Option<String>,
}

/// Idempotent request to persist an exact recording matrix for later execution.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecordingCampaignPlanParams {
    /// Retry-safe mutation identity.
    pub idempotency_key: String,
    /// Setup generation returned by the last configuration status read.
    pub expected_generation: Revision,
    /// Runner identity received during negotiation.
    pub runner_instance_id: String,
    /// Provider profile selected for the campaign.
    pub provider_id: String,
    /// Model selected for the campaign.
    pub model_id: String,
    /// Selected agent IDs.
    pub agent_ids: Vec<String>,
    /// Selected workload IDs.
    pub workload_ids: Vec<String>,
}

/// Durable, privacy-safe recording campaign plan.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecordingCampaignPlan {
    /// Runner identity.
    pub runner_instance_id: String,
    /// Setup generation used by this plan.
    pub generation: Revision,
    /// Stable content-derived campaign identity.
    pub campaign_id: String,
    /// Provider profile selected for the campaign.
    pub provider_id: String,
    /// Model selected for the campaign.
    pub model_id: String,
    /// Selected agents, in canonical order.
    pub agent_ids: Vec<String>,
    /// Selected workloads, in canonical order.
    pub workload_ids: Vec<String>,
    /// Number of exact agent/workload tuples.
    pub tuple_count: u16,
    /// Current lifecycle state; a new plan is always `planned`.
    pub state: String,
    /// A plan never claims offline readiness before coverage reconciliation.
    pub offline_ready: bool,
    /// Stable explanation for the current state.
    pub unavailable_reason: Option<String>,
}

/// Read-only request for the last durable recording campaign plan.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecordingCampaignStatusRequest {
    /// Runner identity received during negotiation.
    pub runner_instance_id: String,
}

/// Restart-safe campaign status projection.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecordingCampaignStatus {
    /// Runner identity.
    pub runner_instance_id: String,
    /// Current setup generation.
    pub generation: Revision,
    /// Last durable plan, if one has been created.
    pub campaign: Option<RecordingCampaignPlan>,
}

/// Idempotent request to admit a planned campaign for runtime-owned capture.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecordingCampaignExecuteParams {
    /// Retry-safe mutation identity.
    pub idempotency_key: String,
    /// Setup generation returned by the last configuration status read.
    pub expected_generation: Revision,
    /// Runner identity received during negotiation.
    pub runner_instance_id: String,
    /// Exact durable campaign to execute.
    pub campaign_id: String,
}

/// Read-only request for durable recording progress.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecordingCampaignProgressRequest {
    /// Runner identity received during negotiation.
    pub runner_instance_id: String,
    /// Exact campaign to inspect.
    pub campaign_id: String,
}

/// Idempotent cancellation of a recording campaign.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecordingCampaignCancelParams {
    /// Retry-safe mutation identity.
    pub idempotency_key: String,
    /// Setup generation returned by the last configuration status read.
    pub expected_generation: Revision,
    /// Runner identity received during negotiation.
    pub runner_instance_id: String,
    /// Exact durable campaign to cancel.
    pub campaign_id: String,
}

/// Idempotent reconciliation of an interrupted recording campaign.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecordingCampaignReconcileParams {
    /// Retry-safe mutation identity.
    pub idempotency_key: String,
    /// Setup generation returned by the last configuration status read.
    pub expected_generation: Revision,
    /// Runner identity received during negotiation.
    pub runner_instance_id: String,
    /// Exact durable campaign to reconcile.
    pub campaign_id: String,
}

/// Idempotent activation of a fully covered campaign as the offline default.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecordingCampaignOfflineDefaultParams {
    /// Retry-safe mutation identity.
    pub idempotency_key: String,
    /// Setup generation returned by the last configuration status read.
    pub expected_generation: Revision,
    /// Runner identity received during negotiation.
    pub runner_instance_id: String,
    /// Exact durable campaign to activate.
    pub campaign_id: String,
}

/// Durable recording lifecycle and tuple coverage projection.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecordingCampaignLifecycle {
    /// Runner identity.
    pub runner_instance_id: String,
    /// Setup generation used by this campaign.
    pub generation: Revision,
    /// Stable campaign identity.
    pub campaign_id: String,
    /// Provider profile selected for the campaign.
    pub provider_id: String,
    /// Model selected for the campaign.
    pub model_id: String,
    /// Selected agents, in canonical order.
    pub agent_ids: Vec<String>,
    /// Selected workloads, in canonical order.
    pub workload_ids: Vec<String>,
    /// Number of exact agent/workload tuples.
    pub tuple_count: u16,
    /// Number of tuples with verified durable cassette coverage.
    pub covered_tuple_count: u16,
    /// Durable lifecycle state.
    pub state: String,
    /// True only after complete coverage has been reconciled.
    pub offline_ready: bool,
    /// Stable explanation when not ready.
    pub unavailable_reason: Option<String>,
}

impl RecordingCampaignExecuteParams {
    /// Validate bounded execute identity.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        validate_campaign_mutation(
            &self.idempotency_key,
            self.expected_generation,
            &self.runner_instance_id,
            &self.campaign_id,
        )
    }
}
impl RecordingCampaignProgressRequest {
    /// Validate bounded progress identity.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        validate_campaign_read(&self.runner_instance_id, &self.campaign_id)
    }
}
impl RecordingCampaignCancelParams {
    /// Validate bounded cancel identity.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        validate_campaign_mutation(
            &self.idempotency_key,
            self.expected_generation,
            &self.runner_instance_id,
            &self.campaign_id,
        )
    }
}
impl RecordingCampaignReconcileParams {
    /// Validate bounded reconcile identity.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        validate_campaign_mutation(
            &self.idempotency_key,
            self.expected_generation,
            &self.runner_instance_id,
            &self.campaign_id,
        )
    }
}
impl RecordingCampaignOfflineDefaultParams {
    /// Validate bounded offline-default identity.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        validate_campaign_mutation(
            &self.idempotency_key,
            self.expected_generation,
            &self.runner_instance_id,
            &self.campaign_id,
        )
    }
}

fn validate_campaign_read(
    runner_instance_id: &str,
    campaign_id: &str,
) -> Result<(), ProtocolError> {
    validate_identity(runner_instance_id)?;
    validate_identity(campaign_id)
}

fn validate_campaign_mutation(
    idempotency_key: &str,
    expected_generation: Revision,
    runner_instance_id: &str,
    campaign_id: &str,
) -> Result<(), ProtocolError> {
    validate_idempotency_key(idempotency_key)?;
    if expected_generation.0 == 0 {
        return Err(ProtocolError::InvalidResponse);
    }
    validate_campaign_read(runner_instance_id, campaign_id)
}

impl RecordingCampaignLifecycle {
    /// Validate explicit lifecycle and coverage invariants.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        validate_identity(&self.runner_instance_id)?;
        validate_identity(&self.campaign_id)?;
        validate_catalog_string(&self.provider_id)?;
        validate_catalog_string(&self.model_id)?;
        validate_sorted_ids(&self.agent_ids, false)?;
        validate_sorted_ids(&self.workload_ids, false)?;
        let tuples = self
            .agent_ids
            .len()
            .checked_mul(self.workload_ids.len())
            .and_then(|v| u16::try_from(v).ok())
            .ok_or(ProtocolError::InvalidResponse)?;
        if self.generation.0 == 0
            || self.tuple_count != tuples
            || self.covered_tuple_count > self.tuple_count
        {
            return Err(ProtocolError::InvalidResponse);
        }
        if !matches!(
            self.state.as_str(),
            "planned" | "recording" | "needs_reconciliation" | "complete" | "cancelled" | "failed"
        ) {
            return Err(ProtocolError::InvalidResponse);
        }
        if self.offline_ready
            != (self.state == "complete" && self.covered_tuple_count == self.tuple_count)
        {
            return Err(ProtocolError::InvalidResponse);
        }
        if self.offline_ready != self.unavailable_reason.is_none() {
            return Err(ProtocolError::InvalidResponse);
        }
        if !self.offline_ready && self.unavailable_reason.is_none() {
            return Err(ProtocolError::InvalidResponse);
        }
        Ok(())
    }
}

impl ConfigurationStatusRequest {
    /// Validate request identity.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        validate_identity(&self.runner_instance_id)
    }
}

impl RecordingCampaignEstimateRequest {
    /// Validate bounded, canonical request fields without provider contact.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        validate_identity(&self.runner_instance_id)?;
        validate_catalog_string(&self.provider_id)?;
        validate_catalog_string(&self.model_id)?;
        validate_sorted_ids(&self.agent_ids, false)?;
        validate_sorted_ids(&self.workload_ids, false)?;
        Ok(())
    }
}

impl RecordingCampaignPlanParams {
    /// Validate bounded, canonical campaign selection without provider effects.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        validate_idempotency_key(&self.idempotency_key)?;
        validate_identity(&self.runner_instance_id)?;
        if self.expected_generation.0 == 0 {
            return Err(ProtocolError::InvalidResponse);
        }
        validate_catalog_string(&self.provider_id)?;
        validate_catalog_string(&self.model_id)?;
        validate_sorted_ids(&self.agent_ids, false)?;
        validate_sorted_ids(&self.workload_ids, false)?;
        let tuples = self
            .agent_ids
            .len()
            .checked_mul(self.workload_ids.len())
            .ok_or(ProtocolError::InvalidResponse)?;
        if tuples == 0 || tuples > 256 {
            return Err(ProtocolError::InvalidResponse);
        }
        Ok(())
    }
}

impl RecordingCampaignPlan {
    /// Validate explicit planned-state semantics and bounded public fields.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        validate_identity(&self.runner_instance_id)?;
        validate_identity(&self.campaign_id)?;
        validate_catalog_string(&self.provider_id)?;
        validate_catalog_string(&self.model_id)?;
        validate_sorted_ids(&self.agent_ids, false)?;
        validate_sorted_ids(&self.workload_ids, false)?;
        let tuples = self
            .agent_ids
            .len()
            .checked_mul(self.workload_ids.len())
            .and_then(|value| u16::try_from(value).ok())
            .ok_or(ProtocolError::InvalidResponse)?;
        if self.generation.0 == 0
            || self.tuple_count != tuples
            || self.state != "planned"
            || self.offline_ready
            || self.unavailable_reason.as_deref() != Some("recording-required")
        {
            return Err(ProtocolError::InvalidResponse);
        }
        Ok(())
    }
}

impl RecordingCampaignStatusRequest {
    /// Validate request identity.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        validate_identity(&self.runner_instance_id)
    }
}

impl RecordingCampaignStatus {
    /// Validate the absence/presence invariants without inferring readiness.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        validate_identity(&self.runner_instance_id)?;
        if self.generation.0 == 0 {
            return Err(ProtocolError::InvalidResponse);
        }
        if let Some(campaign) = &self.campaign {
            campaign.validate()?;
            if campaign.runner_instance_id != self.runner_instance_id
                || campaign.generation != self.generation
            {
                return Err(ProtocolError::InvalidResponse);
            }
        }
        Ok(())
    }
}

impl ConfigurationApplyParams {
    /// Validate bounded replacement selection and credential-reference privacy.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        validate_idempotency_key(&self.idempotency_key)?;
        if self.expected_generation.0 == 0 {
            return Err(ProtocolError::InvalidResponse);
        }
        let selection = &self.selection;
        validate_catalog_string(&selection.provider_id)?;
        validate_catalog_string(&selection.model_id)?;
        if selection.agent_ids.is_empty() || selection.agent_ids.len() > MAX_PROVIDERS {
            return Err(ProtocolError::InvalidResponse);
        }
        let mut agents = BTreeSet::new();
        let mut previous = None;
        for agent in &selection.agent_ids {
            validate_catalog_string(agent)?;
            if previous.is_some_and(|value: &str| value >= agent.as_str())
                || !agents.insert(agent.as_str())
            {
                return Err(ProtocolError::InvalidResponse);
            }
            previous = Some(agent.as_str());
        }
        match (
            &selection.auth_method,
            &selection.credential_reference_sha256,
        ) {
            (ProviderAuthMethod::CredentialReference, Some(digest)) => validate_digest(digest)?,
            (ProviderAuthMethod::CredentialReference, None) => {
                return Err(ProtocolError::InvalidResponse);
            }
            (_, Some(_)) => return Err(ProtocolError::InvalidResponse),
            (_, None) => {}
        }
        Ok(())
    }
}

impl ConfigurationSnapshot {
    /// Validate explicit configured/unconfigured invariants and privacy bounds.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        validate_identity(&self.runner_instance_id)?;
        if self.generation.0 == 0 || self.agent_ids.len() > MAX_PROVIDERS {
            return Err(ProtocolError::InvalidResponse);
        }
        let mut agents = BTreeSet::new();
        let mut previous = None;
        for agent in &self.agent_ids {
            validate_catalog_string(agent)?;
            if previous.is_some_and(|value: &str| value >= agent.as_str())
                || !agents.insert(agent.as_str())
            {
                return Err(ProtocolError::InvalidResponse);
            }
            previous = Some(agent.as_str());
        }
        for value in [&self.provider_id, &self.model_id].into_iter().flatten() {
            validate_catalog_string(value)?;
        }
        if let Some(digest) = &self.credential_reference_sha256 {
            validate_digest(digest)?;
        }
        let complete = self.configured
            && !self.agent_ids.is_empty()
            && self.provider_id.is_some()
            && self.model_id.is_some()
            && self.auth_method.is_some();
        if complete != self.configured {
            return Err(ProtocolError::InvalidResponse);
        }
        if !self.configured
            && (self.credential_reference_sha256.is_some()
                || self.provider_id.is_some()
                || self.model_id.is_some()
                || self.auth_method.is_some())
        {
            return Err(ProtocolError::InvalidResponse);
        }
        Ok(())
    }
}

impl RecordingCampaignEstimate {
    /// Validate explicit incomplete/ready semantics and bounded public fields.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        validate_identity(&self.runner_instance_id)?;
        if self.generation.0 == 0 || self.tuple_count == 0 {
            return Err(ProtocolError::InvalidResponse);
        }
        if self.offline_ready && (!self.complete_coverage || self.unavailable_reason.is_some()) {
            return Err(ProtocolError::InvalidResponse);
        }
        if !self.offline_ready && self.unavailable_reason.is_none() {
            return Err(ProtocolError::InvalidResponse);
        }
        if let Some(reason) = &self.unavailable_reason {
            validate_catalog_string(reason)?;
        }
        Ok(())
    }
}

fn validate_sorted_ids(values: &[String], allow_empty: bool) -> Result<(), ProtocolError> {
    if (!allow_empty && values.is_empty()) || values.len() > MAX_PROVIDERS {
        return Err(ProtocolError::InvalidResponse);
    }
    let mut ids = BTreeSet::new();
    let mut previous = None;
    for value in values {
        validate_catalog_string(value)?;
        if previous.is_some_and(|item: &str| item >= value.as_str()) || !ids.insert(value.as_str())
        {
            return Err(ProtocolError::InvalidResponse);
        }
        previous = Some(value.as_str());
    }
    Ok(())
}

impl ProviderCatalogRequest {
    /// Validate request shape and generation binding supplied by the client.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        validate_identity(&self.runner_instance_id)
    }
}

impl ProviderCatalog {
    /// Validate bounds, canonical ordering, and the authenticated digest.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        validate_identity(&self.runner_instance_id)?;
        validate_digest(&self.catalog_sha256)?;
        if self.providers.is_empty() || self.providers.len() > MAX_PROVIDERS {
            return Err(ProtocolError::InvalidResponse);
        }
        let mut provider_ids = BTreeSet::new();
        let mut previous = None;
        for provider in &self.providers {
            validate_catalog_string(&provider.provider_id)?;
            validate_catalog_string(&provider.display_name)?;
            if previous.is_some_and(|id: &str| id >= provider.provider_id.as_str())
                || !provider_ids.insert(provider.provider_id.as_str())
                || provider.auth_methods.is_empty()
                || provider.auth_methods.len() > MAX_AUTH_METHODS
                || provider.models.is_empty()
                || provider.models.len() > MAX_MODELS_PER_PROVIDER
            {
                return Err(ProtocolError::InvalidResponse);
            }
            previous = Some(provider.provider_id.as_str());
            let mut methods = BTreeSet::new();
            for method in &provider.auth_methods {
                if !methods.insert(method) {
                    return Err(ProtocolError::InvalidResponse);
                }
            }
            let mut models = BTreeSet::new();
            let mut prior_model = None;
            for model in &provider.models {
                validate_catalog_string(&model.model_id)?;
                validate_catalog_string(&model.revision)?;
                if prior_model.is_some_and(|id: &str| id >= model.model_id.as_str())
                    || !models.insert(model.model_id.as_str())
                {
                    return Err(ProtocolError::InvalidResponse);
                }
                prior_model = Some(model.model_id.as_str());
            }
        }
        if self.catalog_sha256 != self.computed_sha256()? {
            return Err(ProtocolError::InvalidResponse);
        }
        Ok(())
    }

    /// Compute the catalog digest with `catalog_sha256` omitted.
    pub fn computed_sha256(&self) -> Result<String, ProtocolError> {
        let mut value = serde_json::to_value(self).map_err(|_| ProtocolError::InvalidResponse)?;
        let serde_json::Value::Object(object) = &mut value else {
            return Err(ProtocolError::InvalidResponse);
        };
        object.remove("catalog_sha256");
        let canonical = canonical_json(value);
        let bytes = serde_json::to_vec(&canonical).map_err(|_| ProtocolError::InvalidResponse)?;
        let digest = Sha256::digest(bytes);
        Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
    }
}

fn validate_catalog_string(value: &str) -> Result<(), ProtocolError> {
    if value.is_empty()
        || value.len() > MAX_CATALOG_STRING_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:-/".contains(&byte))
    {
        return Err(ProtocolError::UnsafePublicValue);
    }
    Ok(())
}

fn canonical_json(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(object) => {
            let sorted: BTreeMap<String, serde_json::Value> = object
                .into_iter()
                .map(|(key, value)| (key, canonical_json(value)))
                .collect();
            serde_json::Value::Object(sorted.into_iter().collect())
        }
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.into_iter().map(canonical_json).collect())
        }
        other => other,
    }
}
