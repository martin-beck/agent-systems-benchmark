// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Versioned, credential-free ASB configuration and atomic persistence.
//!
//! This crate owns durable configuration only.  It contains no terminal, TUI,
//! renderer, provider client, or process-launching code.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use sha2::Digest;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Current on-disk configuration schema.
pub const CONFIG_SCHEMA_VERSION: u16 = 1;
/// Maximum serialized configuration size.
pub const MAX_CONFIG_BYTES: usize = 256 * 1024;
const MAX_NAME_BYTES: usize = 128;
const MAX_VALUE_BYTES: usize = 4096;

/// Durable, credential-free provider enrollment metadata.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthEnrollment {
    /// Registry schema version.
    pub schema_version: u16,
    /// Provider family identifier.
    pub provider: String,
    /// Credential resolver reference; never secret bytes.
    pub credential: CredentialReference,
    /// Exact endpoint identity digest.
    pub endpoint_identity_sha256: String,
    /// Monotonic enrollment generation.
    pub generation: u64,
    /// Public lifecycle status.
    pub status: AuthEnrollmentStatus,
}

/// Public enrollment lifecycle status.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthEnrollmentStatus {
    /// Enrollment is usable.
    Active,
    /// Enrollment has been revoked.
    Revoked,
    /// Enrollment awaits activation.
    Pending,
}

/// A non-secret reference to credentials held by an external resolver.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CredentialReference {
    /// Stable resolver kind, never credential bytes.
    pub kind: CredentialReferenceKind,
    /// SHA-256 of the opaque locator metadata.
    pub locator_sha256: String,
}

/// Supported credential locator kinds.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialReferenceKind {
    /// An explicitly named environment variable.
    Environment,
    /// An already-open descriptor supplied by the caller.
    FileDescriptor,
    /// A pinned helper descriptor.
    Helper,
}

impl Serialize for CredentialReference {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        #[derive(Serialize)]
        struct Wire<'a> {
            kind: CredentialReferenceKind,
            locator_sha256: &'a str,
        }
        Wire {
            kind: self.kind,
            locator_sha256: &self.locator_sha256,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for CredentialReference {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            kind: CredentialReferenceKind,
            locator_sha256: String,
        }
        let Wire {
            kind,
            locator_sha256,
        } = Wire::deserialize(deserializer)?;
        let value = Self {
            kind,
            locator_sha256,
        };
        value.validate().map_err(serde::de::Error::custom)?;
        Ok(value)
    }
}

impl CredentialReference {
    fn validate(&self) -> Result<(), ConfigError> {
        validate_sha256(&self.locator_sha256, "credential locator")
    }
}

/// A credential-free model/provider profile.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelProfile {
    /// Public provider family (for example `openai` or `ollama`).
    pub provider: String,
    /// Public model identifier.
    pub model: String,
    /// Public endpoint; credentials must not be embedded in it.
    pub endpoint: String,
    /// External credential locator, if the provider requires one.
    pub credential: Option<CredentialReference>,
}

/// Connection settings shared by one or more agents.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Connection {
    /// Public connection endpoint label.
    pub endpoint: String,
    /// Maximum in-flight requests.
    pub max_in_flight: u32,
}

/// Agent declaration referencing a model profile and connection.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Agent {
    /// Model profile name.
    pub profile: String,
    /// Connection name.
    pub connection: String,
    /// Whether this agent is enabled by default.
    pub enabled: bool,
}

/// Reusable benchmark defaults.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Defaults {
    /// Default selected agent names.
    pub agents: Vec<String>,
    /// Default measurement names.
    pub measures: Vec<String>,
    /// Default repetition count.
    pub repetitions: u32,
}

/// A partial override applied to an individual agent.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentOverride {
    /// Optional profile replacement.
    pub profile: Option<String>,
    /// Optional connection replacement.
    pub connection: Option<String>,
    /// Optional enabled replacement.
    pub enabled: Option<bool>,
    /// Optional per-agent measurement defaults.
    pub measures: Option<Vec<String>>,
    /// Optional per-agent repetition default.
    pub repetitions: Option<u32>,
}

/// A run-only override, never persisted by [`ConfigStore::save`].
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunOverride {
    /// Optional selected agents for this run.
    pub agents: Option<Vec<String>>,
    /// Optional measurements for this run.
    pub measures: Option<Vec<String>>,
    /// Optional repetition count for this run.
    pub repetitions: Option<u32>,
}

