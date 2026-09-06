// SPDX-License-Identifier: MIT
//! Credential-free provider profiles and fail-closed adapter negotiation.

use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Current provider-profile contract generation.
pub const PROVIDER_PROFILE_V1: ProviderProfileVersion =
    ProviderProfileVersion { major: 1, minor: 0 };
/// Largest minor generation that may appear in a capability range.
pub const MAX_PROVIDER_PROFILE_MINOR: u16 = 1_024;
/// Largest text field accepted by v1.
pub const MAX_PROVIDER_TEXT_BYTES: usize = 1_024;
/// Largest request or response body accepted by v1.
pub const MAX_PROVIDER_BODY_BYTES: u32 = 16 * 1024 * 1024;
/// Largest connect or request timeout accepted by v1.
pub const MAX_PROVIDER_TIMEOUT_MS: u64 = 60 * 60 * 1_000;
/// Largest provider concurrency accepted by v1.
pub const MAX_PROVIDER_CONCURRENCY: u16 = 1_024;

/// A major/minor provider-profile contract generation.
#[derive(
    Clone, Copy, Debug, Deserialize, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(deny_unknown_fields)]
pub struct ProviderProfileVersion {
    /// Breaking contract generation.
    pub major: u16,
    /// Backward-compatible contract generation.
    pub minor: u16,
}

/// Provider API family without account or endpoint identity.
#[derive(
    Clone, Copy, Debug, Deserialize, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    /// OpenAI's public API.
    OpenAi,
    /// A separately operated OpenAI-compatible API.
    OpenAiCompatible,
    /// Anthropic's public API.
    Anthropic,
    /// A local or remote Ollama API.
    Ollama,
}

/// Network placement without a URL, hostname, or private path.
#[derive(
    Clone, Copy, Debug, Deserialize, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum EndpointClass {
    /// Provider-operated public service.
    PublicService,
    /// Loopback endpoint in the attempt environment.
    Loopback,
    /// Private-network endpoint outside loopback.
    PrivateNetwork,
    /// Local replay endpoint backed by pinned evidence.
    Replay,
}

/// Credential lookup mechanism without its value or lookup identifier.
#[derive(
    Clone, Copy, Debug, Deserialize, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum CredentialSource {
    /// No credential is used.
    None,
    /// The process environment supplies the credential.
    Environment,
    /// An inherited file descriptor supplies the credential.
    FileDescriptor,
    /// A helper supplies the credential at execution time.
    Helper,
}

/// Credential provenance without secret values, names, or paths.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[schemars(transform = credential_provenance_schema)]
pub struct CredentialProvenance {
    /// Lookup mechanism used at the adapter boundary.
    pub source: CredentialSource,
    /// Digest of the stable lookup reference, absent only for no credential.
    #[serde(deserialize_with = "required_option")]
    #[schemars(regex(pattern = r"^[0-9a-f]{64}$"), length(equal = 64))]
    pub reference_sha256: Option<String>,
}

/// Endpoint provenance without serializing its URL or authority.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EndpointProvenance {
    /// Network placement of the endpoint.
    pub class: EndpointClass,
    /// Digest of canonical scheme, authority, and base path.
    #[schemars(regex(pattern = r"^[0-9a-f]{64}$"), length(equal = 64))]
    pub identity_sha256: String,
}

/// Explicit inference settings; absence requires proven omission.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[schemars(transform = provider_settings_schema)]
pub struct ProviderSettings {
    /// Temperature multiplied by 1,000.
    #[serde(deserialize_with = "required_option")]
    #[schemars(range(max = 2000))]
    pub temperature_milli: Option<u16>,
    /// Top-p multiplied by 1,000,000.
    #[serde(deserialize_with = "required_option")]
    #[schemars(range(max = 1000000))]
    pub top_p_millionth: Option<u32>,
    /// Deterministic provider seed.
    #[serde(deserialize_with = "required_option")]
    pub seed: Option<u64>,
    /// Requested maximum output tokens.
    #[serde(deserialize_with = "required_option")]
    #[schemars(range(min = 1))]
    pub max_output_tokens: Option<u32>,
    /// Provider-independent reasoning-effort name.
    #[serde(deserialize_with = "required_option")]
    #[schemars(length(min = 1, max = 1024))]
    pub reasoning_effort: Option<String>,
    /// Digest of the canonical credential-free provider-specific remainder.
    #[serde(deserialize_with = "required_option")]
    #[schemars(regex(pattern = r"^[0-9a-f]{64}$"), length(equal = 64))]
    pub additional_settings_sha256: Option<String>,
}

