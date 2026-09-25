// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Pinned credential-free OpenRouter provider profile and fail-closed adapter translations.
//!
//! OpenRouter exposes an OpenAI-compatible API. The profile pins one public
//! endpoint identity, a dated model snapshot instead of a moving alias, explicit
//! transport bounds, and an [`OPENROUTER_API_KEY`](OPENROUTER_API_KEY_ENV)
//! environment credential reference (AR-0318). No credential value or locator is
//! represented here; secrets resolve from the process environment at run time only.

use crate::credential::{
    CredentialResolutionError, EnvironmentCredentialResolver, ResolvedCredential,
};
use asb_protocol::{
    CredentialProvenance, CredentialSource, EndpointClass, EndpointProvenance, PROVIDER_PROFILE_V1,
    ProviderKind, ProviderProfileError, ProviderProfileV1, ProviderSettings,
    ProviderTransportLimits,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::ffi::{OsStr, OsString};
use std::fmt;
use url::Url;

const MAX_OBSERVED_REQUEST_BYTES: usize = 4 * 1024 * 1024;
const MAX_AUTHORIZATION_BYTES: usize = 8 * 1024;

/// Exact public API base accepted by the built-in profile.
pub const OPENROUTER_API_BASE: &str = "https://openrouter.ai/api/v1";
/// Exact free-model identifier transmitted to OpenRouter.
pub const OPENROUTER_MODEL: &str = "cohere/north-mini-code:free";
/// Date on which the [`OPENROUTER_MODEL`] snapshot was pinned.
pub const OPENROUTER_MODEL_SNAPSHOT_DATE: &str = "2026-09-25";
/// Dated snapshot identity recorded as immutable evidence instead of a moving alias.
pub const OPENROUTER_MODEL_SNAPSHOT: &str = "cohere/north-mini-code:free@2026-09-25";
/// Process-environment credential reference for the OpenRouter API key.
pub const OPENROUTER_API_KEY_ENV: &str = "OPENROUTER_API_KEY";

/// Credential-free provider-specific pinning evidence.
const OPENROUTER_ADDITIONAL_SETTINGS: &str = concat!(
    "openrouter-profile-v1\n",
    "model_snapshot=cohere/north-mini-code:free@2026-09-25\n",
    "api=openai-compatible\n",
    "sampling=profile-defaults\n",
);

/// Built-in adapter whose OpenRouter route is being negotiated.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpenRouterAgent {
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

/// Exact OpenRouter API surface selected for an adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpenRouterApiMode {
    /// OpenAI-compatible Chat Completions.
    ChatCompletions,
    /// OpenAI-compatible Responses API.
    Responses,
}

/// Credential injection boundary used by an adapter; never contains a secret value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpenRouterCredentialTarget {
    /// Standard `OPENROUTER_API_KEY` process environment boundary.
    OpenRouterApiKey,
}

impl OpenRouterCredentialTarget {
    /// Process-environment variable name owned by this target.
    pub const fn environment_variable(self) -> &'static str {
        match self {
            Self::OpenRouterApiKey => OPENROUTER_API_KEY_ENV,
        }
    }
}

/// Adapter-specific values derived from one exact profile.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenRouterTranslation {
    endpoint: Url,
    model: String,
    api_mode: OpenRouterApiMode,
    credential_target: OpenRouterCredentialTarget,
}

/// Constructor-controlled validation result for a synthetic-service observation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedOpenRouterRequest {
    agent: OpenRouterAgent,
    api_mode: OpenRouterApiMode,
    profile_sha256: String,
}

impl VerifiedOpenRouterRequest {
    /// Adapter that emitted the request.
    pub const fn agent(&self) -> OpenRouterAgent {
        self.agent
    }

    /// API surface observed by the synthetic service.
    pub const fn api_mode(&self) -> OpenRouterApiMode {
        self.api_mode
    }

    /// Credential-free profile identity; prompts and authorization are never retained or hashed.
    pub fn profile_sha256(&self) -> &str {
        &self.profile_sha256
    }
}

impl OpenRouterTranslation {
    /// Exact public provider base.
    pub fn endpoint(&self) -> &Url {
        &self.endpoint
    }

