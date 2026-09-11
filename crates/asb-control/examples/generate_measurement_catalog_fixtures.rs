// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Generate canonical control request and response fixtures for the measurement catalog.

use std::fs;
use std::path::{Path, PathBuf};

use asb_control::{
    BoundControlResult, ControlCall, ControlRequest, ControlResponse, ControlSuccess,
    JSONRPC_VERSION, MeasurementCatalogPublication, RequestId,
};
use asb_protocol::baseline_measurement_catalog;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: generate-measurement-catalog-control-fixtures OUTPUT_DIRECTORY")?;
    fs::create_dir_all(&output)?;
    let call = ControlCall::MeasurementCatalog;
    let request = ControlRequest {
        jsonrpc: JSONRPC_VERSION.into(),
        id: RequestId(7),
        timeout_ms: 30_000,
        call: call.clone(),
    };
    let result = BoundControlResult::new(
        &call,
        asb_control::ControlResult::MeasurementCatalog(MeasurementCatalogPublication::built_in(
            baseline_measurement_catalog(),
        )),
    )?;
    let response = ControlResponse::success(RequestId(7), ControlSuccess::Operation(result));
    write(&output, "measurement-catalog-request.json", &request)?;
    write(&output, "measurement-catalog-response.json", &response)?;
    Ok(())
}

fn write(
    output: &Path,
    name: &str,
    value: &impl serde::Serialize,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut encoded = serde_json::to_string_pretty(value)?;
    encoded.push('\n');
    fs::write(output.join(name), encoded)?;
    Ok(())
}
