// SPDX-License-Identifier: MIT
#![no_main]

use asb_protocol::{RpcRequest, read_frame, write_frame};
use libfuzzer_sys::fuzz_target;
use std::io::Cursor;

fuzz_target!(|data: &[u8]| {
    let maximum = 4 * 1024;
    let mut framed = data.to_vec();
    framed.push(b'\n');
    let mut reader = Cursor::new(framed);
    if let Ok(request) = read_frame::<RpcRequest>(&mut reader, maximum) {
        let mut encoded = Vec::new();
        let _ = write_frame(&mut encoded, &request, maximum);
    }
});
