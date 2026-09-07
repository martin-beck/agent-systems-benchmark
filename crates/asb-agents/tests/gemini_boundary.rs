// SPDX-License-Identifier: MIT
//! Public Gemini adapter contract tests.

use asb_agents::gemini::{
    GeminiArtifact, GeminiConfig, SUPPORTED_VERSION, TESTED_BUNDLE_TREE_SHA256,
};
use asb_protocol::Capability;
use url::Url;

#[test]
fn registered_module_exposes_a_bounded_pinned_manifest() {
    let config = GeminiConfig::new(
        "/opt/asb/gemini.js",
        "/opt/asb/node",
        "/work",
        "/state",
        Url::parse("http://127.0.0.1:12345").unwrap(),
        "fixture-model",
        1,
        1,
        GeminiArtifact::LinuxX86_64V0_58_0,
    )
    .unwrap();

    assert_eq!(SUPPORTED_VERSION, "0.58.0");
    assert_eq!(TESTED_BUNDLE_TREE_SHA256.len(), 64);
    assert_eq!(
        config.manifest().implementation_version,
        "gemini-0.58.0+asb-0.1.0"
    );
    assert!(config.manifest().capabilities.contains(&Capability::Usage));
    assert!(
        !config
            .manifest()
            .capabilities
            .contains(&Capability::StreamingEvents),
        "buffered wait evidence must not claim incremental delivery"
    );
}