/// Transport limits that an adapter must preserve exactly.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderTransportLimits {
    /// Maximum serialized request body.
    #[schemars(range(min = 1, max = 16777216))]
    pub max_request_bytes: u32,
    /// Maximum serialized response body.
    #[schemars(range(min = 1, max = 16777216))]
    pub max_response_bytes: u32,
    /// Monotonic connection timeout.
    #[schemars(range(min = 1, max = 3600000))]
    pub connect_timeout_ms: u64,
    /// Monotonic end-to-end request timeout.
    #[schemars(range(min = 1, max = 3600000))]
    pub request_timeout_ms: u64,
    /// Maximum concurrent provider requests.
    #[schemars(range(min = 1, max = 1024))]
    pub max_concurrent_requests: u16,
}

/// Complete v1 credential-free provider configuration.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[schemars(transform = provider_profile_schema)]
pub struct ProviderProfileV1 {
    /// Contract generation.
    pub version: ProviderProfileVersion,
    /// Canonical digest of every credential-free field.
    #[schemars(regex(pattern = r"^[0-9a-f]{64}$"), length(equal = 64))]
    pub settings_sha256: String,
    /// Provider API family.
    pub provider: ProviderKind,
    /// Redacted endpoint identity and placement.
    pub endpoint: EndpointProvenance,
    /// Exact provider model identifier.
    #[schemars(length(min = 1, max = 1024))]
    pub model: String,
    /// Explicit inference values and omissions.
    pub settings: ProviderSettings,
    /// Exact transport bounds.
    pub transport: ProviderTransportLimits,
    /// Credential lookup provenance without a value.
    pub credential: CredentialProvenance,
}

/// Optional setting whose value and omission are negotiated independently.
#[derive(
    Clone, Copy, Debug, Deserialize, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ProviderSettingField {
    /// Temperature.
    Temperature,
    /// Top-p.
    TopP,
    /// Provider seed.
    Seed,
    /// Output-token bound.
    MaxOutputTokens,
    /// Reasoning effort.
    ReasoningEffort,
    /// Provider-specific credential-free remainder.
    AdditionalSettings,
}

const ALL_SETTINGS: [ProviderSettingField; 6] = [
    ProviderSettingField::Temperature,
    ProviderSettingField::TopP,
    ProviderSettingField::Seed,
    ProviderSettingField::MaxOutputTokens,
    ProviderSettingField::ReasoningEffort,
    ProviderSettingField::AdditionalSettings,
];

/// Exact boundary states supported for one optional setting.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OptionalSettingSupport {
    /// A supplied value can be emitted without conversion.
    pub exact_value: bool,
    /// Absence can be proven at the provider boundary.
    pub explicit_omission: bool,
}

/// Bounded provider-profile capabilities of one concrete adapter.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderProfileCapabilities {
    /// Lowest accepted contract generation.
    pub minimum_version: ProviderProfileVersion,
    /// Highest accepted contract generation.
    pub maximum_version: ProviderProfileVersion,
    /// Provider families translated without semantic changes.
    pub providers: BTreeSet<ProviderKind>,
    /// Endpoint classes translated without placement changes.
    pub endpoint_classes: BTreeSet<EndpointClass>,
    /// Credential injection mechanisms supported without changing provenance.
    pub credential_sources: BTreeSet<CredentialSource>,
    /// Complete v1 value-and-omission support matrix.
    pub settings: BTreeMap<ProviderSettingField, OptionalSettingSupport>,
    /// Greatest transport values enforceable exactly.
    pub transport_ceiling: ProviderTransportLimits,
}

/// Successful pre-start negotiation awaiting effective-output proof.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NegotiatedProviderProfile {
    expected_sha256: String,
}

impl NegotiatedProviderProfile {
    /// Credential-free identity the adapter must produce.
    pub fn expected_sha256(&self) -> &str {
        &self.expected_sha256
    }

    /// Verify the effective adapter output exactly matches negotiation.
    pub fn verify_effective(
        self,
        effective: &ProviderProfileV1,
    ) -> Result<VerifiedProviderProfile, ProviderProfileError> {
        effective.validate()?;
        if effective.settings_sha256 != self.expected_sha256 {
            return Err(ProviderProfileError::LossyTranslation);
        }
        Ok(VerifiedProviderProfile {
            settings_sha256: self.expected_sha256,
        })
    }
}

