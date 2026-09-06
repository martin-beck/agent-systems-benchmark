// SPDX-License-Identifier: MIT
//! Print the canonical cassette schema to a selected directory.

use std::env;
use std::fs;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: generate-cassette-schema OUTPUT_DIRECTORY")?;
    fs::create_dir_all(&output)?;
    fs::write(
        output.join("cassette.schema.json"),
        include_bytes!("../schema/v1/cassette.schema.json"),
    )?;
    Ok(())
}
