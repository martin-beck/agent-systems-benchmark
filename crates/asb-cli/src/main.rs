// SPDX-License-Identifier: MIT
//! Linux terminal entry point for Agent Systems Benchmark.
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [] => help(),
        [arg] if arg == "--help" || arg == "-h" => help(),
        [arg] if arg == "--version" || arg == "-V" => {
            println!("asb {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("Unsupported command. Use asb --help.");
            ExitCode::from(2)
        }
    }
}

fn help() -> ExitCode {
    println!(
        "Agent Systems Benchmark (ASB)\n\nUsage: asb [--help | --version]\n\nBootstrap release: benchmark execution is planned, not implemented."
    );
    ExitCode::SUCCESS
}