/// Constructor-controlled proof of exact effective configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedProviderProfile {
    settings_sha256: String,
}

impl VerifiedProviderProfile {
    /// Credential-free identity proven at the adapter boundary.
    pub fn settings_sha256(&self) -> &str {
        &self.settings_sha256
    }
}

impl ProviderProfileV1 {
    /// Compute the language-independent credential-free identity.
    pub fn compute_settings_sha256(&self) -> Result<String, ProviderProfileError> {
        self.validate_components()?;
        let mut out = ProfileHash::new();
        out.u16(self.version.major);
        out.u16(self.version.minor);
        out.byte(provider_tag(self.provider));
        out.byte(endpoint_tag(self.endpoint.class));
        out.text(&self.endpoint.identity_sha256);
        out.text(&self.model);
        out.opt_u16(self.settings.temperature_milli);
        out.opt_u32(self.settings.top_p_millionth);
        out.opt_u64(self.settings.seed);
        out.opt_u32(self.settings.max_output_tokens);
        out.opt_text(self.settings.reasoning_effort.as_deref());
        out.opt_text(self.settings.additional_settings_sha256.as_deref());
        out.u32(self.transport.max_request_bytes);
        out.u32(self.transport.max_response_bytes);
        out.u64(self.transport.connect_timeout_ms);
        out.u64(self.transport.request_timeout_ms);
        out.u16(self.transport.max_concurrent_requests);
        out.byte(credential_tag(self.credential.source));
        out.opt_text(self.credential.reference_sha256.as_deref());
        Ok(out.finish())
    }

    /// Replace the declared identity with the canonical digest.
    pub fn refresh_settings_sha256(&mut self) -> Result<(), ProviderProfileError> {
        self.settings_sha256 = self.compute_settings_sha256()?;
        Ok(())
    }

    /// Validate all fields and the declared canonical identity.
    pub fn validate(&self) -> Result<(), ProviderProfileError> {
        let expected = self.compute_settings_sha256()?;
        digest(&self.settings_sha256, "settings_sha256")?;
        if expected != self.settings_sha256 {
            return Err(ProviderProfileError::IdentityMismatch);
        }
        Ok(())
    }

    /// Negotiate exact adapter support before starting an agent.
    pub fn negotiate(
        &self,
        capabilities: &ProviderProfileCapabilities,
    ) -> Result<NegotiatedProviderProfile, ProviderProfileError> {
        self.validate()?;
        capabilities.validate()?;
        if self.version < capabilities.minimum_version
            || self.version > capabilities.maximum_version
        {
            return Err(ProviderProfileError::UnsupportedVersion(self.version));
        }
        if !capabilities.providers.contains(&self.provider) {
            return Err(ProviderProfileError::UnsupportedProvider(self.provider));
        }
        if !capabilities.endpoint_classes.contains(&self.endpoint.class) {
            return Err(ProviderProfileError::UnsupportedEndpoint(
                self.endpoint.class,
            ));
        }
        if !capabilities
            .credential_sources
            .contains(&self.credential.source)
        {
            return Err(ProviderProfileError::UnsupportedCredentialSource(
                self.credential.source,
            ));
        }
        for (field, present) in self.presence() {
            let support = capabilities
                .settings
                .get(&field)
                .ok_or(ProviderProfileError::IncompleteCapabilities(field))?;
            if (present && !support.exact_value) || (!present && !support.explicit_omission) {
                return Err(ProviderProfileError::UnsupportedSetting { field, present });
            }
        }
        if !capabilities.transport_ceiling.contains(self.transport) {
            return Err(ProviderProfileError::UnsupportedTransport);
        }
        Ok(NegotiatedProviderProfile {
            expected_sha256: self.settings_sha256.clone(),
        })
    }

    fn validate_components(&self) -> Result<(), ProviderProfileError> {
        if self.version.major != 1 || self.version.minor > PROVIDER_PROFILE_V1.minor {
            return Err(ProviderProfileError::UnsupportedVersion(self.version));
        }
        digest(&self.endpoint.identity_sha256, "endpoint.identity_sha256")?;
        text(&self.model, "model")?;
        self.settings.validate()?;
        self.transport.validate()?;
        match (
            self.credential.source,
            self.credential.reference_sha256.as_deref(),
        ) {
            (CredentialSource::None, None) => {}
            (CredentialSource::None, Some(_)) | (_, None) => {
                return Err(ProviderProfileError::InconsistentCredentialProvenance);
            }
            (_, Some(reference)) => digest(reference, "credential.reference_sha256")?,
        }
        Ok(())
    }

