// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Integration coverage for the provider-aware launch binding.

use asb_agents::all_agents_provider::EffectiveApiMode;
use asb_agents::provider_launch::{
    CredentialResolverIdentity, CredentialResolverKind, LaunchPolicy, PROVIDER_LAUNCH_V1,
    ProviderLaunchProjection, ProviderLaunchRecord, ProviderLaunchV1, RuntimeBundleIdentity,
};

fn input() -> ProviderLaunchV1 {
    ProviderLaunchV1 {
        schema_version: PROVIDER_LAUNCH_V1,
        catalog_sha256: "a".repeat(64),
        selection_sha256: "b".repeat(64),
        provider_profile_sha256: "c".repeat(64),
        agent: "codex".into(),
        adapter: "codex".into(),
        api_mode: EffectiveApiMode::Responses,
        provider: "openai".into(),
        model: "pinned-model".into(),
        settings_sha256: "d".repeat(64),
        runtime: RuntimeBundleIdentity {
            bundle_sha256: "e".repeat(64),
            executable_sha256: "f".repeat(64),
        },
        credential: CredentialResolverIdentity {
            kind: CredentialResolverKind::Environment,
            reference_sha256: "1".repeat(64),
        },
        workload_sha256: "2".repeat(64),
        run_id: "run-1".into(),
        attempt_id: "run-1-attempt".into(),
        policy: LaunchPolicy {
            max_stdout_bytes: 1,
            max_stderr_bytes: 1,
            timeout_ms: 1,
            max_environment_entries: 1,
            max_argv_entries: 1,
        },
    }
}

#[test]
fn launch_record_is_canonical_and_detects_tampering() {
    let value = input();
    let projection = ProviderLaunchProjection {
        agent: value.agent.clone(),
        adapter: value.adapter.clone(),
        provider: value.provider.clone(),
        model: value.model.clone(),
        api_mode: value.api_mode,
        settings_sha256: value.settings_sha256.clone(),
        credential: value.credential.clone(),
    };
    let record = ProviderLaunchRecord::bind(value.clone(), &projection).unwrap();
    record.validate().unwrap();
    let mut tampered = record.clone();
    tampered.input.provider = "ollama".into();
    assert!(tampered.validate().is_err());
    assert_eq!(record.input.digest().unwrap(), record.launch_sha256);
}