/// Complete authoritative configuration document.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    /// On-disk schema version.
    pub schema_version: u16,
    /// Declared agents.
    pub agents: BTreeMap<String, Agent>,
    /// Named connections.
    pub connections: BTreeMap<String, Connection>,
    /// Named credential-free model profiles.
    pub profiles: BTreeMap<String, ModelProfile>,
    /// Shared benchmark defaults.
    pub defaults: Defaults,
    /// Per-agent overrides.
    pub agent_overrides: BTreeMap<String, AgentOverride>,
}

/// Bounded, credential-free provider/model discovery record.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderRegistryV1 {
    /// Registry contract version.
    pub schema_version: u16,
    /// Managed named connections.
    pub connections: BTreeMap<String, RegistryConnectionV1>,
    /// Models qualified for each connection.
    pub models: BTreeMap<String, Vec<RegistryModelV1>>,
}

/// Public connection identity; endpoint values are never persisted.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryConnectionV1 {
    /// Provider family identifier.
    pub provider: String,
    /// Protocol identifier.
    pub protocol: String,
    /// Endpoint identity digest.
    pub endpoint_identity_sha256: String,
    /// Optional credential locator digest.
    pub credential_locator_sha256: Option<String>,
}

/// Model discovered and qualified for one managed connection.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryModelV1 {
    /// Provider model identifier.
    pub id: String,
    /// Qualification evidence digest.
    pub qualification_sha256: String,
    /// Connection generation used for discovery.
    pub discovered_at_generation: u64,
}

impl ProviderRegistryV1 {
    /// Parse a bounded provider model response into qualified cache entries.
    pub fn parse_model_catalog(
        &mut self,
        connection: &str,
        generation: u64,
        bytes: &[u8],
    ) -> Result<(), ConfigError> {
        if bytes.is_empty() || bytes.len() > 64 * 1024 {
            return Err(ConfigError::InvalidValue("model catalog size".into()));
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Catalog {
            models: Vec<Model>,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Model {
            id: String,
        }
        let catalog: Catalog = serde_json::from_slice(bytes)
            .map_err(|_| ConfigError::InvalidValue("model catalog format".into()))?;
        if catalog.models.len() > 256 {
            return Err(ConfigError::InvalidValue("model catalog count".into()));
        }
        let models = catalog
            .models
            .into_iter()
            .map(|model| RegistryModelV1 {
                qualification_sha256: format!("{:x}", sha2::Sha256::digest(model.id.as_bytes())),
                id: model.id,
                discovered_at_generation: generation,
            })
            .collect();
        self.replace_models(connection, generation, models)
    }

    /// Parse an OpenAI-compatible catalog (`data[].id`).
    pub fn parse_openai_model_catalog(
        &mut self,
        connection: &str,
        generation: u64,
        bytes: &[u8],
    ) -> Result<(), ConfigError> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Catalog {
            data: Vec<Model>,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Model {
            id: String,
        }
        let catalog: Catalog = bounded_json(bytes, "OpenAI model catalog")?;
        self.replace_discovered_ids(
            connection,
            generation,
            catalog.data.into_iter().map(|model| model.id),
        )
    }

    /// Parse a Gemini catalog (`models[].name`), stripping its `models/` prefix.
    pub fn parse_gemini_model_catalog(
        &mut self,
        connection: &str,
        generation: u64,
        bytes: &[u8],
    ) -> Result<(), ConfigError> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Catalog {
            models: Vec<Model>,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Model {
            name: String,
        }
        let catalog: Catalog = bounded_json(bytes, "Gemini model catalog")?;
        self.replace_discovered_ids(
            connection,
            generation,
            catalog.models.into_iter().map(|model| {
                model
                    .name
                    .strip_prefix("models/")
                    .unwrap_or(&model.name)
                    .to_owned()
            }),
        )
    }

    /// Parse an Ollama catalog (`models[].name`).
    pub fn parse_ollama_model_catalog(
        &mut self,
        connection: &str,
        generation: u64,
        bytes: &[u8],
    ) -> Result<(), ConfigError> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Catalog {
            models: Vec<Model>,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Model {
            name: String,
        }
        let catalog: Catalog = bounded_json(bytes, "Ollama model catalog")?;
        self.replace_discovered_ids(
            connection,
            generation,
            catalog.models.into_iter().map(|model| model.name),
        )
    }

    fn replace_discovered_ids(
        &mut self,
        connection: &str,
        generation: u64,
        ids: impl IntoIterator<Item = String>,
    ) -> Result<(), ConfigError> {
        let models = ids
            .into_iter()
            .map(|id| RegistryModelV1 {
                qualification_sha256: format!("{:x}", sha2::Sha256::digest(id.as_bytes())),
                id,
                discovered_at_generation: generation,
            })
            .collect::<Vec<_>>();
        self.replace_models(connection, generation, models)
    }
    /// Add a managed connection, rejecting replacement of an existing identity.
    pub fn add_connection(
        &mut self,
        name: String,
        connection: RegistryConnectionV1,
    ) -> Result<(), ConfigError> {
        if name.is_empty() || self.connections.contains_key(&name) {
            return Err(ConfigError::InvalidValue("duplicate connection".into()));
        }
        self.connections.insert(name, connection);
        self.validate()
    }

    /// Remove a managed connection and its cached models atomically.
    pub fn remove_connection(&mut self, name: &str) -> Result<(), ConfigError> {
        if self.connections.remove(name).is_none() {
            return Err(ConfigError::InvalidValue("unknown connection".into()));
        }
        self.models.remove(name);
        self.validate()
    }

    /// Validate bounded names, digests and generation fencing.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.schema_version != 1 || self.connections.len() > 128 || self.models.len() > 128 {
            return Err(ConfigError::InvalidValue("provider registry".into()));
        }
        for (name, connection) in &self.connections {
            validate_text(name, "connection name")?;
            validate_text(&connection.provider, "provider")?;
            validate_text(&connection.protocol, "protocol")?;
            if !matches!(
                connection.protocol.as_str(),
                "openai_chat" | "gemini" | "ollama" | "openai_compatible"
            ) {
                return Err(ConfigError::InvalidValue(
                    "unsupported provider protocol".into(),
                ));
            }
            validate_sha256(&connection.endpoint_identity_sha256, "endpoint identity")?;
            if let Some(digest) = &connection.credential_locator_sha256 {
                validate_sha256(digest, "credential locator")?;
            }
        }
        for (name, models) in &self.models {
            if !self.connections.contains_key(name) || models.len() > 256 {
                return Err(ConfigError::InvalidValue("model registry".into()));
            }
            for model in models {
                validate_text(&model.id, "model id")?;
                validate_sha256(&model.qualification_sha256, "qualification")?;
                if model.discovered_at_generation == 0 {
                    return Err(ConfigError::InvalidValue("model generation".into()));
                }
            }
        }
        Ok(())
    }

