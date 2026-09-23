// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
#![allow(missing_docs)]

//! Credential-free hostile qualification for the pinned OpenRouter profile.
//!
//! These tests are deliberately transport-independent.  They exercise the
//! same request boundary used by the loopback fixture and validate the small
//! response shapes that a provider adapter is allowed to accept.  No provider
//! hostname, key, prompt, or response content is contacted or persisted.

use asb_agents::openrouter::{
    OPENROUTER_MODEL, OpenRouterAgent, OpenRouterApiMode, OpenRouterProfile,
    openrouter_credential_reference,
};
use asb_replay::{RecordingIndex, SourceChoice, SourceSelectionError};
use serde_json::{Value, json};
use std::collections::BTreeSet;

const FIXTURE_AUTHORIZATION: &str = "Bearer synthetic-openrouter-key";

const AGENTS: [OpenRouterAgent; 8] = [
    OpenRouterAgent::OpenCode,
    OpenRouterAgent::OpenDesk,
    OpenRouterAgent::Aider,
    OpenRouterAgent::Codex,
    OpenRouterAgent::QwenCode,
    OpenRouterAgent::Goose,
    OpenRouterAgent::MiniSwe,
    OpenRouterAgent::OpenHands,
];

#[derive(Debug, Eq, PartialEq)]
enum ResponseFailure {
    HttpStatus(u16),
    InvalidJson,
    WrongModel,
    MissingChoices,
    StreamNotTerminated,
    ToolDivergence,
}

fn profile() -> OpenRouterProfile {
    OpenRouterProfile::new(openrouter_credential_reference().unwrap()).unwrap()
}

fn request(agent: OpenRouterAgent) -> (&'static str, Value) {
    let route = profile()
        .translate(agent, profile().provider_profile())
        .unwrap();
    match route.api_mode() {
        OpenRouterApiMode::ChatCompletions => (
            "/api/v1/chat/completions",
            json!({
                "model": OPENROUTER_MODEL,
                "stream": true,
                "messages": [],
                "tools": [{"type": "function", "function": {"name": "fixture", "parameters": {"type": "object"}}}]
            }),
        ),
        OpenRouterApiMode::Responses => (
            "/api/v1/responses",
            json!({
                "model": OPENROUTER_MODEL,
                "stream": true,
                "input": [],
                "tools": [{"type": "function", "function": {"name": "fixture", "parameters": {"type": "object"}}}]
            }),
        ),
    }
}

fn verify_request(agent: OpenRouterAgent, path: &str, body: &Value) -> bool {
    let profile = profile();
    profile
        .verify_effective_request(
            agent,
            profile.provider_profile(),
            path,
            FIXTURE_AUTHORIZATION,
            &serde_json::to_vec(body).unwrap(),
        )
        .is_ok()
}

fn verify_chat_response(body: &[u8], expected_model: &str) -> Result<(), ResponseFailure> {
    let value: Value = serde_json::from_slice(body).map_err(|_| ResponseFailure::InvalidJson)?;
    if value.get("model").and_then(Value::as_str) != Some(expected_model) {
        return Err(ResponseFailure::WrongModel);
    }
    if value.get("choices").and_then(Value::as_array).is_none() {
        return Err(ResponseFailure::MissingChoices);
    }
    if value
        .get("choices")
        .and_then(Value::as_array)
        .is_some_and(|choices| {
            choices.iter().any(|choice| {
                choice
                    .get("message")
                    .and_then(Value::as_object)
                    .and_then(|message| message.get("tool_calls"))
                    .is_some_and(|calls| !calls.is_array())
            })
        })
    {
        return Err(ResponseFailure::ToolDivergence);
    }
    let serialized = serde_json::to_string(&value).unwrap();
    if serialized.contains("OPENROUTER_API_KEY") || serialized.contains("synthetic-openrouter-key")
    {
        return Err(ResponseFailure::ToolDivergence);
    }
    Ok(())
}

fn verify_sse(events: &[&str], expected_model: &str) -> Result<(), ResponseFailure> {
    if events.last().copied() != Some("data: [DONE]") {
        return Err(ResponseFailure::StreamNotTerminated);
    }
    let mut saw_model = false;
    let mut tool_indexes = BTreeSet::new();
    for event in events.iter().take(events.len().saturating_sub(1)) {
        let payload = event
            .strip_prefix("data: ")
            .ok_or(ResponseFailure::InvalidJson)?;
        let value: Value =
            serde_json::from_str(payload).map_err(|_| ResponseFailure::InvalidJson)?;
        if value.get("model").and_then(Value::as_str) == Some(expected_model) {
            saw_model = true;
        }
        if let Some(index) = value.get("tool_call_index").and_then(Value::as_u64)
            && !tool_indexes.insert(index)
        {
            return Err(ResponseFailure::ToolDivergence);
        }
    }
    if !saw_model {
        return Err(ResponseFailure::WrongModel);
    }
    Ok(())
}