    fn presence(&self) -> [(ProviderSettingField, bool); 6] {
        [
            (
                ProviderSettingField::Temperature,
                self.settings.temperature_milli.is_some(),
            ),
            (
                ProviderSettingField::TopP,
                self.settings.top_p_millionth.is_some(),
            ),
            (ProviderSettingField::Seed, self.settings.seed.is_some()),
            (
                ProviderSettingField::MaxOutputTokens,
                self.settings.max_output_tokens.is_some(),
            ),
            (
                ProviderSettingField::ReasoningEffort,
                self.settings.reasoning_effort.is_some(),
            ),
            (
                ProviderSettingField::AdditionalSettings,
                self.settings.additional_settings_sha256.is_some(),
            ),
        ]
    }
}

impl ProviderProfileCapabilities {
    /// Validate the bounded version interval and complete setting matrix.
    pub fn validate(&self) -> Result<(), ProviderProfileError> {
        if self.minimum_version.major != 1
            || self.maximum_version.major != 1
            || self.minimum_version > self.maximum_version
            || self.maximum_version.minor > MAX_PROVIDER_PROFILE_MINOR
        {
            return Err(ProviderProfileError::InvalidCapabilityVersions);
        }
        for field in ALL_SETTINGS {
            if !self.settings.contains_key(&field) {
                return Err(ProviderProfileError::IncompleteCapabilities(field));
            }
        }
        self.transport_ceiling.validate()
    }
}

impl ProviderSettings {
    fn validate(&self) -> Result<(), ProviderProfileError> {
        if self.temperature_milli.is_some_and(|v| v > 2_000)
            || self.top_p_millionth.is_some_and(|v| v > 1_000_000)
            || self.max_output_tokens == Some(0)
        {
            return Err(ProviderProfileError::InvalidSettings);
        }
        if let Some(value) = &self.reasoning_effort {
            text(value, "settings.reasoning_effort")?;
        }
        if let Some(value) = &self.additional_settings_sha256 {
            digest(value, "settings.additional_settings_sha256")?;
        }
        Ok(())
    }
}

impl ProviderTransportLimits {
    fn validate(&self) -> Result<(), ProviderProfileError> {
        if self.max_request_bytes == 0
            || self.max_request_bytes > MAX_PROVIDER_BODY_BYTES
            || self.max_response_bytes == 0
            || self.max_response_bytes > MAX_PROVIDER_BODY_BYTES
            || self.connect_timeout_ms == 0
            || self.connect_timeout_ms > MAX_PROVIDER_TIMEOUT_MS
            || self.request_timeout_ms == 0
            || self.request_timeout_ms > MAX_PROVIDER_TIMEOUT_MS
            || self.max_concurrent_requests == 0
            || self.max_concurrent_requests > MAX_PROVIDER_CONCURRENCY
        {
            return Err(ProviderProfileError::InvalidTransport);
        }
        Ok(())
    }

    fn contains(self, requested: Self) -> bool {
        requested.max_request_bytes <= self.max_request_bytes
            && requested.max_response_bytes <= self.max_response_bytes
            && requested.connect_timeout_ms <= self.connect_timeout_ms
            && requested.request_timeout_ms <= self.request_timeout_ms
            && requested.max_concurrent_requests <= self.max_concurrent_requests
    }
}

