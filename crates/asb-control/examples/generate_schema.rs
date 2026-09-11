// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Generate the canonical frontend control API schemas.

use std::fs;
use std::path::PathBuf;

use asb_control::{
    analysis_evidence_schema, control_event_schema, control_request_schema,
    control_request_schema_v1_2, control_request_schema_v1_3, control_response_schema,
    control_response_schema_v1_2, control_response_schema_v1_3, history_evidence_schema,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: generate-control-schema OUTPUT_DIRECTORY")?;
    fs::create_dir_all(&output)?;
    write(&output, "request.schema.json", &control_request_schema())?;
    write(&output, "response.schema.json", &control_response_schema())?;
    write(&output, "event.schema.json", &control_event_schema())?;
    write(
        &output,
        "history-evidence.schema.json",
        &history_evidence_schema(),
    )?;
    if let Some(output_v1_2) = std::env::args_os().nth(2).map(PathBuf::from) {
        fs::create_dir_all(&output_v1_2)?;
        write(
            &output_v1_2,
            "request.schema.json",
            &control_request_schema_v1_2(),
        )?;
        write(
            &output_v1_2,
            "response.schema.json",
            &control_response_schema_v1_2(),
        )?;
    }
    if let Some(output_v1_3) = std::env::args_os().nth(3).map(PathBuf::from) {
        fs::create_dir_all(&output_v1_3)?;
        write(
            &output_v1_3,
            "request.schema.json",
            &control_request_schema_v1_3(),
        )?;
        write(
            &output_v1_3,
            "response.schema.json",
            &control_response_schema_v1_3(),
        )?;
    }
    write(
        &output,
        "analysis-evidence.schema.json",
        &analysis_evidence_schema(),
    )?;
    Ok(())
}

fn write(
    path: &std::path::Path,
    name: &str,
    schema: &schemars::Schema,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut encoded = serde_json::to_string_pretty(schema)?;
    encoded.push('\n');
    fs::write(path.join(name), encoded)?;
    Ok(())
}
