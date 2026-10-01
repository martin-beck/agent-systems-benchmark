// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Generate the exact ASB source identity embedded in the CLI binary.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn identity(name: &str, revision: &str) -> String {
    if let Ok(value) = env::var(name) {
        return value;
    }
    let output = Command::new("git")
        .args(["rev-parse", revision])
        .output()
        .unwrap_or_else(|error| panic!("{name} is unavailable: {error}"));
    if !output.status.success() {
        panic!(
            "{name} is unavailable: git rev-parse {revision} failed with {}",
            output.status
        );
    }
    String::from_utf8(output.stdout)
        .unwrap_or_else(|error| panic!("{name} is not UTF-8: {error}"))
        .trim()
        .to_owned()
}

fn main() {
    println!("cargo:rerun-if-env-changed=ASB_SOURCE_COMMIT");
    println!("cargo:rerun-if-env-changed=ASB_SOURCE_TREE");
    let commit = identity("ASB_SOURCE_COMMIT", "HEAD");
    let tree = identity("ASB_SOURCE_TREE", "HEAD^{tree}");
    if commit.len() != 40
        || !commit.bytes().all(|byte| byte.is_ascii_hexdigit())
        || tree.len() != 40
        || !tree.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        panic!("ASB source identity must contain 40 hexadecimal commit and tree values");
    }
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"));
    fs::write(
        out_dir.join("asb_source_identity.rs"),
        format!("pub const COMMIT: &str = \"{commit}\";\npub const TREE: &str = \"{tree}\";\n"),
    )
    .expect("write ASB source identity");
}
