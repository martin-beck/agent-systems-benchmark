// SPDX-License-Identifier: MIT
//! Pinned public OpenAI provider profile and fail-closed adapter translations.

use asb_protocol::{
    CredentialProvenance, CredentialSource, EndpointClass, EndpointProvenance, PROVIDER_PROFILE_V1,
    ProviderKind, ProviderProfileError, ProviderProfileV1, ProviderSettings,
    ProviderTransportLimits,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fmt;
use url::Url;

const MAX_OBSERVED_REQUEST_BYTES: usize = 4 * 1024 * 1024;
const MAX_AUTHORIZATION_BYTES: usize = 8 * 1024;

/// Exact public API base accepted by the built-in profile.
pub const OPENAI_API_BASE: &str = "https://api.openai.com/v1/";
/// Dated model snapshot used instead of a moving alias.
pub const OPENAI_MODEL: &str = "gpt-5.2-2025-12-11";

/// Built-in adapter whose public OpenAI route is being negotiated.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpenAiAgent {
    /// OpenCode custom-provider route.
    OpenCode,
    /// OpenDesk OpenAI-provider route.
    OpenDesk,
    /// aider OpenAI-provider route.
    Aider,
    /// Codex Responses API route.
    Codex,
    /// Gemini native Google route.
    Gemini,
    /// Qwen Code OpenAI-compatible route.
    QwenCode,
    /// Goose OpenAI route.
    Goose,
    /// mini-SWE-agent LiteLLM route.
    MiniSwe,
    /// OpenHands LiteLLM route.
    OpenHands,
}

/// Exact public API surface selected for an adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpenAiApiMode {
    /// OpenAI Chat Completions.
    ChatCompletions,
    /// OpenAI Responses.
    Responses,
}

/// Credential injection boundary used by an adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpenAiCredentialTarget {
    /// Standard OPENAI_API_KEY process environment boundary.
    OpenAiApiKey,
    /// OpenDesk provider-specific process environment boundary.
    OpenDeskApiKey,
    /// Codex provider-specific process environment boundary.
    CodexProviderKey,
}

/// Adapter-specific values derived from one exact profile.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenAiTranslation {
    endpoint: Url,
    model: String,
    api_mode: OpenAiApiMode,
    credential_target: OpenAiCredentialTarget,
}

/// Constructor-controlled validation result for a synthetic-service observation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedOpenAiRequest {
    agent: OpenAiAgent,
    api_mode: OpenAiApiMode,
    profile_sha256: String,
}

impl VerifiedOpenAiRequest {
    /// Adapter that emitted the request.
    pub const fn agent(&self) -> OpenAiAgent {
        self.agent
    }

    /// API surface observed by the synthetic service.
    pub const fn api_mode(&self) -> OpenAiApiMode {
        self.api_mode
    }

    /// Credential-free profile identity; prompts and authorization are never retained or hashed.
    pub fn profile_sha256(&self) -> &str {
        &self.profile_sha256
    }
}

impl OpenAiTranslation {
    /// Exact public provider base.
    pub fn endpoint(&self) -> &Url {
        &self.endpoint
    }

    /// Exact model spelling expected by the adapter.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// API surface selected for the adapter.
    pub const fn api_mode(&self) -> OpenAiApiMode {
        self.api_mode
    }

    /// Credential boundary; this never contains a secret value.
    pub const fn credential_target(&self) -> OpenAiCredentialTarget {
        self.credential_target
    }
}

/// Exact default public OpenAI profile.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenAiProfile {
    profile: ProviderProfileV1,
}

/// Profile construction or adapter translation failure.
#[derive(Debug)]
pub enum OpenAiProfileError {
    /// Credential reference was absent or malformed.
    InvalidCredentialReference,
    /// Adapter has no proven public OpenAI boundary.
    UnsupportedAgent(OpenAiAgent),
    /// Requested profile differs from the pinned profile.
    ProfileMismatch,
    /// Synthetic service observed malformed or semantically different request data.
    EffectiveRequestMismatch,
    /// Common provider-profile validation failed.
    Profile(ProviderProfileError),
}

