// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Cross-agent provider and replay-source parity at credential-free boundaries.

use asb_agents::all_agents_provider::{
    AllAgentsProviderError, SelectedAgent, preflight_ollama_all, preflight_openai_all,
};
use asb_agents::ollama::{
    OLLAMA_CONTEXT_TOKENS, OLLAMA_MODEL, OLLAMA_MODEL_BYTES, OLLAMA_MODEL_SHA256, OllamaProfile,
};
use asb_agents::openai::{OPENAI_MODEL, OpenAiAgent, OpenAiApiMode, OpenAiProfile};
use asb_replay::{
    CassetteLimits, ExecutionSource, RecordingDescriptor, RecordingIndex, SourceChoice,
    SourceSelectionError, canonical_contents_bytes, decode_cassette,
};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::thread;
use url::Url;

const SUPPORTED: [SelectedAgent; 8] = [
    SelectedAgent::OpenCode,
    SelectedAgent::OpenDesk,
    SelectedAgent::Aider,
    SelectedAgent::Codex,
    SelectedAgent::QwenCode,
    SelectedAgent::Goose,
    SelectedAgent::MiniSwe,
    SelectedAgent::OpenHands,
];

fn agent_id(agent: SelectedAgent) -> &'static str {
    match agent {
        SelectedAgent::OpenCode => "opencode",
        SelectedAgent::OpenDesk => "opendesk",
        SelectedAgent::Aider => "aider",
        SelectedAgent::Codex => "codex",
        SelectedAgent::Gemini => "gemini",
        SelectedAgent::QwenCode => "qwen_code",
        SelectedAgent::Goose => "goose",
        SelectedAgent::MiniSwe => "mini_swe",
        SelectedAgent::OpenHands => "openhands",
    }
}

fn openai_agent(agent: SelectedAgent) -> OpenAiAgent {
    match agent {
        SelectedAgent::OpenCode => OpenAiAgent::OpenCode,
        SelectedAgent::OpenDesk => OpenAiAgent::OpenDesk,
        SelectedAgent::Aider => OpenAiAgent::Aider,
        SelectedAgent::Codex => OpenAiAgent::Codex,
        SelectedAgent::Gemini => OpenAiAgent::Gemini,
        SelectedAgent::QwenCode => OpenAiAgent::QwenCode,
        SelectedAgent::Goose => OpenAiAgent::Goose,
        SelectedAgent::MiniSwe => OpenAiAgent::MiniSwe,
        SelectedAgent::OpenHands => OpenAiAgent::OpenHands,
    }
}

fn ollama_tags() -> Vec<u8> {
    format!(
        r#"{{"models":[{{"name":"{OLLAMA_MODEL}","model":"{OLLAMA_MODEL}","digest":"{OLLAMA_MODEL_SHA256}","size":{OLLAMA_MODEL_BYTES},"details":{{"format":"gguf","family":"qwen3moe","quantization_level":"Q4_K_M","context_length":{OLLAMA_CONTEXT_TOKENS}}}}}]}}"#
    ).into_bytes()
}

#[test]
fn openai_effective_requests_preserve_one_profile_for_every_supported_agent() {
    let profile = OpenAiProfile::new("a".repeat(64)).unwrap();
    let plan = preflight_openai_all(&profile, &SUPPORTED).unwrap();
    assert_eq!(plan.effective().len(), SUPPORTED.len());
    assert!(
        plan.effective()
            .iter()
            .all(|item| item.profile_sha256 == plan.profile_sha256())
    );
    let chat = br#"{"model":"gpt-5.2-2025-12-11","stream":true,"reasoning_effort":"none","messages":[],"tools":[{"type":"function"}]}"#;
    let responses = br#"{"model":"gpt-5.2-2025-12-11","stream":true,"reasoning":{"effort":"none"},"input":[],"tools":[{"type":"function"}]}"#;
    for selected in SUPPORTED {
        let agent = openai_agent(selected);
        let route = profile
            .translate(agent, profile.provider_profile())
            .unwrap();
        let (path, body) = match route.api_mode() {
            OpenAiApiMode::ChatCompletions => ("/v1/chat/completions", chat.as_slice()),
            OpenAiApiMode::Responses => ("/v1/responses", responses.as_slice()),
        };
        let observed = profile
            .verify_effective_request(
                agent,
                profile.provider_profile(),
                path,
                "Bearer synthetic-fixture-token",
                body,
            )
            .unwrap();
        assert_eq!(observed.profile_sha256(), plan.profile_sha256());
        assert!(route.model().ends_with(OPENAI_MODEL));
    }
}