    /// Replace a connection's bounded, already-qualified model cache.
    pub fn replace_models(
        &mut self,
        connection: &str,
        generation: u64,
        models: Vec<RegistryModelV1>,
    ) -> Result<(), ConfigError> {
        if generation == 0 || models.len() > 256 || !self.connections.contains_key(connection) {
            return Err(ConfigError::InvalidValue("model discovery".into()));
        }
        if models
            .iter()
            .any(|model| model.discovered_at_generation != generation)
        {
            return Err(ConfigError::InvalidValue("stale model discovery".into()));
        }
        let mut ids = BTreeSet::new();
        if models.iter().any(|model| !ids.insert(&model.id)) {
            return Err(ConfigError::InvalidValue(
                "duplicate discovered model".into(),
            ));
        }
        self.models.insert(connection.to_owned(), models);
        self.validate()
    }

    /// Return only models qualified for the requested provider protocol.
    pub fn compatible_models(
        &self,
        connection: &str,
        provider: &str,
        protocol: &str,
    ) -> Vec<&RegistryModelV1> {
        let Some(identity) = self.connections.get(connection) else {
            return Vec::new();
        };
        if identity.provider != provider || identity.protocol != protocol {
            return Vec::new();
        }
        self.models
            .get(connection)
            .map_or_else(Vec::new, |models| models.iter().collect())
    }
}

fn bounded_json<T: for<'de> Deserialize<'de>>(bytes: &[u8], label: &str) -> Result<T, ConfigError> {
    if bytes.is_empty() || bytes.len() > 64 * 1024 {
        return Err(ConfigError::InvalidValue(format!("{label} size")));
    }
    serde_json::from_slice(bytes).map_err(|_| ConfigError::InvalidValue(format!("{label} format")))
}