impl fmt::Display for OpenAiProfileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCredentialReference => {
                formatter.write_str("invalid OpenAI credential reference digest")
            }
            Self::UnsupportedAgent(agent) => {
                write!(
                    formatter,
                    "adapter has no proven public OpenAI route: {agent:?}"
                )
            }
            Self::ProfileMismatch => formatter.write_str("OpenAI provider profile mismatch"),
            Self::EffectiveRequestMismatch => {
                formatter.write_str("effective OpenAI request does not match the profile")
            }
            Self::Profile(error) => write!(formatter, "invalid OpenAI provider profile: {error}"),
        }
    }
}

impl std::error::Error for OpenAiProfileError {}

impl From<ProviderProfileError> for OpenAiProfileError {
    fn from(value: ProviderProfileError) -> Self {
        Self::Profile(value)
    }
}

impl OpenAiProfile {
    /// Construct the default profile from a redacted logical secret-reference digest.
    pub fn new(credential_reference_sha256: impl Into<String>) -> Result<Self, OpenAiProfileError> {
        let credential_reference_sha256 = credential_reference_sha256.into();
        if !is_sha256(&credential_reference_sha256) {
            return Err(OpenAiProfileError::InvalidCredentialReference);
        }
        let endpoint = Url::parse(OPENAI_API_BASE).expect("constant public API URL");
        let mut endpoint_digest = Sha256::new();
        endpoint_digest.update(endpoint.as_str().as_bytes());
        let mut profile = ProviderProfileV1 {
            version: PROVIDER_PROFILE_V1,
            settings_sha256: String::new(),
            provider: ProviderKind::OpenAi,
            endpoint: EndpointProvenance {
                class: EndpointClass::PublicService,
                identity_sha256: format!("{:x}", endpoint_digest.finalize()),
            },
            model: OPENAI_MODEL.into(),
            settings: ProviderSettings {
                temperature_milli: None,
                top_p_millionth: None,
                seed: None,
                max_output_tokens: None,
                reasoning_effort: Some("none".into()),
                additional_settings_sha256: None,
            },
            transport: ProviderTransportLimits {
                max_request_bytes: 4 * 1024 * 1024,
                max_response_bytes: 16 * 1024 * 1024,
                connect_timeout_ms: 5_000,
                request_timeout_ms: 30 * 60 * 1_000,
                max_concurrent_requests: 1,
            },
            credential: CredentialProvenance {
                source: CredentialSource::Environment,
                reference_sha256: Some(credential_reference_sha256),
            },
        };
        profile.refresh_settings_sha256()?;
        Ok(Self { profile })
    }

    /// Credential-free canonical provider evidence.
    pub fn provider_profile(&self) -> &ProviderProfileV1 {
        &self.profile
    }

    /// Translate an exact profile or reject before any process starts.
    pub fn translate(
        &self,
        agent: OpenAiAgent,
        requested: &ProviderProfileV1,
    ) -> Result<OpenAiTranslation, OpenAiProfileError> {
        requested.validate()?;
        if requested != &self.profile {
            return Err(OpenAiProfileError::ProfileMismatch);
        }
        let (api_mode, credential_target, model) = match agent {
            OpenAiAgent::Codex => (
                OpenAiApiMode::Responses,
                OpenAiCredentialTarget::CodexProviderKey,
                OPENAI_MODEL.into(),
            ),
            OpenAiAgent::OpenDesk => (
                OpenAiApiMode::ChatCompletions,
                OpenAiCredentialTarget::OpenDeskApiKey,
                OPENAI_MODEL.into(),
            ),
            OpenAiAgent::OpenCode => (
                OpenAiApiMode::ChatCompletions,
                OpenAiCredentialTarget::OpenAiApiKey,
                format!("openai/{OPENAI_MODEL}"),
            ),
            OpenAiAgent::Aider
            | OpenAiAgent::QwenCode
            | OpenAiAgent::Goose
            | OpenAiAgent::MiniSwe
            | OpenAiAgent::OpenHands => (
                OpenAiApiMode::ChatCompletions,
                OpenAiCredentialTarget::OpenAiApiKey,
                OPENAI_MODEL.into(),
            ),
            OpenAiAgent::Gemini => return Err(OpenAiProfileError::UnsupportedAgent(agent)),
        };
        Ok(OpenAiTranslation {
            endpoint: Url::parse(OPENAI_API_BASE).expect("constant public API URL"),
            model,
            api_mode,
            credential_target,
        })
    }

