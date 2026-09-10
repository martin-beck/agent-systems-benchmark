// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Write canonical positive, boundary, and negative measurement catalog fixtures.

use std::{fs, path::PathBuf};

use asb_protocol::{MAX_MEASUREMENTS, MeasurementCatalogV1, baseline_measurement_catalog};
use serde_json::Value;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: generate-measurement-fixtures OUTPUT_DIRECTORY")?;
    fs::create_dir_all(&output)?;
    let baseline = baseline_measurement_catalog();
    write(output.join("measurement-catalog.json"), &baseline)?;
    write(
        output.join("measurement-catalog-empty.json"),
        &MeasurementCatalogV1::new(Vec::new(), Vec::new())?,
    )?;

    let mut definitions = Vec::new();
    for index in 0..MAX_MEASUREMENTS {
        let mut definition = baseline.measurements[0].clone();
        definition.id = format!("synthetic.metric_{index:03}");
        definition.name = format!("Synthetic boundary metric {index:03}");
        definitions.push(definition);
    }
    write(
        output.join("measurement-catalog-maximal.json"),
        &MeasurementCatalogV1::new(vec![baseline.groups[0].clone()], definitions)?,
    )?;

    let baseline_json = serde_json::to_value(&baseline)?;
    let mut negatives = Vec::new();
    let mut value = baseline_json.clone();
    value["unexpected"] = Value::Bool(true);
    negatives.push(("measurement-catalog-unknown-field.json", value));
    let mut value = baseline_json.clone();
    value["schema_version"] = Value::from(2);
    negatives.push(("measurement-catalog-wrong-version.json", value));
    let mut value = baseline_json.clone();
    let duplicate = value["measurements"][0].clone();
    value["measurements"]
        .as_array_mut()
        .unwrap()
        .push(duplicate);
    negatives.push(("measurement-catalog-duplicate.json", value));
    let mut value = baseline_json.clone();
    value["measurements"][0]["unit"] = Value::String("By".into());
    negatives.push(("measurement-catalog-unit-mismatch.json", value));
    let mut value = baseline_json.clone();
    value["measurements"][0]["description"] =
        Value::String("https://example.invalid/private".into());
    negatives.push(("measurement-catalog-privacy.json", value));
    let mut value = baseline_json.clone();
    value["measurements"][0]["provenance"]["source"] = Value::String("optional_csb".into());
    value["measurements"][0]["provenance"]["qualification"] = Value::String("unqualified".into());
    negatives.push(("measurement-catalog-unsupported-csb.json", value));
    let mut value = baseline_json;
    value["catalog_sha256"] = Value::String("0".repeat(64));
    negatives.push(("measurement-catalog-digest-mismatch.json", value));
    for (name, value) in negatives {
        write_json(output.join(name), &value)?;
    }
    Ok(())
}

fn write(path: PathBuf, catalog: &MeasurementCatalogV1) -> Result<(), Box<dyn std::error::Error>> {
    let mut encoded = serde_json::to_string_pretty(catalog)?;
    encoded.push('\n');
    fs::write(path, encoded)?;
    Ok(())
}

fn write_json(path: PathBuf, value: &Value) -> Result<(), Box<dyn std::error::Error>> {
    let mut encoded = serde_json::to_string_pretty(value)?;
    encoded.push('\n');
    fs::write(path, encoded)?;
    Ok(())
}