    /// Exact model spelling expected by the adapter.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// API surface selected for the adapter.
    pub const fn api_mode(&self) -> OpenRouterApiMode {
        self.api_mode
    }

    /// Credential boundary; this never contains a secret value.
    pub const fn credential_target(&self) -> OpenRouterCredentialTarget {
        self.credential_target
    }
}

/// Exact default public OpenRouter profile.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenRouterProfile {
    profile: ProviderProfileV1,
}

/// Profile construction or adapter translation failure.
#[derive(Debug)]
pub enum OpenRouterProfileError {
    /// Credential reference was absent or malformed.
    InvalidCredentialReference,
    /// Adapter has no proven public OpenRouter boundary.
    UnsupportedAgent(OpenRouterAgent),
    /// Requested profile differs from the pinned profile.
    ProfileMismatch,
    /// Synthetic service observed malformed or semantically different request data.
    EffectiveRequestMismatch,
    /// Common provider-profile validation failed.
    Profile(ProviderProfileError),
}

impl fmt::Display for OpenRouterProfileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCredentialReference => {
                formatter.write_str("invalid OpenRouter credential reference digest")
            }
            Self::UnsupportedAgent(agent) => {
                write!(
                    formatter,
                    "adapter has no proven public OpenRouter route: {agent:?}"
                )
            }
            Self::ProfileMismatch => formatter.write_str("OpenRouter provider profile mismatch"),
            Self::EffectiveRequestMismatch => {
                formatter.write_str("effective OpenRouter request does not match the profile")
            }
            Self::Profile(error) => {
                write!(formatter, "invalid OpenRouter provider profile: {error}")
            }
        }
    }
}

impl std::error::Error for OpenRouterProfileError {}

impl From<ProviderProfileError> for OpenRouterProfileError {
    fn from(value: ProviderProfileError) -> Self {
        Self::Profile(value)
    }
}

