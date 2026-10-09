// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Built-in coding-agent adapters.

pub mod accounting;
pub mod aider;
pub mod all_agents_provider;
pub mod auth;
pub mod auth_backend;
pub mod authenticated_request;
pub mod cli2key;
pub mod codex;
pub mod credential;
pub mod development_fixture;
pub mod gemini;
pub mod goose;
pub mod launch_bridge;
pub mod mini_swe;
pub mod ollama;
pub mod openai;
pub mod opencode;
pub mod opendesk;
pub mod openhands;
pub mod openjiuwen;
pub mod openrouter;
pub mod production_auth;
pub mod provider;
pub mod provider_launch;
pub mod qwen_code;
pub mod strict_replay;

pub use development_fixture::{
    DEVELOPMENT_FIXTURE_SCHEMA_VERSION, DEVELOPMENT_FIXTURE_SEED, DevelopmentCaptureRequest,
    DevelopmentCaptureResult, DevelopmentCredentialReceipt, DevelopmentFixtureError,
    DevelopmentFixtureState, DevelopmentProviderFixture, DevelopmentProviderSelection,
    DevelopmentReadiness,
};
