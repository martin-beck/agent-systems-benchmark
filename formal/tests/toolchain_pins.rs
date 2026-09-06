// SPDX-License-Identifier: MIT
//! Exact formal-toolchain and workflow pin consistency.

const PINS: &str = include_str!("../toolchains.toml");
const WORKFLOW: &str = include_str!("../../.github/workflows/formal.yml");
const MANIFEST: &str = include_str!("../Cargo.toml");

#[test]
fn workflow_and_manifest_use_the_reviewed_exact_pins() {
    for required in [
        "kani_version = \"0.67.0\"",
        "kani_rust_toolchain = \"nightly-2025-11-21-x86_64-unknown-linux-gnu\"",
        "kani_action_commit = \"f838096619a707b0f6b2118cf435eaccfa33e51f\"",
        "loom_version = \"0.7.2\"",
        "loom_source_commit = \"a7033ee06a97c52eb2f8131c095fbea6c6eecba3\"",
    ] {
        assert!(PINS.contains(required), "missing pin: {required}");
    }
    assert!(MANIFEST.contains("loom = \"=0.7.2\""));
    assert!(
        WORKFLOW
            .contains("model-checking/kani-github-action@f838096619a707b0f6b2118cf435eaccfa33e51f")
    );
    assert!(WORKFLOW.contains("kani-version: \"0.67.0\""));
    assert!(WORKFLOW.contains("args: --locked --output-format=terse"));
    assert!(WORKFLOW.contains("toolchain install 1.93.0"));
    assert!(WORKFLOW.contains("ubuntu-24.04-arm"));
}
