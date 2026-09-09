// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Exact formal-toolchain and workflow pin consistency.

const PINS: &str = include_str!("../toolchains.toml");
const WORKFLOW: &str = include_str!("../../.github/workflows/formal.yml");
const MANIFEST: &str = include_str!("../Cargo.toml");
const TEMPORAL_RUNNER: &str = include_str!("../run_temporal_models.sh");

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
        "tla_release_asset_id = 551717837",
        "tla_release_asset_created_at = \"2026-09-09T01:24:53Z\"",
        "tla_release_asset_url = \"https://github.com/tlaplus/tlaplus/releases/download/v1.8.0/tla2tools.jar\"",
        "tla_sha256 = \"f3a6ba408f84e155d23c75aecc4c89322cfb99822b56d4525833419230cab8bb\"",
        "tla_bytes = 4489229",
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
    assert!(TEMPORAL_RUNNER.contains(
        "TLA_URL=https://github.com/tlaplus/tlaplus/releases/download/v1.8.0/tla2tools.jar"
    ));
    assert!(TEMPORAL_RUNNER.contains("Accept: application/octet-stream"));
    assert!(
        WORKFLOW
            .contains("model-checking/kani-github-action@f838096619a707b0f6b2118cf435eaccfa33e51f")
    );
    assert!(WORKFLOW.contains("kani-version: \"0.67.0\""));
    assert!(WORKFLOW.contains(
        "cargo metadata --locked --format-version 1 --manifest-path formal/Cargo.toml --no-deps >/dev/null"
    ));
    assert!(WORKFLOW.contains("toolchain install 1.93.0"));
    assert!(WORKFLOW.contains("ubuntu-24.04-arm"));
    assert!(WORKFLOW.contains(
        "actions/setup-java@dded0888837ed1f317902acf8a20df0ad188d165"
    ));
    assert!(WORKFLOW.contains("java-version: \"17.0.20+8\""));
    assert!(WORKFLOW.contains("formal/run_temporal_models.sh"));
}
