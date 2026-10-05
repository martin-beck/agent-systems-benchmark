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
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};
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
/// Maximum response body retained by the live capture boundary.
pub const MAX_LIVE_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

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

/// Bounded response returned by the explicit online capture transport.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenRouterLiveResponse {
    /// HTTP status returned by OpenRouter.
    pub status: u16,
    /// Complete bounded response bytes, before cassette redaction.
    pub body: Vec<u8>,
}

impl OpenRouterLiveResponse {
    /// Whether the provider returned a successful HTTP status.
    pub const fn is_success(&self) -> bool {
        self.status >= 200 && self.status <= 299
    }

    /// Stable response size for capture metadata and bounded analysis.
    pub const fn body_len(&self) -> usize {
        self.body.len()
    }

    /// Coarse status class for human-readable analysis without retaining content.
    pub const fn status_class(&self) -> u16 {
        self.status / 100
    }

    /// Whether the provider rejected the request with a client error.
    pub const fn is_client_error(&self) -> bool {
        self.status_class() == 4
    }

    /// Whether the provider failed while handling the request.
    pub const fn is_server_error(&self) -> bool {
        self.status_class() == 5
    }

    /// Whether no provider response bytes were retained.
    pub const fn body_is_empty(&self) -> bool {
        self.body.is_empty()
    }

    /// Whether the status represents a redirect that must not be followed silently.
    pub const fn is_redirect(&self) -> bool {
        self.status_class() == 3
    }

    /// Whether the provider reported rate limiting.
    pub const fn is_rate_limited(&self) -> bool {
        self.status == 429
    }

    /// Compact, content-free summary suitable for human-readable comparison output.
    pub fn analysis_summary(&self) -> String {
        let outcome = if self.is_success() {
            "success"
        } else if self.is_rate_limited() {
            "rate_limited"
        } else if self.is_server_error() {
            "server_error"
        } else {
            "provider_error"
        };
        format!(
            "status={} class={} bytes={} outcome={outcome}",
            self.status,
            self.status_class(),
            self.body_len()
        )
    }

    /// Structured content-free fields consumed by comparison/reporting layers.
    pub fn analysis_fields(&self) -> serde_json::Value {
        serde_json::json!({
            "status": self.status,
            "status_class": self.status_class(),
            "body_bytes": self.body_len(),
            "success": self.is_success(),
            "client_error": self.is_client_error(),
            "server_error": self.is_server_error(),
            "rate_limited": self.is_rate_limited(),
        })
    }

    /// Stable tuple used by lightweight result comparators.
    pub const fn comparison_key(&self) -> (u16, usize, bool) {
        (self.status, self.body.len(), self.is_success())
    }

    /// Human-readable labels used by the TUI analysis projection.
    pub fn analysis_labels(&self) -> Vec<&'static str> {
        let mut labels = Vec::with_capacity(4);
        labels.push(if self.is_success() {
            "success"
        } else {
            "failure"
        });
        if self.is_client_error() {
            labels.push("client_error");
        }
        if self.is_server_error() {
            labels.push("server_error");
        }
        if self.is_rate_limited() {
            labels.push("rate_limited");
        }
        if self.is_redirect() {
            labels.push("redirect");
        }
        if self.body_is_empty() {
            labels.push("empty_body");
        } else {
            labels.push("body_present");
        }
        labels
    }

    /// Ordered scalar columns for table-oriented comparison views.
    pub fn analysis_columns(&self) -> Vec<(&'static str, String)> {
        vec![
            ("status", self.status.to_string()),
            ("status_class", self.status_class().to_string()),
            ("body_bytes", self.body_len().to_string()),
            ("success", self.is_success().to_string()),
            ("client_error", self.is_client_error().to_string()),
            ("server_error", self.is_server_error().to_string()),
            ("rate_limited", self.is_rate_limited().to_string()),
            ("redirect", self.is_redirect().to_string()),
            ("body_empty", self.body_is_empty().to_string()),
            ("summary", self.analysis_summary()),
            ("label_count", self.analysis_labels().len().to_string()),
            ("comparison_status", self.comparison_key().0.to_string()),
            ("comparison_bytes", self.comparison_key().1.to_string()),
            ("comparison_success", self.comparison_key().2.to_string()),
        ]
    }
}

