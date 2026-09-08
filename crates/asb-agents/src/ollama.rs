// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Pinned loopback-only Ollama provider profile and adapter routing.

use asb_protocol::{
    CredentialProvenance, CredentialSource, EndpointClass, EndpointProvenance, PROVIDER_PROFILE_V1,
    ProviderKind, ProviderProfileError, ProviderProfileV1, ProviderSettings,
    ProviderTransportLimits,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::path::Path;
use std::time::Duration;
use url::Url;

/// Pinned Ollama server release exercised by this profile.
pub const OLLAMA_VERSION: &str = "0.33.1";
/// Immutable upstream commit for the exercised Ollama release tag.
pub const OLLAMA_UPSTREAM_REVISION: &str = "13f2fb8c99278469b954429d5541019f4d83a4d0";
/// Immutable upstream source tree for the exercised Ollama release tag.
pub const OLLAMA_UPSTREAM_TREE: &str = "b3e63b82943bb57e17d501fa137d1abae26ee1f6";
/// SHA-256 of the exercised Linux x86_64 Ollama executable.
pub const OLLAMA_LINUX_X86_64_SHA256: &str =
    "9f595107f966433f93f20ee19043f8e0cdea88e7403672f4dba2cadcb45ee085";
/// Exact local model name supplied to Ollama.
pub const OLLAMA_MODEL: &str = "qwen3-coder:30b";
/// Complete Ollama manifest digest for the pinned model.
pub const OLLAMA_MODEL_SHA256: &str =
    "06c1097efce0431c2045fe7b2e5108366e43bee1b4603a7aded8f21689e90bca";
/// Exact manifest size observed for the pinned model.
pub const OLLAMA_MODEL_BYTES: u64 = 18_556_700_761;
/// Context bound declared by the pinned model metadata.
pub const OLLAMA_CONTEXT_TOKENS: u64 = 262_144;

const MAX_PROBE_BYTES: u64 = 16 * 1024 * 1024;
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const OPENAI_PATH: &str = "/v1/";
const ADDITIONAL_SETTINGS: &str = concat!(
    "ollama-profile-v1\n",
    "server=0.33.1\n",
    "model_sha256=06c1097efce0431c2045fe7b2e5108366e43bee1b4603a7aded8f21689e90bca\n",
    "format=gguf\nfamily=qwen3moe\nquantization=Q4_K_M\n",
    "context_tokens=262144\napi=openai-compatible\n"
);

/// Built-in adapter whose Ollama route is being negotiated.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OllamaAgent {
    /// OpenCode custom-provider route.
    OpenCode,
    /// OpenDesk OpenAI-provider route.
    OpenDesk,
    /// aider OpenAI-provider route.
    Aider,
    /// Codex Responses API route.
    Codex,
    /// Gemini's native Google API route.
    Gemini,
    /// Qwen Code OpenAI-compatible route.
    QwenCode,
    /// Goose OpenAI-compatible route.
    Goose,
    /// mini-SWE-agent LiteLLM route.
    MiniSwe,
    /// OpenHands LiteLLM route.
    OpenHands,
}

/// Exact Ollama API surface used by a supported adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OllamaApiMode {
    /// OpenAI-compatible chat completions.
    ChatCompletions,
    /// Stateless OpenAI-compatible Responses API.
    Responses,
}

/// Adapter-specific values proven to preserve the pinned profile.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OllamaTranslation {
    endpoint: Url,
    model: String,
    api_mode: OllamaApiMode,
}

impl OllamaTranslation {
    /// Adapter endpoint, including the OpenAI-compatible `/v1/` base path.
    pub fn endpoint(&self) -> &Url {
        &self.endpoint
    }

    /// Model argument in the exact syntax expected by the adapter.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// OpenAI-compatible surface required by the adapter.
    pub const fn api_mode(&self) -> OllamaApiMode {
        self.api_mode
    }
}

/// Validated loopback provider profile, before daemon identity is proven.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OllamaProfile {
    root: Url,
    profile: ProviderProfileV1,
}

/// Constructor-controlled proof that the local daemon and model match the profile.
#[derive(Clone, Debug)]
pub struct VerifiedOllamaProfile {
    profile: OllamaProfile,
}

impl VerifiedOllamaProfile {
    /// Credential-free provider-profile evidence.
    pub fn provider_profile(&self) -> &ProviderProfileV1 {
        &self.profile.profile
    }

    /// Produce exact adapter values or reject the unsupported route before launch.
    pub fn translate(&self, agent: OllamaAgent) -> Result<OllamaTranslation, OllamaError> {
        self.profile.translate(agent, &self.profile.profile)
    }
}

