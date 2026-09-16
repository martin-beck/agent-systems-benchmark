// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Bounded loopback-to-Unix replay relay.

use std::env;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::thread;

const MAX_BYTES: u64 = 8 * 1024 * 1024;

fn argument(name: &str) -> Result<String, String> {
    let args: Vec<String> = env::args().collect();
    args.windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].clone())
        .ok_or_else(|| format!("missing {name}"))
}

fn main() -> Result<(), String> {
    let listen = argument("--listen")?;
    let relay = PathBuf::from(argument("--relay")?);
    let generation = argument("--generation")?;
    if !relay.is_absolute() || generation.is_empty() || generation.len() > 128 {
        return Err("invalid bounded relay handoff".into());
    }
    let listener = TcpListener::bind(&listen).map_err(|_| "loopback bind failed")?;
    for stream in listener.incoming() {
        let stream = stream.map_err(|_| "loopback accept failed")?;
        let relay = relay.clone();
        let generation = generation.clone();
        thread::spawn(move || {
            let _ = forward(stream, relay, generation);
        });
    }
    Ok(())
}

fn forward(mut tcp: TcpStream, relay: PathBuf, generation: String) -> io::Result<()> {
    tcp.set_read_timeout(Some(std::time::Duration::from_secs(30)))?;
    tcp.set_write_timeout(Some(std::time::Duration::from_secs(30)))?;
    let mut unix = UnixStream::connect(relay)?;
    unix.write_all(format!("ASB-REPLAY/{generation}\n").as_bytes())?;
    let mut left = tcp.try_clone()?;
    let mut right = unix.try_clone()?;
    let a = thread::spawn(move || copy_bounded(&mut left, &mut right));
    let b = thread::spawn(move || copy_bounded(&mut unix, &mut tcp));
    let _ = a.join();
    let _ = b.join();
    Ok(())
}

fn copy_bounded(reader: &mut impl Read, writer: &mut impl Write) -> io::Result<()> {
    let mut total = 0_u64;
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            return Ok(());
        }
        total = total
            .checked_add(read as u64)
            .ok_or_else(|| io::Error::other("relay byte counter overflow"))?;
        if total > MAX_BYTES {
            return Err(io::Error::other("relay byte limit exceeded"));
        }
        writer.write_all(&buffer[..read])?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_copy_preserves_bytes() {
        let mut input = &b"GET /health HTTP/1.1\r\n\r\n"[..];
        let mut output = Vec::new();
        copy_bounded(&mut input, &mut output).unwrap();
        assert_eq!(output, b"GET /health HTTP/1.1\r\n\r\n");
    }

    #[test]
    fn bounded_copy_rejects_oversized_payload() {
        let data = vec![0_u8; (MAX_BYTES + 1) as usize];
        let mut input = data.as_slice();
        let mut output = Vec::new();
        let error = copy_bounded(&mut input, &mut output).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert!(output.len() <= MAX_BYTES as usize);
    }
}
