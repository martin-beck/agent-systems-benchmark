// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Contract and negative-path tests for the OpenJiuwen pre-start boundary.

use asb_agents::openjiuwen::{
    OpenJiuwenArtifact, OpenJiuwenConfig, PACKAGE_SHA256, SUPPORTED_VERSION, UPSTREAM_REVISION,
    bind_provider_profile,
};
use asb_agents::provider::ProviderProfileAdapter;
use asb_protocol::{
    CredentialProvenance, CredentialSource, EndpointClass, EndpointProvenance, PROVIDER_PROFILE_V1,
    ProviderKind, ProviderProfileV1, ProviderSettings, ProviderTransportLimits,
};
use url::Url;

fn config() -> OpenJiuwenConfig {
    OpenJiuwenConfig::new(
        "/opt/openjiuwen/bin/openjiuwen",
        "/opt/openjiuwen/openjiuwen.whl",
        "/work",
        "/state",
        Url::parse("http://127.0.0.1:8080/v1").unwrap(),
        "fixture-model",
        OpenJiuwenArtifact::LinuxX86_64V0_1_17Post1,
    )
    .unwrap()
}

fn profile(config: &OpenJiuwenConfig) -> ProviderProfileV1 {
    let mut value = ProviderProfileV1 {
        version: PROVIDER_PROFILE_V1,
        settings_sha256: String::new(),
        provider: ProviderKind::OpenAiCompatible,
        endpoint: EndpointProvenance {
            class: EndpointClass::Loopback,
            identity_sha256: config.endpoint_identity_sha256(),
        },
        model: config.model().into(),
        settings: ProviderSettings {
            temperature_milli: None,
            top_p_millionth: None,
            seed: None,
            max_output_tokens: Some(128),
            reasoning_effort: None,
            additional_settings_sha256: None,
        },
        transport: ProviderTransportLimits {
            max_request_bytes: 4096,
            max_response_bytes: 8192,
            connect_timeout_ms: 1000,
            request_timeout_ms: 5000,
            max_concurrent_requests: 1,
        },
        credential: CredentialProvenance {
            source: CredentialSource::None,
            reference_sha256: None,
        },
    };
    value.refresh_settings_sha256().unwrap();
    value
}

#[test]
fn manifest_is_pinned_and_does_not_overclaim_live_capabilities() {
    let manifest = config().manifest();
    assert_eq!(manifest.extension_id.0, "agent.openjiuwen");
    assert_eq!(
        manifest.implementation_version,
        "openjiuwen-0.1.17.post1+asb-0.1.0"
    );
    assert_eq!(manifest.executable_sha256, PACKAGE_SHA256);
    assert!(manifest.capabilities.is_empty());
    assert_eq!(SUPPORTED_VERSION, "0.1.17.post1");
    assert_eq!(
        UPSTREAM_REVISION,
        "4d56dad14b67cdb8fbb3fe58a5654ac8c3e3b815"
    );
}

#[test]
fn exact_provider_profile_is_bound_without_starting_process() {
    let adapter = config();
    let value = profile(&adapter);
    let (prepared, verified) = bind_provider_profile(&adapter, &value).unwrap();
    assert_eq!(prepared.profile(), &value);
    assert_eq!(verified.settings_sha256(), value.settings_sha256);
    assert!(adapter.provider_profile_capabilities().validate().is_ok());
}

#[test]
fn mismatched_model_endpoint_and_credential_fail_closed() {
    let adapter = config();
    let mut wrong_model = profile(&adapter);
    wrong_model.model = "other-model".into();
    wrong_model.refresh_settings_sha256().unwrap();
    assert!(bind_provider_profile(&adapter, &wrong_model).is_err());

    let mut wrong_endpoint = profile(&adapter);
    wrong_endpoint.endpoint.identity_sha256 = "a".repeat(64);
    wrong_endpoint.refresh_settings_sha256().unwrap();
    assert!(bind_provider_profile(&adapter, &wrong_endpoint).is_err());

    let mut credential = profile(&adapter);
    credential.credential = CredentialProvenance {
        source: CredentialSource::Environment,
        reference_sha256: Some("b".repeat(64)),
    };
    credential.refresh_settings_sha256().unwrap();
    assert!(bind_provider_profile(&adapter, &credential).is_err());
}

#[test]
fn unsafe_constructor_inputs_are_rejected() {
    assert!(
        OpenJiuwenConfig::new(
            "relative",
            "/pkg.whl",
            "/work",
            "/state",
            Url::parse("http://127.0.0.1:8080").unwrap(),
            "model",
            OpenJiuwenArtifact::LinuxX86_64V0_1_17Post1,
        )
        .is_err()
    );
    assert!(
        OpenJiuwenConfig::new(
            "/bin/openjiuwen",
            "/pkg.whl",
            "/work",
            "/state",
            Url::parse("http://example.invalid:8080").unwrap(),
            "model",
            OpenJiuwenArtifact::LinuxX86_64V0_1_17Post1,
        )
        .is_err()
    );
}