/// Typed failure from the online OpenRouter capture boundary.
#[derive(Debug)]
pub enum OpenRouterLiveError {
    /// Runtime credential lookup failed.
    Credential(CredentialResolutionError),
    /// The request exceeded the pinned profile bounds.
    RequestTooLarge,
    /// The request model is not the model selected by the connected profile.
    ModelMismatch,
    /// The local curl transport could not be started or completed.
    Transport,
    /// OpenRouter returned a body larger than the replay bound.
    ResponseTooLarge,
    /// The transport did not provide a valid HTTP status.
    InvalidStatus,
    /// OpenRouter returned a non-success HTTP status; no cassette may be sealed.
    HttpStatus(u16),
    /// The approved curl executable could not be discovered on PATH.
    CurlUnavailable,
}

impl OpenRouterLiveError {
    /// Whether retrying the same request could reasonably succeed.
    pub const fn is_retryable(&self) -> bool {
        matches!(self, Self::Transport | Self::ResponseTooLarge)
    }
}

impl fmt::Display for OpenRouterLiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Credential(_) => f.write_str("OpenRouter credential unavailable"),
            Self::RequestTooLarge => f.write_str("OpenRouter request exceeds its bound"),
            Self::ModelMismatch => {
                f.write_str("OpenRouter request model does not match the selected provider model")
            }
            Self::Transport => f.write_str("OpenRouter transport failed"),
            Self::ResponseTooLarge => f.write_str("OpenRouter response exceeds its bound"),
            Self::InvalidStatus => f.write_str("OpenRouter returned an invalid HTTP status"),
            Self::HttpStatus(status) => write!(f, "OpenRouter returned HTTP status {status}"),
            Self::CurlUnavailable => f.write_str("OpenRouter curl transport is unavailable"),
        }
    }
}

impl std::error::Error for OpenRouterLiveError {}

/// Send one explicitly opted-in request to OpenRouter and retain its bounded response.
///
/// The environment credential is resolved only here. It is passed to curl through a
/// private config file and erased with the temporary file before returning; it never
/// enters request metadata, cassette content, or diagnostics.
pub fn capture_openrouter_live(
    profile: &OpenRouterProfile,
    agent: OpenRouterAgent,
    request_body: &[u8],
) -> Result<OpenRouterLiveResponse, OpenRouterLiveError> {
    capture_openrouter_live_with_resolver(
        profile,
        agent,
        request_body,
        |profile| resolve_openrouter_environment(profile).map_err(OpenRouterLiveError::Credential),
        openrouter_curl_transport,
    )
}

fn capture_openrouter_live_with_resolver<R, F>(
    profile: &OpenRouterProfile,
    agent: OpenRouterAgent,
    request_body: &[u8],
    resolve: R,
    transport: F,
) -> Result<OpenRouterLiveResponse, OpenRouterLiveError>
where
    R: FnOnce(&OpenRouterProfile) -> Result<ResolvedCredential, OpenRouterLiveError>,
    F: FnOnce(&std::path::Path, &[u8]) -> Result<(bool, Vec<u8>), OpenRouterLiveError>,
{
    profile
        .translate(agent, profile.provider_profile())
        .map_err(|_| OpenRouterLiveError::Transport)?;
    if request_body.is_empty()
        || request_body.len() > profile.provider_profile().transport.max_request_bytes as usize
    {
        return Err(OpenRouterLiveError::RequestTooLarge);
    }
    let expected_model = profile
        .translate(agent, profile.provider_profile())
        .map_err(|_| OpenRouterLiveError::Transport)?
        .model;
    let requested_model = serde_json::from_slice::<Value>(request_body)
        .ok()
        .and_then(|value| {
            value
                .get("model")
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
    if requested_model.as_deref() != Some(expected_model.as_str()) {
        return Err(OpenRouterLiveError::ModelMismatch);
    }
    let credential = resolve(profile)?;
    capture_openrouter_live_with_credential(profile, agent, request_body, credential, transport)
}

fn openrouter_curl_transport(
    config_path: &std::path::Path,
    body: &[u8],
) -> Result<(bool, Vec<u8>), OpenRouterLiveError> {
    let result = Command::new("curl")
        .args(["--silent", "--show-error", "--config"])
        .arg(config_path)
        .args(["--data-binary", "@-", "--write-out", "\n%{http_code}"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // Curl diagnostics are intentionally discarded at this boundary. The
        // typed transport/status error is the only durable diagnostic and
        // provider responses must never leak into the ASB parent's stderr.
        .stderr(Stdio::null())
        .spawn()
        .and_then(|mut child| {
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(body)?;
            }
            child.wait_with_output()
        });
    result
        .map(|output| (output.status.success(), output.stdout))
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                OpenRouterLiveError::CurlUnavailable
            } else {
                OpenRouterLiveError::Transport
            }
        })
}

