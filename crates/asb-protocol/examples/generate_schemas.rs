// SPDX-License-Identifier: MIT
//! Print the canonical v1 JSON Schemas to a selected directory.

use std::fs;
use std::path::PathBuf;

use asb_protocol::{
    ExperimentManifestV1, ExtensionManifest, ExtensionResult, ProviderProfileCapabilities,
    ProviderProfileV1, RpcNotification, RpcRequest, WorkloadManifest,
};
use schemars::schema_for;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: generate-schemas OUTPUT_DIRECTORY")?;
    fs::create_dir_all(&output)?;
    write(
        &output,
        "extension-manifest.schema.json",
        &schema_for!(ExtensionManifest),
    )?;
    write(
        &output,
        "workload-manifest.schema.json",
        &schema_for!(WorkloadManifest),
    )?;
    write(
        &output,
        "experiment-manifest.schema.json",
        &schema_for!(ExperimentManifestV1),
    )?;
    write(
        &output,
        "provider-profile.schema.json",
        &schema_for!(ProviderProfileV1),
    )?;
    write(
        &output,
        "provider-capabilities.schema.json",
        &schema_for!(ProviderProfileCapabilities),
    )?;
    write(&output, "request.schema.json", &schema_for!(RpcRequest))?;
    write(
        &output,
        "notification.schema.json",
        &schema_for!(RpcNotification),
    )?;
    write(&output, "result.schema.json", &schema_for!(ExtensionResult))?;
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
