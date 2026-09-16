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
    use std::os::unix::net::UnixListener;
    use std::time::{SystemTime, UNIX_EPOCH};

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

    #[test]
    fn authenticated_relay_forwards_http_without_host_network() {
        let root = std::env::temp_dir().join(format!(
            "asb-relay-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("relay.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut handshake = Vec::new();
            let mut byte = [0_u8; 1];
            while handshake.last() != Some(&b'\n') {
                stream.read_exact(&mut byte).unwrap();
                handshake.push(byte[0]);
            }
            assert_eq!(handshake, b"ASB-REPLAY/generation-1\n");
            let mut request = [0_u8; 128];
            let count = stream.read(&mut request).unwrap();
            assert_eq!(&request[..count], b"GET /health HTTP/1.1\r\n\r\n");
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nOK")
                .unwrap();
            stream.shutdown(std::net::Shutdown::Both).unwrap();
        });
        let tcp_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = tcp_listener.local_addr().unwrap();
        let client = std::thread::spawn(move || {
            let mut stream = TcpStream::connect(address).unwrap();
            stream.write_all(b"GET /health HTTP/1.1\r\n\r\n").unwrap();
            stream.shutdown(std::net::Shutdown::Write).unwrap();
            let mut response = Vec::new();
            stream.read_to_end(&mut response).unwrap();
            response
        });
        let (stream, _) = tcp_listener.accept().unwrap();
        forward(stream, path.clone(), "generation-1".into()).unwrap();
        let response = client.join().unwrap();
        server.join().unwrap();
        assert!(response.starts_with(b"HTTP/1.1 200 OK"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
