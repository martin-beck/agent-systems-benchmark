// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Executable-level human-output contract tests.

use std::process::Command;

fn asb() -> Command {
    Command::new(env!("CARGO_BIN_EXE_asb"))
}

#[test]
fn redirected_output_honors_40_80_and_120_column_widths_and_stream_boundaries() {
    for width in [40, 80, 120] {
        let output = asb()
            .arg("provider-catalog")
            .env("COLUMNS", width.to_string())
            .output()
            .expect("ASB executable must run");
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(stdout.starts_with("ASB loaded the provider catalog."));
        assert!(
            stdout.lines().all(|line| line.chars().count() <= width),
            "width={width}: {stdout}"
        );
    }
}

#[test]
fn no_color_explicitly_suppresses_terminal_escape_sequences() {
    let output = asb()
        .arg("doctor")
        .env("NO_COLOR", "1")
        .output()
        .expect("ASB executable must run");
    assert!(output.status.success());
    assert!(!output.stdout.contains(&0x1b));
    assert!(!output.stderr.contains(&0x1b));
}

#[test]
fn executable_usage_failure_is_human_on_stdout_and_preserves_exit_two() {
    let output = asb()
        .arg("not-a-command")
        .output()
        .expect("ASB executable must run");
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.starts_with("ASB could not complete the command:"));
    assert!(stdout.ends_with("Next: asb --help\n"));
}

#[test]
fn executable_json_alias_is_machine_readable_and_not_styled() {
    let output = asb()
        .args(["--format", "json", "doctor"])
        .env("NO_COLOR", "1")
        .output()
        .expect("ASB executable must run");
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert!(!output.stdout.contains(&0x1b));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["command"], "doctor");
}
