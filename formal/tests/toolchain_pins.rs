// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Exact formal-toolchain and workflow pin consistency.

const PINS: &str = include_str!("../toolchains.toml");
const WORKFLOW: &str = include_str!("../../.github/workflows/formal.yml");
const MANIFEST: &str = include_str!("../Cargo.toml");
const TEMPORAL_RUNNER: &str = include_str!("../run_temporal_models.sh");
const TLA_SOURCE_BUILD: &str = include_str!("../tla-provenance/source-build.toml");

#[test]
fn workflow_and_manifest_use_the_reviewed_exact_pins() {
    for required in [
        "kani_version = \"0.67.0\"",
        "kani_rust_toolchain = \"nightly-2025-11-21-x86_64-unknown-linux-gnu\"",
        "kani_action_commit = \"f838096619a707b0f6b2118cf435eaccfa33e51f\"",
        "loom_version = \"0.7.2\"",
        "loom_source_commit = \"a7033ee06a97c52eb2f8131c095fbea6c6eecba3\"",
        "tla_version = \"1.8.0\"",
        "tla_source_commit = \"b123b22654942bd7f8b1bcadcc47da4ee2cf4c0e\"",
        "tla_artifact_origin = \"deterministic-source-build\"",
        "tla_provenance_manifest = \"formal/tla-provenance/source-build.toml\"",
        "tla_sha256 = \"8c200a88d151c6c183c8dbc57a6b633d135e7a2b18242a3afbf243a9e4b68d3e\"",
        "tla_bytes = 4512486",
        "alloy_version = \"6.2.0\"",
        "alloy_source_commit = \"59ba2033993449d483d54acad0e11a7bbf20354f\"",
        "alloy_sha256 = \"6b8c1cb5bc93bedfc7c61435c4e1ab6e688a242dc702a394628d9a9801edb78d\"",
        "alloy_bytes = 21062377",
        "java_version = \"17.0.20+8\"",
        "setup_java_action_commit = \"dded0888837ed1f317902acf8a20df0ad188d165\"",
    ] {
        assert!(PINS.contains(required), "missing pin: {required}");
    }
    assert!(MANIFEST.contains("loom = \"=0.7.2\""));
    assert!(TEMPORAL_RUNNER.contains("/tla-provenance/build.sh"));
    assert!(!TEMPORAL_RUNNER.contains("releases/download/v1.8.0/tla2tools.jar"));
    for shared in [
        "8c200a88d151c6c183c8dbc57a6b633d135e7a2b18242a3afbf243a9e4b68d3e",
        "4512486",
        "1f96ee7ef950e456794d13b7e4d8123c345a91528257a3a41b7cc1b506d1b58f",
        "82989507",
        "71334d7e5d98cfe53d6c429a648a5021137a967378667306c5f613dff5180506",
        "6925830",
        "c0d1549d1e0f5fa5b83622ec0033b00456107e0b1d0cfcce4c1d831532ce621e",
    ] {
        assert!(
            TLA_SOURCE_BUILD.contains(shared),
            "source build lacks {shared}"
        );
        assert!(TEMPORAL_RUNNER.contains(shared), "runner lacks {shared}");
    }
    assert!(
        WORKFLOW
            .contains("model-checking/kani-github-action@f838096619a707b0f6b2118cf435eaccfa33e51f")
    );
    assert!(WORKFLOW.contains("kani-version: \"0.67.0\""));
    assert!(WORKFLOW.contains(
        "cargo metadata --locked --format-version 1 --manifest-path formal/Cargo.toml --no-deps >/dev/null"
    ));
    assert!(WORKFLOW.contains("toolchain install 1.93.0"));
    assert!(WORKFLOW.contains("runner: [ubuntu-24.04]"));
    assert!(!WORKFLOW.contains("ubuntu-24.04-arm"));
    assert!(WORKFLOW.contains("actions/setup-java@dded0888837ed1f317902acf8a20df0ad188d165"));
    assert!(WORKFLOW.contains("java-version: \"17.0.20+8\""));
    assert!(WORKFLOW.contains("formal/run_temporal_models.sh"));
}
