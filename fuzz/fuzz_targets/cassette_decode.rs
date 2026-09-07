// SPDX-License-Identifier: MIT
#![no_main]

use asb_replay::{CassetteLimits, decode_cassette, decode_cassette_chunks};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let limits = CassetteLimits {
        max_cassette_bytes: 64 * 1024,
        max_request_bytes: 16 * 1024,
        max_response_bytes: 32 * 1024,
        max_event_bytes: 8 * 1024,
        max_events: 64,
        max_interactions: 16,
        max_headers: 32,
    };
    let _ = decode_cassette(data, limits);
    let split = data.len() / 2;
    let _ = decode_cassette_chunks(
        [
            data.get(..split).unwrap_or_default(),
            data.get(split..).unwrap_or_default(),
        ],
        limits,
    );
});
