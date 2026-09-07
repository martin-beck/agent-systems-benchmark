// SPDX-License-Identifier: MIT
//! Print the canonical runtime-bundle schema.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "{}",
        serde_json::to_string_pretty(&asb_bundle::manifest_schema())?
    );
    Ok(())
}