impl OpenRouterProfile {
    /// Construct the default profile from a redacted logical secret-reference digest.
    pub fn new(
        credential_reference_sha256: impl Into<String>,
    ) -> Result<Self, OpenRouterProfileError> {
        let credential_reference_sha256 = credential_reference_sha256.into();
        if !is_sha256(&credential_reference_sha256) {
            return Err(OpenRouterProfileError::InvalidCredentialReference);
        }
        let endpoint = Url::parse(OPENROUTER_API_BASE).expect("constant public API URL");
        let mut endpoint_digest = Sha256::new();
        endpoint_digest.update(endpoint.as_str().as_bytes());
        let mut additional = Sha256::new();
        additional.update(OPENROUTER_ADDITIONAL_SETTINGS.as_bytes());
        let mut profile = ProviderProfileV1 {
            version: PROVIDER_PROFILE_V1,
            settings_sha256: String::new(),
            provider: ProviderKind::OpenAiCompatible,
            endpoint: EndpointProvenance {
                class: EndpointClass::PublicService,
                identity_sha256: format!("{:x}", endpoint_digest.finalize()),
            },
            model: OPENROUTER_MODEL.into(),
            settings: ProviderSettings {
                temperature_milli: None,
                top_p_millionth: None,
                seed: None,
                max_output_tokens: None,
                reasoning_effort: None,
                additional_settings_sha256: Some(format!("{:x}", additional.finalize())),
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
        agent: OpenRouterAgent,
        requested: &ProviderProfileV1,
    ) -> Result<OpenRouterTranslation, OpenRouterProfileError> {
        requested.validate()?;
        if requested != &self.profile {
            return Err(OpenRouterProfileError::ProfileMismatch);
        }
        let (api_mode, model) = match agent {
            OpenRouterAgent::Codex => (OpenRouterApiMode::Responses, OPENROUTER_MODEL.into()),
            OpenRouterAgent::OpenCode
            | OpenRouterAgent::OpenDesk
            | OpenRouterAgent::Aider
            | OpenRouterAgent::QwenCode
            | OpenRouterAgent::Goose
            | OpenRouterAgent::MiniSwe
            | OpenRouterAgent::OpenHands => {
                (OpenRouterApiMode::ChatCompletions, OPENROUTER_MODEL.into())
            }
            OpenRouterAgent::Gemini => return Err(OpenRouterProfileError::UnsupportedAgent(agent)),
        };
        Ok(OpenRouterTranslation {
            endpoint: Url::parse(OPENROUTER_API_BASE).expect("constant public API URL"),
            model,
            api_mode,
            credential_target: OpenRouterCredentialTarget::OpenRouterApiKey,
        })
    }

    /// Verify a bounded synthetic-service observation without retaining its secret or prompt.
    pub fn verify_effective_request(
        &self,
        agent: OpenRouterAgent,
        requested: &ProviderProfileV1,
        path: &str,
        authorization: &str,
        body: &[u8],
    ) -> Result<VerifiedOpenRouterRequest, OpenRouterProfileError> {
        let translation = self.translate(agent, requested)?;
        if body.is_empty()
            || body.len() > MAX_OBSERVED_REQUEST_BYTES
            || authorization.len() > MAX_AUTHORIZATION_BYTES
            || !authorization.strip_prefix("Bearer ").is_some_and(|token| {
                !token.is_empty() && !token.bytes().any(|byte| byte.is_ascii_whitespace())
            })
        {
            return Err(OpenRouterProfileError::EffectiveRequestMismatch);
        }
        let expected_path = match translation.api_mode {
            OpenRouterApiMode::ChatCompletions => "/api/v1/chat/completions",
            OpenRouterApiMode::Responses => "/api/v1/responses",
        };
        let value: Value = serde_json::from_slice(body)
            .map_err(|_| OpenRouterProfileError::EffectiveRequestMismatch)?;
        let object = value
            .as_object()
            .ok_or(OpenRouterProfileError::EffectiveRequestMismatch)?;
        if path != expected_path
            || object.get("model").and_then(Value::as_str) != Some(OPENROUTER_MODEL)
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
                "reasoning_effort",
                "reasoning",
                "api_key",
                "authorization",
                "openai_api_key",
                "credential",
            ]
            .iter()
            .any(|field| object.contains_key(*field))
        {
            return Err(OpenRouterProfileError::EffectiveRequestMismatch);
        }
        Ok(VerifiedOpenRouterRequest {
            agent,
            api_mode: translation.api_mode,
            profile_sha256: self.profile.settings_sha256.clone(),
        })
    }
}

/// Compute the canonical credential-free digest of the `OPENROUTER_API_KEY` reference.
pub fn openrouter_credential_reference() -> Result<String, OpenRouterProfileError> {
    EnvironmentCredentialResolver::new(OPENROUTER_API_KEY_ENV)
        .map(|resolver| resolver.reference_sha256().to_owned())
        .map_err(|_| OpenRouterProfileError::InvalidCredentialReference)
}

/// Resolve the OpenRouter API key only through an injected lookup boundary.
///
/// The secret never enters the profile or any returned metadata; callers that
/// need the value must consume the opaque [`ResolvedCredential`] at the final
/// transport boundary.
pub fn resolve_openrouter_credential(
    profile: &OpenRouterProfile,
    lookup: impl FnMut(&OsStr) -> Option<OsString>,
) -> Result<ResolvedCredential, CredentialResolutionError> {
    let resolver = EnvironmentCredentialResolver::new(OPENROUTER_API_KEY_ENV)
        .map_err(|_| CredentialResolutionError::InvalidEnvironmentName)?;
    resolver.resolve_with(profile.provider_profile(), lookup)
}

/// Resolve the OpenRouter API key from the current process environment.
///
/// Real provider contact is user opt-in at run time; offline callers must use
/// [`resolve_openrouter_credential`] with a synthetic lookup instead.
pub fn resolve_openrouter_environment(
    profile: &OpenRouterProfile,
) -> Result<ResolvedCredential, CredentialResolutionError> {
    let resolver = EnvironmentCredentialResolver::new(OPENROUTER_API_KEY_ENV)
        .map_err(|_| CredentialResolutionError::InvalidEnvironmentName)?;
    resolver.resolve_environment(profile.provider_profile())
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
    use asb_protocol::{CredentialSource, EndpointClass, ProviderKind};

    fn profile() -> OpenRouterProfile {
        OpenRouterProfile::new(openrouter_credential_reference().unwrap()).unwrap()
    }

    #[test]
    fn profile_is_public_secret_referenced_and_canonical() {
        let profile = profile();
        let public = profile.provider_profile();
        public.validate().unwrap();
        assert_eq!(public.provider, ProviderKind::OpenAiCompatible);
        assert_eq!(public.endpoint.class, EndpointClass::PublicService);
        assert_eq!(public.model, OPENROUTER_MODEL);
        assert_eq!(public.settings.temperature_milli, None);
        assert_eq!(public.settings.top_p_millionth, None);
        assert_eq!(public.settings.seed, None);
        assert_eq!(public.settings.max_output_tokens, None);
        assert_eq!(public.settings.reasoning_effort, None);
        assert!(public.settings.additional_settings_sha256.is_some());
        assert_eq!(public.credential.source, CredentialSource::Environment);
        assert_eq!(
            public.credential.reference_sha256,
            openrouter_credential_reference().ok()
        );
        assert!(
            !serde_json::to_string(public)
                .unwrap()
                .contains(OPENROUTER_API_KEY_ENV)
        );
    }

    #[test]
    fn endpoint_identity_cannot_be_confused_with_public_openai() {
        let openrouter = profile();
        let openrouter_identity = &openrouter.provider_profile().endpoint.identity_sha256;
        let openai_profile = crate::openai::OpenAiProfile::new("a".repeat(64)).unwrap();
        let openai_identity = &openai_profile.provider_profile().endpoint.identity_sha256;
        assert_eq!(
            openrouter_identity,
            &format!("{:x}", Sha256::digest(OPENROUTER_API_BASE.as_bytes()))
        );
        assert_ne!(openrouter_identity, openai_identity);
    }

    #[test]
    fn every_compatible_adapter_gets_the_same_profile() {
        let profile = profile();
        for agent in [
            OpenRouterAgent::OpenCode,
            OpenRouterAgent::OpenDesk,
            OpenRouterAgent::Aider,
            OpenRouterAgent::Codex,
            OpenRouterAgent::QwenCode,
            OpenRouterAgent::Goose,
            OpenRouterAgent::MiniSwe,
            OpenRouterAgent::OpenHands,
        ] {
            let route = profile
                .translate(agent, profile.provider_profile())
                .unwrap();
            assert_eq!(route.endpoint().as_str(), OPENROUTER_API_BASE);
            assert_eq!(route.model(), OPENROUTER_MODEL);
            assert_eq!(
                route.credential_target().environment_variable(),
                OPENROUTER_API_KEY_ENV
            );
        }
        assert_eq!(
            profile
                .translate(OpenRouterAgent::Codex, profile.provider_profile())
                .unwrap()
                .api_mode(),
            OpenRouterApiMode::Responses
        );
        assert!(matches!(
            profile.translate(OpenRouterAgent::Gemini, profile.provider_profile()),
            Err(OpenRouterProfileError::UnsupportedAgent(
                OpenRouterAgent::Gemini
            ))
        ));
    }

    #[test]
    fn credential_profile_and_unsupported_route_fail_closed() {
        for digest in ["", "a", &"A".repeat(64), &"g".repeat(64)] {
            assert!(matches!(
                OpenRouterProfile::new(digest),
                Err(OpenRouterProfileError::InvalidCredentialReference)
            ));
        }
        let profile = profile();
        let mut changed = profile.provider_profile().clone();
        changed.model = "moving-alias".into();
        changed.refresh_settings_sha256().unwrap();
        assert!(matches!(
            profile.translate(OpenRouterAgent::Aider, &changed),
            Err(OpenRouterProfileError::ProfileMismatch)
        ));
        let mut openai_route = profile.provider_profile().clone();
        openai_route.provider = ProviderKind::OpenAi;
        openai_route.refresh_settings_sha256().unwrap();
        assert!(matches!(
            profile.translate(OpenRouterAgent::Aider, &openai_route),
            Err(OpenRouterProfileError::ProfileMismatch)
        ));
    }

    #[test]
    fn bounded_effective_requests_prove_routes_settings_tools_and_redaction() {
        let profile = profile();
        let chat = br#"{"model":"cohere/north-mini-code:free","stream":true,"messages":[],"tools":[{"type":"function"}]}"#;
        let responses = br#"{"model":"cohere/north-mini-code:free","stream":true,"input":[],"tools":[{"type":"function"}]}"#;
        let chat_proof = profile
            .verify_effective_request(
                OpenRouterAgent::Aider,
                profile.provider_profile(),
                "/api/v1/chat/completions",
                "Bearer public-fixture-token",
                chat,
            )
            .unwrap();
        assert_eq!(chat_proof.agent(), OpenRouterAgent::Aider);
        assert_eq!(chat_proof.api_mode(), OpenRouterApiMode::ChatCompletions);
        assert_eq!(
            chat_proof.profile_sha256(),
            profile.provider_profile().settings_sha256
        );
        assert_eq!(
            profile
                .verify_effective_request(
                    OpenRouterAgent::Codex,
                    profile.provider_profile(),
                    "/api/v1/responses",
                    "Bearer public-fixture-token",
                    responses,
                )
                .unwrap()
                .api_mode(),
            OpenRouterApiMode::Responses
        );
    }

    #[test]
    fn malformed_lossy_or_secret_bearing_observations_are_rejected() {
        let profile = profile();
        let cases: [(&str, &str, &[u8]); 9] = [
            ("/api/v1/responses", "Bearer token", b"not-json"),
            ("/api/v1/responses", "", br#"{"model":"cohere/north-mini-code:free"}"#),
            ("/v1/chat/completions", "Bearer token", br#"{"model":"cohere/north-mini-code:free","stream":true,"tools":[{}]}"#),
            ("/api/v1/chat/completions", "Bearer token", br#"{"model":"wrong","stream":true,"tools":[{}]}"#),
            ("/api/v1/responses", "Bearer token", br#"{"model":"cohere/north-mini-code:free","stream":false,"tools":[{}]}"#),
            ("/api/v1/responses", "Bearer token", br#"{"model":"cohere/north-mini-code:free","stream":true,"tools":[]}"#),
            ("/api/v1/responses", "Bearer token", br#"{"model":"cohere/north-mini-code:free","stream":true,"tools":[{}],"temperature":0}"#),
            ("/api/v1/responses", "Bearer token", br#"{"model":"cohere/north-mini-code:free","stream":true,"tools":[{}],"reasoning_effort":"low"}"#),
            ("/api/v1/responses", "Bearer token", br#"{"model":"cohere/north-mini-code:free","stream":true,"tools":[{}],"api_key":"secret"}"#),
        ];
        for (path, authorization, body) in cases {
            assert!(matches!(
                profile.verify_effective_request(
                    OpenRouterAgent::Codex,
                    profile.provider_profile(),
                    path,
                    authorization,
                    body,
                ),
                Err(OpenRouterProfileError::EffectiveRequestMismatch)
            ));
        }
        assert!(matches!(
            profile.verify_effective_request(
                OpenRouterAgent::Codex,
                profile.provider_profile(),
                "/api/v1/responses",
                "Bearer token with-space",
                &vec![b' '; MAX_OBSERVED_REQUEST_BYTES + 1],
            ),
            Err(OpenRouterProfileError::EffectiveRequestMismatch)
        ));
    }

    #[test]
    fn absent_and_mismatched_credentials_fail_closed_without_lookup() {
        let profile = profile();
        assert!(matches!(
            resolve_openrouter_credential(&profile, |_| None),
            Err(CredentialResolutionError::Unavailable)
        ));
        let mut calls = 0;
        let wrong = OpenRouterProfile::new("b".repeat(64)).unwrap();
        assert!(matches!(
            resolve_openrouter_credential(&wrong, |_| {
                calls += 1;
                None
            }),
            Err(CredentialResolutionError::ReferenceMismatch)
        ));
        assert_eq!(calls, 0);
    }

    #[test]
    fn reference_digest_matches_the_environment_resolver_boundary() {
        let resolver = EnvironmentCredentialResolver::new(OPENROUTER_API_KEY_ENV).unwrap();
        assert_eq!(
            openrouter_credential_reference().unwrap(),
            resolver.reference_sha256()
        );
        let profile = profile();
        let resolved = resolve_openrouter_credential(&profile, |name| {
            assert_eq!(name, OsStr::new(OPENROUTER_API_KEY_ENV));
            Some(OsString::from("synthetic-value"))
        })
        .unwrap();
        assert_eq!(resolved.into_transport_bytes(), b"synthetic-value");
    }
}
