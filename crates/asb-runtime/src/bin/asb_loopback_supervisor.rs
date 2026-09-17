// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Minimal runtime-owned supervisor. It is deliberately fail-closed: the
//! caller must place it in a private namespace; this process never shares or
//! mutates host networking.

use std::env;
use std::io::{self, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

fn value(args: &[String], name: &str) -> Result<String, String> {
    args.windows(2)
        .find(|w| w[0] == name)
        .map(|w| w[1].clone())
        .ok_or_else(|| format!("missing {name}"))
}

fn repeated(args: &[String], name: &str) -> Vec<String> {
    args.windows(2)
        .filter(|w| w[0] == name)
        .map(|w| w[1].clone())
        .collect()
}

fn main() -> Result<(), String> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args
        .iter()
        .any(|a| a == "--share-net" || a == "--network=host")
        || !args.iter().any(|a| a == "--unshare-net")
    {
        return Err("private network marker required; host networking is forbidden".into());
    }
    let relay = value(&args, "--relay")?;
    let generation = value(&args, "--generation")?;
    let route = value(&args, "--route-sha256")?;
    let timeout_ms: u64 = value(&args, "--timeout-ms")?
        .parse()
        .map_err(|_| "invalid timeout".to_owned())?;
    if !Path::new(&relay).is_absolute()
        || generation.is_empty()
        || route.len() != 64
        || timeout_ms == 0
    {
        return Err("invalid bounded handoff".into());
    }
    // Binding proves only that this namespace has a usable loopback socket. It
    // is intentionally done before either child starts.
    let listener =
        TcpListener::bind("127.0.0.1:0").map_err(|_| "loopback is unavailable".to_owned())?;
    listener
        .set_nonblocking(true)
        .map_err(|_| "loopback setup failed".to_owned())?;
    let sidecar = spawn(&args, "--sidecar", "--sidecar-arg")?;
    let adapter = spawn(&args, "--adapter", "--adapter-arg")?;
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    println!(
        "{{\"ready\":true,\"generation\":\"{generation}\",\"route_sha256\":\"{route}\",\"loopback\":\"{}\"}}",
        listener
            .local_addr()
            .map_err(|_| "loopback address failed")?
    );
    io::stdout()
        .flush()
        .map_err(|_| "readiness write failed".to_owned())?;
    supervise(sidecar, adapter, deadline)
}

fn spawn(args: &[String], command_name: &str, arg_name: &str) -> Result<Child, String> {
    let executable = value(args, command_name)?;
    if !Path::new(&executable).is_absolute() {
        return Err("child executable must be absolute".into());
    }
    let mut command = Command::new(executable);
    command
        .args(repeated(args, arg_name))
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    command
        .spawn()
        .map_err(|_| format!("failed to start {command_name}"))
}

fn supervise(mut sidecar: Child, mut adapter: Child, deadline: Instant) -> Result<(), String> {
    let mut sidecar_done = None;
    let mut adapter_done = None;
    loop {
        if Instant::now() >= deadline {
            let _ = sidecar.kill();
            let _ = adapter.kill();
            let _ = sidecar.wait();
            let _ = adapter.wait();
            return Err("supervisor deadline exceeded".into());
        }
        if sidecar_done.is_none() {
            sidecar_done = sidecar.try_wait().map_err(|_| "sidecar wait failed")?;
            if sidecar_done.is_some_and(|status| !status.success()) {
                let _ = adapter.kill();
                let _ = adapter.wait();
                return Err("sidecar failed".into());
            }
        }
        if adapter_done.is_none() {
            adapter_done = adapter.try_wait().map_err(|_| "adapter wait failed")?;
            if adapter_done.is_some_and(|status| !status.success()) {
                let _ = sidecar.kill();
                let _ = sidecar.wait();
                return Err("adapter failed".into());
            }
        }
        if adapter_done.is_some_and(|status| status.success()) && sidecar_done.is_none() {
            let _ = sidecar.kill();
            let _ = sidecar.wait();
            return Ok(());
        }
        if sidecar_done.is_some() && adapter_done.is_some() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(5));
    }
}
