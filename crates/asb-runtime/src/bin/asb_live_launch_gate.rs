// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Runtime-owned pre-effect gate for a live provider child.

use std::io::Read;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

const RELEASE: &[u8] = b"ASB-LIVE-RELEASE/1\n";

fn main() {
    if let Err(error) = run(std::env::args().skip(1).collect()) {
        eprintln!("live launch gate stopped: {error}");
        std::process::exit(1);
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    let separator = args
        .iter()
        .position(|arg| arg == "--")
        .ok_or_else(|| "missing adapter separator".to_owned())?;
    let socket = args
        .get(1)
        .filter(|_| args.first().is_some_and(|arg| arg == "--socket"))
        .ok_or_else(|| "missing --socket".to_owned())?;
    let command = args
        .get(separator + 1)
        .ok_or_else(|| "missing gated adapter".to_owned())?;
    if !Path::new(socket).is_absolute() || !Path::new(command).is_absolute() {
        return Err("gate socket and adapter must be absolute".into());
    }
    let mut stream = UnixStream::connect(socket).map_err(|_| "gate connection failed")?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|_| "gate timeout setup failed")?;
    let mut marker = vec![0; RELEASE.len()];
    stream
        .read_exact(&mut marker)
        .map_err(|_| "attestation release was not received")?;
    if marker != RELEASE {
        return Err("attestation release marker was invalid".into());
    }
    let status = Command::new(command)
        .args(&args[separator + 2..])
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|_| "adapter could not be started")?;
    std::process::exit(status.code().unwrap_or(1));
}