    /// Verify a bounded synthetic-service observation without retaining its secret or prompt.
    pub fn verify_effective_request(
        &self,
        agent: OpenAiAgent,
        requested: &ProviderProfileV1,
        path: &str,
        authorization: &str,
        body: &[u8],
    ) -> Result<VerifiedOpenAiRequest, OpenAiProfileError> {
        let translation = self.translate(agent, requested)?;
        if body.is_empty()
            || body.len() > MAX_OBSERVED_REQUEST_BYTES
            || authorization.len() > MAX_AUTHORIZATION_BYTES
            || !authorization.strip_prefix("Bearer ").is_some_and(|token| {
                !token.is_empty() && !token.bytes().any(|byte| byte.is_ascii_whitespace())
            })
        {
            return Err(OpenAiProfileError::EffectiveRequestMismatch);
        }
        let expected_path = match translation.api_mode {
            OpenAiApiMode::ChatCompletions => "/v1/chat/completions",
            OpenAiApiMode::Responses => "/v1/responses",
        };
        let value: Value = serde_json::from_slice(body)
            .map_err(|_| OpenAiProfileError::EffectiveRequestMismatch)?;
        let object = value
            .as_object()
            .ok_or(OpenAiProfileError::EffectiveRequestMismatch)?;
        if path != expected_path
            || object.get("model").and_then(Value::as_str) != Some(OPENAI_MODEL)
            || object.get("stream").and_then(Value::as_bool) != Some(true)
            || object
                .get("tools")
                .and_then(Value::as_array)
                .is_none_or(Vec::is_empty)
            || [
                "temperature",
                "top_p",
                "seed",
                "max_tokens",
                "max_completion_tokens",
                "max_output_tokens",
                "api_key",
                "authorization",
            ]
            .iter()
            .any(|field| object.contains_key(*field))
        {
            return Err(OpenAiProfileError::EffectiveRequestMismatch);
        }
        let reasoning_matches = match translation.api_mode {
            OpenAiApiMode::ChatCompletions => {
                object.get("reasoning_effort").and_then(Value::as_str) == Some("none")
                    && !object.contains_key("reasoning")
            }
            OpenAiApiMode::Responses => {
                object
                    .get("reasoning")
                    .and_then(Value::as_object)
                    .and_then(|reasoning| reasoning.get("effort"))
                    .and_then(Value::as_str)
                    == Some("none")
                    && !object.contains_key("reasoning_effort")
            }
        };
        if !reasoning_matches {
            return Err(OpenAiProfileError::EffectiveRequestMismatch);
        }
        Ok(VerifiedOpenAiRequest {
            agent,
            api_mode: translation.api_mode,
            profile_sha256: self.profile.settings_sha256.clone(),
        })
    }
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> OpenAiProfile {
        OpenAiProfile::new("a".repeat(64)).unwrap()
    }

    #[test]
    fn profile_is_public_secret_referenced_and_canonical() {
        let profile = profile();
        let public = profile.provider_profile();
        public.validate().unwrap();
        assert_eq!(public.provider, ProviderKind::OpenAi);
        assert_eq!(public.endpoint.class, EndpointClass::PublicService);
        assert_eq!(public.model, OPENAI_MODEL);
        assert_eq!(public.settings.reasoning_effort.as_deref(), Some("none"));
        assert_eq!(public.credential.source, CredentialSource::Environment);
        assert_eq!(public.credential.reference_sha256, Some("a".repeat(64)));
    }

    #[test]
    fn every_compatible_adapter_gets_the_same_profile() {
        let profile = profile();
        for agent in [
            OpenAiAgent::OpenCode,
            OpenAiAgent::OpenDesk,
            OpenAiAgent::Aider,
            OpenAiAgent::Codex,
            OpenAiAgent::QwenCode,
            OpenAiAgent::Goose,
            OpenAiAgent::MiniSwe,
            OpenAiAgent::OpenHands,
        ] {
            let route = profile
                .translate(agent, profile.provider_profile())
                .unwrap();
            assert_eq!(route.endpoint().as_str(), OPENAI_API_BASE);
            assert!(route.model().ends_with(OPENAI_MODEL));
        }
        assert_eq!(
            profile
                .translate(OpenAiAgent::Codex, profile.provider_profile())
                .unwrap()
                .api_mode(),
            OpenAiApiMode::Responses
        );
    }

