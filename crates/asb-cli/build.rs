// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Generate the exact ASB source identity embedded in the CLI binary.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn valid_identity(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn identity(name: &str, revision: &str) -> Option<String> {
    if let Ok(value) = env::var(name) {
        if !valid_identity(&value) {
            panic!("{name} must contain a 40-character hexadecimal identity");
        }
        return Some(value);
    }
    let output = Command::new("git").args(["rev-parse", revision]).output();
    let output = output.ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    valid_identity(&value).then_some(value)
}

fn main() {
    println!("cargo:rerun-if-env-changed=ASB_SOURCE_COMMIT");
    println!("cargo:rerun-if-env-changed=ASB_SOURCE_TREE");
    let commit = identity("ASB_SOURCE_COMMIT", "HEAD");
    let tree = identity("ASB_SOURCE_TREE", "HEAD^{tree}");
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"));
    fs::write(
        out_dir.join("asb_source_identity.rs"),
        format!(
            "pub const COMMIT: &str = \"{}\";\npub const TREE: &str = \"{}\";\n",
            commit.as_deref().unwrap_or_default(),
            tree.as_deref().unwrap_or_default()
        ),
    )
    .expect("write ASB source identity");
}