/// Built-in values used when no persisted value exists.
pub fn built_in_defaults() -> Defaults {
    Defaults {
        agents: Vec::new(),
        measures: Vec::new(),
        repetitions: 1,
    }
}

/// Where one effective value originated.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValueOrigin {
    /// Built into ASB.
    BuiltIn,
    /// Shared persisted default.
    Global,
    /// Per-agent persisted override.
    Agent,
    /// Run-only caller override.
    Run,
}

/// One resolved value plus its precedence origin.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Effective<T> {
    /// Effective value.
    pub value: T,
    /// Winning precedence layer.
    pub origin: ValueOrigin,
}

/// Resolved benchmark defaults.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectiveDefaults {
    /// Effective selected agents.
    pub agents: Effective<Vec<String>>,
    /// Effective measurements.
    pub measures: Effective<Vec<String>>,
    /// Effective repetition count.
    pub repetitions: Effective<u32>,
}

/// Apply one existing profile to every requested agent as one validated change.
///
/// The returned document is a new value; callers persist it only after the
/// complete compatibility/reference check succeeds. No partial document is
/// ever returned for an invalid target set.
pub fn apply_profile_to_agents(
    config: &Configuration,
    profile: &str,
    targets: &[String],
) -> Result<Configuration, ConfigError> {
    config.validate()?;
    if !config.profiles.contains_key(profile) {
        return Err(ConfigError::UnknownReference(profile.to_owned()));
    }
    validate_list(targets, "profile targets")?;
    if targets.is_empty() {
        return Err(ConfigError::InvalidValue("profile targets".into()));
    }
    if targets
        .iter()
        .any(|target| !config.agents.contains_key(target))
    {
        return Err(ConfigError::UnknownReference(
            targets
                .iter()
                .find(|target| !config.agents.contains_key(*target))
                .cloned()
                .unwrap_or_default(),
        ));
    }
    let mut updated = config.clone();
    for target in targets {
        updated
            .agents
            .get_mut(target)
            .expect("targets were validated")
            .profile = profile.to_owned();
    }
    updated.validate()?;
    Ok(updated)
}

impl Configuration {
    /// Construct a validated empty configuration with built-in defaults.
    pub fn empty() -> Self {
        Self {
            schema_version: CONFIG_SCHEMA_VERSION,
            agents: BTreeMap::new(),
            connections: BTreeMap::new(),
            profiles: BTreeMap::new(),
            defaults: built_in_defaults(),
            agent_overrides: BTreeMap::new(),
        }
    }

