// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Executable public-command negative journey for human diagnostics.

use std::collections::BTreeSet;
use std::ffi::OsString;

/// Supply an invalid, side-effect-free form for every authoritative public
/// command.  The cardinality assertion below intentionally fences additions:
/// a new command must choose a negative journey before this suite can pass.
fn invalid_args(command: &str) -> Vec<OsString> {
    let words: &[&str] = match command {
        "doctor" | "setup" | "capabilities" | "provider-catalog" | "adapter-catalog"
        | "workload-catalog" | "completion" | "serve" => &[command, "--invalid-diagnostic-fixture"],
        "project" => &["project", "unknown-subcommand"],
        "tool" => &["tool", "unknown-subcommand"],
        "catalog" => &["catalog", "unknown-subcommand"],
        "easy" => &["easy", "unknown-subcommand"],
        "tui" => &["tui", "unknown-subcommand"],
        "config" => &["config", "unknown-subcommand"],
        "auth" => &["auth", "unknown-subcommand"],
        "provider-plan" => &["provider-plan", "--invalid-diagnostic-fixture"],
        "plan" | "run" | "sweep" | "benchmark-live" | "record" | "record-live"
        | "record-campaign" | "replay" | "replay-offline" => {
            &[command, "/definitely/missing/asb-diagnostic-fixture.toml"]
        }
        "compare" | "report" => &[command, "/definitely/missing/asb-diagnostic-fixture"],
        other => panic!("AR-1769 needs an invalid journey for new public command {other}"),
    };
    words.iter().map(OsString::from).collect()
}

#[test]
fn every_public_command_has_a_specific_safe_negative_journey() {
    let expected = [
        "doctor",
        "setup",
        "capabilities",
        "project",
        "tool",
        "catalog",
        "provider-catalog",
        "adapter-catalog",
        "workload-catalog",
        "easy",
        "tui",
        "config",
        "auth",
        "provider-plan",
        "completion",
        "plan",
        "run",
        "sweep",
        "benchmark-live",
        "compare",
        "report",
        "serve",
        "record",
        "record-live",
        "record-campaign",
        "replay",
        "replay-offline",
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    let actual = asb_cli::public_command_inventory()
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    assert_eq!(
        actual, expected,
        "add a negative journey before exposing a command"
    );

    for command in asb_cli::public_command_inventory() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit = asb_cli::run(&invalid_args(command), &mut stdout, &mut stderr);
        assert_ne!(exit, 0, "{command} invalid journey unexpectedly succeeded");
        assert!(
            stderr.is_empty(),
            "{command} leaked human diagnostics to stderr"
        );
        let envelope: serde_json::Value =
            serde_json::from_slice(&stdout).expect("machine envelope is valid JSON");
        let error = envelope["error"].as_object().expect("machine error object");
        assert!(error.contains_key("code"), "{command}: {envelope}");
        let message = error["message"].as_str().expect("machine message");
        assert!(
            !message.contains("/definitely/missing"),
            "{command}: {message}"
        );
        assert!(
            !message.contains("credential payload"),
            "{command}: {message}"
        );
    }
}