/// Profile construction, local probing, or translation failure.
#[derive(Debug)]
pub enum OllamaError {
    /// Endpoint was not the exact local Ollama root.
    InvalidEndpoint,
    /// Local daemon could not be reached or returned malformed HTTP.
    ProbeIo(io::Error),
    /// Probe response exceeded its public bound or was malformed.
    InvalidProbe,
    /// Daemon release differs from the pinned release.
    ServerDrift,
    /// Pinned model is absent, duplicated, or has changed metadata.
    ModelDrift,
    /// Local server executable differs from the exercised artifact.
    BinaryDrift,
    /// Adapter does not expose a compatible provider boundary.
    UnsupportedAgent(OllamaAgent),
    /// Requested profile differs from the pinned profile.
    ProfileMismatch,
    /// Canonical provider-profile construction failed.
    Profile(ProviderProfileError),
}

impl fmt::Display for OllamaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidEndpoint => formatter.write_str("Ollama endpoint must be exact loopback"),
            Self::ProbeIo(error) => write!(formatter, "Ollama probe failed: {error}"),
            Self::InvalidProbe => formatter.write_str("invalid bounded Ollama probe response"),
            Self::ServerDrift => formatter.write_str("Ollama server version drift"),
            Self::ModelDrift => formatter.write_str("Ollama model identity drift"),
            Self::BinaryDrift => formatter.write_str("Ollama server executable drift"),
            Self::UnsupportedAgent(agent) => {
                write!(formatter, "unsupported Ollama route: {agent:?}")
            }
            Self::ProfileMismatch => formatter.write_str("Ollama provider profile mismatch"),
            Self::Profile(error) => write!(formatter, "invalid Ollama provider profile: {error}"),
        }
    }
}

impl std::error::Error for OllamaError {}

impl From<io::Error> for OllamaError {
    fn from(value: io::Error) -> Self {
        Self::ProbeIo(value)
    }
}

impl From<ProviderProfileError> for OllamaError {
    fn from(value: ProviderProfileError) -> Self {
        Self::Profile(value)
    }
}

impl OllamaProfile {
    /// Construct the pinned profile for the exact default local daemon root.
    pub fn new(root: Url) -> Result<Self, OllamaError> {
        if root.scheme() != "http"
            || root.username() != ""
            || root.password().is_some()
            || root.query().is_some()
            || root.fragment().is_some()
            || root.path() != "/"
            || root.port_or_known_default() != Some(11_434)
            || root
                .host_str()
                .and_then(|host| host.parse::<IpAddr>().ok())
                .is_none_or(|ip| !ip.is_loopback())
        {
            return Err(OllamaError::InvalidEndpoint);
        }
        let mut endpoint = Sha256::new();
        endpoint.update(root.as_str().as_bytes());
        let mut additional = Sha256::new();
        additional.update(ADDITIONAL_SETTINGS.as_bytes());
        let mut profile = ProviderProfileV1 {
            version: PROVIDER_PROFILE_V1,
            settings_sha256: String::new(),
            provider: ProviderKind::Ollama,
            endpoint: EndpointProvenance {
                class: EndpointClass::Loopback,
                identity_sha256: format!("{:x}", endpoint.finalize()),
            },
            model: OLLAMA_MODEL.into(),
            settings: ProviderSettings {
                temperature_milli: Some(700),
                top_p_millionth: Some(800_000),
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
                source: CredentialSource::None,
                reference_sha256: None,
            },
        };
        profile.refresh_settings_sha256()?;
        Ok(Self { root, profile })
    }

    /// Canonical credential-free profile before daemon verification.
    pub fn provider_profile(&self) -> &ProviderProfileV1 {
        &self.profile
    }

    /// Probe only the two read-only local endpoints needed for drift detection.
    pub fn probe(&self) -> Result<VerifiedOllamaProfile, OllamaError> {
        let version = self.get("/api/version")?;
        let tags = self.get("/api/tags")?;
        self.verify_probe(&version, &tags)
    }