    /// Validate schema, bounds, references and credential-free values.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.schema_version != CONFIG_SCHEMA_VERSION {
            return Err(ConfigError::UnsupportedVersion(self.schema_version));
        }
        validate_names(self.agents.keys())?;
        validate_names(self.connections.keys())?;
        validate_names(self.profiles.keys())?;
        validate_names(self.agent_overrides.keys())?;
        validate_list(&self.defaults.agents, "defaults.agents")?;
        for name in &self.defaults.agents {
            if !self.agents.contains_key(name) {
                return Err(ConfigError::UnknownReference(name.clone()));
            }
        }
        validate_list(&self.defaults.measures, "defaults.measures")?;
        if self.defaults.repetitions == 0 {
            return Err(ConfigError::InvalidValue("defaults.repetitions".into()));
        }
        for (name, agent) in &self.agents {
            validate_name(name, "agent")?;
            if !self.profiles.contains_key(&agent.profile)
                || !self.connections.contains_key(&agent.connection)
            {
                return Err(ConfigError::UnknownReference(name.clone()));
            }
        }
        for (name, connection) in &self.connections {
            validate_text(&connection.endpoint, "connection.endpoint")?;
            if connection.max_in_flight == 0 {
                return Err(ConfigError::InvalidValue(name.clone()));
            }
        }
        for profile in self.profiles.values() {
            validate_text(&profile.provider, "profile.provider")?;
            validate_text(&profile.model, "profile.model")?;
            validate_endpoint(&profile.endpoint)?;
            if let Some(reference) = &profile.credential {
                reference.validate()?;
            }
        }
        for (name, override_) in &self.agent_overrides {
            if !self.agents.contains_key(name) {
                return Err(ConfigError::UnknownReference(name.clone()));
            }
            if let Some(measures) = &override_.measures {
                validate_list(measures, "agent_override.measures")?;
            }
            if override_.repetitions == Some(0) {
                return Err(ConfigError::InvalidValue(
                    "agent_override.repetitions".into(),
                ));
            }
            if let Some(profile) = &override_.profile
                && !self.profiles.contains_key(profile)
            {
                return Err(ConfigError::UnknownReference(profile.clone()));
            }
            if let Some(connection) = &override_.connection
                && !self.connections.contains_key(connection)
            {
                return Err(ConfigError::UnknownReference(connection.clone()));
            }
        }
        Ok(())
    }

    /// Resolve defaults using built-in < global < agent < run precedence.
    pub fn resolve_defaults(
        &self,
        agent: Option<&str>,
        run: Option<&RunOverride>,
    ) -> Result<EffectiveDefaults, ConfigError> {
        self.validate()?;
        let override_ = agent.and_then(|name| self.agent_overrides.get(name));
        let mut agents = Effective {
            value: self.defaults.agents.clone(),
            origin: ValueOrigin::Global,
        };
        let mut measures = Effective {
            value: self.defaults.measures.clone(),
            origin: ValueOrigin::Global,
        };
        let mut repetitions = Effective {
            value: self.defaults.repetitions,
            origin: ValueOrigin::Global,
        };
        if self.defaults.agents.is_empty() {
            agents = Effective {
                value: built_in_defaults().agents,
                origin: ValueOrigin::BuiltIn,
            };
        } else {
            // `Agent.enabled` controls membership in the implicit default
            // selection; explicit run-level selections remain authoritative.
            agents
                .value
                .retain(|name| self.agents.get(name).is_some_and(|agent| agent.enabled));
        }
        if self.defaults.measures.is_empty() {
            measures = Effective {
                value: built_in_defaults().measures,
                origin: ValueOrigin::BuiltIn,
            };
        }
        if let Some(override_) = override_ {
            if let Some(value) = &override_.measures {
                measures = Effective {
                    value: value.clone(),
                    origin: ValueOrigin::Agent,
                };
            }
            if let Some(value) = override_.repetitions {
                repetitions = Effective {
                    value,
                    origin: ValueOrigin::Agent,
                };
            }
            if override_.enabled == Some(false) {
                agents
                    .value
                    .retain(|name| name != agent.unwrap_or_default());
            }
        }
        if let Some(run) = run {
            if let Some(value) = &run.agents {
                validate_list(value, "run.agents")?;
                agents = Effective {
                    value: value.clone(),
                    origin: ValueOrigin::Run,
                };
            }
            if let Some(value) = &run.measures {
                validate_list(value, "run.measures")?;
                measures = Effective {
                    value: value.clone(),
                    origin: ValueOrigin::Run,
                };
            }
            if let Some(value) = run.repetitions {
                if value == 0 {
                    return Err(ConfigError::InvalidValue("run.repetitions".into()));
                }
                repetitions = Effective {
                    value,
                    origin: ValueOrigin::Run,
                };
            }
        }
        Ok(EffectiveDefaults {
            agents,
            measures,
            repetitions,
        })
    }
}