#[test]
fn ollama_probe_and_translation_preserve_one_profile_for_supported_agents() {
    let profile = OllamaProfile::new(Url::parse("http://127.0.0.1:11434/").unwrap()).unwrap();
    let verified = profile
        .verify_probe(br#"{"version":"0.33.1"}"#, &ollama_tags())
        .unwrap();
    let plan = preflight_ollama_all(&verified, &SUPPORTED).unwrap();
    assert_eq!(plan.effective().len(), SUPPORTED.len());
    assert!(
        plan.effective()
            .iter()
            .all(|item| item.profile_sha256 == plan.profile_sha256())
    );
}

#[test]
fn unsupported_route_fails_atomically_without_a_partial_parity_claim() {
    let profile = OpenAiProfile::new("a".repeat(64)).unwrap();
    let mut selected = SUPPORTED.to_vec();
    selected.push(SelectedAgent::Gemini);
    assert!(matches!(
        preflight_openai_all(&profile, &selected),
        Err(AllAgentsProviderError::Incompatible(issues))
            if issues.len() == 1 && issues[0].agent == SelectedAgent::Gemini
    ));
}

#[test]
fn replay_and_live_choices_stay_agent_and_profile_exact_in_parallel() {
    let profile = OpenAiProfile::new("a".repeat(64)).unwrap();
    let profile_id = profile.provider_profile().settings_sha256.clone();
    let mut index = RecordingIndex::new();
    let mut roots = Vec::new();
    for agent in SUPPORTED {
        let mut cassette = decode_cassette(
            include_bytes!("../../asb-replay/fixtures/v1/buffered.json"),
            CassetteLimits::default(),
        )
        .unwrap();
        cassette.contents.cassette_id = format!("parity-{}", agent_id(agent));
        cassette.integrity.digest = format!(
            "{:x}",
            Sha256::digest(canonical_contents_bytes(&cassette.contents).unwrap())
        );
        let root = cassette.integrity.digest.clone();
        index
            .insert(
                RecordingDescriptor {
                    provider_profile_sha256: profile_id.clone(),
                    agent_id: agent_id(agent).into(),
                    cassette_sha256: root.clone(),
                },
                &cassette,
            )
            .unwrap();
        roots.push((agent, root));
    }
    let index = Arc::new(index);
    let threads = roots
        .into_iter()
        .map(|(agent, root)| {
            let index = Arc::clone(&index);
            let profile_id = profile_id.clone();
            thread::spawn(move || {
                assert_eq!(
                    index
                        .select(
                            &profile_id,
                            agent_id(agent),
                            true,
                            Some(&SourceChoice::Live)
                        )
                        .unwrap(),
                    ExecutionSource::Live
                );
                assert!(matches!(
                    index.select(&profile_id, agent_id(agent), false, Some(&SourceChoice::Replay {
                        cassette_sha256: root.clone(),
                    })).unwrap(),
                    ExecutionSource::Replay { cassette_sha256, .. } if cassette_sha256 == root
                ));
                assert_eq!(
                    index.select(&profile_id, agent_id(agent), false, None),
                    Err(SourceSelectionError::ChoiceRequired)
                );
                assert_eq!(
                    index.select(
                        &profile_id,
                        agent_id(agent),
                        false,
                        Some(&SourceChoice::Live)
                    ),
                    Err(SourceSelectionError::LiveUnavailable)
                );
                assert_eq!(
                    index.select(
                        &"f".repeat(64),
                        agent_id(agent),
                        false,
                        Some(&SourceChoice::Replay {
                            cassette_sha256: root,
                        })
                    ),
                    Err(SourceSelectionError::RecordingUnavailable)
                );
            })
        })
        .collect::<Vec<_>>();
    for handle in threads {
        handle.join().unwrap();
    }
}

#[test]
fn corrupt_recording_is_rejected_and_network_denied_replay_never_falls_back() {
    let profile = OpenAiProfile::new("a".repeat(64)).unwrap();
    let profile_id = profile.provider_profile().settings_sha256.clone();
    let cassette = decode_cassette(
        include_bytes!("../../asb-replay/fixtures/v1/buffered.json"),
        CassetteLimits::default(),
    )
    .unwrap();
    let mut index = RecordingIndex::new();
    assert_eq!(
        index.insert(
            RecordingDescriptor {
                provider_profile_sha256: profile_id.clone(),
                agent_id: "codex".into(),
                cassette_sha256: "f".repeat(64),
            },
            &cassette,
        ),
        Err(SourceSelectionError::CassetteIdentityMismatch)
    );
    let root = cassette.integrity.digest.clone();
    index
        .insert(
            RecordingDescriptor {
                provider_profile_sha256: profile_id.clone(),
                agent_id: "codex".into(),
                cassette_sha256: root.clone(),
            },
            &cassette,
        )
        .unwrap();
    assert!(matches!(
        index
            .select(
                &profile_id,
                "codex",
                false,
                Some(&SourceChoice::Replay {
                    cassette_sha256: root,
                }),
            )
            .unwrap(),
        ExecutionSource::Replay { .. }
    ));
    assert_eq!(
        index.select(&profile_id, "codex", false, Some(&SourceChoice::Live)),
        Err(SourceSelectionError::LiveUnavailable)
    );
}
