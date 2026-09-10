// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Write canonical baseline and empty measurement catalog fixtures.

use std::{fs, path::PathBuf};

use asb_protocol::{MeasurementCatalogV1, baseline_measurement_catalog};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: generate-measurement-fixtures OUTPUT_DIRECTORY")?;
    fs::create_dir_all(&output)?;
    write(
        output.join("measurement-catalog.json"),
        &baseline_measurement_catalog(),
    )?;
    write(
        output.join("measurement-catalog-empty.json"),
        &MeasurementCatalogV1::new(Vec::new(), Vec::new())?,
    )?;
    Ok(())
}

fn write(path: PathBuf, catalog: &MeasurementCatalogV1) -> Result<(), Box<dyn std::error::Error>> {
    let mut encoded = serde_json::to_string_pretty(catalog)?;
    encoded.push('\n');
    fs::write(path, encoded)?;
    Ok(())
}
