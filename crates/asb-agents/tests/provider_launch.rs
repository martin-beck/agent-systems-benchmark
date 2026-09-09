// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Integration coverage for the provider-aware launch binding.

use asb_agents::all_agents_provider::{EffectiveApiMode, SelectedAgent};
use asb_agents::openai::OpenAiProfile;
use asb_agents::provider_launch::{
    LaunchPolicy, PROVIDER_LAUNCH_V1, ProviderLaunchProjection, ProviderLaunchRecord,
    ProviderLaunchV1, RuntimeBundleIdentity,
};

fn input() -> (ProviderLaunchV1, ProviderLaunchProjection) {
    let profile = OpenAiProfile::new("c".repeat(64)).unwrap();
    let projection = ProviderLaunchProjection::openai(&profile, SelectedAgent::Codex).unwrap();
    let input = ProviderLaunchV1 {
        schema_version: PROVIDER_LAUNCH_V1,
        catalog_sha256: "a".repeat(64),
        selection_sha256: "b".repeat(64),
        provider_profile_sha256: "c".repeat(64),
        agent: "codex".into(),
        adapter: "codex".into(),
        api_mode: EffectiveApiMode::Responses,
        provider: "openai".into(),
        model: projection.model().to_owned(),
        settings_sha256: projection.settings_sha256().to_owned(),
        runtime: RuntimeBundleIdentity {
            bundle_sha256: "e".repeat(64),
            executable_sha256: "f".repeat(64),
        },
        credential: projection.credential().clone(),
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
    };
    (input, projection)
}

#[test]
fn launch_record_is_canonical_and_detects_tampering() {
    let (value, projection) = input();
    let record = ProviderLaunchRecord::bind(value.clone(), &projection).unwrap();
    record.validate().unwrap();
    let mut tampered = record.clone();
    tampered.input.provider = "ollama".into();
    assert!(tampered.validate().is_err());
    assert_eq!(record.input.digest().unwrap(), record.launch_sha256);
}
