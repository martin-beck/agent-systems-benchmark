// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Generate the canonical benchmark-validity v1 JSON Schema.

use asb_workloads::BenchmarkValidityRegistry;
use schemars::schema_for;
use std::fs;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: generate-validity-schema OUTPUT_FILE")?;
    let mut encoded = serde_json::to_string_pretty(&schema_for!(BenchmarkValidityRegistry))?;
    encoded.push('\n');
    fs::write(output, encoded)?;
    Ok(())
}
