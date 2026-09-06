// SPDX-License-Identifier: MIT
//! Linux terminal entry point for Agent Systems Benchmark.
use std::process::ExitCode;

fn main() -> ExitCode {
    run(&std::env::args().skip(1).collect::<Vec<_>>())
}

fn run(args: &[String]) -> ExitCode {
    match args {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(values: &[&str]) -> Vec<String> {
        values.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn empty_arguments_show_help() {
        assert_eq!(run(&[]), ExitCode::SUCCESS);
    }

    #[test]
    fn long_and_short_help_succeed() {
        assert_eq!(run(&arguments(&["--help"])), ExitCode::SUCCESS);
        assert_eq!(run(&arguments(&["-h"])), ExitCode::SUCCESS);
    }

    #[test]
    fn long_and_short_version_succeed() {
        assert_eq!(run(&arguments(&["--version"])), ExitCode::SUCCESS);
        assert_eq!(run(&arguments(&["-V"])), ExitCode::SUCCESS);
    }

    #[test]
    fn unsupported_arguments_have_usage_exit_status() {
        assert_eq!(run(&arguments(&["run"])), ExitCode::from(2));
        assert_eq!(run(&arguments(&["--help", "extra"])), ExitCode::from(2));
    }
}