    /// Verify an operator-supplied local server executable without following a symlink.
    pub fn verify_binary(&self, binary: &Path) -> Result<(), OllamaError> {
        let metadata = fs::symlink_metadata(binary)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(OllamaError::BinaryDrift);
        }
        let mut file = File::open(binary)?;
        let mut digest = Sha256::new();
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            digest.update(&buffer[..count]);
        }
        if format!("{:x}", digest.finalize()) == OLLAMA_LINUX_X86_64_SHA256 {
            Ok(())
        } else {
            Err(OllamaError::BinaryDrift)
        }
    }

    /// Verify already captured bounded `/api/version` and `/api/tags` bodies.
    pub fn verify_probe(
        &self,
        version_body: &[u8],
        tags_body: &[u8],
    ) -> Result<VerifiedOllamaProfile, OllamaError> {
        if version_body.len() as u64 > MAX_PROBE_BYTES || tags_body.len() as u64 > MAX_PROBE_BYTES {
            return Err(OllamaError::InvalidProbe);
        }
        let version: VersionResponse =
            serde_json::from_slice(version_body).map_err(|_| OllamaError::InvalidProbe)?;
        if version.version != OLLAMA_VERSION {
            return Err(OllamaError::ServerDrift);
        }
        let tags: TagsResponse =
            serde_json::from_slice(tags_body).map_err(|_| OllamaError::InvalidProbe)?;
        let mut matching = tags
            .models
            .iter()
            .filter(|model| model.name == OLLAMA_MODEL);
        let Some(model) = matching.next() else {
            return Err(OllamaError::ModelDrift);
        };
        if matching.next().is_some()
            || model.model != OLLAMA_MODEL
            || model.digest != OLLAMA_MODEL_SHA256
            || model.size != OLLAMA_MODEL_BYTES
            || model.details.format != "gguf"
            || model.details.family != "qwen3moe"
            || model.details.quantization_level != "Q4_K_M"
            || model.details.context_length != OLLAMA_CONTEXT_TOKENS
        {
            return Err(OllamaError::ModelDrift);
        }
        Ok(VerifiedOllamaProfile {
            profile: self.clone(),
        })
    }

    /// Translate only an exact copy of this pinned profile.
    fn translate(
        &self,
        agent: OllamaAgent,
        requested: &ProviderProfileV1,
    ) -> Result<OllamaTranslation, OllamaError> {
        requested.validate()?;
        if requested != &self.profile {
            return Err(OllamaError::ProfileMismatch);
        }
        let (api_mode, model) = match agent {
            OllamaAgent::OpenCode => (
                OllamaApiMode::ChatCompletions,
                format!("ollama/{OLLAMA_MODEL}"),
            ),
            OllamaAgent::Codex => (OllamaApiMode::Responses, OLLAMA_MODEL.into()),
            OllamaAgent::OpenDesk
            | OllamaAgent::Aider
            | OllamaAgent::QwenCode
            | OllamaAgent::Goose
            | OllamaAgent::MiniSwe
            | OllamaAgent::OpenHands => (OllamaApiMode::ChatCompletions, OLLAMA_MODEL.into()),
            OllamaAgent::Gemini => return Err(OllamaError::UnsupportedAgent(agent)),
        };
        let endpoint = self
            .root
            .join(OPENAI_PATH)
            .map_err(|_| OllamaError::InvalidEndpoint)?;
        Ok(OllamaTranslation {
            endpoint,
            model,
            api_mode,
        })
    }

    fn get(&self, path: &str) -> Result<Vec<u8>, OllamaError> {
        let ip = self
            .root
            .host_str()
            .and_then(|host| host.parse::<IpAddr>().ok())
            .ok_or(OllamaError::InvalidEndpoint)?;
        let address = SocketAddr::new(ip, 11_434);
        let mut stream = TcpStream::connect_timeout(&address, PROBE_TIMEOUT)?;
        stream.set_read_timeout(Some(PROBE_TIMEOUT))?;
        stream.set_write_timeout(Some(PROBE_TIMEOUT))?;
        write!(
            stream,
            "GET {path} HTTP/1.0\r\nHost: {}\r\nConnection: close\r\n\r\n",
            self.root.host_str().ok_or(OllamaError::InvalidEndpoint)?
        )?;
        let mut response = Vec::new();
        stream
            .take(MAX_PROBE_BYTES + 4_096 + 1)
            .read_to_end(&mut response)?;
        if response.len() as u64 > MAX_PROBE_BYTES + 4_096 {
            return Err(OllamaError::InvalidProbe);
        }
        let boundary = response
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .ok_or(OllamaError::InvalidProbe)?;
        let header = &response[..boundary];
        let first_line = header
            .split(|byte| *byte == b'\n')
            .next()
            .unwrap_or_default();
        if first_line != b"HTTP/1.0 200 OK\r" && first_line != b"HTTP/1.1 200 OK\r" {
            return Err(OllamaError::InvalidProbe);
        }
        Ok(response[(boundary + 4)..].to_vec())
    }
}

#[derive(Deserialize)]
struct VersionResponse {
    version: String,
}

#[derive(Deserialize)]
struct TagsResponse {
    models: Vec<Model>,
}

#[derive(Deserialize)]
struct Model {
    name: String,
    model: String,
    digest: String,
    size: u64,
    details: ModelDetails,
}

