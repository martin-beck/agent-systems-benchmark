// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
#![allow(missing_docs)]

use asb_workloads::{
    FIXTURE_IDS, InteractiveAdapter, InteractiveError, InteractiveFamily, InteractiveMockConfig,
    InteractiveToolCall, OriginalWorkloads, SimulatedUserTurn,
};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn scratch_base() -> PathBuf {
    let configured = std::env::var_os("ASB_TEST_SCRATCH")
        .map(|value| ("ASB_TEST_SCRATCH", PathBuf::from(value), false))
        .or_else(|| {
            std::env::var_os("CARGO_TARGET_DIR")
                .map(|value| ("CARGO_TARGET_DIR", PathBuf::from(value), true))
        });
    let Some((name, path, is_target)) = configured else {
        return std::env::temp_dir();
    };
    assert!(path.is_absolute(), "{name} must be an absolute path");
    let base = if is_target {
        path.join("asb-test-scratch")
    } else {
        path
    };
    fs::create_dir_all(&base).unwrap();
    base
}

#[test]
fn public_lifecycle_is_offline_content_free_and_clean() {
    for id in FIXTURE_IDS {
        let root = scratch_base().join(format!(
            "asb-public-workload-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let manifest = OriginalWorkloads::describe(id).unwrap();
        assert!(manifest.allowed_network_destinations.is_empty());
        let prepared = OriginalWorkloads::prepare(id, &root).unwrap();
        assert!(!prepared.prompt().is_empty());
        assert!(prepared.workspace().starts_with(&root));
        assert!(!prepared.workspace().join("reference.patch").exists());
        let initial = prepared.evaluate().unwrap();
        assert!(!initial.passed());
        assert!(initial.failed_checks().iter().all(|item| item.is_ascii()));
        assert_eq!(initial.scoring_version(), "asb-original-oracle-v1");
        prepared.reset().unwrap();
        prepared.cleanup().unwrap();
        assert!(!root.exists());
    }
}

#[test]
fn destination_material_is_never_overwritten() {
    let root = scratch_base().join(format!(
        "asb-public-existing-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&root).unwrap();
    fs::write(root.join("owned"), "caller").unwrap();
    assert!(OriginalWorkloads::prepare(FIXTURE_IDS[0], &root).is_err());
    assert_eq!(fs::read_to_string(root.join("owned")).unwrap(), "caller");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn interactive_public_lifecycle_is_resettable_and_fail_closed() {
    let root = scratch_base().join(format!(
        "asb-public-interactive-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let prepared = InteractiveAdapter::prepare("agentbench", &root).unwrap();
    fs::write(
        prepared.workspace().join("mock-answer.txt"),
        "ASB-LOCAL-MOCK-OK\nstate",
    )
    .unwrap();

    let config = InteractiveMockConfig::for_family(InteractiveFamily::AgentBench);
    let result = prepared.run_local_mock(config).unwrap();
    assert!(result.passed());
    assert!(result.pass_at_k());
    assert!(result.pass_k());
    assert_eq!(result.policy_violations(), 0);

    // Reset removes agent state before a second attempt and never changes the
    // pinned task/scorer identity.
    prepared.reset().unwrap();
    assert!(!prepared.workspace().join("mock-answer.txt").exists());
    let result = prepared
        .run_local_mock(InteractiveMockConfig::for_family(
            InteractiveFamily::AgentBench,
        ))
        .unwrap();
    assert!(!result.passed());
    assert_eq!(
        result.task_revision(),
        "d1e4a10db08c87075c78972e48ecc182be03e2d5"
    );
    prepared.cleanup().unwrap();
    assert!(!root.exists());
}

#[test]
fn interactive_public_contract_rejects_stale_malformed_and_unsafe_inputs() {
    let root = scratch_base().join(format!(
        "asb-public-interactive-negative-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let prepared = InteractiveAdapter::prepare("tau-bench", &root).unwrap();

    let mut stale = InteractiveMockConfig::for_family(InteractiveFamily::TauBench);
    stale.expected_revision = "stale".into();
    assert!(matches!(
        prepared.run_local_mock(stale),
        Err(InteractiveError::StaleRevision)
    ));

    let mut unsafe_tool = InteractiveMockConfig::for_family(InteractiveFamily::TauBench);
    unsafe_tool.tool_calls.push(InteractiveToolCall {
        name: "network.fetch".into(),
        permitted: true,
    });
    assert!(matches!(
        prepared.run_local_mock(unsafe_tool),
        Err(InteractiveError::UnsafeToolCall)
    ));

    let mut malformed_user = InteractiveMockConfig::for_family(InteractiveFamily::TauBench);
    malformed_user.simulated_user = vec![SimulatedUserTurn {
        role: "system".into(),
        content: "unexpected role".into(),
    }];
    assert!(matches!(
        prepared.run_local_mock(malformed_user),
        Err(InteractiveError::MalformedSimulatedUser)
    ));

    let mut scorer_drift = InteractiveMockConfig::for_family(InteractiveFamily::TauBench);
    scorer_drift.scorer_revision = "upstream-scorer".into();
    assert!(matches!(
        prepared.run_local_mock(scorer_drift),
        Err(InteractiveError::ScorerMismatch)
    ));
    prepared.cleanup().unwrap();
}
