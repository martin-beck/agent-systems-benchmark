// SPDX-License-Identifier: MIT
//! Linux terminal entry point for Agent Systems Benchmark.

use std::process::ExitCode;

fn main() -> ExitCode {
    asb_cli::entry(std::env::args_os().skip(1).collect())
}