#[derive(Deserialize)]
struct ModelDetails {
    format: String,
    family: String,
    quantization_level: String,
    context_length: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> OllamaProfile {
        OllamaProfile::new(Url::parse("http://127.0.0.1:11434/").unwrap()).unwrap()
    }

    fn tags(digest: &str) -> Vec<u8> {
        format!(
            r#"{{"models":[{{"name":"{OLLAMA_MODEL}","model":"{OLLAMA_MODEL}","digest":"{digest}","size":{OLLAMA_MODEL_BYTES},"details":{{"format":"gguf","family":"qwen3moe","quantization_level":"Q4_K_M","context_length":{OLLAMA_CONTEXT_TOKENS}}}}}]}}"#
        )
        .into_bytes()
    }

    #[test]
    fn profile_is_loopback_credential_free_and_canonical() {
        let profile = profile();
        let public = profile.provider_profile();
        public.validate().unwrap();
        assert_eq!(public.provider, ProviderKind::Ollama);
        assert_eq!(public.endpoint.class, EndpointClass::Loopback);
        assert_eq!(public.credential.source, CredentialSource::None);
        assert_eq!(public.model, OLLAMA_MODEL);
        assert_eq!(public.settings.temperature_milli, Some(700));
        assert_eq!(public.settings.top_p_millionth, Some(800_000));
    }

    #[test]
    fn endpoint_and_profile_drift_fail_before_translation() {
        for endpoint in [
            "http://localhost:11434/",
            "http://192.0.2.1:11434/",
            "https://127.0.0.1:11434/",
            "http://127.0.0.1:11435/",
            "http://127.0.0.1:11434/v1/",
            "http://user@127.0.0.1:11434/",
        ] {
            assert!(matches!(
                OllamaProfile::new(Url::parse(endpoint).unwrap()),
                Err(OllamaError::InvalidEndpoint)
            ));
        }
        let profile = profile();
        let mut changed = profile.provider_profile().clone();
        changed.model = "qwen3-coder:changed".into();
        changed.refresh_settings_sha256().unwrap();
        assert!(matches!(
            profile.translate(OllamaAgent::Aider, &changed),
            Err(OllamaError::ProfileMismatch)
        ));
    }

    #[test]
    fn exact_probe_enables_only_documented_routes() {
        let profile = profile();
        let verified = profile
            .verify_probe(br#"{"version":"0.33.1"}"#, &tags(OLLAMA_MODEL_SHA256))
            .unwrap();
        for agent in [
            OllamaAgent::OpenCode,
            OllamaAgent::OpenDesk,
            OllamaAgent::Aider,
            OllamaAgent::Codex,
            OllamaAgent::QwenCode,
            OllamaAgent::Goose,
            OllamaAgent::MiniSwe,
            OllamaAgent::OpenHands,
        ] {
            let route = verified.translate(agent).unwrap();
            assert_eq!(route.endpoint().as_str(), "http://127.0.0.1:11434/v1/");
            assert!(route.model().contains(OLLAMA_MODEL));
        }
        assert_eq!(
            verified.translate(OllamaAgent::Codex).unwrap().api_mode(),
            OllamaApiMode::Responses
        );
        assert!(matches!(
            verified.translate(OllamaAgent::Gemini),
            Err(OllamaError::UnsupportedAgent(OllamaAgent::Gemini))
        ));
    }

    #[test]
    fn malformed_and_drifted_probe_evidence_is_rejected() {
        let profile = profile();
        assert!(matches!(
            profile.verify_probe(br#"{"version":"0.33.0"}"#, &tags(OLLAMA_MODEL_SHA256)),
            Err(OllamaError::ServerDrift)
        ));
        assert!(matches!(
            profile.verify_probe(br#"{"version":"0.33.1"}"#, &tags(&"a".repeat(64))),
            Err(OllamaError::ModelDrift)
        ));
        assert!(matches!(
            profile.verify_probe(b"not-json", &tags(OLLAMA_MODEL_SHA256)),
            Err(OllamaError::InvalidProbe)
        ));
        assert!(matches!(
            profile.verify_probe(br#"{"version":"0.33.1"}"#, br#"{"models":[]}"#),
            Err(OllamaError::ModelDrift)
        ));
    }

    #[test]
    #[ignore = "requires the exact preloaded Ollama 0.33.1 qwen3-coder:30b daemon"]
    fn native_read_only_probe_matches_pins() {
        let profile = profile();
        profile
            .verify_binary(Path::new("/usr/local/bin/ollama"))
            .unwrap();
        let verified = profile.probe().unwrap();
        assert_eq!(verified.provider_profile().model, OLLAMA_MODEL);
    }
}