/// Validation or pre-start negotiation failure.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ProviderProfileError {
    /// Contract generation is unsupported or outside the adapter range.
    #[error("unsupported provider-profile version {0:?}")]
    UnsupportedVersion(ProviderProfileVersion),
    /// Text is empty, oversized, padded, or contains a control character.
    #[error("invalid text field {0}")]
    InvalidText(&'static str),
    /// A digest is not lowercase hexadecimal SHA-256.
    #[error("invalid SHA-256 field {0}")]
    InvalidSha256(&'static str),
    /// Inference settings exceed the v1 domain.
    #[error("invalid provider settings")]
    InvalidSettings,
    /// Transport limits are zero or exceed the v1 ceiling.
    #[error("invalid provider transport limits")]
    InvalidTransport,
    /// Credential source and redacted reference presence disagree.
    #[error("credential source and reference presence disagree")]
    InconsistentCredentialProvenance,
    /// Declared identity differs from the canonical fields.
    #[error("provider profile identity mismatch")]
    IdentityMismatch,
    /// Adapter version bounds are malformed or outside v1.
    #[error("invalid adapter version bounds")]
    InvalidCapabilityVersions,
    /// Capability matrix omitted a required v1 setting.
    #[error("adapter capability matrix omits {0:?}")]
    IncompleteCapabilities(ProviderSettingField),
    /// Adapter does not support the provider family.
    #[error("unsupported provider {0:?}")]
    UnsupportedProvider(ProviderKind),
    /// Adapter does not support the endpoint placement.
    #[error("unsupported endpoint class {0:?}")]
    UnsupportedEndpoint(EndpointClass),
    /// Adapter does not support the credential injection mechanism.
    #[error("unsupported credential source {0:?}")]
    UnsupportedCredentialSource(CredentialSource),
    /// Adapter cannot preserve a supplied value or explicit omission.
    #[error("unsupported setting state {field:?}, present={present}")]
    UnsupportedSetting {
        /// Setting that cannot be translated exactly.
        field: ProviderSettingField,
        /// Whether the requested profile supplied a value.
        present: bool,
    },
    /// Requested transport bounds exceed the exact adapter ceiling.
    #[error("unsupported provider transport limits")]
    UnsupportedTransport,
    /// Effective output differs from the negotiated identity.
    #[error("lossy provider-profile translation")]
    LossyTranslation,
}

fn text(value: &str, field: &'static str) -> Result<(), ProviderProfileError> {
    if value.is_empty()
        || value.len() > MAX_PROVIDER_TEXT_BYTES
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return Err(ProviderProfileError::InvalidText(field));
    }
    Ok(())
}

fn digest(value: &str, field: &'static str) -> Result<(), ProviderProfileError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(ProviderProfileError::InvalidSha256(field));
    }
    Ok(())
}

fn required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

fn provider_profile_schema(schema: &mut schemars::Schema) {
    schema.ensure_object().insert(
        "allOf".into(),
        serde_json::json!([
            {
                "properties": {
                    "version": {
                        "properties": {
                            "major": {"const": 1},
                            "minor": {"const": 0}
                        },
                        "required": ["major", "minor"]
                    }
                }
            },
            {
                "oneOf": [
                    {
                        "properties": {
                            "credential": {
                                "properties": {
                                    "source": {"const": "none"},
                                    "reference_sha256": {"type": "null"}
                                },
                                "required": ["source", "reference_sha256"]
                            }
                        }
                    },
                    {
                        "properties": {
                            "credential": {
                                "properties": {
                                    "source": {
                                        "enum": ["environment", "file_descriptor", "helper"]
                                    },
                                    "reference_sha256": {
                                        "type": "string",
                                        "minLength": 64,
                                        "maxLength": 64,
                                        "pattern": "^[0-9a-f]{64}$"
                                    }
                                },
                                "required": ["source", "reference_sha256"]
                            }
                        }
                    }
                ]
            }
        ]),
    );
}

fn credential_provenance_schema(schema: &mut schemars::Schema) {
    schema.ensure_object().insert(
        "required".into(),
        serde_json::json!(["source", "reference_sha256"]),
    );
}

fn provider_settings_schema(schema: &mut schemars::Schema) {
    schema.ensure_object().insert(
        "required".into(),
        serde_json::json!([
            "temperature_milli",
            "top_p_millionth",
            "seed",
            "max_output_tokens",
            "reasoning_effort",
            "additional_settings_sha256"
        ]),
    );
}

fn provider_tag(value: ProviderKind) -> u8 {
    match value {
        ProviderKind::OpenAi => 0,
        ProviderKind::OpenAiCompatible => 1,
        ProviderKind::Anthropic => 2,
        ProviderKind::Ollama => 3,
    }
}

fn endpoint_tag(value: EndpointClass) -> u8 {
    match value {
        EndpointClass::PublicService => 0,
        EndpointClass::Loopback => 1,
        EndpointClass::PrivateNetwork => 2,
        EndpointClass::Replay => 3,
    }
}

fn credential_tag(value: CredentialSource) -> u8 {
    match value {
        CredentialSource::None => 0,
        CredentialSource::Environment => 1,
        CredentialSource::FileDescriptor => 2,
        CredentialSource::Helper => 3,
    }
}

struct ProfileHash(Sha256);