/// Quote one curl-config value using curl's double-quoted string grammar.
///
/// Credentials are validated as printable bytes at resolution time, but the
/// escaping is still required: curl otherwise treats an unquoted header value
/// as a config token and can silently omit the authorization header.
fn curl_config_quote(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for byte in value.bytes() {
        match byte {
            b'\\' | b'"' => {
                quoted.push('\\');
                quoted.push(byte as char);
            }
            _ => quoted.push(byte as char),
        }
    }
    quoted.push('"');
    quoted
}

fn capture_openrouter_live_with_credential<F>(
    profile: &OpenRouterProfile,
    agent: OpenRouterAgent,
    request_body: &[u8],
    credential: ResolvedCredential,
    transport: F,
) -> Result<OpenRouterLiveResponse, OpenRouterLiveError>
where
    F: FnOnce(&std::path::Path, &[u8]) -> Result<(bool, Vec<u8>), OpenRouterLiveError>,
{
    let translation = profile
        .translate(agent, profile.provider_profile())
        .map_err(|_| OpenRouterLiveError::Transport)?;
    if request_body.is_empty()
        || request_body.len() > profile.provider_profile().transport.max_request_bytes as usize
    {
        return Err(OpenRouterLiveError::RequestTooLarge);
    }
    let mut credential = credential.into_transport_bytes();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| OpenRouterLiveError::Transport)?
        .as_nanos();
    let config_path = std::env::temp_dir().join(format!("asb-openrouter-{nonce}.conf"));
    let config = format!(
        "url = {}\nrequest = POST\nheader = {}\nheader = {}\nmax-time = 1800\nconnect-timeout = 5\nmax-filesize = {}\n",
        curl_config_quote(&format!(
            "{}{}",
            translation.endpoint(),
            match translation.api_mode {
                OpenRouterApiMode::Responses => "/responses",
                OpenRouterApiMode::ChatCompletions => "/chat/completions",
            }
        )),
        curl_config_quote(&format!(
            "Authorization: Bearer {}",
            String::from_utf8_lossy(&credential)
        )),
        curl_config_quote("Content-Type: application/json"),
        MAX_LIVE_RESPONSE_BYTES
    );
    write_openrouter_config(&config_path, config.as_bytes())?;
    let result = transport(&config_path, request_body);
    let _ = std::fs::remove_file(&config_path);
    credential.fill(0);
    let (success, stdout) = result?;
    if !success || stdout.len() > MAX_LIVE_RESPONSE_BYTES + 8 {
        return Err(if stdout.len() > MAX_LIVE_RESPONSE_BYTES + 8 {
            OpenRouterLiveError::ResponseTooLarge
        } else {
            OpenRouterLiveError::Transport
        });
    }
    let marker = stdout
        .iter()
        .rposition(|byte| *byte == b'\n')
        .ok_or(OpenRouterLiveError::InvalidStatus)?;
    let status = std::str::from_utf8(&stdout[marker + 1..])
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .ok_or(OpenRouterLiveError::InvalidStatus)?;
    let body = stdout[..marker].to_vec();
    if body.len() > MAX_LIVE_RESPONSE_BYTES {
        return Err(OpenRouterLiveError::ResponseTooLarge);
    }
    if !(200..=299).contains(&status) {
        return Err(OpenRouterLiveError::HttpStatus(status));
    }
    Ok(OpenRouterLiveResponse { status, body })
}