    #[test]
    fn credential_profile_and_unsupported_route_fail_closed() {
        for digest in ["", "a", &"A".repeat(64), &"g".repeat(64)] {
            assert!(matches!(
                OpenAiProfile::new(digest),
                Err(OpenAiProfileError::InvalidCredentialReference)
            ));
        }
        let profile = profile();
        let mut changed = profile.provider_profile().clone();
        changed.model = "moving-alias".into();
        changed.refresh_settings_sha256().unwrap();
        assert!(matches!(
            profile.translate(OpenAiAgent::Aider, &changed),
            Err(OpenAiProfileError::ProfileMismatch)
        ));
        assert!(matches!(
            profile.translate(OpenAiAgent::Gemini, profile.provider_profile()),
            Err(OpenAiProfileError::UnsupportedAgent(OpenAiAgent::Gemini))
        ));
    }

    #[test]
    fn bounded_effective_requests_prove_routes_settings_tools_and_redaction() {
        let profile = profile();
        let chat = br#"{"model":"gpt-5.2-2025-12-11","stream":true,"reasoning_effort":"none","messages":[],"tools":[{"type":"function"}]}"#;
        let responses = br#"{"model":"gpt-5.2-2025-12-11","stream":true,"reasoning":{"effort":"none"},"input":[],"tools":[{"type":"function"}]}"#;
        let chat_proof = profile
            .verify_effective_request(
                OpenAiAgent::Aider,
                profile.provider_profile(),
                "/v1/chat/completions",
                "Bearer public-fixture-token",
                chat,
            )
            .unwrap();
        assert_eq!(chat_proof.agent(), OpenAiAgent::Aider);
        assert_eq!(chat_proof.api_mode(), OpenAiApiMode::ChatCompletions);
        assert_eq!(
            chat_proof.profile_sha256(),
            profile.provider_profile().settings_sha256
        );
        assert_eq!(
            profile
                .verify_effective_request(
                    OpenAiAgent::Codex,
                    profile.provider_profile(),
                    "/v1/responses",
                    "Bearer public-fixture-token",
                    responses,
                )
                .unwrap()
                .api_mode(),
            OpenAiApiMode::Responses
        );
    }

    #[test]
    fn malformed_lossy_or_secret_bearing_observations_are_rejected() {
        let profile = profile();
        let cases: [(&str, &str, &[u8]); 7] = [
            ("/v1/responses", "Bearer token", b"not-json"),
            ("/v1/responses", "", br#"{"model":"gpt-5.2-2025-12-11"}"#),
            ("/v1/chat/completions", "Bearer token", br#"{"model":"wrong","stream":true,"reasoning":{"effort":"none"},"tools":[{}]}"#),
            ("/v1/responses", "Bearer token", br#"{"model":"gpt-5.2-2025-12-11","stream":false,"reasoning":{"effort":"none"},"tools":[{}]}"#),
            ("/v1/responses", "Bearer token", br#"{"model":"gpt-5.2-2025-12-11","stream":true,"reasoning":{"effort":"none"},"tools":[]}"#),
            ("/v1/responses", "Bearer token", br#"{"model":"gpt-5.2-2025-12-11","stream":true,"reasoning":{"effort":"none"},"tools":[{}],"temperature":0}"#),
            ("/v1/responses", "Bearer token", br#"{"model":"gpt-5.2-2025-12-11","stream":true,"reasoning":{"effort":"none"},"tools":[{}],"api_key":"secret"}"#),
        ];
        for (path, authorization, body) in cases {
            assert!(matches!(
                profile.verify_effective_request(
                    OpenAiAgent::Codex,
                    profile.provider_profile(),
                    path,
                    authorization,
                    body,
                ),
                Err(OpenAiProfileError::EffectiveRequestMismatch)
            ));
        }
        assert!(matches!(
            profile.verify_effective_request(
                OpenAiAgent::Codex,
                profile.provider_profile(),
                "/v1/responses",
                "Bearer token with-space",
                &vec![b' '; MAX_OBSERVED_REQUEST_BYTES + 1],
            ),
            Err(OpenAiProfileError::EffectiveRequestMismatch)
        ));
    }
}