/// Errors raised by validation and durable persistence.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// Unsupported schema version.
    #[error("unsupported configuration schema version: {0}")]
    UnsupportedVersion(u16),
    /// Input exceeded a bounded field or document limit.
    #[error("configuration value exceeds bound: {0}")]
    TooLarge(&'static str),
    /// Invalid value or path.
    #[error("invalid configuration value: {0}")]
    InvalidValue(String),
    /// A reference points at a missing declaration.
    #[error("unknown configuration reference: {0}")]
    UnknownReference(String),
    /// Configuration file is corrupt or malformed.
    #[error("configuration is corrupt: {0}")]
    Corrupt(String),
    /// Filesystem failure.
    #[error("configuration storage I/O failed: {0}")]
    Io(#[from] io::Error),
    /// Serialization failure.
    #[error("configuration encoding failed: {0}")]
    Encoding(#[from] serde_json::Error),
}

/// Owner-private XDG configuration store.
#[derive(Debug)]
pub struct ConfigStore {
    path: PathBuf,
}

impl ConfigStore {
    /// Resolve `$XDG_CONFIG_HOME/asb/config.json`, with a private HOME fallback.
    pub fn from_environment() -> Result<Self, ConfigError> {
        let root = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
            .ok_or_else(|| ConfigError::InvalidValue("missing XDG_CONFIG_HOME/HOME".into()))?;
        Ok(Self::new(root.join("asb").join("config.json")))
    }

    /// Construct a store at an explicit path (primarily for isolated callers/tests).
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Return the configured path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Load a validated document, returning `None` when no file exists.
    pub fn load(&self) -> Result<Option<Configuration>, ConfigError> {
        if !self.path.exists() {
            return Ok(None);
        }
        reject_symlink_components(&self.path)?;
        reject_symlink(&self.path)?;
        let metadata = fs::metadata(&self.path)?;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(ConfigError::InvalidValue(
                "configuration file is not owner-private".into(),
            ));
        }
        if metadata.len() > MAX_CONFIG_BYTES as u64 {
            return Err(ConfigError::TooLarge("configuration"));
        }
        let mut file = File::open(&self.path)?;
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        file.read_to_end(&mut bytes)?;
        Ok(Some(decode(&bytes)?))
    }

    /// Atomically persist a validated, bounded, owner-private document.
    pub fn save(&self, config: &Configuration) -> Result<(), ConfigError> {
        config.validate()?;
        let bytes = serde_json::to_vec_pretty(config)?;
        if bytes.len() > MAX_CONFIG_BYTES {
            return Err(ConfigError::TooLarge("configuration"));
        }
        let parent = self
            .path
            .parent()
            .ok_or_else(|| ConfigError::InvalidValue("configuration path has no parent".into()))?;
        reject_symlink_components(&self.path)?;
        private_dir(parent)?;
        reject_symlink(parent)?;
        let temp = parent.join(format!(
            ".{}.tmp",
            self.path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("config")
        ));
        reject_symlink(&temp)?;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true).mode(0o600);
        let result: Result<(), io::Error> = (|| {
            let mut file = options.open(&temp)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temp, &self.path)?;
            File::open(parent)?.sync_all()?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result.map_err(ConfigError::from)
    }
}

/// Decode and validate a bounded configuration document.
///
/// Documents from the pre-versioned prototype are migrated by adding schema
/// version 1 in memory; newer or malformed documents are rejected without
/// changing the source bytes.
pub fn decode(bytes: &[u8]) -> Result<Configuration, ConfigError> {
    if bytes.len() > MAX_CONFIG_BYTES {
        return Err(ConfigError::TooLarge("configuration"));
    }
    let mut value: Value =
        serde_json::from_slice(bytes).map_err(|error| ConfigError::Corrupt(error.to_string()))?;
    if let Value::Object(object) = &mut value
        && !object.contains_key("schema_version")
    {
        object.insert("schema_version".into(), Value::from(CONFIG_SCHEMA_VERSION));
    }
    let config: Configuration =
        serde_json::from_value(value).map_err(|error| ConfigError::Corrupt(error.to_string()))?;
    config.validate()?;
    Ok(config)
}

fn private_dir(path: &Path) -> Result<(), ConfigError> {
    reject_symlink_components(path)?;
    if path.exists() {
        reject_symlink(path)?;
        let metadata = fs::metadata(path)?;
        if !metadata.is_dir() {
            return Err(ConfigError::InvalidValue(
                "configuration parent is not a directory".into(),
            ));
        }
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
        return Ok(());
    }
    fs::create_dir_all(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

fn reject_symlink(path: &Path) -> Result<(), ConfigError> {
    if fs::symlink_metadata(path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        Err(ConfigError::InvalidValue(
            "symlink configuration path".into(),
        ))
    } else {
        Ok(())
    }
}

fn reject_symlink_components(path: &Path) -> Result<(), ConfigError> {
    let mut current = if path.is_absolute() {
        PathBuf::from("/")
    } else {
        std::env::current_dir()?
    };
    for component in path.components() {
        use std::path::Component;
        match component {
            Component::RootDir | Component::CurDir => continue,
            Component::ParentDir => {
                current.pop();
            }
            Component::Normal(part) => {
                current.push(part);
                if fs::symlink_metadata(&current)
                    .map(|metadata| metadata.file_type().is_symlink())
                    .unwrap_or(false)
                {
                    return Err(ConfigError::InvalidValue(
                        "symlink configuration path".into(),
                    ));
                }
            }
            Component::Prefix(_) => {}
        }
    }
    Ok(())
}
fn validate_name(name: &str, field: &str) -> Result<(), ConfigError> {
    if name.is_empty() || name.len() > MAX_NAME_BYTES || name.chars().any(|c| c.is_control()) {
        return Err(ConfigError::InvalidValue(field.into()));
    }
    Ok(())
}
fn validate_names<'a>(names: impl Iterator<Item = &'a String>) -> Result<(), ConfigError> {
    for name in names {
        validate_name(name, "name")?;
    }
    Ok(())
}
fn validate_text(value: &str, field: &'static str) -> Result<(), ConfigError> {
    if value.is_empty()
        || value.len() > MAX_VALUE_BYTES
        || value.chars().any(|c| c.is_control())
        || looks_secret(value)
    {
        return Err(ConfigError::InvalidValue(field.into()));
    }
    Ok(())
}
fn validate_endpoint(value: &str) -> Result<(), ConfigError> {
    validate_text(value, "profile.endpoint")?;
    if value.contains('@')
        || value.contains("token=")
        || value.contains("key=")
        || value.contains("secret=")
    {
        return Err(ConfigError::InvalidValue("profile.endpoint".into()));
    }
    Ok(())
}
fn looks_secret(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.contains("bearer ")
        || lower.contains("sk-")
        || lower.contains("api_key=")
        || lower.contains("password=")
        || lower.contains("secret=")
}
fn validate_list(values: &[String], field: &'static str) -> Result<(), ConfigError> {
    if values.len() > 256 {
        return Err(ConfigError::TooLarge(field));
    }
    let mut seen = BTreeSet::new();
    for value in values {
        validate_name(value, field)?;
        if !seen.insert(value) {
            return Err(ConfigError::InvalidValue(field.into()));
        }
    }
    Ok(())
}
fn validate_sha256(value: &str, field: &'static str) -> Result<(), ConfigError> {
    if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ConfigError::InvalidValue(field.into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path() -> PathBuf {
        std::env::temp_dir().join(format!(
            "asb-config-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }
    fn sample() -> Configuration {
        let mut profiles = BTreeMap::new();
        profiles.insert(
            "default".into(),
            ModelProfile {
                provider: "ollama".into(),
                model: "llama".into(),
                endpoint: "http://127.0.0.1:11434".into(),
                credential: None,
            },
        );
        let mut connections = BTreeMap::new();
        connections.insert(
            "local".into(),
            Connection {
                endpoint: "loopback".into(),
                max_in_flight: 1,
            },
        );
        let mut agents = BTreeMap::new();
        agents.insert(
            "codex".into(),
            Agent {
                profile: "default".into(),
                connection: "local".into(),
                enabled: true,
            },
        );
        Configuration {
            schema_version: CONFIG_SCHEMA_VERSION,
            agents,
            connections,
            profiles,
            defaults: Defaults {
                agents: vec!["codex".into()],
                measures: vec!["latency".into()],
                repetitions: 2,
            },
            agent_overrides: BTreeMap::new(),
        }
    }

    #[test]
    fn round_trip_and_restart() {
        let path = temp_path().join("config.json");
        let store = ConfigStore::new(&path);
        store.save(&sample()).unwrap();
        assert_eq!(store.load().unwrap(), Some(sample()));
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }
    #[test]
    fn secret_shaped_values_rejected() {
        let mut config = sample();
        config.profiles.get_mut("default").unwrap().endpoint = "https://x/?api_key=secret".into();
        assert!(config.validate().is_err());
    }
    #[test]
    fn malformed_file_does_not_get_replaced() {
        let path = temp_path().join("config.json");
        private_dir(path.parent().unwrap()).unwrap();
        fs::write(&path, b"{").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let store = ConfigStore::new(&path);
        assert!(matches!(store.load(), Err(ConfigError::Corrupt(_))));
        assert_eq!(fs::read(&path).unwrap(), b"{");
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }
    #[test]
    fn precedence_is_explicit() {
        let mut config = sample();
        let resolved = config.resolve_defaults(None, None).unwrap();
        assert_eq!(resolved.repetitions.origin, ValueOrigin::Global);
        let run = RunOverride {
            repetitions: Some(5),
            ..Default::default()
        };
        assert_eq!(
            config
                .resolve_defaults(None, Some(&run))
                .unwrap()
                .repetitions,
            Effective {
                value: 5,
                origin: ValueOrigin::Run
            }
        );
        config.defaults.repetitions = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn disabled_default_agents_are_not_selected() {
        let mut config = sample();
        config.agents.get_mut("codex").unwrap().enabled = false;
        assert!(
            config
                .resolve_defaults(None, None)
                .unwrap()
                .agents
                .value
                .is_empty()
        );
    }

    #[test]
    fn defaults_must_reference_declared_agents() {
        let mut config = sample();
        config.defaults.agents = vec!["missing".into()];
        assert!(matches!(
            config.validate(),
            Err(ConfigError::UnknownReference(name)) if name == "missing"
        ));
    }

    #[test]
    fn profile_application_is_all_or_nothing() {
        let config = sample();
        let mut changed = config.clone();
        changed.profiles.insert(
            "second".into(),
            ModelProfile {
                provider: "ollama".into(),
                model: "other".into(),
                endpoint: "http://127.0.0.1:11434".into(),
                credential: None,
            },
        );
        let targets = vec!["codex".into()];
        let applied = apply_profile_to_agents(&changed, "second", &targets).unwrap();
        assert_eq!(applied.agents["codex"].profile, "second");
        assert!(apply_profile_to_agents(&config, "missing", &targets).is_err());
        assert!(apply_profile_to_agents(&config, "default", &["missing".into()]).is_err());
        assert_eq!(config, sample());
    }
    #[test]
    fn credential_bytes_are_not_deserializable() {
        let raw = r#"{"kind":"environment","locator_sha256":"sk-secret"}"#;
        assert!(serde_json::from_str::<CredentialReference>(raw).is_err());
    }

    #[test]
    fn provider_specific_catalogs_are_bounded_and_fail_closed() {
        let connection = RegistryConnectionV1 {
            provider: "openai".into(),
            protocol: "openai_chat".into(),
            endpoint_identity_sha256: "a".repeat(64),
            credential_locator_sha256: None,
        };
        let mut registry = ProviderRegistryV1 {
            schema_version: 1,
            connections: BTreeMap::from([("primary".into(), connection)]),
            models: BTreeMap::new(),
        };
        registry
            .parse_openai_model_catalog("primary", 1, br#"{"data":[{"id":"gpt-test"}]}"#)
            .unwrap();
        assert_eq!(registry.models["primary"][0].id, "gpt-test");
        assert!(
            registry
                .parse_gemini_model_catalog(
                    "primary",
                    2,
                    br#"{"models":[{"name":"models/gemini-test"}]}"#,
                )
                .is_ok()
        );
        assert_eq!(registry.models["primary"][0].id, "gemini-test");
        assert!(
            registry
                .parse_ollama_model_catalog("primary", 3, br#"{"models":[{"name":"llama-test"}]}"#,)
                .is_ok()
        );
        assert!(
            registry
                .parse_openai_model_catalog("primary", 4, br#"{"data":[{"id":"x"}],"extra":true}"#)
                .is_err()
        );
        assert!(
            registry
                .parse_ollama_model_catalog(
                    "primary",
                    4,
                    br#"{"models":[{"name":"x"},{"name":"x"}]}"#
                )
                .is_err()
        );
    }

    #[test]
    fn unversioned_document_is_migrated_without_writing_it() {
        let encoded = serde_json::to_value(sample()).unwrap();
        let mut object = encoded.as_object().unwrap().clone();
        object.remove("schema_version");
        let migrated = decode(&serde_json::to_vec(&object).unwrap()).unwrap();
        assert_eq!(migrated, sample());
    }

    #[test]
    fn auth_enrollment_is_versioned_and_rejects_unknown_fields() {
        let enrollment = AuthEnrollment {
            schema_version: 1,
            provider: "openai".into(),
            credential: CredentialReference {
                kind: CredentialReferenceKind::Environment,
                locator_sha256: "a".repeat(64),
            },
            endpoint_identity_sha256: "b".repeat(64),
            generation: 1,
            status: AuthEnrollmentStatus::Active,
        };
        let mut value = serde_json::to_value(enrollment).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("secret".into(), "nope".into());
        assert!(serde_json::from_value::<AuthEnrollment>(value).is_err());
    }
}