fn write_openrouter_config(
    config_path: &std::path::Path,
    config: &[u8],
) -> Result<(), OpenRouterLiveError> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(config_path)
        .map_err(|_| OpenRouterLiveError::Transport)?;
    file.write_all(config)
        .map_err(|_| OpenRouterLiveError::Transport)?;
    drop(file);
    Ok(())
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
    fn live_capture_rejects_invalid_route_or_request_before_credential_lookup() {
        let profile = profile();
        assert!(matches!(
            capture_openrouter_live(&profile, OpenRouterAgent::Gemini, b"{}"),
            Err(OpenRouterLiveError::Transport)
        ));
        assert!(matches!(
            capture_openrouter_live(&profile, OpenRouterAgent::Aider, &[]),
            Err(OpenRouterLiveError::RequestTooLarge)
        ));
        let oversized =
            vec![b'x'; profile.provider_profile().transport.max_request_bytes as usize + 1];
        assert!(matches!(
            capture_openrouter_live(&profile, OpenRouterAgent::Aider, &oversized),
            Err(OpenRouterLiveError::RequestTooLarge)
        ));
    }

    #[test]
    fn live_negative_matrix_rejects_unavailable_model_and_malformed_credential() {
        let profile = profile();
        let unavailable_model = capture_openrouter_live_with_resolver(
            &profile,
            OpenRouterAgent::Aider,
            br#"{"model":"provider/removed-model"}"#,
            |_profile| panic!("credential lookup must not run for an unavailable model"),
            |_config, _body| panic!("transport must not run for an unavailable model"),
        );
        assert!(matches!(
            unavailable_model,
            Err(OpenRouterLiveError::ModelMismatch)
        ));

        let malformed_credential = capture_openrouter_live_with_resolver(
            &profile,
            OpenRouterAgent::Aider,
            br#"{"model":"cohere/north-mini-code:free"}"#,
            |_profile| {
                Err(OpenRouterLiveError::Credential(
                    CredentialResolutionError::InvalidValue,
                ))
            },
            |_config, _body| panic!("transport must not run for malformed credentials"),
        );
        assert!(matches!(
            malformed_credential,
            Err(OpenRouterLiveError::Credential(
                CredentialResolutionError::InvalidValue
            ))
        ));
    }

    #[test]
    fn live_capture_transport_boundary_preserves_bounded_response_and_errors() {
        let profile = profile();
        let credential = || {
            resolve_openrouter_credential(&profile, |_| Some(OsString::from("synthetic-key")))
                .unwrap()
        };
        let response = capture_openrouter_live_with_credential(
            &profile,
            OpenRouterAgent::Aider,
            br#"{"model":"cohere/north-mini-code:free"}"#,
            credential(),
            |config, body| {
                assert_eq!(body, br#"{"model":"cohere/north-mini-code:free"}"#);
                assert!(
                    std::fs::read_to_string(config)
                        .unwrap()
                        .contains("Authorization: Bearer synthetic-key")
                );
                Ok((true, b"{\"id\":\"captured\"}\n200".to_vec()))
            },
        )
        .unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.body, br#"{"id":"captured"}"#);
        assert!(response.is_success());
        assert_eq!(response.body_len(), 17);
        assert_eq!(response.status_class(), 2);
        assert!(!response.body_is_empty());
        assert!(!response.is_redirect());
        assert!(!response.is_rate_limited());
        assert_eq!(
            response.analysis_summary(),
            "status=200 class=2 bytes=17 outcome=success"
        );
        assert_eq!(response.analysis_fields()["body_bytes"], 17);
        assert_eq!(response.analysis_fields()["success"], true);
        assert_eq!(response.comparison_key(), (200, 17, true));
        assert_eq!(response.analysis_labels(), vec!["success", "body_present"]);
        assert_eq!(response.analysis_columns().len(), 14);
        let failure_response = OpenRouterLiveResponse {
            status: 503,
            body: Vec::new(),
        };
        assert!(!failure_response.is_success());
        assert_eq!(failure_response.body_len(), 0);
        assert_eq!(failure_response.status_class(), 5);
        assert!(!failure_response.is_client_error());
        assert!(failure_response.is_server_error());
        let client_response = OpenRouterLiveResponse {
            status: 429,
            body: Vec::new(),
        };
        assert!(client_response.is_client_error());
        assert!(!client_response.is_server_error());
        assert!(client_response.body_is_empty());
        assert!(!client_response.is_redirect());
        assert!(client_response.is_rate_limited());
        assert_eq!(
            client_response.analysis_summary(),
            "status=429 class=4 bytes=0 outcome=rate_limited"
        );
        assert_eq!(
            client_response.analysis_labels(),
            vec!["failure", "client_error", "rate_limited", "empty_body"]
        );
        let redirect_response = OpenRouterLiveResponse {
            status: 302,
            body: Vec::new(),
        };
        assert!(redirect_response.is_redirect());
        assert_eq!(
            redirect_response.analysis_summary(),
            "status=302 class=3 bytes=0 outcome=provider_error"
        );
        assert_eq!(
            failure_response.analysis_summary(),
            "status=503 class=5 bytes=0 outcome=server_error"
        );
        assert_eq!(
            failure_response.analysis_labels(),
            vec!["failure", "server_error", "empty_body"]
        );
        assert_eq!(
            redirect_response.analysis_labels(),
            vec!["failure", "redirect", "empty_body"]
        );

        let resolved_wrapper = capture_openrouter_live_with_resolver(
            &profile,
            OpenRouterAgent::Aider,
            br#"{"model":"cohere/north-mini-code:free"}"#,
            |_profile| Ok(credential()),
            |_config, body| Ok((true, [body, b"\n201"].concat())),
        )
        .unwrap();
        assert_eq!(resolved_wrapper.status, 201);

        let responses_wrapper = capture_openrouter_live_with_resolver(
            &profile,
            OpenRouterAgent::Codex,
            br#"{"model":"cohere/north-mini-code:free"}"#,
            |_profile| Ok(credential()),
            |config, _body| {
                assert!(
                    std::fs::read_to_string(config)
                        .unwrap()
                        .contains("/responses")
                );
                Ok((true, b"{}\n200".to_vec()))
            },
        )
        .unwrap();
        assert_eq!(responses_wrapper.status, 200);

        let transport_error = capture_openrouter_live_with_credential(
            &profile,
            OpenRouterAgent::Aider,
            b"{}",
            credential(),
            |_config, _body| Ok((false, b"provider failed".to_vec())),
        );
        assert!(matches!(
            transport_error,
            Err(OpenRouterLiveError::Transport)
        ));
        let provider_error = capture_openrouter_live_with_credential(
            &profile,
            OpenRouterAgent::Aider,
            br#"{"model":"cohere/north-mini-code:free"}"#,
            credential(),
            |_config, _body| {
                Ok((
                    true,
                    br#"{"error":"rate limited"}
429"#
                        .to_vec(),
                ))
            },
        );
        assert!(matches!(
            provider_error,
            Err(OpenRouterLiveError::HttpStatus(429))
        ));
        assert!(OpenRouterLiveError::Transport.is_retryable());
        assert!(OpenRouterLiveError::ResponseTooLarge.is_retryable());
        assert!(!OpenRouterLiveError::RequestTooLarge.is_retryable());
        assert!(!OpenRouterLiveError::InvalidStatus.is_retryable());
        assert!(!OpenRouterLiveError::HttpStatus(429).is_retryable());

        let invalid_status = capture_openrouter_live_with_credential(
            &profile,
            OpenRouterAgent::Aider,
            b"{}",
            credential(),
            |_config, _body| Ok((true, b"{}\nnot-status".to_vec())),
        );
        assert!(matches!(
            invalid_status,
            Err(OpenRouterLiveError::InvalidStatus)
        ));

        let resolver_error = capture_openrouter_live_with_resolver(
            &profile,
            OpenRouterAgent::Aider,
            br#"{"model":"cohere/north-mini-code:free"}"#,
            |_profile| {
                Err(OpenRouterLiveError::Credential(
                    CredentialResolutionError::Unavailable,
                ))
            },
            |_config, _body| panic!("transport must not run when resolution fails"),
        );
        assert!(matches!(
            resolver_error,
            Err(OpenRouterLiveError::Credential(_))
        ));

        let transport_result_error = capture_openrouter_live_with_credential(
            &profile,
            OpenRouterAgent::Aider,
            b"{}",
            credential(),
            |_config, _body| Err(OpenRouterLiveError::Transport),
        );
        assert!(matches!(
            transport_result_error,
            Err(OpenRouterLiveError::Transport)
        ));

        let invalid_numeric_status = capture_openrouter_live_with_credential(
            &profile,
            OpenRouterAgent::Aider,
            b"{}",
            credential(),
            |_config, _body| Ok((true, b"{}\n99999".to_vec())),
        );
        assert!(matches!(
            invalid_numeric_status,
            Err(OpenRouterLiveError::InvalidStatus)
        ));

        let invalid_route = capture_openrouter_live_with_credential(
            &profile,
            OpenRouterAgent::Gemini,
            b"{}",
            credential(),
            |_config, _body| panic!("transport must not run for unsupported route"),
        );
        assert!(matches!(invalid_route, Err(OpenRouterLiveError::Transport)));

        let empty_request = capture_openrouter_live_with_credential(
            &profile,
            OpenRouterAgent::Aider,
            b"",
            credential(),
            |_config, _body| panic!("transport must not run for empty request"),
        );
        assert!(matches!(
            empty_request,
            Err(OpenRouterLiveError::RequestTooLarge)
        ));

        let missing_marker = capture_openrouter_live_with_credential(
            &profile,
            OpenRouterAgent::Aider,
            b"{}",
            credential(),
            |_config, _body| Ok((true, b"no status marker".to_vec())),
        );
        assert!(matches!(
            missing_marker,
            Err(OpenRouterLiveError::InvalidStatus)
        ));

        let invalid_utf8_status = capture_openrouter_live_with_credential(
            &profile,
            OpenRouterAgent::Aider,
            b"{}",
            credential(),
            |_config, _body| Ok((true, b"{}\n\xff".to_vec())),
        );
        assert!(matches!(
            invalid_utf8_status,
            Err(OpenRouterLiveError::InvalidStatus)
        ));

        let oversized_response = capture_openrouter_live_with_credential(
            &profile,
            OpenRouterAgent::Aider,
            b"{}",
            credential(),
            |_config, _body| Ok((true, vec![b'x'; MAX_LIVE_RESPONSE_BYTES + 9])),
        );
        assert!(matches!(
            oversized_response,
            Err(OpenRouterLiveError::ResponseTooLarge)
        ));

        let oversized_body = capture_openrouter_live_with_credential(
            &profile,
            OpenRouterAgent::Aider,
            b"{}",
            credential(),
            |_config, _body| {
                let mut output = vec![b'x'; MAX_LIVE_RESPONSE_BYTES + 1];
                output.extend_from_slice(b"\n200");
                Ok((true, output))
            },
        );
        assert!(matches!(
            oversized_body,
            Err(OpenRouterLiveError::ResponseTooLarge)
        ));
    }

    #[test]
    fn curl_config_quotes_headers_and_transport_parses_them() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        assert_eq!(
            curl_config_quote("Authorization: Bearer synthetic\"\\key"),
            "\"Authorization: Bearer synthetic\\\"\\\\key\""
        );
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 1024];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let read = stream.read(&mut buffer).unwrap();
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
            }
            let request = String::from_utf8(request).unwrap();
            assert!(request.contains("Authorization: Bearer synthetic-key\r\n"));
            assert!(request.contains("Content-Type: application/json\r\n"));
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .unwrap();
        });
        let path = std::env::temp_dir().join(format!(
            "asb-openrouter-curl-config-test-{}",
            std::process::id()
        ));
        let config = format!(
            "url = {}\nrequest = POST\nheader = {}\nheader = {}\n",
            curl_config_quote(&format!("http://{address}/")),
            curl_config_quote("Authorization: Bearer synthetic-key"),
            curl_config_quote("Content-Type: application/json")
        );
        write_openrouter_config(&path, config.as_bytes()).unwrap();
        let result = openrouter_curl_transport(&path, br#"{}"#).unwrap();
        let _ = std::fs::remove_file(&path);
        server.join().unwrap();
        assert!(result.0);
        assert_eq!(result.1, b"ok\n200");
    }

    #[test]
    fn live_capture_without_environment_credential_is_typed_and_fail_closed() {
        let profile = profile();
        let result = capture_openrouter_live(
            &profile,
            OpenRouterAgent::Aider,
            br#"{"model":"cohere/north-mini-code:free"}"#,
        );
        assert!(matches!(result, Err(OpenRouterLiveError::Credential(_))));
    }

    #[test]
    fn live_error_display_and_curl_boundary_are_stable() {
        let errors = [
            OpenRouterLiveError::Credential(CredentialResolutionError::Unavailable),
            OpenRouterLiveError::RequestTooLarge,
            OpenRouterLiveError::ModelMismatch,
            OpenRouterLiveError::Transport,
            OpenRouterLiveError::ResponseTooLarge,
            OpenRouterLiveError::InvalidStatus,
            OpenRouterLiveError::HttpStatus(503),
            OpenRouterLiveError::CurlUnavailable,
        ];
        for error in errors {
            assert!(!error.to_string().is_empty());
        }
        let result = openrouter_curl_transport(
            std::path::Path::new("/definitely/missing/openrouter-config"),
            b"{}",
        );
        assert!(matches!(
            result,
            Ok((false, _)) | Err(OpenRouterLiveError::Transport)
        ));
        let directory_path =
            std::env::temp_dir().join(format!("asb-openrouter-config-dir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory_path);
        std::fs::create_dir(&directory_path).unwrap();
        assert!(matches!(
            write_openrouter_config(&directory_path, b"config"),
            Err(OpenRouterLiveError::Transport)
        ));
        std::fs::remove_dir(&directory_path).unwrap();
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
