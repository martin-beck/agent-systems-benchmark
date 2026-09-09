// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
#![forbid(unsafe_code)]
#![deny(missing_docs)]
//! Plain startup shell for the independent ASB terminal frontend.

fn main() {
    let terminal_doctor = std::env::args().skip(1).collect::<Vec<_>>();
    if terminal_doctor == ["doctor", "--terminal"] || terminal_doctor == ["--terminal"] {
        match asb_tui::terminal::doctor(asb_tui::terminal::TerminalEvidence::from_environment()) {
            Ok(output) => {
                println!("{output}");
                return;
            }
            Err(error) => {
                eprintln!("terminal doctor failed: {error}");
                std::process::exit(2);
            }
        }
    }
    println!("asb-tui requires a negotiated local frontend connection");
}