fn verify_http_status(status: u16) -> Result<(), ResponseFailure> {
    if (200..300).contains(&status) {
        Ok(())
    } else {
        Err(ResponseFailure::HttpStatus(status))
    }
}

#[test]
fn pinned_profile_conforms_identically_for_every_compatible_agent() {
    let profile = profile();
    let digest = profile.provider_profile().settings_sha256.clone();
    for agent in AGENTS {
        let (path, body) = request(agent);
        assert!(verify_request(agent, path, &body));
        let route = profile
            .translate(agent, profile.provider_profile())
            .unwrap();
        assert_eq!(route.model(), OPENROUTER_MODEL);
        assert_eq!(route.endpoint().as_str(), "https://openrouter.ai/api/v1");
        assert_eq!(profile.provider_profile().settings_sha256, digest);
    }
}

#[test]
fn credential_bleed_and_endpoint_identity_fail_closed() {
    let (path, mut body) = request(OpenRouterAgent::Aider);
    for field in ["api_key", "authorization", "openai_api_key", "credential"] {
        body[field] = Value::String("secret-must-not-cross-boundary".into());
        assert!(
            !verify_request(OpenRouterAgent::Aider, path, &body),
            "field {field}"
        );
        body.as_object_mut().unwrap().remove(field);
    }
    assert!(!verify_request(
        OpenRouterAgent::Aider,
        "/v1/chat/completions",
        &body
    ));
    assert!(!verify_request(
        OpenRouterAgent::Aider,
        "https://api.openai.com/v1/chat/completions",
        &body
    ));
}

#[test]
fn response_hostiles_fail_closed_without_secret_or_uncertain_effect() {
    let valid = json!({
        "id": "fixture", "object": "chat.completion", "model": OPENROUTER_MODEL,
        "choices": [{"index": 0, "message": {"role": "assistant", "content": "fixture"}, "finish_reason": "stop"}]
    });
    assert_eq!(
        verify_chat_response(b"not-json", OPENROUTER_MODEL),
        Err(ResponseFailure::InvalidJson)
    );
    let mut wrong_model = valid.clone();
    wrong_model["model"] = Value::String("moving-alias".into());
    assert_eq!(
        verify_chat_response(&serde_json::to_vec(&wrong_model).unwrap(), OPENROUTER_MODEL),
        Err(ResponseFailure::WrongModel)
    );
    let mut missing_choices = valid.clone();
    missing_choices.as_object_mut().unwrap().remove("choices");
    assert_eq!(
        verify_chat_response(
            &serde_json::to_vec(&missing_choices).unwrap(),
            OPENROUTER_MODEL
        ),
        Err(ResponseFailure::MissingChoices)
    );
    assert_eq!(
        verify_chat_response(&serde_json::to_vec(&valid).unwrap(), OPENROUTER_MODEL),
        Ok(())
    );
    assert_eq!(
        verify_sse(
            &["data: {\"model\":\"deepseek/deepseek-chat-v3-0324:free\"}"],
            OPENROUTER_MODEL
        ),
        Err(ResponseFailure::StreamNotTerminated)
    );
    assert_eq!(
        verify_sse(
            &[
                "data: {\"model\":\"deepseek/deepseek-chat-v3-0324:free\"}",
                "data: [DONE]"
            ],
            OPENROUTER_MODEL
        ),
        Ok(())
    );
    assert_eq!(
        verify_sse(
            &[
                "data: {\"model\":\"deepseek/deepseek-chat-v3-0324:free\",\"tool_call_index\":0}",
                "data: {\"model\":\"deepseek/deepseek-chat-v3-0324:free\",\"tool_call_index\":0}",
                "data: [DONE]"
            ],
            OPENROUTER_MODEL
        ),
        Err(ResponseFailure::ToolDivergence)
    );
    assert_eq!(
        verify_http_status(429),
        Err(ResponseFailure::HttpStatus(429))
    );
    assert_eq!(
        verify_http_status(503),
        Err(ResponseFailure::HttpStatus(503))
    );

    let index = RecordingIndex::new();
    let profile_digest = "a".repeat(64);
    let cassette_digest = "b".repeat(64);
    assert_eq!(
        index.select(&profile_digest, "aider", false, Some(&SourceChoice::Live)),
        Err(SourceSelectionError::LiveUnavailable)
    );
    assert_eq!(
        index.select(
            &profile_digest,
            "aider",
            false,
            Some(&SourceChoice::Replay {
                cassette_sha256: cassette_digest,
            }),
        ),
        Err(SourceSelectionError::RecordingUnavailable)
    );
    assert_eq!(
        index.select(&profile_digest, "aider", false, None),
        Err(SourceSelectionError::ChoiceRequired)
    );
}
