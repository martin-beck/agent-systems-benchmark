// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Minimal runtime-owned supervisor. It is deliberately fail-closed: the
//! caller must place it in a private namespace; this process never shares or
//! mutates host networking.

use std::env;
use std::io::{self, BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
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

fn main() {
    if let Err(error) = run() {
        let code = error
            .strip_prefix("adapter failed: ")
            .and_then(|value| value.parse::<i32>().ok())
            .filter(|value| (1..=125).contains(value))
            .unwrap_or(1);
        eprintln!("{error}");
        std::process::exit(code);
    }
}

fn run() -> Result<(), String> {
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
    let sidecar_executable = value(&args, "--sidecar")?;
    let sidecar_args = repeated(&args, "--sidecar-arg");
    let sidecar_is_protocol = sidecar_args.iter().any(|arg| arg == "--listen");
    let sidecar_has_handshake_probe = sidecar_args.iter().any(|arg| arg == "--handshake-mode");
    let sidecar = if sidecar_is_protocol && !sidecar_has_handshake_probe {
        spawn_ready_sidecar(&args, &sidecar_executable)?
    } else {
        spawn(&args, "--sidecar", "--sidecar-arg")?
    };
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

fn spawn_ready_sidecar(args: &[String], executable: &str) -> Result<Child, String> {
    let mut command = Command::new(executable);
    command
        .args(repeated(args, "--sidecar-arg"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    let mut child = command.spawn().map_err(|_| "failed to start sidecar")?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "sidecar readiness pipe unavailable".to_owned())?;
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut line = String::new();
        let result = BufReader::new(stdout).read_line(&mut line).map(|_| line);
        let _ = sender.send(result);
    });
    let line = match receiver.recv_timeout(Duration::from_secs(2)) {
        Ok(Ok(line)) => line,
        Ok(Err(_)) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err("sidecar readiness read failed".into());
        }
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err("sidecar readiness timed out".into());
        }
    };
    if line.trim_end() != "ASB_SIDECAR_READY" {
        let _ = child.kill();
        let _ = child.wait();
        return Err("sidecar readiness marker invalid".into());
    }
    Ok(child)
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
            if adapter_done
                .as_ref()
                .is_some_and(|status| !status.success())
            {
                if let Some(status) = adapter_done.as_ref() {
                    println!(
                        "ASB_SUPERVISED_ADAPTER_EXIT={}",
                        status.code().unwrap_or(-1)
                    );
                    let _ = io::stdout().flush();
                }
                // Preserve the final authenticated response even when the
                // adapter deliberately exits non-zero after consuming it.
                thread::sleep(Duration::from_millis(250));
                let _ = sidecar.kill();
                let _ = sidecar.wait();
                return Err(format!(
                    "adapter failed: {}",
                    adapter_done
                        .as_ref()
                        .and_then(std::process::ExitStatus::code)
                        .unwrap_or(1)
                ));
            }
        }
        if adapter_done.is_some_and(|status| status.success()) && sidecar_done.is_none() {
            // Let the bounded relay-copy threads flush the adapter's final
            // response before terminating the long-lived listener.  The
            // grace is fixed and short; it cannot extend the launch deadline.
            thread::sleep(Duration::from_millis(250));
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