impl ProfileHash {
    fn new() -> Self {
        let mut hash = Sha256::new();
        hash.update(b"asb-provider-profile-v1\0");
        Self(hash)
    }
    fn byte(&mut self, value: u8) {
        self.0.update([value]);
    }
    fn u16(&mut self, value: u16) {
        self.0.update(value.to_be_bytes());
    }
    fn u32(&mut self, value: u32) {
        self.0.update(value.to_be_bytes());
    }
    fn u64(&mut self, value: u64) {
        self.0.update(value.to_be_bytes());
    }
    fn text(&mut self, value: &str) {
        self.u64(value.len() as u64);
        self.0.update(value.as_bytes());
    }
    fn opt_text(&mut self, value: Option<&str>) {
        self.byte(u8::from(value.is_some()));
        if let Some(value) = value {
            self.text(value);
        }
    }
    fn opt_u16(&mut self, value: Option<u16>) {
        self.byte(u8::from(value.is_some()));
        if let Some(value) = value {
            self.u16(value);
        }
    }
    fn opt_u32(&mut self, value: Option<u32>) {
        self.byte(u8::from(value.is_some()));
        if let Some(value) = value {
            self.u32(value);
        }
    }
    fn opt_u64(&mut self, value: Option<u64>) {
        self.byte(u8::from(value.is_some()));
        if let Some(value) = value {
            self.u64(value);
        }
    }
    fn finish(self) -> String {
        format!("{:x}", self.0.finalize())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> ProviderProfileV1 {
        let digest = "a".repeat(64);
        let mut value = ProviderProfileV1 {
            version: PROVIDER_PROFILE_V1,
            settings_sha256: String::new(),
            provider: ProviderKind::OpenAiCompatible,
            endpoint: EndpointProvenance {
                class: EndpointClass::PrivateNetwork,
                identity_sha256: digest.clone(),
            },
            model: "example-model-v1".into(),
            settings: ProviderSettings {
                temperature_milli: Some(250),
                top_p_millionth: Some(900_000),
                seed: Some(7),
                max_output_tokens: Some(4096),
                reasoning_effort: Some("medium".into()),
                additional_settings_sha256: Some(digest.clone()),
            },
            transport: ProviderTransportLimits {
                max_request_bytes: 1_048_576,
                max_response_bytes: 2_097_152,
                connect_timeout_ms: 10_000,
                request_timeout_ms: 300_000,
                max_concurrent_requests: 8,
            },
            credential: CredentialProvenance {
                source: CredentialSource::Environment,
                reference_sha256: Some(digest),
            },
        };
        value.refresh_settings_sha256().unwrap();
        value
    }

    fn capabilities() -> ProviderProfileCapabilities {
        ProviderProfileCapabilities {
            minimum_version: PROVIDER_PROFILE_V1,
            maximum_version: PROVIDER_PROFILE_V1,
            providers: [ProviderKind::OpenAiCompatible].into_iter().collect(),
            endpoint_classes: [EndpointClass::PrivateNetwork].into_iter().collect(),
            credential_sources: [CredentialSource::Environment].into_iter().collect(),
            settings: ALL_SETTINGS
                .into_iter()
                .map(|field| {
                    (
                        field,
                        OptionalSettingSupport {
                            exact_value: true,
                            explicit_omission: true,
                        },
                    )
                })
                .collect(),
            transport_ceiling: ProviderTransportLimits {
                max_request_bytes: MAX_PROVIDER_BODY_BYTES,
                max_response_bytes: MAX_PROVIDER_BODY_BYTES,
                connect_timeout_ms: MAX_PROVIDER_TIMEOUT_MS,
                request_timeout_ms: MAX_PROVIDER_TIMEOUT_MS,
                max_concurrent_requests: MAX_PROVIDER_CONCURRENCY,
            },
        }
    }

    #[test]
    fn identity_negotiation_and_effective_proof_are_exact() {
        let value = profile();
        assert_eq!(value.validate(), Ok(()));
        assert_eq!(
            value.settings_sha256,
            "3ef77e8fcc34900d8cece0e1bccf3bb3b23cb389669ca6eb857853be93612ae5"
        );
        let negotiated = value.negotiate(&capabilities()).unwrap();
        assert_eq!(negotiated.expected_sha256(), value.settings_sha256);
        assert_eq!(
            negotiated
                .verify_effective(&value)
                .unwrap()
                .settings_sha256(),
            value.settings_sha256
        );
    }

    #[test]
    fn unsupported_value_omission_transport_and_loss_fail_closed() {
        let value = profile();
        let mut caps = capabilities();
        caps.settings
            .get_mut(&ProviderSettingField::Seed)
            .unwrap()
            .exact_value = false;
        assert!(matches!(
            value.negotiate(&caps),
            Err(ProviderProfileError::UnsupportedSetting { present: true, .. })
        ));
        let mut omitted = value.clone();
        omitted.settings.temperature_milli = None;
        omitted.refresh_settings_sha256().unwrap();
        let mut caps = capabilities();
        caps.settings
            .get_mut(&ProviderSettingField::Temperature)
            .unwrap()
            .explicit_omission = false;
        assert!(matches!(
            omitted.negotiate(&caps),
            Err(ProviderProfileError::UnsupportedSetting { present: false, .. })
        ));
        let mut caps = capabilities();
        caps.transport_ceiling.max_response_bytes = value.transport.max_response_bytes - 1;
        assert_eq!(
            value.negotiate(&caps),
            Err(ProviderProfileError::UnsupportedTransport)
        );
        let negotiated = value.negotiate(&capabilities()).unwrap();
        let mut changed = value.clone();
        changed.model = "substituted".into();
        changed.refresh_settings_sha256().unwrap();
        assert_eq!(
            negotiated.verify_effective(&changed),
            Err(ProviderProfileError::LossyTranslation)
        );
    }

    #[test]
    fn malformed_unknown_and_unbounded_inputs_are_rejected() {
        for bad in [
            "",
            " padded",
            "control\n",
            &"x".repeat(MAX_PROVIDER_TEXT_BYTES + 1),
        ] {
            let mut value = profile();
            value.model = bad.into();
            assert!(matches!(
                value.compute_settings_sha256(),
                Err(ProviderProfileError::InvalidText(_))
            ));
        }
        let mut value = profile();
        value.endpoint.identity_sha256 = "https://secret@example.invalid".into();
        assert!(matches!(
            value.compute_settings_sha256(),
            Err(ProviderProfileError::InvalidSha256(_))
        ));
        let mut value = profile();
        value.credential.reference_sha256 = None;
        assert_eq!(
            value.compute_settings_sha256(),
            Err(ProviderProfileError::InconsistentCredentialProvenance)
        );
        let mut value = profile();
        value.settings.temperature_milli = Some(2001);
        assert_eq!(
            value.compute_settings_sha256(),
            Err(ProviderProfileError::InvalidSettings)
        );
        let mut value = profile();
        value.transport.request_timeout_ms = 0;
        assert_eq!(
            value.compute_settings_sha256(),
            Err(ProviderProfileError::InvalidTransport)
        );
        let mut json = serde_json::to_value(profile()).unwrap();
        json.as_object_mut()
            .unwrap()
            .insert("future".into(), true.into());
        assert!(serde_json::from_value::<ProviderProfileV1>(json).is_err());

        let mut missing = serde_json::to_value(profile()).unwrap();
        missing["settings"]
            .as_object_mut()
            .unwrap()
            .remove("temperature_milli");
        assert!(serde_json::from_value::<ProviderProfileV1>(missing).is_err());
        let mut explicit = serde_json::to_value(profile()).unwrap();
        explicit["settings"]["temperature_milli"] = serde_json::Value::Null;
        assert!(serde_json::from_value::<ProviderProfileV1>(explicit).is_ok());
    }

    #[test]
    fn versions_and_capability_matrix_are_bounded() {
        let value = profile();
        let mut caps = capabilities();
        caps.maximum_version.major = 2;
        assert_eq!(
            value.negotiate(&caps),
            Err(ProviderProfileError::InvalidCapabilityVersions)
        );
        let mut caps = capabilities();
        caps.settings.remove(&ProviderSettingField::TopP);
        assert_eq!(
            value.negotiate(&caps),
            Err(ProviderProfileError::IncompleteCapabilities(
                ProviderSettingField::TopP
            ))
        );
        let mut newer = value;
        newer.version.minor = 1;
        assert!(matches!(
            newer.compute_settings_sha256(),
            Err(ProviderProfileError::UnsupportedVersion(_))
        ));
    }

    #[test]
    fn generated_schema_requires_explicit_omissions_and_consistent_provenance() {
        let schema = serde_json::to_value(schemars::schema_for!(ProviderProfileV1)).unwrap();
        let validator = jsonschema::validator_for(&schema).unwrap();
        let fixture = serde_json::to_value(profile()).unwrap();
        assert!(validator.is_valid(&fixture));

        let mut missing = fixture.clone();
        missing["settings"]
            .as_object_mut()
            .unwrap()
            .remove("temperature_milli");
        assert!(!validator.is_valid(&missing));

        let mut future = fixture.clone();
        future["version"]["minor"] = 1.into();
        assert!(!validator.is_valid(&future));

        let mut inconsistent = fixture.clone();
        inconsistent["credential"]["source"] = "none".into();
        assert!(!validator.is_valid(&inconsistent));

        let mut no_credential = fixture.clone();
        no_credential["credential"]["source"] = "none".into();
        no_credential["credential"]["reference_sha256"] = serde_json::Value::Null;
        let errors: Vec<_> = validator
            .iter_errors(&no_credential)
            .map(|error| error.to_string())
            .collect();
        assert!(errors.is_empty(), "{errors:?}");

        let mut temperature = fixture;
        temperature["settings"]["temperature_milli"] = 2_001.into();
        assert!(!validator.is_valid(&temperature));
    }

    #[test]
    fn every_family_and_redacted_credential_mode_has_a_stable_tag() {
        let original = profile();
        for provider in [
            ProviderKind::OpenAi,
            ProviderKind::Anthropic,
            ProviderKind::Ollama,
        ] {
            let mut value = original.clone();
            value.provider = provider;
            value.refresh_settings_sha256().unwrap();
            assert_ne!(value.settings_sha256, original.settings_sha256);
        }
        for endpoint in [
            EndpointClass::PublicService,
            EndpointClass::Loopback,
            EndpointClass::Replay,
        ] {
            let mut value = original.clone();
            value.endpoint.class = endpoint;
            value.refresh_settings_sha256().unwrap();
            assert_ne!(value.settings_sha256, original.settings_sha256);
        }
        for source in [CredentialSource::FileDescriptor, CredentialSource::Helper] {
            let mut value = original.clone();
            value.credential.source = source;
            value.refresh_settings_sha256().unwrap();
            assert_ne!(value.settings_sha256, original.settings_sha256);
        }
        let mut no_credential = original;
        no_credential.credential.source = CredentialSource::None;
        no_credential.credential.reference_sha256 = None;
        no_credential.refresh_settings_sha256().unwrap();
        assert_eq!(no_credential.validate(), Ok(()));
    }

    #[test]
    fn identity_provider_endpoint_and_version_mismatches_fail_closed() {
        let mut value = profile();
        value.settings_sha256 = "b".repeat(64);
        assert_eq!(
            value.validate(),
            Err(ProviderProfileError::IdentityMismatch)
        );

        let value = profile();
        let mut caps = capabilities();
        caps.providers.clear();
        assert_eq!(
            value.negotiate(&caps),
            Err(ProviderProfileError::UnsupportedProvider(value.provider))
        );
        let mut caps = capabilities();
        caps.endpoint_classes.clear();
        assert_eq!(
            value.negotiate(&caps),
            Err(ProviderProfileError::UnsupportedEndpoint(
                value.endpoint.class
            ))
        );
        let mut caps = capabilities();
        caps.credential_sources.clear();
        assert_eq!(
            value.negotiate(&caps),
            Err(ProviderProfileError::UnsupportedCredentialSource(
                value.credential.source
            ))
        );
        let mut caps = capabilities();
        caps.minimum_version.minor = 1;
        caps.maximum_version.minor = 1;
        assert_eq!(
            value.negotiate(&caps),
            Err(ProviderProfileError::UnsupportedVersion(value.version))
        );
    }

    #[test]
    fn malformed_optional_settings_and_transport_ceiling_are_rejected() {
        let mut value = profile();
        value.settings.reasoning_effort = Some(" padded".into());
        assert!(matches!(
            value.compute_settings_sha256(),
            Err(ProviderProfileError::InvalidText(_))
        ));
        let mut value = profile();
        value.settings.additional_settings_sha256 = Some("A".repeat(64));
        assert!(matches!(
            value.compute_settings_sha256(),
            Err(ProviderProfileError::InvalidSha256(_))
        ));
        let value = profile();
        let mut caps = capabilities();
        caps.transport_ceiling.max_request_bytes = 0;
        assert_eq!(
            value.negotiate(&caps),
            Err(ProviderProfileError::InvalidTransport)
        );
    }
}
