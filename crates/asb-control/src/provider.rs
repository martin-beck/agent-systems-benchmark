// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Renderer-neutral provider, authentication, and model catalog types.

use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{ProtocolError, Revision, validate_digest, validate_identity};

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
